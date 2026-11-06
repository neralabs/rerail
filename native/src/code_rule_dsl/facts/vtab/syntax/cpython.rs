//! CPython's own facts (`code_rule_dsl::cpython`, native: no Python runs) as base relations, made by a batch only for a
//! program that reads them (`CPython::new`'s `reads`), over each Python document's text:
//!
//! ```text
//! cpython_syntax_error(d, l, msg)    `ast.parse` (CPython 3.13) rejects document d: its SyntaxError's lineno and msg
//! literal_json(n, v)                 W's pythonAst `literal_json` of expression node n: `json.dumps(ast.literal_eval(
//!                                    node), sort_keys=True, separators=(",", ":"))` of the `ast` node at n's span
//!                                    whose class n's kind is (W's `KINDS`); no row where either raises
//! ```
//!
//! core.rr's `syntax_error(d, l, msg)` lowers to `cpython_syntax_error` where a program does not declare a
//! `syntax_error` of its own (`rr::lower::core`); the tape's `syntax_error(d)` (an ERROR or MISSING node) is apart.
//! Rows hold string indexes, as `Batch::build`'s do.

use super::{Dialect, Projected};
use std::collections::{BTreeSet, HashMap};

/// The relation the corpus binds for CPython's verdict (core.rr's `syntax_error(d, l, msg)`).
pub(crate) const SYNTAX_ERROR: &str = "cpython_syntax_error";
/// W's `literal_json` (`cpython::literal`).
pub(crate) const LITERAL_JSON: &str = "literal_json";

#[derive(Default)]
pub(super) struct CPython {
    syntax_error: bool,
    literal_json: bool,
    syntax_errors: Vec<[i64; 3]>,
    literals: Vec<[i64; 2]>,
}

impl CPython {
    pub(super) fn new(reads: &BTreeSet<String>) -> Self {
        Self { syntax_error: reads.contains(SYNTAX_ERROR), literal_json: reads.contains(LITERAL_JSON), ..Self::default() }
    }

    /// Document `p`'s rows (`dialect` its grammar's kinds; `python`: it is Python's).
    pub(super) fn document(&mut self, p: &Projected, dialect: Option<&Dialect>, python: bool, string: &mut dyn FnMut(&str) -> usize) {
        let (true, Some(dialect)) = (python, dialect) else { return };
        let source = p.facts.source.as_str();
        if self.syntax_error
            && let Some(e) = crate::code_rule_dsl::cpython::syntax::syntax_error(source)
        {
            self.syntax_errors.push([i64::from(p.key), e.line as i64, string(&e.msg) as i64]);
        }
        if self.literal_json {
            let literals = crate::code_rule_dsl::cpython::literal::literals(source);
            if literals.is_empty() {
                return;
            }
            // The document's nodes by span.
            let f = &*p.facts;
            let mut by_span: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
            for (local, &id) in p.core.iter().enumerate() {
                by_span.entry((f.start[id as usize], f.end[id as usize])).or_default().push(local as u32);
            }
            for l in &literals {
                let Some(nodes) = by_span.get(&(l.start, l.end)) else { continue };
                let mut v = None;
                for &local in nodes {
                    if l.class.of_kind(dialect.name(f, p.core[local as usize])) {
                        let v = *v.get_or_insert_with(|| string(&l.json) as i64);
                        self.literals.push([p.global(local), v]);
                    }
                }
            }
        }
    }

    /// The relations read, their strings as atoms (`atom` of a string index).
    pub(super) fn relations(self, atom: &dyn Fn(i64) -> i64) -> Vec<(&'static str, super::Relation)> {
        let mut out = Vec::new();
        if self.syntax_error {
            let mut r = super::Relation::new(&["d", "l", "msg"]);
            self.syntax_errors.iter().for_each(|&[d, l, m]| r.push(&[d, l, atom(m)]));
            out.push((SYNTAX_ERROR, r));
        }
        if self.literal_json {
            let mut r = super::Relation::new(&["n", "v"]);
            self.literals.iter().for_each(|&[n, v]| r.push(&[n, atom(v)]));
            out.push((LITERAL_JSON, r));
        }
        out
    }
}
