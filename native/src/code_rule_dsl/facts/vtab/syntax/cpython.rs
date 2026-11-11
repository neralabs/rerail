//! CPython's own facts (`code_rule_dsl::cpython`, native: no Python runs) as base relations, made by a batch only for a
//! program that reads them (`CPython::new`'s `reads`), over each Python document's text:
//!
//! ```text
//! cpython_syntax_error(d, l, msg)    `ast.parse` (CPython 3.13) rejects document d: its SyntaxError's lineno and msg
//! literal_json(n, v)                 W's pythonAst `literal_json` of expression node n: `json.dumps(ast.literal_eval(
//!                                    node), sort_keys=True, separators=(",", ":"))` of the `ast` node at n's span
//!                                    whose class n's kind is (W's `KINDS`); no row where either raises
//! cpython_unparse(n, s)              core.rr's @py_unparse(n): `ast.unparse` of the `ast` expression (keyword, arg,
//!                                    alias) at n's span whose class n's kind is
//! cpython_ast_dump(n, s)             core.rr's @ast_dump(n): `ast.dump(node, include_attributes=False)` of the
//!                                    `ast` node at n's span whose class n's kind is
//! dump_sha256(n, h)                  W's `dump_sha256`: sha256 of that dump (lowercase hex)
//! body_sha256(f, h)                  W's `body_sha256` of def f: of `ast.dump(ast.Module(body=f.body, type_ignores=[]))`
//! signature_sha256(f, h)             W's `signature_sha256` of def f: of `dump(args)|dump(returns) or "None"|
//!                                    dump(ast.Tuple(elts=type_params))`
//! ```
//!
//! `cpython_syntax_error` is core.rr's `syntax_error(d, l, msg)` of a Python document under the name the converter
//! gives it: both read the tape's one cached verdict (`Facts::python_syntax`, `python_syntax::error`); the tape's
//! `syntax_error(d)` (an ERROR or MISSING node) is apart. Where `ast.parse` raises no `SyntaxError` but another
//! exception (a format spec's `UnicodeDecodeError`), a batch reading any of these relations for that document fails,
//! as W's worker, which catches only `SyntaxError`, fails its batch (`python_syntax::failure`).
//! `@py_unparse(n)` and `@ast_dump(n)` lower to a variable `cpython_unparse` and `cpython_ast_dump` bind
//! (`logic::plan::corpus`'s `cpython_functors`). A node is found as W's worker found it (`cpython::printed::of_kind`;
//! a def, a class or an assignment that tree-sitter ends later than CPython by its start). Rows hold string indexes,
//! as `Batch::build`'s do.

use super::{Dialect, Projected};
use crate::code_rule_dsl::cpython::printed::{Printed, Want, by_start, of_kind, printed};
use std::collections::{BTreeSet, HashMap};

pub(crate) use crate::code_rule_dsl::facts::core::cpython_relations::{
    AST_DUMP, BODY_SHA256, DUMP_SHA256, LITERAL_JSON, SIGNATURE_SHA256, SYNTAX_ERROR, UNPARSE, columns,
};

#[derive(Default)]
pub(super) struct CPython {
    syntax_error: bool,
    literal_json: bool,
    want: Want,
    syntax_errors: Vec<[i64; 3]>,
    literals: Vec<[i64; 2]>,
    /// Rows of `UNPARSE`, `AST_DUMP`, `DUMP_SHA256`, `BODY_SHA256`, `SIGNATURE_SHA256`.
    printed: [Vec<[i64; 2]>; 5],
}

/// The printed relations, in `CPython::printed`'s order.
const PRINTED: [&str; 5] = [UNPARSE, AST_DUMP, DUMP_SHA256, BODY_SHA256, SIGNATURE_SHA256];

impl CPython {
    pub(super) fn start(reads: &BTreeSet<String>) -> Self {
        let want = Want {
            unparse: reads.contains(UNPARSE),
            dump: reads.contains(AST_DUMP),
            dump_sha256: reads.contains(DUMP_SHA256),
            body_sha256: reads.contains(BODY_SHA256),
            signature_sha256: reads.contains(SIGNATURE_SHA256),
        };
        Self { syntax_error: reads.contains(SYNTAX_ERROR), literal_json: reads.contains(LITERAL_JSON), want, ..Self::default() }
    }

    /// Document `p`'s rows (`path` its path, `dialect` its grammar's kinds; `python`: it is Python's). Where
    /// `ast.parse` raises no `SyntaxError` but another exception, the batch fails, as W's worker does for any field
    /// but a text one.
    pub(super) fn document(
        &mut self,
        p: &Projected,
        path: &str,
        dialect: Option<&Dialect>,
        python: bool,
        string: &mut dyn FnMut(&str) -> usize,
    ) -> crate::content_store::StoreResult<()> {
        let (true, Some(dialect)) = (python, dialect) else { return Ok(()) };
        if !(self.syntax_error || self.literal_json || self.want.any()) {
            return Ok(());
        }
        let source = p.facts.source.as_str();
        let verdict = p.facts.python_syntax.get_or_init(|| super::python_syntax::error(source));
        let error = verdict.as_ref().map_err(|raised| super::python_syntax::failure(path, raised))?;
        if self.syntax_error
            && let Some((line, msg)) = error
        {
            self.syntax_errors.push([i64::from(p.key), i64::from(*line), string(msg) as i64]);
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
            return Ok(());
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
            let fields = [&node.unparse, &node.dump, &node.dump_sha256, &node.body_sha256, &node.signature_sha256];
            for (rows, field) in self.printed.iter_mut().zip(fields) {
                if let Some(text) = field {
                    rows.push([n, string(text) as i64]);
                }
            }
        }
        Ok(())
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
        let wanted = [self.want.unparse, self.want.dump, self.want.dump_sha256, self.want.body_sha256, self.want.signature_sha256];
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


/// CPython's syntax verdicts, `ast.unparse` and `ast.dump` as a batch's producer (`syntax::Producer`).
pub(crate) struct Producer;

impl super::Producer for Producer {
    fn dialect(&self) -> crate::code_rule_dsl::language::GrammarDialect {
        crate::code_rule_dsl::language::GrammarDialect::Python
    }
    fn start(&self, reads: &BTreeSet<String>, _: &[String]) -> Box<dyn super::ProducerRun> {
        Box::new(CPython::start(reads))
    }
}

impl super::ProducerRun for CPython {
    fn document(&mut self, p: &super::Projected, d: &super::Document, dialect: Option<&super::Dialect>, own: bool, string: &mut dyn FnMut(&str) -> usize) -> crate::content_store::StoreResult<()> {
        CPython::document(self, p, &d.path, dialect, own, string)
    }
    fn relations(self: Box<Self>, atom: &dyn Fn(i64) -> i64) -> Vec<(&'static str, super::Relation)> {
        CPython::relations(*self, atom)
    }
}
