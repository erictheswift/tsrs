use tsrs_ast::{Kind, NodeFlags, SourceFileParseOptions};
use tsrs_core::{P, ScriptKind, TextRange, arena::Region};
use tsrs_frontend_arena::{
    ContextParser, Diagnostic, DiagnosticPhase, FileArena, ImportParser, ListParser, NameParser,
    Node, NodeData, ParsingContext, TokenParser,
};

fn compare(native: &Node<'_>, legacy: P<tsrs_ast::Node>) {
    assert_eq!(
        (native.kind, native.loc, native.flags),
        (legacy.kind(), legacy.loc(), legacy.flags())
    );
    if matches!(native.kind, Kind::Identifier | Kind::StringLiteral) {
        assert_eq!(native.text().as_bytes(), legacy.text().as_bytes());
    }
    match native.data {
        NodeData::ImportClause(data) => {
            assert_eq!(
                data.phase_modifier,
                legacy.as_import_clause().phase_modifier.get()
            );
        }
        NodeData::ImportSpecifier(data) => {
            assert_eq!(data.is_type_only, legacy.is_type_only());
            assert_eq!(
                data.property_name.is_some(),
                legacy.as_import_specifier().property_name().is_some()
            );
        }
        NodeData::NamedImports(data) => {
            let old = legacy.as_named_imports().elements;
            assert_eq!(data.elements.loc, old.loc());
            assert_eq!(data.elements.has_trailing_comma, old.has_trailing_comma());
            assert_eq!(
                data.elements.is_missing,
                <tsrs_parser::Parser as ListParser>::is_missing_node_list(Some(old))
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

fn compare_diagnostics(
    native: &[Diagnostic],
    legacy: &[P<tsrs_ast::Diagnostic>],
    phase: DiagnosticPhase,
) {
    assert_eq!(native.len(), legacy.len());
    for (native, old) in native.iter().zip(legacy) {
        assert_eq!(native.phase, phase);
        assert_eq!(
            (native.code, native.loc, native.category),
            (old.code(), old.loc(), old.category())
        );
        assert_eq!(native.message, old.message().unwrap().text());
        assert_eq!(native.arguments, old.message_args());
    }
}

#[test]
fn clauses_and_diagnostics_match_source_file_parser() {
    let cases = [
        ("import {} from 'mod';", Kind::Unknown),
        ("import value from 'mod';", Kind::Unknown),
        ("import * as ns from 'mod';", Kind::Unknown),
        ("import value, * as ns from 'mod';", Kind::Unknown),
        ("import value, {x as y, z,} from 'mod';", Kind::Unknown),
        (
            "import {type, type as, type as as, type as as as, type T, type T as U} from 'mod';",
            Kind::Unknown,
        ),
        (
            "import {'x-y' as value, '\\ud800' as lone, default as d} from 'mod';",
            Kind::Unknown,
        ),
        (
            "import { /* π */ '\\u0061' as x, \\u0074ype T} from 'mod';",
            Kind::Unknown,
        ),
        ("import {await as x, yield as y} from 'mod';", Kind::Unknown),
        ("import {type as as 'bad'} from 'mod';", Kind::Unknown),
        (
            "import { /* comment */ 'invalid'} from 'mod';",
            Kind::Unknown,
        ),
        ("import {x as class} from 'mod';", Kind::Unknown),
        ("import {x as} from 'mod';", Kind::Unknown),
        ("import {x y} from 'mod';", Kind::Unknown),
        ("import {from 'mod';", Kind::Unknown),
        ("import * ns from 'mod';", Kind::Unknown),
        ("import value, from 'mod';", Kind::Unknown),
        ("import type {T, type U} from 'mod';", Kind::TypeKeyword),
        ("import type from 'mod';", Kind::Unknown),
        ("import type from from 'mod';", Kind::TypeKeyword),
        ("import defer * as ns from 'mod';", Kind::DeferKeyword),
        ("import defer from 'mod';", Kind::Unknown),
    ];
    for (source, phase) in cases {
        for script_kind in [ScriptKind::TS, ScriptKind::JS] {
            let arena = FileArena::new();
            let mut diagnostics = (Vec::new(), Vec::new());
            let clause = arena.build(|factory| {
                let mut parser = factory.parser(source, script_kind);
                parser.replace_parsing_contexts(1 << ParsingContext::SourceElements as u32);
                parser.next_token();
                parser.parse_expected(Kind::ImportKeyword);
                let pos = parser.node_pos();
                // Phase classification belongs to the enclosing declaration, which has not migrated.
                if phase != Kind::Unknown {
                    parser.parse_expected(phase);
                }
                let name = parser.is_identifier().then(|| parser.parse_identifier());
                let clause = parser
                    .try_parse_import_clause(name, pos, phase, false)
                    .unwrap();
                assert_eq!(parser.token(), Kind::StringLiteral, "{source}");
                assert!(parser.node_stack().is_empty());
                assert_eq!(
                    parser.parsing_contexts(),
                    1 << ParsingContext::SourceElements as u32
                );
                diagnostics = parser.into_diagnostic_streams();
                clause
            });
            let region = Region::new(4096);
            let _scope = region.enter();
            let file = tsrs_parser::parse_source_file(
                SourceFileParseOptions {
                    file_name: "/imports.ts".into(),
                    ..Default::default()
                },
                source,
                script_kind,
            );
            compare(clause, file.statements.nodes()[0].import_clause().unwrap());
            compare_diagnostics(&diagnostics.0, file.diagnostics(), DiagnosticPhase::Parse);
            // Declaration-level `import type` checks are outside the migrated clause grammar.
            let js: Vec<_> = file
                .js_diagnostics()
                .iter()
                .copied()
                .filter(|d| d.message_args() == ["import...type"])
                .collect();
            compare_diagnostics(&diagnostics.1, &js, DiagnosticPhase::JavaScript);
        }
    }
}

#[test]
fn export_name_worker_matches_source_file_parser() {
    for specifier in [
        "type",
        "type as",
        "type as as",
        "type as as as",
        "type T as U",
        "x as 'x-y'",
        "'\\ud800' as value",
        "default as class",
        "x as",
    ] {
        let source = format!("export {{{specifier}}};");
        let arena = FileArena::new();
        let mut type_only = false;
        let mut diagnostics = Vec::new();
        let node = arena.build(|factory| {
            let mut parser = factory.parser(&source, ScriptKind::TS);
            parser.set_context_flags(NodeFlags::AwaitContext, true);
            parser.next_token();
            parser.parse_expected(Kind::ExportKeyword);
            parser.parse_expected(Kind::OpenBraceToken);
            let pos = parser.node_pos();
            let (is_type_only, property_name, name) =
                parser.parse_import_or_export_specifier(Kind::ExportSpecifier);
            type_only = is_type_only;
            let node = parser
                .factory()
                .new_export_specifier(is_type_only, property_name, name);
            let node = parser.finish_node(node, pos);
            assert_eq!(parser.token(), Kind::CloseBraceToken);
            diagnostics = parser.into_diagnostics();
            node
        });
        let region = Region::new(4096);
        let _scope = region.enter();
        let file = tsrs_parser::parse_source_file(
            SourceFileParseOptions {
                file_name: "/exports.ts".into(),
                ..Default::default()
            },
            &source,
            ScriptKind::TS,
        );
        let exports = file.statements.nodes()[0]
            .as_export_declaration()
            .export_clause
            .unwrap();
        let specifier = exports.as_named_exports().elements.nodes()[0];
        assert_eq!(type_only, specifier.is_type_only());
        compare(node, specifier);
        compare_diagnostics(&diagnostics, file.diagnostics(), DiagnosticPhase::Parse);
    }
    for name in ["ns", "default", "'arbitrary-name'", "'\\ud800'"] {
        let source = format!("export * as {name} from 'mod';");
        let arena = FileArena::new();
        let node = arena.build(|factory| {
            let mut parser = factory.parser(&source, ScriptKind::TS);
            parser.set_context_flags(NodeFlags::AwaitContext, true);
            parser.next_token();
            parser.parse_expected(Kind::ExportKeyword);
            let pos = parser.node_pos();
            parser.parse_expected(Kind::AsteriskToken);
            parser.parse_expected(Kind::AsKeyword);
            parser.parse_namespace_export(pos)
        });
        let region = Region::new(4096);
        let _scope = region.enter();
        let file = tsrs_parser::parse_source_file(
            SourceFileParseOptions {
                file_name: "/exports.ts".into(),
                ..Default::default()
            },
            &source,
            ScriptKind::TS,
        );
        compare(
            node,
            file.statements.nodes()[0]
                .as_export_declaration()
                .export_clause
                .unwrap(),
        );
    }
}

mod native {
    use super::*;

    #[test]
    fn speculative_imports_retain_nodes_and_rewind_both_diagnostic_streams() {
        let arena = FileArena::new();
        let region = Region::new(4096);
        let scope = region.enter();
        let mut diagnostics = (Vec::new(), Vec::new());
        let root = arena.build(|factory| {
            let source = String::from("{type T, '\\ud800' as lone, 'invalid'} from 'mod'");
            let mut parser = factory.parser(&source, ScriptKind::JS);
            drop(source);
            parser.next_token();
            let mut saved = None;
            assert!(parser.look_ahead(|p| {
                let root = p
                    .try_parse_import_clause(None, 0, Kind::Unknown, false)
                    .unwrap();
                assert_eq!(p.diagnostics().len(), 1);
                assert_eq!(p.js_diagnostics().len(), 1);
                saved = Some(root);
                true
            }));
            assert_eq!(parser.token(), Kind::OpenBraceToken);
            assert_eq!(parser.diagnostics(), []);
            assert_eq!(parser.js_diagnostics(), []);
            let old = saved.unwrap();
            for _ in 0..if cfg!(miri) { 2 } else { 1024 } {
                parser.look_ahead(|p| {
                    p.try_parse_import_clause(None, 0, Kind::Unknown, false);
                    true
                });
            }
            let new = parser
                .try_parse_import_clause(None, 0, Kind::Unknown, false)
                .unwrap();
            assert!(!std::ptr::eq(old, new));
            assert_eq!(parser.diagnostics().len(), 1);
            assert_eq!(parser.js_diagnostics().len(), 1);
            diagnostics = parser.into_diagnostic_streams();
            old
        });
        assert_eq!(region.used_bytes(), 0);
        drop(scope);
        drop(region);
        let NodeData::ImportClause(clause) = root.data else {
            panic!("import clause")
        };
        let NodeData::NamedImports(imports) = clause.named_bindings.unwrap().data else {
            panic!("named imports")
        };
        let NodeData::ImportSpecifier(specifier) = imports.elements.nodes[1].data else {
            panic!("specifier")
        };
        let property = specifier.property_name.unwrap();
        std::thread::scope(|s| {
            let text = s.spawn(|| property.text());
            let name = s.spawn(|| specifier.name.text());
            assert_eq!(text.join().unwrap().as_bytes(), [0xed, 0xa0, 0x80]);
            assert_eq!(name.join().unwrap(), "lone");
        });
        drop(arena);
        assert_eq!(diagnostics.0[0].phase, DiagnosticPhase::Parse);
        assert_eq!(diagnostics.1[0].phase, DiagnosticPhase::JavaScript);
        assert_eq!(diagnostics.1[0].arguments, ["import...type"]);
    }

    #[test]
    fn js_diagnostics_do_not_deduplicate_or_mark_following_nodes() {
        let arena = FileArena::new();
        arena.build(|factory| {
            let mut parser = factory.parser("type T, next", ScriptKind::JS);
            parser.next_token();
            let node = parser.parse_import_specifier();
            assert!(!node.flags().intersects(NodeFlags::ThisNodeHasError));
            let first = parser.js_diagnostics()[0].clone();
            let checkpoint = parser.mark();
            parser.check_js_import_specifier(node);
            assert_eq!(parser.js_diagnostics(), [first.clone(), first.clone()]);
            parser.rewind(checkpoint);
            assert_eq!(parser.js_diagnostics(), [first]);
            parser.parse_expected(Kind::CommaToken);
            let next = parser.parse_import_specifier();
            assert!(!next.flags().intersects(NodeFlags::ThisNodeHasError));
            assert_eq!(parser.diagnostics(), []);
            node
        });
    }

    #[test]
    fn jsdoc_and_reparsed_nodes_suppress_js_syntax_checks() {
        for flags in [NodeFlags::None, NodeFlags::JSDoc, NodeFlags::Reparsed] {
            let arena = FileArena::new();
            arena.build(|factory| {
                let mut parser = factory.parser(" /* comment */ type T", ScriptKind::JS);
                parser.set_context_flags(flags, true);
                parser.next_token();
                let node = parser.parse_import_specifier();
                assert_eq!(parser.js_diagnostics().len(), usize::from(flags.is_empty()));
                if flags.is_empty() {
                    assert_eq!(parser.js_diagnostics()[0].loc, Some(TextRange::new(15, 21)));
                }
                node
            });
        }
    }

    #[test]
    fn import_clauses_restore_await_and_jsdoc_scanner_state() {
        let arena = FileArena::new();
        arena.build(|factory| {
            let mut parser = factory.parser(
                "{ type T,\n * await as a,\n * U } from 'mod'\n*after",
                ScriptKind::JS,
            );
            parser.set_context_flags(NodeFlags::JSDoc, true);
            parser.next_token();
            let clause = parser
                .try_parse_import_clause(None, 0, Kind::Unknown, true)
                .unwrap();
            assert!(!parser.statement_has_await_identifier());
            assert_eq!(parser.diagnostics(), []);
            assert_eq!(parser.js_diagnostics(), []);
            let imports = clause
                .as_import_clause()
                .named_bindings()
                .unwrap()
                .as_named_imports();
            assert_eq!(imports.elements().nodes().len(), 3);
            assert_eq!(parser.token(), Kind::StringLiteral);
            parser.next_token();
            assert_eq!(parser.token(), Kind::AsteriskToken);
            clause
        });
        let arena = FileArena::new();
        arena.build(|factory| {
            let mut parser = factory.parser("* as ns from 'mod'", ScriptKind::TS);
            parser.next_token();
            parser.set_statement_has_await_identifier(true);
            let clause = parser
                .try_parse_import_clause(None, 0, Kind::Unknown, false)
                .unwrap();
            assert!(parser.statement_has_await_identifier());
            clause
        });
    }

    #[test]
    fn side_effect_import_probe_keeps_token_and_pending_error() {
        let arena = FileArena::new();
        arena.build(|factory| {
            let mut parser = factory.parser("'mod'", ScriptKind::TS);
            parser.next_token();
            parser.parse_expected(Kind::FromKeyword);
            assert!(
                parser
                    .try_parse_import_clause(None, 0, Kind::Unknown, false)
                    .is_none()
            );
            assert_eq!(parser.token(), Kind::StringLiteral);
            assert_eq!(parser.diagnostics().len(), 1);
            let literal = parser.parse_literal_expression();
            assert!(literal.flags().intersects(NodeFlags::ThisNodeHasError));
            literal
        });
    }
}
