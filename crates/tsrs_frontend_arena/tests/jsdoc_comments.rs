use tsrs_ast::{Kind, NodeFlags, SourceFileParseOptions};
use tsrs_core::{P, ScriptKind, arena::Region};
use tsrs_frontend_arena::{
    ContextParser, Diagnostic, FileArena, JSDocParser, ListParser, Node, NodeData, Parser,
    TokenParser, construction,
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
        NodeData::JSDocText(data) => Some((data.text, legacy.as_jsdoc_text().text())),
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
    let comments = match native.data {
        NodeData::JSDocUnknownTag(data) => Some(data.comment),
        NodeData::JSDocPublicTag(data) => Some(data.comment),
        NodeData::JSDocPrivateTag(data) => Some(data.comment),
        NodeData::JSDocProtectedTag(data) => Some(data.comment),
        NodeData::JSDocReadonlyTag(data) => Some(data.comment),
        NodeData::JSDocOverrideTag(data) => Some(data.comment),
        NodeData::JSDocDeprecatedTag(data) => Some(data.comment),
        NodeData::JSDocSeeTag(data) => Some(data.comment),
        _ => None,
    };
    if let Some(comments) = comments {
        let old = legacy.comment_list();
        assert_eq!(comments.is_some(), old.is_some());
        if let Some(list) = comments {
            let old = old.unwrap();
            assert_eq!(list.loc, old.loc());
            assert_eq!(list.has_trailing_comma, old.has_trailing_comma());
            assert_eq!(list.nodes.len(), old.nodes().len());
            assert!(!list.is_missing);
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

// The unmigrated dispatcher supplies the @ token, tag identifier, margin and constructor.
fn parse_tag<'build, 'ast>(
    parser: &mut Parser<'build, 'ast>,
    kind: Kind,
    margin: u32,
) -> &'build construction::Node<'build, 'ast> {
    assert_eq!(parser.token(), Kind::AtToken);
    let start = parser.scanner().token_start();
    parser.next_token_jsdoc();
    let name = parser.parse_jsdoc_identifier_name(Some(&tsrs_diagnostics::Identifier_expected));
    let indent = parser.skip_whitespace_or_asterisk();
    match kind {
        Kind::JSDocUnknownTag => parser.parse_unknown_tag(start, name, margin, &indent),
        Kind::JSDocSeeTag => parser.parse_see_tag(start, name, margin, &indent),
        _ => parser.parse_simple_tag(
            start,
            |p, name, comments| {
                let factory = p.factory();
                match kind {
                    Kind::JSDocPublicTag => factory.new_jsdoc_public_tag(name, comments),
                    Kind::JSDocPrivateTag => factory.new_jsdoc_private_tag(name, comments),
                    Kind::JSDocProtectedTag => factory.new_jsdoc_protected_tag(name, comments),
                    Kind::JSDocReadonlyTag => factory.new_jsdoc_readonly_tag(name, comments),
                    Kind::JSDocOverrideTag => factory.new_jsdoc_override_tag(name, comments),
                    Kind::JSDocDeprecatedTag => factory.new_jsdoc_deprecated_tag(name, comments),
                    _ => panic!("unexpected simple tag"),
                }
            },
            name,
            margin,
            &indent,
        ),
    }
}

fn check_fixture(source: &str, kind: Kind, script_kind: ScriptKind) {
    let start = source.find('@').unwrap();
    let line_start = source[..start].rfind('\n').map_or(0, |i| i + 1);
    let end = source.find("*/").unwrap();
    let arena = FileArena::new();
    let mut diagnostics = Vec::new();
    let tag = arena.build(|factory| {
        let mut parser = factory.parser(&source[..end], script_kind);
        parser.set_context_flags(NodeFlags::JSDoc, true);
        parser.reset_scanner_pos(start as u32);
        parser.next_token_jsdoc();
        let tag = parse_tag(&mut parser, kind, (start - line_start) as u32);
        assert_eq!(parser.jsdoc_tag_comments_space().as_slice(), &[] as &[&str]);
        assert!(parser.jsdoc_tag_comments_parts_space().is_empty());
        assert!(parser.node_stack().is_empty());
        diagnostics = parser.into_diagnostics();
        tag
    });
    let region = Region::new(4096);
    let _scope = region.enter();
    let file = tsrs_parser::parse_source_file(
        SourceFileParseOptions {
            file_name: "/comments.ts".into(),
            ..Default::default()
        },
        source,
        script_kind,
    );
    let doc = file.statements.nodes()[0].jsdoc(Some(file.get()))[0];
    compare(tag, doc.as_jsdoc().tags.unwrap().nodes()[0]);
    if script_kind == ScriptKind::JS {
        compare_diagnostics(&diagnostics, file.jsdoc_diagnostics());
    }
}

#[test]
fn simple_and_unknown_tag_comments_match_source_file_parser() {
    for (name, kind) in [
        ("public", Kind::JSDocPublicTag),
        ("private", Kind::JSDocPrivateTag),
        ("protected", Kind::JSDocProtectedTag),
        ("readonly", Kind::JSDocReadonlyTag),
        ("override", Kind::JSDocOverrideTag),
        ("deprecated", Kind::JSDocDeprecatedTag),
        ("custom", Kind::JSDocUnknownTag),
    ] {
        for tail in [
            "",
            " description",
            " π description \t\u{feff}",
            "\n * first\n *   indented\n * last",
            "\n *\n * spaced\n *\n * ",
            " before {@link Foo} after",
            " {@linkcode A.B | label} and {@linkplain Foo#member}",
            " {@link Foo.} malformed",
            " {@unknown text} braces",
            " `inline @tag {@link Hidden}` text",
            "\n * ```ts\n * @notATag {@link Hidden}\n * ```\n * after",
            "\n * ````\n * @hidden\n * ````\n * text\n * @other ignored",
            " first @next second",
            " email@example.com",
            "\n *      initial indent",
        ] {
            let source = format!("/** @{name}{tail} */\nconst value = 0;");
            for script_kind in [ScriptKind::JS, ScriptKind::TS] {
                check_fixture(&source, kind, script_kind);
            }
        }
    }
}

#[test]
fn see_tag_names_urls_and_comments_match_source_file_parser() {
    for tail in [
        "",
        " Foo description",
        " {A.B#member} {@link Other}",
        " https://example.com/path label",
        " http://example.com",
        " {@link Target} description",
        " {Foo description",
        " Foo.#member description",
        " 123 text",
        "\n * π.名称\n * description",
        " Foo `@tag` description\n * @next ignored",
    ] {
        let source = format!("/**\n * @see{tail}\n */\nconst value = 0;");
        for script_kind in [ScriptKind::JS, ScriptKind::TS] {
            check_fixture(&source, Kind::JSDocSeeTag, script_kind);
        }
    }
}

mod native {
    use super::*;

    fn comment_text(node: &Node<'_>) -> Vec<u8> {
        let NodeData::JSDocText(text) = node.data else {
            panic!("comment text")
        };
        text.text
            .iter()
            .flat_map(|part| part.as_bytes())
            .copied()
            .collect()
    }

    #[test]
    fn comments_survive_rewind_buffer_reuse_growth_and_source_release() {
        let region = Region::new(4096);
        let scope = region.enter();
        let arena = FileArena::new();
        let root = {
            let source =
                String::from("before {@linkcode F\\u006fo#member | π} after\n@next ignored");
            arena.build(|factory| {
                let mut parser = factory.parser(&source, ScriptKind::JS);
                parser.set_context_flags(NodeFlags::JSDoc, true);
                parser.next_token_jsdoc();
                let checkpoint = parser.mark();
                let retained = parser.parse_tag_comments(0, None).unwrap();
                assert_eq!(parser.jsdoc_tag_comments_space().as_slice(), &[] as &[&str]);
                assert!(parser.jsdoc_tag_comments_parts_space().is_empty());
                assert_eq!(parser.token(), Kind::AtToken);
                parser.rewind(checkpoint);
                let replayed = parser.parse_tag_comments(0, None).unwrap();
                assert!(!std::ptr::eq(retained, replayed));
                assert_eq!(retained.loc(), replayed.loc());
                assert_eq!(retained.nodes().len(), 3);
                assert_eq!(parser.diagnostics(), []);
                let first = parser.factory().new_block(retained, false);
                parser.finish_node(first, 0);
                let second = parser.factory().new_block(replayed, false);
                parser.finish_node(second, 0);
                let list = parser.factory().new_node_list(&[first, second]);
                let root = parser.factory().new_block(list, false);
                parser.finish_node(root, 0)
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
                    let NodeData::Block(root) = root.data else {
                        panic!()
                    };
                    for block in root.statements.nodes {
                        let NodeData::Block(data) = block.data else {
                            panic!()
                        };
                        let nodes = data.statements.nodes;
                        assert_eq!(comment_text(nodes[0]), b"before ");
                        assert_eq!(comment_text(nodes[2]), b" after");
                        let NodeData::JSDocLinkCode(link) = nodes[1].data else {
                            panic!()
                        };
                        let NodeData::QualifiedName(name) = link.name.unwrap().data else {
                            panic!()
                        };
                        assert_eq!(name.left.text().as_bytes(), b"Foo");
                        assert_eq!(name.right.text().as_bytes(), b"member");
                        for node in nodes {
                            assert!(std::ptr::eq(node.parent.unwrap(), *block));
                        }
                    }
                });
            }
        });
    }

    #[test]
    fn trailing_indent_text_is_copied_into_the_file_arena() {
        let arena = FileArena::new();
        let root = arena.build(|factory| {
            let mut parser = factory.parser("body", ScriptKind::TS);
            parser.next_token_jsdoc();
            let mut indent = String::from("     ");
            let comments = parser
                .parse_trailing_tag_comments(0, 0, 1, &indent)
                .unwrap();
            indent.clear();
            drop(indent);
            let root = parser.factory().new_block(comments, false);
            parser.finish_node(root, 0)
        });
        let NodeData::Block(data) = root.data else {
            panic!()
        };
        assert_eq!(comment_text(data.statements.nodes[0]), b"    body");
    }

    #[test]
    fn fenced_code_keeps_links_and_tags_as_text_until_the_next_tag() {
        let source = "```\n @inside {@link Hidden}\n```\n@outside";
        let arena = FileArena::new();
        let root = arena.build(|factory| {
            let mut parser = factory.parser(source, ScriptKind::TS);
            parser.next_token_jsdoc();
            let comments = parser.parse_tag_comments(0, None).unwrap();
            assert_eq!(comments.nodes().len(), 1);
            assert_eq!(parser.token(), Kind::AtToken);
            assert_eq!(
                parser.scanner().token_end(),
                source.find("@outside").unwrap() as u32
            );
            // The comment routine rewinds to the @ itself for the enclosing comment parser.
            assert_eq!(parser.next_token_jsdoc(), Kind::AtToken);
            assert_eq!(parser.next_token_jsdoc(), Kind::Identifier);
            assert_eq!(parser.scanner().token_text(), "outside");
            let root = parser.factory().new_block(comments, false);
            parser.finish_node(root, 0)
        });
        let NodeData::Block(data) = root.data else {
            panic!()
        };
        assert_eq!(
            comment_text(data.statements.nodes[0]),
            b"```\n @inside {@link Hidden}\n```"
        );
    }

    #[test]
    fn empty_comments_and_failed_links_leave_scratch_buffers_reusable() {
        let arena = FileArena::new();
        arena.build(|factory| {
            let mut parser = factory.parser(
                " \n @first {@unknown broken} text\n @second {@link Good}",
                ScriptKind::TS,
            );
            parser.next_token_jsdoc();
            assert!(parser.parse_tag_comments(0, None).is_none());
            assert_eq!(parser.jsdoc_tag_comments_space().as_slice(), &[] as &[&str]);
            assert!(parser.jsdoc_tag_comments_parts_space().is_empty());
            parser.next_token_jsdoc();
            let first = parse_tag(&mut parser, Kind::JSDocUnknownTag, 1);
            assert_eq!(parser.jsdoc_tag_comments_space().as_slice(), &[] as &[&str]);
            assert!(parser.jsdoc_tag_comments_parts_space().is_empty());
            parser.next_token_jsdoc();
            let second = parse_tag(&mut parser, Kind::JSDocUnknownTag, 1);
            assert_eq!(parser.diagnostics(), []);
            assert_eq!(
                first
                    .as_jsdoc_unknown_tag()
                    .comment()
                    .unwrap()
                    .nodes()
                    .len(),
                1
            );
            assert_eq!(
                second
                    .as_jsdoc_unknown_tag()
                    .comment()
                    .unwrap()
                    .nodes()
                    .len(),
                2
            );
            assert!(parser.node_stack().is_empty());
            let list = parser.factory().new_node_list(&[first, second]);
            let root = parser.factory().new_block(list, false);
            parser.finish_node_with_end(root, 0, 0)
        });
    }
}
