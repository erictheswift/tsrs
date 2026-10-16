use tsrs_ast::{Kind, NodeFlags, SourceFileParseOptions};
use tsrs_core::{P, ScriptKind, TextRange, arena::Region};
use tsrs_frontend_arena::{
    ContextParser, Diagnostic, FileArena, JsxParser, Node, NodeData, TokenParser,
};

fn compare(native: &Node<'_>, legacy: P<tsrs_ast::Node>) {
    assert_eq!(
        (native.kind, native.loc, native.flags),
        (legacy.kind(), legacy.loc(), legacy.flags())
    );
    match native.data {
        NodeData::Identifier(_) => assert_eq!(native.text().as_bytes(), legacy.text().as_bytes()),
        NodeData::StringLiteral(data) => {
            assert_eq!(data.text.as_bytes(), legacy.text().as_bytes());
            assert_eq!(data.token_flags, legacy.as_string_literal().token_flags());
        }
        NodeData::JsxText(data) => {
            let old = legacy.as_jsx_text();
            assert_eq!(data.text.as_bytes(), old.text().as_bytes());
            assert_eq!(
                data.contains_only_trivia_white_spaces,
                old.contains_only_trivia_white_spaces
            );
        }
        NodeData::PropertyAccessExpression(data) => {
            assert!(data.question_dot_token.is_none());
            assert!(
                legacy
                    .as_property_access_expression()
                    .question_dot_token()
                    .is_none()
            );
        }
        _ => {}
    }
    let mut children = Vec::new();
    native.for_each_child(&mut |node| {
        children.push(node);
        false
    });
    let old = legacy.iter_children();
    assert_eq!(children.len(), old.len());
    for (node, old) in children.into_iter().zip(old) {
        assert!(std::ptr::eq(node.parent.unwrap(), native));
        compare(node, old);
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

#[test]
fn element_names_match_source_file_parser() {
    for name in [
        "div",
        "custom-element",
        "this",
        "this.Component",
        "A.B.C",
        "svg:path",
        "this:tag",
        "class",
        "await",
        "π.名称",
        "ns:data-name",
        "A.",
        "ns:",
        r"\u0061",
        r"A.\u0062",
        r"ns:\u0062",
        "A.#private",
    ] {
        for script_kind in [ScriptKind::TSX, ScriptKind::JSX] {
            let source = format!("<{name}/>;");
            let arena = FileArena::new();
            let mut diagnostics = Vec::new();
            let node = arena.build(|factory| {
                let mut parser = factory.parser(&source, script_kind);
                parser.next_token();
                parser.parse_expected(Kind::LessThanToken);
                let name = parser.parse_jsx_element_name();
                assert_eq!(parser.token(), Kind::SlashToken, "{source}");
                diagnostics = parser.into_diagnostics();
                name
            });
            let region = Region::new(4096);
            let _scope = region.enter();
            let file = tsrs_parser::parse_source_file(
                SourceFileParseOptions {
                    file_name: "/jsx.tsx".into(),
                    ..Default::default()
                },
                &source,
                script_kind,
            );
            compare(
                node,
                file.statements.nodes()[0].expression().unwrap().tag_name(),
            );
            compare_diagnostics(&diagnostics, file.diagnostics());
        }
    }
}

#[test]
fn attribute_names_and_string_values_match_source_file_parser() {
    for attribute in [
        "disabled",
        "data-id='value'",
        "xml:lang='en'",
        "this='keyword'",
        "class='a'",
        "π:名称='π'",
        "a='&amp;&#x1f680;&#xD800;'",
        "a='line\nbreak'",
        "a='\\ud800'",
        r"\u0061='escape'",
        r"ns:\u0062='escape'",
        "ns:='missing'",
    ] {
        for script_kind in [ScriptKind::TSX, ScriptKind::JSX] {
            let source = format!("<div {attribute}/>;");
            let arena = FileArena::new();
            let mut diagnostics = Vec::new();
            let node = arena.build(|factory| {
                let mut parser = factory.parser(&source, script_kind);
                parser.next_token();
                parser.parse_expected(Kind::LessThanToken);
                parser.parse_jsx_element_name();
                let pos = parser.node_pos();
                let name = parser.parse_jsx_attribute_name();
                let initializer = if parser.token() == Kind::EqualsToken {
                    assert_eq!(parser.scan_jsx_attribute_value(), Kind::StringLiteral);
                    Some(parser.parse_literal_expression())
                } else {
                    None
                };
                let node = parser.factory().new_jsx_attribute(name, initializer);
                parser.finish_node(node, pos);
                assert_eq!(parser.token(), Kind::SlashToken, "{source}");
                diagnostics = parser.into_diagnostics();
                node
            });
            let region = Region::new(4096);
            let _scope = region.enter();
            let file = tsrs_parser::parse_source_file(
                SourceFileParseOptions {
                    file_name: "/jsx.tsx".into(),
                    ..Default::default()
                },
                &source,
                script_kind,
            );
            let element = file.statements.nodes()[0]
                .expression()
                .unwrap()
                .as_jsx_self_closing_element();
            compare(
                node,
                element.attributes.as_jsx_attributes().properties.nodes()[0],
            );
            compare_diagnostics(&diagnostics, file.diagnostics());
        }
    }
}

#[test]
fn text_and_closing_elements_match_source_file_parser() {
    for (name, text) in [
        ("div", ""),
        ("div", "hello &amp; π"),
        ("custom-element", "  \r\n  "),
        ("A.B.C", " > } "),
        ("svg:path", "&#xD800;"),
        ("this.Component", " "),
        ("this", "text"),
        ("this:tag", "#text"),
    ] {
        for script_kind in [ScriptKind::TSX, ScriptKind::JSX] {
            let source = format!("<{name}>{text}</{name}>;");
            let arena = FileArena::new();
            let mut diagnostics = Vec::new();
            let node = arena.build(|factory| {
                let mut parser = factory.parser(&source, script_kind);
                parser.next_token();
                let pos = parser.node_pos();
                parser.parse_expected(Kind::LessThanToken);
                let tag = parser.parse_jsx_element_name();
                let attributes_pos = parser.node_pos();
                let list = parser.factory().alloc_node_list(
                    TextRange::new(attributes_pos, attributes_pos),
                    &[],
                    false,
                    false,
                );
                let attrs = parser.factory().new_jsx_attributes(list);
                parser.finish_node(attrs, attributes_pos);
                assert!(parser.parse_expected_without_advancing(Kind::GreaterThanToken));
                parser.scan_jsx_text();
                let opening = parser.factory().new_jsx_opening_element(tag, None, attrs);
                parser.finish_node(opening, pos);
                let children_pos = parser.node_pos();
                let child_token = parser.re_scan_jsx_token(true);
                let children = if matches!(child_token, Kind::JsxText | Kind::JsxTextAllWhiteSpaces)
                {
                    let text = parser.parse_jsx_text();
                    assert_eq!(parser.re_scan_jsx_token(true), Kind::LessThanSlashToken);
                    vec![text]
                } else {
                    Vec::new()
                };
                let list = parser.factory().alloc_node_list(
                    TextRange::new(children_pos, parser.node_pos()),
                    parser.factory().nodes(&children),
                    false,
                    false,
                );
                let closing = parser.parse_jsx_closing_element(opening, true);
                let node = parser.factory().new_jsx_element(opening, list, closing);
                parser.finish_node(node, pos);
                assert_eq!(parser.token(), Kind::SemicolonToken);
                diagnostics = parser.into_diagnostics();
                node
            });
            let region = Region::new(4096);
            let _scope = region.enter();
            let file = tsrs_parser::parse_source_file(
                SourceFileParseOptions {
                    file_name: "/jsx.tsx".into(),
                    ..Default::default()
                },
                &source,
                script_kind,
            );
            compare(node, file.statements.nodes()[0].expression().unwrap());
            compare_diagnostics(&diagnostics, file.diagnostics());
        }
    }
}

#[test]
fn closing_fragments_match_source_file_parser() {
    for source in ["<></>;", "<>text</>;", "<> \n </>;", "<></wrong>;"] {
        let arena = FileArena::new();
        let mut diagnostics = Vec::new();
        let node = arena.build(|factory| {
            let mut parser = factory.parser(source, ScriptKind::TSX);
            parser.next_token();
            parser.parse_expected(Kind::LessThanToken);
            assert!(parser.parse_expected_without_advancing(Kind::GreaterThanToken));
            parser.scan_jsx_text();
            if matches!(parser.token(), Kind::JsxText | Kind::JsxTextAllWhiteSpaces) {
                parser.parse_jsx_text();
            }
            let node = parser.parse_jsx_closing_fragment(true);
            diagnostics = parser.into_diagnostics();
            node
        });
        let region = Region::new(4096);
        let _scope = region.enter();
        let file = tsrs_parser::parse_source_file(
            SourceFileParseOptions {
                file_name: "/jsx.tsx".into(),
                ..Default::default()
            },
            source,
            ScriptKind::TSX,
        );
        compare(
            node,
            file.statements.nodes()[0]
                .expression()
                .unwrap()
                .as_jsx_fragment()
                .closing_fragment,
        );
        // Recovery beyond the closing fragment belongs to the enclosing expression parser.
        compare_diagnostics(&diagnostics, &file.diagnostics()[..diagnostics.len()]);
    }
}

mod native {
    use super::*;
    use tsrs_frontend_arena::{ListParser, NameParser, Parser};

    #[test]
    fn closing_tag_scanner_transitions_follow_name_equivalence() {
        for (left, right, equivalent) in [
            ("div", "div", true),
            ("div", "span", false),
            ("this", "this", true),
            ("this", "other", false),
            ("A.B.C", "A.B.C", true),
            ("A.B", "A.C", false),
            ("A.B", "X.B", false),
            ("A.B", "A", false),
            ("a:b", "a:b", true),
            ("a:b", "a:c", false),
            ("a:b", "x:b", false),
            ("this.B", "this.B", true),
        ] {
            for in_expression_context in [false, true] {
                let source = format!("{left}</{right}> tail");
                let arena = FileArena::new();
                arena.build(|factory| {
                    let mut parser = factory.parser(&source, ScriptKind::TSX);
                    parser.next_token();
                    let tag = parser.parse_jsx_element_name();
                    let list =
                        parser
                            .factory()
                            .alloc_node_list(TextRange::new(0, 0), &[], false, false);
                    let attrs = parser.factory().new_jsx_attributes(list);
                    let open = parser.factory().new_jsx_opening_element(tag, None, attrs);
                    let close = parser.parse_jsx_closing_element(open, in_expression_context);
                    assert_eq!(
                        Parser::tag_names_are_equivalent(tag, Parser::jsx_tag_name(close)),
                        equivalent
                    );
                    assert_eq!(
                        parser.token(),
                        if equivalent && !in_expression_context {
                            Kind::JsxText
                        } else {
                            Kind::Identifier
                        }
                    );
                    assert_eq!(parser.diagnostics(), []);
                    close
                });
            }
        }
        for in_expression_context in [false, true] {
            let arena = FileArena::new();
            arena.build(|factory| {
                let mut parser = factory.parser("</> tail", ScriptKind::TSX);
                parser.next_token();
                let close = parser.parse_jsx_closing_fragment(in_expression_context);
                assert_eq!(
                    parser.token(),
                    if in_expression_context {
                        Kind::Identifier
                    } else {
                        Kind::JsxText
                    }
                );
                assert_eq!(parser.diagnostics(), []);
                close
            });
        }
    }

    #[test]
    fn namespaced_names_stop_before_property_access() {
        let arena = FileArena::new();
        arena.build(|factory| {
            let mut parser = factory.parser("ns:tag.member", ScriptKind::TSX);
            parser.next_token();
            let node = parser.parse_jsx_element_name();
            assert_eq!(node.kind(), Kind::JsxNamespacedName);
            assert_eq!(parser.token(), Kind::DotToken);
            assert_eq!(parser.diagnostics(), []);
            node
        });
    }

    #[test]
    fn speculative_jsx_text_survives_rewind_growth_and_source_release() {
        let region = Region::new(4096);
        let scope = region.enter();
        let arena = FileArena::new();
        let mut diagnostics = Vec::new();
        let root = {
            let source = String::from("<view> π > } text </view>");
            arena.build(|factory| {
                let mut parser = factory.parser(&source, ScriptKind::TSX);
                parser.next_token();
                parser.parse_expected(Kind::LessThanToken);
                parser.parse_jsx_element_name();
                let checkpoint = parser.mark();
                assert_eq!(parser.token(), Kind::GreaterThanToken);
                parser.scan_jsx_text();
                let retained = parser.parse_jsx_text();
                assert_eq!(parser.diagnostics().len(), 2);
                assert!(retained.flags().intersects(NodeFlags::ThisNodeHasError));
                parser.rewind(checkpoint);
                assert_eq!(parser.token(), Kind::GreaterThanToken);
                assert_eq!(parser.diagnostics(), []);
                parser.scan_jsx_text();
                let replayed = parser.parse_jsx_text();
                assert!(!std::ptr::eq(retained, replayed));
                assert_eq!(retained.loc(), replayed.loc());
                assert_eq!(retained.flags(), replayed.flags());
                assert_eq!(retained.text(), replayed.text());
                let nodes = parser.factory().nodes(&[retained, replayed]);
                let list = parser.factory().alloc_node_list(
                    TextRange::new(0, parser.node_pos()),
                    nodes,
                    false,
                    false,
                );
                let root = parser.factory().new_block(list, false);
                parser.finish_node(root, 0);
                diagnostics = parser.into_diagnostics();
                root
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
                    for child in block.statements.nodes {
                        assert_eq!(child.text().as_bytes(), " π > } text ".as_bytes());
                        assert!(std::ptr::eq(child.parent.unwrap(), root));
                    }
                });
            }
        });
        drop(arena);
        assert_eq!(diagnostics.len(), 2);
        assert!(diagnostics.iter().all(|d| !d.message.is_empty()));
    }

    #[test]
    fn jsx_scanner_errors_restore_pending_error_state() {
        let arena = FileArena::new();
        arena.build(|factory| {
            let mut parser = factory.parser("a='unterminated", ScriptKind::TSX);
            parser.next_token();
            let name = parser.parse_jsx_attribute_name();
            let checkpoint = parser.mark();
            assert_eq!(parser.scan_jsx_attribute_value(), Kind::StringLiteral);
            let bad = parser.parse_literal_expression();
            assert_eq!(parser.diagnostics().len(), 1);
            assert!(bad.flags().intersects(NodeFlags::ThisNodeHasError));
            parser.rewind(checkpoint);
            assert_eq!(parser.token(), Kind::EqualsToken);
            assert_eq!(parser.diagnostics(), []);
            let clean = parser.new_identifier("clean");
            parser.finish_node(clean, 0);
            assert!(!clean.flags().intersects(NodeFlags::ThisNodeHasError));
            assert!(parser.node_stack().is_empty());
            parser.factory().new_jsx_attribute(name, Some(bad))
        });
    }
}
