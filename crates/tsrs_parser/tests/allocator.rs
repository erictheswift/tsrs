use tsrs_ast::{Kind, Node, SourceFileParseOptions};
use tsrs_core::arena::Region;
use tsrs_core::{P, ScriptKind, TextRange};
use tsrs_parser::raw::{parse_source_file, parse_source_file_embedded, parse_source_file_owned};

fn tree(node: P<Node>) -> Vec<(Kind, TextRange, String)> {
    fn visit(node: P<Node>, result: &mut Vec<(Kind, TextRange, String)>) {
        let text = match node.kind() {
            Kind::Identifier
            | Kind::StringLiteral
            | Kind::NumericLiteral
            | Kind::BigIntLiteral
            | Kind::RegularExpressionLiteral
            | Kind::TemplateHead
            | Kind::TemplateMiddle
            | Kind::TemplateTail
            | Kind::NoSubstitutionTemplateLiteral
            | Kind::JSDocText
            | Kind::JSDocLink => node.text().to_owned(),
            _ => String::new(),
        };
        result.push((node.kind(), node.loc(), text));
        node.for_each_child(&mut |child| {
            visit(child, result);
            false
        });
    }
    let mut result = Vec::new();
    visit(node, &mut result);
    result
}

#[test]
fn parse_entry_points_preserve_text_and_tree_after_parser_drop() {
    let cases = [
        (
            "/src/a.ts",
            "export {}; await f(); const s = `a${1_2}b`; const t = '\\u0061';",
            ScriptKind::TS,
        ),
        ("/src/a.tsx", "const v = <div title={'a\\nb'}>{`x${1_2}`}</div>;", ScriptKind::TSX),
        (
            "/src/a.js",
            "/** A {@link value} with text. @type {number} */\nconst value = 1_2;",
            ScriptKind::JS,
        ),
        ("/src/a.json", "{\"value\": [1, true, \"a\\nb\"]}", ScriptKind::JSON),
        ("/src/broken.ts", "const x = 'unterminated\nfunction f( {", ScriptKind::TS),
    ];
    for (name, text, kind) in cases {
        let region = Region::new(4096);
        let _scope = region.enter();
        let opts = SourceFileParseOptions { file_name: name.into(), ..Default::default() };
        let input = text.to_owned();
        // SAFETY: The test's region or thread arena outlives every use of the parsed AST.
        let borrowed = unsafe { parse_source_file(opts.clone(), &input, kind) };
        drop(input);
        // SAFETY: The test's region or thread arena outlives every use of the parsed AST.
        let owned = unsafe { parse_source_file_owned(opts.clone(), text.to_owned(), kind) };
        // SAFETY: The test's region or thread arena outlives every use of the parsed AST.
        let embedded = unsafe { parse_source_file_embedded(opts, text, kind) };
        for file in [borrowed, owned, embedded] {
            assert_eq!(file.text(), text);
            assert_eq!(tree(file.as_node()), tree(borrowed.as_node()), "{name}");
            assert_eq!(file.diagnostics.get().len(), borrowed.diagnostics.get().len());
        }
    }
}

#[test]
fn lazy_jsdoc_uses_the_file_region_after_the_initial_parser_is_gone() {
    let region = Region::new(4096);
    let scope = region.enter();
    // SAFETY: The test's region or thread arena outlives every use of the parsed AST.
    let file = unsafe {
        parse_source_file_owned(
            SourceFileParseOptions { file_name: "/src/a.ts".into(), ..Default::default() },
            "/** Description with text. @param x the value */\nfunction f(x: number) {}".into(),
            ScriptKind::TS,
        )
    };
    drop(scope);
    let docs = {
        let nested = Region::new(4096);
        let _nested_scope = nested.enter();
        file.statements.nodes()[0].jsdoc(Some(file.get()))
    };
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].as_jsdoc().comment().nodes()[0].text(), "Description with text.");
    assert!(Region::containing(docs.as_ptr().addr()).unwrap().ptr_eq(&region));
}

#[test]
fn lazy_members_reparse_with_a_new_allocator_guard() {
    tsrs_parser::enable_lazy_dts();
    let region = Region::new(4096);
    let _scope = region.enter();
    // SAFETY: The test's region or thread arena outlives every use of the parsed AST.
    let file = unsafe {
        parse_source_file_owned(
            SourceFileParseOptions { file_name: "/src/a.d.ts".into(), ..Default::default() },
            "interface I { value: string; method(x: number): number; }".into(),
            ScriptKind::TS,
        )
    };
    assert_eq!(file.lazy_lists.get().len(), 1);
    for record in file.lazy_lists.get() {
        let members = record.parse_now(file);
        assert_eq!(members.len(), 2);
        assert!(members.iter().all(|member| member.parent() == Some(record.owner())));
        assert_eq!(members[0].name().unwrap().text(), "value");
    }
}
