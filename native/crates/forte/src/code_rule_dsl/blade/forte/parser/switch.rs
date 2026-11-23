//! Forte's `Parser\Concerns\ProcessesSwitchDirectives`: `@switch` blocks, their cases and
//! breaks, and `@endswitch`.

use super::super::ParseError;
use super::super::node::{NodeKind, StructureRole};
use super::{SwitchFrame, TreeBuilder};
use std::borrow::Cow;

impl<'a> TreeBuilder<'a, '_> {
    /// `isSwitchRelatedDirective`.
    pub(super) fn is_switch_related_directive(&self, directive_name: &[u8]) -> bool {
        let directives = self.directives;
        if directives.is_switch(directive_name) {
            return true;
        }
        if directive_name == b"endswitch" {
            return true;
        }
        !self.open_switches.is_empty()
            && (directives.is_switch_branch(directive_name)
                || directives.is_switch_terminator(directive_name))
    }

    /// `processSwitchDirective`.
    pub(super) fn process_switch_directive(
        &mut self,
        directive_name: Cow<'a, [u8]>,
        start_pos: usize,
        token_count: usize,
        args_content: Option<&'a [u8]>,
    ) -> Result<(), ParseError> {
        let directives = self.directives;
        if &*directive_name == b"endswitch" && !self.open_switches.is_empty() {
            return self.close_switch(&directive_name, start_pos, token_count, args_content);
        }
        if directives.is_switch_terminator(&directive_name) && !self.open_switches.is_empty() {
            self.process_switch_break(&directive_name, start_pos, token_count, args_content);
            return Ok(());
        }
        if !self.open_switches.is_empty() && directives.is_switch_branch(&directive_name) {
            return self.open_switch_case(&directive_name, start_pos, token_count, args_content);
        }
        if directives.is_switch(&directive_name) {
            return self.open_switch(directive_name, start_pos, token_count, args_content);
        }
        self.create_standalone_directive(&directive_name, start_pos, token_count, args_content);
        Ok(())
    }

    /// `openSwitch`: a block node, its `@switch` directive, and a switch frame.
    fn open_switch(
        &mut self,
        directive_name: Cow<'a, [u8]>,
        start_pos: usize,
        token_count: usize,
        args_content: Option<&'a [u8]>,
    ) -> Result<(), ParseError> {
        let block_node = Self::create_directive_node(
            NodeKind::DirectiveBlock,
            0,
            start_pos,
            token_count,
            &directive_name,
            args_content,
            None,
        );
        let block_idx = self.add_child(block_node);
        let element_stack_base = self.open_elements.len();
        self.check_element_depth()?;
        self.open_elements.push(block_idx);
        let switch_directive_node = Self::create_directive_node(
            NodeKind::Directive,
            block_idx,
            start_pos,
            token_count,
            &directive_name,
            args_content,
            Some(StructureRole::Opening),
        );
        let switch_directive_idx = self.add_child(switch_directive_node);
        // Forte checks the directive stack's depth here, not the switches'.
        self.check_directive_depth()?;
        self.open_switches.push(SwitchFrame {
            block_idx,
            switch_directive_idx,
            current_case_idx: None,
            name: directive_name,
            element_stack_base,
        });
        self.check_element_depth()?;
        self.open_elements.push(switch_directive_idx);
        self.pos += token_count;
        Ok(())
    }

    /// `openSwitchCase`: a case of the innermost switch, open until the next case or the end.
    fn open_switch_case(
        &mut self,
        directive_name: &[u8],
        start_pos: usize,
        token_count: usize,
        args_content: Option<&'a [u8]>,
    ) -> Result<(), ParseError> {
        let Some(switch_idx) = self.open_switches.len().checked_sub(1) else {
            // Treat as standalone.
            self.create_standalone_directive(directive_name, start_pos, token_count, args_content);
            return Ok(());
        };
        let element_stack_base = self.open_switches[switch_idx].element_stack_base;
        self.close_unmatched_structures_at_depth(element_stack_base + 2, start_pos)?;
        let Some(frame) = self.open_switches.get(switch_idx) else {
            return Ok(());
        };
        let (switch_directive_idx, element_stack_base, current_case_idx) = (
            frame.switch_directive_idx,
            frame.element_stack_base,
            frame.current_case_idx,
        );
        // +2 because both block and @switch directive are on the stack.
        self.pop_elements_to_depth(element_stack_base + 2);
        if let Some(current_case_idx) = current_case_idx {
            self.pop_if_top(current_case_idx);
        }
        let case_node = Self::create_directive_node(
            NodeKind::Directive,
            switch_directive_idx,
            start_pos,
            token_count,
            directive_name,
            args_content,
            Some(StructureRole::Intermediate),
        );
        let case_idx = self.add_child(case_node);
        self.open_switches[switch_idx].current_case_idx = Some(case_idx);
        self.check_element_depth()?;
        self.open_elements.push(case_idx);
        self.pos += token_count;
        Ok(())
    }

    /// `processSwitchBreak`: a `@break` in the current case.
    fn process_switch_break(
        &mut self,
        directive_name: &[u8],
        start_pos: usize,
        token_count: usize,
        args_content: Option<&'a [u8]>,
    ) {
        let current_case_idx = self
            .open_switches
            .last()
            .and_then(|frame| frame.current_case_idx);
        let Some(current_case_idx) = current_case_idx else {
            self.create_standalone_directive(directive_name, start_pos, token_count, args_content);
            return;
        };
        let break_node = Self::create_directive_node(
            NodeKind::Directive,
            current_case_idx,
            start_pos,
            token_count,
            directive_name,
            args_content,
            Some(StructureRole::Intermediate),
        );
        self.add_child(break_node);
        self.pos += token_count;
    }

    /// `closeSwitch`.
    fn close_switch(
        &mut self,
        directive_name: &[u8],
        start_pos: usize,
        token_count: usize,
        args_content: Option<&'a [u8]>,
    ) -> Result<(), ParseError> {
        let Some(frame) = self.open_switches.last() else {
            self.create_standalone_directive(directive_name, start_pos, token_count, args_content);
            return Ok(());
        };
        let element_stack_base = frame.element_stack_base;
        self.close_unmatched_structures_at_depth(element_stack_base + 2, start_pos)?;
        let Some(frame) = self.open_switches.pop() else {
            return Ok(());
        };
        let block_idx = frame.block_idx;
        // +2 because both block and @switch directive are on the stack.
        self.pop_elements_to_depth(frame.element_stack_base + 2);
        if let Some(current_case_idx) = frame.current_case_idx {
            self.pop_if_top(current_case_idx);
        }
        // Pop @switch directive from openElements.
        self.pop_if_top(frame.switch_directive_idx);
        let end_switch_node = Self::create_directive_node(
            NodeKind::Directive,
            block_idx,
            start_pos,
            token_count,
            directive_name,
            args_content,
            Some(StructureRole::Closing),
        );
        self.add_child(end_switch_node);
        // Pop block from openElements.
        self.pop_if_top(block_idx);
        let block = &mut self.nodes[block_idx];
        block.token_count = (start_pos + token_count) as i32 - block.token_start;
        self.pos += token_count;
        Ok(())
    }
}
