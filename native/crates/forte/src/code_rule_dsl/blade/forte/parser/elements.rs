//! Forte's `Parser\Concerns\ProcessesElements`: opening tags (with their names and attributes)
//! and closing tags, matched against the open elements within the innermost open directive.

use super::super::ParseError;
use super::super::node::NodeKind;
use super::super::token::TokenType;
use super::TreeBuilder;

impl TreeBuilder<'_, '_> {
    /// `processElementStart`.
    pub(super) fn process_element_start(&mut self) -> Result<(), ParseError> {
        let token_count = self.token_total;
        let start_pos = self.pos;
        self.pos += 1;
        if start_pos >= token_count || self.tokens[start_pos].kind != TokenType::LessThan {
            return Ok(());
        }
        // Closing tag?
        if self.kind_at(self.pos) == Some(TokenType::Slash) {
            return self.process_element_end();
        }
        let tag_name_start = self.pos;
        let (tag_name_count, generic_offset) = self.scan_element_name()?;
        if tag_name_count == 0 {
            // No valid tag name. Treat "<" as text.
            self.add_child(Self::create_node(NodeKind::Text, 0, start_pos, 1));
            return Ok(());
        }
        let mut lower_new_tag = std::mem::take(&mut self.scratch);
        lower_new_tag.clear();
        self.get_tag_name_text(tag_name_start, tag_name_count, &mut lower_new_tag);
        lower_new_tag.make_ascii_lowercase();
        let result = self.open_element(
            start_pos,
            tag_name_start,
            tag_name_count,
            generic_offset,
            &lower_new_tag,
        );
        self.scratch = lower_new_tag;
        result
    }

    /// The rest of `processElementStart`, once the tag's lowercase name is known.
    fn open_element(
        &mut self,
        start_pos: usize,
        tag_name_start: usize,
        tag_name_count: usize,
        generic_offset: usize,
        lower_new_tag: &[u8],
    ) -> Result<(), ParseError> {
        let token_count = self.token_total;
        // Auto-close applicable siblings before adding this node.
        self.auto_close_elements_for_sibling(lower_new_tag)?;
        let element_idx = self.add_child(Self::create_node(NodeKind::Element, 0, start_pos, 0));
        let element_name_idx = self.create_first_child(
            element_idx,
            NodeKind::ElementName,
            tag_name_start,
            tag_name_count,
        );
        self.build_element_name_parts(tag_name_start, tag_name_count, element_name_idx)?;
        // Consume TsxGenericType, if present.
        if self.kind_at(self.pos) == Some(TokenType::TsxGenericType) {
            self.pos += 1;
        }
        self.build_attributes(element_idx)?;
        // Self-closing detection.
        let mut self_closing = false;
        let mut synthetic_close = false;
        if self.kind_at(self.pos) == Some(TokenType::Slash) {
            self.pos += 1;
            self_closing = true;
        }
        if self.pos < token_count {
            match self.tokens[self.pos].kind {
                TokenType::GreaterThan => self.pos += 1,
                TokenType::SyntheticClose => {
                    self.pos += 1;
                    synthetic_close = true;
                }
                _ => {}
            }
        }
        let element = &mut self.nodes[element_idx];
        element.token_count = self.pos as i32 - start_pos as i32;
        element.generic_offset = if generic_offset > 0 {
            generic_offset as i32 + 1
        } else {
            0
        };
        element.data = self_closing as i32;
        // Decide if an element stays open.
        let mut should_stay_open = !self_closing;
        if synthetic_close
            && should_stay_open
            && self.kind_at(self.pos) == Some(TokenType::LessThan)
            && !self.has_attribute_tokens(start_pos, self.pos)?
        {
            should_stay_open = false;
        }
        if !should_stay_open {
            return Ok(());
        }
        // Keep element open (check depth to prevent DoS).
        self.check_element_depth()?;
        self.open_elements.push(element_idx);
        self.tag_names.set(element_idx, lower_new_tag);
        // Push to the per-tag-name stack for static tags.
        if !lower_new_tag.is_empty() && !self.is_dynamic_tag_name(tag_name_start, tag_name_count) {
            let depth = self.open_elements.len() - 1;
            match self.tag_name_stacks.get_mut(lower_new_tag) {
                Some(stack) => stack.push(depth),
                None => {
                    self.tag_name_stacks
                        .insert(lower_new_tag.into(), vec![depth]);
                }
            }
        }
        Ok(())
    }

    /// `processElementEnd`: at the `/` after `<`.
    fn process_element_end(&mut self) -> Result<(), ParseError> {
        let token_count = self.token_total;
        let start_pos = self.pos - 1;
        self.pos += 1; // skip "/"
        let tag_name_start_pos = self.pos;
        let (tag_name_count, _) = self.scan_element_name()?;
        let mut closing_tag_name = std::mem::take(&mut self.scratch);
        closing_tag_name.clear();
        if tag_name_count > 0 {
            self.get_tag_name_text(tag_name_start_pos, tag_name_count, &mut closing_tag_name);
        }
        let result = self.close_element(
            start_pos,
            tag_name_start_pos,
            tag_name_count,
            token_count,
            &closing_tag_name,
        );
        self.scratch = closing_tag_name;
        result
    }

    /// The rest of `processElementEnd`, once the closing tag's name is known.
    fn close_element(
        &mut self,
        start_pos: usize,
        tag_name_start_pos: usize,
        tag_name_count: usize,
        token_count: usize,
        closing_tag_name: &[u8],
    ) -> Result<(), ParseError> {
        // Skip to ">" or SyntheticClose.
        while self.pos < token_count
            && !matches!(
                self.tokens[self.pos].kind,
                TokenType::GreaterThan | TokenType::SyntheticClose
            )
        {
            self.spend()?;
            self.pos += 1;
        }
        if self.pos < token_count {
            self.pos += 1; // consume end token
        }
        // Directive scope boundary.
        let search_limit = match self.open_directives.last() {
            Some(current_directive) => current_directive.element_stack_base + 1,
            None => 1, // skip root
        };
        let lower_closing_tag = closing_tag_name.to_ascii_lowercase();
        let mut found_match = false;
        let mut match_depth = 0;
        let mut matched_open_elements_index = 0;
        if let Some(stack) = self.tag_name_stacks.get(lower_closing_tag.as_slice()) {
            let open_elements_count = self.open_elements.len();
            for &open_elements_index in stack.iter().rev() {
                self.budget.spend(1)?;
                if open_elements_index >= search_limit && open_elements_index < open_elements_count
                {
                    let element_idx = self.open_elements[open_elements_index];
                    let element_tag_name = self.tag_names.get(element_idx).unwrap_or(b"");
                    if element_tag_name == lower_closing_tag.as_slice() {
                        matched_open_elements_index = open_elements_index;
                        found_match = true;
                        match_depth = open_elements_count - open_elements_index;
                        break;
                    }
                }
            }
        }
        // Fallback search (dynamic tags / missed).
        let mut built_name = Vec::new();
        for i in (search_limit..self.open_elements.len()).rev() {
            self.budget.spend(1)?;
            let element_idx = self.open_elements[i];
            if self.nodes[element_idx].kind == NodeKind::DirectiveBlock {
                continue;
            }
            let open_tag_name: &[u8] = match self.tag_names.get(element_idx) {
                Some(name) => name,
                None => {
                    built_name.clear();
                    self.get_open_element_tag_name(element_idx, &mut built_name);
                    &built_name
                }
            };
            if open_tag_name.eq_ignore_ascii_case(closing_tag_name)
                || is_slot_closing_match(open_tag_name, closing_tag_name)
            {
                found_match = true;
                match_depth = self.open_elements.len() - i;
                matched_open_elements_index = i;
                break;
            }
        }
        if found_match {
            let element_idx = self.open_elements[matched_open_elements_index];
            let end_pos = self.pos; // After closing token.
            let element = &mut self.nodes[element_idx];
            element.token_count = end_pos as i32 - element.token_start;
            let closing_name_idx = self.create_and_link_closing_name_node(
                element_idx,
                tag_name_start_pos,
                tag_name_count,
            );
            self.build_element_name_parts(tag_name_start_pos, tag_name_count, closing_name_idx)?;
            // Pop matched element and any nested open elements.
            for _ in 0..match_depth {
                if self.open_elements.len() > 1 {
                    let popped = self.open_elements.pop();
                    self.cleanup_tag_name_stack(popped);
                }
            }
            return Ok(());
        }
        // Didn't find a match, so we will emit UnpairedClosingTag.
        let mut node = Self::create_node(
            NodeKind::UnpairedClosingTag,
            0,
            start_pos,
            self.pos - start_pos,
        );
        node.data = tag_name_count as i32;
        self.add_child(node);
        Ok(())
    }

    /// `hasAttributeTokens`.
    fn has_attribute_tokens(
        &mut self,
        start_pos: usize,
        end_pos: usize,
    ) -> Result<bool, ParseError> {
        let limit = end_pos.min(self.token_total);
        for i in start_pos..limit {
            self.budget.spend(1)?;
            if matches!(
                self.tokens[i].kind,
                TokenType::AttributeName
                    | TokenType::Equals
                    | TokenType::Quote
                    | TokenType::AttributeValue
                    | TokenType::EscapedAttribute
                    | TokenType::BoundAttribute
                    | TokenType::ShorthandAttribute
                    | TokenType::JsxAttributeValue
                    | TokenType::JsxShorthandAttribute
                    | TokenType::Directive
            ) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `createAndLinkClosingNameNode`: the closing name, linked as the element's last child.
    fn create_and_link_closing_name_node(
        &mut self,
        parent_idx: usize,
        token_start: usize,
        token_count: usize,
    ) -> usize {
        let idx = self.nodes.len();
        self.nodes.push(Self::create_node(
            NodeKind::ClosingElementName,
            parent_idx,
            token_start,
            token_count,
        ));
        let last_child = self.nodes[parent_idx].last_child;
        match self.node_mut(last_child) {
            Some(last) => last.next_sibling = idx as i32,
            None => self.nodes[parent_idx].first_child = idx as i32,
        }
        self.nodes[parent_idx].last_child = idx as i32;
        idx
    }
}

/// `isSlotClosingMatch`: `</x-slot>` closes `<x-slot:name>` and `<x-slot[name]>`.
fn is_slot_closing_match(open_tag_name: &[u8], closing_tag_name: &[u8]) -> bool {
    if !closing_tag_name.eq_ignore_ascii_case(b"x-slot") {
        return false;
    }
    let starts_with = |prefix: &[u8]| {
        open_tag_name.len() >= prefix.len()
            && open_tag_name[..prefix.len()].eq_ignore_ascii_case(prefix)
    };
    starts_with(b"x-slot:") || starts_with(b"x-slot[")
}
