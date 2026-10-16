use std::fmt::Write as _;
use tsrs_ast::Kind;
use tsrs_core::{LanguageVariant, ScriptTarget, arena::Region};
use tsrs_frontend_arena::FileArena;
use tsrs_scanner::{
    Scanner,
    storage::{ScannerStorage, TokenBuffer, TokenValue},
};

#[derive(Debug, PartialEq, Eq)]
struct Token {
    kind: Kind,
    start: u32,
    end: u32,
    flags: tsrs_ast::TokenFlags,
    value: Vec<u8>,
    errors: Vec<(i32, u32, u32, Vec<String>)>,
}
fn trace<'a, S: ScannerStorage<'a>>(mut scanner: Scanner<'a, S>, mode: u8) -> Vec<Token> {
    scanner.set_skip_trivia(mode != 2);
    scanner.set_on_error(true);
    scanner.set_script_target(if mode == 4 {
        ScriptTarget::ES5
    } else {
        ScriptTarget::ESNext
    });
    scanner.set_language_variant(LanguageVariant::JSX);
    let mut result = Vec::new();
    let mut stack = Vec::new();
    loop {
        let mut kind = scanner.scan();
        if mode >= 3 {
            match kind {
                Kind::SlashToken | Kind::SlashEqualsToken => {
                    kind = scanner.re_scan_slash_token(true);
                }
                Kind::TemplateHead => stack.push(true),
                Kind::OpenBraceToken => stack.push(false),
                Kind::CloseBraceToken if stack.pop() == Some(true) => {
                    kind = scanner.re_scan_template_token(false);
                    if kind == Kind::TemplateMiddle {
                        stack.push(true);
                    }
                }
                Kind::GreaterThanToken => kind = scanner.re_scan_greater_than_token(),
                _ => {}
            }
        }
        result.push(Token {
            kind,
            start: scanner.token_start(),
            end: scanner.token_end(),
            flags: scanner.token_flags(),
            value: scanner.token_value().as_bytes().to_vec(),
            errors: scanner
                .take_errors()
                .into_iter()
                .map(|e| (e.message.code(), e.start, e.length, e.args))
                .collect(),
        });
        if kind == Kind::EndOfFile {
            break;
        }
    }
    // Include directive order and spans in the comparison, without using native or legacy AST nodes.
    for directive in scanner.comment_directives() {
        result.push(Token {
            kind: Kind::SingleLineCommentTrivia,
            start: directive.loc.pos().expect_source("directive position"),
            end: directive.loc.end().expect_source("directive end"),
            flags: tsrs_ast::TokenFlags::None,
            value: format!("{:?}", directive.kind).into_bytes(),
            errors: Vec::new(),
        });
    }
    result
}
fn compare(source: &str) {
    let arena = FileArena::new();
    let region = Region::new(4096);
    let scope = region.enter();
    for mode in 1..=4 {
        let before = region.used_bytes();
        let native = trace(arena.scanner(source), mode);
        assert_eq!(
            region.used_bytes(),
            before,
            "native scanner allocated in the legacy region"
        );
        let mut legacy = Scanner::new();
        legacy.set_text(source);
        assert_eq!(native, trace(legacy, mode));
    }
    drop(scope);
    drop(region);
}

#[test]
fn native_and_legacy_tokens_errors_and_directives_match() {
    for source in [
        "// @ts-ignore\nconst café = 1_000 + 0xFF + 0b101n + 1.2e+003; /* @ts-expect-error */ café;",
        r#"'\ud800' "\u{dc00}" "\ud83d\ude80" `line\n${name}\ud800`"#,
        r"/(?<name>a)\k<name>/u /[\ud800-\udfff]/ /[😀-🦀]/u /[\q{ab}--a]/v /\k<missing>/u",
        "'unterminated\n1__2 0x_1 123n 0123 000123n \\u0061 \\u{62} `first\r\nsecond`",
        r"/** @param {Array.<T>} a */ function f(a) { return a?.[0] ?? /a/g; }",
    ] {
        compare(source);
    }
}

#[test]
fn native_jsx_jsdoc_and_regexp_rescans() {
    let arena = FileArena::new();
    let mut scanner = arena.scanner(r#"tag-\u0061 "raw\ud800" > café </"#);
    assert_eq!(scanner.scan(), Kind::Identifier);
    assert_eq!(scanner.scan_jsx_identifier(), Kind::Identifier);
    assert_eq!(scanner.token_value(), b"tag-a");
    assert_eq!(scanner.scan_jsx_attribute_value(), Kind::StringLiteral);
    assert_eq!(scanner.token_value(), br"raw\ud800");
    assert_eq!(scanner.scan(), Kind::GreaterThanToken);
    assert_eq!(scanner.scan_jsx_token(), Kind::JsxText);
    assert_eq!(scanner.token_value(), " café ".as_bytes());
    assert_eq!(scanner.scan_jsx_token(), Kind::LessThanSlashToken);

    let mut scanner = arena.scanner(r"@param {Foo.<T>} café-description");
    while scanner.scan_jsdoc_token() != Kind::EndOfFile {}
    assert_eq!(scanner.token_value(), "café-description".as_bytes());
    let mut scanner = arena.scanner("description {T}");
    assert_eq!(
        scanner.scan_jsdoc_comment_text_token(false),
        Kind::JSDocCommentTextToken
    );
    assert_eq!(scanner.token_value(), b"description ");

    let mut scanner = arena.scanner(r"/[\udfff-\ud800]/u");
    scanner.set_on_error(true);
    scanner.set_script_target(ScriptTarget::ESNext);
    assert_eq!(scanner.scan(), Kind::SlashToken);
    assert_eq!(
        scanner.re_scan_slash_token(true),
        Kind::RegularExpressionLiteral
    );
    let errors = scanner.take_errors();
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].message.code(),
        tsrs_diagnostics::Range_out_of_order_in_character_class.code()
    );
}

#[test]
fn values_snapshots_and_reset_survive_legacy_region_release() {
    let arena = FileArena::new();
    let mut scanner;
    let value;
    let snapshot;
    let directives;
    {
        let region = Region::new(4096);
        let scope = region.enter();
        scanner = arena.scanner("// @ts-ignore\n'\\ud800' 1_000 1_000");
        assert_eq!(scanner.scan(), Kind::StringLiteral);
        value = scanner.token_value();
        snapshot = scanner.mark();
        directives = scanner.comment_directives();
        assert_eq!(scanner.scan(), Kind::NumericLiteral);
        assert_eq!(scanner.token_value(), b"1000");
        assert_eq!(region.used_bytes(), 0);
        drop(scope);
        drop(region);
    }
    assert_eq!(value, &[0xed, 0xa0, 0x80]);
    assert_eq!(directives.len(), 1);
    scanner.rewind(snapshot);
    assert_eq!(scanner.token_value(), value);
    assert_eq!(scanner.scan(), Kind::NumericLiteral);
    assert_eq!(scanner.scan(), Kind::NumericLiteral);
    assert_eq!(scanner.token_value(), b"1000");
    scanner.reset();
    scanner.set_text("2_000");
    assert_eq!(scanner.scan(), Kind::NumericLiteral);
    assert_eq!(scanner.token_value(), b"2000");
    drop(scanner);
    std::thread::scope(|scope| {
        scope
            .spawn(|| assert_eq!(value, &[0xed, 0xa0, 0x80]))
            .join()
            .unwrap();
    });
}

#[test]
fn every_lone_surrogate_is_preserved_as_bytes() {
    let arena = FileArena::new();
    let mut source = String::from("\"");
    let mut expected = Vec::new();
    // A separator prevents adjacent high/low code units from forming a surrogate pair.
    for code in 0xd800u32..=0xdfff {
        write!(source, "\\u{code:04x}|").unwrap();
        expected.extend_from_slice(&[
            0xed,
            0x80 | ((code >> 6) & 0x3f) as u8,
            0x80 | (code & 0x3f) as u8,
            b'|',
        ]);
    }
    source.push('"');
    let mut scanner = arena.scanner(&source);
    assert_eq!(scanner.scan(), Kind::StringLiteral);
    assert_eq!(scanner.token_value(), expected);
    assert!(std::str::from_utf8(scanner.token_value()).is_err());
}

#[test]
fn token_buffer_growth_and_other_allocations_are_disjoint() {
    let arena = FileArena::new();
    let mut a = (&arena).buffer(1);
    a.push_str("a");
    a.push_rune(0xd800);
    let a = a.finish();
    let mut b = (&arena).buffer(0);
    let text = "é".repeat(40_000);
    b.push_str(&text);
    b.push_rune(0x1f680);
    let b = b.finish();
    let mut scanner = arena.scanner("'value'");
    scanner.scan();
    assert_eq!(a, &[b'a', 0xed, 0xa0, 0x80]);
    assert_eq!(&b[..text.len()], text.as_bytes());
    assert_eq!(&b[text.len()..], "🚀".as_bytes());
    assert_eq!(scanner.token_value(), b"value");
}

#[test]
#[ignore = "requires an external public corpus manifest in TSRS_LIFETIME_MANIFEST"]
fn public_corpus_scanner_storage_parity() {
    let manifest =
        std::fs::read_to_string(std::env::var("TSRS_LIFETIME_MANIFEST").unwrap()).unwrap();
    for path in manifest.lines() {
        compare(&std::fs::read_to_string(path).unwrap());
    }
}
