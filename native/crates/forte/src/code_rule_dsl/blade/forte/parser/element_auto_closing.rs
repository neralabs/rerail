//! Forte's `Parser\Concerns\ProcessesElementAutoClosing`: closing `<li>`, `<p>`, `<td>` and the
//! other elements whose closing tags HTML lets a template omit, when a sibling opens.

use super::super::ParseError;
use super::super::node::NodeKind;
use super::TreeBuilder;
use super::optional_tags::{
    is_in_valid_parent_context, should_auto_close_element, valid_parents_for,
};

impl TreeBuilder<'_, '_> {
    /// `autoCloseElementsForSibling`: `new_element_tag` is lowercase.
    pub(super) fn auto_close_elements_for_sibling(
        &mut self,
        new_element_tag: &[u8],
    ) -> Result<(), ParseError> {
        if new_element_tag.is_empty() {
            return Ok(());
        }
        if let Some(valid_parents) = valid_parents_for(new_element_tag) {
            if let Some(container_index) = self.find_nearest_valid_container(valid_parents)? {
                self.auto_close_elements_between(container_index, new_element_tag)?;
            }
            // Valid parents were specified but none found: be lenient, allow nesting.
            return Ok(());
        }
        // No specific container requirements: only check the immediate parent.
        self.auto_close_immediate_parent(new_element_tag);
        Ok(())
    }

    /// `autoCloseElementsBetween`: closes up to the deepest element above the container that
    /// the new element closes.
    fn auto_close_elements_between(
        &mut self,
        container_index: usize,
        new_element_tag: &[u8],
    ) -> Result<(), ParseError> {
        let mut close_up_to_index = None;
        // Walk downward from the top to (but not including) the container.
        for i in (container_index + 1..self.open_elements.len()).rev() {
            self.budget.spend(1)?;
            let element_idx = self.open_elements[i];
            if !self.is_element_node(element_idx) {
                continue;
            }
            let tag_name = self.tag_name_at(element_idx);
            if tag_name.is_empty() {
                continue;
            }
            // Forte passes the parent's name too, which only a parent's end consults.
            if should_auto_close_element(tag_name, new_element_tag) {
                // Keep the deepest element that needs closing.
                close_up_to_index = Some(i);
            }
        }
        if let Some(close_up_to_index) = close_up_to_index {
            while self.open_elements.len() > close_up_to_index {
                let popped = self.open_elements.pop();
                self.cleanup_tag_name_stack(popped);
            }
        }
        Ok(())
    }

    /// `autoCloseImmediateParent`.
    fn auto_close_immediate_parent(&mut self, new_element_tag: &[u8]) {
        if self.open_elements.len() <= 1 {
            return;
        }
        let Some(&current_idx) = self.open_elements.last() else {
            return;
        };
        if !self.is_element_node(current_idx) {
            return;
        }
        let current_tag_name = self.tag_name_at(current_idx);
        if current_tag_name.is_empty() {
            return;
        }
        let parent_tag_name = self.parent_tag_name_of(current_idx);
        // If the current element is not in a valid parent context, allow nesting (be lenient).
        if !is_in_valid_parent_context(current_tag_name, parent_tag_name) {
            return;
        }
        if should_auto_close_element(current_tag_name, new_element_tag) {
            self.open_elements.pop();
            self.cleanup_tag_name_stack(Some(current_idx));
        }
    }

    /// `closeRemainingElements`: elements still open end at the last token.
    pub(super) fn close_remaining_elements(&mut self) {
        let total_tokens = self.token_total as i32;
        for &element_idx in &self.open_elements {
            if element_idx == 0 {
                continue; // Root node
            }
            let node = &mut self.nodes[element_idx];
            node.token_count = total_tokens - node.token_start;
        }
    }

    /// `findNearestValidContainer`: the stack position of the innermost open element named one
    /// of `valid_parents`.
    fn find_nearest_valid_container(
        &mut self,
        valid_parents: &[&[u8]],
    ) -> Result<Option<usize>, ParseError> {
        for i in (1..self.open_elements.len()).rev() {
            self.budget.spend(1)?;
            let element_idx = self.open_elements[i];
            if !self.is_element_node(element_idx) {
                continue;
            }
            let tag_name = self.tag_name_at(element_idx);
            if tag_name.is_empty() {
                continue;
            }
            if valid_parents.contains(&tag_name) {
                return Ok(Some(i));
            }
        }
        Ok(None)
    }

    /// `isElementNode`.
    fn is_element_node(&self, idx: usize) -> bool {
        self.nodes
            .get(idx)
            .is_some_and(|node| node.kind == NodeKind::Element)
    }

    /// `tagNameAt`: the tag name, or nothing.
    fn tag_name_at(&self, idx: usize) -> &[u8] {
        self.tag_names.get(idx).unwrap_or(b"")
    }

    /// `parentTagNameOf`: the tag name of the node's parent element, if any.
    fn parent_tag_name_of(&self, idx: usize) -> Option<&[u8]> {
        let parent_idx = usize::try_from(self.nodes.get(idx)?.parent).ok()?;
        if parent_idx == 0 {
            return None;
        }
        if self.nodes.get(parent_idx)?.kind != NodeKind::Element {
            return None;
        }
        self.tag_names.get(parent_idx)
    }
}
