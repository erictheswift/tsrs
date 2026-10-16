use tsrs_ast::{Kind, SourceFileParseOptions};
use tsrs_core::{P, ScriptKind, arena::Region};
use tsrs_frontend_arena::{
    ContextParser, FileArena, ListParser, NameParser, Node, NodeData, Parser, ParsingContext,
    TokenParser, construction,
};

// Exercise list recursion with already migrated leaf grammar; this is not a source-file parser.
fn array<'build, 'ast>(
    parser: &mut Parser<'build, 'ast>,
) -> &'build construction::Node<'build, 'ast> {
    let pos = parser.node_pos();
    parser.parse_expected(Kind::OpenBracketToken);
    let multi_line = parser.has_preceding_line_break();
    let elements = parser
        .parse_delimited_list(ParsingContext::ArrayLiteralMembers, |p| match p.token() {
            Kind::OpenBracketToken => array(p),
            Kind::CommaToken => p.finish_node(p.factory().new_omitted_expression(), p.node_pos()),
            _ => p.parse_literal_expression(),
        })
        .unwrap();
    parser.parse_expected(Kind::CloseBracketToken);
    parser.finish_node(
        parser
            .factory()
            .new_array_literal_expression(elements, multi_line),
        pos,
    )
}

fn compare(native: &Node<'_>, legacy: P<tsrs_ast::Node>) {
    assert_eq!(
        (native.kind, native.loc, native.flags),
        (legacy.kind(), legacy.loc(), legacy.flags())
    );
    if matches!(native.kind, Kind::StringLiteral | Kind::NumericLiteral) {
        assert_eq!(native.text().as_bytes(), legacy.text().as_bytes());
    }
    let lists = match native.data {
        NodeData::ArrayLiteralExpression(data) => {
            Some((data.elements, legacy.as_array_literal_expression().elements))
        }
        NodeData::TupleTypeNode(data) => {
            Some((data.elements, legacy.as_tuple_type_node().elements))
        }
        _ => None,
    };
    if let Some((native, legacy)) = lists {
        assert_eq!(native.loc, legacy.loc());
        assert_eq!(native.has_trailing_comma, legacy.has_trailing_comma());
        assert!(!native.is_missing);
    }
    let mut children = Vec::new();
    native.for_each_child(&mut |child| {
        children.push(child);
        false
    });
    let legacy_children = legacy.iter_children();
    assert_eq!(children.len(), legacy_children.len());
    for (child, old) in children.into_iter().zip(legacy_children) {
        assert!(std::ptr::eq(child.parent.unwrap(), native));
        compare(child, old);
    }
}

#[test]
fn lists_match_source_file_parser() {
    for source in [
        "[]",
        "[1, 2,]",
        "[, ,]",
        "[1 2]",
        "['a' 'b']",
        "[\n1,\n2\n]",
        "[[1, 2], [3,], [], ['\\ud800',],]",
        "type T = []",
        "type T = [1, 'x', true, false, null,]",
        "type T = [1 2]",
    ] {
        for script_kind in [ScriptKind::TS, ScriptKind::JS] {
            let is_type = source.starts_with("type");
            let arena = FileArena::new();
            let mut diagnostics = Vec::new();
            let root = arena.build(|factory| {
                let mut parser = factory.parser(source, script_kind);
                for _ in 0..if is_type { 4 } else { 1 } {
                    parser.next_token();
                }
                parser.replace_parsing_contexts(1 << ParsingContext::SourceElements as u32);
                let result = if is_type {
                    let pos = parser.node_pos();
                    let elements = parser
                        .parse_bracketed_list(
                            ParsingContext::TupleElementTypes,
                            |p| p.parse_literal_type_node(false),
                            Kind::OpenBracketToken,
                            Kind::CloseBracketToken,
                        )
                        .unwrap();
                    parser.finish_node(parser.factory().new_tuple_type_node(elements), pos)
                } else {
                    array(&mut parser)
                };
                assert_eq!(parser.token(), Kind::EndOfFile);
                assert!(parser.node_stack().is_empty());
                assert_eq!(
                    parser.parsing_contexts(),
                    1 << ParsingContext::SourceElements as u32
                );
                diagnostics = parser.into_diagnostics();
                result
            });
            let region = Region::new(4096);
            let _scope = region.enter();
            let file = tsrs_parser::parse_source_file(
                SourceFileParseOptions {
                    file_name: "/lists.ts".into(),
                    ..Default::default()
                },
                source,
                script_kind,
            );
            let statement = file.statements.nodes()[0];
            let legacy = if is_type {
                statement.type_node().unwrap()
            } else {
                statement.expression().unwrap()
            };
            compare(root, legacy);
            assert_eq!(diagnostics.len(), file.diagnostics().len(), "{source}");
            for (native, legacy) in diagnostics.iter().zip(file.diagnostics()) {
                assert_eq!(
                    (native.code, native.loc, native.category),
                    (legacy.code(), legacy.loc(), legacy.category()),
                    "{source}"
                );
                assert_eq!(native.arguments, legacy.message_args());
                assert_eq!(native.message, legacy.message().unwrap().text());
            }
        }
    }
}

#[test]
fn missing_parameter_lists_match_and_survive_publication() {
    for (source, missing) in [("function f {}", true), ("function f() {}", false)] {
        let arena = FileArena::new();
        let root = arena.build(|factory| {
            let mut parser = factory.parser(source, ScriptKind::TS);
            for _ in 0..3 {
                parser.next_token();
            }
            let list = parser
                .parse_bracketed_list(
                    ParsingContext::Parameters,
                    NameParser::parse_binding_identifier,
                    Kind::OpenParenToken,
                    Kind::CloseParenToken,
                )
                .unwrap();
            assert_eq!(Parser::is_missing_node_list(Some(list)), missing);
            assert!(!Parser::is_missing_node_list(None));
            assert_eq!(parser.diagnostics().len(), usize::from(missing));
            parser.factory().new_block(list, false)
        });
        let NodeData::Block(block) = root.data else {
            panic!("block")
        };
        assert_eq!(block.statements.is_missing, missing);
        let published_arena = FileArena::new();
        let published = tsrs_frontend_arena::parse_and_bind(
            &published_arena,
            source,
            tsrs_frontend_arena::ParseOptions::default(),
        );
        let declaration = published.root.source_file().unwrap().statements.nodes[0];
        let NodeData::FunctionDeclaration(function) = declaration.data else {
            panic!("function")
        };
        let parameters = function.parameters.unwrap();
        assert_eq!(parameters.is_missing, missing);
        assert_eq!(parameters.loc, block.statements.loc);
        assert!(parameters.nodes.is_empty());
    }
}

mod native {
    use super::*;

    #[test]
    fn speculative_lists_survive_rewind_growth_and_parser_drop() {
        let arena = FileArena::new();
        let region = Region::new(4096);
        let scope = region.enter();
        let root = arena.build(|factory| {
            let source = String::from("[1, ['\\ud800', 2,], 3,]");
            let mut parser = factory.parser(&source, ScriptKind::TS);
            drop(source);
            parser.next_token();
            let mut saved = None;
            assert!(parser.look_ahead(|p| {
                saved = Some(array(p));
                true
            }));
            assert!(parser.node_stack().is_empty());
            assert_eq!(parser.parsing_contexts(), 0);
            assert_eq!(parser.token(), Kind::OpenBracketToken);
            let saved = saved.unwrap();
            for _ in 0..if cfg!(miri) { 2 } else { 1024 } {
                parser.look_ahead(|p| {
                    array(p);
                    true
                });
            }
            let replay = array(&mut parser);
            assert!(!std::ptr::eq(saved, replay));
            assert!(parser.node_stack().is_empty());
            saved
        });
        assert_eq!(region.used_bytes(), 0);
        drop(scope);
        drop(region);
        let NodeData::ArrayLiteralExpression(data) = root.data else {
            panic!("array")
        };
        assert!(data.elements.has_trailing_comma);
        let NodeData::ArrayLiteralExpression(nested) = data.elements.nodes[1].data else {
            panic!("nested array")
        };
        assert!(nested.elements.has_trailing_comma);
        std::thread::scope(|s| {
            let first = s.spawn(|| data.elements.nodes[0].text());
            let nested = s.spawn(|| nested.elements.nodes[0].text());
            assert_eq!(first.join().unwrap(), "1");
            assert_eq!(nested.join().unwrap().as_bytes(), [0xed, 0xa0, 0x80]);
        });
    }

    #[test]
    fn failed_delimited_list_restores_outer_stack_and_context() {
        let arena = FileArena::new();
        arena.build(|factory| {
            let mut parser = factory.parser("first, second)", ScriptKind::TS);
            parser.next_token();
            let sentinel = parser
                .factory()
                .new_identifier(parser.factory().text(b"outer"));
            parser.node_stack_mut().push(sentinel);
            let outer = 1 << ParsingContext::SourceElements as u32;
            parser.replace_parsing_contexts(outer);
            let mut calls = 0;
            let mut retained = None;
            let result = parser.parse_delimited_list(ParsingContext::Parameters, |p| {
                calls += 1;
                let node = p.parse_binding_identifier();
                if calls == 1 {
                    retained = Some(node);
                    Some(node)
                } else {
                    None
                }
            });
            assert!(result.is_none());
            assert_eq!(calls, 2);
            assert_eq!(parser.parsing_contexts(), outer);
            assert_eq!(parser.node_stack().len(), 1);
            assert!(std::ptr::eq(parser.node_stack()[0], sentinel));
            assert_eq!(retained.unwrap().text(), "first");
            assert_eq!(parser.token(), Kind::CloseParenToken);
            retained.unwrap()
        });
    }

    #[test]
    fn recovery_preserves_enclosing_list_terminators() {
        let arena = FileArena::new();
        arena.build(|factory| {
            let mut parser = factory.parser("[1, }", ScriptKind::TS);
            parser.next_token();
            parser.parse_expected(Kind::OpenBracketToken);
            let outer = 1 << ParsingContext::ObjectLiteralMembers as u32;
            parser.replace_parsing_contexts(outer);
            let list = parser
                .parse_delimited_list(ParsingContext::ArrayLiteralMembers, |p| {
                    p.parse_literal_expression()
                })
                .unwrap();
            assert_eq!(list.nodes().len(), 1);
            assert!(list.has_trailing_comma());
            assert_eq!(parser.token(), Kind::CloseBraceToken);
            assert_eq!(parser.parsing_contexts(), outer);
            assert!(parser.node_stack().is_empty());
            assert_eq!(
                parser.diagnostics()[0].code,
                tsrs_diagnostics::Expression_or_comma_expected.code()
            );
            parser.factory().new_array_literal_expression(list, false)
        });
    }

    #[test]
    fn semicolon_recovery_and_zero_width_elements_make_progress() {
        for (source, context) in [
            ("a; b}", ParsingContext::ObjectLiteralMembers),
            ("a; b}", ParsingContext::ImportAttributes),
            ("a b}", ParsingContext::EnumMembers),
        ] {
            let arena = FileArena::new();
            arena.build(|factory| {
                let mut parser = factory.parser(source, ScriptKind::TS);
                parser.next_token();
                let list = parser
                    .parse_delimited_list(context, NameParser::parse_identifier_name)
                    .unwrap();
                assert_eq!(list.nodes().len(), 2);
                assert_eq!(parser.diagnostics().len(), 1);
                assert_eq!(parser.token(), Kind::CloseBraceToken);
                assert_eq!(parser.parsing_contexts(), 0);
                parser.factory().new_block(list, false)
            });
        }
        let arena = FileArena::new();
        arena.build(|factory| {
            let mut parser = factory.parser("a b)", ScriptKind::TS);
            parser.next_token();
            let list = parser
                .parse_delimited_list(ParsingContext::Parameters, |p| {
                    p.finish_node(
                        p.factory()
                            .new_identifier(tsrs_frontend_arena::Text::default()),
                        p.node_pos(),
                    )
                })
                .unwrap();
            assert_eq!(list.nodes().len(), 2);
            assert_eq!(parser.token(), Kind::CloseParenToken);
            assert_eq!(parser.diagnostics().len(), 2);
            parser.factory().new_block(list, false)
        });
    }

    #[test]
    fn reparsed_declarations_propagate_to_statement_lists_in_order() {
        let arena = FileArena::new();
        arena.build(|factory| {
            let mut parser = factory.parser("member }", ScriptKind::JS);
            parser.next_token();
            let mut nested = None;
            let statements = parser.parse_list(ParsingContext::BlockStatements, |p| {
                let members = p.parse_list(ParsingContext::TypeMembers, |p| {
                    let name = p.parse_identifier();
                    let alias = p
                        .factory()
                        .new_js_type_alias_declaration(None, name, None, None);
                    let import = p
                        .factory()
                        .new_js_import_declaration(None, None, name, None);
                    let extra = p.factory().new_empty_statement();
                    p.reparse_list().extend([alias, import, extra]);
                    name
                });
                nested = Some(members);
                p.factory().new_type_literal_node(members)
            });
            assert_eq!(
                statements
                    .nodes()
                    .iter()
                    .map(|n| n.kind())
                    .collect::<Vec<_>>(),
                [
                    Kind::JSTypeAliasDeclaration,
                    Kind::JSImportDeclaration,
                    Kind::TypeLiteral
                ]
            );
            assert_eq!(
                nested
                    .unwrap()
                    .nodes()
                    .iter()
                    .map(|n| n.kind())
                    .collect::<Vec<_>>(),
                [Kind::EmptyStatement, Kind::Identifier]
            );
            assert!(parser.reparse_list().is_empty());
            assert!(parser.node_stack().is_empty());
            assert_eq!(parser.parsing_contexts(), 0);
            assert_eq!(parser.token(), Kind::CloseBraceToken);
            parser.factory().new_block(statements, false)
        });
    }

    #[test]
    fn indexed_lists_include_reparsed_elements_in_following_indexes() {
        let arena = FileArena::new();
        arena.build(|factory| {
            let mut parser = factory.parser("a b", ScriptKind::TS);
            parser.next_token();
            let mut indexes = Vec::new();
            let nodes = parser.parse_list_index(ParsingContext::SourceElements, |p, index| {
                indexes.push(index);
                if index == 0 {
                    let extra = p.factory().new_empty_statement();
                    p.reparse_list().push(extra);
                }
                p.parse_identifier()
            });
            assert_eq!(indexes, [0, 2]);
            assert_eq!(nodes.len(), 3);
            assert!(parser.node_stack().is_empty());
            parser
                .factory()
                .new_block(parser.factory().new_node_list(&nodes), false)
        });
    }
}
