//! `json-object`: a rule file's JSON objects (decision 0495 §1), each a JSON program of its own: a `.policy { … }`
//! document, a violation's or a fix's `options { … }`, a `{ … }` term.
//!
//! ```text
//! .policy {"id": "p", "revision": 1}             a JSON program: the object, at its bytes
//! a({"siblingLinks": ["x"]}).                    a JSON program: the term's object
//! ```
//!
//! A program's text is its `json_object` leaf, byte for byte, so its nodes stand at the file's bytes, and its host
//! node is that leaf.

use super::host::HostNode;
use super::segments::SegmentMap;
use super::{Embedder, Program};
use crate::code_rule_dsl::language::GrammarDialect;

/// The embedder of `Via::JsonObject`.
pub struct JsonObject;

impl Embedder for JsonObject {
    fn build(&self, root: HostNode<'_>, source: &str, budget: &dyn Fn() -> bool) -> Option<Program> {
        self.build_all(root, source, budget).into_iter().next()
    }

    fn build_all(&self, root: HostNode<'_>, source: &str, budget: &dyn Fn() -> bool) -> Vec<Program> {
        let mut programs = Vec::new();
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            if node.kind() != "json_object" {
                let children = node.named_children().collect::<Vec<_>>();
                stack.extend(children.into_iter().rev());
                continue;
            }
            let range = node.byte_range();
            let text = source[range.clone()].to_owned();
            let mut segments = SegmentMap::new();
            segments.push(0, text.len(), range, true, false);
            segments.index();
            let Some(parse) = super::Parse::read_program(GrammarDialect::Json, &text, &segments, budget) else { continue };
            programs.push(Program { dialect: GrammarDialect::Json, parse, text, segments, host_errors: Vec::new(), at: None });
        }
        programs
    }

    fn per_place(&self) -> bool {
        true
    }
}
