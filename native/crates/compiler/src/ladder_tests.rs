//! JSON's grammars in this crate (`ladder`): each parses as the reference.

use crate::ladder::*;

/// Each grammar: the reference's nodes equal the interpreters' and the compiled path's on every
/// document; tier 1 compiled equals its interpreter; each grammar parses some documents and sends
/// some to tree-sitter. Printed per grammar: the grammar's sentences and the mutated copies apart,
/// and why the reference refused each sentence it refused.
#[test]
fn every_grammar_parses_as_the_reference() {
    let differ = check_ladder(&grammars(), b" \nav9,eE-{}[]\":/*\\tu", &[], &[], &|_, _, _, _| None);
    assert!(differ.is_empty(), "{} grammars differ:\n{}", differ.len(), differ.join("\n"));
}

