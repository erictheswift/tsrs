//! `JSDoc` names, links and tag descriptions shared by both frontend representations.
use crate::utilities::{is_keyword_or_punctuation, token_is_identifier_or_keyword};
use crate::{list_parser::ListParser, parser_3::is_reserved_word};
use tsrs_ast::Kind;
use tsrs_core::{TextRange, stringutil};
use tsrs_diagnostics::{self as diagnostics, Message};
use tsrs_scanner::{self as scanner, storage::TokenValue};

/// `JSDoc` lexical grammar. Callers provide comment bounds, context flags and the current token.
///
/// Tag descriptions, simple/unknown tags and `@see` are supported. Complete comment/tag dispatch,
/// attachment and type expressions remain with the enclosing parser. Finish comment parsing before
/// rewinding: checkpoints do not restore scratch buffers, which are cleared at the end of each call.
pub trait JSDocParser<'text>: ListParser<'text> {
    type TextList;

    fn next_jsdoc_comment_text_token(&mut self, in_backticks: bool) -> Kind;
    fn jsdoc_tag_comments_space(&mut self) -> &mut Vec<&'text str>;
    fn jsdoc_tag_comments_parts_space(&mut self) -> &mut Vec<Self::Node>;
    fn copy_jsdoc_text(&self, text: &str) -> &'text str;
    fn new_jsdoc_text(&self, text: &[&'text str]) -> Self::Node;
    fn new_jsdoc_unknown_tag(
        &self,
        tag_name: Self::Node,
        comment: Option<Self::NodeList>,
    ) -> Self::Node;
    fn new_jsdoc_see_tag(
        &self,
        tag_name: Self::Node,
        name_expression: Option<Self::Node>,
        comment: Option<Self::NodeList>,
    ) -> Self::Node;

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

    fn parse_trailing_tag_comments(
        &mut self,
        pos: u32,
        end: u32,
        margin: u32,
        indent_text: &str,
    ) -> Option<Self::NodeList> {
        let mut margin = margin;
        // some tags, like typedef and callback, have already parsed their comments earlier
        if indent_text.is_empty() {
            margin += end - pos;
        }
        let mut initial_margin: &'text str = "";
        if margin < indent_text.len() as u32 {
            initial_margin = self.copy_jsdoc_text(&indent_text[margin as usize..]);
        }
        self.parse_tag_comments(margin, Some(initial_margin))
    }

    fn parse_tag_comments(
        &mut self,
        indent: u32,
        initial_margin: Option<&'text str>,
    ) -> Option<Self::NodeList> {
        let mut indent = indent;
        let comments_pos = self.node_pos();
        let mut comments: Vec<&'text str> = std::mem::take(self.jsdoc_tag_comments_space());
        let mut parts: Vec<Self::Node> = std::mem::take(self.jsdoc_tag_comments_parts_space());
        let mut link_end = None;
        let mut state = JsdocState::BeginningOfLine;
        let mut backtick_count = 0;
        let mut in_fenced_code_block = false;
        let mut margin = None;
        macro_rules! push_comment {
            ($text:expr_2021) => {{
                let text: &'text str = $text;
                if margin.is_none() {
                    margin = Some(indent);
                }
                comments.push(text);
                indent += text.len() as u32;
            }};
        }

        if let Some(initial_margin) = initial_margin {
            // jump straight to saving comments if there is some initial indentation
            if !initial_margin.is_empty() {
                push_comment!(initial_margin);
            }
            state = JsdocState::SawAsterisk;
        }
        let mut tok = self.token();
        loop {
            // Detect fenced code blocks by counting consecutive backtick tokens.
            // Three or more consecutive backticks toggle the fenced code block state.
            if tok != Kind::BacktickToken && backtick_count > 0 {
                if backtick_count >= 3 {
                    in_fenced_code_block = !in_fenced_code_block;
                }
                backtick_count = 0;
            }
            let mut is_default = false;
            match tok {
                Kind::NewLineTrivia => {
                    state = JsdocState::BeginningOfLine;
                    // don't use pushComment here because we want to keep the margin unchanged
                    comments.push(self.scanner().token_text());
                    indent = 0;
                }
                Kind::AtToken => {
                    if !in_fenced_code_block && self.scanner().can_follow_jsdoc_at() {
                        let pos = self.scanner().token_end() - 1;
                        self.reset_scanner_pos(pos);
                        break;
                    }
                    if in_fenced_code_block {
                        state = JsdocState::SavingBackticks;
                    } else {
                        state = JsdocState::SavingComments;
                    }
                    push_comment!(self.scanner().token_text());
                }
                Kind::EndOfFile => {
                    // Done
                    break;
                }
                Kind::WhitespaceTrivia => {
                    assert!(
                        !(state == JsdocState::SavingComments
                            || state == JsdocState::SavingBackticks),
                        "whitespace shouldn't come from the scanner while saving comment text"
                    );
                    let whitespace = self.scanner().token_text();
                    // if the whitespace crosses the margin, take only the whitespace that passes the margin
                    let whitespace_len = whitespace.len() as u32;
                    if let Some(margin) = margin.filter(|&margin| indent + whitespace_len > margin)
                    {
                        comments.push(&whitespace[margin.saturating_sub(indent) as usize..]);
                        if in_fenced_code_block {
                            state = JsdocState::SavingBackticks;
                        } else {
                            state = JsdocState::SavingComments;
                        }
                    }
                    indent += whitespace_len;
                }
                Kind::OpenBraceToken => {
                    if in_fenced_code_block {
                        state = JsdocState::SavingBackticks;
                        push_comment!(self.scanner().token_text());
                    } else {
                        state = JsdocState::SavingComments;
                        let comment_end = self.scanner().token_full_start();
                        let link_start = self.scanner().token_end() - 1;
                        let link = self.parse_jsdoc_link(link_start);
                        if let Some(link) = link {
                            let comment_start = link_end.unwrap_or(comments_pos);
                            let text = self.new_jsdoc_text(&comments);
                            let text = self.finish_node_with_end(text, comment_start, comment_end);
                            parts.push(text);
                            parts.push(link);
                            comments.clear();
                            link_end = Some(self.scanner().token_end());
                        } else {
                            push_comment!(self.scanner().token_text());
                        }
                    }
                }
                Kind::BacktickToken => {
                    backtick_count += 1;
                    if state == JsdocState::SavingBackticks {
                        state = JsdocState::SavingComments;
                    } else {
                        state = JsdocState::SavingBackticks;
                    }
                    push_comment!(self.scanner().token_text());
                }
                Kind::JSDocCommentTextToken => {
                    if state != JsdocState::SavingBackticks {
                        if in_fenced_code_block {
                            state = JsdocState::SavingBackticks;
                        } else {
                            state = JsdocState::SavingComments;
                        }
                        // leading identifiers start recording as well
                    }
                    push_comment!(self.scanner().token_value().utf8());
                }
                Kind::AsteriskToken => {
                    if state == JsdocState::BeginningOfLine {
                        // leading asterisks start recording on the *next* (non-whitespace) token
                        state = JsdocState::SawAsterisk;
                        indent += 1;
                    } else {
                        // record the * as a comment
                        is_default = true;
                    }
                }
                _ => is_default = true,
            }
            if is_default {
                if state != JsdocState::SavingBackticks {
                    if in_fenced_code_block {
                        state = JsdocState::SavingBackticks;
                    } else {
                        state = JsdocState::SavingComments;
                    }
                    // leading identifiers start recording as well
                }
                push_comment!(self.scanner().token_text());
            }
            if state == JsdocState::SavingComments || state == JsdocState::SavingBackticks {
                tok = self.next_jsdoc_comment_text_token(state == JsdocState::SavingBackticks);
            } else {
                tok = self.next_token_jsdoc();
            }
        }

        remove_leading_newlines(&mut comments);
        remove_trailing_whitespace(&mut comments);
        if !comments.is_empty() {
            let comment_start = link_end.unwrap_or(comments_pos);
            let text = self.new_jsdoc_text(&comments);
            let text = self.finish_node(text, comment_start);
            parts.push(text);
        }
        comments.clear();
        *self.jsdoc_tag_comments_space() = comments;

        let result = if !parts.is_empty() {
            let end = self.scanner().token_end();
            Some(self.new_node_list(TextRange::new(comments_pos, end), &parts))
        } else {
            None
        };
        parts.clear();
        *self.jsdoc_tag_comments_parts_space() = parts;
        result
    }

    fn parse_unknown_tag(
        &mut self,
        start: u32,
        tag_name: Self::Node,
        indent: u32,
        indent_text: &str,
    ) -> Self::Node {
        let pos = self.node_pos();
        let comments = self.parse_trailing_tag_comments(start, pos, indent, indent_text);
        let node = self.new_jsdoc_unknown_tag(tag_name, comments);
        self.finish_node(node, start)
    }

    fn parse_see_tag(
        &mut self,
        start: u32,
        tag_name: Self::Node,
        indent: u32,
        indent_text: &str,
    ) -> Self::Node {
        let has_name_reference = self.is_identifier()
            && !self.scanner().text()[self.scanner().token_end() as usize..].starts_with("://")
            || self.token() == Kind::OpenBraceToken
                && self.look_ahead(Self::next_token_is_identifier_or_keyword);
        let mut name_expression = None;
        if has_name_reference {
            name_expression = Some(self.parse_jsdoc_name_reference());
        }
        let pos = self.node_pos();
        let comments = self.parse_trailing_tag_comments(start, pos, indent, indent_text);
        let node = self.new_jsdoc_see_tag(tag_name, name_expression, comments);
        self.finish_node(node, start)
    }

    fn parse_simple_tag(
        &mut self,
        start: u32,
        create_tag: impl FnOnce(&mut Self, Self::Node, Option<Self::NodeList>) -> Self::Node,
        tag_name: Self::Node,
        margin: u32,
        indent_text: &str,
    ) -> Self::Node {
        let pos = self.node_pos();
        let comments = self.parse_trailing_tag_comments(start, pos, margin, indent_text);
        let node = create_tag(self, tag_name, comments);
        self.finish_node(node, start)
    }
}

fn is_jsdoc_link_tag(kind: &str) -> bool {
    kind == "link" || kind == "linkcode" || kind == "linkplain"
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum JsdocState {
    BeginningOfLine,
    SawAsterisk,
    SavingComments,
    SavingBackticks,
}

pub(crate) fn remove_leading_newlines(comments: &mut Vec<&str>) {
    let mut i = 0;
    i += (i..comments.len())
        .take_while(|&i| comments[i].trim_start_matches(['\r', '\n']).is_empty())
        .count();
    comments.drain(..i);
}

fn trim_end(s: &str) -> &str {
    s.trim_end_matches(|c: char| stringutil::is_white_space_like(c as u32))
}

pub(crate) fn remove_trailing_whitespace(comments: &mut Vec<&str>) {
    let mut end = comments.len();
    for (i, element) in comments.iter_mut().enumerate().rev() {
        let trimmed = trim_end(element);
        if trimmed.is_empty() {
            end = i;
        } else {
            (*element) = trimmed;
            break;
        }
    }
    comments.truncate(end);
}

impl JSDocParser<'static> for crate::Parser {
    type TextList = &'static [&'static str];

    fn next_jsdoc_comment_text_token(&mut self, in_backticks: bool) -> Kind {
        crate::Parser::next_jsdoc_comment_text_token(self, in_backticks)
    }
    fn jsdoc_tag_comments_space(&mut self) -> &mut Vec<&'static str> {
        &mut self.jsdoc_tag_comments_space
    }
    fn jsdoc_tag_comments_parts_space(&mut self) -> &mut Vec<Self::Node> {
        &mut self.jsdoc_tag_comments_parts_space
    }
    fn copy_jsdoc_text(&self, text: &str) -> &'static str {
        tsrs_core::alloc_str(text)
    }
    fn new_jsdoc_text(&self, text: &[&'static str]) -> Self::Node {
        self.factory.new_jsdoc_text(tsrs_core::alloc_slice(text))
    }
    fn new_jsdoc_unknown_tag(
        &self,
        tag_name: Self::Node,
        comment: Option<Self::NodeList>,
    ) -> Self::Node {
        self.factory.new_jsdoc_unknown_tag(tag_name, comment)
    }
    fn new_jsdoc_see_tag(
        &self,
        tag_name: Self::Node,
        name_expression: Option<Self::Node>,
        comment: Option<Self::NodeList>,
    ) -> Self::Node {
        self.factory
            .new_jsdoc_see_tag(tag_name, name_expression, comment)
    }

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
