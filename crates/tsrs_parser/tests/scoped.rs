use tsrs_ast::{Kind, NodeFactory, SourceFileParseOptions};
use tsrs_core::{Allocator, ScriptKind, arena::Region};
use tsrs_parser::{NodeRef, parse_source_file_in};

fn options(name: &str) -> SourceFileParseOptions {
    SourceFileParseOptions { file_name: name.into(), ..Default::default() }
}

fn collect_text(node: NodeRef<'_>, result: &mut Vec<String>) {
    if matches!(
        node.kind(),
        Kind::Identifier
            | Kind::StringLiteral
            | Kind::NumericLiteral
            | Kind::JsxText
            | Kind::JSDocText
    ) {
        result.push(node.text().to_owned());
    }
    for child in node.children() {
        collect_text(child, result);
    }
}

#[test]
fn scoped_tree_survives_input_parser_and_surrounding_regions() {
    let outer = Region::new(4096);
    let outer_scope = outer.enter();
    let tree = Region::new_scratch(4096);
    let tree_scope = tree.enter_scratch();
    let allocator = Allocator::current();
    let input = String::from("export {}; await f(); const value = 'a\\nb';");
    let nested = Region::new(4096);
    let nested_scope = nested.enter();
    let file = parse_source_file_in(&allocator, options("/src/a.ts"), &input, ScriptKind::TS);
    drop(input);
    drop(nested_scope);
    drop(nested);
    drop(tree_scope);
    drop(tree);
    drop(outer_scope);
    drop(outer);

    assert_eq!(file.file_name(), "/src/a.ts");
    assert_eq!(file.script_kind(), ScriptKind::TS);
    assert!(file.text().contains("await f()"));
    assert_eq!(file.statements().len(), 3);
    assert_eq!(file.diagnostics().len(), 0);
    let mut texts = Vec::new();
    collect_text(file.root(), &mut texts);
    assert!(texts.iter().any(|s| s == "value"));
    assert!(texts.iter().any(|s| s == "a\nb"));
}

#[test]
fn scoped_lazy_members_and_jsdoc_use_the_retained_owner() {
    tsrs_parser::enable_lazy_dts();
    let region = Region::new(4096);
    let scope = region.enter();
    let allocator = Allocator::current();
    let file = parse_source_file_in(
        &allocator,
        options("/src/a.d.ts"),
        "/** Description with text. */ interface I { value: string; method(x: number): number; }",
        ScriptKind::TS,
    );
    drop(scope);
    drop(region);
    let nested = Region::new(4096);
    let nested_scope = nested.enter();
    let declaration = file.statements().get(0).unwrap();
    let members = declaration.members();
    let docs = declaration.jsdoc();
    drop(nested_scope);
    drop(nested);

    assert_eq!(declaration.name().unwrap().text(), "I");
    assert_eq!(members.len(), 2);
    assert_eq!(members.get(0).unwrap().name().unwrap().text(), "value");
    assert!(members.iter().all(|node| node.parent() == Some(declaration)));
    assert_eq!(docs.len(), 1);
    let mut texts = Vec::new();
    collect_text(docs.get(0).unwrap(), &mut texts);
    assert!(texts.iter().any(|s| s == "Description with text."));
}

#[test]
fn scoped_diagnostics_and_multiple_files_share_a_live_allocator() {
    let region = Region::new(4096);
    let scope = region.enter();
    let allocator = Allocator::current();
    let bad = parse_source_file_in(
        &allocator,
        options("/src/bad.ts"),
        "const x = 'unterminated\n",
        ScriptKind::TS,
    );
    let jsx = parse_source_file_in(
        &allocator,
        options("/src/a.tsx"),
        "const view = <div>hello</div>;",
        ScriptKind::TSX,
    );
    let json = parse_source_file_in(
        &allocator,
        options("/src/a.json"),
        "{\"answer\":42}",
        ScriptKind::JSON,
    );
    drop(scope);
    drop(region);
    assert!(bad.diagnostics().any(|d| d.code() == 1002 && d.message().contains("Unterminated")));
    assert_eq!(jsx.diagnostics().len(), 0);
    assert_eq!(json.diagnostics().len(), 0);
    let mut texts = Vec::new();
    collect_text(jsx.root(), &mut texts);
    collect_text(json.root(), &mut texts);
    assert!(texts.iter().any(|s| s == "hello"));
    assert!(texts.iter().any(|s| s == "42"));
}

#[test]
fn factory_nodes_lists_and_clones_stay_in_the_captured_region() {
    let region = Region::new(4096);
    let _scope = region.enter();
    let allocator = Allocator::current();
    // SAFETY: the guard remains live through every use of the raw test nodes and their text.
    let factory = unsafe { NodeFactory::new_in(&allocator) };
    let name = allocator.alloc_str("value");
    let nested = Region::new(4096);
    let _nested_scope = nested.enter();
    let node = factory.new_identifier(name);
    let list = factory.new_node_list(vec![node]);
    let clone = factory.deep_clone_reparse(Some(node)).unwrap();
    for address in [node.addr(), list.addr(), clone.addr(), list.nodes().as_ptr().addr()] {
        assert!(Region::containing(address).unwrap().ptr_eq(&region));
    }
    assert_eq!(clone.text(), "value");
    assert_ne!(clone, node);
    assert_eq!(factory.node_count(), 2);
}

#[test]
fn scoped_and_compiler_views_preserve_the_same_parse_tree() {
    fn scoped_tree(node: NodeRef<'_>, result: &mut Vec<(Kind, tsrs_core::TextRange)>) {
        result.push((node.kind(), node.loc()));
        for child in node.children() {
            scoped_tree(child, result);
        }
    }
    fn raw_tree(
        node: tsrs_core::P<tsrs_ast::Node>,
        result: &mut Vec<(Kind, tsrs_core::TextRange)>,
    ) {
        result.push((node.kind(), node.loc()));
        node.for_each_child(&mut |child| {
            raw_tree(child, result);
            false
        });
    }
    let cases = [
        ("/src/a.ts", "export {}; await f(); const t = `a${1_2}b`;", ScriptKind::TS),
        ("/src/a.tsx", "const v = <div title={'a\\nb'}>{`x${1_2}`}</div>;", ScriptKind::TSX),
        ("/src/a.js", "/** @typedef {object} Value @property {string} name */", ScriptKind::JS),
        ("/src/a.json", "{\"value\": [1, true, \"a\\nb\"]}", ScriptKind::JSON),
        ("/src/bad.ts", "const x = 'unterminated\nfunction f( {", ScriptKind::TS),
    ];
    for (name, source, kind) in cases {
        let region = Region::new(4096);
        let _scope = region.enter();
        let allocator = Allocator::current();
        let scoped = parse_source_file_in(&allocator, options(name), source, kind);
        // SAFETY: the region remains entered until all raw comparison handles are discarded.
        let raw = unsafe { tsrs_parser::raw::parse_source_file(options(name), source, kind) };
        let mut expected = Vec::new();
        let mut actual = Vec::new();
        raw_tree(raw.as_node(), &mut expected);
        scoped_tree(scoped.root(), &mut actual);
        assert_eq!(actual, expected, "{name}");
        assert_eq!(
            scoped.diagnostics().map(|d| (d.code(), d.loc())).collect::<Vec<_>>(),
            raw.diagnostics.get().iter().map(|d| (d.code(), d.loc())).collect::<Vec<_>>(),
            "{name}"
        );
    }
}

#[test]
fn long_factory_text_and_lists_survive_a_nested_region() {
    let region = Region::new(4096);
    let _scope = region.enter();
    let allocator = Allocator::current();
    // SAFETY: every input is in the captured region, retained through all raw handle uses.
    let factory = unsafe { NodeFactory::new_in(&allocator) };
    let text = factory.alloc_text(&"x".repeat(1 << 16));
    let (node, list) = {
        let nested = Region::new(4096);
        let _scope = nested.enter();
        let node = factory.new_identifier(text);
        (node, factory.new_node_list(vec![node; 1 << 16]))
    };
    assert_eq!(node.text(), text);
    assert!(Region::containing(node.text().as_ptr().addr()).unwrap().ptr_eq(&region));
    assert_eq!(list.nodes().len(), 1 << 16);
    assert_eq!(list.nodes().first(), Some(&node));
    assert_eq!(list.nodes().last(), Some(&node));
}

#[test]
fn scoped_long_literal_retains_normalized_text() {
    let region = Region::new(4096);
    let scope = region.enter();
    let allocator = Allocator::current();
    let expected = format!("{}\n", "x".repeat(1 << 16));
    let file = {
        let nested = Region::new(4096);
        let _scope = nested.enter();
        let input = format!("const value = '{}\\n';", "x".repeat(1 << 16));
        parse_source_file_in(&allocator, options("/src/long.ts"), &input, ScriptKind::TS)
    };
    drop(scope);
    drop(region);
    let mut texts = Vec::new();
    collect_text(file.root(), &mut texts);
    assert!(texts.contains(&expected));
    assert_eq!(file.diagnostics().len(), 0);
}
