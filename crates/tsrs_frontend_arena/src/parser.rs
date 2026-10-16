//! Scoped native parser state for the grammar shared with the production frontend.
use crate::{Diagnostic, DiagnosticPhase, FileArena, Node, NodeFactory, Text, construction};
use std::fmt::Display;
use std::marker::PhantomData;
use tsrs_ast::{Kind, NodeFlags, TokenFlags};
use tsrs_core::{LanguageVariant, ScriptKind, TextRange};
use tsrs_diagnostics::{self as diagnostics, Message};
use tsrs_parser::name_parser::NameParser;
use tsrs_parser::token_parser::TokenParser;
use tsrs_scanner::{Scanner, ScannerState};
use tsrs_parser::{context_parser::ContextParser, lookahead_parser::LookaheadParser};
use tsrs_parser::{ParsingContexts, list_parser::ListParser};
use tsrs_parser::import_parser::ImportParser;
use tsrs_parser::jsx_parser::JsxParser;
use tsrs_parser::jsdoc_parser::JSDocParser;

/// Parse the same complete entity-name input as `tsrs_parser::parse_isolated_entity_name`, using
/// native references throughout. The result and its token text borrow the supplied arena.
///
/// ```
/// use tsrs_frontend_arena::{FileArena, parse_isolated_entity_name};
/// let arena = FileArena::new();
/// let name = parse_isolated_entity_name(&arena, "Namespace.Type").unwrap();
/// assert!(name.parent.is_none());
/// assert!(parse_isolated_entity_name(&arena, "Namespace.").is_none());
/// ```
///
/// ```compile_fail
/// use tsrs_frontend_arena::{FileArena, parse_isolated_entity_name};
/// let arena = FileArena::new();
/// let name = parse_isolated_entity_name(&arena, "Namespace.Type").unwrap();
/// drop(arena);
/// println!("{:?}", name.kind);
/// ```
pub fn parse_isolated_entity_name<'ast>(
    arena: &'ast FileArena,
    text: &str,
) -> Option<&'ast Node<'ast>> {
    arena
        .try_build(|factory| {
            let mut parser = factory.parser(text, ScriptKind::JS);
            parser.next_token();
            let name = parser.parse_entity_name(true, false, None);
            if parser.token == Kind::EndOfFile && parser.diagnostics.is_empty() {
                Ok(name)
            } else {
                Err(())
            }
        })
        .ok()
}

/// Parser state limited to one construction scope.
///
/// Supports migrated names, tokens, literals and template parts, plus context/speculation,
/// syntax lookahead, individual modifiers, semicolons, lists, import clauses, JSX names/text/closing
/// tags and `JSDoc` links/names/tag comments through shared traits.
/// Full expression and source-file parsing are not provided yet.
/// Create it with `NodeFactory::parser`.
///
/// Parsed lists carry the same construction lifetime as their elements.
///
/// ```compile_fail
/// use tsrs_frontend_arena::{FileArena, ListParser, NameParser, ParsingContext, ScriptKind, TokenParser};
/// let arena = FileArena::new();
/// let mut saved = None;
/// arena.build(|factory| {
///     let mut parser = factory.parser("a, b", ScriptKind::TS);
///     parser.next_token();
///     let list = parser.parse_delimited_list(ParsingContext::Parameters, |p| p.parse_identifier()).unwrap();
///     saved = Some(list);
///     parser.factory().new_block(list, false)
/// });
/// saved.unwrap().set_nodes(&[]);
/// ```
pub struct Parser<'build, 'ast: 'build> {
    scanner: Scanner<'ast, &'ast FileArena>,
    factory: NodeFactory<'build, 'ast>,
    token: Kind,
    context_flags: NodeFlags,
    statement_has_await_identifier: bool,
    has_parse_error: bool,
    diagnostics: Vec<Diagnostic>,
    js_diagnostics: Vec<Diagnostic>,
    parsing_contexts: ParsingContexts,
    node_stack: Vec<&'build construction::Node<'build, 'ast>>,
    reparse_list: Vec<&'build construction::Node<'build, 'ast>>,
    jsdoc_tag_comments_space: Vec<&'ast str>,
    jsdoc_tag_comments_parts_space: Vec<&'build construction::Node<'build, 'ast>>,
}

/// A checkpoint confined to one native construction scope.
///
/// Restoring it rewinds scanner/parser state, retaining all arena allocations. Construction
/// references obtained after a checkpoint stay valid, as do cooked text and scanner caches.
///
/// ```compile_fail
/// use tsrs_frontend_arena::{ContextParser, FileArena, ScriptKind, TokenParser};
/// let arena = FileArena::new();
/// let mut saved = None;
/// arena.build(|factory| {
///     let mut parser = factory.parser("42", ScriptKind::TS);
///     parser.next_token();
///     saved = Some(parser.mark());
///     parser.parse_literal_expression()
/// });
/// drop(saved);
/// ```
#[derive(Clone, Copy)]
pub struct ParserState<'build, 'ast: 'build> {
    scanner_state: ScannerState<'ast, &'ast [u8]>,
    context_flags: NodeFlags,
    diagnostics_len: usize,
    js_diagnostics_len: usize,
    statement_has_await_identifier: bool,
    has_parse_error: bool,
    brand: PhantomData<fn(&'build ()) -> &'build ()>,
}

impl<'build, 'ast> ContextParser<'ast> for Parser<'build, 'ast> {
    type State = ParserState<'build, 'ast>;

    fn mark(&self) -> Self::State {
        ParserState {
            scanner_state: self.scanner.mark(),
            context_flags: self.context_flags,
            diagnostics_len: self.diagnostics.len(),
            js_diagnostics_len: self.js_diagnostics.len(),
            statement_has_await_identifier: self.statement_has_await_identifier,
            has_parse_error: self.has_parse_error,
            brand: PhantomData,
        }
    }

    fn rewind(&mut self, state: Self::State) {
        self.scanner.rewind(state.scanner_state);
        self.token = self.scanner.token();
        self.context_flags = state.context_flags;
        self.diagnostics.truncate(state.diagnostics_len);
        self.js_diagnostics.truncate(state.js_diagnostics_len);
        self.statement_has_await_identifier = state.statement_has_await_identifier;
        self.has_parse_error = state.has_parse_error;
        // Never reclaim storage here: speculative nodes and scanner values may still be borrowed.
    }

    fn context_flags(&self) -> NodeFlags {
        self.context_flags
    }
    fn replace_context_flags(&mut self, flags: NodeFlags) {
        self.context_flags = flags;
    }
}

impl<'ast> LookaheadParser<'ast> for Parser<'_, 'ast> {
    fn new_modifier(&self, kind: Kind) -> Self::Node {
        // The legacy factory uses the same token constructor for modifiers.
        self.factory.new_token(kind)
    }
}

impl<'build, 'ast> ListParser<'ast> for Parser<'build, 'ast> {
    type NodeList = &'build construction::NodeList<'build, 'ast>;

    fn parsing_contexts(&self) -> ParsingContexts {
        self.parsing_contexts
    }
    fn replace_parsing_contexts(&mut self, contexts: ParsingContexts) {
        self.parsing_contexts = contexts;
    }
    fn node_stack(&self) -> &[Self::Node] {
        &self.node_stack
    }
    fn node_stack_mut(&mut self) -> &mut Vec<Self::Node> {
        &mut self.node_stack
    }
    fn reparse_list(&mut self) -> &mut Vec<Self::Node> {
        &mut self.reparse_list
    }
    fn node_kind(node: Self::Node) -> Kind {
        node.kind()
    }
    fn new_node_list(&self, loc: TextRange, nodes: &[Self::Node]) -> Self::NodeList {
        // Parsed lists derive trailing commas from their last child's end, just like NodeList.
        // The legacy clone-only marker is not needed when constructing source syntax directly.
        let has_trailing_comma = nodes.last().is_some_and(|last| last.loc().end() < loc.end());
        self.factory.alloc_node_list(loc, self.factory.nodes(nodes), has_trailing_comma, false)
    }
    fn new_missing_node_list(&self, loc: TextRange) -> Self::NodeList {
        // An explicit bit preserves the empty/missing distinction without a raw-pointer sentinel.
        self.factory.alloc_node_list(loc, &[], false, true)
    }
    fn is_missing_node_list(list: Option<Self::NodeList>) -> bool {
        list.is_some_and(construction::NodeList::is_missing)
    }
}

impl<'ast> ImportParser<'ast> for Parser<'_, 'ast> {
    fn node_loc(node: Self::Node) -> TextRange { node.loc() }
    fn node_flags(node: Self::Node) -> NodeFlags { node.flags() }
    fn node_text(node: Self::Node) -> &'ast [u8] { node.text().as_bytes() }
    fn import_specifier_is_type_only(node: Self::Node) -> bool {
        node.as_import_specifier().is_type_only()
    }
    fn set_skip_jsdoc_leading_asterisks(&mut self, skip: bool) {
        self.scanner.set_skip_jsdoc_leading_asterisks(skip);
    }
    fn js_error_at_range(&mut self, loc: TextRange, message: &'static Message, args: &[&dyn Display]) {
        let range = self.skip_range_trivia(loc);
        // JavaScript syntax errors neither deduplicate nor mark the next parsed node as erroneous.
        self.js_diagnostics.push(Self::new_diagnostic(DiagnosticPhase::JavaScript, range, message, args));
    }
    fn new_namespace_import(&self, name: Self::Node) -> Self::Node {
        self.factory.new_namespace_import(name)
    }
    fn new_named_imports(&self, elements: Self::NodeList) -> Self::Node {
        self.factory.new_named_imports(elements)
    }
    fn new_import_specifier(&self, is_type_only: bool, property_name: Option<Self::Node>, name: Self::Node) -> Self::Node {
        self.factory.new_import_specifier(is_type_only, property_name, name)
    }
    fn new_namespace_export(&self, name: Self::Node) -> Self::Node {
        self.factory.new_namespace_export(name)
    }
    fn new_import_clause(&self, phase_modifier: Kind, name: Option<Self::Node>, named_bindings: Option<Self::Node>) -> Self::Node {
        self.factory.new_import_clause(phase_modifier, name, named_bindings)
    }
}

impl<'build, 'ast> Parser<'build, 'ast> {
    pub(crate) fn new(
        mut scanner: Scanner<'ast, &'ast FileArena>,
        factory: NodeFactory<'build, 'ast>,
        script_kind: ScriptKind,
    ) -> Self {
        assert!(
            script_kind != ScriptKind::Unknown,
            "ScriptKind must be specified"
        );
        scanner.set_on_error(true);
        scanner.set_language_variant(match script_kind {
            ScriptKind::TSX | ScriptKind::JSX | ScriptKind::JS | ScriptKind::JSON => {
                LanguageVariant::JSX
            }
            _ => LanguageVariant::Standard,
        });
        Self {
            scanner,
            factory,
            token: Kind::Unknown,
            context_flags: match script_kind {
                ScriptKind::JS | ScriptKind::JSX => NodeFlags::JavaScriptFile,
                ScriptKind::JSON => NodeFlags::JavaScriptFile | NodeFlags::JsonFile,
                _ => NodeFlags::None,
            },
            statement_has_await_identifier: false,
            has_parse_error: false,
            diagnostics: Vec::new(),
            js_diagnostics: Vec::new(),
            parsing_contexts: 0,
            node_stack: Vec::new(),
            reparse_list: Vec::new(),
            jsdoc_tag_comments_space: Vec::new(),
            jsdoc_tag_comments_parts_space: Vec::new(),
        }
    }

    /// Parse diagnostics produced so far, in scanner/parser order.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// JavaScript syntax diagnostics, kept separate from parse errors as in the source-file parser.
    pub fn js_diagnostics(&self) -> &[Diagnostic] {
        &self.js_diagnostics
    }

    /// Construct nodes for list-element callbacks in this parser's existing construction scope.
    pub fn factory(&self) -> &NodeFactory<'build, 'ast> {
        &self.factory
    }

    /// Detach parse diagnostics without retaining the parser or its construction scope.
    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    /// Detach both diagnostic streams, in parse-then-JavaScript order.
    pub fn into_diagnostic_streams(self) -> (Vec<Diagnostic>, Vec<Diagnostic>) {
        (self.diagnostics, self.js_diagnostics)
    }

    fn new_diagnostic(
        phase: DiagnosticPhase,
        loc: TextRange,
        message: &'static Message,
        args: &[&dyn Display],
    ) -> Diagnostic {
        Diagnostic {
            phase,
            file_name: None,
            loc: Some(loc),
            code: message.code(),
            category: message.category(),
            reports_unnecessary: message.reports_unnecessary(),
            reports_deprecated: message.reports_deprecated(),
            message: message.text().to_owned(),
            arguments: args.iter().map(ToString::to_string).collect(),
            chain: Vec::new(),
            related: Vec::new(),
        }
    }

    #[cold]
    fn report_scan_errors(&mut self) {
        for error in self.scanner.take_errors() {
            let args: Vec<&dyn Display> =
                error.args.iter().map(|arg| arg as &dyn Display).collect();
            self.report_error(
                TextRange::new(error.start, error.start + error.length),
                error.message,
                &args,
            );
        }
    }
}

impl<'ast> NameParser<'ast> for Parser<'_, 'ast> {

    fn statement_has_await_identifier(&self) -> bool {
        self.statement_has_await_identifier
    }
    fn set_statement_has_await_identifier(&mut self, value: bool) {
        self.statement_has_await_identifier = value;
    }

    fn new_identifier(&mut self, text: &'ast str) -> Self::Node {
        if text == "await" {
            self.statement_has_await_identifier = true;
        }
        self.factory
            .new_identifier(Text::from_bytes(text.as_bytes()))
    }

    fn new_source_identifier(&mut self, text: &'ast [u8], _loc: TextRange) -> Self::Node {
        if text == b"await" {
            self.statement_has_await_identifier = true;
        }
        self.factory.new_identifier(Text::from_bytes(text))
    }

    fn new_private_identifier(&self, text: &'ast [u8]) -> Self::Node {
        self.factory.new_private_identifier(Text::from_bytes(text))
    }

    fn new_qualified_name(&self, left: Self::Node, right: Self::Node) -> Self::Node {
        self.factory.new_qualified_name(left, right)
    }
}

impl<'build, 'ast> TokenParser<'ast> for Parser<'build, 'ast> {
    type Node = &'build construction::Node<'build, 'ast>;
    type Storage = &'ast FileArena;

    fn scanner(&self) -> &Scanner<'ast, &'ast FileArena> {
        &self.scanner
    }

    fn token(&self) -> Kind {
        self.token
    }

    fn next_token(&mut self) -> Kind {
        if tsrs_ast::is_keyword(self.token)
            && (self.scanner.has_unicode_escape() || self.scanner.has_extended_unicode_escape())
        {
            self.parse_error_at_current_token(
                &diagnostics::Keywords_cannot_contain_escape_characters,
                &[],
            );
        }
        self.next_token_without_check()
    }

    fn next_token_without_check(&mut self) -> Kind {
        self.token = self.scanner.scan();
        if self.scanner.has_errors() {
            self.report_scan_errors();
        }
        self.token
    }

    fn finish_node(&mut self, node: Self::Node, pos: u32) -> Self::Node {
        self.finish_node_with_end(node, pos, self.node_pos())
    }

    fn finish_node_with_end(&mut self, node: Self::Node, pos: u32, end: u32) -> Self::Node {
        node.set_loc(TextRange::new(pos, end));
        node.set_flags(node.flags() | self.context_flags);
        if self.has_parse_error {
            node.set_flags(node.flags() | NodeFlags::ThisNodeHasError);
            self.has_parse_error = false;
        }
        node.for_each_child(&mut |child| {
            child.set_parent(Some(node));
            false
        });
        node
    }

    fn report_error(&mut self, loc: TextRange, message: &'static Message, args: &[&dyn Display]) {
        // Match parse_error_at_range: suppress only consecutive errors at the same start position.
        if self
            .diagnostics
            .last()
            .is_none_or(|last| last.loc.map(TextRange::source_pos) != Some(loc.source_pos()))
        {
            self.diagnostics.push(Self::new_diagnostic(DiagnosticPhase::Parse, loc, message, args));
        }
        self.has_parse_error = true;
    }
    fn re_scan_slash_token(&mut self) -> Kind {
        self.token = self.scanner.re_scan_slash_token(false);
        if self.scanner.has_errors() {
            self.report_scan_errors();
        }
        self.token
    }
    fn re_scan_template_token(&mut self, is_tagged_template: bool) -> Kind {
        self.token = self.scanner.re_scan_template_token(is_tagged_template);
        if self.scanner.has_errors() {
            self.report_scan_errors();
        }
        self.token
    }
    fn new_token(&self, kind: Kind) -> Self::Node {
        self.factory.new_token(kind)
    }
    fn new_keyword_expression(&self, kind: Kind) -> Self::Node {
        self.factory.new_keyword_expression(kind)
    }
    fn new_string_literal(&self, text: &'ast [u8], flags: TokenFlags) -> Self::Node {
        self.factory
            .new_string_literal(Text::from_bytes(text), flags)
    }
    fn new_numeric_literal(&self, text: &'ast [u8], flags: TokenFlags) -> Self::Node {
        self.factory
            .new_numeric_literal(Text::from_bytes(text), flags)
    }
    fn new_big_int_literal(&self, text: &'ast [u8], flags: TokenFlags) -> Self::Node {
        self.factory
            .new_big_int_literal(Text::from_bytes(text), flags)
    }
    fn new_regular_expression_literal(&self, text: &'ast [u8], flags: TokenFlags) -> Self::Node {
        self.factory
            .new_regular_expression_literal(Text::from_bytes(text), flags)
    }
    fn new_no_substitution_template_literal(
        &self,
        text: &'ast [u8],
        flags: TokenFlags,
    ) -> Self::Node {
        self.factory
            .new_no_substitution_template_literal(Text::from_bytes(text), flags)
    }
    fn new_template_head(
        &self,
        text: &'ast [u8],
        raw_text: &'ast str,
        flags: TokenFlags,
    ) -> Self::Node {
        self.factory.new_template_head(
            Text::from_bytes(text),
            Text::from_bytes(raw_text.as_bytes()),
            flags,
        )
    }
    fn new_template_middle(
        &self,
        text: &'ast [u8],
        raw_text: &'ast str,
        flags: TokenFlags,
    ) -> Self::Node {
        self.factory.new_template_middle(
            Text::from_bytes(text),
            Text::from_bytes(raw_text.as_bytes()),
            flags,
        )
    }
    fn new_template_tail(
        &self,
        text: &'ast [u8],
        raw_text: &'ast str,
        flags: TokenFlags,
    ) -> Self::Node {
        self.factory.new_template_tail(
            Text::from_bytes(text),
            Text::from_bytes(raw_text.as_bytes()),
            flags,
        )
    }
    fn new_prefix_unary_expression(&self, kind: Kind, operand: Self::Node) -> Self::Node {
        self.factory.new_prefix_unary_expression(kind, operand)
    }
    fn new_literal_type_node(&self, literal: Self::Node) -> Self::Node {
        self.factory.new_literal_type_node(literal)
    }
}

impl<'ast> JsxParser<'ast> for Parser<'_, 'ast> {
    fn re_scan_jsx_token(&mut self, allow_multiline_jsx_text: bool) -> Kind {
        let token = self.scanner.re_scan_jsx_token(allow_multiline_jsx_text);
        self.report_scan_errors();
        token
    }
    fn scan_jsx_text(&mut self) -> Kind {
        self.token = self.scanner.scan_jsx_token();
        self.report_scan_errors();
        self.token
    }
    fn scan_jsx_identifier(&mut self) -> Kind {
        self.token = self.scanner.scan_jsx_identifier();
        self.report_scan_errors();
        self.token
    }
    fn scan_jsx_attribute_value(&mut self) -> Kind {
        self.token = self.scanner.scan_jsx_attribute_value();
        self.report_scan_errors();
        self.token
    }
    fn is_jsx_namespaced_name(node: Self::Node) -> bool {
        node.kind() == Kind::JsxNamespacedName
    }
    fn jsx_tag_name(node: Self::Node) -> Self::Node {
        match node.data() {
            construction::NodeData::JsxOpeningElement(data) => data.tag_name(),
            construction::NodeData::JsxClosingElement(data) => data.tag_name(),
            construction::NodeData::JsxSelfClosingElement(data) => data.tag_name(),
            _ => panic!("JSX element expected"),
        }
    }
    fn tag_names_are_equivalent(lhs: Self::Node, rhs: Self::Node) -> bool {
        // The same cases as ast::tag_names_are_equivalent, using borrowed construction payloads.
        if lhs.kind() != rhs.kind() {
            return false;
        }
        match lhs.kind() {
            Kind::Identifier => lhs.text() == rhs.text(),
            Kind::ThisKeyword => true,
            Kind::JsxNamespacedName => {
                let left = lhs.as_jsx_namespaced_name();
                let right = rhs.as_jsx_namespaced_name();
                left.namespace().text() == right.namespace().text()
                    && left.name().text() == right.name().text()
            }
            Kind::PropertyAccessExpression => {
                let left = lhs.as_property_access_expression();
                let right = rhs.as_property_access_expression();
                left.name().text() == right.name().text()
                    && Self::tag_names_are_equivalent(left.expression(), right.expression())
            }
            _ => panic!("Unhandled case in TagNamesAreEquivalent"),
        }
    }
    fn new_jsx_text(
        &self,
        text: &'ast [u8],
        contains_only_trivia_white_spaces: bool,
    ) -> Self::Node {
        self.factory
            .new_jsx_text(Text::from_bytes(text), contains_only_trivia_white_spaces)
    }
    fn new_jsx_namespaced_name(&self, namespace: Self::Node, name: Self::Node) -> Self::Node {
        self.factory.new_jsx_namespaced_name(namespace, name)
    }
    fn new_jsx_property_access(&self, expression: Self::Node, name: Self::Node) -> Self::Node {
        self.factory
            .new_property_access_expression(expression, None, name, NodeFlags::None)
    }
    fn new_jsx_closing_element(&self, tag_name: Self::Node) -> Self::Node {
        self.factory.new_jsx_closing_element(tag_name)
    }
    fn new_jsx_closing_fragment(&self) -> Self::Node {
        self.factory.new_jsx_closing_fragment()
    }
}

impl<'ast> JSDocParser<'ast> for Parser<'_, 'ast> {
    type TextList = &'ast [Text<'ast>];

    fn next_jsdoc_comment_text_token(&mut self, in_backticks: bool) -> Kind {
        self.token = self.scanner.scan_jsdoc_comment_text_token(in_backticks);
        self.report_scan_errors();
        self.token
    }
    fn jsdoc_tag_comments_space(&mut self) -> &mut Vec<&'ast str> {
        &mut self.jsdoc_tag_comments_space
    }
    fn jsdoc_tag_comments_parts_space(&mut self) -> &mut Vec<Self::Node> {
        &mut self.jsdoc_tag_comments_parts_space
    }
    fn copy_jsdoc_text(&self, text: &str) -> &'ast str {
        self.factory.source(text)
    }
    fn new_jsdoc_text(&self, text: &[&'ast str]) -> Self::Node {
        let text: Vec<_> = text
            .iter()
            .map(|s| Text::from_bytes(s.as_bytes()))
            .collect();
        self.factory.new_jsdoc_text(self.factory.values(&text))
    }
    fn new_jsdoc_unknown_tag(
        &self,
        tag_name: Self::Node,
        comment: Option<Self::NodeList>,
    ) -> Self::Node {
        self.factory.new_jsdoc_unknown_tag(tag_name, comment)
    }
    fn new_jsdoc_see_tag(
        &self,
        tag_name: Self::Node,
        name_expression: Option<Self::Node>,
        comment: Option<Self::NodeList>,
    ) -> Self::Node {
        self.factory
            .new_jsdoc_see_tag(tag_name, name_expression, comment)
    }

    fn next_token_jsdoc(&mut self) -> Kind {
        self.token = self.scanner.scan_jsdoc_token();
        self.report_scan_errors();
        self.token
    }
    fn reset_scanner_pos(&mut self, pos: u32) {
        self.scanner.reset_pos(pos);
    }
    fn re_scan_hash_token(&mut self) {
        self.scanner.re_scan_hash_token();
        self.report_scan_errors();
    }
    fn jsdoc_name_pos(node: Self::Node) -> u32 {
        node.loc()
            .pos()
            .expect_source("parsed nodes have source positions")
    }
    fn alloc_jsdoc_text(&self, text: Vec<&'ast str>) -> Self::TextList {
        let text: Vec<_> = text
            .into_iter()
            .map(|s| Text::from_bytes(s.as_bytes()))
            .collect();
        self.factory.values(&text)
    }
    fn new_jsdoc_name_reference(&self, name: Self::Node) -> Self::Node {
        self.factory.new_jsdoc_name_reference(name)
    }
    fn new_jsdoc_link(&self, name: Option<Self::Node>, text: Self::TextList) -> Self::Node {
        self.factory.new_jsdoc_link(name, text)
    }
    fn new_jsdoc_link_code(&self, name: Option<Self::Node>, text: Self::TextList) -> Self::Node {
        self.factory.new_jsdoc_link_code(name, text)
    }
    fn new_jsdoc_link_plain(&self, name: Option<Self::Node>, text: Self::TextList) -> Self::Node {
        self.factory.new_jsdoc_link_plain(name, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_rewinds_lookahead_and_preserves_parser_error_flags() {
        let arena = FileArena::new();
        let scanner = arena.scanner("a.\nb c");
        let mut diagnostics = Vec::new();
        let root = arena.build(|factory| {
            let mut parser = Parser::new(scanner, factory, ScriptKind::JS);
            parser.next_token();
            let node = parser.parse_entity_name(true, false, None);
            assert_eq!(parser.token, Kind::Identifier);
            assert_eq!(parser.scanner.token_value(), b"b");
            diagnostics = parser.diagnostics;
            node
        });
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, diagnostics::Identifier_expected.code());
        assert_eq!(diagnostics[0].loc, Some(TextRange::new(2, 2)));
        let crate::NodeData::QualifiedName(name) = root.data else {
            panic!("qualified name");
        };
        assert!(name.right.flags.intersects(NodeFlags::ThisNodeHasError));
        assert_eq!(name.right.loc, TextRange::new(2, 2));
        assert!(!root.flags.intersects(NodeFlags::ThisNodeHasError));
        drop(arena);
        assert_eq!(
            diagnostics[0].message,
            diagnostics::Identifier_expected.text()
        );
    }

    #[test]
    fn scanner_error_replayed_after_lookahead_is_retained_once() {
        let arena = FileArena::new();
        let scanner = arena.scanner("a.\nb /* unterminated");
        let mut diagnostics = Vec::new();
        let root = arena.build(|factory| {
            let mut parser = Parser::new(scanner, factory, ScriptKind::JS);
            parser.next_token();
            let node = parser.parse_entity_name(true, false, None);
            assert_eq!(parser.token, Kind::EndOfFile);
            diagnostics = parser.diagnostics;
            node
        });
        assert_eq!(diagnostics.len(), 1);
        let crate::NodeData::QualifiedName(name) = root.data else {
            panic!("qualified name");
        };
        assert_eq!(name.right.text(), "b");
        assert!(name.right.flags.intersects(NodeFlags::ThisNodeHasError));
    }

    #[test]
    fn identifier_context_and_await_state_match_the_parser_contract() {
        for (text, flag) in [
            ("await", NodeFlags::AwaitContext),
            ("yield", NodeFlags::YieldContext),
        ] {
            for binding in [false, true] {
                let arena = FileArena::new();
                let scanner = arena.scanner(text);
                let root = arena.build(|factory| {
                    let mut parser = Parser::new(scanner, factory, ScriptKind::JS);
                    parser.context_flags |= flag;
                    parser.next_token();
                    let node = if binding {
                        parser.parse_binding_identifier()
                    } else {
                        parser.parse_identifier()
                    };
                    assert_eq!(parser.diagnostics.is_empty(), binding);
                    assert!(!parser.statement_has_await_identifier);
                    node
                });
                assert_eq!(root.text(), if binding { text } else { "" });
                assert_eq!(root.flags.intersects(NodeFlags::ThisNodeHasError), !binding);
            }
        }
    }

    #[test]
    fn cooked_identifier_bytes_are_borrowed_without_a_second_copy() {
        for text in ["identifier", r"\u0069dentifier"] {
            let arena = FileArena::new();
            let scanner = arena.scanner(text);
            let mut address = 0;
            let root = arena.build(|factory| {
                let mut parser = Parser::new(scanner, factory, ScriptKind::JS);
                parser.next_token();
                address = parser.scanner.token_value().as_ptr().addr();
                parser.parse_entity_name(true, false, None)
            });
            assert_eq!(root.text(), "identifier");
            assert_eq!(root.text().as_bytes().as_ptr().addr(), address);
        }
    }

    #[test]
    fn failed_construction_returns_owned_diagnostics_and_allows_another_parse() {
        let arena = FileArena::new();
        let scanner = arena.scanner("a.#private");
        let diagnostics = arena
            .try_build(|factory| {
                let mut parser = Parser::new(scanner, factory, ScriptKind::JS);
                parser.next_token();
                parser.parse_entity_name(true, false, None);
                Err(parser.diagnostics)
            })
            .err()
            .unwrap();
        assert_eq!(diagnostics.len(), 1);
        let next = parse_isolated_entity_name(&arena, "after").unwrap();
        assert_eq!(next.text(), "after");
        drop(arena);
        assert_eq!(diagnostics[0].code, diagnostics::Identifier_expected.code());
    }
}
