use tsrs_ast::{Kind, NodeFlags, SourceFileParseOptions, TokenFlags};
use tsrs_core::{P, ScriptKind, TextRange, arena::Region};
use tsrs_frontend_arena::{Diagnostic, FileArena, NameParser, Node, NodeData, TokenParser};

fn compare_node(native: &Node<'_>, legacy: P<tsrs_ast::Node>) {
    assert_eq!(native.kind, legacy.kind());
    assert_eq!(native.loc, legacy.loc());
    assert_eq!(native.flags, legacy.flags());
    let flags = match native.data {
        NodeData::StringLiteral(data) => {
            Some((data.token_flags, legacy.as_string_literal().token_flags()))
        }
        NodeData::NumericLiteral(data) => {
            Some((data.token_flags, legacy.as_numeric_literal().token_flags()))
        }
        NodeData::BigIntLiteral(data) => {
            Some((data.token_flags, legacy.as_big_int_literal().token_flags()))
        }
        NodeData::RegularExpressionLiteral(data) => Some((
            data.token_flags,
            legacy.as_regular_expression_literal().token_flags(),
        )),
        NodeData::NoSubstitutionTemplateLiteral(data) => {
            let old = legacy.as_no_substitution_template_literal();
            assert_eq!(data.raw_text.as_bytes(), old.raw_text().as_bytes());
            assert_eq!(data.template_flags, old.template_flags());
            Some((data.token_flags, old.token_flags()))
        }
        NodeData::TemplateHead(data) => {
            let old = legacy.as_template_head();
            assert_eq!(data.raw_text.as_bytes(), old.raw_text().as_bytes());
            assert_eq!(data.template_flags, old.template_flags());
            Some((data.token_flags, old.token_flags()))
        }
        NodeData::TemplateMiddle(data) => {
            let old = legacy.as_template_middle();
            assert_eq!(data.raw_text.as_bytes(), old.raw_text().as_bytes());
            assert_eq!(data.template_flags, old.template_flags());
            Some((data.token_flags, old.token_flags()))
        }
        NodeData::TemplateTail(data) => {
            let old = legacy.as_template_tail();
            assert_eq!(data.raw_text.as_bytes(), old.raw_text().as_bytes());
            assert_eq!(data.template_flags, old.template_flags());
            Some((data.token_flags, old.token_flags()))
        }
        NodeData::LiteralTypeNode(data) => {
            assert!(std::ptr::eq(data.literal.parent.unwrap(), native));
            compare_node(data.literal, legacy.as_literal_type_node().literal);
            None
        }
        NodeData::PrefixUnaryExpression(data) => {
            let old = legacy.as_prefix_unary_expression();
            assert_eq!(data.operator, old.operator);
            assert!(std::ptr::eq(data.operand.parent.unwrap(), native));
            compare_node(data.operand, old.operand);
            None
        }
        NodeData::KeywordExpression(_) => None,
        _ => panic!("unexpected literal fixture: {:?}", native.kind),
    };
    if let Some((native_flags, legacy_flags)) = flags {
        assert_eq!(native_flags, legacy_flags);
        assert_eq!(native.text().as_bytes(), legacy.text().as_bytes());
    }
}

fn compare_diagnostics(native: &[Diagnostic], legacy: &[P<tsrs_ast::Diagnostic>]) {
    assert_eq!(native.len(), legacy.len());
    for (native, legacy) in native.iter().zip(legacy) {
        assert_eq!(native.code, legacy.code());
        assert_eq!(native.loc, legacy.loc());
        assert_eq!(native.category, legacy.category());
        assert_eq!(native.arguments, legacy.message_args());
        assert_eq!(native.message, legacy.message().unwrap().text());
    }
}

#[test]
fn literals_and_keywords_match_source_file_parser() {
    for source in [
        "0",
        "  1_000;",
        "0xff",
        "0b101",
        "0o755",
        "0123",
        "1e+20",
        ".25",
        "123n",
        "0xFFn",
        "'plain'",
        r"'a\n\u{1f680}'",
        "'π名称'",
        "'unterminated",
        r"'\u00zz'",
        r"/a[bc]+/giu",
        "/unterminated",
        "/[a/",
        r"/\p{Letter}/u",
        "`plain`",
        "`a\r\nb`",
        r"`a\n\u{1f680}`",
        "`unterminated",
        r"`\u{110000}`",
        "true",
        "false",
        "null",
        "this",
        r"tr\u0075e",
    ] {
        for script_kind in [
            ScriptKind::TS,
            ScriptKind::JS,
            ScriptKind::TSX,
            ScriptKind::JSX,
        ] {
            let arena = FileArena::new();
            let mut diagnostics = Vec::new();
            let root = arena.build(|factory| {
                let mut parser = factory.parser(source, script_kind);
                parser.next_token();
                if matches!(parser.token(), Kind::SlashToken | Kind::SlashEqualsToken) {
                    parser.re_scan_slash_token();
                }
                // These are the same caller preconditions as parse_primary_expression.
                if parser.token() == Kind::NoSubstitutionTemplateLiteral
                    && parser
                        .scanner()
                        .token_flags()
                        .intersects(TokenFlags::IsInvalid)
                {
                    parser.re_scan_template_token(false);
                }
                let node = if matches!(
                    parser.token(),
                    Kind::TrueKeyword | Kind::FalseKeyword | Kind::NullKeyword | Kind::ThisKeyword
                ) {
                    parser.parse_keyword_expression()
                } else {
                    parser.parse_literal_expression()
                };
                diagnostics = parser.into_diagnostics();
                node
            });
            let region = Region::new(4096);
            let _scope = region.enter();
            let old = tsrs_parser::parse_source_file(
                SourceFileParseOptions {
                    file_name: "/tokens.ts".to_owned(),
                    ..Default::default()
                },
                source,
                script_kind,
            );
            let expression = old.statements.nodes()[0].expression().unwrap();
            compare_node(root, expression);
            compare_diagnostics(&diagnostics, old.diagnostics.get());
        }
    }
}

#[test]
fn literal_types_match_source_file_parser_and_link_parents() {
    for literal in [
        "-1", "-1n", "- 0xff", "1_000", "'hello'", "true", "false", "null", "`text`",
    ] {
        let source = format!("type T = {literal};");
        let arena = FileArena::new();
        let root = arena.build(|factory| {
            let mut parser = factory.parser(&source, ScriptKind::TS);
            for _ in 0..4 {
                parser.next_token();
            }
            assert_eq!(parser.diagnostics(), []);
            parser.parse_literal_type_node(literal.starts_with('-'))
        });
        let region = Region::new(4096);
        let _scope = region.enter();
        let old = tsrs_parser::parse_source_file(
            SourceFileParseOptions {
                file_name: "/tokens.ts".to_owned(),
                ..Default::default()
            },
            &source,
            ScriptKind::TS,
        );
        assert_eq!(old.diagnostics.get(), []);
        compare_node(root, old.statements.nodes()[0].type_node().unwrap());
    }
}

#[test]
fn template_parts_match_source_file_parser_including_rescan_errors() {
    for source in [
        r"`a\n${x}b\t${y}c`",
        "`a\r\n${x}π${y}🚀`",
        r"tag`\u{110000}${x}\u00zz${y}\8`",
        r"`\u{110000}${x}\u00zz${y}\8`",
        "`a${x}tail",
        "`a${x",
    ] {
        let region = Region::new(4096);
        let _scope = region.enter();
        let old = tsrs_parser::parse_source_file(
            SourceFileParseOptions {
                file_name: "/tokens.ts".to_owned(),
                ..Default::default()
            },
            source,
            ScriptKind::TS,
        );
        let mut parts = Vec::new();
        fn visit(node: P<tsrs_ast::Node>, parts: &mut Vec<P<tsrs_ast::Node>>) {
            if matches!(
                node.kind(),
                Kind::TemplateHead | Kind::TemplateMiddle | Kind::TemplateTail
            ) {
                parts.push(node);
            }
            node.for_each_child(&mut |child| {
                visit(child, parts);
                false
            });
        }
        visit(old.as_node(), &mut parts);
        assert!(parts.len() >= 2);
        for (index, &part) in parts.iter().enumerate() {
            let arena = FileArena::new();
            let mut diagnostics = Vec::new();
            let root = arena.build(|factory| {
                let mut parser = factory.parser(source, ScriptKind::TS);
                parser.next_token();
                let tagged = source.starts_with("tag");
                if tagged {
                    parser.parse_identifier();
                }
                let mut node = parser.parse_template_head(tagged);
                for _ in 0..index {
                    parser.parse_identifier();
                    node = parser.parse_literal_of_template_span(tagged);
                }
                diagnostics = parser.into_diagnostics();
                node
            });
            compare_node(root, part);
            if index == parts.len() - 1 {
                compare_diagnostics(&diagnostics, old.diagnostics.get());
            }
        }
    }
}

// These tests use only native graph operations and run under strict-provenance Miri as well.
mod native {
    use super::*;

    #[test]
    fn token_recovery_preserves_position_flags_and_diagnostic_order() {
        let arena = FileArena::new();
        let mut diagnostics = Vec::new();
        let root = arena.build(|factory| {
            let mut parser = factory.parser("  ; :", ScriptKind::JSON);
            assert_eq!(parser.next_token(), Kind::SemicolonToken);
            assert!(parser.parse_expected_without_advancing(Kind::SemicolonToken));
            assert_eq!(parser.token(), Kind::SemicolonToken);
            assert!(parser.parse_optional_token(Kind::CommaToken).is_none());
            let missing = parser.parse_expected_token(Kind::CommaToken);
            assert_eq!(missing.loc(), TextRange::new(0, 0));
            assert!(missing.flags().intersects(NodeFlags::ThisNodeHasError));
            assert_eq!(parser.diagnostics()[0].loc, Some(TextRange::new(2, 3)));
            assert!(!parser.parse_expected_with_diagnostic(
                Kind::OpenBraceToken,
                Some(&tsrs_diagnostics::Identifier_expected),
                true
            ));
            assert_eq!(parser.diagnostics().len(), 1);
            assert_eq!(parser.token(), Kind::SemicolonToken);
            let semicolon = parser
                .parse_optional_token_jsdoc(Kind::SemicolonToken)
                .unwrap();
            assert_eq!(semicolon.loc(), TextRange::new(0, 3));
            assert!(semicolon.flags().intersects(NodeFlags::ThisNodeHasError));
            let colon = parser.parse_expected_token_jsdoc(Kind::ColonToken);
            assert_eq!(colon.loc(), TextRange::new(3, 5));
            assert!(!colon.flags().intersects(NodeFlags::ThisNodeHasError));
            assert!(!parser.parse_optional(Kind::CommaToken));
            assert!(!parser.parse_expected(Kind::CloseBraceToken));
            let missing_end = parser.parse_expected_token_jsdoc(Kind::CloseBraceToken);
            assert_eq!(missing_end.loc(), TextRange::new(5, 5));
            diagnostics = parser.into_diagnostics();
            colon
        });
        assert_eq!(root.kind, Kind::ColonToken);
        assert_eq!(root.flags, NodeFlags::JavaScriptFile | NodeFlags::JsonFile);
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].arguments, [","]);
        assert_eq!(diagnostics[1].arguments, ["}"]);
        drop(arena);
        assert_eq!(diagnostics[1].loc, Some(TextRange::new(5, 5)));
    }

    #[test]
    fn literals_borrow_cooked_bytes_and_survive_input_and_region_release() {
        let arena = FileArena::new();
        let region = Region::new(4096);
        let scope = region.enter();
        let mut roots = Vec::new();
        for source in [
            r"'\ud800'",
            r"'\udfff'",
            r"'\ud83d\ude80'",
            r"'plain\ntext'",
            "1_000",
            "123n",
            "/a+/u",
            "`hello`",
        ] {
            let source = source.to_owned();
            let mut address = 0;
            let node = arena.build(|factory| {
                let mut parser = factory.parser(&source, ScriptKind::TS);
                parser.next_token();
                if parser.token() == Kind::SlashToken {
                    parser.re_scan_slash_token();
                }
                address = parser.scanner().token_value().as_ptr().addr();
                let node = parser.parse_literal_expression();
                assert_eq!(parser.diagnostics(), []);
                node
            });
            drop(source);
            assert_eq!(node.text().as_bytes().as_ptr().addr(), address);
            roots.push(node);
        }
        assert_eq!(region.used_bytes(), 0);
        drop(scope);
        drop(region);
        assert_eq!(roots[0].text().as_bytes(), [0xed, 0xa0, 0x80]);
        assert_eq!(roots[1].text().as_bytes(), [0xed, 0xbf, 0xbf]);
        assert_eq!(roots[2].text(), "🚀");
        assert!(roots[0].text().as_str().is_err());
        std::thread::scope(|scope| {
            let first = scope.spawn(|| roots[3].text());
            let second = scope.spawn(|| roots[4].text());
            // Growth must not invalidate existing nodes or text read on other threads.
            arena.build(|factory| {
                let source = format!(
                    "'{}'",
                    "\\u0061".repeat(if cfg!(miri) { 64 } else { 32768 })
                );
                let mut parser = factory.parser(&source, ScriptKind::TS);
                parser.next_token();
                parser.parse_literal_expression()
            });
            assert_eq!(first.join().unwrap(), "plain\ntext");
            assert_eq!(second.join().unwrap(), "1000");
        });
        assert_eq!(roots[0].text().as_bytes(), [0xed, 0xa0, 0x80]);
    }

    #[test]
    fn template_raw_and_cooked_storage_survive_parser_drop() {
        let arena = FileArena::new();
        let mut cooked_address = 0;
        let mut raw_address = 0;
        let root = arena.build(|factory| {
            let source = String::from(r"`\ud800\n${x}tail`");
            let mut parser = factory.parser(&source, ScriptKind::TS);
            drop(source);
            parser.next_token();
            cooked_address = parser.scanner().token_value().as_ptr().addr();
            raw_address = parser.scanner().token_text().as_ptr().addr() + 1;
            parser.parse_template_head(false)
        });
        let NodeData::TemplateHead(head) = root.data else {
            panic!("template head")
        };
        assert_eq!(head.text.as_bytes(), [0xed, 0xa0, 0x80, b'\n']);
        assert_eq!(head.raw_text, r"\ud800\n");
        assert_eq!(head.text.as_bytes().as_ptr().addr(), cooked_address);
        assert_eq!(head.raw_text.as_bytes().as_ptr().addr(), raw_address);
    }

    #[test]
    fn template_rescans_preserve_native_text_and_owned_errors() {
        for tagged in [false, true] {
            let arena = FileArena::new();
            let region = Region::new(4096);
            let scope = region.enter();
            let mut diagnostics = Vec::new();
            let root = arena.build(|factory| {
                let mut parser = factory.parser(r"`a${x}\udfff${y}\u00zz`", ScriptKind::TS);
                parser.next_token();
                let head = parser.parse_template_head(tagged);
                parser.parse_identifier();
                let middle = parser.parse_literal_of_template_span(tagged);
                parser.parse_identifier();
                let tail = parser.parse_literal_of_template_span(tagged);
                assert_eq!(parser.token(), Kind::EndOfFile);
                assert_eq!(head.text(), "a");
                assert_eq!(middle.text().as_bytes(), [0xed, 0xbf, 0xbf]);
                assert_eq!(middle.kind(), Kind::TemplateMiddle);
                diagnostics = parser.into_diagnostics();
                tail
            });
            assert_eq!(region.used_bytes(), 0);
            drop(scope);
            drop(region);
            let NodeData::TemplateTail(tail) = root.data else {
                panic!("template tail")
            };
            assert_eq!(tail.raw_text, r"\u00zz");
            assert_eq!(root.flags.intersects(NodeFlags::ThisNodeHasError), !tagged);
            assert_eq!(diagnostics.is_empty(), tagged);
            drop(arena);
            if !tagged {
                assert_eq!(
                    diagnostics[0].code,
                    tsrs_diagnostics::Hexadecimal_digit_expected.code()
                );
            }
        }
    }

    #[test]
    fn negative_literal_parent_links_survive_publication() {
        let arena = FileArena::new();
        let root = arena.build(|factory| {
            let mut parser = factory.parser("-1n", ScriptKind::TS);
            parser.next_token();
            parser.parse_literal_type_node(true)
        });
        let NodeData::LiteralTypeNode(ty) = root.data else {
            panic!("literal type")
        };
        let NodeData::PrefixUnaryExpression(prefix) = ty.literal.data else {
            panic!("negative literal")
        };
        assert!(std::ptr::eq(ty.literal.parent.unwrap(), root));
        assert!(std::ptr::eq(prefix.operand.parent.unwrap(), ty.literal));
        assert_eq!(prefix.operand.text(), "1n");
        assert_eq!(root.loc, TextRange::new(0, 3));
        assert_eq!(prefix.operand.loc, TextRange::new(1, 3));
    }
}
