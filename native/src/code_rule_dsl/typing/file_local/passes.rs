//! The split's preparation passes on the pass manager (decision 0281's path, step 5), each the tree pass of
//! `file_local` it replaces, one for one, reading the split from the version (`FileLocal`).

use super::FileLocal;
use crate::code_rule_dsl::logic::plan::fields::{Fields, FieldsData};
use crate::code_rule_dsl::logic::program_image::build::T;
use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::{Lit as L, RuleRef, Slot, TermRef, Version};
use std::collections::{BTreeSet, HashMap, HashSet};

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

/// `file_local::locate` on the version: every relation the chunks hand on (an export, or a file-local output) given
/// a location column, so each of its rows names its document (decision 0052). A relation without one, `r(p, l)`, is
/// derived by the chunks as `r_at(d, p, l)` (`r_at_` where the program has an `r_at` of its own), `d` a document its
/// rules bind, and the cross-file part reads `r(p, l) :- r_at(_, p, l)`. A relation that file-local rules also
/// read, a lattice, or one whose rule binds no location is left, and named in `blocked`.
///
/// One round of the tree's `locate` (its `locate_once`): a blocked relation the split still holds file-local is
/// qualified `CROSS_FILE` in the version this pass derives (`moving`), so the split moves it, and those reading it,
/// to the cross-file part (decision 0290). `locate_on` runs the rounds until none moves, as `locate` does.
#[derive(Default)]
pub struct Locate {
    pub located: Vec<String>,
    pub blocked: Vec<String>,
    /// The blocked relations this round qualifies `CROSS_FILE`: none already cross-file.
    pub moving: Vec<String>,
    /// Every relation qualified `CROSS_FILE` in the version this round read (`moved_in`).
    pub moved: Vec<String>,
}

impl Locate {
    /// This round's result: the relations located, or those left.
    pub fn result(&self) -> Result<Vec<String>, Vec<String>> {
        if self.blocked.is_empty() { Ok(self.located.clone()) } else { Err(self.blocked.clone()) }
    }
}

/// The tree's `locate` on the pipeline's version: `Locate` rounds, each moving the relations the last left to the
/// cross-file part, until a round leaves none (`Ok`: every relation located, over the rounds) or moves none (`Err`:
/// the relations moved, which then cannot be placed).
pub fn locate_on(p: &mut crate::code_rule_dsl::logic::program_image::manager::Pipeline) -> Result<Vec<String>, Vec<String>> {
    let mut located = Vec::new();
    loop {
        let mut round = Locate::default();
        p.program(&mut round);
        located.append(&mut round.located);
        if round.blocked.is_empty() {
            return Ok(located);
        }
        if round.moving.is_empty() {
            return Err(round.moved);
        }
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
            .chain(f.outputs().filter(|o| file_local.contains(*o)))
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
            let read_locally = readers.get(rel.as_str()).is_some_and(|&n| n > 0);
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
        // A blocked relation still file-local moves to the cross-file part (0290), as the tree's `locate` moves it.
        self.moved = super::moved_in(&f).into_iter().map(str::to_owned).collect();
        self.moving = self.blocked.iter().filter(|r| !f.qualified(r, super::CROSS_FILE)).cloned().collect();
        if self.located.is_empty() && self.moving.is_empty() {
            return Ok(());
        }
        let input = w.input().clone();
        let b = w.derive()?;
        let moving: Vec<(&str, &str)> = self.moving.iter().map(|r| (r.as_str(), super::CROSS_FILE)).collect();
        b.add_qualifiers(&moving);
        for (at, cols) in &ats {
            let cols: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
            b.declare(at, &cols);
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
