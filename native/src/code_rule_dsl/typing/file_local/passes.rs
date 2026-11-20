//! The split's preparation passes on the pass manager (decision 0281's path, step 5), each reading the split from the
//! version (`FileLocal`).

use super::FileLocal;
use crate::code_rule_dsl::logic::plan::fields::{Fields, FieldsData};
use crate::code_rule_dsl::logic::program_image::Builder;
use crate::code_rule_dsl::logic::program_image::build::T;
use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::plan::body;
use crate::code_rule_dsl::logic::program_image::{Lit as L, LitRef, RuleRef, Slot, TermRef, Version};
use std::collections::{BTreeSet, HashMap, HashSet};

mod factorize;
pub(crate) use factorize::factorize_intermediates;
mod push;
mod requests;
pub use factorize::FactorizeExports;
pub use requests::RequestsFileLocal;
pub use push::PushLocalJoins;

/// A term of `v`'s rule as a builder's term: a variable by the atom naming it (a derivation's ids are its input's).
pub(super) fn t_of<'a>(v: &'a Version, rule: RuleRef<'a>, t: TermRef<'a>) -> T<'a> {
    match t {
        TermRef::Var(s) => T::VarAtom(rule.var(s)),
        TermRef::Wild => T::Wild,
        TermRef::Top => T::Top,
        TermRef::Int(n) => T::Int(n),
        TermRef::Str(a) => T::StrAtom(a),
        TermRef::Call(f, args) => T::Call(v.text(f), args.map(|a| t_of(v, rule, a)).collect()),
        TermRef::Branded(b) => T::Branded(b),
    }
}

/// One round of `locate_all`: every relation the chunks hand on (an export, or a file-local output) given a location
/// column, so each of its rows names its document (decision 0052). A relation without one, `r(p, l)`, is derived by
/// the chunks as `r_at(d, p, l)` (`r_at_` where the program has an `r_at` of its own), `d` a document its rules bind
/// (from `node`, `doc`, `tape_line`, …, a document column left unnamed included, or a node through `node(d, n, …)`),
/// and the cross-file part reads `r(p, l) :- r_at(_, p, l)`. A relation that file-local rules also read, a lattice, or
/// one whose rule binds no location is left, and named in `blocked`: its rows are not attributable.
#[derive(Default)]
pub struct Locate {
    pub located: Vec<String>,
    pub blocked: Vec<String>,
}

impl Locate {
    /// The round's result: the relations located, or those left.
    pub fn result(&self) -> Result<Vec<String>, Vec<String>> {
        if self.blocked.is_empty() { Ok(self.located.clone()) } else { Err(self.blocked.clone()) }
    }
}

/// Every relation the chunks hand on made to name its document per row, so a file's rows can be memoised (decision
/// 0052): `Locate`, and a relation a round leaves (its rows name no document) moved to the cross-file part
/// (`MoveCrossFile`, decision 0290), with every file-local relation that reads it, and the program located again,
/// until every row handed on is located. The chunks then hand on only located rows, and the per-file memo holds for the
/// rest of the program. The relations located; the `Err` names the moved relations where a round blocks one the moves
/// cannot place, which none is.
pub fn locate_all(p: &mut crate::code_rule_dsl::logic::program_image::manager::Pipeline<'_>) -> Result<Vec<String>, Vec<String>> {
    let mut moved: Vec<String> = Vec::new();
    let mut located: Vec<String> = Vec::new();
    loop {
        let mut locate = Locate::default();
        p.program(&mut locate);
        located.extend(locate.located);
        if locate.blocked.is_empty() {
            return Ok(located);
        }
        let fresh: Vec<String> = locate.blocked.into_iter().filter(|r| !moved.contains(r)).collect();
        if fresh.is_empty() {
            moved.sort();
            return Err(moved);
        }
        moved.extend(fresh.iter().cloned());
        p.program(&mut MoveCrossFile(fresh));
    }
}

/// A round of `locate_all`'s move: the relations a round of `Locate` left, their rows naming no document,
/// qualified `CROSS_FILE` (decision 0290). The split then holds them cross-file, with their file-local readers, and the
/// program is located again (`preparation::run`).
pub struct MoveCrossFile(pub Vec<String>);

impl ProgramPass for MoveCrossFile {
    const NAME: &'static str = "move the relations naming no document to the cross-file part";
    fn settings(&self) -> Vec<u8> {
        self.0.join("\n").into_bytes()
    }
    fn run(&mut self, _cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let words: Vec<(&str, &str)> = self.0.iter().map(|r| (r.as_str(), super::CROSS_FILE)).collect();
        w.on()?.fields().add_qualifiers(&words);
        Ok(())
    }
}

/// How a located rule names its document.
#[derive(Clone, Copy)]
enum Doc {
    /// A document column it binds.
    Var(Slot),
    /// One it leaves unnamed: body literal `.0`'s column `.1`, which then names it.
    Unnamed(usize, usize),
    /// Through a node it binds: by `docof` (`true`) or `node`.
    Through(Slot, bool),
}

/// A rule of the located program, in order: kept, made from a target's rule, or a target read from its located form.
enum Item<'v> {
    Keep(RuleRef<'v>),
    Made { rule: RuleRef<'v>, at: usize, doc: Doc },
    ReadAt { rel: String, at: usize, width: usize },
}

impl ProgramPass for Locate {
    const NAME: &'static str = "locate the chunks' rows";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let v = cx.v();
        let d = cx.get::<FieldsData>()?;
        let f = Fields::over(v, &d);
        let split = cx.get::<FileLocal>()?;
        let (file_local, constants) = split.found();
        let locations = &split.locations;
        let super::on_version::Crossing { exports, .. } = super::on_version::crossing(v, f, &file_local);
        let rules: Vec<RuleRef<'_>> = v.rules().collect();
        let head = |r: &RuleRef<'_>| v.rel_name(r.head().rel());
        let local: BTreeSet<&str> = rules.iter().map(head).filter(|h| file_local.contains(*h) || constants.contains(*h)).collect();
        let targets: BTreeSet<String> = exports
            .iter()
            .map(String::as_str)
            .chain(f.outputs().filter(|o| file_local.contains(*o) && !f.qualified(o, crate::code_rule_dsl::logic::plan::fields::PART_OUT)))
            .filter(|r| local.contains(r) && locations.get(*r).is_none_or(Vec::is_empty))
            .map(str::to_owned)
            .collect();
        let docof = f.is_input("docof");
        let node_declared = f.declared("node");
        let node_typed = |rel: &str, c: usize| f.column_type(rel, c) == Some("node");
        let doc_type = f.column_type("node", 0).unwrap_or("int");
        // How many rules of a file-local head read each relation (not their own): a target read so is blocked.
        let reads = |r: &RuleRef<'_>| -> BTreeSet<&str> { r.body().filter_map(|l| l.atom()).map(|a| v.rel_name(a.rel())).filter(|a| *a != head(r)).collect() };
        let mut readers: HashMap<&str, usize> = HashMap::new();
        for r in rules.iter().filter(|r| local.contains(head(r))) {
            for a in reads(r) {
                *readers.entry(a).or_default() += 1;
            }
        }
        let mut items: Vec<Item<'_>> = rules.iter().map(|&r| Item::Keep(r)).collect();
        let mut ats: Vec<(String, Vec<(String, String)>)> = Vec::new();
        let mut names: HashSet<String> = f.decls().iter().map(|d| v.text(d.name()).to_owned()).collect();
        for rel in &targets {
            let read_locally = readers.get(rel.as_str()).is_some_and(|&n| n > 0) || f.qualified(rel, crate::code_rule_dsl::logic::plan::fields::PART_LOCAL_READ);
            let Some(cols) = f.columns(rel).filter(|_| !read_locally && !f.lattice(rel)) else {
                self.blocked.push(rel.clone());
                continue;
            };
            let mut at = format!("{rel}_at");
            while names.contains(&at) {
                at.push('_');
            }
            let of_rel: Vec<RuleRef<'_>> = items.iter().filter_map(|i| if let Item::Keep(r) = i { Some(*r) } else { None }).filter(|r| head(r) == rel).collect();
            let mut made: Vec<(RuleRef<'_>, Doc)> = Vec::new();
            for &rule in &of_rel {
                let positive = || {
                    rule.body().enumerate().filter_map(|(i, l)| match l.view() {
                        L::Atom(a) if !a.neg() => Some((i, a)),
                        _ => None,
                    })
                };
                let named = positive().find_map(|(_, a)| match super::document_column(v.rel_name(a.rel())).and_then(|c| a.args().nth(c)) {
                    Some(TermRef::Var(s)) => Some(s),
                    _ => None,
                });
                let unnamed = positive().find_map(|(i, a)| super::document_column(v.rel_name(a.rel())).filter(|&c| matches!(a.args().nth(c), Some(TermRef::Wild))).map(|c| (i, c)));
                let doc = match (named, unnamed) {
                    (Some(s), _) => Doc::Var(s),
                    (None, Some((i, c))) => Doc::Unnamed(i, c),
                    (None, None) => {
                        let node = positive().find_map(|(_, a)| a.args().enumerate().find_map(|(c, t)| t.var().filter(|_| node_typed(v.rel_name(a.rel()), c))));
                        match node.filter(|_| docof || node_declared) {
                            Some(n) => Doc::Through(n, docof),
                            None => break,
                        }
                    }
                };
                made.push((rule, doc));
            }
            if made.len() != of_rel.len() {
                self.blocked.push(rel.clone());
                continue;
            }
            let mut at_cols = vec![("d".to_owned(), doc_type.to_owned())];
            at_cols.extend(cols.iter().map(|(n, t)| ((*n).to_owned(), (*t).to_owned())));
            names.insert(at.clone());
            let k = ats.len();
            ats.push((at, at_cols));
            for r in &of_rel {
                for a in reads(r) {
                    if let Some(n) = readers.get_mut(a) {
                        *n -= 1;
                    }
                }
            }
            items.retain(|i| !matches!(i, Item::Keep(r) if head(r) == rel));
            items.extend(made.into_iter().map(|(rule, doc)| Item::Made { rule, at: k, doc }));
            items.push(Item::ReadAt { rel: rel.clone(), at: k, width: cols.len() });
            self.located.push(rel.clone());
        }
        if self.located.is_empty() {
            return Ok(());
        }
        let input = w.input().clone();
        let b = w.derive()?;
        for (at, cols) in &ats {
            let cols: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
            b.fields().declare(at, &cols);
        }
        const D: &str = "__located_d";
        for item in items {
            match item {
                Item::Keep(r) => {
                    b.keep(&input, r);
                }
                Item::Made { rule, at, doc } => {
                    let mut args: Vec<T<'_>> = vec![match doc {
                        Doc::Var(s) => T::VarAtom(rule.var(s)),
                        Doc::Unnamed(..) | Doc::Through(..) => T::Var(D),
                    }];
                    args.extend(rule.head().args().map(|t| t_of(&input, rule, t)));
                    let mut rb = b.rule().source(rule.id()).head(ats[at].0.as_str(), &args);
                    for (i, l) in rule.body().enumerate() {
                        rb = match (doc, l.view()) {
                            (Doc::Unnamed(j, c), L::Atom(a)) if j == i => {
                                let args: Vec<T<'_>> = a.args().enumerate().map(|(k, t)| if k == c { T::Var(D) } else { t_of(&input, rule, t) }).collect();
                                rb.atom_with(a.rel(), &args, a.neg(), a.gated())
                            }
                            _ => rb.lit(&input, rule, l),
                        };
                    }
                    if let Doc::Through(n, docof) = doc {
                        let n = T::VarAtom(rule.var(n));
                        rb = if docof { rb.atom("docof", &[n, T::Var(D)]) } else { rb.atom("node", &[T::Var(D), n, T::Wild, T::Wild, T::Wild]) };
                    }
                    rb.finish()?;
                }
                Item::ReadAt { rel, at, width } => {
                    let cols: Vec<String> = (0..width).map(|i| format!("__c{i}")).collect();
                    let vars: Vec<T<'_>> = cols.iter().map(|c| T::Var(c.as_str())).collect();
                    let mut args = vec![T::Wild];
                    args.extend(vars.iter().cloned());
                    b.rule().head(rel.as_str(), &vars).atom(ats[at].0.as_str(), &args).finish()?;
                }
            }
        }
        Ok(())
    }
}

// ---- rules a pass makes from its input's, written into a derivation (`FactorizeExports`, `PushLocalJoins`) ----

/// A body item: a literal of the source rule, or an atom made (`rel`, args).
#[derive(Clone)]
pub(super) enum BodyItem<'v> {
    Orig(LitRef<'v>),
    New(String, Vec<NOwned<'v>>),
    /// A negated atom made (a cached product's parts read while its pairs are not kept, decision 0506).
    Not(String, Vec<NOwned<'v>>),
}

/// A term of a rule being made: one of its source rule's, or a variable by name.
#[derive(Clone)]
pub(super) enum NOwned<'v> {
    T(TermRef<'v>),
    V(String),
    /// A constant.
    C(i64),
}

/// A rule of the round's program: its source rule, its head (the source's, or one made), its body.
#[derive(Clone)]
pub(super) struct Made<'v> {
    pub(super) src: RuleRef<'v>,
    /// `None`: the source's own head.
    pub(super) head: Option<(String, Vec<NOwned<'v>>)>,
    pub(super) body: Vec<BodyItem<'v>>,
    /// Unchanged from its source (kept as it is).
    pub(super) kept: bool,
}

fn t_from<'b>(input: &'b Version, src: RuleRef<'b>, n: &'b NOwned<'b>) -> T<'b> {
    match n {
        NOwned::T(t) => t_of(input, src, *t),
        NOwned::V(s) => T::Var(s.as_str()),
        NOwned::C(n) => T::Int(*n),
    }
}

impl Made<'_> {
    /// The names of the variables the rule being made reads or writes.
    pub(super) fn names(&self, v: &Version) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let owned = |args: &[NOwned<'_>], out: &mut BTreeSet<String>| {
            for a in args {
                match a {
                    NOwned::T(t) => out.extend(term_vars(v, self.src, *t)),
                    NOwned::V(s) => drop(out.insert(s.clone())),
                    NOwned::C(_) => {}
                }
            }
        };
        match &self.head {
            None => self.src.head().args().for_each(|t| out.extend(term_vars(v, self.src, t))),
            Some((_, args)) => owned(args, &mut out),
        }
        for it in &self.body {
            match it {
                BodyItem::Orig(l) => out.extend(lit_vars(v, self.src, *l)),
                BodyItem::New(_, args) | BodyItem::Not(_, args) => owned(args, &mut out),
            }
        }
        out
    }
}

/// The round's program written: the declarations it states and drops, its rules, its keys' qualifiers.
pub(super) struct Round<'v> {
    pub(super) decls: Vec<(String, Vec<(String, String)>)>,
    pub(super) undeclared: Vec<String>,
    pub(super) rules: Vec<Made<'v>>,
    pub(super) no_inline: Vec<String>,
}

pub(super) fn write(b: &mut Builder<'_>, input: &Version, round: &Round<'_>) -> Result<(), String> {
    write_onto(b, input, round, false)
}

/// A round's program as an overlay of `input` holding only what the round changes (the rules it rewrites or drops
/// removed, the ones it makes added), and its split, carried from `input`'s across that edit: a round costs its
/// change, where a derivation of the whole program split it whole.
pub(super) fn candidate(input: &std::sync::Arc<Version>, round: &Round<'_>) -> Result<(std::sync::Arc<Version>, std::sync::Arc<FileLocal>), String> {
    let mut b = Builder::on(input);
    write_onto(&mut b, input, round, true)?;
    let candidate = b.finish_overlay()?;
    let split = crate::code_rule_dsl::logic::program_image::manager::carried::<FileLocal>(input, &candidate)?;
    Ok((candidate, split))
}

/// `write`, into a derivation of `input` (`onto` false: every rule written, in order), or onto an edit of it (`onto`:
/// the rules kept are there already, and the others are removed).
fn write_onto(b: &mut Builder<'_>, input: &Version, round: &Round<'_>, onto: bool) -> Result<(), String> {
    if onto {
        let kept: HashSet<u32> = round.rules.iter().filter(|m| m.kept).map(|m| m.src.id().0).collect();
        let gone: Vec<_> = input.rules().map(|r| r.id()).filter(|id| !kept.contains(&id.0)).collect();
        for id in gone {
            b.remove(id);
        }
    }
    for (rel, cols) in &round.decls {
        let cols: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
        b.fields().declare(rel, &cols);
    }
    for rel in &round.undeclared {
        b.fields().undeclare(rel);
    }
    for m in &round.rules {
        if m.kept {
            if !onto {
                b.keep(input, m.src);
            }
            continue;
        }
        let mut rb = b.rule().source(m.src.id());
        rb = match &m.head {
            None => rb.head_from(input, m.src, m.src.head()),
            Some((rel, args)) => {
                let args: Vec<T<'_>> = args.iter().map(|n| t_from(input, m.src, n)).collect();
                rb.head(rel.as_str(), &args)
            }
        };
        for it in &m.body {
            rb = match it {
                BodyItem::Orig(l) => rb.lit(input, m.src, *l),
                BodyItem::New(rel, args) => {
                    let args: Vec<T<'_>> = args.iter().map(|n| t_from(input, m.src, n)).collect();
                    rb.atom(rel.as_str(), &args)
                }
                BodyItem::Not(rel, args) => {
                    let args: Vec<T<'_>> = args.iter().map(|n| t_from(input, m.src, n)).collect();
                    rb.not(rel.as_str(), &args)
                }
            };
        }
        rb.finish()?;
    }
    if !round.no_inline.is_empty() {
        let words: Vec<(&str, &str)> = round.no_inline.iter().map(|k| (k.as_str(), "no_inline")).collect();
        b.fields().add_qualifiers(&words);
    }
    Ok(())
}

/// A variable of `rule` by its name (the passes read variables by name, so their sets and searches run in a fixed
/// order).
pub(super) fn name(v: &Version, rule: RuleRef<'_>, s: crate::code_rule_dsl::logic::program_image::Slot) -> String {
    v.text(rule.var(s)).to_owned()
}

pub(super) fn lit_vars(v: &Version, rule: RuleRef<'_>, l: LitRef<'_>) -> BTreeSet<String> {
    body::lit_vars(l).iter().map(|s| name(v, rule, s)).collect()
}

pub(super) fn term_vars(v: &Version, rule: RuleRef<'_>, t: TermRef<'_>) -> BTreeSet<String> {
    let mut out = body::VarSet::new();
    body::term_vars(t, &mut out);
    out.iter().map(|s| name(v, rule, s)).collect()
}
