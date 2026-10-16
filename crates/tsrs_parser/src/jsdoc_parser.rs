//! `JSDoc` names, links and whitespace grammar shared by both frontend representations.
use crate::utilities::{is_keyword_or_punctuation, token_is_identifier_or_keyword};
use crate::{name_parser::NameParser, parser_3::is_reserved_word};
use tsrs_ast::Kind;
use tsrs_diagnostics::{self as diagnostics, Message};
use tsrs_scanner::{self as scanner, storage::TokenValue};

/// `JSDoc` lexical grammar. Callers provide comment bounds, context flags and the current token.
/// Complete comment/tag parsing, attachment and type expressions remain with the enclosing parser.
pub trait JSDocParser<'text>: NameParser<'text> {
    type TextList;

    fn next_token_jsdoc(&mut self) -> Kind;
    fn reset_scanner_pos(&mut self, pos: u32);
    fn re_scan_hash_token(&mut self);
    fn jsdoc_name_pos(node: Self::Node) -> u32;
    fn alloc_jsdoc_text(&self, text: Vec<&'text str>) -> Self::TextList;
    fn new_jsdoc_name_reference(&self, name: Self::Node) -> Self::Node;
    fn new_jsdoc_link(&self, name: Option<Self::Node>, text: Self::TextList) -> Self::Node;
    fn new_jsdoc_link_code(&self, name: Option<Self::Node>, text: Self::TextList) -> Self::Node;
    fn new_jsdoc_link_plain(&self, name: Option<Self::Node>, text: Self::TextList) -> Self::Node;

    /// Called after the enclosing `@see` parser's name-start probe.
    fn parse_jsdoc_name_reference(&mut self) -> Self::Node {
        let pos = self.node_pos();
        let has_brace = self.parse_optional(Kind::OpenBraceToken);
        let entity_name = self.parse_jsdoc_link_name();
        if has_brace {
            self.parse_expected_jsdoc(Kind::CloseBraceToken);
        }
        let full_start = self.scanner().token_full_start();
        self.reset_scanner_pos(full_start);
        self.next_token_jsdoc();
        let node = self.new_jsdoc_name_reference(entity_name.unwrap());
        self.finish_node(node, pos)
    }

    fn is_next_nonwhitespace_token_end_of_file(&mut self) -> bool {
        // We must use infinite lookahead, as there could be any number of newlines :(
        loop {
            self.next_token_jsdoc();
            if self.token() == Kind::EndOfFile {
                return true;
            }
            if !(self.token() == Kind::WhitespaceTrivia || self.token() == Kind::NewLineTrivia) {
                return false;
            }
        }
    }

    fn skip_whitespace(&mut self) {
        if self.token() == Kind::WhitespaceTrivia || self.token() == Kind::NewLineTrivia {
            if self.look_ahead(Self::is_next_nonwhitespace_token_end_of_file) {
                return;
                // Don't skip whitespace prior to EoF (or end of comment) - that shouldn't be included in any node's range
            }
        }
        while self.token() == Kind::WhitespaceTrivia || self.token() == Kind::NewLineTrivia {
            self.next_token_jsdoc();
        }
    }

    fn skip_whitespace_or_asterisk(&mut self) -> String {
        if self.token() == Kind::WhitespaceTrivia || self.token() == Kind::NewLineTrivia {
            if self.look_ahead(Self::is_next_nonwhitespace_token_end_of_file) {
                return String::new();
                // Don't skip whitespace prior to EoF (or end of comment) - that shouldn't be included in any node's range
            }
        }

        let mut preceding_line_break = self.scanner().has_preceding_line_break();
        let mut seen_line_break = false;
        let mut indents: Vec<&'text str> = Vec::with_capacity(4);
        while (preceding_line_break && self.token() == Kind::AsteriskToken)
            || self.token() == Kind::WhitespaceTrivia
            || self.token() == Kind::NewLineTrivia
        {
            indents.push(self.scanner().token_text());
            if self.token() == Kind::NewLineTrivia {
                preceding_line_break = true;
                seen_line_break = true;
                indents.clear();
            } else if self.token() == Kind::AsteriskToken {
                preceding_line_break = false;
            }
            self.next_token_jsdoc();
        }
        if seen_line_break {
            indents.concat()
        } else {
            String::new()
        }
    }

    fn parse_jsdoc_link(&mut self, start: u32) -> Option<Self::Node> {
        let state = self.mark();
        let Some(link_type) = self.parse_jsdoc_link_prefix() else {
            self.rewind(state);
            return None;
        };
        self.next_token_jsdoc();
        // start at token after link, then skip any whitespace
        self.skip_whitespace();
        let name = self.parse_jsdoc_link_name();
        let mut text: Vec<&'text str> = Vec::new();
        while self.token() != Kind::CloseBraceToken
            && self.token() != Kind::NewLineTrivia
            && self.token() != Kind::EndOfFile
        {
            text.push(self.scanner().token_text());
            self.next_token_jsdoc(); // Couldn't this be nextTokenCommentJSDoc?
        }
        let text = self.alloc_jsdoc_text(text);
        let create = match link_type {
            "link" => self.new_jsdoc_link(name, text),
            "linkcode" => self.new_jsdoc_link_code(name, text),
            _ => self.new_jsdoc_link_plain(name, text),
        };
        let end = self.scanner().token_end();
        Some(self.finish_node_with_end(create, start, end))
    }

    fn parse_jsdoc_link_name(&mut self) -> Option<Self::Node> {
        if token_is_identifier_or_keyword(self.token()) {
            let pos = self.node_pos();
            let mut name = self.parse_identifier_name();
            while self.parse_optional(Kind::DotToken) {
                let right = if self.token() == Kind::PrivateIdentifier {
                    self.create_missing_identifier()
                } else {
                    self.parse_identifier_name()
                };
                let qualified = self.new_qualified_name(name, right);
                name = self.finish_node(qualified, pos);
            }
            while self.token() == Kind::PrivateIdentifier {
                self.re_scan_hash_token();
                self.next_token_jsdoc();
                let right = self.parse_identifier();
                let qualified = self.new_qualified_name(name, right);
                name = self.finish_node(qualified, pos);
            }
            return Some(name);
        }
        None
    }

    fn parse_jsdoc_link_prefix(&mut self) -> Option<&'text str> {
        self.skip_whitespace_or_asterisk();
        if self.token() == Kind::OpenBraceToken
            && self.next_token_jsdoc() == Kind::AtToken
            && token_is_identifier_or_keyword(self.next_token_jsdoc())
        {
            let kind = self.scanner().token_value();
            if is_jsdoc_link_tag(kind.utf8()) {
                return Some(kind.utf8());
            }
        }
        None
    }

    fn parse_optional_jsdoc(&mut self, t: Kind) -> bool {
        if self.token() == t {
            self.next_token_jsdoc();
            return true;
        }
        false
    }

    fn parse_jsdoc_entity_name(
        &mut self,
        diagnostic_message: Option<&'static Message>,
    ) -> Self::Node {
        let mut entity = self.parse_jsdoc_identifier_name(diagnostic_message);
        if self.parse_optional(Kind::OpenBracketToken) {
            self.parse_expected(Kind::CloseBracketToken);
            // Note that y[] is accepted as an entity name, but the postfix brackets are not saved for checking.
            // Technically usejsdoc.org requires them for specifying a property of a type equivalent to Array<{ x: ...}>
            // but it's not worth it to enforce that restriction.
        }
        while self.parse_optional(Kind::DotToken) {
            let name = self.parse_jsdoc_identifier_name(Some(&diagnostics::Identifier_expected));
            if self.parse_optional(Kind::OpenBracketToken) {
                self.parse_expected(Kind::CloseBracketToken);
            }
            let pos = Self::jsdoc_name_pos(entity);
            let qualified = self.new_qualified_name(entity, name);
            entity = self.finish_node(qualified, pos);
        }
        entity
    }

    fn parse_jsdoc_identifier_name(
        &mut self,
        diagnostic_message: Option<&'static Message>,
    ) -> Self::Node {
        if !token_is_identifier_or_keyword(self.token()) {
            if let Some(diagnostic_message) = diagnostic_message {
                self.parse_error_at_current_token(diagnostic_message, &[]);
            } else if is_reserved_word(self.token()) {
                let text = self.scanner().token_text();
                self.parse_error_at_current_token(
                    &diagnostics::Identifier_expected_0_is_a_reserved_word_that_cannot_be_used_here,
                    &[&text],
                );
            }
            let identifier = self.new_identifier("");
            let pos = self.node_pos();
            return self.finish_node(identifier, pos);
        }
        let pos = self.scanner().token_start();
        let end = self.scanner().token_end();
        let text = self.scanner().token_value();
        self.next_token_jsdoc();
        let identifier = self.new_identifier(text.utf8());
        self.finish_node_with_end(identifier, pos, end)
    }

    fn parse_expected_jsdoc(&mut self, kind: Kind) -> bool {
        if self.token() == kind {
            self.next_token_jsdoc();
            return true;
        }
        assert!(
            is_keyword_or_punctuation(kind),
            "Invalid JSDoc kind: expected keyword or punctuation"
        );
        self.parse_error_at_current_token(
            &diagnostics::X_0_expected,
            &[&scanner::token_to_string(kind)],
        );
        false
    }
}

fn is_jsdoc_link_tag(kind: &str) -> bool {
    kind == "link" || kind == "linkcode" || kind == "linkplain"
}

impl JSDocParser<'static> for crate::Parser {
    type TextList = &'static [&'static str];

    fn next_token_jsdoc(&mut self) -> Kind {
        crate::Parser::next_token_jsdoc(self)
    }
    fn reset_scanner_pos(&mut self, pos: u32) {
        self.scanner.reset_pos(pos);
    }
    fn re_scan_hash_token(&mut self) {
        self.scanner.re_scan_hash_token();
        self.report_scan_errors();
    }
    fn jsdoc_name_pos(node: Self::Node) -> u32 {
        node.pos()
            .expect_source("parsed nodes have source positions")
    }
    fn alloc_jsdoc_text(&self, text: Vec<&'static str>) -> Self::TextList {
        tsrs_core::alloc_vec(text)
    }
    fn new_jsdoc_name_reference(&self, name: Self::Node) -> Self::Node {
        self.factory.new_jsdoc_name_reference(name)
    }
    fn new_jsdoc_link(&self, name: Option<Self::Node>, text: Self::TextList) -> Self::Node {
        self.factory.new_jsdoc_link(name, text)
    }
    fn new_jsdoc_link_code(&self, name: Option<Self::Node>, text: Self::TextList) -> Self::Node {
        self.factory.new_jsdoc_link_code(name, text)
    }
    fn new_jsdoc_link_plain(&self, name: Option<Self::Node>, text: Self::TextList) -> Self::Node {
        self.factory.new_jsdoc_link_plain(name, text)
    }
}
