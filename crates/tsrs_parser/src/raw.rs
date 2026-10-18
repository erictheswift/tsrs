//! Compiler entry points for the lifetime-erased AST. Prefer [`crate::parse_source_file_in`]
//! when the result does not need to enter the compiler's manually owned object graph.

use tsrs_ast::{Node, SourceFile, SourceFileParseOptions};
use tsrs_core::{P, ScriptKind};

/// Parses into the current compiler allocation region.
///
/// # Safety
/// The caller must retain all backing regions for every use of the file, its nodes, and derived
/// references. Releasing or rewinding a region while any such value is reachable is invalid.
pub unsafe fn parse_source_file(
    opts: SourceFileParseOptions,
    text: &str,
    kind: ScriptKind,
) -> P<SourceFile> {
    crate::parser_1::parse_source_file(opts, text, kind)
}

/// Parses owned input under the compiler's region ownership contract.
///
/// # Safety
/// The caller must retain the file's backing regions for every use of the returned AST and
/// references obtained from it, including lazy `JSDoc` and declaration members.
pub unsafe fn parse_source_file_owned(
    opts: SourceFileParseOptions,
    text: String,
    kind: ScriptKind,
) -> P<SourceFile> {
    crate::parser_1::parse_source_file_owned(opts, text, kind)
}

/// Parses while keeping source text beyond reclamation of a per-file scratch tree.
///
/// # Safety
/// Tree handles and references must not be used after the tree region is released. Only the
/// separately retained source-file metadata and text may survive that release.
pub unsafe fn parse_source_file_keep_text(
    opts: SourceFileParseOptions,
    text: String,
    kind: ScriptKind,
) -> P<SourceFile> {
    crate::parser_1::parse_source_file_keep_text(opts, text, kind)
}

/// Parses process-lifetime source text into a manually retained compiler tree.
///
/// # Safety
/// Static source text does not make the AST static. Retain the tree's backing regions for every
/// use of the returned handles and references.
pub unsafe fn parse_source_file_embedded(
    opts: SourceFileParseOptions,
    text: &'static str,
    kind: ScriptKind,
) -> P<SourceFile> {
    crate::parser_1::parse_source_file_embedded(opts, text, kind)
}

/// Parses an entity name in the current allocation region.
///
/// # Safety
/// The caller must retain the backing region for every use of the returned nodes and their text.
pub unsafe fn parse_isolated_entity_name(text: &str) -> Option<P<Node>> {
    crate::parser_1::parse_isolated_entity_name(text)
}
