//! Shared token, literal, and template-part grammar for both frontend representations.
use crate::utilities::is_keyword_or_punctuation;
use std::fmt::Display;
use tsrs_ast::{Kind, TokenFlags};
use tsrs_core::TextRange;
use tsrs_diagnostics::{self as diagnostics, Message};
use tsrs_scanner::{self as scanner, Scanner, storage::ScannerStorage};

/// Parser state and construction operations used by grammar routines that consume tokens.
/// Scanner values and nodes retain the implementation's source and construction lifetimes.
pub trait TokenParser<'text> {
    type Node: Copy;
    type Storage: ScannerStorage<'text>;

    fn scanner(&self) -> &Scanner<'text, Self::Storage>;
    fn token(&self) -> Kind;
    fn next_token(&mut self) -> Kind;
    fn next_token_without_check(&mut self) -> Kind;
    fn re_scan_slash_token(&mut self) -> Kind;
    fn re_scan_template_token(&mut self, is_tagged_template: bool) -> Kind;
    fn finish_node(&mut self, node: Self::Node, pos: u32) -> Self::Node;
    fn report_error(&mut self, loc: TextRange, message: &'static Message, args: &[&dyn Display]);
    fn new_token(&self, kind: Kind) -> Self::Node;
    fn new_keyword_expression(&self, kind: Kind) -> Self::Node;
    fn new_string_literal(
        &self,
        text: <Self::Storage as ScannerStorage<'text>>::Value,
        flags: TokenFlags,
    ) -> Self::Node;
    fn new_numeric_literal(
        &self,
        text: <Self::Storage as ScannerStorage<'text>>::Value,
        flags: TokenFlags,
    ) -> Self::Node;
    fn new_big_int_literal(
        &self,
        text: <Self::Storage as ScannerStorage<'text>>::Value,
        flags: TokenFlags,
    ) -> Self::Node;
    fn new_regular_expression_literal(
        &self,
        text: <Self::Storage as ScannerStorage<'text>>::Value,
        flags: TokenFlags,
    ) -> Self::Node;
    fn new_no_substitution_template_literal(
        &self,
        text: <Self::Storage as ScannerStorage<'text>>::Value,
        flags: TokenFlags,
    ) -> Self::Node;
    fn new_template_head(
        &self,
        text: <Self::Storage as ScannerStorage<'text>>::Value,
        raw_text: &'text str,
        flags: TokenFlags,
    ) -> Self::Node;
    fn new_template_middle(
        &self,
        text: <Self::Storage as ScannerStorage<'text>>::Value,
        raw_text: &'text str,
        flags: TokenFlags,
    ) -> Self::Node;
    fn new_template_tail(
        &self,
        text: <Self::Storage as ScannerStorage<'text>>::Value,
        raw_text: &'text str,
        flags: TokenFlags,
    ) -> Self::Node;
    fn new_prefix_unary_expression(&self, kind: Kind, operand: Self::Node) -> Self::Node;
    fn new_literal_type_node(&self, literal: Self::Node) -> Self::Node;

    fn skip_range_trivia(&self, text_range: TextRange) -> TextRange {
        TextRange::new(
            scanner::skip_trivia(
                self.scanner().text(),
                text_range
                    .pos()
                    .expect_source("parsed ranges have source positions"),
            ),
            text_range
                .end()
                .expect_source("parsed ranges have source positions"),
        )
    }

    fn node_pos(&self) -> u32 {
        self.scanner().token_full_start()
    }
    fn has_preceding_line_break(&self) -> bool {
        self.scanner().has_preceding_line_break()
    }
    fn parse_error_at(
        &mut self,
        pos: u32,
        end: u32,
        message: &'static Message,
        args: &[&dyn Display],
    ) {
        self.report_error(TextRange::new(pos, end), message, args);
    }
    fn parse_error_at_current_token(&mut self, message: &'static Message, args: &[&dyn Display]) {
        self.report_error(self.scanner().token_range(), message, args);
    }
    fn parse_optional(&mut self, token: Kind) -> bool {
        if self.token() == token {
            self.next_token();
            return true;
        }
        false
    }
    fn parse_expected(&mut self, kind: Kind) -> bool {
        self.parse_expected_with_diagnostic(kind, None, true)
    }

    fn parse_expected_without_advancing(&mut self, kind: Kind) -> bool {
        self.parse_expected_with_diagnostic(kind, None, false)
    }

    #[inline]
    fn parse_expected_with_diagnostic(
        &mut self,
        kind: Kind,
        message: Option<&'static Message>,
        should_advance: bool,
    ) -> bool {
        if self.token() == kind {
            if should_advance {
                self.next_token();
            }
            return true;
        }
        self.report_expected(kind, message)
    }

    #[cold]
    fn report_expected(&mut self, kind: Kind, message: Option<&'static Message>) -> bool {
        // Report specific message if provided with one.  Otherwise, report generic fallback message.
        if let Some(message) = message {
            self.parse_error_at_current_token(message, &[]);
        } else {
            self.parse_error_at_current_token(
                &diagnostics::X_0_expected,
                &[&scanner::token_to_string(kind)],
            );
        }
        false
    }

    fn parse_token_node(&mut self) -> Self::Node {
        let pos = self.node_pos();
        let kind = self.token();
        self.next_token();
        let token = self.new_token(kind);
        self.finish_node(token, pos)
    }

    fn parse_expected_token(&mut self, kind: Kind) -> Self::Node {
        if let Some(token) = self.parse_optional_token(kind) {
            token
        } else {
            self.parse_error_at_current_token(
                &diagnostics::X_0_expected,
                &[&scanner::token_to_string(kind)],
            );
            let token = self.new_token(kind);
            let pos = self.node_pos();
            self.finish_node(token, pos)
        }
    }

    fn parse_optional_token(&mut self, kind: Kind) -> Option<Self::Node> {
        if self.token() == kind {
            return Some(self.parse_token_node());
        }
        None
    }

    fn parse_expected_token_jsdoc(&mut self, kind: Kind) -> Self::Node {
        if let Some(optional) = self.parse_optional_token_jsdoc(kind) {
            optional
        } else {
            assert!(
                is_keyword_or_punctuation(kind),
                "expected keyword or punctuation"
            );
            self.parse_error_at_current_token(
                &diagnostics::X_0_expected,
                &[&scanner::token_to_string(kind)],
            );
            let token = self.new_token(kind);
            let pos = self.node_pos();
            self.finish_node(token, pos)
        }
    }

    fn parse_optional_token_jsdoc(&mut self, kind: Kind) -> Option<Self::Node> {
        if self.token() == kind {
            return Some(self.parse_token_node());
        }
        None
    }

    fn parse_literal_type_node(&mut self, negative: bool) -> Self::Node {
        let pos = self.node_pos();
        if negative {
            self.next_token();
        }
        let mut expression;
        if self.token() == Kind::TrueKeyword
            || self.token() == Kind::FalseKeyword
            || self.token() == Kind::NullKeyword
        {
            expression = self.parse_keyword_expression();
        } else {
            expression = self.parse_literal_expression();
        }
        if negative {
            let node = self.new_prefix_unary_expression(Kind::MinusToken, expression);
            expression = self.finish_node(node, pos);
        }
        let node = self.new_literal_type_node(expression);
        self.finish_node(node, pos)
    }

    fn parse_template_head(&mut self, is_tagged_template: bool) -> Self::Node {
        if !is_tagged_template
            && self
                .scanner()
                .token_flags()
                .intersects(TokenFlags::IsInvalid)
        {
            self.re_scan_template_token(false /*isTaggedTemplate*/);
        }
        let pos = self.node_pos();
        let text = self.scanner().token_value();
        let raw_text = self.get_template_literal_raw_text(2 /*endLength*/);
        let result = self.new_template_head(text, raw_text, self.scanner().token_flags());
        self.next_token();
        self.finish_node(result, pos)
    }

    fn get_template_literal_raw_text(&self, end_length: usize) -> &'text str {
        let mut end_length = end_length;
        let token_text = self.scanner().token_text();
        if self
            .scanner()
            .token_flags()
            .intersects(TokenFlags::Unterminated)
        {
            end_length = 0;
        }
        &token_text[1..token_text.len() - end_length]
    }

    fn parse_literal_of_template_span(&mut self, is_tagged_template: bool) -> Self::Node {
        if self.token() == Kind::CloseBraceToken {
            self.re_scan_template_token(is_tagged_template);
            return self.parse_template_middle_or_tail();
        }
        self.parse_error_at_current_token(
            &diagnostics::X_0_expected,
            &[&scanner::token_to_string(Kind::CloseBraceToken)],
        );
        let node = self.new_template_tail(Default::default(), "", TokenFlags::None);
        let pos = self.node_pos();
        self.finish_node(node, pos)
    }

    fn parse_template_middle_or_tail(&mut self) -> Self::Node {
        let pos = self.node_pos();

        let result = if self.token() == Kind::TemplateMiddle {
            let text = self.scanner().token_value();
            let raw_text = self.get_template_literal_raw_text(2 /*endLength*/);
            self.new_template_middle(text, raw_text, self.scanner().token_flags())
        } else {
            let text = self.scanner().token_value();
            let raw_text = self.get_template_literal_raw_text(1 /*endLength*/);
            self.new_template_tail(text, raw_text, self.scanner().token_flags())
        };
        self.next_token();
        self.finish_node(result, pos)
    }

    fn parse_keyword_expression(&mut self) -> Self::Node {
        let pos = self.node_pos();
        let result = self.new_keyword_expression(self.token());
        self.next_token();
        self.finish_node(result, pos)
    }

    fn parse_literal_expression(&mut self) -> Self::Node {
        let pos = self.node_pos();
        let text = self.scanner().token_value();
        let token_flags = self.scanner().token_flags();
        let result = match self.token() {
            Kind::StringLiteral => self.new_string_literal(text, token_flags),
            Kind::NumericLiteral => self.new_numeric_literal(text, token_flags),
            Kind::BigIntLiteral => self.new_big_int_literal(text, token_flags),
            Kind::RegularExpressionLiteral => {
                self.new_regular_expression_literal(text, token_flags)
            }
            Kind::NoSubstitutionTemplateLiteral => {
                self.new_no_substitution_template_literal(text, token_flags)
            }
            _ => panic!("Unhandled case in parseLiteralExpression"),
        };
        self.next_token();
        self.finish_node(result, pos)
    }
}

impl TokenParser<'static> for crate::Parser {
    type Node = tsrs_core::P<tsrs_ast::Node>;
    type Storage = tsrs_scanner::storage::LegacyStorage;

    fn scanner(&self) -> &Scanner<'static> {
        &self.scanner
    }

    fn token(&self) -> Kind {
        self.token
    }

    fn next_token(&mut self) -> Kind {
        crate::Parser::next_token(self)
    }

    fn next_token_without_check(&mut self) -> Kind {
        crate::Parser::next_token_without_check(self)
    }

    fn finish_node(&mut self, node: Self::Node, pos: u32) -> Self::Node {
        crate::Parser::finish_node(self, node, pos)
    }

    fn report_error(&mut self, loc: TextRange, message: &'static Message, args: &[&dyn Display]) {
        self.parse_error_at_range(loc, message, args);
    }
    fn re_scan_slash_token(&mut self) -> Kind {
        crate::Parser::re_scan_slash_token(self)
    }
    fn re_scan_template_token(&mut self, is_tagged_template: bool) -> Kind {
        crate::Parser::re_scan_template_token(self, is_tagged_template)
    }
    fn new_token(&self, kind: Kind) -> Self::Node {
        self.factory.new_token(kind)
    }
    fn new_keyword_expression(&self, kind: Kind) -> Self::Node {
        self.factory.new_keyword_expression(kind)
    }
    fn new_string_literal(&self, text: &'static str, flags: TokenFlags) -> Self::Node {
        self.factory.new_string_literal(text, flags)
    }
    fn new_numeric_literal(&self, text: &'static str, flags: TokenFlags) -> Self::Node {
        self.factory.new_numeric_literal(text, flags)
    }
    fn new_big_int_literal(&self, text: &'static str, flags: TokenFlags) -> Self::Node {
        self.factory.new_big_int_literal(text, flags)
    }
    fn new_regular_expression_literal(&self, text: &'static str, flags: TokenFlags) -> Self::Node {
        self.factory.new_regular_expression_literal(text, flags)
    }
    fn new_no_substitution_template_literal(
        &self,
        text: &'static str,
        flags: TokenFlags,
    ) -> Self::Node {
        self.factory
            .new_no_substitution_template_literal(text, flags)
    }
    fn new_template_head(
        &self,
        text: &'static str,
        raw_text: &'static str,
        flags: TokenFlags,
    ) -> Self::Node {
        self.factory.new_template_head(text, raw_text, flags)
    }
    fn new_template_middle(
        &self,
        text: &'static str,
        raw_text: &'static str,
        flags: TokenFlags,
    ) -> Self::Node {
        self.factory.new_template_middle(text, raw_text, flags)
    }
    fn new_template_tail(
        &self,
        text: &'static str,
        raw_text: &'static str,
        flags: TokenFlags,
    ) -> Self::Node {
        self.factory.new_template_tail(text, raw_text, flags)
    }
    fn new_prefix_unary_expression(&self, kind: Kind, operand: Self::Node) -> Self::Node {
        self.factory.new_prefix_unary_expression(kind, operand)
    }
    fn new_literal_type_node(&self, literal: Self::Node) -> Self::Node {
        self.factory.new_literal_type_node(literal)
    }
}
