use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tsrs_core::arena::Region;
use tsrs_core::{Allocator, ArenaStringBuilder};

#[test]
fn guard_keeps_region_alive_after_original_handles_are_dropped() {
    let freed = Arc::new(AtomicBool::new(false));
    let region = Region::new(4096);
    let on_free = Arc::clone(&freed);
    region.on_free(Box::new(move || on_free.store(true, Ordering::SeqCst)));
    let scope = region.enter();
    let allocator = Allocator::current();
    let text = allocator.alloc_str("retained");
    drop(scope);
    drop(region);
    assert!(!freed.load(Ordering::SeqCst));
    assert_eq!(text, "retained");
    drop(allocator);
    assert!(freed.load(Ordering::SeqCst));
}

#[test]
fn nested_scopes_do_not_redirect_scoped_allocations_or_builder_growth() {
    let region = Region::new(4096);
    let _scope = region.enter();
    let allocator = Allocator::current();
    let mut builder = ArenaStringBuilder::with_capacity_in(&allocator, 1);
    builder.push('a');
    let text;
    let slice;
    let values;
    {
        let nested = Region::new(4096);
        let _nested_scope = nested.enter();
        builder.push_str(" longer string 🦀");
        text = builder.into_str();
        slice = allocator.alloc_slice(&[1, 2, 3]);
        values = allocator.alloc_vec(vec![String::from("owned")]);
        for address in [text.as_ptr().addr(), slice.as_ptr().addr(), values.as_ptr().addr()] {
            assert!(Region::containing(address).unwrap().ptr_eq(&region));
        }
    }
    assert_eq!(text, "a longer string 🦀");
    assert_eq!(slice, &[1, 2, 3]);
    assert_eq!(values[0], "owned");
    let borrowed = String::from("borrowed");
    assert_eq!(allocator.alloc_slice(&[borrowed.as_str()]), &["borrowed"]);
}

#[test]
fn checkpoints_and_pins_use_the_captured_region() {
    let region = Region::new(4096);
    let _scope = region.enter();
    let allocator = Allocator::current();
    let checkpoint = allocator.checkpoint();
    let before = region.used_bytes();
    allocator.alloc_str("discarded");
    let nested = Region::new(4096);
    let _nested_scope = nested.enter();
    // SAFETY: no reference to the discarded allocation was retained.
    unsafe { allocator.rewind(checkpoint) };
    assert_eq!(region.used_bytes(), before);
    let checkpoint = allocator.checkpoint();
    let kept = allocator.alloc_str("kept");
    allocator.pin();
    assert!(!allocator.rewindable(&checkpoint));
    // SAFETY: pin prevents this rewind from discarding the retained string.
    unsafe { allocator.rewind(checkpoint) };
    assert_eq!(kept, "kept");
}

#[test]
fn scoped_parse_routing_restores_nested_scratch_and_diagnostics_on_unwind() {
    let outer = Region::new(4096);
    let _outer_scope = outer.enter();
    let scratch = Region::new_scratch(4096);
    let _scratch_scope = scratch.enter_scratch();
    let allocator = Allocator::current();
    let nested = Region::new(4096);
    let _nested_scope = nested.enter();
    let diagnostics = Region::new(4096);
    let _diagnostic_route = diagnostics.route_diagnostics();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: the closure leaves all enclosing routing guards alive, including on unwind.
        unsafe { allocator.with_scoped(|| panic!("parse failed")) }
    }));
    assert!(result.is_err());
    assert!(tsrs_core::arena::current_region().unwrap().ptr_eq(&nested));
    let before = scratch.used_bytes();
    tsrs_core::alloc_str_scratch("restored scratch routing");
    assert!(scratch.used_bytes() > before);
    let _route = tsrs_core::arena::escape_for_diagnostic();
    let text = tsrs_core::alloc_str("restored diagnostic routing");
    assert!(Region::containing(text.as_ptr().addr()).unwrap().ptr_eq(&diagnostics));
}

#[test]
fn borrowed_raw_payloads_drop_with_their_allocation_region() {
    struct Borrowed<'a> {
        text: &'a str,
        dropped: &'a AtomicBool,
    }
    impl Drop for Borrowed<'_> {
        fn drop(&mut self) {
            assert_eq!(self.text, "borrowed");
            self.dropped.store(true, Ordering::SeqCst);
        }
    }
    let text = String::from("borrowed");
    let captured_dropped = AtomicBool::new(false);
    let ambient_dropped = AtomicBool::new(false);
    {
        let region = Region::new(4096);
        let _scope = region.enter();
        let allocator = Allocator::current();
        {
            let nested = Region::new(4096);
            let _scope = nested.enter();
            // SAFETY: text and both drop flags outlive reclamation of both regions.
            let (captured, ambient) = unsafe {
                (
                    allocator.alloc_raw(Borrowed { text: &text, dropped: &captured_dropped }),
                    tsrs_core::ptr::alloc_raw_in(
                        false,
                        Borrowed { text: &text, dropped: &ambient_dropped },
                    ),
                )
            };
            assert!(Region::containing(captured.addr().get()).unwrap().ptr_eq(&region));
            assert!(Region::containing(ambient.addr().get()).unwrap().ptr_eq(&nested));
        }
        assert!(ambient_dropped.load(Ordering::SeqCst));
        assert!(!captured_dropped.load(Ordering::SeqCst));
    }
    assert!(captured_dropped.load(Ordering::SeqCst));
}
