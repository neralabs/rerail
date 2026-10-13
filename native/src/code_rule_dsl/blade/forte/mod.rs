//! A Rust port of Forte 1.1.0 (github.com/fortephp/forte, MIT, by John Koster), the
//! fault-tolerant Blade parser: its lexer (`Lexer\*`), its tree builder (`Parser\*`) and the
//! directive registry that decides structure (`Parser\Directives\*`), ported method by method.
//! `docs/blade-parser.md` says what is ported and why; `THIRD_PARTY_NOTICES` holds
//! Forte's license, which covers this module's port of its code.
//!
//! `parse` does what `Document::parse` does: lex, train the registry when it accepts every
//! directive, build the flat tree. Its output is Forte's flat arrays, which the oracle
//! (`fixtures/blade/forte`) records and the tests compare byte for byte; `tree` reads them as
//! Rerail's tree.
//!
//! Bounds: Forte's tree builder throws past its depth limits (512 elements, 256 directives, 256
//! conditions), and `parse` returns that error with Forte's message. What Forte leaves unbounded
//! is bounded here: every loop that may revisit input spends steps from a budget proportional to
//! the template's length (`Budget`), and nothing recurses deeper than a depth limit, so a
//! template costs bounded time and stack whatever its bytes.

pub(crate) mod ast;
pub(crate) mod directives;
pub(crate) mod lexer;
pub(crate) mod node;
pub(crate) mod parser;
pub(crate) mod scan;
pub(crate) mod token;
pub(crate) mod tree;

#[cfg(test)]
mod tests;

use directives::Directives;
use node::FlatNode;
use token::{LexerError, Token, TokenType};

/// The tree builder's stack limits (`TreeBuilder::DEFAULT_MAX_*_DEPTH`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DepthLimits {
    pub(crate) elements: usize,
    pub(crate) directives: usize,
    pub(crate) conditions: usize,
}

impl Default for DepthLimits {
    fn default() -> Self {
        Self {
            elements: 512,
            directives: 256,
            conditions: 256,
        }
    }
}

/// Steps a parse may spend: a bound on the work of loops that may revisit input.
#[derive(Debug)]
pub(crate) struct Budget {
    remaining: u64,
}

/// The budget ran out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Exhausted;

impl Budget {
    /// Steps per template byte and token, and the floor, of `Budget::for_template`.
    pub(crate) const PER_UNIT: u64 = 256;
    pub(crate) const FLOOR: u64 = 1 << 20;

    pub(crate) fn new(steps: u64) -> Self {
        Self { remaining: steps }
    }

    /// The budget for a template of `bytes` bytes.
    pub(crate) fn for_template(bytes: usize) -> Self {
        Self::new(
            (bytes as u64)
                .saturating_mul(Self::PER_UNIT)
                .saturating_add(Self::FLOOR),
        )
    }

    /// Spends `steps`; `Err` once the budget is spent.
    #[inline]
    pub(crate) fn spend(&mut self, steps: u64) -> Result<(), Exhausted> {
        match self.remaining.checked_sub(steps) {
            Some(remaining) => {
                self.remaining = remaining;
                Ok(())
            }
            None => {
                self.remaining = 0;
                Err(Exhausted)
            }
        }
    }

    /// Steps left.
    #[cfg(test)]
    pub(crate) fn remaining(&self) -> u64 {
        self.remaining
    }
}

/// Why a template has no tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ParseError {
    /// Forte throws: a depth limit was reached (its `RuntimeException` message).
    Depth(String),
    /// The step budget ran out (Rerail's bound; Forte has none).
    Budget,
}

impl From<Exhausted> for ParseError {
    fn from(_: Exhausted) -> Self {
        ParseError::Budget
    }
}

/// What Forte parses a template to (`Document`'s flat arrays).
#[derive(Clone, Debug)]
pub(crate) struct Parsed {
    pub(crate) tokens: Vec<Token>,
    pub(crate) nodes: Vec<FlatNode>,
    pub(crate) errors: Vec<LexerError>,
}

/// Parses a template as `Document::parse($source, ParserOptions::make()->directives(...))` does.
/// The registry is trained (`trainCompact`) when it accepts every directive.
pub(crate) fn parse(
    source: &[u8],
    mut directives: Directives,
    limits: DepthLimits,
    budget: &mut Budget,
) -> Result<Parsed, ParseError> {
    let lexed = lexer::tokenize(source, &directives, budget)?;
    if directives.accepts_all_directives() {
        directives.train(
            lexed
                .tokens
                .iter()
                .filter(|token| token.kind == TokenType::Directive)
                .map(|token| source[token.start as usize..token.end as usize].into()),
        );
    }
    let nodes = parser::build(&lexed.tokens, source, &directives, limits, budget)?;
    Ok(Parsed {
        tokens: lexed.tokens,
        nodes,
        errors: lexed.errors,
    })
}
