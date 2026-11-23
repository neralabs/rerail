//! Forte's `Parser\Concerns\ProcessesElementAttributes`: an opening tag's attributes, as
//! attribute nodes (name and value children, each split into text, echo, PHP and directive
//! parts), whitespace nodes, and the directives between them, which build their own blocks
//! within the tag.

use super::super::ParseError;
use super::super::directives::helper::{check_directive_args, extract_directive_name_from_token};
use super::super::node::NodeKind;
use super::super::token::TokenType;
use super::TreeBuilder;
use super::construct_scanner::{
    advance_past, count_construct_tokens, get_node_kind, is_construct_start, is_echo_start,
    is_php_start,
};

/// What `scanAttributeBounds` returns.
struct Bounds {
    length: usize,
    end: usize,
    is_name_value: bool,
    name_count: usize,
    value_start: usize,
    value_count: usize,
}

impl TreeBuilder<'_, '_> {
    /// `buildAttributes`: the element is the current parent meanwhile, and the open directives
    /// and conditions outside the tag are set aside. (Forte's `$lastChildIdx` parameter is
    /// unused, and it hands extension token types to attribute extensions; none are registered.)
    pub(super) fn build_attributes(&mut self, element_idx: usize) -> Result<(), ParseError> {
        let attr_end = self.find_attribute_region_end()?;
        // Push element to stack so directive children are correctly parented.
        self.open_elements.push(element_idx);
        let saved_open_directives = std::mem::take(&mut self.open_directives);
        let saved_open_conditions = std::mem::take(&mut self.open_conditions);
        let saved_attribute_region_end = self.attribute_region_end.replace(attr_end);
        while self.pos < attr_end {
            self.spend()?;
            let token_type = self.tokens[self.pos].kind;
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
            if token_type == TokenType::Directive {
                self.process_directive()?;
                continue;
            }
            if token_type == TokenType::JsxShorthandAttribute {
                self.add_child(Self::create_node(NodeKind::JsxAttribute, 0, self.pos, 1));
                self.pos += 1;
                continue;
            }
            self.build_unified_attribute(attr_end)?;
        }
        self.close_remaining_attribute_directives();
        self.open_directives = saved_open_directives;
        self.open_conditions = saved_open_conditions;
        self.attribute_region_end = saved_attribute_region_end;
        self.open_elements.pop();
        Ok(())
    }

    /// `closeRemainingAttributeDirectives`: blocks still open at the tag's end end there.
    fn close_remaining_attribute_directives(&mut self) {
        while let Some(frame) = self.open_conditions.pop() {
            self.pop_if_top(frame.current_branch_idx);
            self.pop_if_top(frame.block_idx);
            let block = &mut self.nodes[frame.block_idx];
            block.token_count = self.pos as i32 - block.token_start;
        }
        while let Some(frame) = self.open_directives.pop() {
            self.pop_if_top(frame.start_directive_idx);
            self.pop_if_top(frame.block_idx);
            let block = &mut self.nodes[frame.block_idx];
            block.token_count = self.pos as i32 - block.token_start;
        }
    }

    /// `buildAttributeNameParts`.
    fn build_attribute_name_parts(
        &mut self,
        start_pos: usize,
        token_count: usize,
        parent_idx: usize,
    ) -> Result<(), ParseError> {
        self.build_parts(start_pos, start_pos + token_count, parent_idx, false)
    }

    /// `buildAttributeValueParts`: the parts between the quotes.
    fn build_attribute_value_parts(
        &mut self,
        start_pos: usize,
        token_count: usize,
        parent_idx: usize,
    ) -> Result<(), ParseError> {
        let mut end_pos = start_pos + token_count;
        let mut i = start_pos;
        // Skip opening quote.
        if i < end_pos && self.kind_at(i) == Some(TokenType::Quote) {
            i += 1;
        }
        // Stop before closing the quote.
        if end_pos > start_pos && self.kind_at(end_pos - 1) == Some(TokenType::Quote) {
            end_pos -= 1;
        }
        self.build_parts(i, end_pos, parent_idx, true)
    }

    /// `buildUnifiedAttribute`.
    fn build_unified_attribute(&mut self, attr_end: usize) -> Result<(), ParseError> {
        let attr_start = self.pos;
        let bounds = self.scan_attribute_bounds(attr_end)?;
        if bounds.length == 0 {
            self.pos += 1; // Skip unknown things.
            return Ok(());
        }
        self.pos = bounds.end;
        if bounds.is_name_value {
            let attr_idx = self.add_child(Self::create_node(
                NodeKind::Attribute,
                0,
                attr_start,
                bounds.length,
            ));
            let name_idx = self.create_first_child(
                attr_idx,
                NodeKind::AttributeName,
                attr_start,
                bounds.name_count,
            );
            self.build_attribute_name_parts(attr_start, bounds.name_count, name_idx)?;
            if bounds.value_count > 0 {
                let value_idx = self.create_sibling_child(
                    name_idx,
                    attr_idx,
                    NodeKind::AttributeValue,
                    bounds.value_start,
                    bounds.value_count,
                );
                self.build_attribute_value_parts(
                    bounds.value_start,
                    bounds.value_count,
                    value_idx,
                )?;
            }
            return Ok(());
        }
        // Standalone attribute.
        let first_type = self.tokens[attr_start].kind;
        if bounds.length <= 3 && is_echo_start(first_type) {
            self.pos = attr_start;
            return self.process_construct_in_attributes(NodeKind::Echo, Some(attr_end));
        }
        if bounds.length <= 5 && is_php_start(first_type) {
            self.pos = attr_start;
            return self.process_construct_in_attributes(NodeKind::PhpTag, Some(attr_end));
        }
        let attr_idx = self.add_child(Self::create_node(
            NodeKind::Attribute,
            0,
            attr_start,
            bounds.length,
        ));
        let name_idx =
            self.create_first_child(attr_idx, NodeKind::AttributeName, attr_start, bounds.length);
        self.build_attribute_name_parts(attr_start, bounds.length, name_idx)
    }

    /// `findAttributeRegionEnd`: the `>`, synthetic close, or `/` before one, ending the tag.
    fn find_attribute_region_end(&mut self) -> Result<usize, ParseError> {
        let mut attr_end = self.pos;
        let token_count = self.token_total;
        while attr_end < token_count {
            self.budget.spend(1)?;
            let token_type = self.tokens[attr_end].kind;
            if matches!(
                token_type,
                TokenType::GreaterThan | TokenType::SyntheticClose
            ) {
                break;
            }
            if token_type == TokenType::Slash
                && attr_end + 1 < token_count
                && matches!(
                    self.tokens[attr_end + 1].kind,
                    TokenType::GreaterThan | TokenType::SyntheticClose
                )
            {
                break;
            }
            attr_end += 1;
        }
        Ok(attr_end)
    }

    /// `scanAttributeBounds`: the token span of the attribute at the position.
    fn scan_attribute_bounds(&mut self, attr_end: usize) -> Result<Bounds, ParseError> {
        let attr_start = self.pos;
        let mut equals_pos = None;
        let mut name_end = None;
        let mut scan_pos = self.pos;
        let mut last_non_whitespace = self.pos;
        let kind = |tokens: &[super::super::token::Token], idx: usize| {
            tokens.get(idx).map(|token| token.kind)
        };
        while scan_pos < attr_end {
            self.budget.spend(1)?;
            let token_type = kind(self.tokens, scan_pos);
            if token_type == Some(TokenType::Whitespace) {
                let mut look_ahead = scan_pos + 1;
                while look_ahead < attr_end
                    && kind(self.tokens, look_ahead) == Some(TokenType::Whitespace)
                {
                    self.budget.spend(1)?;
                    look_ahead += 1;
                }
                if look_ahead < attr_end && kind(self.tokens, look_ahead) == Some(TokenType::Equals)
                {
                    scan_pos = look_ahead;
                    continue;
                }
                break; // whitespace ends attribute
            }
            if token_type == Some(TokenType::Equals) {
                equals_pos = Some(scan_pos);
                name_end = Some(last_non_whitespace);
                scan_pos += 1;
                while scan_pos < attr_end
                    && kind(self.tokens, scan_pos) == Some(TokenType::Whitespace)
                {
                    self.budget.spend(1)?;
                    scan_pos += 1;
                }
                if scan_pos < attr_end {
                    if kind(self.tokens, scan_pos) == Some(TokenType::Quote) {
                        scan_pos += 1; // opening "
                        while scan_pos < attr_end
                            && kind(self.tokens, scan_pos) != Some(TokenType::Quote)
                        {
                            self.budget.spend(1)?;
                            scan_pos = advance_past(self.tokens, scan_pos, attr_end, self.budget)?;
                        }
                        if scan_pos < attr_end {
                            scan_pos += 1; // closing "
                        }
                    } else {
                        while scan_pos < attr_end
                            && kind(self.tokens, scan_pos) != Some(TokenType::Whitespace)
                        {
                            self.budget.spend(1)?;
                            scan_pos = advance_past(self.tokens, scan_pos, attr_end, self.budget)?;
                        }
                    }
                }
                break;
            }
            scan_pos = advance_past(self.tokens, scan_pos, attr_end, self.budget)?;
            last_non_whitespace = scan_pos;
        }
        let length = scan_pos - attr_start;
        let none = Bounds {
            length,
            end: scan_pos,
            is_name_value: false,
            name_count: 0,
            value_start: 0,
            value_count: 0,
        };
        if length == 0 {
            return Ok(none);
        }
        if let (Some(equals_pos), Some(name_end)) = (equals_pos, name_end) {
            let name_count = name_end - attr_start;
            let mut value_start = equals_pos + 1;
            while value_start < scan_pos
                && kind(self.tokens, value_start) == Some(TokenType::Whitespace)
            {
                self.budget.spend(1)?;
                value_start += 1;
            }
            return Ok(Bounds {
                length,
                end: scan_pos,
                is_name_value: true,
                name_count,
                value_start,
                value_count: scan_pos - value_start,
            });
        }
        Ok(none)
    }

    /// `processConstructInAttributes`: an echo or PHP span standing alone among attributes.
    fn process_construct_in_attributes(
        &mut self,
        default: NodeKind,
        limit: Option<usize>,
    ) -> Result<(), ParseError> {
        let start_pos = self.pos;
        let token_count = limit.unwrap_or(self.token_total);
        let start_type = self.tokens[start_pos].kind;
        let node_kind = get_node_kind(start_type).unwrap_or(default);
        let construct_token_count =
            count_construct_tokens(self.tokens, start_pos, token_count, self.budget)?;
        self.pos += construct_token_count;
        self.add_child(Self::create_node(
            node_kind,
            0,
            start_pos,
            construct_token_count,
        ));
        Ok(())
    }

    /// `buildParts`: text, construct and (in values) directive children of a name or value.
    fn build_parts(
        &mut self,
        start_pos: usize,
        end_pos: usize,
        parent_idx: usize,
        as_value: bool,
    ) -> Result<(), ParseError> {
        let mut i = start_pos;
        let mut last_child_idx = None;
        while i < end_pos {
            self.spend()?;
            let Some(token) = self.tokens.get(i) else {
                break;
            };
            let token_type = token.kind;
            let mut child_token_count = 1;
            let child = if is_text_like(token_type, as_value) {
                Self::create_node(NodeKind::Text, parent_idx, i, 1)
            } else if is_construct_start(token_type) {
                let node_kind = get_node_kind(token_type).unwrap_or(NodeKind::Echo);
                child_token_count = count_construct_tokens(self.tokens, i, end_pos, self.budget)?;
                Self::create_node(node_kind, parent_idx, i, child_token_count)
            } else if as_value && token_type == TokenType::Directive {
                let directive_name = extract_directive_name_from_token(token, self.source);
                let args_info = check_directive_args(self.tokens, self.source, i + 1, end_pos);
                child_token_count = 1 + args_info.consumed;
                Self::create_directive_node(
                    NodeKind::Directive,
                    parent_idx,
                    i,
                    child_token_count,
                    &directive_name,
                    args_info.args_content,
                    None,
                )
            } else {
                Self::create_node(NodeKind::Text, parent_idx, i, 1)
            };
            last_child_idx = Some(self.link_child_node(parent_idx, last_child_idx, child));
            i += child_token_count;
        }
        Ok(())
    }
}

/// `isTextLike`.
fn is_text_like(token_type: TokenType, as_value: bool) -> bool {
    if as_value {
        return matches!(token_type, TokenType::AttributeValue | TokenType::Text);
    }
    matches!(
        token_type,
        TokenType::AttributeName
            | TokenType::BoundAttribute
            | TokenType::EscapedAttribute
            | TokenType::ShorthandAttribute
            | TokenType::Text
    )
}
