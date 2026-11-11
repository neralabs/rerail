//! The rule engine's `.rr` parser, in a crate of its own so that the parsers' tests can check
//! against it without the engine: a `.rr` file's statements (`syntax`) and its tree, numbered
//! by the tape's kinds table (`tape`, decision 0061). The engine (`code_rule_dsl::logic::rr`)
//! re-exports both under the same names.

pub mod lean;
#[doc(hidden)]
pub mod lean_scalar;
mod micro;
pub mod syntax;
pub mod tape;

/// A `.rr` file that does not parse: the engine's `Error::Syntax`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Syntax(String),
}

/// A `.rr` file's tree, as its tape holds it (decision 0061): its nodes in preorder, comments
/// as extras, and an ERROR leaf for each statement that does not parse. Never fails: a file
/// the engine refuses to run still has a tape.
pub fn tape(src: &str) -> Vec<tape::Node> {
    let mut parser = syntax::Parser::taped(src);
    // A taped parse reads on past an error; what it could not read is in the tree.
    let _ = parser.statements();
    parser.into_tape().0
}
