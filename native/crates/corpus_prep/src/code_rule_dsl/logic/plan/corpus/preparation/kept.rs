//! Decision 0597 rule 8 (step 7): a pass's tape-local facts are kept by its tape. A context relation whose rows are a
//! function of its own tape (`tape_local`) is the same for every string holding the text, so a value's rows of it are
//! made once and kept under the value's key (a memo entry of the value: its text, `tape:<grammar>`, the program), and
//! a later chunk or run that asks for the text is given them and runs none of the pass's strata over it. The rewrite
//! (`Kept`) puts that in the program, before the preparation proper (`front_of`):
//!
//! - each tape-local relation `R(cols)` of a pass carries its value's document, `R_rr_kd(rr_kd, cols)` (a name
//!   `front::document_column` knows, so the split locates what reads it): its rules bind it
//!   from the pass's nodes, now `rr_fresh(rr_kd, n)` (the nodes of the roots no entry gave), or from the tape-local
//!   relations they read;
//! - the pass's fresh roots, `rr_fresh_roots_<pass>(r) :- roots(r), !rr_kept_roots_<pass>(r)`, a host relation: the
//!   roots whose rows are made;
//! - each frontier relation (a tape-local relation a rule outside them reads) has an input of its kept rows,
//!   `R_rr_kept(rr_kd, cols)`, and the host reads it through `{prefix}rr_read_<pass>_<rel>(rr_kd, cols)`, its made
//!   rows and its kept ones: the export as read. A rule outside reads that, as a host's rule reads a `.from` alias, so
//!   a reader in the cross-file part reads a file-local relation, never the kept input (a base relation, which the
//!   corpus batch does not hold);
//! - `rr_kept_roots_<pass>` and the kept rows are inputs the value's request answers (`guests::answered`).
//!
//! A pass is kept so only where every condition holds (`groups`); otherwise it runs as before.

use crate::code_rule_dsl::logic::passes_made::{self, PREFIX};
use crate::code_rule_dsl::logic::plan::corpus::guests::{FRESH_ROOTS, KEPT, KEPT_ROOTS, TAPE};
use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::{AtomRef, Builder, Rewrite, RuleRef, T, Version, fields::ProgramFields};
use crate::code_rule_dsl::logic::plan::fields::BY_NAME;
use std::collections::{BTreeMap, BTreeSet};

/// A context's nodes (`contexts::NODES`) and a region's holes (`contexts::HELD`), as instances name them.
const NODES: &str = "rr_nodes";
const HELD: &str = "rr_held";
/// The fresh roots' nodes, by document: `instance(pass, FRESH)(rr_kd, n)`.
pub const FRESH: &str = "rr_fresh";
/// The document column a kept relation carries, and the variable its rules bind it to.
pub const DOC: &str = "rr_kd";
/// The variables a host's read of a frontier relation names its columns by (a relation is narrower than this).
const COLS: [&str; 12] = ["rr_c0", "rr_c1", "rr_c2", "rr_c3", "rr_c4", "rr_c5", "rr_c6", "rr_c7", "rr_c8", "rr_c9", "rr_c10", "rr_c11"];
/// A guest's host (0431): a read of one is a read of the host's place.
const HOSTS_NODE: [&str; 2] = ["host_node", "guest_of"];

/// A pass under a program's prefix (a pack's `p0_`): its instances are `{prefix}rr_pass_{pass}_…`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pass {
    pub prefix: String,
    pub name: String,
}

impl Pass {
    fn instance(&self, rel: &str) -> String {
        format!("{}{PREFIX}{}_{rel}", self.prefix, self.name)
    }
    fn nodes(&self) -> String {
        self.instance(NODES)
    }
    pub fn fresh(&self) -> String {
        self.instance(FRESH)
    }
    pub fn kept_roots(&self) -> String {
        format!("{}{KEPT_ROOTS}{}", self.prefix, self.name)
    }
    pub fn fresh_roots(&self) -> String {
        format!("{}{FRESH_ROOTS}{}", self.prefix, self.name)
    }
    /// The host's read of frontier relation `rel` (an instance of this pass): its made rows and its kept ones.
    pub fn read(&self, rel: &str) -> String {
        let local = rel.strip_prefix(&self.instance("")).unwrap_or(rel);
        format!("{}rr_read_{}_{local}", self.prefix, self.name)
    }
}

/// Tape-local relation `rel` as the kept program derives it, its rows led by their document.
pub fn tape(rel: &str) -> String {
    format!("{rel}{TAPE}")
}

/// The kept rows of frontier relation `rel`.
pub fn kept(rel: &str) -> String {
    format!("{rel}{KEPT}")
}

/// The made passes of `v` (its `.pass` settings): each one's name, its roots as `v` names them, and whether its roots
/// are regions (`holes`), whose nodes are the host's tape, not a value's.
fn made(v: &Version) -> Vec<(String, String, bool)> {
    ProgramFields::settings(v)
        .into_iter()
        .filter(|(k, _)| k == "pass")
        .filter_map(|(_, text)| {
            let w = passes_made::words(&text);
            (w.len() > 4 && w[1] == "of" && w[3] == "roots").then(|| (w[0].clone(), w[4].clone(), w.iter().any(|x| x == "holes")))
        })
        .collect()
}

/// The pass relation `rel` is an instance of, of `names` (the made passes'), under its prefix.
fn pass_of(rel: &str, names: &BTreeSet<String>) -> Option<Pass> {
    let at = rel.find(PREFIX)?;
    let rest = &rel[at + PREFIX.len()..];
    names.iter().filter(|n| rest.starts_with(&format!("{n}_"))).max_by_key(|n| n.len()).map(|n| Pass { prefix: rel[..at].to_owned(), name: n.clone() })
}

/// Each made pass's instance relations of `v`, by what they read (0597 rule 8): those whose rows are a function of the
/// pass's own tape (their rules read views, base relations, the pass's node set and its other such relations), and
/// those that read a host's (`host_node`/`guest_of`, or any derived relation not the pass's). The node set is neither.
pub fn tape_local(v: &Version) -> BTreeMap<Pass, (BTreeSet<String>, BTreeSet<String>)> {
    let names: BTreeSet<String> = made(v).into_iter().map(|(n, ..)| n).collect();
    let views = crate::code_rule_dsl::logic::plan::corpus::hiding::VIEWS;
    let derived = |x: &str| v.rel(x).is_some_and(|id| v.heads(id).next().is_some());
    let is_nodes = |rel: &str, p: &Pass| rel == p.nodes() || rel == p.instance(HELD);
    let rels: BTreeMap<String, Pass> = v.rules().map(|r| v.rel_name(r.head().rel()).to_owned()).filter_map(|h| pass_of(&h, &names).map(|p| (h, p))).collect();
    let mut own: BTreeSet<String> = rels.iter().filter(|(r, p)| !is_nodes(r, p)).map(|(r, _)| r.clone()).collect();
    loop {
        let was = own.clone();
        own.retain(|rel| {
            let p = &rels[rel];
            v.heads(v.rel(rel).expect("a head")).all(|r| {
                r.reads().all(|a| {
                    let x = v.rel_name(a.rel());
                    if HOSTS_NODE.contains(&x) {
                        return false;
                    }
                    if !derived(x) || views.contains(&x) {
                        return true;
                    }
                    is_nodes(x, p) || (rels.get(x) == Some(p) && was.contains(x))
                })
            })
        });
        if own.len() == was.len() {
            break;
        }
    }
    let mut out: BTreeMap<Pass, (BTreeSet<String>, BTreeSet<String>)> = BTreeMap::new();
    for (rel, p) in rels {
        if is_nodes(&rel, &p) {
            continue;
        }
        let e = out.entry(p).or_default();
        if own.contains(&rel) { e.0.insert(rel) } else { e.1.insert(rel) };
    }
    out
}

/// A pass kept by its tape: its roots relation, its tape-local relations and of them the frontier.
#[derive(Clone, Debug)]
pub struct Group {
    pub pass: Pass,
    pub roots: String,
    pub tape: BTreeSet<String>,
    pub frontier: BTreeSet<String>,
}

/// The passes of `v` kept by their tapes, each where: its roots are text roots (no holes) and named once; no
/// tape-local relation is an output; and each tape-local rule binds its document (a positive read of the pass's nodes
/// or of a tape-local relation). With the reason each other pass is not.
pub fn groups(v: &Version) -> (Vec<Group>, Vec<String>) {
    let made = made(v);
    let outputs: BTreeSet<&str> = v.outputs().map(|a| v.text(a)).collect();
    let (mut out, mut not) = (Vec::new(), Vec::new());
    for (pass, (tape, _)) in tape_local(v) {
        let at = |why: String| format!("{}{}: {why}", pass.prefix, pass.name);
        let roots: Vec<&(String, String, bool)> = made.iter().filter(|(n, r, _)| *n == pass.name && (pass.prefix.is_empty() || r.starts_with(&pass.prefix))).collect();
        let [(_, roots, holes)] = roots.as_slice() else {
            not.push(at(format!("its roots are named {} times", roots.len())));
            continue;
        };
        if *holes {
            not.push(at("its roots are regions of the host's tape".into()));
            continue;
        }
        if let Some(o) = tape.iter().find(|r| outputs.contains(r.as_str())) {
            not.push(at(format!("{o} is an output")));
            continue;
        }
        let nodes = pass.nodes();
        let binds = |r: RuleRef<'_>| r.body().filter_map(|l| l.atom()).any(|a| !a.neg() && (v.rel_name(a.rel()) == nodes || tape.contains(v.rel_name(a.rel()))));
        if let Some(r) = tape.iter().flat_map(|t| v.heads(v.rel(t).expect("a head"))).find(|&r| !binds(r)) {
            not.push(at(format!("a rule of {} binds no document", v.rel_name(r.head().rel()))));
            continue;
        }
        let outside = |r: RuleRef<'_>| !tape.contains(v.rel_name(r.head().rel()));
        let frontier: BTreeSet<String> = v.rules().filter(|&r| outside(r)).flat_map(|r| r.reads().map(|a| v.rel_name(a.rel()).to_owned()).collect::<Vec<_>>()).filter(|x| tape.contains(x)).collect();
        if frontier.is_empty() {
            not.push(at("no rule outside reads its tape-local relations".into()));
            continue;
        }
        if let Some(f) = frontier.iter().find(|f| v.columns_of(f).is_none_or(|c| c.len() >= COLS.len())) {
            not.push(at(format!("{f} has no columns declared, or too many")));
            continue;
        }
        out.push(Group { pass, roots: roots.clone(), tape, frontier });
    }
    (out, not)
}

/// The rewrite of one rule: within a tape-local rule, each tape-local atom (the head too) carries the document and
/// the pass's nodes are the fresh roots'; outside, each read of the frontier reads the host's read of it, any document.
struct Each<'g> {
    /// The tape-local relations, whose atoms carry the document.
    carry: &'g BTreeSet<String>,
    /// Within a tape-local rule (the document's variable), or outside (`_`).
    inside: bool,
    /// The pass's nodes relation, read as the fresh roots' nodes (inside only).
    nodes: Option<(&'g str, String)>,
    /// Each frontier relation's read (outside only).
    reads: &'g BTreeMap<String, String>,
    names: &'g dyn Fn(AtomRef<'_>) -> String,
}

impl Rewrite for Each<'_> {
    fn head_as(&mut self, a: AtomRef<'_>) -> Option<String> {
        let rel = (self.names)(a);
        (self.inside && self.carry.contains(&rel)).then(|| tape(&rel))
    }
    fn atom_as(&mut self, a: AtomRef<'_>) -> Option<(String, bool, bool)> {
        let rel = (self.names)(a);
        if let Some((nodes, fresh)) = &self.nodes
            && rel == *nodes
        {
            return Some((fresh.clone(), a.neg(), a.gated()));
        }
        if self.inside && self.carry.contains(&rel) {
            return Some((tape(&rel), a.neg(), a.gated()));
        }
        if !self.inside
            && let Some(read) = self.reads.get(&rel)
        {
            return Some((read.clone(), a.neg(), a.gated()));
        }
        None
    }
    fn prefix(&mut self, a: AtomRef<'_>, out: &mut Vec<T<'static>>) {
        let rel = (self.names)(a);
        let carries = self.carry.contains(&rel) || self.nodes.as_ref().is_some_and(|(n, _)| rel == *n);
        if carries {
            out.push(if self.inside { T::Var(DOC) } else { T::Wild });
        }
    }
}

/// `v` with each kept pass's rules rewritten (`groups`).
pub fn write(v: &Version, groups: &[Group], b: &mut Builder<'_>) -> Result<(), String> {
    let names = |a: AtomRef<'_>| v.rel_name(a.rel()).to_owned();
    let tape_of: BTreeMap<&str, &Group> = groups.iter().flat_map(|g| g.tape.iter().map(move |t| (t.as_str(), g))).collect();
    let frontier: BTreeSet<String> = groups.iter().flat_map(|g| g.frontier.iter().cloned()).collect();
    let reads: BTreeMap<String, String> = groups.iter().flat_map(|g| g.frontier.iter().map(move |f| (f.clone(), g.pass.read(f)))).collect();
    let carry: BTreeSet<String> = tape_of.keys().map(|s| (*s).to_owned()).collect();
    for r in v.rules() {
        let head = v.rel_name(r.head().rel());
        if let Some(g) = tape_of.get(head) {
            let nodes = g.pass.nodes();
            let mut each = Each { carry: &carry, inside: true, nodes: Some((nodes.as_str(), g.pass.fresh())), reads: &reads, names: &names };
            b.rewrite(v, r, &mut each).finish()?;
            continue;
        }
        if !r.reads().any(|a| frontier.contains(v.rel_name(a.rel()))) {
            b.keep(v, r);
            continue;
        }
        let mut each = Each { carry: &carry, inside: false, nodes: None, reads: &reads, names: &names };
        b.rewrite(v, r, &mut each).finish()?;
    }
    for g in groups {
        let p = &g.pass;
        let arity = v.columns_of(&g.roots).map_or(1, |c| c.len());
        let root: Vec<T<'_>> = std::iter::once(T::Var("r")).chain((1..arity).map(|_| T::Wild)).collect();
        let (fresh_roots, kept_roots, fresh) = (p.fresh_roots(), p.kept_roots(), p.fresh());
        b.fields().declare(&fresh_roots, &[("r", "node")]);
        b.fields().add_qualifiers(&[(fresh_roots.as_str(), BY_NAME)]);
        b.fields().declare(&kept_roots, &[("r", "node")]);
        b.fields().add_input(&kept_roots);
        b.fields().declare(&fresh, &[(DOC, "int"), ("n", "node")]);
        b.rule().head(fresh_roots.as_str(), &[T::Var("r")]).atom(g.roots.as_str(), &root).not(kept_roots.as_str(), &[T::Var("r")]).finish()?;
        b.rule().head(fresh.as_str(), &[T::Var(DOC), T::Var("r")]).atom(fresh_roots.as_str(), &[T::Var("r")]).atom("docof", &[T::Var("r"), T::Var(DOC)]).finish()?;
        if v.columns_of("within").is_some() {
            b.rule().head(fresh.as_str(), &[T::Var(DOC), T::Var("n")]).atom(fresh_roots.as_str(), &[T::Var("r")]).atom("within", &[T::Var("n"), T::Var("r")]).atom("docof", &[T::Var("r"), T::Var(DOC)]).finish()?;
        }
        for rel in &g.tape {
            let cols = v.columns_of(rel).unwrap_or_default();
            let pairs: Vec<(&str, &str)> = std::iter::once((DOC, "int")).chain(cols.iter().map(|(n, t)| (n.as_str(), t.as_str()))).collect();
            b.fields().declare(&tape(rel), &pairs);
            b.fields().undeclare(rel);
            if g.frontier.contains(rel) {
                let (k, read) = (kept(rel), p.read(rel));
                // The tape rows are read by name at the chunk's end; the host's read keeps its document column, the
                // location the split places it by (its readers ask for none).
                b.fields().add_qualifiers(&[(tape(rel).as_str(), BY_NAME), (read.as_str(), BY_NAME)]);
                b.fields().declare(&k, &pairs);
                b.fields().add_input(&k);
                b.fields().declare(&read, &pairs);
                let row: Vec<T<'_>> = pairs.iter().enumerate().map(|(i, _)| if i == 0 { T::Var(DOC) } else { T::Var(COLS[(i - 1).min(COLS.len() - 1)]) }).collect();
                b.rule().head(read.as_str(), &row).atom(tape(rel).as_str(), &row).finish()?;
                b.rule().head(read.as_str(), &row).atom(k.as_str(), &row).finish()?;
            }
        }
    }
    Ok(())
}

/// The rewrite, a pass of the preparation (`front_of`): each pass kept by its tape where it can be, the others named
/// in the run's profile with why.
pub struct Kept;

impl ProgramPass for Kept {
    const NAME: &'static str = "0597 rule 8: a pass's tape-local facts kept by its tape";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let v = cx.v();
        let (groups, not) = groups(v);
        crate::code_rule_dsl::logic::profile::decision("the passes kept by their tapes (decision 0597 rule 8)", || {
            let kept: Vec<String> = groups.iter().map(|g| format!("{}{}: {} tape-local, frontier {}", g.pass.prefix, g.pass.name, g.tape.len(), g.frontier.iter().cloned().collect::<Vec<_>>().join(" "))).collect();
            format!("kept: {}; not: {}", if kept.is_empty() { "none".into() } else { kept.join("; ") }, if not.is_empty() { "none".into() } else { not.join("; ") })
        });
        if groups.is_empty() {
            return Ok(());
        }
        write(v, &groups, w.derive()?)
    }
}
