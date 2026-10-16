//! First native parser entry point, using the production parser's shared name grammar.
use crate::{Diagnostic, DiagnosticPhase, FileArena, Node, NodeFactory, Text, construction};
use std::fmt::Display;
use tsrs_ast::{Kind, NodeFlags};
use tsrs_core::{LanguageVariant, TextRange};
use tsrs_diagnostics::{self as diagnostics, Message};
use tsrs_parser::name_parser::NameParser;
use tsrs_scanner::Scanner;

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
    let scanner = arena.scanner(text);
    arena
        .try_build(|factory| {
            let mut parser = Parser::new(scanner, factory);
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

struct Parser<'build, 'ast: 'build> {
    scanner: Scanner<'ast, &'ast FileArena>,
    factory: NodeFactory<'build, 'ast>,
    token: Kind,
    context_flags: NodeFlags,
    statement_has_await_identifier: bool,
    has_parse_error: bool,
    diagnostics: Vec<Diagnostic>,
}

impl<'build, 'ast> Parser<'build, 'ast> {
    fn new(
        mut scanner: Scanner<'ast, &'ast FileArena>,
        factory: NodeFactory<'build, 'ast>,
    ) -> Self {
        // Isolated entity names use the JavaScript parser context, including its JSX scanner variant.
        scanner.set_on_error(true);
        scanner.set_language_variant(LanguageVariant::JSX);
        Self {
            scanner,
            factory,
            token: Kind::Unknown,
            context_flags: NodeFlags::JavaScriptFile,
            statement_has_await_identifier: false,
            has_parse_error: false,
            diagnostics: Vec::new(),
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

impl<'build, 'ast> NameParser<'ast> for Parser<'build, 'ast> {
    type Node = &'build construction::Node<'build, 'ast>;
    type Storage = &'ast FileArena;

    fn scanner(&self) -> &Scanner<'ast, &'ast FileArena> {
        &self.scanner
    }
    fn token(&self) -> Kind {
        self.token
    }
    fn context_flags(&self) -> NodeFlags {
        self.context_flags
    }
    fn statement_has_await_identifier(&self) -> bool {
        self.statement_has_await_identifier
    }
    fn set_statement_has_await_identifier(&mut self, value: bool) {
        self.statement_has_await_identifier = value;
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

    fn look_ahead_next_identifier_on_same_line(&mut self) -> bool {
        let state = self.scanner.mark();
        let diagnostics_len = self.diagnostics.len();
        let has_parse_error = self.has_parse_error;
        let result =
            self.next_token() >= Kind::Identifier && !self.scanner.has_preceding_line_break();
        self.scanner.rewind(state);
        self.token = self.scanner.token();
        self.diagnostics.truncate(diagnostics_len);
        self.has_parse_error = has_parse_error;
        // This lookahead only scans. It cannot change node state; arena bytes stay alive for 'ast.
        result
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

    fn finish_node(&mut self, node: Self::Node, pos: u32) -> Self::Node {
        node.set_loc(TextRange::new(pos, self.node_pos()));
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
            self.diagnostics.push(Diagnostic {
                phase: DiagnosticPhase::Parse,
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
            });
        }
        self.has_parse_error = true;
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
            let mut parser = Parser::new(scanner, factory);
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
            let mut parser = Parser::new(scanner, factory);
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
                    let mut parser = Parser::new(scanner, factory);
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
                let mut parser = Parser::new(scanner, factory);
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
                let mut parser = Parser::new(scanner, factory);
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
