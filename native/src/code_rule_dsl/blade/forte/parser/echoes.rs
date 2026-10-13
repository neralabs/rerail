//! Forte's `Parser\Concerns\ProcessesEchoes`.

use super::super::ParseError;
use super::super::node::NodeKind;
use super::super::token::TokenType;
use super::TreeBuilder;

impl TreeBuilder<'_, '_> {
    /// `processEcho`.
    pub(super) fn process_echo(&mut self) -> Result<(), ParseError> {
        self.process_echo_span(TokenType::EchoEnd, NodeKind::Echo)
    }

    /// `processRawEcho`.
    pub(super) fn process_raw_echo(&mut self) -> Result<(), ParseError> {
        self.process_echo_span(TokenType::RawEchoEnd, NodeKind::RawEcho)
    }

    /// `processTripleEcho`.
    pub(super) fn process_triple_echo(&mut self) -> Result<(), ParseError> {
        self.process_echo_span(TokenType::TripleEchoEnd, NodeKind::TripleEcho)
    }

    /// `processEchoSpan`: through the end token; text up to a nested echo or Blade comment
    /// start.
    fn process_echo_span(
        &mut self,
        end_type: TokenType,
        node_kind: NodeKind,
    ) -> Result<(), ParseError> {
        let start_pos = self.pos;
        self.pos += 1;
        let limit = self.tokens.len();
        while self.pos < limit {
            self.spend()?;
            let token_type = self.tokens[self.pos].kind;
            if token_type == end_type {
                self.pos += 1;
                break;
            }
            if matches!(
                token_type,
                TokenType::EchoStart
                    | TokenType::RawEchoStart
                    | TokenType::TripleEchoStart
                    | TokenType::BladeCommentStart
            ) {
                self.emit_text_span(start_pos, self.pos - start_pos);
                return Ok(());
            }
            self.pos += 1;
        }
        self.emit_node(node_kind, start_pos, self.pos - start_pos);
        Ok(())
    }

    /// `emitTextSpan`.
    fn emit_text_span(&mut self, start: usize, length: usize) {
        if length == 0 {
            return;
        }
        self.add_child(Self::create_node(NodeKind::Text, 0, start, length));
    }

    /// `emitNode`.
    fn emit_node(&mut self, kind: NodeKind, start: usize, length: usize) {
        self.add_child(Self::create_node(kind, 0, start, length));
    }
}
