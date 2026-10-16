use tsrs_ast::{Kind, NodeFlags, SourceFileParseOptions};
use tsrs_core::{P, ScriptKind, TextRange, arena::Region};
use tsrs_frontend_arena::{
    ContextParser, Diagnostic, FileArena, JSDocParser, NameParser, Node, NodeData, TokenParser,
};

fn compare(native: &Node<'_>, legacy: P<tsrs_ast::Node>) {
    assert_eq!(
        (native.kind, native.loc, native.flags),
        (legacy.kind(), legacy.loc(), legacy.flags())
    );
    if native.kind == Kind::Identifier {
        assert_eq!(native.text().as_bytes(), legacy.text().as_bytes());
    }
    let texts = match native.data {
        NodeData::JSDocLink(data) => Some((data.text, legacy.as_jsdoc_link().text())),
        NodeData::JSDocLinkCode(data) => Some((data.text, legacy.as_jsdoc_link_code().text())),
        NodeData::JSDocLinkPlain(data) => Some((data.text, legacy.as_jsdoc_link_plain().text())),
        _ => None,
    };
    if let Some((text, old)) = texts {
        assert_eq!(text.len(), old.len());
        for (part, old) in text.iter().zip(old) {
            assert_eq!(part.as_bytes(), old.as_bytes());
        }
    }
    let mut children = Vec::new();
    native.for_each_child(&mut |node| {
        children.push(node);
        false
    });
    let old = legacy.iter_children();
    assert_eq!(children.len(), old.len());
    for (child, old) in children.into_iter().zip(old) {
        assert!(std::ptr::eq(child.parent.unwrap(), native));
        compare(child, old);
    }
}

fn compare_diagnostics(native: &[Diagnostic], legacy: &[P<tsrs_ast::Diagnostic>]) {
    assert_eq!(native.len(), legacy.len());
    for (native, old) in native.iter().zip(legacy) {
        assert_eq!(
            (native.code, native.loc, native.category),
            (old.code(), old.loc(), old.category())
        );
        assert_eq!(native.message, old.message().unwrap().text());
        assert_eq!(native.arguments, old.message_args());
    }
}

fn find(node: P<tsrs_ast::Node>, kind: Kind) -> Option<P<tsrs_ast::Node>> {
    if node.kind() == kind {
        return Some(node);
    }
    node.iter_children()
        .into_iter()
        .find_map(|child| find(child, kind))
}

#[test]
fn links_match_source_file_jsdoc_parser() {
    for body in [
        "{@link Foo}",
        "{@linkcode A.B}",
        "{@linkplain π.名称 | display text}",
        "{@link Foo#member}",
        "{@link A.B#member}",
        "{@link Foo.#member}",
        "{@link Foo#member#next}",
        "{@link Foo | display text}",
        "{@link https://example.com}",
        "{@link }",
        "{@link 123}",
        "{@link Foo.}",
        "{@link Foo | text\nmore}",
        "{@link Foo | unterminated",
        r"{@link F\u006fo}",
        r"{@link \uD800}",
        "{@linkcode this.method}",
        "{@linkplain class}",
    ] {
        for script_kind in [ScriptKind::JS, ScriptKind::TS] {
            let source = format!("/** {body} */\nconst value = 0;");
            let end = source.find("*/").unwrap();
            let arena = FileArena::new();
            let mut diagnostics = Vec::new();
            let node = arena.build(|factory| {
                // The enclosing JSDoc comment parser supplies a source prefix ending before */.
                let mut parser = factory.parser(&source[..end], script_kind);
                parser.set_context_flags(NodeFlags::JSDoc, true);
                parser.reset_scanner_pos(4);
                parser.next_token_jsdoc();
                let node = parser.parse_jsdoc_link(4).unwrap();
                diagnostics = parser.into_diagnostics();
                node
            });
            let region = Region::new(4096);
            let _scope = region.enter();
            let file = tsrs_parser::parse_source_file(
                SourceFileParseOptions {
                    file_name: "/jsdoc.ts".into(),
                    ..Default::default()
                },
                &source,
                script_kind,
            );
            let doc = file.statements.nodes()[0].jsdoc(Some(file.get()))[0];
            compare(node, find(doc, node.kind).unwrap());
            if script_kind == ScriptKind::JS {
                // Complete comment parsing routes these parse errors into its JSDoc stream.
                compare_diagnostics(&diagnostics, file.jsdoc_diagnostics());
            }
        }
    }
}

#[test]
fn see_name_references_match_source_file_jsdoc_parser() {
    for name in [
        "Foo",
        "{Foo}",
        "A.B",
        "{A.B#member}",
        "Foo#member",
        "Foo.#member",
        "{Foo",
        "{π.名称}",
        r"F\u006fo",
        "{Foo.}",
    ] {
        let source = format!("/** @see {name} description */\nconst value = 0;");
        let end = source.find("*/").unwrap();
        let arena = FileArena::new();
        let mut diagnostics = Vec::new();
        let node = arena.build(|factory| {
            let mut parser = factory.parser(&source[..end], ScriptKind::JS);
            parser.set_context_flags(NodeFlags::JSDoc, true);
            parser.reset_scanner_pos(9);
            // @see switches to ordinary scanning before its optional name reference.
            parser.next_token();
            let node = parser.parse_jsdoc_name_reference();
            diagnostics = parser.into_diagnostics();
            node
        });
        let region = Region::new(4096);
        let _scope = region.enter();
        let file = tsrs_parser::parse_source_file(
            SourceFileParseOptions {
                file_name: "/jsdoc.js".into(),
                ..Default::default()
            },
            &source,
            ScriptKind::JS,
        );
        let doc = file.statements.nodes()[0].jsdoc(Some(file.get()))[0];
        compare(node, find(doc, Kind::JSDocNameReference).unwrap());
        compare_diagnostics(&diagnostics, file.jsdoc_diagnostics());
    }
}

#[test]
fn parameter_entity_names_match_source_file_jsdoc_parser() {
    for name in [
        "value",
        "π.名称",
        "items[].value",
        "items[].values[]",
        "a.b.c",
        "items[.value",
        "a.",
        "a..b",
        r"v\u0061lue",
    ] {
        let source = format!("/** @param {name} description */\nfunction f(value) {{}}");
        let end = source.find("*/").unwrap();
        let arena = FileArena::new();
        let mut diagnostics = Vec::new();
        let node = arena.build(|factory| {
            let mut parser = factory.parser(&source[..end], ScriptKind::JS);
            parser.set_context_flags(NodeFlags::JSDoc, true);
            parser.reset_scanner_pos(11);
            parser.next_token_jsdoc();
            let node = parser.parse_jsdoc_entity_name(Some(&tsrs_diagnostics::Identifier_expected));
            diagnostics = parser.into_diagnostics();
            node
        });
        let region = Region::new(4096);
        let _scope = region.enter();
        let file = tsrs_parser::parse_source_file(
            SourceFileParseOptions {
                file_name: "/jsdoc.js".into(),
                ..Default::default()
            },
            &source,
            ScriptKind::JS,
        );
        let doc = file.statements.nodes()[0].jsdoc(Some(file.get()))[0];
        let tag = find(doc, Kind::JSDocParameterTag).unwrap();
        compare(node, tag.name().unwrap());
        compare_diagnostics(&diagnostics, file.jsdoc_diagnostics());
    }
}

mod native {
    use super::*;

    #[test]
    fn failed_link_probe_restores_scanner_diagnostics_and_pending_error() {
        for source in [
            "{@unknown Foo}",
            "{ link Foo}",
            "\n * {@other Foo}",
            r"{@\u{110000}}",
        ] {
            let arena = FileArena::new();
            arena.build(|factory| {
                let mut parser = factory.parser(source, ScriptKind::TS);
                parser.set_context_flags(NodeFlags::JSDoc, true);
                parser.next_token_jsdoc();
                parser.parse_error_at(0, 0, &tsrs_diagnostics::Identifier_expected, &[]);
                let before = (
                    parser.token(),
                    parser.node_pos(),
                    parser.scanner().token_end(),
                    parser.context_flags(),
                );
                let errors = parser.diagnostics().to_vec();
                assert!(parser.parse_jsdoc_link(0).is_none());
                assert_eq!(
                    before,
                    (
                        parser.token(),
                        parser.node_pos(),
                        parser.scanner().token_end(),
                        parser.context_flags()
                    )
                );
                assert_eq!(parser.diagnostics(), errors);
                let id = parser.new_identifier("retained error");
                parser.finish_node_with_end(id, 0, 0);
                assert!(id.flags().intersects(NodeFlags::ThisNodeHasError));
                id
            });
        }
    }

    #[test]
    fn comment_whitespace_preserves_eof_and_indent_boundaries() {
        for source in [" \r\n \t", "\n", " "] {
            let arena = FileArena::new();
            arena.build(|factory| {
                let mut parser = factory.parser(source, ScriptKind::TS);
                parser.next_token_jsdoc();
                let before = (
                    parser.token(),
                    parser.node_pos(),
                    parser.scanner().token_end(),
                );
                parser.skip_whitespace();
                assert_eq!(
                    before,
                    (
                        parser.token(),
                        parser.node_pos(),
                        parser.scanner().token_end()
                    )
                );
                assert_eq!(parser.skip_whitespace_or_asterisk(), "");
                assert_eq!(
                    before,
                    (
                        parser.token(),
                        parser.node_pos(),
                        parser.scanner().token_end()
                    )
                );
                parser.factory().new_token(Kind::EndOfFile)
            });
        }
        for (source, indent, kind) in [
            ("\n *   name", " *   ", Kind::Identifier),
            ("  name", "", Kind::Identifier),
            ("*name", "", Kind::AsteriskToken),
        ] {
            let arena = FileArena::new();
            arena.build(|factory| {
                let mut parser = factory.parser(source, ScriptKind::TS);
                parser.next_token_jsdoc();
                assert_eq!(parser.skip_whitespace_or_asterisk(), indent);
                assert_eq!(parser.token(), kind);
                assert_eq!(parser.diagnostics(), []);
                parser.factory().new_token(Kind::EndOfFile)
            });
        }
    }

    #[test]
    fn jsdoc_token_helpers_keep_whitespace_and_explicit_node_end() {
        let arena = FileArena::new();
        arena.build(|factory| {
            let mut parser = factory.parser("name   } tail", ScriptKind::JS);
            parser.set_context_flags(NodeFlags::JSDoc, true);
            parser.next_token_jsdoc();
            let name = parser.parse_jsdoc_identifier_name(None);
            assert_eq!(name.loc(), TextRange::new(0, 4));
            assert_eq!(parser.token(), Kind::WhitespaceTrivia);
            parser.skip_whitespace();
            assert!(!parser.parse_optional_jsdoc(Kind::CommaToken));
            assert!(!parser.parse_expected_jsdoc(Kind::CommaToken));
            assert_eq!(parser.token(), Kind::CloseBraceToken);
            assert!(parser.parse_expected_jsdoc(Kind::CloseBraceToken));
            assert_eq!(parser.token(), Kind::WhitespaceTrivia);
            parser.skip_whitespace();
            let tail = parser.parse_jsdoc_identifier_name(None);
            assert!(tail.flags().intersects(NodeFlags::ThisNodeHasError));
            assert_eq!(tail.loc(), TextRange::new(9, 13));
            assert_eq!(parser.diagnostics().len(), 1);
            name
        });
    }

    #[test]
    fn speculative_links_survive_rewind_source_release_growth_and_threads() {
        let region = Region::new(4096);
        let scope = region.enter();
        let arena = FileArena::new();
        let root = {
            let source = String::from(r"{@linkcode F\u006fo#member | π description}");
            arena.build(|factory| {
                let mut parser = factory.parser(&source, ScriptKind::JS);
                parser.set_context_flags(NodeFlags::JSDoc, true);
                parser.next_token_jsdoc();
                let before = parser.mark();
                let retained = parser.parse_jsdoc_link(0).unwrap();
                assert_eq!(parser.token(), Kind::CloseBraceToken);
                let end = parser.scanner().token_end();
                assert_eq!(retained.loc(), TextRange::new(0, end));
                parser.rewind(before);
                assert_eq!(parser.token(), Kind::OpenBraceToken);
                let replayed = parser.parse_jsdoc_link(0).unwrap();
                assert!(!std::ptr::eq(retained, replayed));
                assert_eq!(retained.loc(), replayed.loc());
                assert_eq!(parser.diagnostics(), []);
                let list = parser.factory().new_node_list(&[retained, replayed]);
                let root = parser.factory().new_block(list, false);
                parser.finish_node_with_end(root, 0, end)
            })
        };
        assert_eq!(region.used_bytes(), 0);
        drop(scope);
        drop(region);
        arena.build(|factory| {
            for _ in 0..1024 {
                factory.new_identifier(factory.text(b"arena growth"));
            }
            factory.new_token(Kind::EndOfFile)
        });
        std::thread::scope(|scope| {
            for _ in 0..2 {
                scope.spawn(|| {
                    let NodeData::Block(block) = root.data else {
                        panic!()
                    };
                    for node in block.statements.nodes {
                        let NodeData::JSDocLinkCode(link) = node.data else {
                            panic!()
                        };
                        assert!(std::ptr::eq(node.parent.unwrap(), root));
                        let name = link.name.unwrap();
                        assert!(std::ptr::eq(name.parent.unwrap(), *node));
                        let NodeData::QualifiedName(name) = name.data else {
                            panic!()
                        };
                        assert_eq!(name.left.text().as_bytes(), b"Foo");
                        assert_eq!(name.right.text().as_bytes(), b"member");
                        let text: Vec<_> = link
                            .text
                            .iter()
                            .flat_map(|s| s.as_bytes())
                            .copied()
                            .collect();
                        assert_eq!(text, "| π description".as_bytes());
                    }
                });
            }
        });
    }
}
