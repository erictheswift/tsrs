//! Shared name grammar for the legacy parser and the scoped, borrowed frontend.
//! Implementations supply storage, parser state, and diagnostics; the grammar stays in one place.
use crate::parser_3::is_reserved_word;
use crate::token_parser::TokenParser;
use crate::utilities::token_is_identifier_or_keyword;
use tsrs_ast::{Kind, NodeFlags};
use tsrs_core::TextRange;
use tsrs_diagnostics::{self as diagnostics, Message};
use tsrs_scanner::storage::ScannerStorage;

/// Storage-dependent operations used by the parser's identifier and qualified-name routines.
/// `Node` is either a legacy handle or an ordinary reference limited to a construction scope.
pub trait NameParser<'text>: TokenParser<'text> {
    fn context_flags(&self) -> NodeFlags;
    fn statement_has_await_identifier(&self) -> bool;
    fn set_statement_has_await_identifier(&mut self, value: bool);
    fn look_ahead_next_identifier_on_same_line(&mut self) -> bool;
    fn new_identifier(&mut self, text: &'text str) -> Self::Node;
    fn new_source_identifier(
        &mut self,
        text: <Self::Storage as ScannerStorage<'text>>::Value,
        loc: TextRange,
    ) -> Self::Node;
    fn new_private_identifier(
        &self,
        text: <Self::Storage as ScannerStorage<'text>>::Value,
    ) -> Self::Node;
    fn new_qualified_name(&self, left: Self::Node, right: Self::Node) -> Self::Node;

    fn parse_entity_name(
        &mut self,
        allow_reserved_words: bool,
        allow_private_name: bool,
        diagnostic_message: Option<&'static Message>,
    ) -> Self::Node {
        let pos = self.node_pos();
        let mut entity;
        if allow_reserved_words {
            entity = self.parse_identifier_name_with_diagnostic(diagnostic_message);
        } else {
            entity = self.parse_identifier_with_diagnostic(diagnostic_message, None);
        }
        while self.parse_optional(Kind::DotToken) {
            if self.token() == Kind::LessThanToken {
                // The entity is part of a JSDoc-style generic. We will use the gap between `typeName` and
                // `typeArguments` to report it as a grammar error in the checker.
                break;
            }
            let right = self.parse_right_side_of_dot(
                allow_reserved_words,
                allow_private_name,
                true, /*allowUnicodeEscapeSequenceInIdentifierName*/
            );
            let node = self.new_qualified_name(entity, right);
            entity = self.finish_node(node, pos);
        }
        entity
    }

    fn parse_right_side_of_dot(
        &mut self,
        allow_identifier_names: bool,
        allow_private_identifiers: bool,
        allow_unicode_escape_sequence_in_identifier_name: bool,
    ) -> Self::Node {
        // Technically a keyword is valid here as all identifiers and keywords are identifier names.
        // However, often we'll encounter this in error situations when the identifier or keyword
        // is actually starting another valid construct.
        //
        // So, we check for the following specific case:
        //
        //      name.
        //      identifierOrKeyword identifierNameOrKeyword
        //
        // Note: the newlines are important here.  For example, if that above code
        // were rewritten into:
        //
        //      name.identifierOrKeyword
        //      identifierNameOrKeyword
        //
        // Then we would consider it valid.  That's because ASI would take effect and
        // the code would be implicitly: "name.identifierOrKeyword; identifierNameOrKeyword".
        // In the first case though, ASI will not take effect because there is not a
        // line terminator after the identifier or keyword.
        if self.has_preceding_line_break()
            && token_is_identifier_or_keyword(self.token())
            && self.look_ahead_next_identifier_on_same_line()
        {
            // Report that we need an identifier.  However, report it right after the dot,
            // and not on the next token.  This is because the next token might actually
            // be an identifier and the error would be quite confusing.
            let pos = self.node_pos();
            self.parse_error_at(pos, pos, &diagnostics::Identifier_expected, &[]);
            return self.create_missing_identifier();
        }
        if self.token() == Kind::PrivateIdentifier {
            let node = self.parse_private_identifier();
            if allow_private_identifiers {
                return node;
            }
            let pos = self.node_pos();
            self.parse_error_at(pos, pos, &diagnostics::Identifier_expected, &[]);
            return self.create_missing_identifier();
        }
        if allow_identifier_names {
            if allow_unicode_escape_sequence_in_identifier_name {
                return self.parse_identifier_name();
            }
            return self.parse_identifier_name_error_on_unicode_escape_sequence();
        }
        let save_has_await_identifier = self.statement_has_await_identifier();
        let id = self.parse_identifier();
        self.set_statement_has_await_identifier(save_has_await_identifier);
        id
    }

    fn create_missing_identifier(&mut self) -> Self::Node {
        let node = self.new_identifier("");
        let pos = self.node_pos();
        self.finish_node(node, pos)
    }

    fn parse_private_identifier(&mut self) -> Self::Node {
        let pos = self.node_pos();
        let text = self.scanner().token_value();
        self.next_token();
        let node = self.new_private_identifier(text);
        self.finish_node(node, pos)
    }

    fn parse_identifier_name_error_on_unicode_escape_sequence(&mut self) -> Self::Node {
        if self.scanner().has_unicode_escape() || self.scanner().has_extended_unicode_escape() {
            self.parse_error_at_current_token(
                &diagnostics::Unicode_escape_sequence_cannot_appear_here,
                &[],
            );
        }
        self.create_identifier(token_is_identifier_or_keyword(self.token()))
    }

    fn parse_binding_identifier(&mut self) -> Self::Node {
        self.parse_binding_identifier_with_diagnostic(None)
    }

    fn parse_binding_identifier_with_diagnostic(
        &mut self,
        private_identifier_diagnostic_message: Option<&'static Message>,
    ) -> Self::Node {
        let save_has_await_identifier = self.statement_has_await_identifier();
        let is_binding_identifier = self.is_binding_identifier();
        let id = self.create_identifier_with_diagnostic(
            is_binding_identifier,
            None, /*diagnosticMessage*/
            private_identifier_diagnostic_message,
        );
        self.set_statement_has_await_identifier(save_has_await_identifier);
        id
    }

    fn parse_identifier_name(&mut self) -> Self::Node {
        self.parse_identifier_name_with_diagnostic(None)
    }

    fn parse_identifier_name_with_diagnostic(
        &mut self,
        diagnostic_message: Option<&'static Message>,
    ) -> Self::Node {
        self.create_identifier_with_diagnostic(
            token_is_identifier_or_keyword(self.token()),
            diagnostic_message,
            None,
        )
    }

    fn parse_identifier(&mut self) -> Self::Node {
        self.parse_identifier_with_diagnostic(None, None)
    }

    fn parse_identifier_with_diagnostic(
        &mut self,
        diagnostic_message: Option<&'static Message>,
        private_identifier_diagnostic_message: Option<&'static Message>,
    ) -> Self::Node {
        let is_identifier = self.is_identifier();
        self.create_identifier_with_diagnostic(
            is_identifier,
            diagnostic_message,
            private_identifier_diagnostic_message,
        )
    }

    fn create_identifier(&mut self, is_identifier: bool) -> Self::Node {
        self.create_identifier_with_diagnostic(is_identifier, None, None)
    }

    fn create_identifier_with_diagnostic(
        &mut self,
        is_identifier: bool,
        diagnostic_message: Option<&'static Message>,
        private_identifier_diagnostic_message: Option<&'static Message>,
    ) -> Self::Node {
        if is_identifier {
            let pos = if self.scanner().has_preceding_jsdoc_leading_asterisks() {
                self.scanner().token_start()
            } else {
                self.node_pos()
            };
            let text = self.scanner().token_value();
            let end = self.scanner().token_end();
            self.next_token_without_check();
            let id = self.new_source_identifier(text, TextRange::new(pos, end));
            return self.finish_node(id, pos);
        }
        self.create_missing_identifier_with_diagnostic(
            diagnostic_message,
            private_identifier_diagnostic_message,
        )
    }

    #[cold]
    fn create_missing_identifier_with_diagnostic(
        &mut self,
        diagnostic_message: Option<&'static Message>,
        private_identifier_diagnostic_message: Option<&'static Message>,
    ) -> Self::Node {
        if self.token() == Kind::PrivateIdentifier {
            if let Some(private_identifier_diagnostic_message) =
                private_identifier_diagnostic_message
            {
                self.parse_error_at_current_token(private_identifier_diagnostic_message, &[]);
            } else {
                self.parse_error_at_current_token(
                    &diagnostics::Private_identifiers_are_not_allowed_outside_class_bodies,
                    &[],
                );
            }
            return self.create_identifier(true /*isIdentifier*/);
        }
        // Only for end of file because the error gets reported incorrectly on embedded script tags.
        let report_at_current_position = self.token() == Kind::EndOfFile;
        if let Some(diagnostic_message) = diagnostic_message {
            if report_at_current_position {
                let pos = self.scanner().token_full_start();
                self.parse_error_at(pos, pos, diagnostic_message, &[]);
            } else {
                self.parse_error_at_current_token(diagnostic_message, &[]);
            }
        } else if is_reserved_word(self.token()) {
            let token_text = self.scanner().token_text();
            if report_at_current_position {
                let pos = self.scanner().token_full_start();
                self.parse_error_at(
                    pos,
                    pos,
                    &diagnostics::Identifier_expected_0_is_a_reserved_word_that_cannot_be_used_here,
                    &[&token_text],
                );
            } else {
                self.parse_error_at_current_token(
                    &diagnostics::Identifier_expected_0_is_a_reserved_word_that_cannot_be_used_here,
                    &[&token_text],
                );
            }
        } else if report_at_current_position {
            let pos = self.scanner().token_full_start();
            self.parse_error_at(pos, pos, &diagnostics::Identifier_expected, &[]);
        } else {
            self.parse_error_at_current_token(&diagnostics::Identifier_expected, &[]);
        }
        self.create_missing_identifier()
    }

    fn is_identifier(&self) -> bool {
        if self.token() == Kind::Identifier {
            return true;
        }
        // If we have a 'yield' keyword, and we're in the [yield] context, then 'yield' is
        // considered a keyword and is not an identifier.
        // If we have a 'await' keyword, and we're in the [Await] context, then 'await' is
        // considered a keyword and is not an identifier.
        if self.token() == Kind::YieldKeyword
            && self.context_flags().intersects(NodeFlags::YieldContext)
            || self.token() == Kind::AwaitKeyword
                && self.context_flags().intersects(NodeFlags::AwaitContext)
        {
            return false;
        }
        self.token() > Kind::LastReservedWord
    }

    fn is_binding_identifier(&self) -> bool {
        // `let await`/`let yield` in [Yield] or [Await] are allowed here and disallowed in the binder.
        self.token() == Kind::Identifier || self.token() > Kind::LastReservedWord
    }
}

impl NameParser<'static> for crate::Parser {
    fn context_flags(&self) -> NodeFlags {
        self.context_flags
    }
    fn statement_has_await_identifier(&self) -> bool {
        self.statement_has_await_identifier
    }
    fn set_statement_has_await_identifier(&mut self, value: bool) {
        self.statement_has_await_identifier = value;
    }

    fn look_ahead_next_identifier_on_same_line(&mut self) -> bool {
        self.look_ahead(crate::Parser::next_token_is_identifier_or_keyword_on_same_line)
    }
    fn new_identifier(&mut self, text: &'static str) -> Self::Node {
        crate::Parser::new_identifier(self, text)
    }
    fn new_source_identifier(&mut self, text: &'static str, loc: TextRange) -> Self::Node {
        crate::Parser::new_source_identifier(self, text, loc)
    }
    fn new_private_identifier(&self, text: &'static str) -> Self::Node {
        self.factory.new_private_identifier(text)
    }
    fn new_qualified_name(&self, left: Self::Node, right: Self::Node) -> Self::Node {
        self.factory.new_qualified_name(left, right)
    }
}
