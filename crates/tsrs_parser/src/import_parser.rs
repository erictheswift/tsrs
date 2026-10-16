//! Shared import clauses and module export names for both frontend representations.
use crate::{ParsingContext, list_parser::ListParser, utilities::token_is_identifier_or_keyword};
use std::fmt::Display;
use tsrs_ast::{self as ast, Kind, NodeFlags};
use tsrs_core::TextRange;
use tsrs_diagnostics::{self as diagnostics, Message};

/// Import binding grammar. Complete declarations, module-specifier expressions and export `JSDoc`
/// attachment remain the caller's responsibility.
pub trait ImportParser<'text>: ListParser<'text> {
    fn node_loc(node: Self::Node) -> TextRange;
    fn node_flags(node: Self::Node) -> NodeFlags;
    fn node_text(node: Self::Node) -> &'text [u8];
    fn import_specifier_is_type_only(node: Self::Node) -> bool;
    fn set_skip_jsdoc_leading_asterisks(&mut self, skip: bool);
    fn js_error_at_range(
        &mut self,
        loc: TextRange,
        message: &'static Message,
        args: &[&dyn Display],
    );
    fn new_namespace_import(&self, name: Self::Node) -> Self::Node;
    fn new_named_imports(&self, elements: Self::NodeList) -> Self::Node;
    fn new_import_specifier(
        &self,
        is_type_only: bool,
        property_name: Option<Self::Node>,
        name: Self::Node,
    ) -> Self::Node;
    fn new_namespace_export(&self, name: Self::Node) -> Self::Node;
    fn new_import_clause(
        &self,
        phase_modifier: Kind,
        name: Option<Self::Node>,
        named_bindings: Option<Self::Node>,
    ) -> Self::Node;

    /// The import-specifier branch of `check_js_syntax`, including its shared entry guard.
    fn check_js_import_specifier(&mut self, node: Self::Node) -> Self::Node {
        let flags = Self::node_flags(node);
        if !flags.intersects(NodeFlags::JavaScriptFile)
            || flags.intersects(NodeFlags::JSDoc | NodeFlags::Reparsed)
        {
            return node;
        }
        if Self::import_specifier_is_type_only(node) {
            self.js_error_at_range(
                Self::node_loc(node),
                &diagnostics::X_0_declarations_can_only_be_used_in_TypeScript_files,
                &[&"import...type"],
            );
        }
        node
    }

    fn parse_namespace_import(&mut self) -> Self::Node {
        // NameSpaceImport:
        //  * as ImportedBinding
        let pos = self.node_pos();
        self.parse_expected(Kind::AsteriskToken);
        self.parse_expected(Kind::AsKeyword);
        let name = self.parse_identifier();
        let node = self.new_namespace_import(name);
        self.finish_node(node, pos)
    }

    fn parse_named_imports(&mut self) -> Self::Node {
        let pos = self.node_pos();
        // NamedImports:
        //  { }
        //  { ImportsList }
        //  { ImportsList, }
        let imports = self
            .parse_bracketed_list(
                ParsingContext::ImportOrExportSpecifiers,
                Self::parse_import_specifier,
                Kind::OpenBraceToken,
                Kind::CloseBraceToken,
            )
            .unwrap();
        let node = self.new_named_imports(imports);
        self.finish_node(node, pos)
    }

    fn parse_import_specifier(&mut self) -> Self::Node {
        let pos = self.node_pos();
        let (is_type_only, property_name, name) =
            self.parse_import_or_export_specifier(Kind::ImportSpecifier);
        let identifier_name;
        if Self::node_kind(name) == Kind::Identifier {
            identifier_name = name;
        } else {
            let loc = self.skip_range_trivia(Self::node_loc(name));
            self.report_error(loc, &diagnostics::Identifier_expected, &[]);
            identifier_name = self.new_identifier("");
            self.finish_node(
                identifier_name,
                Self::node_loc(name)
                    .pos()
                    .expect_source("parsed nodes have source positions"),
            );
        }
        let node = self.new_import_specifier(is_type_only, property_name, identifier_name);
        let node = self.finish_node(node, pos);
        self.check_js_import_specifier(node)
    }

    fn parse_import_or_export_specifier(
        &mut self,
        kind: Kind,
    ) -> (bool, Option<Self::Node>, Self::Node) {
        // ImportSpecifier:
        //   BindingIdentifier
        //   ModuleExportName as BindingIdentifier
        // ExportSpecifier:
        //   ModuleExportName
        //   ModuleExportName as ModuleExportName
        // let checkIdentifierIsKeyword = isKeyword(token()) && !isIdentifier();
        // let checkIdentifierStart = scanner.getTokenStart();
        // let checkIdentifierEnd = scanner.getTokenEnd();
        let mut is_type_only = false;
        let mut property_name: Option<Self::Node> = None;
        let mut can_parse_as_keyword = true;
        let disallow_keywords = kind == Kind::ImportSpecifier;
        let (mut name, mut name_ok) = self.parse_module_export_name(disallow_keywords);
        if Self::node_kind(name) == Kind::Identifier && Self::node_text(name) == b"type" {
            // If the first token of an import specifier is 'type', there are a lot of possibilities,
            // especially if we see 'as' afterwards:
            //
            // import { type } from "mod";          - isTypeOnly: false,   name: type
            // import { type as } from "mod";       - isTypeOnly: true,    name: as
            // import { type as as } from "mod";    - isTypeOnly: false,   name: as,    propertyName: type
            // import { type as as as } from "mod"; - isTypeOnly: true,    name: as,    propertyName: as
            if self.token() == Kind::AsKeyword {
                // { type as ...? }
                let first_as = self.parse_identifier_name();
                if self.token() == Kind::AsKeyword {
                    // { type as as ...? }
                    let second_as = self.parse_identifier_name();
                    if self.can_parse_module_export_name() {
                        // { type as as something }
                        // { type as as "something" }
                        is_type_only = true;
                        property_name = Some(first_as);
                        (name, name_ok) = self.parse_module_export_name(disallow_keywords);
                        can_parse_as_keyword = false;
                    } else {
                        // { type as as }
                        property_name = Some(name);
                        name = second_as;
                        can_parse_as_keyword = false;
                    }
                } else if self.can_parse_module_export_name() {
                    // { type as something }
                    // { type as "something" }
                    property_name = Some(name);
                    can_parse_as_keyword = false;
                    (name, name_ok) = self.parse_module_export_name(disallow_keywords);
                } else {
                    // { type as }
                    is_type_only = true;
                    name = first_as;
                }
            } else if self.can_parse_module_export_name() {
                // { type something ...? }
                // { type "something" ...? }
                is_type_only = true;
                (name, name_ok) = self.parse_module_export_name(disallow_keywords);
            }
        }
        if can_parse_as_keyword && self.token() == Kind::AsKeyword {
            property_name = Some(name);
            self.parse_expected(Kind::AsKeyword);
            (name, name_ok) = self.parse_module_export_name(disallow_keywords);
        }

        if !name_ok {
            let loc = self.skip_range_trivia(Self::node_loc(name));
            self.report_error(loc, &diagnostics::Identifier_expected, &[]);
        }

        (is_type_only, property_name, name)
    }

    fn can_parse_module_export_name(&self) -> bool {
        token_is_identifier_or_keyword(self.token()) || self.token() == Kind::StringLiteral
    }

    fn parse_module_export_name(&mut self, disallow_keywords: bool) -> (Self::Node, bool) {
        let mut name_ok = true;

        if self.token() == Kind::StringLiteral {
            return (self.parse_literal_expression(), name_ok);
        }
        if disallow_keywords && ast::is_keyword(self.token()) && !self.is_identifier() {
            name_ok = false;
        }
        (self.parse_identifier_name(), name_ok)
    }

    fn parse_namespace_export(&mut self, pos: u32) -> Self::Node {
        let (export_name, _) = self.parse_module_export_name(false /*disallowKeywords*/);
        let node = self.new_namespace_export(export_name);
        self.finish_node(node, pos)
    }

    fn next_token_is_from_keyword_or_equals_token(&mut self) -> bool {
        self.next_token();
        self.token() == Kind::FromKeyword || self.token() == Kind::EqualsToken
    }

    fn token_after_import_definitely_produces_import_declaration(&self) -> bool {
        self.token() == Kind::AsteriskToken || self.token() == Kind::OpenBraceToken
    }

    fn token_after_imported_identifier_definitely_produces_import_declaration(&self) -> bool {
        // In `import id ___`, the current token decides whether to produce
        // an ImportDeclaration or ImportEqualsDeclaration.
        self.token() == Kind::CommaToken || self.token() == Kind::FromKeyword
    }

    fn try_parse_import_clause(
        &mut self,
        identifier: Option<Self::Node>,
        pos: u32,
        phase_modifier: Kind,
        skip_jsdoc_leading_asterisks: bool,
    ) -> Option<Self::Node> {
        // ImportDeclaration:
        //  import ImportClause from ModuleSpecifier ;
        //  import ModuleSpecifier;
        if identifier.is_some()
            || self.token() == Kind::AsteriskToken
            || self.token() == Kind::OpenBraceToken
        {
            let import_clause = self.parse_import_clause(
                identifier,
                pos,
                phase_modifier,
                skip_jsdoc_leading_asterisks,
            );
            self.parse_expected(Kind::FromKeyword);
            return Some(import_clause);
        }
        None
    }

    fn parse_import_clause(
        &mut self,
        identifier: Option<Self::Node>,
        pos: u32,
        phase_modifier: Kind,
        skip_jsdoc_leading_asterisks: bool,
    ) -> Self::Node {
        // ImportClause:
        //  ImportedDefaultBinding
        //  NameSpaceImport
        //  NamedImports
        //  ImportedDefaultBinding, NameSpaceImport
        //  ImportedDefaultBinding, NamedImports
        // If there was no default import or if there is comma token after default import
        // parse namespace or named imports
        let mut named_bindings = None;
        let save_has_await_identifier = self.statement_has_await_identifier();
        if identifier.is_none() || self.parse_optional(Kind::CommaToken) {
            if skip_jsdoc_leading_asterisks {
                self.set_skip_jsdoc_leading_asterisks(true);
            }
            if self.token() == Kind::AsteriskToken {
                named_bindings = Some(self.parse_namespace_import());
            } else {
                named_bindings = Some(self.parse_named_imports());
            }
            if skip_jsdoc_leading_asterisks {
                self.set_skip_jsdoc_leading_asterisks(false);
            }
        }
        let result = self.new_import_clause(phase_modifier, identifier, named_bindings);
        let result = self.finish_node(result, pos);
        self.set_statement_has_await_identifier(save_has_await_identifier);
        result
    }
}

impl ImportParser<'static> for crate::Parser {
    fn node_loc(node: Self::Node) -> TextRange {
        node.loc()
    }
    fn node_flags(node: Self::Node) -> NodeFlags {
        node.flags()
    }
    fn node_text(node: Self::Node) -> &'static [u8] {
        node.text().as_bytes()
    }
    fn import_specifier_is_type_only(node: Self::Node) -> bool {
        node.is_type_only()
    }
    fn set_skip_jsdoc_leading_asterisks(&mut self, skip: bool) {
        self.scanner.set_skip_jsdoc_leading_asterisks(skip);
    }
    fn js_error_at_range(
        &mut self,
        loc: TextRange,
        message: &'static Message,
        args: &[&dyn Display],
    ) {
        crate::Parser::js_error_at_range(self, loc, message, args);
    }
    fn new_namespace_import(&self, name: Self::Node) -> Self::Node {
        self.factory.new_namespace_import(name)
    }
    fn new_named_imports(&self, elements: Self::NodeList) -> Self::Node {
        self.factory.new_named_imports(elements)
    }
    fn new_import_specifier(
        &self,
        is_type_only: bool,
        property_name: Option<Self::Node>,
        name: Self::Node,
    ) -> Self::Node {
        self.factory
            .new_import_specifier(is_type_only, property_name, name)
    }
    fn new_namespace_export(&self, name: Self::Node) -> Self::Node {
        self.factory.new_namespace_export(name)
    }
    fn new_import_clause(
        &self,
        phase_modifier: Kind,
        name: Option<Self::Node>,
        named_bindings: Option<Self::Node>,
    ) -> Self::Node {
        self.factory
            .new_import_clause(phase_modifier, name, named_bindings)
    }
}
