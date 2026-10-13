//! Forte's `Parser\Concerns\ProcessesElementNames`: tag names, which may hold echoes and PHP
//! (`<x-{{ $type }}>`), and their child nodes.

use super::super::ParseError;
use super::super::directives::token_index::token_text;
use super::super::node::NodeKind;
use super::super::token::TokenType;
use super::TreeBuilder;
use super::construct_scanner::{
    count_construct_tokens, get_node_kind, is_construct_start, scan_construct,
};

impl TreeBuilder<'_, '_> {
    /// `scanElementName`: `[tokenCount, genericOffset]` of the name at the position, which it
    /// moves past.
    pub(super) fn scan_element_name(&mut self) -> Result<(usize, usize), ParseError> {
        let limit = self.tokens.len();
        let mut count = 0;
        let mut generic_offset = 0;
        let tag_name_start = self.pos;
        while self.pos < limit {
            self.spend()?;
            let token_type = self.tokens[self.pos].kind;
            if is_element_name_terminator(token_type) {
                if token_type == TokenType::TsxGenericType {
                    generic_offset = self.pos - tag_name_start + 1;
                }
                break;
            }
            if token_type == TokenType::TagName {
                count += 1;
                self.pos += 1;
                continue;
            }
            if is_construct_start(token_type) {
                let (new_pos, construct_count) =
                    scan_construct(self.tokens, self.pos, limit, self.budget)?;
                count += construct_count;
                self.pos = new_pos;
                continue;
            }
            // Unexpected token ends the name.
            break;
        }
        Ok((count, generic_offset))
    }

    /// `buildElementNameParts`: a text child per tag name token, an echo or PHP child per
    /// construct.
    pub(super) fn build_element_name_parts(
        &mut self,
        start_pos: usize,
        token_count: usize,
        parent_idx: usize,
    ) -> Result<(), ParseError> {
        let end_pos = start_pos + token_count;
        let mut i = start_pos;
        let mut last_child_idx = None;
        while i < end_pos {
            self.spend()?;
            let Some(token_type) = self.kind_at(i) else {
                break;
            };
            let mut child_token_count = 1;
            let mut child = None;
            if token_type == TokenType::TagName {
                child = Some(Self::create_node(NodeKind::Text, parent_idx, i, 1));
            } else if is_construct_start(token_type) {
                let node_kind = get_node_kind(token_type).unwrap_or(NodeKind::Echo);
                child_token_count = count_construct_tokens(self.tokens, i, end_pos, self.budget)?;
                child = Some(Self::create_node(
                    node_kind,
                    parent_idx,
                    i,
                    child_token_count,
                ));
            }
            if let Some(child) = child {
                last_child_idx = Some(self.link_child_node(parent_idx, last_child_idx, child));
            }
            i += child_token_count;
        }
        Ok(())
    }

    /// `getTagNameText`: the tag name tokens' text, appended to `text`.
    pub(super) fn get_tag_name_text(
        &self,
        start_pos: usize,
        token_count: usize,
        text: &mut Vec<u8>,
    ) {
        for idx in start_pos..start_pos.saturating_add(token_count) {
            let Some(token) = self.tokens.get(idx) else {
                break;
            };
            if token.kind == TokenType::TagName {
                text.extend_from_slice(token_text(self.source, token.start, token.end));
            }
        }
    }

    /// `getOpenElementTagName`: an element's name from its tokens (`data` taken for the
    /// count, as Forte takes it); nothing for other nodes.
    pub(super) fn get_open_element_tag_name(&self, node_idx: usize, text: &mut Vec<u8>) {
        let node = &self.nodes[node_idx];
        if node.kind != NodeKind::Element {
            return;
        }
        let (Ok(tag_name_count), Ok(tag_name_start)) = (
            usize::try_from(node.data),
            usize::try_from(node.token_start + 1),
        ) else {
            return;
        };
        self.get_tag_name_text(tag_name_start, tag_name_count, text);
    }

    /// `isDynamicTagName`: a token of the name is not a tag name token.
    pub(super) fn is_dynamic_tag_name(&self, start_pos: usize, token_count: usize) -> bool {
        for idx in start_pos..start_pos + token_count {
            let Some(token) = self.tokens.get(idx) else {
                break;
            };
            if token.kind != TokenType::TagName {
                return true;
            }
        }
        false
    }
}

/// `isElementNameTerminator`.
fn is_element_name_terminator(token_type: TokenType) -> bool {
    matches!(
        token_type,
        TokenType::Whitespace
            | TokenType::GreaterThan
            | TokenType::Slash
            | TokenType::Directive
            | TokenType::TsxGenericType
    )
}
