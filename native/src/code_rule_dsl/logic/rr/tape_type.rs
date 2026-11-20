//! The `tape` column type (decision 0103): a tape is the content key of a parsed text (`@parse`, `parse_of`,
//! `tape_of`). It is stored as tapes always were (a symbol: an atom over atoms), so the engine sees `sym`; the type
//! is checked here, before lowering: a variable that stands in a tape column stands in no symbol, number or node
//! column. A tape is addressed by its content, so unlike a node it can be exported from a phase and kept.
//!
//! ```text
//! .decl ann_tree(s: symbol, v: symbol, t: tape)
//! ann_tree(s, v, t) :- ann_key(s, v), parse_of(s, "annotation", t).     t is a tape on both sides
//! ```

use std::collections::{BTreeMap, BTreeSet};

use super::clause::{At, L, LitAt, Reads, T};
use super::from_tape::{RrFile, S, Stmt};
use super::tape::Field;
use crate::code_rule_dsl::logic::front::Error;

/// The types a tape is not: a variable in one of these and in a tape column is refused.
const OTHER: [&str; 8] = ["symbol", "sym", "number", "int", "unsigned", "float", "node", "span"];

/// The relations of `core.rerail` with a tape column, and their columns' types.
const BUILT_IN: &[(&str, &[&str])] = &[
    ("sub_tape", &["node", "symbol", "tape"]),
    ("tape_of", &["number", "tape"]),
    ("parse_of", &["symbol", "symbol", "tape"]),
    ("unparse_of", &["tape", "symbol"]),
    ("splice_of", &["tape", "node", "tape", "tape"]),
];

/// A relation of `core.rerail`'s tape columns.
pub(super) fn tape_columns(rel: &str) -> Vec<usize> {
    BUILT_IN.iter().filter(|(r, _)| *r == rel).flat_map(|(_, cols)| cols.iter().enumerate().filter(|(_, t)| **t == "tape").map(|(i, _)| i)).collect()
}

/// The tape operations' functors: which arguments are tapes, and whether the result is.
fn functor(name: &str) -> Option<(&'static [usize], bool)> {
    match name {
        "parse" => Some((&[], true)),
        "unparse" => Some((&[0], false)),
        "splice" => Some((&[0, 2], true)),
        _ => None,
    }
}

/// Each variable's types in one clause, as its columns and functors give them; the clause read off file .0.
struct Seen<'f>(&'f RrFile, BTreeMap<String, BTreeSet<String>>);

impl Seen<'_> {
    fn add(&mut self, v: &str, t: &str) {
        self.1.entry(v.to_owned()).or_default().insert(t.to_owned());
    }
    fn args(&mut self, cols: Option<&Vec<String>>, args: &[At]) {
        for (i, a) in args.iter().enumerate() {
            match self.0.tv(a) {
                T::Var(v) => {
                    if let Some(t) = cols.and_then(|c| c.get(i)) {
                        self.add(&v, t);
                    }
                }
                _ => self.term(a),
            }
        }
    }
    fn term(&mut self, t: &At) {
        match self.0.tv(t) {
            T::Call { name, args, functor: true } if let Some((tapes, _)) = functor(&name) => {
                for (i, a) in args.iter().enumerate() {
                    match self.0.tv(a) {
                        T::Var(v) if tapes.contains(&i) => self.add(&v, "tape"),
                        _ => self.term(a),
                    }
                }
            }
            T::Call { args, .. } => args.iter().for_each(|a| self.term(a)),
            T::Bin { l, r, .. } => {
                self.term(&l);
                self.term(&r);
            }
            T::Named(_, t) => self.term(&t),
            _ => {}
        }
    }
}

/// The relations' declared column types, then each clause's variables: an error at the first variable that is a tape
/// and something else.
pub(super) fn check(statements: &[Stmt]) -> Result<(), Error> {
    check_in(statements, &BTreeMap::new())
}

/// `check`, with the column types `given` declares besides the statements' own (a module's includes').
pub(super) fn check_in(statements: &[Stmt], given: &BTreeMap<String, Vec<String>>) -> Result<(), Error> {
    let mut decls: BTreeMap<String, Vec<String>> = given.clone();
    for s in statements.iter().filter(|s| matches!(s.what(), Some(S::Decl(_)))) {
        let (t, n) = s.tape();
        if let Ok((name, cols, _)) = t.decl(n) {
            decls.insert(name, cols.into_iter().map(|(_, t)| t).collect());
        }
    }
    // The guest relations' tapes (`core.rerail`), for a program read without it.
    for (rel, cols) in BUILT_IN {
        decls.entry((*rel).to_owned()).or_insert_with(|| cols.iter().map(|t| (*t).to_owned()).collect());
    }
    for s in statements.iter().filter(|s| s.what() == Some(S::Clause)) {
        let (f, n) = (s.file(), s.node());
        f.check_clause(n)?;
        let line = s.line();
        let mut seen = Seen(f, BTreeMap::new());
        for h in f.heads_at(n) {
            seen.args(decls.get(&h.rel), &h.args);
        }
        let mut stack: Vec<LitAt> = f.tape().get(n, Field::Body).map_or_else(Vec::new, |b| f.body_at(b, 0));
        while let Some(l) = stack.pop() {
            match f.lit(&l) {
                L::Atom { rel, args, .. } => seen.args(decls.get(&rel), &args),
                L::Group { alts, .. } => stack.extend(alts.into_iter().flatten()),
                L::Cmp { l, op, r } => {
                    // `t = @parse(…)`: t is a tape.
                    for (a, b) in [(&l, &r), (&r, &l)] {
                        if op == "="
                            && let (T::Var(v), T::Call { name, functor: true, .. }) = (f.tv(a), f.tv(b))
                            && functor(&name).is_some_and(|(_, tape)| tape)
                        {
                            seen.add(&v, "tape");
                        }
                    }
                    seen.term(&l);
                    seen.term(&r);
                }
                L::Pattern { subject, .. } => seen.term(&subject),
            }
        }
        for (v, types) in &seen.1 {
            if types.contains("tape")
                && let Some(other) = types.iter().find(|t| OTHER.contains(&t.as_str()))
            {
                return Err(Error::Syntax(format!("line {line}: {v} is a tape in one column and a {other} in another: declare the column `tape`")));
            }
        }
    }
    Ok(())
}

// cratify: `tests` is rerail-logic's (it mounts it)
