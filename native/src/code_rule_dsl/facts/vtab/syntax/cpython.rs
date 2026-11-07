//! CPython's own facts (`code_rule_dsl::cpython`, native: no Python runs) as base relations, made by a batch only for a
//! program that reads them (`CPython::new`'s `reads`), over each Python document's text:
//!
//! ```text
//! cpython_syntax_error(d, l, msg)    `ast.parse` (CPython 3.13) rejects document d: its SyntaxError's lineno and msg
//! literal_json(n, v)                 W's pythonAst `literal_json` of expression node n: `json.dumps(ast.literal_eval(
//!                                    node), sort_keys=True, separators=(",", ":"))` of the `ast` node at n's span
//!                                    whose class n's kind is (W's `KINDS`); no row where either raises
//! cpython_unparse(n, s)              core.rr's @unparse(n): `ast.unparse` of the `ast` expression (keyword, arg,
//!                                    alias) at n's span whose class n's kind is
//! ```
//!
//! core.rr's `syntax_error(d, l, msg)` lowers to `cpython_syntax_error` where a program does not declare a
//! `syntax_error` of its own (`rr::lower::core`); the tape's `syntax_error(d)` (an ERROR or MISSING node) is apart.
//! `@unparse(n)` lowers to a variable `cpython_unparse` binds (`logic::plan::corpus`'s `cpython_functors`). A node is found as W's worker found it (`cpython::printed::of_kind`;
//! a def, a class or an assignment that tree-sitter ends later than CPython by its start). Rows hold string indexes,
//! as `Batch::build`'s do.

use super::{Dialect, Projected};
use crate::code_rule_dsl::cpython::printed::{Printed, Want, by_start, of_kind, printed};
use std::collections::{BTreeSet, HashMap};

/// The relation the corpus binds for CPython's verdict (core.rr's `syntax_error(d, l, msg)`).
pub(crate) const SYNTAX_ERROR: &str = "cpython_syntax_error";
/// W's `literal_json` (`cpython::literal`).
pub(crate) const LITERAL_JSON: &str = "literal_json";
/// core.rr's `@unparse(n)` (`cpython::unparse`).
pub(crate) const UNPARSE: &str = "cpython_unparse";

#[derive(Default)]
pub(super) struct CPython {
    syntax_error: bool,
    literal_json: bool,
    want: Want,
    syntax_errors: Vec<[i64; 3]>,
    literals: Vec<[i64; 2]>,
    /// Rows of `UNPARSE`.
    printed: [Vec<[i64; 2]>; 1],
}

/// The printed relations, in `CPython::printed`'s order.
const PRINTED: [&str; 1] = [UNPARSE];

impl CPython {
    pub(super) fn new(reads: &BTreeSet<String>) -> Self {
        let want = Want { unparse: reads.contains(UNPARSE) };
        Self { syntax_error: reads.contains(SYNTAX_ERROR), literal_json: reads.contains(LITERAL_JSON), want, ..Self::default() }
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
            if !literals.is_empty() {
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
        let found = printed(source, self.want);
        if found.is_empty() {
            return;
        }
        // CPython's located nodes by span and by start, in preorder.
        let mut by_span: HashMap<(u32, u32), Vec<&Printed>> = HashMap::new();
        let mut starting: HashMap<u32, Vec<&Printed>> = HashMap::new();
        for n in &found {
            by_span.entry((n.start, n.end)).or_default().push(n);
            starting.entry(n.start).or_default().push(n);
        }
        let f = &*p.facts;
        for (local, &id) in p.core.iter().enumerate() {
            let (start, end) = (f.start[id as usize], f.end[id as usize]);
            let exact = by_span.get(&(start, end));
            if exact.is_none() && !starting.contains_key(&start) {
                continue;
            }
            let kind = dialect.name(f, id);
            let mut node = exact.and_then(|ns| ns.iter().find(|n| of_kind(n.class, kind)).copied());
            if node.is_none() && by_start(kind) {
                let mut within = starting.get(&start).into_iter().flatten().filter(|n| n.end <= end && of_kind(n.class, kind));
                node = match (within.next(), within.next()) {
                    (Some(one), None) => Some(*one),
                    _ => None,
                };
            }
            let Some(node) = node else { continue };
            let n = p.global(local as u32);
            let fields = [&node.unparse];
            for (rows, field) in self.printed.iter_mut().zip(fields) {
                if let Some(text) = field {
                    rows.push([n, string(text) as i64]);
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
        let wanted = [self.want.unparse];
        for ((rel, rows), wanted) in PRINTED.into_iter().zip(&self.printed).zip(wanted) {
            if wanted {
                let mut r = super::Relation::new(columns(rel));
                rows.iter().for_each(|&[n, s]| r.push(&[n, atom(s)]));
                out.push((rel, r));
            }
        }
        out
    }
}

/// A printed relation's columns.
pub(crate) fn columns(_rel: &str) -> &'static [&'static str] {
    &["n", "s"]
}
