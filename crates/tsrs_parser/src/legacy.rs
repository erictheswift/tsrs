//! Boundary to AST storage whose `P<T>` handles and fields still erase region lifetimes.
//!
//! Parser construction is crate-private. Its input is copied into the captured allocator, borrowed
//! from an existing source file, or process-lifetime text. All computed text and slices stored through
//! these adapters have the same region ownership. Callers of the public parse functions must keep
//! that region alive for the returned AST, as before. An arbitrary caller-owned borrow must never be
//! passed to these adapters. Keep erasure here until AST storage carries its own lifetimes.

use crate::Parser;

impl<'alloc> Parser<'alloc> {
    #[expect(
        clippy::unused_self,
        reason = "the receiver binds the input lifetime to this parser's backing storage contract"
    )]
    pub(crate) fn legacy_text(&self, text: &'alloc str) -> &'static str {
        // SAFETY: parser text is backed by its region or an existing source file, never by the parser
        // object or caller's temporary input; AST users retain that backing under the legacy contract.
        unsafe { &*std::ptr::from_ref(text) }
    }

    #[expect(
        clippy::unused_self,
        reason = "the receiver binds the input lifetime to this parser's backing storage contract"
    )]
    pub(crate) fn legacy_slice<T: 'static>(&self, items: &'alloc [T]) -> &'static [T] {
        // SAFETY: parser slices are backed by the region retained by AST users (see the module contract).
        unsafe { &*std::ptr::from_ref(items) }
    }

    pub(crate) fn legacy_texts(&self, items: &[&'alloc str]) -> &'static [&'static str] {
        let items = self.allocator.alloc_slice(items);
        // SAFETY: both the copied slice and its text follow the module's region ownership contract.
        // Lifetimes do not change the representation of the text references.
        unsafe { std::slice::from_raw_parts(items.as_ptr().cast::<&'static str>(), items.len()) }
    }
}
