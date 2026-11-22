//! Forte's `Parser\Concerns\ProcessesMarkup`: `@verbatim`, doctypes, CDATA, declarations
//! (`<!ELEMENT …>` and the like, with attribute-like parts) and single tokens as text.

use super::super::ParseError;
use super::super::node::NodeKind;
use super::super::token::TokenType;
use super::TreeBuilder;
use super::construct_scanner::{count_construct_tokens, get_node_kind};

impl TreeBuilder<'_, '_> {
    /// `createVerbatim`.
    pub(super) fn create_verbatim(&mut self, start_pos: usize) -> Result<(), ParseError> {
        self.create_block_node(start_pos, TokenType::VerbatimEnd, NodeKind::Verbatim)
    }

    /// `createDoctype`.
    pub(super) fn create_doctype(&mut self, start_pos: usize) -> Result<(), ParseError> {
        self.create_block_node(start_pos, TokenType::DoctypeEnd, NodeKind::Doctype)
    }

    /// `createCdata`.
    pub(super) fn create_cdata(&mut self, start_pos: usize) -> Result<(), ParseError> {
        self.create_block_node(start_pos, TokenType::CdataEnd, NodeKind::Cdata)
    }

    /// `createDecl`.
    pub(super) fn create_decl(&mut self, start_pos: usize) -> Result<(), ParseError> {
        let token_count = self.tokens.len();
        let mut end_pos = self.pos;
        let mut has_closing = false;
        while end_pos < token_count {
            self.spend()?;
            if self.tokens[end_pos].kind == TokenType::DeclEnd {
                has_closing = true;
                break;
            }
            end_pos += 1;
        }
        let decl_token_count = if has_closing {
            end_pos + 1 - start_pos
        } else {
            end_pos - start_pos
        };
        let mut node = Self::create_node(NodeKind::Decl, 0, start_pos, decl_token_count);
        node.data = has_closing as i32;
        let decl_idx = self.add_child(node);
        self.check_element_depth()?;
        self.open_elements.push(decl_idx);
        self.pos += 1;
        while self.pos < token_count {
            self.spend()?;
            let token_type = self.tokens[self.pos].kind;
            if token_type == TokenType::DeclEnd {
                self.pos += 1;
                break;
            }
            if token_type == TokenType::Whitespace {
                self.add_child(Self::create_node(
                    NodeKind::AttributeWhitespace,
                    0,
                    self.pos,
                    1,
                ));
                self.pos += 1;
                continue;
            }
            if matches!(
                token_type,
                TokenType::EchoStart | TokenType::RawEchoStart | TokenType::TripleEchoStart
            ) {
                self.process_decl_echo(end_pos)?;
                continue;
            }
            if matches!(
                token_type,
                TokenType::AttributeName
                    | TokenType::BoundAttribute
                    | TokenType::EscapedAttribute
                    | TokenType::ShorthandAttribute
            ) {
                self.build_decl_attribute(end_pos)?;
                continue;
            }
            self.pos += 1;
        }
        self.open_elements.pop();
        Ok(())
    }

    /// `processDeclEcho`.
    fn process_decl_echo(&mut self, decl_end_pos: usize) -> Result<(), ParseError> {
        let start_pos = self.pos;
        let start_type = self.tokens[start_pos].kind;
        let node_kind = get_node_kind(start_type).unwrap_or(NodeKind::Echo);
        let construct_token_count =
            count_construct_tokens(self.tokens, start_pos, decl_end_pos, self.budget)?;
        self.pos += construct_token_count;
        self.add_child(Self::create_node(
            node_kind,
            0,
            start_pos,
            construct_token_count,
        ));
        Ok(())
    }

    /// `buildDeclAttribute`.
    fn build_decl_attribute(&mut self, attr_end: usize) -> Result<(), ParseError> {
        let token_count = self.tokens.len();
        let attr_start = self.pos;
        let name_start = self.pos;
        let mut name_count = 0;
        while self.pos < token_count && self.pos < attr_end {
            self.spend()?;
            if matches!(
                self.tokens[self.pos].kind,
                TokenType::AttributeName
                    | TokenType::EchoStart
                    | TokenType::RawEchoStart
                    | TokenType::TripleEchoStart
            ) {
                name_count += 1;
                self.pos += 1;
                continue;
            }
            break;
        }
        if name_count == 0 {
            self.pos += 1;
            return Ok(());
        }
        let (has_value, value_start, value_count) =
            self.scan_decl_equals_and_value(attr_end, token_count)?;
        let attr_token_count = self.pos - attr_start;
        let attr_idx = self.add_child(Self::create_node(
            NodeKind::Attribute,
            0,
            attr_start,
            attr_token_count,
        ));
        let name_idx =
            self.create_first_child(attr_idx, NodeKind::AttributeName, name_start, name_count);
        self.build_decl_attribute_parts(name_start, name_count, name_idx, false);
        if has_value && value_count > 0 {
            let value_idx = self.create_sibling_child(
                name_idx,
                attr_idx,
                NodeKind::AttributeValue,
                value_start,
                value_count,
            );
            self.build_decl_attribute_parts(value_start, value_count, value_idx, true);
        }
        Ok(())
    }

    /// `scanDeclAttributeValue`: the tokens of a quoted value; the number consumed.
    fn scan_decl_attribute_value(&mut self, attr_end: usize) -> Result<usize, ParseError> {
        let token_count = self.tokens.len();
        if self.pos >= token_count || self.tokens[self.pos].kind != TokenType::Quote {
            return Ok(0);
        }
        let mut count = 1;
        self.pos += 1;
        while self.pos < token_count && self.pos < attr_end {
            self.spend()?;
            let token_type = self.tokens[self.pos].kind;
            if token_type == TokenType::Quote {
                count += 1;
                self.pos += 1;
                break;
            }
            if token_type == TokenType::DeclEnd {
                break;
            }
            count += 1;
            self.pos += 1;
        }
        Ok(count)
    }

    /// `buildDeclAttributeParts`: a child per token, quotes and echo ends left out of values.
    fn build_decl_attribute_parts(
        &mut self,
        start: usize,
        count: usize,
        parent_idx: usize,
        is_value: bool,
    ) {
        let mut last_child_idx = None;
        for token_pos in start..start + count {
            let Some(token_type) = self.kind_at(token_pos) else {
                break;
            };
            if is_value
                && matches!(
                    token_type,
                    TokenType::Quote
                        | TokenType::EchoEnd
                        | TokenType::RawEchoEnd
                        | TokenType::TripleEchoEnd
                )
            {
                continue;
            }
            let node_kind = decl_part_node_kind(token_type);
            let child = Self::create_node(node_kind, parent_idx, token_pos, 1);
            last_child_idx = Some(self.link_child_node(parent_idx, last_child_idx, child));
        }
    }

    /// `scanDeclEqualsAndValue`: `[hasValue, valueStart, valueCount]`.
    fn scan_decl_equals_and_value(
        &mut self,
        attr_end: usize,
        token_count: usize,
    ) -> Result<(bool, usize, usize), ParseError> {
        let mut has_value = false;
        let mut value_start = 0;
        let mut value_count = 0;
        let mut check_pos = self.pos;
        if check_pos < token_count && self.tokens[check_pos].kind == TokenType::Whitespace {
            check_pos += 1;
        }
        if check_pos < token_count && self.tokens[check_pos].kind == TokenType::Equals {
            self.pos = check_pos + 1;
            if self.pos < token_count && self.tokens[self.pos].kind == TokenType::Whitespace {
                self.pos += 1;
            }
            if self.pos < token_count {
                value_start = self.pos;
                value_count = self.scan_decl_attribute_value(attr_end)?;
                has_value = value_count > 0;
            }
        }
        Ok((has_value, value_start, value_count))
    }

    /// `emitSingleTokenText`.
    pub(super) fn emit_single_token_text(&mut self) {
        let start_pos = self.pos;
        self.pos += 1;
        self.add_child(Self::create_node(NodeKind::Text, 0, start_pos, 1));
    }
}

/// `declPartNodeKind`.
fn decl_part_node_kind(token_type: TokenType) -> NodeKind {
    match token_type {
        TokenType::EchoStart => NodeKind::Echo,
        TokenType::RawEchoStart => NodeKind::RawEcho,
        TokenType::TripleEchoStart => NodeKind::TripleEcho,
        _ => NodeKind::Text,
    }
}
