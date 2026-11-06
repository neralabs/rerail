//! W's `parse: {grammar: pythonFString}` (`literal::parse_python_fstring`) as core.rr's relations, made by a batch
//! only for a program that reads them (`FStrings::new`'s `reads`), off the document's tape (`Document::tape`):
//!
//! ```text
//! fstring_piece(n, i, kind, v)        the root children of n's tree, in order from 0: `literal` (v its decoded text)
//!                                     and `interpolation` (v its expression's source); at an interpolation's i,
//!                                     `debug` (v the `x = ` of `{x = }`) and `format_spec` (v the `>10` of
//!                                     `{x:>10}`, without its `:`) where its field has them
//! fstring_template(n, marker, v)      the root's value with interpolation slot text `marker`: the decoded text, each
//!                                     interpolation `marker`, a debug field's `x = ` and a spec's literal text kept
//! ```
//!
//! `n` is a Python `string`, `concatenated_string` or `format_specifier` node (W's hosts of the grammar). There is no
//! row where W gets no tree: no f-string among the parts (plain or concatenated plain strings), bytes, a template
//! string, or a value W cannot decode. W's budget does not apply: every host is parsed. The markers are the
//! constants the program's `fstring_template` atoms hold (`logic::strings::Vocabulary::markers`). Rows hold string
//! indexes, as `Batch::build`'s do.

use super::{Dialect, Document, Projected, Relation};
use crate::code_rule_dsl::facts::core::NONE;
use crate::code_rule_dsl::literal;
use crate::content_store::{StoreError, StoreResult};
use std::collections::BTreeSet;

/// The relations' names.
pub(crate) const RELATIONS: [&str; 2] = ["fstring_piece", "fstring_template"];

/// W's hosts of `pythonFString` (`compile::parse`'s host kinds).
const HOSTS: [&str; 3] = ["string", "concatenated_string", "format_specifier"];

#[derive(Default)]
pub(super) struct FStrings {
    piece: bool,
    /// The markers of the `fstring_template` rows, when it is read.
    markers: Vec<String>,
    pieces: Vec<[i64; 4]>,
    templates: Vec<[i64; 3]>,
}

impl FStrings {
    pub(super) fn new(reads: &BTreeSet<String>, markers: &[String]) -> Self {
        Self {
            piece: reads.contains("fstring_piece"),
            markers: if reads.contains("fstring_template") { markers.to_vec() } else { Vec::new() },
            ..Self::default()
        }
    }

    fn reads(&self) -> bool {
        self.piece || !self.markers.is_empty()
    }

    /// Document `p`'s rows (`d` its document; `python`: it is Python's).
    pub(super) fn document(&mut self, p: &Projected, d: &Document, dialect: Option<&Dialect>, python: bool, string: &mut dyn FnMut(&str) -> usize) -> StoreResult<()> {
        let (true, true, Some(dialect)) = (self.reads(), python, dialect) else { return Ok(()) };
        let f = &*p.facts;
        let Some((tape, _)) = &d.tape else {
            return Err(StoreError::Invalid(format!("{}: fstring_piece and fstring_template need the document's tape", d.path)));
        };
        if tape.transducer_len() != f.len() {
            return Err(StoreError::Invalid(format!("{}: the tape is not the one the facts were read off", d.path)));
        }
        let root = ast_grep_core::AstGrep::doc(tape.clone());
        let markers: Vec<i64> = self.markers.iter().map(|m| string(m) as i64).collect();
        for id in 0..f.len() as u32 {
            let local = p.local[id as usize];
            if local == NONE || !HOSTS.contains(&dialect.name(f, id)) {
                continue;
            }
            let n = p.global(local);
            let node = crate::code_rule_dsl::matcher::node_at(&root, id);
            if self.piece {
                for (i, kind, v) in literal::fstring_pieces(&node).unwrap_or_default() {
                    self.pieces.push([n, i as i64, string(kind.name()) as i64, string(&v) as i64]);
                }
            }
            for (marker, &m) in self.markers.iter().zip(&markers) {
                if let Some(v) = literal::fstring_template(&node, marker) {
                    self.templates.push([n, m, string(&v) as i64]);
                }
            }
        }
        Ok(())
    }

    /// The relations read, their strings as atoms (`atom` of a string index).
    pub(super) fn relations(self, atom: &dyn Fn(i64) -> i64) -> Vec<(&'static str, Relation)> {
        let mut out = Vec::new();
        if self.piece {
            let mut r = Relation::new(&["n", "i", "kind", "v"]);
            self.pieces.iter().for_each(|&[n, i, k, v]| r.push(&[n, i, atom(k), atom(v)]));
            out.push(("fstring_piece", r));
        }
        if !self.markers.is_empty() {
            let mut r = Relation::new(&["n", "marker", "v"]);
            self.templates.iter().for_each(|&[n, m, v]| r.push(&[n, atom(m), atom(v)]));
            out.push(("fstring_template", r));
        }
        out
    }
}
