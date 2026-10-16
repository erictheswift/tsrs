use std::rc::Rc;
use tsrs_frontend_arena::{FileArena, Node, NodeData, ParseOptions, ScriptKind, parse_and_bind};

fn walk<'ast>(root: &'ast Node<'ast>) -> Vec<&'ast Node<'ast>> {
    let mut result = vec![root];
    let mut i = 0;
    while i < result.len() {
        result[i].for_each_child(&mut |n| {
            result.push(n);
            false
        });
        i += 1;
    }
    result
}

#[test]
fn published_graph_survives_construction_and_has_native_cycles() {
    let arena = FileArena::new();
    let file = parse_and_bind(&arena, "export function f(x: number) { let y = x + 1; return y; }", ParseOptions::default());
    assert!(file.diagnostics.is_empty(), "{:?}", file.diagnostics);
    let source = file.root.source_file().unwrap();
    let function = source.statements.nodes[0];
    assert!(std::ptr::eq(function.parent.unwrap(), file.root));
    assert_eq!(function.symbol().unwrap().name, "f");
    assert!(std::ptr::eq(function.symbol().unwrap().declarations[0], function));
    let nodes = walk(file.root);
    assert!(nodes.iter().any(|n| n.text() == "x"));
    assert!(nodes.iter().any(|n| n.flow_node().is_some()));
    let sum = |root| walk(root).iter().map(|n| n.loc.len().unwrap_or(0) as u64).sum::<u64>();
    std::thread::scope(|s| {
        let first = s.spawn(|| sum(file.root));
        let second = s.spawn(|| sum(file.root));
        assert_eq!(first.join().unwrap(), second.join().unwrap());
    });
    assert!(arena.used_bytes() > 0);
}

#[test]
fn independently_owned_files_share_a_borrow_scope() {
    fn cache<'ast>(a: &'ast Node<'ast>, b: &'ast Node<'ast>) -> Vec<&'ast Node<'ast>> {
        vec![a, b]
    }
    let a = FileArena::new();
    let a_file = parse_and_bind(&a, "const a = 1", ParseOptions::default());
    {
        let b = FileArena::new();
        let b_file = parse_and_bind(&b, "const b = a", ParseOptions::default());
        let nodes = cache(a_file.root, b_file.root);
        assert_eq!(nodes[0].locals().unwrap().get("a").unwrap().name, "a");
        assert_eq!(nodes[1].locals().unwrap().get("b").unwrap().name, "b");
    }
    assert_eq!(a_file.root.source_file().unwrap().text, "const a = 1");
}

#[test]
fn versions_and_detached_diagnostics_have_independent_owners() {
    let old = Rc::new(FileArena::new());
    let request_owner = Rc::clone(&old);
    let weak = Rc::downgrade(&old);
    let previous = parse_and_bind(&request_owner, "const value = 1", ParseOptions::default());
    drop(old);
    let new = FileArena::new();
    let current = parse_and_bind(&new, "const value = 2", ParseOptions::default());
    assert!(previous.root.source_file().unwrap().text.ends_with('1'));
    assert!(current.root.source_file().unwrap().text.ends_with('2'));
    drop(previous);
    drop(request_owner);
    assert!(weak.upgrade().is_none());
    let diagnostics = {
        let malformed = FileArena::new();
        parse_and_bind(&malformed, "const = ;", ParseOptions::default()).diagnostics
    };
    assert_ne!(diagnostics, []);
    assert!(diagnostics.iter().all(|d| d.code > 0 && !d.message.is_empty()));
}

#[test]
fn jsdoc_jsx_unicode_and_repeated_release() {
    // Three cycles exercise reuse under Miri; native runs stress repeated release more heavily.
    let rounds = if cfg!(miri) { 3 } else { 32 };
    for _ in 0..rounds {
        let arena = FileArena::new();
        let file = parse_and_bind(
            &arena,
            "/** @param {number} café explanation */\nexport function f(café) { return <div>{café}</div> }",
            ParseOptions { file_name: "/prototype.jsx".into(), script_kind: ScriptKind::JSX, ..Default::default() },
        );
        let nodes = walk(file.root);
        assert!(nodes.iter().any(|n| n.text() == "café"));
        assert!(nodes.iter().any(|n| !n.jsdoc.is_empty()));
        assert!(nodes.iter().any(|n| matches!(n.data, NodeData::JsxElement(_))));
    }
}

#[test]
fn arena_release_during_unwind_and_next_parse() {
    let panic = std::panic::catch_unwind(|| {
        let arena = FileArena::new();
        let _file = parse_and_bind(&arena, "const x = 1", ParseOptions::default());
        panic!("cancel request");
    });
    assert!(panic.is_err());
    // An invalid parser contract panics after source registration, before publication starts.
    let construction_panic = std::panic::catch_unwind(|| {
        let arena = FileArena::new();
        parse_and_bind(&arena, "const x = 1", ParseOptions { file_name: "relative.ts".into(), ..Default::default() });
    });
    assert!(construction_panic.is_err());
    let arena = FileArena::new();
    assert_eq!(parse_and_bind(&arena, "const x = 2", ParseOptions::default()).diagnostics, []);
}

#[test]
fn cooked_lone_surrogates_remain_bytes_without_invalid_rust_strings() {
    let arena = FileArena::new();
    let file = parse_and_bind(
        &arena,
        r#"const x = "\ud800"; const y = "\ud83d\ude80"; class C { "\ud800"() {} } declare module "\ud800*" {}"#,
        ParseOptions::default(),
    );
    let nodes = walk(file.root);
    let lone = nodes.iter().find(|n| n.text().as_bytes() == [0xed, 0xa0, 0x80]).unwrap().text();
    assert!(lone.as_str().is_err());
    let members = file.root.locals().unwrap().get("C").unwrap().members.unwrap();
    assert_eq!(members.get_bytes(lone.as_bytes()).unwrap().name.as_bytes(), lone.as_bytes());
    assert_eq!(file.root.source_file().unwrap().metadata.pattern_ambient_modules[0].text.as_bytes(), &[0xed, 0xa0, 0x80, b'*']);
    assert!(nodes.iter().any(|n| n.text().as_str() == Ok("🚀")));
}
