//! JSX names, text and closing-tag grammar shared by both frontend representations.
use crate::name_parser::NameParser;
use tsrs_ast::Kind;
use tsrs_diagnostics as diagnostics;
use tsrs_scanner::storage::ScannerStorage;

/// JSX lexical grammar. Opening elements, attributes with expressions, children and complete
/// JSX expressions still require the enclosing expression/type parser.
pub trait JsxParser<'text>: NameParser<'text> {
    /// Re-scan the current child without replacing the parser's token, as in `parse_jsx_children`.
    fn re_scan_jsx_token(&mut self, allow_multiline_jsx_text: bool) -> Kind;
    fn scan_jsx_text(&mut self) -> Kind;
    fn scan_jsx_identifier(&mut self) -> Kind;
    fn scan_jsx_attribute_value(&mut self) -> Kind;
    fn is_jsx_namespaced_name(node: Self::Node) -> bool;
    fn jsx_tag_name(node: Self::Node) -> Self::Node;
    fn tag_names_are_equivalent(lhs: Self::Node, rhs: Self::Node) -> bool;
    fn new_jsx_text(
        &self,
        text: <Self::Storage as ScannerStorage<'text>>::Value,
        contains_only_trivia_white_spaces: bool,
    ) -> Self::Node;
    fn new_jsx_namespaced_name(&self, namespace: Self::Node, name: Self::Node) -> Self::Node;
    fn new_jsx_property_access(&self, expression: Self::Node, name: Self::Node) -> Self::Node;
    fn new_jsx_closing_element(&self, tag_name: Self::Node) -> Self::Node;
    fn new_jsx_closing_fragment(&self) -> Self::Node;

    fn parse_jsx_text(&mut self) -> Self::Node {
        let pos = self.node_pos();
        let text = self.scanner().token_value();
        let result = self.new_jsx_text(text, self.token() == Kind::JsxTextAllWhiteSpaces);
        self.scan_jsx_text();
        self.finish_node(result, pos)
    }

    fn parse_jsx_closing_element(
        &mut self,
        open: Self::Node,
        in_expression_context: bool,
    ) -> Self::Node {
        let pos = self.node_pos();
        self.parse_expected(Kind::LessThanSlashToken);
        let tag_name = self.parse_jsx_element_name();
        if self.parse_expected_with_diagnostic(
            Kind::GreaterThanToken,
            None,  /*diagnosticMessage*/
            false, /*shouldAdvance*/
        ) {
            // manually advance the scanner in order to look for jsx text inside jsx
            if in_expression_context
                || !Self::tag_names_are_equivalent(Self::jsx_tag_name(open), tag_name)
            {
                self.next_token();
            } else {
                self.scan_jsx_text();
            }
        }
        let node = self.new_jsx_closing_element(tag_name);
        self.finish_node(node, pos)
    }

    fn parse_jsx_element_name(&mut self) -> Self::Node {
        let pos = self.node_pos();
        // JsxElement can have name in the form of
        //      propertyAccessExpression
        //      primaryExpression in the form of an identifier and "this" keyword
        // We can't just simply use parseLeftHandSideExpressionOrHigher because then we will start consider class,function etc as a keyword
        // We only want to consider "this" as a primaryExpression
        let initial_expression = self.parse_jsx_tag_name();
        if Self::is_jsx_namespaced_name(initial_expression) {
            return initial_expression; // `a:b.c` is invalid syntax, don't even look for the `.` if we parse `a:b`, and let `parseAttribute` report "unexpected :" instead.
        }
        let mut expression = initial_expression;
        while self.parse_optional(Kind::DotToken) {
            let name = self.parse_right_side_of_dot(
                true,  /*allowIdentifierNames*/
                false, /*allowPrivateIdentifiers*/
                false, /*allowUnicodeEscapeSequenceInIdentifierName*/
            );
            let node = self.new_jsx_property_access(expression, name);
            expression = self.finish_node(node, pos);
        }
        expression
    }

    fn parse_jsx_tag_name(&mut self) -> Self::Node {
        let pos = self.node_pos();
        self.scan_jsx_identifier();
        let is_this = self.token() == Kind::ThisKeyword;
        let tag_name = self.parse_identifier_name_error_on_unicode_escape_sequence();
        if self.parse_optional(Kind::ColonToken) {
            self.scan_jsx_identifier();
            let name = self.parse_identifier_name_error_on_unicode_escape_sequence();
            let node = self.new_jsx_namespaced_name(tag_name, name);
            return self.finish_node(node, pos);
        }
        if is_this {
            let result = self.new_keyword_expression(Kind::ThisKeyword);
            return self.finish_node(result, pos);
        }
        tag_name
    }

    fn parse_jsx_attribute_name(&mut self) -> Self::Node {
        let pos = self.node_pos();
        self.scan_jsx_identifier();
        let attr_name = self.parse_identifier_name_error_on_unicode_escape_sequence();
        if self.parse_optional(Kind::ColonToken) {
            self.scan_jsx_identifier();
            let name = self.parse_identifier_name_error_on_unicode_escape_sequence();
            let node = self.new_jsx_namespaced_name(attr_name, name);
            return self.finish_node(node, pos);
        }
        attr_name
    }

    fn parse_jsx_closing_fragment(&mut self, in_expression_context: bool) -> Self::Node {
        let pos = self.node_pos();
        self.parse_expected(Kind::LessThanSlashToken);
        if self.parse_expected_with_diagnostic(
            Kind::GreaterThanToken,
            Some(&diagnostics::Expected_corresponding_closing_tag_for_JSX_fragment),
            false, /*shouldAdvance*/
        ) {
            // manually advance the scanner in order to look for jsx text inside jsx
            if in_expression_context {
                self.next_token();
            } else {
                self.scan_jsx_text();
            }
        }
        let node = self.new_jsx_closing_fragment();
        self.finish_node(node, pos)
    }
}

impl JsxParser<'static> for crate::Parser {
    fn re_scan_jsx_token(&mut self, allow_multiline_jsx_text: bool) -> Kind {
        let token = self.scanner.re_scan_jsx_token(allow_multiline_jsx_text);
        self.report_scan_errors();
        token
    }
    fn scan_jsx_text(&mut self) -> Kind {
        crate::Parser::scan_jsx_text(self)
    }
    fn scan_jsx_identifier(&mut self) -> Kind {
        crate::Parser::scan_jsx_identifier(self)
    }
    fn scan_jsx_attribute_value(&mut self) -> Kind {
        crate::Parser::scan_jsx_attribute_value(self)
    }
    fn is_jsx_namespaced_name(node: Self::Node) -> bool {
        tsrs_ast::is_jsx_namespaced_name(node)
    }
    fn jsx_tag_name(node: Self::Node) -> Self::Node {
        node.tag_name()
    }
    fn tag_names_are_equivalent(lhs: Self::Node, rhs: Self::Node) -> bool {
        tsrs_ast::tag_names_are_equivalent(lhs, rhs)
    }
    fn new_jsx_text(
        &self,
        text: &'static str,
        contains_only_trivia_white_spaces: bool,
    ) -> Self::Node {
        self.factory
            .new_jsx_text(text, contains_only_trivia_white_spaces)
    }
    fn new_jsx_namespaced_name(&self, namespace: Self::Node, name: Self::Node) -> Self::Node {
        self.factory.new_jsx_namespaced_name(namespace, name)
    }
    fn new_jsx_property_access(&self, expression: Self::Node, name: Self::Node) -> Self::Node {
        self.factory.new_property_access_expression(
            expression,
            None,
            name,
            tsrs_ast::NodeFlags::None,
        )
    }
    fn new_jsx_closing_element(&self, tag_name: Self::Node) -> Self::Node {
        self.factory.new_jsx_closing_element(tag_name)
    }
    fn new_jsx_closing_fragment(&self) -> Self::Node {
        self.factory.new_jsx_closing_fragment()
    }
}
