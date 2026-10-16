//! Shared list construction and recovery for legacy handles and scoped native references.
use crate::utilities::token_is_identifier_or_keyword;
use crate::{ParsingContext, ParsingContexts, lookahead_parser::LookaheadParser};
use tsrs_ast::{self as ast, Kind};
use tsrs_core::{TextRange, new_text_range};
use tsrs_diagnostics as diagnostics;
use tsrs_scanner as scanner;

/// List grammar with representation-specific storage.
///
/// Nested lists use one scratch stack and restore its length and parsing contexts on every normal
/// or failed return, except the explicit
/// `parse_list_index_on_stack` entry point which leaves its elements for the caller to finish.
/// Speculation callers must finish each list before rewinding; checkpoints do not roll back
/// scratch-stack mutations.
pub trait ListParser<'text>: LookaheadParser<'text> {
    type NodeList: Copy;

    fn parsing_contexts(&self) -> ParsingContexts;
    fn replace_parsing_contexts(&mut self, contexts: ParsingContexts);
    fn node_stack(&self) -> &[Self::Node];
    fn node_stack_mut(&mut self) -> &mut Vec<Self::Node>;
    fn reparse_list(&mut self) -> &mut Vec<Self::Node>;
    fn node_kind(node: Self::Node) -> Kind;
    fn new_node_list(&self, loc: TextRange, nodes: &[Self::Node]) -> Self::NodeList;
    fn new_missing_node_list(&self, loc: TextRange) -> Self::NodeList;
    fn is_missing_node_list(list: Option<Self::NodeList>) -> bool;

    fn parse_list_index(
        &mut self,
        kind: ParsingContext,
        parse_element: impl FnMut(&mut Self, usize) -> Self::Node,
    ) -> Vec<Self::Node> {
        let start = self.parse_list_index_on_stack(kind, parse_element);
        self.node_stack_mut().split_off(start)
    }

    /// `parseListIndex` with the elements left on the scratch stack above the returned start index.
    fn parse_list_index_on_stack(
        &mut self,
        kind: ParsingContext,
        mut parse_element: impl FnMut(&mut Self, usize) -> Self::Node,
    ) -> usize {
        let save_parsing_contexts = self.parsing_contexts();
        self.replace_parsing_contexts(self.parsing_contexts() | (1 << kind as u32));
        let mut outer_reparse_list = std::mem::take(self.reparse_list());
        let start = self.node_stack().len();
        while !self.is_list_terminator(kind) {
            if self.is_list_element(kind, false /*inErrorRecovery*/) {
                let index = self.node_stack().len() - start;
                let elt = parse_element(self, index);
                if !self.reparse_list().is_empty() {
                    outer_reparse_list = std::mem::take(self.reparse_list()).into_iter().fold(
                        outer_reparse_list,
                        |mut outer_reparse_list, e| {
                            // Propagate @typedef type alias declarations outwards to a context that permits them.
                            if matches!(
                                Self::node_kind(e),
                                Kind::JSTypeAliasDeclaration | Kind::JSImportDeclaration
                            ) && kind != ParsingContext::SourceElements
                                && kind != ParsingContext::BlockStatements
                            {
                                outer_reparse_list.push(e);
                            } else {
                                self.node_stack_mut().push(e);
                            }

                            outer_reparse_list
                        },
                    );
                }
                self.node_stack_mut().push(elt);
                continue;
            }
            if self.abort_parsing_list_or_move_to_next_token(kind) {
                break;
            }
        }
        *self.reparse_list() = outer_reparse_list;
        self.replace_parsing_contexts(save_parsing_contexts);
        start
    }

    fn parse_list(
        &mut self,
        kind: ParsingContext,
        mut parse_element: impl FnMut(&mut Self) -> Self::Node,
    ) -> Self::NodeList {
        let pos = self.node_pos();
        let start = self.parse_list_index_on_stack(kind, |p, _| parse_element(p));
        let end = self.node_pos();
        self.finish_node_list(start, new_text_range(pos, end))
    }

    /// Move the elements above `start` off the scratch stack into a new arena list.
    fn finish_node_list(&mut self, start: usize, loc: TextRange) -> Self::NodeList {
        let list = self.new_node_list(loc, &self.node_stack()[start..]);
        self.node_stack_mut().truncate(start);
        list
    }

    /// Return a possibly empty list on success, or `None` when an element callback fails.
    fn parse_delimited_list<R: Into<Option<Self::Node>>>(
        &mut self,
        kind: ParsingContext,
        mut parse_element: impl FnMut(&mut Self) -> R,
    ) -> Option<Self::NodeList> {
        let pos = self.node_pos();
        let save_parsing_contexts = self.parsing_contexts();
        self.replace_parsing_contexts(self.parsing_contexts() | (1 << kind as u32));
        let start = self.node_stack().len();
        loop {
            if self.is_list_element(kind, false /*inErrorRecovery*/) {
                let start_pos = self.node_pos();
                let element: Option<Self::Node> = parse_element(self).into();
                let Some(element) = element else {
                    self.replace_parsing_contexts(save_parsing_contexts);
                    self.node_stack_mut().truncate(start);
                    // Return nil to indicate parseElement failed
                    return None;
                };
                self.node_stack_mut().push(element);
                if self.parse_optional(Kind::CommaToken) {
                    // No need to check for a zero length node since we know we parsed a comma
                    continue;
                }
                if self.is_list_terminator(kind) {
                    break;
                }
                // We didn't get a comma, and the list wasn't terminated, explicitly parse
                // out a comma so we give a good error message.
                if self.token() != Kind::CommaToken && kind == ParsingContext::EnumMembers {
                    self.parse_error_at_current_token(
                        &diagnostics::An_enum_member_name_must_be_followed_by_a_or,
                        &[],
                    );
                } else {
                    self.parse_expected(Kind::CommaToken);
                }
                // If the token was a semicolon, and the caller allows that, then skip it and
                // continue.  This ensures we get back on track and don't result in tons of
                // parse errors.  For example, this can happen when people do things like use
                // a semicolon to delimit object literal members.   Note: we'll have already
                // reported an error when we called parseExpected above.
                if (kind == ParsingContext::ObjectLiteralMembers
                    || kind == ParsingContext::ImportAttributes)
                    && self.token() == Kind::SemicolonToken
                    && !self.has_preceding_line_break()
                {
                    self.next_token();
                }
                if start_pos == self.node_pos() {
                    // What we're parsing isn't actually remotely recognizable as a element and we've consumed no tokens whatsoever
                    // Consume a token to advance the parser in some way and avoid an infinite loop
                    // This can happen when we're speculatively parsing parenthesized expressions which we think may be arrow functions,
                    // or when a modifier keyword which is disallowed as a parameter name (ie, `static` in strict mode) is supplied
                    self.next_token();
                }
                continue;
            }
            if self.is_list_terminator(kind) {
                break;
            }
            if self.abort_parsing_list_or_move_to_next_token(kind) {
                break;
            }
        }
        self.replace_parsing_contexts(save_parsing_contexts);
        let end = self.node_pos();
        Some(self.finish_node_list(start, new_text_range(pos, end)))
    }

    /// Return a missing list if the opening token is absent, or `None` if an element callback fails.
    fn parse_bracketed_list<R: Into<Option<Self::Node>>>(
        &mut self,
        kind: ParsingContext,
        parse_element: impl FnMut(&mut Self) -> R,
        opening: Kind,
        closing: Kind,
    ) -> Option<Self::NodeList> {
        if self.parse_expected(opening) {
            let result = self.parse_delimited_list(kind, parse_element);
            self.parse_expected(closing);
            return result;
        }
        Some(self.create_missing_list())
    }

    fn parse_empty_node_list(&self) -> Self::NodeList {
        let pos = self.node_pos();
        self.new_node_list(new_text_range(pos, pos), &[])
    }

    fn create_missing_list(&self) -> Self::NodeList {
        let pos = self.node_pos();
        self.new_missing_node_list(new_text_range(pos, pos))
    }

    fn abort_parsing_list_or_move_to_next_token(&mut self, kind: ParsingContext) -> bool {
        self.parsing_context_errors(kind);
        if self.is_in_some_parsing_context() {
            return true;
        }
        self.next_token();
        false
    }

    fn is_in_some_parsing_context(&mut self) -> bool {
        // We should be in at least one parsing context, be it SourceElements while parsing
        // a SourceFile, or JSDocComment when lazily parsing JSDoc.
        assert!(self.parsing_contexts() != 0, "Missing parsing context");
        if ParsingContext::ALL.into_iter().any(|kind| {
            if self.parsing_contexts() & (1 << kind as u32) != 0 {
                if self.is_list_element(kind, true /*inErrorRecovery*/)
                    || self.is_list_terminator(kind)
                {
                    return true;
                }
            }

            false
        }) {
            return true;
        }
        false
    }

    fn parsing_context_errors(&mut self, context: ParsingContext) {
        match context {
            ParsingContext::SourceElements => {
                if self.token() == Kind::DefaultKeyword {
                    self.parse_error_at_current_token(&diagnostics::X_0_expected, &[&"export"]);
                } else {
                    self.parse_error_at_current_token(
                        &diagnostics::Declaration_or_statement_expected,
                        &[],
                    );
                }
            }
            ParsingContext::BlockStatements => {
                self.parse_error_at_current_token(
                    &diagnostics::Declaration_or_statement_expected,
                    &[],
                );
            }
            ParsingContext::SwitchClauses => {
                self.parse_error_at_current_token(&diagnostics::X_case_or_default_expected, &[]);
            }
            ParsingContext::SwitchClauseStatements => {
                self.parse_error_at_current_token(&diagnostics::Statement_expected, &[]);
            }
            ParsingContext::RestProperties | ParsingContext::TypeMembers => {
                self.parse_error_at_current_token(
                    &diagnostics::Property_or_signature_expected,
                    &[],
                );
            }
            ParsingContext::ClassMembers => {
                self.parse_error_at_current_token(&diagnostics::Unexpected_token_A_constructor_method_accessor_or_property_was_expected, &[]);
            }
            ParsingContext::EnumMembers => {
                self.parse_error_at_current_token(&diagnostics::Enum_member_expected, &[]);
            }
            ParsingContext::HeritageClauseElement => {
                self.parse_error_at_current_token(&diagnostics::Expression_expected, &[]);
            }
            ParsingContext::VariableDeclarations => {
                if ast::is_keyword(self.token()) {
                    let text = scanner::token_to_string(self.token());
                    self.parse_error_at_current_token(
                        &diagnostics::X_0_is_not_allowed_as_a_variable_declaration_name,
                        &[&text],
                    );
                } else {
                    self.parse_error_at_current_token(
                        &diagnostics::Variable_declaration_expected,
                        &[],
                    );
                }
            }
            ParsingContext::ObjectBindingElements => {
                self.parse_error_at_current_token(
                    &diagnostics::Property_destructuring_pattern_expected,
                    &[],
                );
            }
            ParsingContext::ArrayBindingElements => {
                self.parse_error_at_current_token(
                    &diagnostics::Array_element_destructuring_pattern_expected,
                    &[],
                );
            }
            ParsingContext::ArgumentExpressions => {
                self.parse_error_at_current_token(&diagnostics::Argument_expression_expected, &[]);
            }
            ParsingContext::ObjectLiteralMembers => {
                self.parse_error_at_current_token(&diagnostics::Property_assignment_expected, &[]);
            }
            ParsingContext::ArrayLiteralMembers => {
                self.parse_error_at_current_token(&diagnostics::Expression_or_comma_expected, &[]);
            }
            ParsingContext::JSDocParameters => {
                self.parse_error_at_current_token(
                    &diagnostics::Parameter_declaration_expected,
                    &[],
                );
            }
            ParsingContext::Parameters => {
                if ast::is_keyword(self.token()) {
                    let text = scanner::token_to_string(self.token());
                    self.parse_error_at_current_token(
                        &diagnostics::X_0_is_not_allowed_as_a_parameter_name,
                        &[&text],
                    );
                } else {
                    self.parse_error_at_current_token(
                        &diagnostics::Parameter_declaration_expected,
                        &[],
                    );
                }
            }
            ParsingContext::TypeParameters => {
                self.parse_error_at_current_token(
                    &diagnostics::Type_parameter_declaration_expected,
                    &[],
                );
            }
            ParsingContext::TypeArguments => {
                self.parse_error_at_current_token(&diagnostics::Type_argument_expected, &[]);
            }
            ParsingContext::TupleElementTypes => {
                self.parse_error_at_current_token(&diagnostics::Type_expected, &[]);
            }
            ParsingContext::HeritageClauses => {
                self.parse_error_at_current_token(&diagnostics::Unexpected_token_expected, &[]);
            }
            ParsingContext::ImportOrExportSpecifiers => {
                if self.token() == Kind::FromKeyword {
                    self.parse_error_at_current_token(&diagnostics::X_0_expected, &[&"}"]);
                } else {
                    self.parse_error_at_current_token(&diagnostics::Identifier_expected, &[]);
                }
            }
            ParsingContext::JsxAttributes
            | ParsingContext::JsxChildren
            | ParsingContext::JSDocComment => {
                self.parse_error_at_current_token(&diagnostics::Identifier_expected, &[]);
            }
            ParsingContext::ImportAttributes => {
                self.parse_error_at_current_token(
                    &diagnostics::Identifier_or_string_literal_expected,
                    &[],
                );
            }
            ParsingContext::Count => panic!("Unhandled case in parsingContextErrors"),
        }
    }

    fn is_list_element(
        &mut self,
        parsing_context: ParsingContext,
        in_error_recovery: bool,
    ) -> bool {
        match parsing_context {
            ParsingContext::SourceElements
            | ParsingContext::BlockStatements
            | ParsingContext::SwitchClauseStatements => {
                // If we're in error recovery, then we don't want to treat ';' as an empty statement.
                // The problem is that ';' can show up in far too many contexts, and if we see one
                // and assume it's a statement, then we may bail out inappropriately from whatever
                // we're parsing.  For example, if we have a semicolon in the middle of a class, then
                // we really don't want to assume the class is over and we're on a statement in the
                // outer module.  We just want to consume and move on.
                !(self.token() == Kind::SemicolonToken && in_error_recovery)
                    && self.is_start_of_statement()
            }
            ParsingContext::SwitchClauses => {
                self.token() == Kind::CaseKeyword || self.token() == Kind::DefaultKeyword
            }
            ParsingContext::TypeMembers => self.look_ahead(Self::scan_type_member_start),
            ParsingContext::ClassMembers => {
                // We allow semicolons as class elements (as specified by ES6) as long as we're
                // not in error recovery.  If we're in error recovery, we don't want an errant
                // semicolon to be treated as a class member (since they're almost always used
                // for statements.
                self.look_ahead(Self::scan_class_member_start)
                    || self.token() == Kind::SemicolonToken && !in_error_recovery
            }
            ParsingContext::EnumMembers => {
                // Include open bracket computed properties. This technically also lets in indexers,
                // which would be a candidate for improved error reporting.
                self.token() == Kind::OpenBracketToken || self.is_literal_property_name()
            }
            ParsingContext::ObjectLiteralMembers => match self.token() {
                // Not an object literal member, but don't want to close the object (see `tests/cases/fourslash/completionsDotInObjectLiteral.ts`)
                Kind::OpenBracketToken
                | Kind::AsteriskToken
                | Kind::DotDotDotToken
                | Kind::DotToken => true,
                _ => self.is_literal_property_name(),
            },
            ParsingContext::RestProperties => self.is_literal_property_name(),
            ParsingContext::ObjectBindingElements => {
                self.token() == Kind::OpenBracketToken
                    || self.token() == Kind::DotDotDotToken
                    || self.is_literal_property_name()
            }
            ParsingContext::ImportAttributes => self.is_import_attribute_name(),
            ParsingContext::HeritageClauseElement => {
                // If we see `{ ... }` then only consume it as an expression if it is followed by `,` or `{`
                // That way we won't consume the body of a class in its heritage clause.
                if self.token() == Kind::OpenBraceToken {
                    return self.is_valid_heritage_clause_object_literal();
                }
                if !in_error_recovery {
                    return self.is_start_of_left_hand_side_expression()
                        && !self.is_heritage_clause_extends_or_implements_keyword();
                }
                // If we're in error recovery we tighten up what we're willing to match.
                // That way we don't treat something like "this" as a valid heritage clause
                // element during recovery.
                self.is_identifier() && !self.is_heritage_clause_extends_or_implements_keyword()
            }
            ParsingContext::VariableDeclarations => {
                self.is_binding_identifier_or_private_identifier_or_pattern()
            }
            ParsingContext::ArrayBindingElements => {
                self.token() == Kind::CommaToken
                    || self.token() == Kind::DotDotDotToken
                    || self.is_binding_identifier_or_private_identifier_or_pattern()
            }
            ParsingContext::TypeParameters => {
                self.token() == Kind::InKeyword
                    || self.token() == Kind::ConstKeyword
                    || self.is_identifier()
            }
            ParsingContext::ArrayLiteralMembers | ParsingContext::ArgumentExpressions => {
                if parsing_context == ParsingContext::ArrayLiteralMembers {
                    // Not an array literal member, but don't want to close the array (see `tests/cases/fourslash/completionsDotInArrayLiteralInObjectLiteral.ts`)
                    if self.token() == Kind::CommaToken || self.token() == Kind::DotToken {
                        return true;
                    }
                }
                self.token() == Kind::DotDotDotToken || self.is_start_of_expression()
            }
            ParsingContext::Parameters => {
                self.is_start_of_parameter(false /*isJSDocParameter*/)
            }
            ParsingContext::JSDocParameters => {
                self.is_start_of_parameter(true /*isJSDocParameter*/)
            }
            ParsingContext::TypeArguments | ParsingContext::TupleElementTypes => {
                self.token() == Kind::CommaToken
                    || self.is_start_of_type(false /*inStartOfParameter*/)
            }
            ParsingContext::HeritageClauses => self.is_heritage_clause(),
            ParsingContext::ImportOrExportSpecifiers => {
                // bail out if the next token is [FromKeyword StringLiteral].
                // That means we're in something like `import { from "mod"`. Stop here can give better error message.
                if self.token() == Kind::FromKeyword
                    && self.look_ahead(Self::next_token_is_token_string_literal)
                {
                    return false;
                }
                if self.token() == Kind::StringLiteral {
                    return true; // For "arbitrary module namespace identifiers"
                }
                token_is_identifier_or_keyword(self.token())
            }
            ParsingContext::JsxAttributes => {
                token_is_identifier_or_keyword(self.token()) || self.token() == Kind::OpenBraceToken
            }
            ParsingContext::JsxChildren => true,
            ParsingContext::JSDocComment => true,
            ParsingContext::Count => panic!("Unhandled case in isListElement"),
        }
    }

    fn is_list_terminator(&mut self, kind: ParsingContext) -> bool {
        if self.token() == Kind::EndOfFile {
            return true;
        }
        match kind {
            ParsingContext::BlockStatements
            | ParsingContext::SwitchClauses
            | ParsingContext::TypeMembers
            | ParsingContext::ClassMembers
            | ParsingContext::EnumMembers
            | ParsingContext::ObjectLiteralMembers
            | ParsingContext::ObjectBindingElements
            | ParsingContext::ImportOrExportSpecifiers
            | ParsingContext::ImportAttributes => self.token() == Kind::CloseBraceToken,
            ParsingContext::SwitchClauseStatements => {
                self.token() == Kind::CloseBraceToken
                    || self.token() == Kind::CaseKeyword
                    || self.token() == Kind::DefaultKeyword
            }
            ParsingContext::HeritageClauseElement => {
                self.token() == Kind::OpenBraceToken
                    || self.token() == Kind::ExtendsKeyword
                    || self.token() == Kind::ImplementsKeyword
            }
            ParsingContext::VariableDeclarations => {
                // If we can consume a semicolon (either explicitly, or with ASI), then consider us done
                // with parsing the list of variable declarators.
                // In the case where we're parsing the variable declarator of a 'for-in' statement, we
                // are done if we see an 'in' keyword in front of us. Same with for-of
                // ERROR RECOVERY TWEAK:
                // For better error recovery, if we see an '=>' then we just stop immediately.  We've got an
                // arrow function here and it's going to be very unlikely that we'll resynchronize and get
                // another variable declaration.
                self.can_parse_semicolon()
                    || self.token() == Kind::InKeyword
                    || self.token() == Kind::OfKeyword
                    || self.token() == Kind::EqualsGreaterThanToken
            }
            ParsingContext::TypeParameters => {
                // Tokens other than '>' are here for better error recovery
                self.token() == Kind::GreaterThanToken
                    || self.token() == Kind::OpenParenToken
                    || self.token() == Kind::OpenBraceToken
                    || self.token() == Kind::ExtendsKeyword
                    || self.token() == Kind::ImplementsKeyword
            }
            ParsingContext::ArgumentExpressions => {
                // Tokens other than ')' are here for better error recovery
                self.token() == Kind::CloseParenToken || self.token() == Kind::SemicolonToken
            }
            ParsingContext::ArrayLiteralMembers
            | ParsingContext::TupleElementTypes
            | ParsingContext::ArrayBindingElements => self.token() == Kind::CloseBracketToken,
            ParsingContext::JSDocParameters
            | ParsingContext::Parameters
            | ParsingContext::RestProperties => {
                // Tokens other than ')' and ']' (the latter for index signatures) are here for better error recovery
                self.token() == Kind::CloseParenToken || self.token() == Kind::CloseBracketToken /*|| token == ast.KindOpenBraceToken*/
            }
            ParsingContext::TypeArguments => {
                // All other tokens should cause the type-argument to terminate except comma token
                self.token() != Kind::CommaToken
            }
            ParsingContext::HeritageClauses => {
                self.token() == Kind::OpenBraceToken || self.token() == Kind::CloseBraceToken
            }
            ParsingContext::JsxAttributes => {
                self.token() == Kind::GreaterThanToken || self.token() == Kind::SlashToken
            }
            ParsingContext::JsxChildren => {
                self.token() == Kind::LessThanToken && self.look_ahead(Self::next_token_is_slash)
            }
            _ => false,
        }
    }
}

impl ListParser<'static> for crate::Parser {
    type NodeList = tsrs_core::P<tsrs_ast::NodeList>;

    fn parsing_contexts(&self) -> ParsingContexts {
        self.parsing_contexts
    }
    fn replace_parsing_contexts(&mut self, contexts: ParsingContexts) {
        self.parsing_contexts = contexts;
    }
    fn node_stack(&self) -> &[Self::Node] {
        &self.node_stack
    }
    fn node_stack_mut(&mut self) -> &mut Vec<Self::Node> {
        &mut self.node_stack
    }
    fn reparse_list(&mut self) -> &mut Vec<Self::Node> {
        &mut self.reparse_list
    }
    fn node_kind(node: Self::Node) -> Kind {
        node.kind()
    }
    fn new_node_list(&self, loc: TextRange, nodes: &[Self::Node]) -> Self::NodeList {
        crate::Parser::new_node_list(self, loc, nodes)
    }
    fn new_missing_node_list(&self, loc: TextRange) -> Self::NodeList {
        let list = self
            .factory
            .new_node_list_from_static(crate::parser_1::missing_list_nodes());
        list.loc.set(loc);
        list
    }
    fn is_missing_node_list(list: Option<Self::NodeList>) -> bool {
        crate::parser_1::is_missing_node_list(list)
    }
}
