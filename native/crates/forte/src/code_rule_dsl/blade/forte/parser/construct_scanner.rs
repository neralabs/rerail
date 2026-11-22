//! Forte's `Parser\ConstructScanner`: echoes and PHP spans inside names and attributes, from
//! their start token through their end token. Its scans spend a step per token.

use super::super::node::NodeKind;
use super::super::token::{Token, TokenType};
use super::super::{Budget, Exhausted};

/// `getEndType` (`CONSTRUCT_PAIRS`).
pub(super) fn get_end_type(start_type: TokenType) -> Option<TokenType> {
    match start_type {
        TokenType::EchoStart => Some(TokenType::EchoEnd),
        TokenType::RawEchoStart => Some(TokenType::RawEchoEnd),
        TokenType::TripleEchoStart => Some(TokenType::TripleEchoEnd),
        TokenType::PhpTagStart => Some(TokenType::PhpTagEnd),
        TokenType::PhpBlockStart => Some(TokenType::PhpBlockEnd),
        _ => None,
    }
}

/// `isConstructStart`.
pub(super) fn is_construct_start(token_type: TokenType) -> bool {
    get_end_type(token_type).is_some()
}

/// `isEchoStart`.
pub(super) fn is_echo_start(token_type: TokenType) -> bool {
    matches!(
        token_type,
        TokenType::EchoStart | TokenType::RawEchoStart | TokenType::TripleEchoStart
    )
}

/// `isPhpStart`.
pub(super) fn is_php_start(token_type: TokenType) -> bool {
    matches!(
        token_type,
        TokenType::PhpTagStart | TokenType::PhpBlockStart
    )
}

/// `getNodeKind` (`CONSTRUCT_KINDS`).
pub(super) fn get_node_kind(start_type: TokenType) -> Option<NodeKind> {
    match start_type {
        TokenType::EchoStart => Some(NodeKind::Echo),
        TokenType::RawEchoStart => Some(NodeKind::RawEcho),
        TokenType::TripleEchoStart => Some(NodeKind::TripleEcho),
        TokenType::PhpTagStart => Some(NodeKind::PhpTag),
        TokenType::PhpBlockStart => Some(NodeKind::PhpBlock),
        _ => None,
    }
}

/// `scanConstruct`: the position after the construct at `pos` (its end token included when
/// found before `end`) and its token count.
pub(super) fn scan_construct(
    tokens: &[Token],
    mut pos: usize,
    end: usize,
    budget: &mut Budget,
) -> Result<(usize, usize), Exhausted> {
    if pos >= end {
        return Ok((pos, 0));
    }
    let kind = |idx: usize| tokens.get(idx).map(|token| token.kind);
    let Some(end_type) = kind(pos).and_then(get_end_type) else {
        // Not starting anything, just return a single token to advance.
        return Ok((pos + 1, 1));
    };
    let mut count = 1;
    pos += 1;
    while pos < end && kind(pos) != Some(end_type) {
        budget.spend(1)?;
        count += 1;
        pos += 1;
    }
    if pos < end && kind(pos) == Some(end_type) {
        count += 1;
        pos += 1;
    }
    Ok((pos, count))
}

/// `findConstructEnd`.
pub(super) fn find_construct_end(
    tokens: &[Token],
    pos: usize,
    end: usize,
    budget: &mut Budget,
) -> Result<usize, Exhausted> {
    Ok(scan_construct(tokens, pos, end, budget)?.0)
}

/// `advancePast`: past the construct or the single token at `pos`.
pub(super) fn advance_past(
    tokens: &[Token],
    pos: usize,
    end: usize,
    budget: &mut Budget,
) -> Result<usize, Exhausted> {
    if pos >= end {
        return Ok(pos);
    }
    match tokens.get(pos) {
        Some(token) if is_construct_start(token.kind) => {
            find_construct_end(tokens, pos, end, budget)
        }
        _ => Ok(pos + 1),
    }
}

/// `countConstructTokens`.
pub(super) fn count_construct_tokens(
    tokens: &[Token],
    pos: usize,
    end: usize,
    budget: &mut Budget,
) -> Result<usize, Exhausted> {
    Ok(scan_construct(tokens, pos, end, budget)?.1)
}
