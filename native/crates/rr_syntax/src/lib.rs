//! The rule engine's `.rerail` parser, in a crate of its own so that the parsers' tests can check
//! against it without the engine: a `.rerail` file's statements (`syntax`) and its tree, numbered
//! by the tape's kinds table (`tape`, decision 0061). The engine (`code_rule_dsl::logic::rr`)
//! re-exports both under the same names.

pub mod lean;
#[doc(hidden)]
pub mod lean_scalar;
mod micro;
pub mod syntax;
pub mod tape;

/// The tape's format and its in-place readers (`f16`, `tape::Tape`), for the readers of this crate's tapes.
pub use rerail_tape_format as tape_format;

/// A `.rerail` file that does not parse: the engine's `Error::Syntax`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Syntax(String),
}

/// A `.rerail` file's tree, as its tape holds it (decision 0061): its nodes in preorder, comments
/// as extras, and an ERROR leaf for each statement that does not parse: a file the engine refuses
/// to run still has a tape.
///
/// Written by the lean parser (`lean::tape`, docs/parser-generators/30) in format 16 (decisions 0140, 0434, 0626:
/// `rerail_tape_format::Writer`), each statement's nodes passed to format 16's ops as the parse commits them, with no
/// statements built and no conversion after it. It is `engine_tape`'s tree written by format 16's reference ops,
/// byte for byte (`lean_tape_is_the_engines`, every `.rerail` file of the repository and seven cuts of each). `Err`
/// only for a code the language's table lacks.
pub fn tape(src: &str) -> Result<rerail_tape_format::f16::Tape16, String> {
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

// The browser build has no JIT: wasm/cranelift-jit stands in under the same name (wasm/README.md).
#[cfg(target_family = "wasm")]
extern crate cranelift_jit_wasm as cranelift_jit;
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
            pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::front::ast::{AXIS, AggOp, DESCENDANTS, EVERY_KIND, GLOBAL, LIBRARY, LatticeDecl, NEAREST, Native, OPTION_GLOB, QUALIFIERS, RESERVED, SCOPE_NOT, SpanIn, Value, axis, globals, is_axis, is_transduced};
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
