//! Allocation policies for the shared scanner. Native policies borrow an explicit file arena.
use tsrs_ast::CommentDirective;
use tsrs_core::{ArenaStringBuilder, stringutil};

/// A token's cooked value. Native scanners use bytes because JavaScript permits lone surrogates.
pub trait TokenValue<'text>: Copy + Default {
    fn as_bytes(self) -> &'text [u8];
    /// Used only for identifiers and numeric spellings, which are UTF-8.
    fn utf8(self) -> &'text str;
}
impl<'text> TokenValue<'text> for &'text str {
    #[inline]
    fn as_bytes(self) -> &'text [u8] {
        self.as_bytes()
    }
    #[inline]
    fn utf8(self) -> &'text str {
        self
    }
}
impl<'text> TokenValue<'text> for &'text [u8] {
    #[inline]
    fn as_bytes(self) -> &'text [u8] {
        self
    }
    #[inline]
    fn utf8(self) -> &'text str {
        std::str::from_utf8(self).expect("identifiers and numeric spellings are UTF-8")
    }
}

/// An unpublished token buffer. Finishing transfers its bytes to the allocation policy's owner.
pub trait TokenBuffer<'text>: std::fmt::Write {
    type Value: TokenValue<'text>;
    fn push_str(&mut self, text: &str);
    fn push(&mut self, ch: char);
    fn push_rune(&mut self, code: u32);
    fn finish(self) -> Self::Value;
    fn finish_str(self) -> &'text str
    where
        Self: Sized,
    {
        self.finish().utf8()
    }
}

/// Scanner allocations must remain alive for `'text`, including saved snapshots and cached values.
/// Implementations cannot rewind storage while any of those references remain live.
pub trait ScannerStorage<'text>: Copy {
    type Value: TokenValue<'text>;
    type Buffer: TokenBuffer<'text, Value = Self::Value>;
    fn buffer(self, capacity: usize) -> Self::Buffer;
    fn source(self, text: &'text str) -> Self::Value;
    fn text(self, text: &str) -> &'text str;
    fn directives(self, values: Vec<CommentDirective>) -> &'text [CommentDirective];
    fn pin(self);
}

/// Compatibility with the existing parser's current arena/region contract.
/// Cooked lone surrogates retain its legacy string representation; native policies use bytes instead.
#[derive(Clone, Copy, Default)]
pub struct LegacyStorage;
impl<'text> ScannerStorage<'text> for LegacyStorage {
    type Value = &'text str;
    type Buffer = ArenaStringBuilder;
    #[inline]
    fn buffer(self, capacity: usize) -> Self::Buffer {
        ArenaStringBuilder::with_capacity(capacity)
    }
    #[inline]
    fn source(self, text: &'text str) -> Self::Value {
        text
    }
    #[inline]
    fn text(self, text: &str) -> &'text str {
        tsrs_core::alloc_str(text)
    }
    #[inline]
    fn directives(self, values: Vec<CommentDirective>) -> &'text [CommentDirective] {
        tsrs_core::alloc_vec(values)
    }
    #[inline]
    fn pin(self) {
        tsrs_core::arena_pin();
    }
}
impl<'text> TokenBuffer<'text> for ArenaStringBuilder {
    type Value = &'text str;
    #[inline]
    fn push_str(&mut self, text: &str) {
        self.push_str(text);
    }
    #[inline]
    fn push(&mut self, ch: char) {
        self.push(ch);
    }
    #[inline]
    fn push_rune(&mut self, code: u32) {
        self.push_str(&stringutil::encode_js_string_rune(code));
    }
    #[inline]
    fn finish(self) -> Self::Value {
        self.into_str()
    }
}

/// A decoded escape or regexp atom. Surrogates remain code units; no invalid Rust string is formed.
#[derive(Clone, Copy)]
pub(crate) enum Cooked<'text> {
    Borrowed(&'text str),
    Rune(u32),
}
impl<'text> Cooked<'text> {
    #[inline]
    pub(crate) fn rune(code: u32) -> Self {
        Self::Rune(if code <= 0x0010_ffff { code } else { 0xfffd })
    }
    #[inline]
    pub(crate) fn decode(&self) -> (u32, usize) {
        match self {
            Self::Borrowed(text) => stringutil::decode_js_string_rune_bytes(text.as_bytes()),
            Self::Rune(code) => (*code, self.len()),
        }
    }
    #[inline]
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Borrowed(text) => text.len(),
            Self::Rune(code) => match code {
                0..=0x7f => 1,
                0x80..=0x7ff => 2,
                0x800..=0xffff => 3,
                _ => 4,
            },
        }
    }
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        matches!(self, Self::Borrowed(text) if text.is_empty())
    }
    #[inline]
    pub(crate) fn append(self, buffer: &mut impl TokenBuffer<'text>) {
        match self {
            Self::Borrowed(text) => buffer.push_str(text),
            Self::Rune(code) => buffer.push_rune(code),
        }
    }
}

/// UTF-8 for a Unicode scalar, CESU-8 for a lone UTF-16 surrogate, or U+FFFD for an invalid code point.
#[inline]
pub fn encode_code_point(code: u32, bytes: &mut [u8; 4]) -> &[u8] {
    if stringutil::is_surrogate(code) {
        bytes[0] = 0xe0 | (code >> 12) as u8;
        bytes[1] = 0x80 | ((code >> 6) & 0x3f) as u8;
        bytes[2] = 0x80 | (code & 0x3f) as u8;
        &bytes[..3]
    } else {
        char::from_u32(code)
            .unwrap_or(char::REPLACEMENT_CHARACTER)
            .encode_utf8(bytes)
            .as_bytes()
    }
}
