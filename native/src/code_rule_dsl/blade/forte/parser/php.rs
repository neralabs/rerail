//! Forte's `Parser\Concerns\ProcessesPhp`.

use super::super::ParseError;
use super::super::node::NodeKind;
use super::super::token::TokenType;
use super::TreeBuilder;

impl TreeBuilder<'_, '_> {
    /// `createPhpBlock`.
    pub(super) fn create_php_block(&mut self, start_pos: usize) -> Result<(), ParseError> {
        self.create_block_node(start_pos, TokenType::PhpBlockEnd, NodeKind::PhpBlock)
    }

    /// `createPhpTag`.
    pub(super) fn create_php_tag(&mut self, start_pos: usize) -> Result<(), ParseError> {
        self.create_block_node(start_pos, TokenType::PhpTagEnd, NodeKind::PhpTag)
    }

    /// `processOrphanPhpBlockEnd`: an `@endphp` with no `@php` is a directive named `endphp`.
    pub(super) fn process_orphan_php_block_end(&mut self) {
        let start_pos = self.pos;
        self.pos += 1;
        let node = Self::create_directive_node(
            NodeKind::Directive,
            0,
            start_pos,
            1,
            b"endphp",
            None,
            None,
        );
        self.add_child(node);
    }
}
