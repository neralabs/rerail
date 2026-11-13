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

/// The tape image's format and its in-place reader (`View`), for the readers of this crate's images.
pub use rerail_tape_format as tape_format;

/// A `.rr` file that does not parse: the engine's `Error::Syntax`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Syntax(String),
}

/// A `.rr` file's tree, as its tape holds it (decision 0061): its nodes in preorder, comments
/// as extras, and an ERROR leaf for each statement that does not parse. Never fails: a file
/// the engine refuses to run still has a tape.
///
/// Written by the lean parser (`lean::tape`, docs/parser-generators/30) as the engine's image, its
/// bytes (decision 0140: `rerail_tape_format::Writer`), each node's columns at their place as the
/// parse goes, with no statements built and no conversion after it. It is `engine_tape`'s tree
/// through the writer, byte for byte (`lean_tape_is_the_engines`, every `.rr` file of the
/// repository and seven cuts of each).
pub fn tape(src: &str) -> Vec<u8> {
    lean::tape(src)
}

/// The tree as the statement parser makes it (`syntax::Parser::taped`, flattened): the lean
/// parser's oracle.
pub fn engine_tape(src: &str) -> Vec<tape::Node> {
    let mut parser = syntax::Parser::taped(src);
    // A taped parse reads on past an error; what it could not read is in the tree.
    let _ = parser.statements();
    parser.into_tape().0
}

// cratify: mounts (tools/cratify/mounts.py; generated, do not edit) begin
extern crate self as rerail_rr_syntax;
#[path = "../../../src/code_rule_dsl"]
pub mod code_rule_dsl {
    #[path = "logic"]
    pub mod logic {
        #[path = "front"]
        pub mod front {
            pub mod ast;
            pub mod name;
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
            #[allow(unused_imports)]
            use std::fmt;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::front::name::Name;
            #[allow(unused_imports)]
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::front::ast::{AXIS, Again, AggOp, Atom, DESCENDANTS, EVERY_KIND, GLOBAL, LIBRARY, LatticeDecl, Literal, NEAREST, Native, OPTION_GLOB, Program, QUALIFIERS, RESERVED, Rule, SCOPE_NOT, SpanIn, Term, Value, axis, cmp_binds, globals, is_axis, is_transduced, join};
        }
    }
    #[allow(unused_imports)]
    use napi::Result;
    #[allow(unused_imports)]
    use napi_derive::napi;
    #[allow(unused_imports)]
    use serde_json::Value;
    #[allow(unused_imports)]
    use std::sync::Arc;
}
// cratify: mounts end
