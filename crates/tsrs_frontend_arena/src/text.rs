/// Borrowed JavaScript text, including CESU-8 for lone UTF-16 surrogates in cooked literals.
///
/// Source text remains UTF-8. Cooked literals and symbol names can contain lone surrogate code units,
/// so they use bytes and expose UTF-8 only through a checked conversion.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Text<'ast>(pub(crate) &'ast [u8]);

impl<'ast> Text<'ast> {
    /// Borrow cooked token bytes, including lone surrogates, without copying them again.
    pub const fn from_bytes(bytes: &'ast [u8]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(self) -> &'ast [u8] {
        self.0
    }
    pub fn as_str(self) -> Result<&'ast str, std::str::Utf8Error> {
        std::str::from_utf8(self.0)
    }
}
impl PartialEq<&str> for Text<'_> {
    fn eq(&self, other: &&str) -> bool {
        self.0 == other.as_bytes()
    }
}
impl PartialEq<Text<'_>> for &str {
    fn eq(&self, other: &Text<'_>) -> bool {
        self.as_bytes() == other.0
    }
}
