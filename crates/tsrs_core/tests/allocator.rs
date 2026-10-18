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
