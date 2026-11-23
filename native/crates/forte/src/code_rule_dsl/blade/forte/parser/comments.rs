//! Forte's `Parser\Concerns\ProcessesComments`.

use super::super::ParseError;
use super::super::node::NodeKind;
use super::super::token::TokenType;
use super::TreeBuilder;

/// A conditional comment whose contents are being processed: what `createConditionalComment`
/// holds across its loop.
pub(super) struct ConditionalCommentFrame {
    /// The end token's position (where the contents stop).
    pub(super) end_pos: usize,
    /// Past the end token.
    full_end_pos: usize,
    element_stack_depth: usize,
}

impl TreeBuilder<'_, '_> {
    /// `createHtmlComment`.
    pub(super) fn create_html_comment(&mut self, start_pos: usize) -> Result<(), ParseError> {
        self.create_block_node(start_pos, TokenType::CommentEnd, NodeKind::Comment)
    }

    /// `createBladeComment`.
    pub(super) fn create_blade_comment(&mut self, start_pos: usize) -> Result<(), ParseError> {
        self.create_block_node(
            start_pos,
            TokenType::BladeCommentEnd,
            NodeKind::BladeComment,
        )
    }

    /// `createBogusComment`.
    pub(super) fn create_bogus_comment(&mut self, start_pos: usize) {
        let mut node = Self::create_node(NodeKind::BogusComment, 0, start_pos, 1);
        node.data = 1;
        self.add_child(node);
        self.pos += 1;
    }

    /// `createConditionalComment`, up to its loop: the node spans through its end token and
    /// stays open; `build_tree` processes its contents up to the end token as its children,
    /// then calls `finish_conditional_comment`.
    pub(super) fn create_conditional_comment(
        &mut self,
        start_pos: usize,
    ) -> Result<(), ParseError> {
        let mut end_pos = start_pos + 1;
        let mut has_closing = false;
        while end_pos < self.tokens.len() {
            self.spend()?;
            if self.tokens[end_pos].kind == TokenType::ConditionalCommentEnd {
                has_closing = true;
                break;
            }
            end_pos += 1;
        }
        let full_end_pos = if has_closing { end_pos + 1 } else { end_pos };
        let mut node = Self::create_node(
            NodeKind::ConditionalComment,
            0,
            start_pos,
            full_end_pos - start_pos,
        );
        node.data = has_closing as i32;
        let node_idx = self.add_child(node);
        let element_stack_depth = self.open_elements.len();
        self.check_element_depth()?;
        self.open_elements.push(node_idx);
        self.pos = start_pos + 1;
        self.conditional_comments.push(ConditionalCommentFrame {
            end_pos,
            full_end_pos,
            element_stack_depth,
        });
        Ok(())
    }

    /// `createConditionalComment` after its loop: closes what its contents left open, and
    /// itself, and moves past the end token (back, when a child ran past it).
    pub(super) fn finish_conditional_comment(&mut self, frame: ConditionalCommentFrame) {
        while self.open_elements.len() > frame.element_stack_depth + 1 {
            let popped = self.open_elements.pop();
            self.cleanup_tag_name_stack(popped);
        }
        let popped = self.open_elements.pop();
        self.cleanup_tag_name_stack(popped);
        self.pos = frame.full_end_pos;
    }

    /// `createProcessingInstruction`.
    pub(super) fn create_processing_instruction(
        &mut self,
        start_pos: usize,
    ) -> Result<(), ParseError> {
        self.create_block_node(start_pos, TokenType::PiEnd, NodeKind::ProcessingInstruction)
    }
}
