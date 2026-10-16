use crate::FileArena;
use std::alloc::Layout;
use std::ptr::NonNull;
use tsrs_ast::CommentDirective;
use tsrs_scanner::Scanner;
use tsrs_scanner::storage::{ScannerStorage, TokenBuffer, encode_code_point};

impl FileArena {
    /// Copy source into this file and scan with all persistent token storage borrowed from this arena.
    /// Cooked values are bytes: JavaScript strings may contain lone UTF-16 surrogates.
    ///
    /// ```compile_fail
    /// use tsrs_frontend_arena::FileArena;
    /// let arena = FileArena::new();
    /// let value = {
    ///     let mut scanner = arena.scanner(r#""\ud800""#);
    ///     scanner.scan();
    ///     scanner.token_value()
    /// };
    /// drop(arena);
    /// println!("{value:?}");
    /// ```
    ///
    /// ```compile_fail
    /// use tsrs_frontend_arena::FileArena;
    /// let arena = FileArena::new();
    /// let state = { let mut scanner = arena.scanner("1_000"); scanner.scan(); scanner.mark() };
    /// drop(arena);
    /// println!("{state:?}");
    /// ```
    ///
    /// ```compile_fail
    /// use tsrs_frontend_arena::FileArena;
    /// let arena = FileArena::new();
    /// let directives = {
    ///     let mut scanner = arena.scanner("// @ts-ignore\nvalue");
    ///     scanner.scan();
    ///     scanner.comment_directives()
    /// };
    /// drop(arena);
    /// println!("{}", directives.len());
    /// ```
    pub fn scanner(&self, source: &str) -> Scanner<'_, &Self> {
        let mut scanner = Scanner::with_storage(self);
        scanner.set_text(self.text(source));
        scanner
    }
}

impl<'ast> ScannerStorage<'ast> for &'ast FileArena {
    type Value = &'ast [u8];
    type Buffer = FileTokenBuffer<'ast>;
    fn buffer(self, capacity: usize) -> Self::Buffer {
        FileTokenBuffer {
            arena: self,
            ptr: self.reserve(Layout::array::<u8>(capacity).unwrap()),
            len: 0,
            capacity,
        }
    }
    fn source(self, text: &'ast str) -> Self::Value {
        text.as_bytes()
    }
    fn text(self, text: &str) -> &'ast str {
        FileArena::text(self, text)
    }
    fn directives(self, values: Vec<CommentDirective>) -> &'ast [CommentDirective] {
        self.slice(&values)
    }
    fn pin(self) {} // FileArena cannot rewind; outstanding references already borrow its owner.
}

/// A token under construction in a file arena. Its bytes become immutable when it is finished.
pub struct FileTokenBuffer<'ast> {
    arena: &'ast FileArena,
    ptr: NonNull<u8>,
    len: usize,
    capacity: usize,
}
impl FileTokenBuffer<'_> {
    fn append(&mut self, bytes: &[u8]) {
        let required = self
            .len
            .checked_add(bytes.len())
            .expect("token length overflow");
        if required > self.capacity {
            self.grow(required);
        }
        // SAFETY: the buffer exclusively owns this fresh arena range until finish consumes it. The
        // source cannot overlap this unpublished range, and reserve/grow ensures enough capacity.
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                self.ptr.as_ptr().add(self.len),
                bytes.len(),
            );
        }
        self.len = required;
    }
    #[cold]
    fn grow(&mut self, required: usize) {
        let capacity = self
            .capacity
            .checked_mul(2)
            .unwrap_or(required)
            .max(required)
            .max(16);
        let ptr = self
            .arena
            .reserve(Layout::array::<u8>(capacity).expect("token capacity overflow"));
        // SAFETY: both ranges are in the same live arena, disjoint, and the old range contains len
        // initialized bytes. No reference to either unpublished range has escaped this buffer.
        unsafe {
            std::ptr::copy_nonoverlapping(self.ptr.as_ptr(), ptr.as_ptr(), self.len);
        }
        self.ptr = ptr;
        self.capacity = capacity;
    }
}
impl<'ast> TokenBuffer<'ast> for FileTokenBuffer<'ast> {
    type Value = &'ast [u8];
    fn push_str(&mut self, text: &str) {
        self.append(text.as_bytes());
    }
    fn push(&mut self, ch: char) {
        self.append(ch.encode_utf8(&mut [0; 4]).as_bytes());
    }
    fn push_rune(&mut self, code: u32) {
        self.append(encode_code_point(code, &mut [0; 4]));
    }
    fn finish(self) -> Self::Value {
        // SAFETY: len bytes were initialized inside this arena. Consuming the sole buffer prevents
        // further writes, and the returned slice cannot outlive its borrowed arena owner.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}
impl std::fmt::Write for FileTokenBuffer<'_> {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        self.push_str(text);
        Ok(())
    }
    fn write_char(&mut self, ch: char) -> std::fmt::Result {
        self.push(ch);
        Ok(())
    }
}
