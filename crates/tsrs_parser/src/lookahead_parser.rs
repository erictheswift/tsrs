//! Scanner-only syntax decisions and modifier parsing shared by both frontends.
use crate::name_parser::NameParser;
use crate::utilities::{
    token_is_identifier_or_keyword, token_is_identifier_or_keyword_or_greater_than,
};
use tsrs_ast::{self as ast, Kind, OperatorPrecedence};

/// The production parser's lookahead, modifier and semicolon grammar.
/// `scan_*` and `next_*` routines advance tokens; use `look_ahead` when their state must be restored.
pub trait LookaheadParser<'text>: NameParser<'text> {
    fn new_modifier(&self, kind: Kind) -> Self::Node;

    fn try_parse_modifier(
        &mut self,
        has_seen_static_modifier: bool,
        permit_const_as_modifier: bool,
        stop_on_start_of_class_static_block: bool,
    ) -> Option<Self::Node> {
        let pos = self.node_pos();
        let kind = self.token();
        if self.token() == Kind::ConstKeyword && permit_const_as_modifier {
            // We need to ensure that any subsequent modifiers appear on the same line
            // so that when 'const' is a standalone declaration, we don't issue an error.
            if !self.look_ahead(Self::next_token_is_on_same_line_and_can_follow_modifier) {
                return None;
            }
            self.next_token();
        } else if (stop_on_start_of_class_static_block
            && self.token() == Kind::StaticKeyword
            && self.look_ahead(Self::next_token_is_open_brace))
            || (has_seen_static_modifier && self.token() == Kind::StaticKeyword)
            || (!self.parse_any_contextual_modifier())
        {
            return None;
        }
        let node = self.new_modifier(kind);
        Some(self.finish_node(node, pos))
    }

    #[inline]
    fn parse_contextual_modifier(&mut self, t: Kind) -> bool {
        // A nonmatching token would only mark and rewind unchanged parser state.
        if self.token() != t {
            return false;
        }
        self.parse_contextual_modifier_worker()
    }

    #[inline(never)]
    fn parse_contextual_modifier_worker(&mut self) -> bool {
        let state = self.mark();
        if self.next_token_can_follow_modifier() {
            return true;
        }
        self.rewind(state);
        false
    }

    fn parse_any_contextual_modifier(&mut self) -> bool {
        // Not a modifier: Go marks and rewinds an unchanged state (a no-op) before returning false.
        if !ast::is_modifier_kind(self.token()) {
            return false;
        }
        let state = self.mark();
        if self.next_token_can_follow_modifier() {
            return true;
        }
        self.rewind(state);
        false
    }

    fn next_token_can_follow_modifier(&mut self) -> bool {
        match self.token() {
            Kind::ConstKeyword => {
                // 'const' is only a modifier if followed by 'enum'.
                self.next_token() == Kind::EnumKeyword
            }
            Kind::ExportKeyword => {
                self.next_token();
                if self.token() == Kind::DefaultKeyword {
                    return self.look_ahead(Self::next_token_can_follow_default_keyword);
                }
                if self.token() == Kind::TypeKeyword {
                    return self.look_ahead(Self::next_token_can_follow_export_modifier);
                }
                self.can_follow_export_modifier()
            }
            Kind::DefaultKeyword => self.next_token_can_follow_default_keyword(),
            Kind::StaticKeyword => {
                self.next_token();
                self.can_follow_modifier()
            }
            Kind::GetKeyword | Kind::SetKeyword => {
                self.next_token();
                self.can_follow_get_or_set_keyword()
            }
            _ => self.next_token_is_on_same_line_and_can_follow_modifier(),
        }
    }

    fn next_token_can_follow_default_keyword(&mut self) -> bool {
        match self.next_token() {
            Kind::ClassKeyword | Kind::FunctionKeyword | Kind::InterfaceKeyword | Kind::AtToken => {
                true
            }
            Kind::AbstractKeyword => {
                self.look_ahead(Self::next_token_is_class_keyword_on_same_line)
            }
            Kind::AsyncKeyword => {
                self.look_ahead(Self::next_token_is_function_keyword_on_same_line)
            }
            _ => false,
        }
    }

    fn next_token_is_identifier_or_keyword(&mut self) -> bool {
        token_is_identifier_or_keyword(self.next_token())
    }

    fn next_token_is_identifier_or_keyword_or_greater_than(&mut self) -> bool {
        token_is_identifier_or_keyword_or_greater_than(self.next_token())
    }

    fn next_token_is_identifier_or_keyword_on_same_line(&mut self) -> bool {
        self.next_token_is_identifier_or_keyword() && !self.has_preceding_line_break()
    }

    fn next_token_is_identifier_or_keyword_or_literal_on_same_line(&mut self) -> bool {
        (self.next_token_is_identifier_or_keyword()
            || self.token() == Kind::NumericLiteral
            || self.token() == Kind::BigIntLiteral
            || self.token() == Kind::StringLiteral)
            && !self.has_preceding_line_break()
    }

    fn next_token_is_class_keyword_on_same_line(&mut self) -> bool {
        self.next_token() == Kind::ClassKeyword && !self.has_preceding_line_break()
    }

    fn next_token_is_function_keyword_on_same_line(&mut self) -> bool {
        self.next_token() == Kind::FunctionKeyword && !self.has_preceding_line_break()
    }

    fn next_token_can_follow_export_modifier(&mut self) -> bool {
        self.next_token();
        self.can_follow_export_modifier()
    }

    fn can_follow_export_modifier(&self) -> bool {
        self.token() == Kind::AtToken
            || self.token() != Kind::AsteriskToken
                && self.token() != Kind::AsKeyword
                && self.token() != Kind::OpenBraceToken
                && self.can_follow_modifier()
    }

    fn can_follow_modifier(&self) -> bool {
        self.token() == Kind::OpenBracketToken
            || self.token() == Kind::OpenBraceToken
            || self.token() == Kind::AsteriskToken
            || self.token() == Kind::DotDotDotToken
            || self.is_literal_property_name()
    }

    fn can_follow_get_or_set_keyword(&self) -> bool {
        self.token() == Kind::OpenBracketToken || self.is_literal_property_name()
    }

    fn next_token_is_on_same_line_and_can_follow_modifier(&mut self) -> bool {
        self.next_token();
        if self.has_preceding_line_break() {
            return false;
        }
        self.can_follow_modifier()
    }

    fn next_token_is_open_brace(&mut self) -> bool {
        self.next_token() == Kind::OpenBraceToken
    }

    fn next_token_is_slash(&mut self) -> bool {
        self.next_token() == Kind::SlashToken
    }

    fn scan_type_member_start(&mut self) -> bool {
        // Return true if we have the start of a signature member
        if self.token() == Kind::OpenParenToken
            || self.token() == Kind::LessThanToken
            || self.token() == Kind::GetKeyword
            || self.token() == Kind::SetKeyword
        {
            return true;
        }
        let mut id_token = false;
        // Eat up all modifiers, but hold on to the last one in case it is actually an identifier
        while ast::is_modifier_kind(self.token()) {
            id_token = true;
            self.next_token();
        }
        // Index signatures and computed property names are type members
        if self.token() == Kind::OpenBracketToken {
            return true;
        }
        // Try to get the first property-like token following all modifiers
        if self.is_literal_property_name() {
            id_token = true;
            self.next_token();
        }
        // If we were able to get any potential identifier, check that it is
        // the start of a member declaration
        if id_token {
            return self.token() == Kind::OpenParenToken
                || self.token() == Kind::LessThanToken
                || self.token() == Kind::QuestionToken
                || self.token() == Kind::ColonToken
                || self.token() == Kind::CommaToken
                || self.can_parse_semicolon();
        }
        false
    }

    fn scan_class_member_start(&mut self) -> bool {
        let mut id_token = Kind::Unknown;
        if self.token() == Kind::AtToken {
            return true;
        }
        // Eat up all modifiers, but hold on to the last one in case it is actually an identifier.
        while ast::is_modifier_kind(self.token()) {
            id_token = self.token();
            // If the idToken is a class modifier (protected, private, public, and static), it is
            // certain that we are starting to parse class member. This allows better error recovery
            // Example:
            //      public foo() ...     // true
            //      public @dec blah ... // true; we will then report an error later
            //      export public ...    // true; we will then report an error later
            if ast::is_class_member_modifier(id_token) {
                return true;
            }
            self.next_token();
        }
        if self.token() == Kind::AsteriskToken {
            return true;
        }
        // Try to get the first property-like token following all modifiers.
        // This can either be an identifier or the 'get' or 'set' keywords.
        if self.is_literal_property_name() {
            id_token = self.token();
            self.next_token();
        }
        // Index signatures and computed properties are class members; we can parse.
        if self.token() == Kind::OpenBracketToken {
            return true;
        }
        // If we were able to get any potential identifier...
        if id_token != Kind::Unknown {
            // If we have a non-keyword identifier, or if we have an accessor, then it's safe to parse.
            if !ast::is_keyword(id_token)
                || id_token == Kind::SetKeyword
                || id_token == Kind::GetKeyword
            {
                return true;
            }
            // If it *is* a keyword, but not an accessor, check a little farther along
            // to see if it should actually be parsed as a class member.
            match self.token() {
                Kind::OpenParenToken // Method declaration
                | Kind::LessThanToken // Generic Method declaration
                | Kind::ExclamationToken // Non-null assertion on property name
                | Kind::ColonToken // Type Annotation for declaration
                | Kind::EqualsToken // Initializer for declaration
                | Kind::QuestionToken => {
                    // Not valid, but permitted so that it gets caught later on.
                    return true;
                }
                _ => {}
            }
            // Covers
            //  - Semicolons     (declaration termination)
            //  - Closing braces (end-of-class, must be declaration)
            //  - End-of-files   (not valid, but permitted so that it gets caught later on)
            //  - Line-breaks    (enabling *automatic semicolon insertion*)
            return self.can_parse_semicolon();
        }
        false
    }

    fn can_parse_semicolon(&self) -> bool {
        // If there's a real semicolon, then we can always parse it out.
        // We can parse out an optional semicolon in ASI cases in the following cases.
        self.token() == Kind::SemicolonToken
            || self.token() == Kind::CloseBraceToken
            || self.token() == Kind::EndOfFile
            || self.has_preceding_line_break()
    }

    fn try_parse_semicolon(&mut self) -> bool {
        if !self.can_parse_semicolon() {
            return false;
        }
        if self.token() == Kind::SemicolonToken {
            // consume the semicolon if it was explicitly provided.
            self.next_token();
        }
        true
    }

    fn parse_semicolon(&mut self) -> bool {
        self.try_parse_semicolon() || self.parse_expected(Kind::SemicolonToken)
    }

    fn is_literal_property_name(&self) -> bool {
        token_is_identifier_or_keyword(self.token())
            || self.token() == Kind::StringLiteral
            || self.token() == Kind::NumericLiteral
            || self.token() == Kind::BigIntLiteral
    }

    fn is_start_of_statement(&mut self) -> bool {
        match self.token() {
            // 'catch' and 'finally' do not actually indicate that the code is part of a statement,
            // however, we say they are here so that we may gracefully parse them and error later.
            Kind::AtToken
            | Kind::SemicolonToken
            | Kind::OpenBraceToken
            | Kind::VarKeyword
            | Kind::LetKeyword
            | Kind::UsingKeyword
            | Kind::FunctionKeyword
            | Kind::ClassKeyword
            | Kind::EnumKeyword
            | Kind::IfKeyword
            | Kind::DoKeyword
            | Kind::WhileKeyword
            | Kind::ForKeyword
            | Kind::ContinueKeyword
            | Kind::BreakKeyword
            | Kind::ReturnKeyword
            | Kind::WithKeyword
            | Kind::SwitchKeyword
            | Kind::ThrowKeyword
            | Kind::TryKeyword
            | Kind::DebuggerKeyword
            | Kind::CatchKeyword
            | Kind::FinallyKeyword => true,
            Kind::ImportKeyword => {
                self.is_start_of_declaration()
                    || self.is_next_token_open_paren_or_less_than_or_dot()
            }
            Kind::ConstKeyword | Kind::ExportKeyword => self.is_start_of_declaration(),
            Kind::AsyncKeyword
            | Kind::DeclareKeyword
            | Kind::InterfaceKeyword
            | Kind::ModuleKeyword
            | Kind::NamespaceKeyword
            | Kind::TypeKeyword
            | Kind::GlobalKeyword
            | Kind::DeferKeyword => {
                // When these don't start a declaration, they're an identifier in an expression statement
                true
            }
            Kind::AccessorKeyword
            | Kind::PublicKeyword
            | Kind::PrivateKeyword
            | Kind::ProtectedKeyword
            | Kind::StaticKeyword
            | Kind::ReadonlyKeyword => {
                // When these don't start a declaration, they may be the start of a class member if an identifier
                // immediately follows. Otherwise they're an identifier in an expression statement.
                self.is_start_of_declaration()
                    || !self.look_ahead(Self::next_token_is_identifier_or_keyword_on_same_line)
            }
            _ => self.is_start_of_expression(),
        }
    }

    fn is_start_of_declaration(&mut self) -> bool {
        self.look_ahead(Self::scan_start_of_declaration)
    }

    fn scan_start_of_declaration(&mut self) -> bool {
        loop {
            match self.token() {
                Kind::VarKeyword
                | Kind::LetKeyword
                | Kind::ConstKeyword
                | Kind::FunctionKeyword
                | Kind::ClassKeyword
                | Kind::EnumKeyword => {
                    return true;
                }
                Kind::UsingKeyword => return self.is_using_declaration(),
                Kind::AwaitKeyword => return self.is_await_using_declaration(),
                // 'declare', 'module', 'namespace', 'interface'* and 'type' are all legal JavaScript identifiers;
                // however, an identifier cannot be followed by another identifier on the same line. This is what we
                // count on to parse out the respective declarations. For instance, we exploit this to say that
                //
                //    namespace n
                //
                // can be none other than the beginning of a namespace declaration, but need to respect that JavaScript sees
                //
                //    namespace
                //    n
                //
                // as the identifier 'namespace' on one line followed by the identifier 'n' on another.
                // We need to look one token ahead to see if it permissible to try parsing a declaration.
                //
                // *Note*: 'interface' is actually a strict mode reserved word. So while
                //
                //   "use strict"
                //   interface
                //   I {}
                //
                // could be legal, it would add complexity for very little gain.
                Kind::InterfaceKeyword | Kind::TypeKeyword | Kind::DeferKeyword => {
                    return self.next_token_is_identifier_on_same_line();
                }
                Kind::ModuleKeyword | Kind::NamespaceKeyword => {
                    return self.next_token_is_identifier_or_string_literal_on_same_line();
                }
                Kind::AbstractKeyword
                | Kind::AccessorKeyword
                | Kind::AsyncKeyword
                | Kind::DeclareKeyword
                | Kind::PrivateKeyword
                | Kind::ProtectedKeyword
                | Kind::PublicKeyword
                | Kind::ReadonlyKeyword => {
                    let previous_token = self.token();
                    self.next_token();
                    // ASI takes effect for this modifier.
                    if self.has_preceding_line_break() {
                        return false;
                    }
                    if previous_token == Kind::DeclareKeyword && self.token() == Kind::TypeKeyword {
                        // If we see 'declare type', then commit to parsing a type alias. parseTypeAliasDeclaration will
                        // report Line_break_not_permitted_here if needed.
                        return true;
                    }
                    continue;
                }
                Kind::GlobalKeyword => {
                    self.next_token();
                    return self.token() == Kind::OpenBraceToken
                        || self.token() == Kind::Identifier
                        || self.token() == Kind::ExportKeyword;
                }
                Kind::ImportKeyword => {
                    self.next_token();
                    return self.token() == Kind::DeferKeyword
                        || self.token() == Kind::StringLiteral
                        || self.token() == Kind::AsteriskToken
                        || self.token() == Kind::OpenBraceToken
                        || token_is_identifier_or_keyword(self.token());
                }
                Kind::ExportKeyword => {
                    self.next_token();
                    if self.token() == Kind::EqualsToken
                        || self.token() == Kind::AsteriskToken
                        || self.token() == Kind::OpenBraceToken
                        || self.token() == Kind::DefaultKeyword
                        || self.token() == Kind::AsKeyword
                        || self.token() == Kind::AtToken
                    {
                        return true;
                    }
                    if self.token() == Kind::TypeKeyword {
                        self.next_token();
                        return self.token() == Kind::AsteriskToken
                            || self.token() == Kind::OpenBraceToken
                            || self.is_identifier() && !self.has_preceding_line_break();
                    }
                    continue;
                }
                Kind::StaticKeyword => {
                    self.next_token();
                    continue;
                }
                _ => {}
            }
            return false;
        }
    }

    fn is_start_of_expression(&mut self) -> bool {
        if self.is_start_of_left_hand_side_expression() {
            return true;
        }
        match self.token() {
            Kind::PlusToken
            | Kind::MinusToken
            | Kind::TildeToken
            | Kind::ExclamationToken
            | Kind::DeleteKeyword
            | Kind::TypeOfKeyword
            | Kind::VoidKeyword
            | Kind::PlusPlusToken
            | Kind::MinusMinusToken
            | Kind::LessThanToken
            | Kind::AwaitKeyword
            | Kind::YieldKeyword
            | Kind::PrivateIdentifier
            | Kind::AtToken => {
                // Yield/await always starts an expression.  Either it is an identifier (in which case
                // it is definitely an expression).  Or it's a keyword (either because we're in
                // a generator or async function, or in strict mode (or both)) and it started a yield or await expression.
                return true;
            }
            _ => {}
        }
        // Error tolerance.  If we see the start of some binary operator, we consider
        // that the start of an expression.  That way we'll parse out a missing identifier,
        // give a good message about an identifier being missing, and then consume the
        // rest of the binary expression.
        if self.is_binary_operator() {
            return true;
        }
        self.is_identifier()
    }

    fn is_start_of_left_hand_side_expression(&mut self) -> bool {
        match self.token() {
            Kind::ThisKeyword
            | Kind::SuperKeyword
            | Kind::NullKeyword
            | Kind::TrueKeyword
            | Kind::FalseKeyword
            | Kind::NumericLiteral
            | Kind::BigIntLiteral
            | Kind::StringLiteral
            | Kind::NoSubstitutionTemplateLiteral
            | Kind::TemplateHead
            | Kind::OpenParenToken
            | Kind::OpenBracketToken
            | Kind::OpenBraceToken
            | Kind::FunctionKeyword
            | Kind::ClassKeyword
            | Kind::NewKeyword
            | Kind::SlashToken
            | Kind::SlashEqualsToken
            | Kind::Identifier => return true,
            Kind::ImportKeyword => return self.is_next_token_open_paren_or_less_than_or_dot(),
            _ => {}
        }
        self.is_identifier()
    }

    fn is_start_of_type(&mut self, in_start_of_parameter: bool) -> bool {
        match self.token() {
            Kind::AnyKeyword
            | Kind::UnknownKeyword
            | Kind::StringKeyword
            | Kind::NumberKeyword
            | Kind::BigIntKeyword
            | Kind::BooleanKeyword
            | Kind::ReadonlyKeyword
            | Kind::SymbolKeyword
            | Kind::UniqueKeyword
            | Kind::VoidKeyword
            | Kind::UndefinedKeyword
            | Kind::NullKeyword
            | Kind::ThisKeyword
            | Kind::TypeOfKeyword
            | Kind::NeverKeyword
            | Kind::OpenBraceToken
            | Kind::OpenBracketToken
            | Kind::LessThanToken
            | Kind::BarToken
            | Kind::AmpersandToken
            | Kind::NewKeyword
            | Kind::StringLiteral
            | Kind::NumericLiteral
            | Kind::BigIntLiteral
            | Kind::TrueKeyword
            | Kind::FalseKeyword
            | Kind::ObjectKeyword
            | Kind::AsteriskToken
            | Kind::QuestionToken
            | Kind::ExclamationToken
            | Kind::DotDotDotToken
            | Kind::InferKeyword
            | Kind::ImportKeyword
            | Kind::AssertsKeyword
            | Kind::NoSubstitutionTemplateLiteral
            | Kind::TemplateHead => return true,
            Kind::FunctionKeyword => return !in_start_of_parameter,
            Kind::MinusToken => {
                return !in_start_of_parameter
                    && self.look_ahead(Self::next_token_is_numeric_or_big_int_literal);
            }
            Kind::OpenParenToken => {
                // Only consider '(' the start of a type if followed by ')', '...', an identifier, a modifier,
                // or something that starts a type. We don't want to consider things like '(1)' a type.
                return !in_start_of_parameter
                    && self.look_ahead(Self::next_is_parenthesized_or_function_type);
            }
            _ => {}
        }
        self.is_identifier()
    }

    fn next_token_is_numeric_or_big_int_literal(&mut self) -> bool {
        self.next_token();
        self.token() == Kind::NumericLiteral || self.token() == Kind::BigIntLiteral
    }

    fn next_is_parenthesized_or_function_type(&mut self) -> bool {
        self.next_token();
        self.token() == Kind::CloseParenToken
            || self.is_start_of_parameter(false /*isJSDocParameter*/)
            || self.is_start_of_type(false /*inStartOfParameter*/)
    }

    fn is_start_of_parameter(&mut self, is_jsdoc_parameter: bool) -> bool {
        self.token() == Kind::DotDotDotToken
            || self.is_binding_identifier_or_private_identifier_or_pattern()
            || ast::is_modifier_kind(self.token())
            || self.token() == Kind::AtToken
            || self.is_start_of_type(!is_jsdoc_parameter /*inStartOfParameter*/)
    }

    fn is_binding_identifier_or_private_identifier_or_pattern(&self) -> bool {
        self.token() == Kind::OpenBraceToken
            || self.token() == Kind::OpenBracketToken
            || self.token() == Kind::PrivateIdentifier
            || self.is_binding_identifier()
    }

    fn is_next_token_open_paren_or_less_than_or_dot(&mut self) -> bool {
        self.look_ahead(Self::next_token_is_open_paren_or_less_than_or_dot)
    }

    fn next_token_is_open_paren_or_less_than_or_dot(&mut self) -> bool {
        matches!(
            self.next_token(),
            Kind::OpenParenToken | Kind::LessThanToken | Kind::DotToken
        )
    }

    fn next_token_is_identifier_on_same_line(&mut self) -> bool {
        self.next_token();
        self.is_identifier() && !self.has_preceding_line_break()
    }

    fn next_token_is_identifier_or_string_literal_on_same_line(&mut self) -> bool {
        self.next_token();
        (self.is_identifier() || self.token() == Kind::StringLiteral)
            && !self.has_preceding_line_break()
    }

    fn is_import_attribute_name(&self) -> bool {
        token_is_identifier_or_keyword(self.token()) || self.token() == Kind::StringLiteral
    }

    fn is_binary_operator(&self) -> bool {
        if self.in_disallow_in_context() && self.token() == Kind::InKeyword {
            return false;
        }
        ast::get_binary_operator_precedence(self.token()) != OperatorPrecedence::Invalid
    }

    fn is_valid_heritage_clause_object_literal(&mut self) -> bool {
        self.look_ahead(Self::next_is_valid_heritage_clause_object_literal)
    }

    fn next_is_valid_heritage_clause_object_literal(&mut self) -> bool {
        if self.next_token() == Kind::CloseBraceToken {
            // if we see "extends {}" then only treat the {} as what we're extending (and not
            // the class body) if we have:
            //
            //      extends {} {
            //      extends {},
            //      extends {} extends
            //      extends {} implements
            let next = self.next_token();
            return next == Kind::CommaToken
                || next == Kind::OpenBraceToken
                || next == Kind::ExtendsKeyword
                || next == Kind::ImplementsKeyword;
        }
        true
    }

    fn is_heritage_clause(&self) -> bool {
        self.token() == Kind::ExtendsKeyword || self.token() == Kind::ImplementsKeyword
    }

    fn is_heritage_clause_extends_or_implements_keyword(&mut self) -> bool {
        self.is_heritage_clause() && self.look_ahead(Self::next_is_start_of_expression)
    }

    fn next_is_start_of_expression(&mut self) -> bool {
        self.next_token();
        self.is_start_of_expression()
    }

    fn is_using_declaration(&mut self) -> bool {
        // 'using' always starts a lexical declaration if followed by an identifier. We also eagerly parse
        // |ObjectBindingPattern| so that we can report a grammar error during check. We don't parse out
        // |ArrayBindingPattern| since it potentially conflicts with element access (i.e., `using[x]`).
        self.look_ahead(|p| {
            p.next_token_is_binding_identifier_or_start_of_destructuring_on_same_line(
                false, /*disallowOf*/
            )
        })
    }

    fn next_token_is_equals_or_semicolon_or_colon_token(&mut self) -> bool {
        self.next_token();
        self.token() == Kind::EqualsToken
            || self.token() == Kind::SemicolonToken
            || self.token() == Kind::ColonToken
    }

    fn next_token_is_binding_identifier_or_start_of_destructuring_on_same_line(
        &mut self,
        disallow_of: bool,
    ) -> bool {
        self.next_token();
        if disallow_of && self.token() == Kind::OfKeyword {
            return self.look_ahead(Self::next_token_is_equals_or_semicolon_or_colon_token);
        }
        (self.is_binding_identifier() || self.token() == Kind::OpenBraceToken)
            && !self.has_preceding_line_break()
    }

    fn next_token_is_binding_identifier_or_start_of_destructuring_on_same_line_disallow_of(
        &mut self,
    ) -> bool {
        self.next_token_is_binding_identifier_or_start_of_destructuring_on_same_line(
            true, /*disallowOf*/
        )
    }

    fn is_await_using_declaration(&mut self) -> bool {
        self.look_ahead(Self::next_is_using_keyword_then_binding_identifier_or_start_of_object_destructuring_on_same_line)
    }

    fn next_is_using_keyword_then_binding_identifier_or_start_of_object_destructuring_on_same_line(
        &mut self,
    ) -> bool {
        self.next_token() == Kind::UsingKeyword
            && self.next_token_is_binding_identifier_or_start_of_destructuring_on_same_line(
                false, /*disallowOf*/
            )
    }

    fn next_token_is_token_string_literal(&mut self) -> bool {
        self.next_token() == Kind::StringLiteral
    }
}

impl LookaheadParser<'static> for crate::Parser {
    fn new_modifier(&self, kind: Kind) -> Self::Node {
        self.factory.new_modifier(kind)
    }
}
