use tsrs_ast::Kind;
use tsrs_core::{P, arena::Region};
use tsrs_frontend_arena::{FileArena, Node, NodeData, parse_isolated_entity_name};

fn compare(text: &str) {
    let arena = FileArena::new();
    let native = parse_isolated_entity_name(&arena, text);
    let region = Region::new(4096);
    let _scope = region.enter();
    let legacy = tsrs_parser::parse_isolated_entity_name(text);
    assert_eq!(native.is_some(), legacy.is_some(), "{text:?}");
    let Some((native, legacy)) = native.zip(legacy) else {
        return;
    };
    let mut stack: Vec<(&Node<'_>, P<tsrs_ast::Node>)> = vec![(native, legacy)];
    assert!(native.parent.is_none());
    assert!(legacy.parent().is_none());
    while let Some((native, legacy)) = stack.pop() {
        assert_eq!(native.kind, legacy.kind(), "kind: {text:?}");
        assert_eq!(native.loc, legacy.loc(), "range: {text:?}");
        assert_eq!(native.flags, legacy.flags(), "flags: {text:?}");
        if native.kind == Kind::Identifier {
            assert_eq!(
                native.text().as_bytes(),
                legacy.text().as_bytes(),
                "text: {text:?}"
            );
        }
        if let NodeData::QualifiedName(data) = native.data {
            let old = legacy.as_qualified_name();
            assert!(std::ptr::eq(data.left.parent.unwrap(), native));
            assert!(std::ptr::eq(data.right.parent.unwrap(), native));
            assert_eq!(old.left.parent(), Some(legacy));
            assert_eq!(old.right.parent(), Some(legacy));
            stack.extend([(data.left, old.left), (data.right, old.right)]);
        }
    }
}

#[test]
fn native_entity_names_match_legacy_nodes_and_rejection() {
    for text in [
        "value",
        "A.B.C",
        " await.yield",
        "default.class",
        "a.\n b",
        "a.\nb c",
        "a.\nb\nc",
        "a /* comment */ . b // trailing",
        "a.// comment\nb",
        "a.\r\nb",
        "a.\u{2028}b",
        "π.名称",
        "𐐀.𐐁",
        "a\u{200c}.b\u{200d}",
        r"\u0061.\u{62}",
        r"cl\u0061ss.default",
        "",
        " ",
        "a.",
        ".a",
        "a..b",
        "#private",
        "a.#private",
        "a.#private.z",
        "a.b;",
        "a?.b",
        "a[b]",
        "a.<B>",
        "a b",
        "1.a",
        "a/*",
        "a.\n b \\",
        r"a.\u0062 \u00zz",
        r"a.\u{110000}",
        "a.\n b // @ts-ignore\n c",
        "a.\n b /* unterminated",
        "a.\n b class",
    ] {
        compare(text);
    }
}

#[test]
fn parsed_names_own_source_and_survive_region_release() {
    let arena = FileArena::new();
    let region = Region::new(4096);
    let scope = region.enter();
    let source = String::from(r"  Namespace.\u0054ype");
    let root = parse_isolated_entity_name(&arena, &source).unwrap();
    drop(source);
    assert_eq!(region.used_bytes(), 0);
    drop(scope);
    drop(region);
    let NodeData::QualifiedName(name) = root.data else {
        panic!("qualified name");
    };
    assert_eq!(name.left.text(), "Namespace");
    assert_eq!(name.right.text(), "Type");
    std::thread::scope(|scope| {
        let left = scope.spawn(|| name.left.text());
        let right = scope.spawn(|| name.right.text());
        assert_eq!(left.join().unwrap(), "Namespace");
        assert_eq!(right.join().unwrap(), "Type");
    });
    assert!(parse_isolated_entity_name(&arena, "broken.").is_none());
    assert_eq!(root.kind, Kind::QualifiedName);
    let next = parse_isolated_entity_name(&arena, "next").unwrap();
    assert_eq!(next.text(), "next");
    assert_eq!(name.right.text(), "Type");
}

#[test]
#[cfg_attr(miri, ignore = "the smaller fixture set covers these paths under Miri")]
fn names_keywords_trivia_and_recovery_combinations_match() {
    let names = [
        "a",
        "await",
        "yield",
        "class",
        "default",
        "let",
        "π",
        "名称",
        r"\u0061",
        r"aw\u0061it",
        "#x",
        "",
        "123",
    ];
    let separators = [
        ".",
        " . ",
        ".\n",
        ".\r\n",
        ". /* c */ ",
        ". // c\n",
        ".\u{2028}",
        "..",
        "?.",
    ];
    for left in names {
        for right in names {
            for separator in separators {
                compare(&format!("{left}{separator}{right}"));
                compare(&format!("{left}{separator}{right} trailing"));
            }
        }
    }
}

#[test]
#[cfg_attr(
    miri,
    ignore = "large growth fixture; small native graphs are exercised under Miri"
)]
fn long_qualified_name_preserves_all_parent_links() {
    let text = std::iter::repeat_n("identifier", 4096)
        .collect::<Vec<_>>()
        .join(".");
    compare(&text);
}
