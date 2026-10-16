//! A per-file front-end lifetime prototype. Published links are ordinary `&'ast Node<'ast>`.
//!
//! The eager parser and binder currently supply private construction storage. Publication copies
//! their complete syntax payloads and binder graph into immutable records, then frees construction
//! storage. This is an intermediate step toward allocating the final representation directly.
//! `FileArena::build` now supports direct construction and publication in the same slots; wiring the
//! source-file parser and binder to that scoped factory remains in progress.
//! `parse_isolated_entity_name` already parses directly into native references using shared grammar.
//! `NodeFactory::parser` also exposes scoped name, token, literal, literal-type and template-part parsing.
//!
//! ```compile_fail
//! use tsrs_frontend_arena::{FileArena, ParseOptions, parse_and_bind};
//! let node;
//! { let arena = FileArena::new(); node = parse_and_bind(&arena, "const a = 1", ParseOptions::default()).root; }
//! println!("{:?}", node.kind);
//! ```
//!
//! ```compile_fail
//! use tsrs_frontend_arena::{FileArena, ParseOptions, parse_and_bind};
//! let text;
//! { let arena = FileArena::new(); text = parse_and_bind(&arena, "const a = 1", ParseOptions::default()).root.source_file().unwrap().text; }
//! println!("{text}");
//! ```
//!
//! ```compile_fail
//! use tsrs_frontend_arena::{FileArena, ParseOptions, parse_and_bind};
//! let arena = FileArena::new();
//! let file = parse_and_bind(&arena, "const a = 1", ParseOptions::default());
//! let nodes = file.root.source_file().unwrap().statements.nodes.iter();
//! drop(arena);
//! println!("{}", nodes.count());
//! ```
//!
//! ```compile_fail
//! use tsrs_frontend_arena::{FileArena, ParseOptions, parse_and_bind};
//! let arena = FileArena::new();
//! let file = parse_and_bind(&arena, "const a = 1", ParseOptions::default());
//! let declarations = file.root.locals().unwrap().get("a").unwrap().declarations;
//! drop(arena);
//! println!("{}", declarations.len());
//! ```
//!
//! ```compile_fail
//! use tsrs_frontend_arena::{FileArena, ParseOptions, parse_and_bind};
//! let a = FileArena::new(); let b = FileArena::new();
//! let cache = vec![parse_and_bind(&a, "", ParseOptions::default()).root,
//!                  parse_and_bind(&b, "", ParseOptions::default()).root];
//! drop(a);
//! println!("{}", cache.len());
//! ```

//! Mutating a published node is rejected even on its publishing thread.
//!
//! ```compile_fail
//! use tsrs_frontend_arena::{FileArena, ParseOptions, TextRange, parse_and_bind};
//! let arena = FileArena::new();
//! let root = parse_and_bind(&arena, "", ParseOptions::default()).root;
//! root.loc = TextRange::new(0, 1);
//! ```

mod arena;
pub mod construction;
mod factory;
mod parser;
pub mod generated;
mod metadata;
mod model;
mod publish;
mod scanner;
mod text;
pub use scanner::FileTokenBuffer;
pub use text::Text;

pub use arena::FileArena;
pub use factory::NodeFactory;
pub use parser::{Parser, parse_isolated_entity_name};
pub use tsrs_parser::{name_parser::NameParser, token_parser::TokenParser};
pub use generated::NodeData;
pub use metadata::{FileMetadata, FileReference, PatternAmbientModule, Pragma, PragmaArgument};
pub use model::{
    FlowList, FlowNode, FlowReduceLabelData, FlowSwitchClauseData, ModifierList, Node, NodeList, Symbol, SymbolEntry, SymbolTable,
};
pub use tsrs_core::{ScriptKind, TextRange};

#[derive(Clone)]
pub struct ParseOptions {
    pub file_name: String,
    pub script_kind: ScriptKind,
    /// Compare explicit factory allocation with scoped thread-local construction allocation.
    pub direct_allocation: bool,
}
impl Default for ParseOptions {
    fn default() -> Self {
        Self { file_name: "/prototype.ts".to_owned(), script_kind: ScriptKind::TS, direct_allocation: true }
    }
}

/// Detached diagnostics survive release of both the construction and published arenas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub phase: DiagnosticPhase,
    pub file_name: Option<String>,
    pub loc: Option<TextRange>,
    pub code: i32,
    pub category: tsrs_diagnostics::Category,
    pub reports_unnecessary: bool,
    pub reports_deprecated: bool,
    pub message: String,
    pub arguments: Vec<String>,
    pub chain: Vec<Diagnostic>,
    pub related: Vec<Diagnostic>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticPhase {
    Parse,
    JavaScript,
    JSDoc,
    Bind,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GraphCounts {
    pub nodes: usize,
    pub symbols: usize,
    pub flows: usize,
}

pub struct BoundFile<'ast> {
    pub root: &'ast Node<'ast>,
    pub diagnostics: Vec<Diagnostic>,
    pub counts: GraphCounts,
    pub construction_bytes: usize,
}

fn diagnostic(source: tsrs_core::P<tsrs_ast::Diagnostic>, phase: DiagnosticPhase) -> Diagnostic {
    Diagnostic {
        phase,
        file_name: source.file().map(|f| f.file_name().to_owned()),
        loc: source.loc(),
        code: source.code(),
        category: source.category(),
        reports_unnecessary: source.reports_unnecessary(),
        reports_deprecated: source.reports_deprecated(),
        message: source.message().map_or_else(|| source.message_text().to_owned(), |m| m.text().to_owned()),
        arguments: source.message_args().to_vec(),
        chain: source.message_chain().iter().map(|&d| diagnostic(d, phase)).collect(),
        related: source.related_information().iter().map(|&d| diagnostic(d, phase)).collect(),
    }
}

/// Parse and bind one file, publish ordinary borrowed references, then release construction storage.
/// The file name must be normalized and absolute, matching the production parser contract.
pub fn parse_and_bind<'ast>(arena: &'ast FileArena, source: &str, options: ParseOptions) -> BoundFile<'ast> {
    let construction = tsrs_core::arena::Region::new(source.len().saturating_mul(10).saturating_add(4096).min(64 << 20));
    let scope = construction.enter();
    let parse_options = tsrs_ast::SourceFileParseOptions { file_name: options.file_name, ..Default::default() };
    let file = tsrs_parser::parse_source_file_for_publication(
        &construction,
        parse_options,
        source,
        options.script_kind,
        options.direct_allocation,
    );
    tsrs_binder::bind_source_file_for_publication(file);
    let (root, counts) = publish::Publisher::new(arena, file).finish();
    let mut diagnostics = Vec::new();
    for (items, phase) in [
        (file.diagnostics(), DiagnosticPhase::Parse),
        (file.js_diagnostics(), DiagnosticPhase::JavaScript),
        (file.jsdoc_diagnostics(), DiagnosticPhase::JSDoc),
        (file.bind_diagnostics(), DiagnosticPhase::Bind),
    ] {
        diagnostics.extend(items.iter().map(|&d| diagnostic(d, phase)));
    }
    let construction_bytes = construction.used_bytes();
    drop(scope);
    drop(construction);
    BoundFile { root, diagnostics, counts, construction_bytes }
}
