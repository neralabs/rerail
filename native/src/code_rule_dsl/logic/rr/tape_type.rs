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

use rerail_rr_syntax::syntax::{Head, Item, Lit, Statement, Term};
use crate::code_rule_dsl::logic::front::Error;

/// The types a tape is not: a variable in one of these and in a tape column is refused.
const OTHER: [&str; 8] = ["symbol", "sym", "number", "int", "unsigned", "float", "node", "span"];

/// The relations of `core.rr` with a tape column, and their columns' types.
const BUILT_IN: &[(&str, &[&str])] = &[
    ("sub_tape", &["node", "symbol", "tape"]),
    ("tape_of", &["number", "tape"]),
    ("parse_of", &["symbol", "symbol", "tape"]),
    ("unparse_of", &["tape", "symbol"]),
    ("splice_of", &["tape", "node", "tape", "tape"]),
];

/// A relation of `core.rr`'s tape columns.
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

/// Each variable's types in one clause, as its columns and functors give them.
#[derive(Default)]
struct Seen(BTreeMap<String, BTreeSet<String>>);

impl Seen {
    fn add(&mut self, v: &str, t: &str) {
        self.0.entry(v.to_owned()).or_default().insert(t.to_owned());
    }
    fn args(&mut self, cols: Option<&Vec<String>>, args: &[Term]) {
        for (i, a) in args.iter().enumerate() {
            match a {
                Term::Var(v) => {
                    if let Some(t) = cols.and_then(|c| c.get(i)) {
                        self.add(v, t);
                    }
                }
                other => self.term(other),
            }
        }
    }
    fn term(&mut self, t: &Term) {
        match t {
            Term::Call { name, args, functor: true } if let Some((tapes, _)) = functor(name) => {
                for (i, a) in args.iter().enumerate() {
                    match a {
                        Term::Var(v) if tapes.contains(&i) => self.add(v, "tape"),
                        other => self.term(other),
                    }
                }
            }
            Term::Call { args, .. } => args.iter().for_each(|a| self.term(a)),
            Term::Bin { l, r, .. } => {
                self.term(l);
                self.term(r);
            }
            Term::Named(_, t) => self.term(t),
            _ => {}
        }
    }
}

/// The relations' declared column types, then each clause's variables: an error at the first variable that is a tape
/// and something else.
pub(super) fn check(statements: &[Statement]) -> Result<(), Error> {
    let mut decls: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (item, _) in statements {
        if let Item::Decl { name, cols, .. } = item {
            decls.insert(name.clone(), cols.iter().map(|(_, t)| t.clone()).collect());
        }
    }
    // The guest relations' tapes (`core.rr`), for a program read without it.
    for (rel, cols) in BUILT_IN {
        decls.entry((*rel).to_owned()).or_insert_with(|| cols.iter().map(|t| (*t).to_owned()).collect());
    }
    for (item, line) in statements {
        let Item::Clause { heads, body, .. } = item else { continue };
        let mut seen = Seen::default();
        for Head { rel, args, .. } in heads {
            seen.args(decls.get(rel), args);
        }
        let mut stack: Vec<&Lit> = body.iter().collect();
        while let Some(l) = stack.pop() {
            match l {
                Lit::Atom { rel, args, .. } => seen.args(decls.get(rel), args),
                Lit::Group { alts, .. } => stack.extend(alts.iter().flatten()),
                Lit::Cmp { l, op, r } => {
                    // `t = @parse(…)`: t is a tape.
                    for (a, b) in [(l, r), (r, l)] {
                        if *op == "="
                            && let (Term::Var(v), Term::Call { name, functor: true, .. }) = (a, b)
                            && functor(name).is_some_and(|(_, tape)| tape)
                        {
                            seen.add(v, "tape");
                        }
                    }
                    seen.term(l);
                    seen.term(r);
                }
                Lit::Pattern { subject, .. } => seen.term(subject),
            }
        }
        for (v, types) in &seen.0 {
            if types.contains("tape")
                && let Some(other) = types.iter().find(|t| OTHER.contains(&t.as_str()))
            {
                return Err(Error::Syntax(format!("line {line}: {v} is a tape in one column and a {other} in another: declare the column `tape`")));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "tape_type_tests.rs"]
mod tests;
