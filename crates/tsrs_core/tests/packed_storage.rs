use tsrs_core::{Allocator, PackedStr, ThinSlice, arena::Region};

#[test]
fn packed_text_retains_the_captured_allocator_at_the_length_boundary() {
    let region = Region::new(4096);
    let scope = region.enter();
    let allocator = Allocator::current();
    for len in [0, 5, u16::MAX as usize - 1, u16::MAX as usize, 1 << 16] {
        let input = "x".repeat(len);
        let packed = {
            let nested = Region::new(4096);
            let _scope = nested.enter();
            let packed = PackedStr::new_in(&allocator, &input);
            #[cfg(target_pointer_width = "64")]
            if len >= u16::MAX as usize {
                assert!(
                    Region::containing(packed.as_str().as_ptr().addr()).unwrap().ptr_eq(&region)
                );
            }
            packed
        };
        assert_eq!(packed.as_str(), input);
        if len < u16::MAX as usize {
            assert_eq!(packed.as_str().as_ptr(), input.as_ptr());
        }
    }
    drop(scope);
    drop(region);
    assert_eq!(PackedStr::new_in(&allocator, "retained").as_str(), "retained");
}

#[test]
fn packed_lists_keep_slice_identity_and_the_long_header_owner() {
    let region = Region::new(4096);
    let _scope = region.enter();
    let allocator = Allocator::current();
    for len in [0, 5, u16::MAX as usize, 1 << 16] {
        let input = vec![42_u16; len];
        let packed = {
            let nested = Region::new(4096);
            let _scope = nested.enter();
            let packed = ThinSlice::new_in(&allocator, &input);
            #[cfg(target_pointer_width = "64")]
            if len >= 1 << 16 {
                let header = packed.long_ref().unwrap();
                assert!(
                    Region::containing(std::ptr::from_ref(header).addr()).unwrap().ptr_eq(&region)
                );
            }
            packed
        };
        assert_eq!(packed.get().as_ptr(), input.as_ptr());
        assert_eq!(packed.get().len(), len);
        assert_eq!(packed.get().last(), input.last());
    }
}

#[test]
fn packed_lists_can_borrow_elements_and_an_external_header() {
    let allocator = Allocator::current();
    let text = String::from("borrowed");
    let values = [text.as_str()];
    let slice = &values[..];
    let packed = ThinSlice::new_in(&allocator, slice);
    let from_header = ThinSlice::from_ref(&slice);
    assert_eq!(packed.get(), &["borrowed"]);
    assert_eq!(from_header.get().as_ptr(), slice.as_ptr());
    assert!(from_header.is_long());
    assert!(std::ptr::eq(from_header.long_ref().unwrap(), &raw const slice));
}

#[test]
fn packed_storage_keeps_its_layout() {
    assert_eq!(size_of::<PackedStr<'_>>(), 8);
    assert_eq!(size_of::<ThinSlice<'_, u16>>(), 8);
    assert_eq!(size_of::<Option<ThinSlice<'_, u16>>>(), 8);
}
