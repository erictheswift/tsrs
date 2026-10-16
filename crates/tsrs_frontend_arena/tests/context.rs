use tsrs_ast::{Kind, NodeFlags, SourceFileParseOptions};
use tsrs_core::{ScriptKind, TextRange, arena::Region};
use tsrs_frontend_arena::{
    ContextParser, FileArena, LookaheadParser, NameParser, NodeData, TokenParser,
};

#[test]
fn modifiers_match_source_file_parser() {
    for (source, in_class) in [
        ("export default async function f() {}", false),
        ("export default abstract class C {}", false),
        ("export declare abstract class C {}", false),
        ("export type T = number;", false),
        ("const enum E {}", false),
        ("class C { public static readonly x = 1; }", true),
        ("class C { static static() {} }", true),
        ("class C { private async *method() {} }", true),
        ("class C { protected accessor value = 1; }", true),
        ("class C { static { } }", true),
    ] {
        for script_kind in [ScriptKind::TS, ScriptKind::JS] {
            let arena = FileArena::new();
            let mut modifiers = Vec::new();
            arena.build(|factory| {
                let mut parser = factory.parser(source, script_kind);
                for _ in 0..if in_class { 4 } else { 1 } {
                    parser.next_token();
                }
                // parse_declaration enters Ambient before consuming a declared modifier list.
                if source.contains("declare") {
                    parser.set_context_flags(NodeFlags::Ambient, true);
                }
                let mut has_static = false;
                let mut first = None;
                while let Some(node) = parser.try_parse_modifier(has_static, false, in_class) {
                    modifiers.push((node.kind(), node.loc(), node.flags()));
                    has_static |= node.kind() == Kind::StaticKeyword;
                    first = first.or(Some(node));
                }
                assert_eq!(parser.diagnostics(), [], "{source}");
                first.unwrap_or_else(|| parser.parse_token_node())
            });
            let region = Region::new(4096);
            let _scope = region.enter();
            let file = tsrs_parser::parse_source_file(
                SourceFileParseOptions {
                    file_name: "/modifiers.ts".to_owned(),
                    ..Default::default()
                },
                source,
                script_kind,
            );
            let statement = file.statements.nodes()[0];
            let declaration = if in_class {
                statement.members()[0]
            } else {
                statement
            };
            let legacy = declaration.modifiers().map_or_else(Vec::new, |list| {
                list.nodes()
                    .iter()
                    .map(|node| (node.kind(), node.loc(), node.flags()))
                    .collect()
            });
            assert_eq!(modifiers, legacy, "{source}");
        }
    }
}

mod native {
    use super::*;

    #[test]
    fn speculative_nodes_and_cooked_values_survive_rewind_and_growth() {
        let arena = FileArena::new();
        let region = Region::new(4096);
        let scope = region.enter();
        let mut cooked = None;
        let root = arena.build(|factory| {
            let source = String::from(r"await.\u004eame 1_000 '\ud800'");
            let mut parser = factory.parser(&source, ScriptKind::TS);
            drop(source);
            parser.next_token();
            let start_used = arena.used_bytes();
            let mut saved = None;
            assert!(parser.look_ahead(|p| {
                saved = Some(p.parse_entity_name(true, false, None));
                assert!(p.statement_has_await_identifier());
                p.set_context_flags(NodeFlags::YieldContext, true);
                let state = p.mark();
                let number = p.parse_literal_expression();
                let string = p.parse_literal_expression();
                cooked = Some(string.text());
                p.rewind(state);
                assert_eq!(p.token(), Kind::NumericLiteral);
                assert_eq!(number.text(), "1000");
                assert_eq!(string.text().as_bytes(), [0xed, 0xa0, 0x80]);
                let replay = p.parse_literal_expression();
                assert!(!std::ptr::eq(number, replay));
                assert_eq!(number.text(), replay.text());
                true
            }));
            assert_eq!(parser.token(), Kind::AwaitKeyword);
            assert_eq!(parser.node_pos(), 0);
            assert_eq!(parser.context_flags(), NodeFlags::None);
            assert!(!parser.statement_has_await_identifier());
            assert_eq!(parser.diagnostics(), []);
            assert!(arena.used_bytes() > start_used);
            let old = saved.unwrap();
            let new = parser.parse_entity_name(true, false, None);
            assert!(!std::ptr::eq(old, new));
            // Allocate enough subsequent syntax to cross arena chunks while the speculative graph is live.
            for _ in 0..if cfg!(miri) { 4 } else { 2048 } {
                let checkpoint = parser.mark();
                parser.parse_literal_expression();
                parser.rewind(checkpoint);
            }
            assert_eq!(old.as_qualified_name().right().text(), "Name");
            old
        });
        assert_eq!(region.used_bytes(), 0);
        drop(scope);
        drop(region);
        let NodeData::QualifiedName(name) = root.data else {
            panic!("qualified name")
        };
        assert!(std::ptr::eq(name.left.parent.unwrap(), root));
        assert!(std::ptr::eq(name.right.parent.unwrap(), root));
        std::thread::scope(|scope| {
            let left = scope.spawn(|| name.left.text());
            let right = scope.spawn(|| name.right.text());
            assert_eq!(left.join().unwrap(), "await");
            assert_eq!(right.join().unwrap(), "Name");
        });
        assert_eq!(cooked.unwrap().as_bytes(), [0xed, 0xa0, 0x80]);
    }

    #[test]
    fn nested_lookahead_restores_directives_and_replays_errors_once() {
        let arena = FileArena::new();
        let mut diagnostics = Vec::new();
        let root = arena.build(|factory| {
            let mut parser = factory.parser("x // @ts-ignore\n '\\u00zz' /*", ScriptKind::TS);
            parser.next_token();
            for _ in 0..2 {
                assert!(parser.look_ahead(|p| {
                    p.next_token();
                    assert_eq!(p.scanner().comment_directives().len(), 1);
                    assert_eq!(p.diagnostics().len(), 1);
                    let first_error = p.diagnostics()[0].clone();
                    assert!(!p.look_ahead(|p| {
                        p.parse_literal_expression();
                        assert_eq!(p.diagnostics().len(), 2);
                        p.set_context_flags(NodeFlags::AwaitContext, true);
                        false
                    }));
                    assert_eq!(p.token(), Kind::StringLiteral);
                    assert_eq!(p.context_flags(), NodeFlags::None);
                    assert_eq!(p.diagnostics(), [first_error]);
                    p.parse_literal_expression();
                    assert_eq!(p.diagnostics().len(), 2);
                    true
                }));
                assert_eq!(parser.token(), Kind::Identifier);
                assert_eq!(parser.scanner().comment_directives(), []);
                assert_eq!(parser.diagnostics(), []);
            }
            parser.parse_identifier();
            let node = parser.parse_literal_expression();
            assert_eq!(parser.scanner().comment_directives().len(), 1);
            assert_eq!(parser.diagnostics().len(), 2);
            assert!(node.flags().intersects(NodeFlags::ThisNodeHasError));
            diagnostics = parser.into_diagnostics();
            node
        });
        assert_eq!(root.kind, Kind::StringLiteral);
        drop(arena);
        assert_eq!(
            diagnostics[0].code,
            tsrs_diagnostics::Hexadecimal_digit_expected.code()
        );
        assert_eq!(
            diagnostics[1].code,
            tsrs_diagnostics::Asterisk_Slash_expected.code()
        );
    }

    #[test]
    fn rewind_restores_an_error_pending_before_the_checkpoint() {
        let arena = FileArena::new();
        let root = arena.build(|factory| {
            let mut parser = factory.parser(r"'\u00zz' next", ScriptKind::TS);
            parser.next_token();
            let error = parser.diagnostics()[0].clone();
            assert!(parser.look_ahead(|p| {
                let speculative = p.parse_literal_expression();
                assert!(speculative.flags().intersects(NodeFlags::ThisNodeHasError));
                p.parse_identifier();
                true
            }));
            assert_eq!(parser.diagnostics(), [error]);
            parser.parse_literal_expression()
        });
        assert!(root.flags.intersects(NodeFlags::ThisNodeHasError));
    }

    #[test]
    fn nested_contexts_restore_flags_but_keep_parser_results() {
        let arena = FileArena::new();
        let root = arena.build(|factory| {
            let mut parser = factory.parser("await yield", ScriptKind::JS);
            parser.next_token();
            parser.set_context_flags(NodeFlags::AwaitContext | NodeFlags::DisallowInContext, true);
            assert!(parser.in_await_context());
            assert!(parser.in_disallow_in_context());
            let initial = parser.context_flags();
            let node = parser.do_in_context(NodeFlags::AwaitContext, false, |p| {
                assert!(!p.in_await_context());
                let outer = p.context_flags();
                p.do_in_context(
                    NodeFlags::YieldContext
                        | NodeFlags::DecoratorContext
                        | NodeFlags::DisallowConditionalTypesContext,
                    true,
                    |p| {
                        assert!(p.in_yield_context());
                        assert!(p.in_decorator_context());
                        assert!(p.in_disallow_conditional_types_context());
                    },
                );
                assert_eq!(p.context_flags(), outer);
                p.parse_identifier()
            });
            assert_eq!(parser.context_flags(), initial);
            assert_eq!(parser.token(), Kind::YieldKeyword);
            // do_in_context restores flags, not ordinary parsing progress or the await-name marker.
            assert!(parser.statement_has_await_identifier());
            assert!(!node.flags().intersects(NodeFlags::AwaitContext));
            assert!(node.flags().intersects(NodeFlags::JavaScriptFile));
            node
        });
        assert_eq!(root.text(), "await");
    }

    #[test]
    fn contextual_modifiers_commit_only_on_a_valid_continuation() {
        for (source, expected, contextual) in [
            ("async function", true, true),
            ("async\nfunction", false, false),
            ("static\nname", true, true),
            ("export default async function", true, true),
            ("export default async\nfunction", false, false),
            ("export type T", true, true),
            ("export type {", false, false),
            ("export *", false, false),
            ("const enum", true, true),
            ("const x", false, false),
            ("get [", false, true),
            ("get (", false, false),
            ("public '\\u00zz'", true, true),
            ("public /*", false, false),
            ("name other", false, false),
        ] {
            let arena = FileArena::new();
            arena.build(|factory| {
                let mut parser = factory.parser(source, ScriptKind::TS);
                let kind = parser.next_token();
                let range = parser.scanner().token_range();
                let checkpoint = parser.mark();
                assert_eq!(parser.parse_any_contextual_modifier(), expected, "{source}");
                if !expected {
                    assert_eq!(parser.token(), kind);
                    assert_eq!(parser.scanner().token_range(), range);
                    assert_eq!(parser.diagnostics(), []);
                }
                parser.rewind(checkpoint);
                assert!(!parser.parse_contextual_modifier(Kind::ReadonlyKeyword));
                assert_eq!(parser.scanner().token_range(), range);
                if tsrs_ast::is_modifier_kind(kind)
                    || matches!(kind, Kind::GetKeyword | Kind::SetKeyword)
                {
                    assert_eq!(
                        parser.parse_contextual_modifier(kind),
                        contextual,
                        "{source}"
                    );
                }
                parser.rewind(checkpoint);
                parser.parse_token_node()
            });
        }
    }

    #[test]
    fn modifier_options_stop_static_blocks_duplicates_and_standalone_const() {
        for (source, seen_static, permit_const, stop_static_block, expected) in [
            ("const T", false, true, false, true),
            ("const\nT", false, true, false, false),
            ("const T", false, false, false, false),
            ("static {}", false, false, true, false),
            ("static {}", false, false, false, true),
            ("static name", true, false, false, false),
        ] {
            let arena = FileArena::new();
            arena.build(|factory| {
                let mut parser = factory.parser(source, ScriptKind::TS);
                let kind = parser.next_token();
                let node = parser.try_parse_modifier(seen_static, permit_const, stop_static_block);
                assert_eq!(node.is_some(), expected, "{source}");
                if let Some(node) = node {
                    assert_eq!(node.kind(), kind);
                    assert_eq!(node.loc().source_pos(), Some(0));
                    node
                } else {
                    assert_eq!(parser.token(), kind);
                    assert_eq!(parser.diagnostics(), []);
                    parser.parse_token_node()
                }
            });
        }
    }

    #[test]
    fn syntax_decisions_restore_nested_scanner_lookahead() {
        for (source, expected) in [
            ("const x = 1", [false, false, true, true]),
            ("class C {}", [false, true, true, true]),
            ("interface I {}", [true, true, true, true]),
            ("interface\nI {}", [true, true, true, false]),
            ("import('x')", [true, true, true, false]),
            ("import x from 'm'", [true, false, true, true]),
            ("-1", [true, true, true, false]),
            ("-x", [false, true, true, false]),
            ("if (x)", [false, false, true, false]),
            ("}", [false, false, false, false]),
            ("() => T", [true, true, true, false]),
            (":", [false, false, false, false]),
            ("using {x} = y", [true, true, true, true]),
            ("using[x]", [true, true, true, false]),
            ("await using x = y", [true, true, true, true]),
        ] {
            let arena = FileArena::new();
            arena.build(|factory| {
                let mut parser = factory.parser(source, ScriptKind::TS);
                let token = parser.next_token();
                let range = parser.scanner().token_range();
                let actual = [
                    parser.is_start_of_type(false),
                    parser.is_start_of_expression(),
                    parser.is_start_of_statement(),
                    parser.is_start_of_declaration(),
                ];
                assert_eq!(actual, expected, "{source}");
                assert_eq!(parser.token(), token);
                assert_eq!(parser.scanner().token_range(), range);
                assert_eq!(parser.diagnostics(), []);
                parser.parse_token_node()
            });
        }
    }

    #[test]
    fn semicolon_recovery_distinguishes_explicit_asi_and_missing_tokens() {
        for (source, expected, next) in [
            ("x; y", true, Kind::Identifier),
            ("x\ny", true, Kind::Identifier),
            ("x /*\n*/ y", true, Kind::Identifier),
            ("x}", true, Kind::CloseBraceToken),
            ("x", true, Kind::EndOfFile),
            ("x y", false, Kind::Identifier),
        ] {
            let arena = FileArena::new();
            arena.build(|factory| {
                let mut parser = factory.parser(source, ScriptKind::TS);
                parser.next_token();
                let node = parser.parse_identifier();
                assert_eq!(parser.can_parse_semicolon(), expected, "{source}");
                assert_eq!(parser.parse_semicolon(), expected, "{source}");
                assert_eq!(parser.token(), next);
                assert_eq!(parser.diagnostics().is_empty(), expected);
                if !expected {
                    assert_eq!(parser.diagnostics()[0].loc, Some(TextRange::new(2, 3)));
                    assert_eq!(parser.diagnostics()[0].arguments, [";"]);
                }
                node
            });
        }
    }
}
