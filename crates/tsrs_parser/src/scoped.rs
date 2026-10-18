//! Borrowed views of parsed storage. Raw compiler pointers never leave this interface.

use tsrs_ast::{Diagnostic, Kind, Node, NodeFlags, SourceFile, SourceFileParseOptions};
use tsrs_core::{Allocator, P, ScriptKind, TextRange};

/// A parsed file and all views obtained from it borrow the allocation guard.
///
/// Input is copied into the allocator, so the caller's string can be dropped. Node, list, text,
/// and diagnostic accessors preserve the allocator borrow without exposing raw compiler handles.
#[derive(Clone, Copy)]
pub struct ParsedFile<'alloc> {
    allocator: &'alloc Allocator,
    file: P<SourceFile>,
}

/// Parses into an explicit owner. Nested ambient regions do not change the owner of the AST,
/// its source text, or diagnostics. Lazy parsing uses the same owner when a view is accessed.
pub fn parse_source_file_in<'alloc>(
    allocator: &'alloc Allocator,
    opts: SourceFileParseOptions,
    text: &str,
    script_kind: ScriptKind,
) -> ParsedFile<'alloc> {
    // SAFETY: parsing invokes no caller callbacks and cannot drop enclosing routing guards.
    let file = unsafe {
        allocator.with_scoped(|| {
            let file = crate::parser_1::parse_source_file_with_allocator(
                allocator,
                opts,
                allocator.alloc_str(text),
                script_kind,
            );
            // A scoped tree is read without the compiler's binder. Make pending lists available
            // to normal AST traversal; each is still parsed only when first accessed.
            for record in file.lazy_lists.get() {
                record.mark_unbound();
            }
            let text_index = file.text_index.get();
            allocator.on_free(Box::new(move || tsrs_ast::unregister_source_text(text_index)));
            file
        })
    };
    ParsedFile { allocator, file }
}

impl<'alloc> ParsedFile<'alloc> {
    pub fn text(self) -> &'alloc str {
        self.file.text()
    }
    pub fn file_name(self) -> &'alloc str {
        self.file.get().file_name()
    }
    pub fn script_kind(self) -> ScriptKind {
        self.file.script_kind()
    }
    pub fn root(self) -> NodeRef<'alloc> {
        self.node(self.file.as_node())
    }
    pub fn statements(self) -> NodeList<'alloc> {
        self.list(self.file.statements.nodes())
    }
    pub fn diagnostics(self) -> impl ExactSizeIterator<Item = ParseDiagnostic<'alloc>> + 'alloc {
        self.file
            .diagnostics
            .get()
            .iter()
            .map(move |&diagnostic| ParseDiagnostic { file: self, diagnostic })
    }
    fn node(self, node: P<Node>) -> NodeRef<'alloc> {
        NodeRef { file: self, node }
    }
    fn list(self, nodes: &'alloc [P<Node>]) -> NodeList<'alloc> {
        NodeList { file: self, nodes }
    }
    fn with_owner<R>(self, f: impl FnOnce() -> R) -> R {
        // SAFETY: only internal AST operations run here; they cannot drop the caller's routing guards.
        unsafe { self.allocator.with_scoped(f) }
    }
}

/// A read-only node view. Its children, text, lists, and lazy `JSDoc` keep the same owner borrow.
#[derive(Clone, Copy)]
pub struct NodeRef<'alloc> {
    file: ParsedFile<'alloc>,
    node: P<Node>,
}

impl<'alloc> NodeRef<'alloc> {
    pub fn kind(self) -> Kind {
        self.node.kind()
    }
    pub fn loc(self) -> TextRange {
        self.node.loc()
    }
    pub fn flags(self) -> NodeFlags {
        self.node.flags()
    }
    pub fn text(self) -> &'alloc str {
        self.file.with_owner(|| match self.kind() {
            Kind::JsxText | Kind::JsxTextAllWhiteSpaces => self.node.as_jsx_text().text(),
            _ => self.node.text(),
        })
    }
    pub fn parent(self) -> Option<Self> {
        self.node.parent().map(|n| self.file.node(n))
    }
    pub fn name(self) -> Option<Self> {
        self.node.name().map(|n| self.file.node(n))
    }
    pub fn body(self) -> Option<Self> {
        self.node.body().map(|n| self.file.node(n))
    }
    pub fn expression(self) -> Option<Self> {
        self.node.expression().map(|n| self.file.node(n))
    }
    pub fn type_node(self) -> Option<Self> {
        self.node.type_node().map(|n| self.file.node(n))
    }
    pub fn initializer(self) -> Option<Self> {
        self.node.initializer().map(|n| self.file.node(n))
    }
    pub fn children(self) -> impl ExactSizeIterator<Item = Self> + 'alloc {
        self.file
            .with_owner(|| self.node.iter_children())
            .into_iter()
            .map(move |n| self.file.node(n))
    }
    pub fn members(self) -> NodeList<'alloc> {
        self.file.with_owner(|| self.file.list(self.node.members()))
    }
    pub fn statements(self) -> NodeList<'alloc> {
        self.file.with_owner(|| self.file.list(self.node.statements()))
    }
    pub fn parameters(self) -> NodeList<'alloc> {
        self.file.list(self.node.parameters())
    }
    pub fn modifiers(self) -> NodeList<'alloc> {
        self.file.list(self.node.modifier_nodes())
    }
    pub fn jsdoc(self) -> NodeList<'alloc> {
        self.file.with_owner(|| self.file.list(self.node.jsdoc(Some(self.file.file.get()))))
    }
}

impl PartialEq for NodeRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.node == other.node
    }
}
impl Eq for NodeRef<'_> {}

/// An AST list whose elements cannot be detached from their allocator lifetime.
#[derive(Clone, Copy)]
pub struct NodeList<'alloc> {
    file: ParsedFile<'alloc>,
    nodes: &'alloc [P<Node>],
}

impl<'alloc> NodeList<'alloc> {
    pub fn len(self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(self) -> bool {
        self.nodes.is_empty()
    }
    pub fn get(self, index: usize) -> Option<NodeRef<'alloc>> {
        self.nodes.get(index).map(|&node| self.file.node(node))
    }
    pub fn iter(
        self,
    ) -> impl ExactSizeIterator<Item = NodeRef<'alloc>> + DoubleEndedIterator + 'alloc {
        self.nodes.iter().map(move |&node| self.file.node(node))
    }
}

/// A diagnostic view that keeps its message storage alive without exposing raw AST handles.
#[derive(Clone, Copy)]
pub struct ParseDiagnostic<'alloc> {
    file: ParsedFile<'alloc>,
    diagnostic: P<Diagnostic>,
}

impl<'alloc> ParseDiagnostic<'alloc> {
    pub fn code(self) -> i32 {
        self.diagnostic.code()
    }
    pub fn loc(self) -> Option<TextRange> {
        self.diagnostic.loc()
    }
    pub fn message_text(self) -> &'alloc str {
        self.diagnostic.message_text()
    }
    pub fn message(self) -> String {
        self.file.with_owner(|| self.diagnostic.localize())
    }
}
