//! Callable per-file relations (the "Callable Per-file Relations" page; a spike, CPU only, explicit callables).
//!
//! `.callable R PATTERN` (`.callable same_globals bbbf`): relation R is never made whole. Its readers ask for it
//! with the columns PATTERN marks `b` bound, and each ask is answered by R's own rules in the file its column 0
//! names, over that file's relations as they stand, inside the caller's fixpoint (requests answered there, decision
//! 0065 §2a). Nothing materializes R for every combination of its free columns: the asks are request rows, the
//! answers R's rows for them.
//!
//! [`split`] makes the two programs:
//!
//! - **the caller**: R's rules gone; each rule that reads R also asks for it, by a request rule `R__call(bound) :-
//!   the rule's other positive atoms and the filters they bind`; and the answers `R__call -> R`. Leaving the
//!   negations and aggregates out of a request only widens it.
//! - **the callee**: R's rules, each reading its request first (`R(..) :- R__call(bound), body`). Its inputs are the
//!   request and the relations R reads, which are the file's relations as they stand: a call writes nothing of its
//!   file's.
//!
//! Refused at load, each naming what it refuses: a pattern of the wrong length or letters; column 0 unbound (it names
//! the file); a column of type `node` (a call's answer is values: no node crosses, decision 0677); a callable with no
//! rules, or an input; a reader that does not bind a bound column before its call; and a callable whose rules read a
//! callable, or a relation made from calls' answers (its file's state would not stand while it answers).

use crate::code_rule_dsl::logic::plan::body::{VarSet, atom_vars, lit_vars, term_vars};
use crate::code_rule_dsl::logic::program_facts::Field;
use crate::code_rule_dsl::logic::program_image::fields::{FieldsMut, ProgramFields};
use crate::code_rule_dsl::logic::program_image::{Builder, CmpOp, Lit, LitRef, T, TermRef, Version, t_of};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// The directive: `.callable R PATTERN`.
pub const SETTING: &str = "callable";
/// A callable's request relation is its name and this: `same_globals__call`.
pub const REQUEST: &str = "__call";

/// A relation declared callable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Callable {
    pub rel: String,
    /// The columns its readers bind, in order; column 0, the file's path, first.
    pub bound: Vec<usize>,
    pub width: usize,
    /// Its request relation (`R__call`), of the bound columns.
    pub request: String,
}

impl Callable {
    /// The pattern as written (`bbbf`).
    pub fn pattern(&self) -> String {
        (0..self.width).map(|c| if self.bound.contains(&c) { 'b' } else { 'f' }).collect()
    }
}

/// The program's callables, linted.
pub fn declared(v: &Version) -> Result<Vec<Callable>, String> {
    let mut out: Vec<Callable> = Vec::new();
    for text in ProgramFields::setting_values(v, SETTING) {
        let bad = |why: String| format!("`.callable {text}`: {why} (write `.callable R PATTERN`, PATTERN a `b` or `f` per column)");
        let words: Vec<&str> = text.split_whitespace().collect();
        let [rel, pattern] = words[..] else { return Err(bad("two words".to_owned())) };
        let decl = v.decl_of(rel).ok_or_else(|| bad(format!("{rel} is not declared")))?;
        let cols: Vec<(String, String)> = decl.cols().map(|(n, t)| (v.text(n).to_owned(), v.text(t).to_owned())).collect();
        if pattern.len() != cols.len() || !pattern.bytes().all(|c| c == b'b' || c == b'f') {
            return Err(bad(format!("{rel} has {} columns, and PATTERN is a `b` or `f` for each", cols.len())));
        }
        if !pattern.starts_with('b') {
            return Err(bad(format!("{rel}'s column 0 names the file a call is answered in, and is bound")));
        }
        if let Some((name, _)) = cols.iter().find(|(_, t)| t == "node") {
            return Err(bad(format!("column `{name}` of {rel} is a node: a call's asks and answers are values, and no node leaves its file (decision 0677)")));
        }
        if v.inputs().any(|a| v.text(a) == rel) {
            return Err(bad(format!("{rel} is an input: a callable is answered by its rules")));
        }
        if !v.rules().any(|r| v.rel_name(r.head().rel()) == rel) {
            return Err(bad(format!("{rel} has no rules")));
        }
        if out.iter().any(|c| c.rel == rel) {
            return Err(bad(format!("{rel} is declared callable twice")));
        }
        let bound: Vec<usize> = pattern.bytes().enumerate().filter(|(_, c)| *c == b'b').map(|(i, _)| i).collect();
        out.push(Callable { rel: rel.to_owned(), bound, width: cols.len(), request: format!("{rel}{REQUEST}") });
    }
    Ok(out)
}

/// A callable's own program, answered in a file.
#[derive(Clone)]
pub struct Callee {
    pub callable: Callable,
    /// R's rules, each reading the request first; its inputs the request and `reads`, its output R.
    pub program: Arc<Version>,
    /// The relations R's rules read, besides stated facts: the file's state a call reads.
    pub reads: Vec<String>,
}

/// A program with callables, split (`split`).
#[derive(Clone)]
pub struct Split {
    pub caller: Arc<Version>,
    pub callees: Vec<Callee>,
}

/// The relations each relation's rules read.
fn reads_of(v: &Version) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for r in v.rules() {
        let head = v.rel_name(r.head().rel()).to_owned();
        out.entry(head).or_default().extend(r.reads().map(|a| v.rel_name(a.rel()).to_owned()));
    }
    out
}

/// The relations made from any of `from`, through the rules (each of `from` too).
fn made_from(reads: &BTreeMap<String, BTreeSet<String>>, from: &BTreeSet<String>) -> BTreeSet<String> {
    let mut out = from.clone();
    loop {
        let more: Vec<String> = reads.iter().filter(|(h, rs)| !out.contains(*h) && rs.iter().any(|r| out.contains(r))).map(|(h, _)| h.clone()).collect();
        if more.is_empty() {
            return out;
        }
        out.extend(more);
    }
}

/// Which of rule `r`'s literals a request for its call at literal `call` keeps, and the variables they bind: every
/// positive atom but those of the called relation, then each filter, comparison and axis whose variables those bind
/// (an equality binds its one unbound side's variable from the other's).
fn request_body(v: &Version, lits: &[LitRef<'_>], call: usize, rel: &str) -> (Vec<usize>, VarSet) {
    let mut keep = vec![false; lits.len()];
    let mut bound = VarSet::new();
    for (i, l) in lits.iter().enumerate() {
        if let Lit::Atom(a) = l.view()
            && i != call
            && !a.neg()
            && v.rel_name(a.rel()) != rel
        {
            keep[i] = true;
            bound.extend(&atom_vars(a));
        }
    }
    let lone = |t: TermRef<'_>| match t {
        TermRef::Var(s) => Some(s),
        _ => None,
    };
    let vars = |t: TermRef<'_>| {
        let mut out = VarSet::new();
        term_vars(t, &mut out);
        out
    };
    loop {
        let mut changed = false;
        for (i, l) in lits.iter().enumerate() {
            if keep[i] {
                continue;
            }
            match l.view() {
                Lit::Cmp { l: lt, op, r: rt } => {
                    if lit_vars(*l).is_subset(&bound) {
                        keep[i] = true;
                        changed = true;
                    } else if op == CmpOp::Eq {
                        for (side, other) in [(lt, rt), (rt, lt)] {
                            if let Some(s) = lone(side).filter(|s| !bound.contains(*s))
                                && vars(other).is_subset(&bound)
                            {
                                keep[i] = true;
                                bound.insert(s);
                                changed = true;
                                break;
                            }
                        }
                    }
                }
                Lit::Nearest { n, a, .. } if bound.contains(n) => {
                    keep[i] = true;
                    bound.insert(a);
                    changed = true;
                }
                _ => {}
            }
        }
        if !changed {
            break;
        }
    }
    ((0..lits.len()).filter(|&i| keep[i]).collect(), bound)
}

/// `v` split into its caller and its callees (module docs); `None` where it declares no callable.
pub fn split(v: &Arc<Version>) -> Result<Option<Split>, String> {
    let callables = declared(v)?;
    if callables.is_empty() {
        return Ok(None);
    }
    let by_name: BTreeMap<&str, &Callable> = callables.iter().map(|c| (c.rel.as_str(), c)).collect();
    let reads = reads_of(v);
    let answered = made_from(&reads, &callables.iter().map(|c| c.rel.clone()).collect());
    let cols_of = |rel: &str| -> Vec<(String, String)> { v.decl_of(rel).map(|d| d.cols().map(|(n, t)| (v.text(n).to_owned(), v.text(t).to_owned())).collect()).unwrap_or_default() };
    let derived: BTreeSet<&str> = v.rules().map(|r| v.rel_name(r.head().rel())).collect();
    let inputs: BTreeSet<String> = v.inputs().map(|a| v.text(a).to_owned()).collect();

    // The callees, checked first: what a call reads stands while it answers.
    let mut callees = Vec::new();
    for c in &callables {
        let own = reads.get(&c.rel).cloned().unwrap_or_default();
        if let Some(other) = own.iter().find(|r| by_name.contains_key(r.as_str())) {
            return Err(format!("`.callable {} {}`: its rules read callable {other}: a call is answered from its file's relations alone", c.rel, c.pattern()));
        }
        if let Some(made) = own.iter().find(|r| answered.contains(*r)) {
            return Err(format!("`.callable {} {}`: its rules read {made}, which is made from calls' answers: a call reads its file's relations as they stand", c.rel, c.pattern()));
        }
        let state: Vec<String> = own.iter().filter(|r| derived.contains(r.as_str()) || inputs.contains(*r)).cloned().collect();
        let mut b = Builder::derive(v);
        for r in v.rules().filter(|r| v.rel_name(r.head().rel()) == c.rel) {
            let head: Vec<TermRef<'_>> = r.head().args().collect();
            let key: Vec<T<'_>> = c.bound.iter().map(|&k| t_of(r, head[k])).collect();
            let mut w = b.rule().head_from(v, r, r.head()).atom(c.request.as_str(), &key);
            for l in r.body() {
                w = w.lit(v, r, l);
            }
            w.finish()?;
        }
        let cols = cols_of(&c.rel);
        let key_cols: Vec<(&str, &str)> = c.bound.iter().map(|&k| (cols[k].0.as_str(), cols[k].1.as_str())).collect();
        b.fields().declare(&c.request, &key_cols);
        let callee_inputs: Vec<String> = std::iter::once(c.request.clone()).chain(state.iter().cloned()).collect();
        b.fields().set_inputs(&callee_inputs);
        b.fields().set_outputs(std::slice::from_ref(&c.rel));
        b.fields().set_field(Field::Answers(&BTreeMap::new()));
        let (program, _) = b.finish()?;
        callees.push(Callee { callable: c.clone(), program, reads: state });
    }

    // The caller: R's rules gone, each read of R asking for it.
    let mut b = Builder::derive(v);
    for r in v.rules() {
        if by_name.contains_key(v.rel_name(r.head().rel())) {
            continue;
        }
        b.keep(v, r);
        let lits: Vec<LitRef<'_>> = r.body().collect();
        for (i, l) in lits.iter().enumerate() {
            let Lit::Atom(a) = l.view() else { continue };
            let Some(c) = by_name.get(v.rel_name(a.rel())) else { continue };
            if a.neg() {
                return Err(format!("`{}`: callable {} is negated: a call is asked for with its columns {} bound, and answers rows", v.show_rule(r), c.rel, c.pattern()));
            }
            let (kept, bound) = request_body(v, &lits, i, &c.rel);
            let args: Vec<TermRef<'_>> = a.args().collect();
            for &k in &c.bound {
                let ok = match args[k] {
                    TermRef::Wild | TermRef::Top => false,
                    t => {
                        let mut vs = VarSet::new();
                        term_vars(t, &mut vs);
                        vs.is_subset(&bound)
                    }
                };
                if !ok {
                    return Err(format!("`{}`: column {k} of callable {} is bound in its pattern {}, and no other literal of the rule binds it before the call", v.show_rule(r), c.rel, c.pattern()));
                }
            }
            let key: Vec<T<'_>> = c.bound.iter().map(|&k| t_of(r, args[k])).collect();
            let mut w = b.rule().head(c.request.as_str(), &key);
            for &j in &kept {
                w = w.lit(v, r, lits[j]);
            }
            w.finish()?;
        }
    }
    let mut answers = ProgramFields::answers(&**v);
    for c in &callables {
        let cols = cols_of(&c.rel);
        let key_cols: Vec<(&str, &str)> = c.bound.iter().map(|&k| (cols[k].0.as_str(), cols[k].1.as_str())).collect();
        b.fields().declare(&c.request, &key_cols);
        answers.entry(c.request.clone()).or_default().push(c.rel.clone());
    }
    b.fields().set_field(Field::Answers(&answers));
    let (caller, _) = b.finish()?;
    Ok(Some(Split { caller, callees }))
}
