use super::*;

macro_rules! verify_edge {
    ($method:ident, $map:ident, $old:ty, $new:ty) => {
        pub(crate) fn $method(&self, source: Option<P<$old>>, target: Option<&$new>) {
            assert_eq!(source.map(|n| self.$map[&n.addr()].as_ptr().cast_const()), target.map(std::ptr::from_ref));
        }
    };
}

impl<'ast> Publisher<'ast> {
    verify_edge!(verify_node, nodes, old::Node, Node<'ast>);
    verify_edge!(verify_list, lists, old::NodeList, NodeList<'ast>);
    verify_edge!(verify_modifiers, modifiers, old::ModifierList, ModifierList<'ast>);
    verify_edge!(verify_symbol, symbols, old::Symbol, Symbol<'ast>);
    verify_edge!(verify_table, tables, old::SymbolTable, SymbolTable<'ast>);
    verify_edge!(verify_flow, flows, old::flow::FlowNode, FlowNode<'ast>);
    verify_edge!(verify_flow_list, flow_lists, old::flow::FlowList, FlowList<'ast>);

    pub(crate) fn verify_nodes(&self, source: &[P<old::Node>], target: &[&Node<'ast>]) {
        assert_eq!(source.len(), target.len());
        for (&a, &b) in source.iter().zip(target) {
            self.verify_node(Some(a), Some(b));
        }
    }

    pub(super) fn verify_work(&self, work: Work<'ast>) {
        // SAFETY: finish calls this only after the entire queue is initialized. These shared reads
        // exercise every copied edge, including cyclic references, while construction is still live.
        unsafe {
            match work {
                Work::Node(a, b) => {
                    let b = b.as_ref();
                    assert_eq!((a.kind(), a.loc(), a.flags()), (b.kind, b.loc, b.flags));
                    self.verify_node(a.parent(), b.parent);
                    self.verify_nodes(a.jsdoc(Some(self.source.get())), b.jsdoc);
                    self.verify_data(a.data(), b.data);
                    let mut old_children = Vec::new();
                    a.for_each_child(&mut |n| {
                        old_children.push(n);
                        false
                    });
                    let mut new_children = Vec::new();
                    b.for_each_child(&mut |n| {
                        new_children.push(n);
                        false
                    });
                    self.verify_nodes(&old_children, &new_children);
                }
                Work::List(a, b) => {
                    let b = b.as_ref();
                    assert_eq!((a.loc(), a.has_trailing_comma()), (b.loc, b.has_trailing_comma));
                    self.verify_nodes(a.nodes(), b.nodes);
                }
                Work::Modifiers(a, b) => {
                    let b = b.as_ref();
                    assert_eq!((a.loc(), a.has_trailing_comma(), a.modifier_flags), (b.loc, b.has_trailing_comma, b.flags));
                    self.verify_nodes(a.nodes(), b.nodes);
                }
                Work::Symbol(a, b) => {
                    let b = b.as_ref();
                    assert_eq!((a.flags(), a.check_flags()), (b.flags, b.check_flags));
                    assert_eq!(a.name().as_bytes(), b.name.as_bytes());
                    self.verify_nodes(a.declarations(), b.declarations);
                    self.verify_node(a.value_declaration(), b.value_declaration);
                    self.verify_symbol(a.parent(), b.parent);
                    self.verify_symbol(a.export_symbol(), b.export_symbol);
                    self.verify_table(a.members(), b.members);
                    self.verify_table(a.exports(), b.exports);
                }
                Work::Table(a, b) => {
                    let b = b.as_ref();
                    let entries = a.entries();
                    assert_eq!(entries.len(), b.entries.len());
                    for (name, symbol) in &entries {
                        self.verify_symbol(Some(*symbol), b.get(name));
                    }
                }
                Work::Flow(a, b) => {
                    let b = b.as_ref();
                    assert_eq!(a.flags(), b.flags);
                    self.verify_node(a.node(), b.node);
                    self.verify_flow(a.antecedent(), b.antecedent);
                    self.verify_flow_list(a.antecedents(), b.antecedents);
                }
                Work::FlowList(a, b) => {
                    let b = b.as_ref();
                    self.verify_flow(Some(a.flow), Some(b.flow));
                    self.verify_flow_list(a.next.get(), b.next);
                }
            }
        }
    }
}

#[test]
fn syntax_symbols_flows_and_diagnostics_match_eager_frontend() {
    use crate::{ParseOptions, ScriptKind, parse_and_bind};
    let cases = [
        (
            ScriptKind::TS,
            "import {x as y} from './a'; export {y}; export default class C<T> { #v = 1; static n = 2; constructor(public x: T) {} m(v?: T): T { return v ?? this.x; } }",
        ),
        (
            ScriptKind::TS,
            "export interface I<T> extends Base<T> { f(x: T): T; p?: string } type A<T> = T extends string ? { [K in keyof T]?: T[K] } : [T, ...T[]]; enum E { A, B = 3 } namespace N { export const x = 1; }",
        ),
        (
            ScriptKind::TS,
            "function f(x: number) { outer: for(let i=0;i<4;i++){ if(x) continue outer; else break; } do { x--; } while(x); try { switch(x) {case 0: x++; break; default: throw x} } catch(e) { return e; } finally { x = 3; } return x; }",
        ),
        (ScriptKind::TSX, "const f = <T,>(x:T) => x; const y = <A.B p='x' {...props}>{f(1)}<span/>text</A.B>; const z = <>a{y}</>;"),
        (
            ScriptKind::JS,
            "/** @template T\n * @param {T} café comment\n * @returns {T} explanation */\nexport function f(café) { return café; } module.exports.x = f; exports.a = 1;",
        ),
        (ScriptKind::TS, "const = ; function f( { ; let x = ; const duplicate = 1; const duplicate = 2;"),
        (ScriptKind::TS, "const a = `x${1+2}`; const b = /abc/g; const c = { ...a, ['x']: 1 }; async function* f() { yield await a; }"),
        (ScriptKind::JSON, "{\"name\":\"café\",\"values\":[1,true,null]}"),
    ];
    for (kind, source) in cases {
        let baseline_region = tsrs_core::arena::Region::new(64 << 10);
        let scope = baseline_region.enter();
        let file = tsrs_parser::parse_source_file_owned(
            old::SourceFileParseOptions { file_name: "/prototype.ts".into(), ..Default::default() },
            source.to_owned(),
            kind,
        );
        tsrs_binder::bind_source_file(file);
        // Force the same eagerly published JSDoc before comparing its diagnostics.
        let baseline_arena = FileArena::new();
        let (baseline, _) = Publisher::new(&baseline_arena, file).finish();
        let mut expected = Vec::new();
        for (items, phase) in [
            (file.diagnostics(), crate::DiagnosticPhase::Parse),
            (file.js_diagnostics(), crate::DiagnosticPhase::JavaScript),
            (file.jsdoc_diagnostics(), crate::DiagnosticPhase::JSDoc),
            (file.bind_diagnostics(), crate::DiagnosticPhase::Bind),
        ] {
            expected.extend(items.iter().map(|&d| crate::diagnostic(d, phase)));
        }
        let outline = |root: &Node<'_>| {
            let mut stack = vec![root];
            let mut result = Vec::new();
            while let Some(n) = stack.pop() {
                result.push((n.kind, n.loc, n.flags, n.text().as_bytes().to_vec()));
                n.for_each_child(&mut |child| {
                    stack.push(child);
                    false
                });
            }
            result
        };
        for direct in [false, true] {
            let arena = FileArena::new();
            let published =
                parse_and_bind(&arena, source, ParseOptions { script_kind: kind, direct_allocation: direct, ..Default::default() });
            assert_eq!(expected, published.diagnostics, "{source}");
            assert_eq!(outline(baseline), outline(published.root), "{source}");
        }
        drop(scope);
        drop(baseline_region);
    }
}

impl Publisher<'_> {
    pub(crate) fn verify_metadata(&self, a: &old::SourceFile, b: &crate::FileMetadata<'_>) {
        assert_eq!(a.parse_options().path.as_str(), b.path);
        assert_eq!(a.parse_options().external_module_indicator_options, b.external_module_options);
        assert_eq!(
            (a.language_variant.get(), a.script_kind.get(), a.is_declaration_file.get()),
            (b.language_variant, b.script_kind, b.is_declaration_file)
        );
        assert_eq!(a.uses_uri_style_node_core_modules.get(), b.uses_uri_style_node_core_modules);
        assert_eq!(
            (a.identifier_count.get(), a.node_count.get(), a.text_count.get(), a.symbol_count.get()),
            (b.identifier_count, b.node_count, b.text_count, b.symbol_count)
        );
        self.verify_nodes(a.imports.get(), b.imports);
        self.verify_nodes(a.module_augmentations.get(), b.module_augmentations);
        self.verify_nodes(a.reparsed_clones.get(), b.reparsed_clones);
        assert_eq!(a.ambient_module_names.get(), b.ambient_module_names);
        assert_eq!(a.comment_directives.get(), b.comment_directives);
        assert_eq!(a.check_js_directive.get().map(|d| *d), b.check_js_directive);
        self.verify_node(a.common_js_module_indicator.get(), b.common_js_module_indicator);
        self.verify_node(a.external_module_indicator.get(), b.external_module_indicator);
        self.verify_table(a.global_exports.get(), b.global_exports);
        assert_eq!(a.pattern_ambient_modules.get().len(), b.pattern_ambient_modules.len());
        for (a, b) in a.pattern_ambient_modules.get().iter().zip(b.pattern_ambient_modules) {
            assert_eq!(a.pattern.text.as_bytes(), b.text.as_bytes());
            assert_eq!(a.pattern.kind, b.kind);
            self.verify_symbol(Some(a.symbol), Some(b.symbol));
        }
        for (a, b) in [
            (a.referenced_files.get(), b.referenced_files),
            (a.type_reference_directives.get(), b.type_reference_directives),
            (a.lib_reference_directives.get(), b.lib_reference_directives),
        ] {
            assert_eq!(a.len(), b.len());
            for (a, b) in a.iter().zip(b) {
                assert_eq!(
                    (a.text_range, a.file_name.as_str(), a.resolution_mode, a.preserve),
                    (b.loc, b.file_name, b.resolution_mode, b.preserve)
                );
            }
        }
        assert_eq!(a.pragmas.get().len(), b.pragmas.len());
        for (a, b) in a.pragmas.get().iter().zip(b.pragmas) {
            assert_eq!((a.name.as_str(), a.comment_range, a.args.len()), (b.name, b.range, b.arguments.len()));
            for arg in b.arguments {
                let expected = &a.args[arg.key];
                assert_eq!((expected.text_range, expected.name.as_str(), expected.value.as_str()), (arg.loc, arg.name, arg.value));
            }
        }
    }
}

#[test]
fn declaration_file_metadata_and_pragmas_survive_release() {
    let arena = FileArena::new();
    let source = "/// <reference path=\"./other.d.ts\" preserve=\"true\" />\n/// <reference types=\"node\" />\n/// <reference lib=\"es2020\" />\n// @ts-check\n// @ts-ignore\ndeclare module '*.foo' { export const x: number; }\n";
    let file = crate::parse_and_bind(&arena, source, crate::ParseOptions { file_name: "/module.d.ts".into(), ..Default::default() });
    let meta = file.root.source_file().unwrap().metadata;
    assert!(meta.is_declaration_file);
    assert_eq!(meta.referenced_files[0].file_name, "./other.d.ts");
    assert_eq!(meta.type_reference_directives[0].file_name, "node");
    assert_eq!(meta.lib_reference_directives[0].file_name, "es2020");
    assert_ne!(meta.pragmas, []);
    assert!(meta.check_js_directive.unwrap().enabled);
    assert_eq!(meta.pattern_ambient_modules[0].text, "*.foo");
}

#[test]
#[ignore = "requires an external public corpus manifest in TSRS_LIFETIME_MANIFEST"]
fn public_corpus_matches_construction_field_by_field() {
    let manifest = std::fs::read_to_string(std::env::var("TSRS_LIFETIME_MANIFEST").unwrap()).unwrap();
    let mut count = 0;
    for name in manifest.lines() {
        let source = std::fs::read_to_string(name).unwrap();
        let kind = tsrs_core::get_script_kind_from_file_name(name);
        let arena = FileArena::new();
        let file =
            crate::parse_and_bind(&arena, &source, crate::ParseOptions { file_name: name.into(), script_kind: kind, ..Default::default() });
        assert_eq!(file.root.source_file().unwrap().text, source);
        count += 1;
    }
    assert!(count > 0);
    eprintln!("verified {count} public files");
}
