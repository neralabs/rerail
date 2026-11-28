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
//! `.callable R PATTERN by H [C]` routes each ask by relation H of the program, `H(k, d)`: the documents that answer
//! a key whose column C (bound; the first bound column where none is written) is `k`, however the program finds them
//! (a name's home, read off the key by the language's library; an index of the files a key's rows lie in). Column 0
//! then need not name the file. The split hands H the values asked, as rows of `H__ask(k)` (declared by the program,
//! which H's rules read), and the request is `R__call(d, key..) :- the reader's body, H(k, d)`: the document first,
//! which the calls route by. One router serves every callable whose column C holds its kind of key.
//!
//! Refused at load, each naming what it refuses: a pattern of the wrong length or letters; column 0 unbound (it names
//! the file) where no H routes; an H or `H__ask` of other columns than the key's; a column of type `node` (a call's answer is values: no node crosses, decision 0677); a callable with no
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
/// The keys asked of a router `H`: `H__ask(key..)`, which the split makes and H's rules read.
pub const ASK: &str = "__ask";

/// A relation declared callable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Callable {
    pub rel: String,
    /// The columns its readers bind, in order; column 0, the file's path, first.
    pub bound: Vec<usize>,
    pub width: usize,
    /// Its request relation (`R__call`), of the bound columns (after the document, where `by` routes).
    pub request: String,
    /// The relation that routes its asks (`by H C`), `H(k, d)`, and the column C whose value is `k`.
    pub by: Option<(String, usize)>,
}

impl Callable {
    /// The pattern as written (`bbbf`).
    pub fn pattern(&self) -> String {
        (0..self.width).map(|c| if self.bound.contains(&c) { 'b' } else { 'f' }).collect()
    }
    /// Its request's columns: the document (where `by` routes), then the bound columns.
    pub fn request_width(&self) -> usize {
        self.bound.len() + usize::from(self.by.is_some())
    }
}

/// The program's callables, linted.
pub fn declared(v: &Version) -> Result<Vec<Callable>, String> {
    let mut out: Vec<Callable> = Vec::new();
    for text in ProgramFields::setting_values(v, SETTING) {
        let bad = |why: String| format!("`.callable {text}`: {why} (write `.callable R PATTERN [by H]`, PATTERN a `b` or `f` per column)");
        let words: Vec<&str> = text.split_whitespace().collect();
        let (rel, pattern, by, on) = match words[..] {
            [rel, pattern] => (rel, pattern, None, None),
            [rel, pattern, "by", h] => (rel, pattern, Some(h), None),
            [rel, pattern, "by", h, c] => (rel, pattern, Some(h), Some(c.parse::<usize>().map_err(|_| bad(format!("{c} is no column number")))?)),
            _ => return Err(bad("two words, four or five".to_owned())),
        };
        let decl = v.decl_of(rel).ok_or_else(|| bad(format!("{rel} is not declared")))?;
        let cols: Vec<(String, String)> = decl.cols().map(|(n, t)| (v.text(n).to_owned(), v.text(t).to_owned())).collect();
        if pattern.len() != cols.len() || !pattern.bytes().all(|c| c == b'b' || c == b'f') {
            return Err(bad(format!("{rel} has {} columns, and PATTERN is a `b` or `f` for each", cols.len())));
        }
        if by.is_none() && !pattern.starts_with('b') {
            return Err(bad(format!("{rel}'s column 0 names the file a call is answered in, and is bound")));
        }
        if !pattern.contains('b') {
            return Err(bad(format!("{rel} is asked by no column")));
        }
        if let Some((name, _)) = cols.iter().find(|(_, t)| t == "node") {
            return Err(bad(format!("column `{name}` of {rel} is a node: a call's asks and answers are values, and no node leaves its file (decision 0677)")));
        }
        if v.inputs().any(|a| v.text(a) == rel) {
            return Err(bad(format!("{rel} is an input: a callable is answered by its rules")));
        }
        // A caller already (its rules gone, its readers' requests written, the setting kept): split once.
        if v.rules().any(|r| v.rel_name(r.head().rel()) == format!("{rel}{REQUEST}")) {
            continue;
        }
        if !v.rules().any(|r| v.rel_name(r.head().rel()) == rel) {
            // A program made of another's rules keeps its settings (the run's constants, its requests): a callable
            // it neither makes nor reads is none of its.
            if !v.rules().any(|r| r.reads().any(|a| v.rel_name(a.rel()) == rel)) {
                continue;
            }
            return Err(bad(format!("{rel} has no rules")));
        }
        if out.iter().any(|c| c.rel == rel) {
            return Err(bad(format!("{rel} is declared callable twice")));
        }
        let bound: Vec<usize> = pattern.bytes().enumerate().filter(|(_, c)| *c == b'b').map(|(i, _)| i).collect();
        let by = match by {
            None => None,
            Some(h) => {
                let c = on.unwrap_or(bound[0]);
                if !bound.contains(&c) {
                    return Err(bad(format!("column {c} of {rel} routes its asks, and is bound")));
                }
                // H(k, d) and H__ask(k).
                let width = |r: &str| v.decl_of(r).map(|d| d.cols().count());
                if width(h) != Some(2) {
                    return Err(bad(format!("router {h} is `{h}(k, d)`: a key's value and a document that answers it")));
                }
                let ask = format!("{h}{ASK}");
                if width(&ask) != Some(1) {
                    return Err(bad(format!("router {h} reads the values asked as `{ask}(k)`, declared")));
                }
                Some((h.to_owned(), c))
            }
        };
        out.push(Callable { rel: rel.to_owned(), bound, width: cols.len(), request: format!("{rel}{REQUEST}"), by });
    }
    Ok(out)
}

/// The routers the program's callables name (`by H`), as written: what the split reads beside the program's outputs,
/// so no pass before it drops them as unread.
pub fn routers(v: &Version) -> Vec<String> {
    ProgramFields::setting_values(v, SETTING)
        .into_iter()
        .filter_map(|text| {
            let words: Vec<&str> = text.split_whitespace().collect();
            (words.get(2) == Some(&"by")).then(|| words.get(3).map(|h| (*h).to_owned())).flatten()
        })
        .collect()
}

/// Whether `rel` is a callable's router or the asks it reads (`H`, `H__ask`): the split reads them by name and fills
/// the asks, after the passes that drop what no output reads and what joins a relation nothing makes; so they stay as
/// written.
pub fn routed(v: &Version, rel: &str) -> bool {
    routers(v).iter().any(|h| h == rel || rel.strip_suffix(ASK) == Some(h.as_str()))
}

/// A callable's own program, answered in a file.
#[derive(Clone)]
pub struct Callee {
    pub callable: Callable,
    /// The relations R's rules read, besides stated facts: the file's state a call reads.
    pub reads: Vec<String>,
}

/// A program with callables, split (`split`).
#[derive(Clone)]
pub struct Split {
    pub caller: Arc<Version>,
    pub callees: Vec<Callee>,
    /// Every callee's rules in one program, each reading its request first: its inputs every request and the
    /// relations any callee reads, its outputs every callable relation. A file entered once answers every callable's
    /// requests of it at once (one program run over its rows); a callable asked nothing has an empty request and
    /// makes nothing.
    pub program: Arc<Version>,
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
pub(super) fn request_body(v: &Version, lits: &[LitRef<'_>], call: usize, rel: &str) -> (Vec<usize>, VarSet) {
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

/// A per-file relation the cross part could call (the feedback's first version): every reader binds `bound`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub rel: String,
    /// The columns every reader binds before its read, column 0 first.
    pub bound: Vec<usize>,
    pub width: usize,
    /// The rules that read it.
    pub readers: usize,
    /// Why `.callable` would refuse it as it stands (a node column), where it would.
    pub refused: Option<String>,
    /// Whether its readers bind its file (column 0, its location): a call is routed to the file it names. Otherwise
    /// a call is routed by the bound columns, which needs each file's keys.
    pub by_file: bool,
}

impl Candidate {
    /// Its pattern (`bbbf`).
    pub fn pattern(&self) -> String {
        (0..self.width).map(|c| if self.bound.contains(&c) { 'b' } else { 'f' }).collect()
    }
}

/// The relations of `v` its files make (`file_local`) that the cross part reads (`exports`), each assessed as a
/// callable: a [`Candidate`] where it is not an output, every reader is a rule of the cross part reading it
/// positively, they all bind at least one column before the read (as a request binds them, `request_body`), and at
/// least one column is left free; otherwise why not. A relay the split made (`R(..) :- R_at(_, ..)`, a file's rows
/// with their document in front) is looked through: R's readers are the relay's. Routed by its file where the
/// readers bind column 0 and it is the file's location (`located`); by its bound columns otherwise. A node column is
/// named, not a reason: it is what `.callable` refuses today.
pub fn candidates(v: &Version, file_local: &BTreeSet<String>, exports: &[String], located: &dyn Fn(&str) -> bool) -> Vec<(String, Result<Candidate, String>)> {
    let outputs: BTreeSet<&str> = v.outputs().map(|a| v.text(a)).collect();
    exports.iter().map(|rel| (rel.clone(), assessed(v, rel, file_local, &outputs, located))).collect()
}

fn assessed(v: &Version, rel: &str, file_local: &BTreeSet<String>, outputs: &BTreeSet<&str>, located: &dyn Fn(&str) -> bool) -> Result<Candidate, String> {
    if !file_local.contains(rel) {
        return Err("not made by its files alone".to_owned());
    }
    let decl = v.decl_of(rel).ok_or_else(|| "not declared".to_owned())?;
    let cols: Vec<(String, String)> = decl.cols().map(|(n, t)| (v.text(n).to_owned(), v.text(t).to_owned())).collect();
    let (bound, readers) = readers_bind(v, rel, file_local, outputs, 3)?;
    if bound.is_empty() {
        return Err("its readers bind none of its columns".to_owned());
    }
    if bound.len() == cols.len() {
        return Err("its readers bind every column (a test, not a call)".to_owned());
    }
    let by_file = bound.contains(&0) && located(rel);
    if by_file && bound.len() == 1 {
        return Err("its readers bind only its file".to_owned());
    }
    let refused = cols.iter().find(|(_, t)| t == "node").map(|(n, _)| format!("column `{n}` is a node"));
    Ok(Candidate { rel: rel.to_owned(), bound: bound.into_iter().collect(), width: cols.len(), readers, refused, by_file })
}

/// The columns of `rel` every reader binds before its read, and how many rules read it (through relays, `depth`
/// deep); why not, where a reader is a file's rule, a negation or an output.
fn readers_bind(v: &Version, rel: &str, file_local: &BTreeSet<String>, outputs: &BTreeSet<&str>, depth: usize) -> Result<(BTreeSet<usize>, usize), String> {
    if outputs.contains(rel) {
        return Err(format!("{rel} is an output"));
    }
    let mut bound: Option<BTreeSet<usize>> = None;
    let mut readers = 0;
    let mut meet = |here: BTreeSet<usize>| {
        bound = Some(match bound.take() {
            None => here,
            Some(b) => b.intersection(&here).copied().collect(),
        })
    };
    for r in v.rules() {
        let lits: Vec<LitRef<'_>> = r.body().collect();
        let mut reads = false;
        for (i, l) in lits.iter().enumerate() {
            let Lit::Atom(a) = l.view() else { continue };
            if v.rel_name(a.rel()) != rel {
                continue;
            }
            reads = true;
            let head = v.rel_name(r.head().rel());
            if a.neg() {
                return Err(format!("negated in {head}'s rule"));
            }
            if file_local.contains(head) {
                return Err(format!("read by {head}, a file's relation"));
            }
            // A relay: the rule's one literal is this read, and its head's columns are some of the read's variables.
            let args: Vec<TermRef<'_>> = a.args().collect();
            let relay: Option<Vec<usize>> = (lits.len() == 1 && depth > 0 && head != rel)
                .then(|| r.head().args().map(|t| t.var().and_then(|s| args.iter().position(|x| x.var() == Some(s)))).collect::<Option<Vec<usize>>>())
                .flatten();
            if let Some(map) = relay {
                let (theirs, n) = readers_bind(v, head, file_local, outputs, depth - 1)?;
                readers += n;
                meet(theirs.into_iter().map(|j| map[j]).collect());
                continue;
            }
            let (_, vars) = request_body(v, &lits, i, rel);
            meet(
                args.iter()
                    .enumerate()
                    .filter(|(_, t)| !matches!(t, TermRef::Wild | TermRef::Top))
                    .filter(|(_, t)| {
                        let mut vs = VarSet::new();
                        term_vars(**t, &mut vs);
                        vs.is_subset(&vars)
                    })
                    .map(|(k, _)| k)
                    .collect(),
            );
            readers += 1;
        }
        let _ = reads;
    }
    bound.map(|b| (b, readers)).ok_or_else(|| format!("no rule reads {rel}"))
}

/// The variable a routed request's document is bound to (`H(key.., d)`).
const ROUTED: &str = "rr_routed_d";

/// A callable's request's columns: the document H names (its type H's last column's), where `by` routes, then the
/// bound columns.
fn request_cols(v: &Version, c: &Callable) -> Vec<(String, String)> {
    let cols: Vec<(String, String)> = v.decl_of(&c.rel).map(|d| d.cols().map(|(n, t)| (v.text(n).to_owned(), v.text(t).to_owned())).collect()).unwrap_or_default();
    let doc = c.by.as_ref().and_then(|(h, _)| v.decl_of(h)).and_then(|d| d.cols().last()).map(|(_, t)| ("rr_d".to_owned(), v.text(t).to_owned()));
    doc.into_iter().chain(c.bound.iter().map(|&k| cols[k].clone())).collect()
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
    let derived: BTreeSet<&str> = v.rules().map(|r| v.rel_name(r.head().rel())).collect();
    let inputs: BTreeSet<String> = v.inputs().map(|a| v.text(a).to_owned()).collect();

    // The callees, checked first: what a call reads stands while it answers. Their rules, in one program.
    let mut callees = Vec::new();
    let mut b = Builder::derive(v);
    let mut state_all: Vec<String> = Vec::new();
    for c in &callables {
        let own = reads.get(&c.rel).cloned().unwrap_or_default();
        if let Some(other) = own.iter().find(|r| by_name.contains_key(r.as_str())) {
            return Err(format!("`.callable {} {}`: its rules read callable {other}: a call is answered from its file's relations alone", c.rel, c.pattern()));
        }
        if let Some(made) = own.iter().find(|r| answered.contains(*r)) {
            return Err(format!("`.callable {} {}`: its rules read {made}, which is made from calls' answers: a call reads its file's relations as they stand", c.rel, c.pattern()));
        }
        let state: Vec<String> = own.iter().filter(|r| derived.contains(r.as_str()) || inputs.contains(*r)).cloned().collect();
        for r in v.rules().filter(|r| v.rel_name(r.head().rel()) == c.rel) {
            let head: Vec<TermRef<'_>> = r.head().args().collect();
            let key: Vec<T<'_>> = c.by.iter().map(|_| T::Wild).chain(c.bound.iter().map(|&k| t_of(r, head[k]))).collect();
            let mut w = b.rule().head_from(v, r, r.head()).atom(c.request.as_str(), &key);
            for l in r.body() {
                w = w.lit(v, r, l);
            }
            w.finish()?;
        }
        let rc = request_cols(v, c);
        b.fields().declare(&c.request, &rc.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect::<Vec<_>>());
        for r in &state {
            if !state_all.contains(r) {
                state_all.push(r.clone());
            }
        }
        callees.push(Callee { callable: c.clone(), reads: state });
    }
    let callee_inputs: Vec<String> = callables.iter().map(|c| c.request.clone()).chain(state_all.iter().cloned()).collect();
    b.fields().set_inputs(&callee_inputs);
    b.fields().set_outputs(&callables.iter().map(|c| c.rel.clone()).collect::<Vec<_>>());
    b.fields().set_field(Field::Answers(&BTreeMap::new()));
    let (program, _) = b.finish()?;

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
            match &c.by {
                None => {
                    let mut w = b.rule().head(c.request.as_str(), &key);
                    for &j in &kept {
                        w = w.lit(v, r, lits[j]);
                    }
                    w.finish()?;
                }
                Some((h, on)) => {
                    // The values asked, H's to route; and the request, at each document H names.
                    let ask = format!("{h}{ASK}");
                    let k = t_of(r, args[*on]);
                    let mut w = b.rule().head(ask.as_str(), std::slice::from_ref(&k));
                    for &j in &kept {
                        w = w.lit(v, r, lits[j]);
                    }
                    w.finish()?;
                    let mut head = vec![T::Var(ROUTED)];
                    head.extend(key.iter().cloned());
                    let at = [k, T::Var(ROUTED)];
                    let mut w = b.rule().head(c.request.as_str(), &head);
                    for &j in &kept {
                        w = w.lit(v, r, lits[j]);
                    }
                    w.atom(h.as_str(), &at).finish()?;
                }
            }
        }
    }
    let mut answers = ProgramFields::answers(&**v);
    for c in &callables {
        let rc = request_cols(v, c);
        b.fields().declare(&c.request, &rc.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect::<Vec<_>>());
        answers.entry(c.request.clone()).or_default().push(c.rel.clone());
    }
    b.fields().set_field(Field::Answers(&answers));
    let (caller, _) = b.finish()?;
    Ok(Some(Split { caller, callees, program }))
}
