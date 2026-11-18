//! `hiding::hide` on the version (decision 0058 §1, 0281's path, step 7a-1): the program's unscoped location
//! variables bounded to its shown documents, the relations read with a guest's node copied for its grammar, and the
//! kinds and fields of a guest's nodes named in its grammar. See `hiding` for what each part does and why: this is
//! that pass, round for round and rule for rule, over the version's rules.
//!
//! The rounds edit rules and add copies, each round reading the rules as it began. Each rule here is its source rule
//! in the input version and what the rounds changed of it (`W`: the relation each atom reads, a `nearest`'s kind
//! set, a kind name a comparison qualified, and for a copy its head); the version is written once at the end, each
//! rule copied from its source with those edits, the bounds the last stage adds, and the aggregates' helpers.

use super::super::hiding::{ENTRIES, Fact, VIEWS, dialect_label, qualified};
use crate::code_rule_dsl::logic::front::{self, Value};
use crate::code_rule_dsl::logic::program_image::fields::{Edit as Staged, FieldsMut, ProgramFields};
use crate::code_rule_dsl::logic::program_image::build::{Rewrite, Site, T};
use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::{AtomId, AtomRef, CmpOp, Lit, RuleId, Slot, TermRef, Version};
use rustc_hash::FxHashMap as HashMap;
use std::collections::{BTreeMap, BTreeSet};

type Result<T> = std::result::Result<T, String>;

/// A term as the analysis reads it.
#[derive(Clone, PartialEq)]
enum S {
    Var(Slot),
    Str(String),
    Wild,
    Other,
}

impl S {
    fn of(v: &Version, t: TermRef<'_>) -> S {
        match t {
            TermRef::Var(s) => S::Var(s),
            TermRef::Str(a) => S::Str(v.text(a).to_owned()),
            TermRef::Wild => S::Wild,
            _ => S::Other,
        }
    }
    fn var(&self) -> Option<Slot> {
        if let S::Var(s) = self { Some(*s) } else { None }
    }
}

/// An atom: its node in the source rule, the relation it reads now, and its arguments.
#[derive(Clone)]
struct A {
    node: u32,
    rel: String,
    neg: bool,
    args: Vec<S>,
}

#[derive(Clone)]
enum L {
    Atom(A),
    Agg(A),
    Nearest { n: Slot, a: Slot, set: String },
    /// A comparison: whether it is `=`, and its sides.
    Cmp { eq: bool, l: S, r: S },
    Other,
}

impl L {
    fn atom(&self) -> Option<&A> {
        match self {
            L::Atom(a) | L::Agg(a) => Some(a),
            _ => None,
        }
    }
    fn positive(&self) -> Option<&A> {
        match self {
            L::Atom(a) if !a.neg => Some(a),
            _ => None,
        }
    }
}

/// A rule as the rounds left it: its source rule, its head's relation, and its body, by literal node.
#[derive(Clone)]
struct W {
    src: RuleId,
    head: String,
    head_args: Vec<S>,
    body: Vec<(u32, L)>,
}

impl W {
    fn of(v: &Version, r: crate::code_rule_dsl::logic::program_image::RuleRef<'_>) -> W {
        let atom = |a: AtomRef<'_>| A { node: a.node(), rel: v.rel_name(a.rel()).to_owned(), neg: a.neg(), args: a.args().map(|t| S::of(v, t)).collect() };
        let body = r
            .body()
            .map(|l| {
                (l.node(), match l.view() {
                    Lit::Atom(a) => L::Atom(atom(a)),
                    Lit::Agg { atom: a, .. } => L::Agg(atom(a)),
                    Lit::Nearest { n, a, kindset } => L::Nearest { n, a, set: v.text(kindset).to_owned() },
                    Lit::Cmp { l, op, r } => L::Cmp { eq: op == CmpOp::Eq, l: S::of(v, l), r: S::of(v, r) },
                    Lit::Ast { .. } => L::Other,
                })
            })
            .collect();
        W { src: r.id(), head: v.rel_name(r.head().rel()).to_owned(), head_args: r.head().args().map(|t| S::of(v, t)).collect(), body }
    }
}

type Facts = BTreeMap<Slot, Fact>;

/// `hiding::hide` on the version, with `bound` (`hiding::bound`).
pub(in crate::code_rule_dsl::logic::plan::corpus) struct Hide {
    pub(in crate::code_rule_dsl::logic::plan::corpus) bound: u32,
}

/// A round's edit of one literal (by its place in the rule), applied once the round has read every rule.
enum Edit {
    Kindset(String),
    Rel(String),
    Str(String, String),
}

impl ProgramPass for Hide {
    const NAME: &'static str = "hide";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        let v = cx.v();
        // The fields' edits, staged on the input (read through them) and stated on the derivation at the end; the
        // kind sets apart, stated once.
        let input = std::sync::Arc::clone(w.input());
        let mut prog = Staged::on(&input);
        let mut all_kindsets = v.kindsets();
        let mut rules: Vec<W> = v.rules().map(|r| W::of(v, r)).collect();
        let inputs = prog.input_names();
        let views: BTreeSet<String> = VIEWS.iter().filter(|r| !inputs.iter().any(|i| i == *r)).map(|r| (*r).to_owned()).collect();
        let view = |rel: &str| views.contains(rel);
        let base: BTreeSet<String> = inputs.iter().cloned().chain(VIEWS.iter().filter(|r| view(r)).map(|r| (*r).to_owned())).filter(|r| !ENTRIES.iter().any(|(e, ..)| e == r)).collect();
        let locations = |rel: &str| -> Vec<usize> {
            let mut cols: Vec<usize> = prog.columns_of(rel).map_or_else(Vec::new, |c| c.iter().enumerate().filter(|(_, (_, t))| t == "node").map(|(i, _)| i).collect());
            if let Some(c) = crate::code_rule_dsl::typing::file_local::document_column(rel)
                && !cols.contains(&c)
            {
                cols.push(c);
            }
            cols
        };
        let base_locations: HashMap<String, Vec<usize>> = base.iter().map(|r| (r.clone(), locations(r))).collect();
        let mut seeds: Vec<Facts> = vec![Facts::new(); rules.len()];
        let mut made: BTreeSet<String> = BTreeSet::new();
        let mut specialized: BTreeMap<String, (String, BTreeMap<usize, String>)> = BTreeMap::new();
        let mut split: BTreeSet<usize> = BTreeSet::new();
        let mut delta = Delta::new(&rules);
        let mut touched: Vec<usize> = (0..rules.len()).collect();
        loop {
            let _round = crate::code_rule_dsl::logic::profile::span(|| "hiding: a round".to_owned());
            let looked = delta.round(&rules, &prog.b, &seeds, &base_locations, &view, &touched);
            let reaches = &delta.reaches;
            let carried = &delta.carried;
            let mut edits: Vec<(usize, usize, Edit)> = Vec::new();
            let ruled = &delta.heads;
            let mut copies: Vec<(W, Facts)> = Vec::new();
            let mut decls: Vec<(String, Vec<(String, String)>)> = Vec::new();
            let mut axes: Vec<String> = Vec::new();
            let mut kindsets: Vec<(String, Vec<String>)> = Vec::new();
            let mut kind_facts: Vec<(String, String, Vec<String>)> = Vec::new();
            let mut changed = false;
            let analysed: Vec<Facts> = looked
                .iter()
                .map(|&i| {
                    let rule = &rules[i];
                    if view(&rule.head) || !reaches[i] {
                        return Facts::new();
                    }
                    analyse(rule, &base_locations, carried, &seeds[i])
                })
                .collect();
            for (i, facts) in looked.iter().copied().zip(analysed) {
                let rule = &rules[i];
                if view(&rule.head) {
                    continue;
                }
                let maybe: Facts = facts
                    .iter()
                    .filter(|(_, f)| f.scoped && !f.definite && f.grammar.as_ref().is_some_and(|g| !g.is_empty()))
                    .map(|(v, f)| (*v, Fact { definite: true, ..f.clone() }))
                    .collect();
                if !maybe.is_empty() && split.insert(i) {
                    let mut seed = seeds[i].clone();
                    seed.extend(maybe);
                    copies.push((rule.clone(), seed));
                    changed = true;
                }
                let grammar_of = |s: Slot| facts.get(&s).filter(|f| f.scoped && f.definite).and_then(|f| f.grammar.clone()).filter(|g| !g.is_empty());
                let kind_of: BTreeMap<Slot, String> = rule
                    .body
                    .iter()
                    .filter_map(|(_, l)| {
                        let a = l.positive()?;
                        match (a.rel.as_str(), a.args.as_slice()) {
                            ("kind", [S::Var(x), S::Var(k)]) | ("node", [_, S::Var(x), S::Var(k), _, _]) => grammar_of(*x).map(|g| (*k, g)),
                            _ => None,
                        }
                    })
                    .collect();
                for (k, (_, l)) in rule.body.iter().enumerate() {
                    if !kind_of.is_empty() {
                        match l {
                            L::Cmp { l: S::Var(x), r: S::Str(s), .. } | L::Cmp { l: S::Str(s), r: S::Var(x), .. } if qualified(s).is_none() && kind_of.contains_key(x) => {
                                edits.push((i, k, Edit::Str(s.clone(), format!("{}:{s}", dialect_label(&kind_of[x])))));
                                changed = true;
                                continue;
                            }
                            // Negated as well, as the tree's (`hiding.rs`): `kind(s, k), !def_or_class(k)` over a guest's node
                            // reads the set's qualified copy; the bare set holds none of the guest's kinds.
                            L::Atom(a)
                                if !a.rel.starts_with("rr_in_")
                                    && !ruled.contains_key(a.rel.as_str())
                                    && let [S::Var(x)] = a.args.as_slice()
                                    && let Some(g) = kind_of.get(x)
                                    && let Some(rows) = prog.states_facts(a.rel.as_str()).then(|| prog.fact_rows(a.rel.as_str())) =>
                            {
                                let name = format!("rr_in_{}_rows_{}", g.replace(|ch: char| !ch.is_ascii_alphanumeric(), "_"), a.rel);
                                if made.insert(name.clone()) {
                                    let label = dialect_label(g);
                                    let names = rows.iter().filter_map(|row| match row.as_slice() {
                                        [Value::Str(k)] => Some(k.as_str()),
                                        _ => None,
                                    });
                                    let kinds: Vec<String> = names.map(|k| if qualified(k).is_some() { k.to_owned() } else { format!("{label}:{k}") }).collect();
                                    kind_facts.push((name.clone(), a.rel.clone(), kinds));
                                }
                                edits.push((i, k, Edit::Rel(name)));
                                changed = true;
                                continue;
                            }
                            _ => {}
                        }
                    }
                    if let L::Nearest { n, set, .. } = l {
                        if let Some(g) = grammar_of(*n)
                            && !set.starts_with("rr_in_")
                        {
                            let name = format!("rr_in_{g}_{set}");
                            if made.insert(name.clone()) {
                                let label = dialect_label(&g);
                                let kinds = all_kindsets.get(set.as_str()).cloned().unwrap_or_default();
                                kindsets.push((name.clone(), kinds.into_iter().map(|k| if qualified(&k).is_some() { k } else { format!("{label}:{k}") }).collect()));
                            }
                            edits.push((i, k, Edit::Kindset(name)));
                            changed = true;
                        }
                        continue;
                    }
                    if let Some(a) = l.atom()
                        && let Some((set, descendants)) = front::axis(&a.rel)
                        && !set.starts_with("rr_in_")
                        && all_kindsets.contains_key(set)
                        && let Some(g) = a.args.iter().find_map(|t| t.var().and_then(grammar_of))
                    {
                        let copy = format!("rr_in_{}_{set}", g.replace(|ch: char| !ch.is_ascii_alphanumeric(), "_"));
                        let prefix = if descendants { front::DESCENDANTS } else { front::AXIS };
                        let name = format!("{prefix}{copy}");
                        if made.insert(name.clone()) {
                            let label = dialect_label(&g);
                            let kinds = all_kindsets.get(set).cloned().unwrap_or_default();
                            kindsets.push((copy, kinds.into_iter().map(|k| if qualified(&k).is_some() { k } else { format!("{label}:{k}") }).collect()));
                            if let Some(d) = prog.columns_of(a.rel.as_str()) {
                                decls.push((name.clone(), d));
                            }
                            axes.push(name.clone());
                        }
                        edits.push((i, k, Edit::Rel(name)));
                        changed = true;
                        continue;
                    }
                    let Some(a) = l.atom() else { continue };
                    let Some(cols) = carried.get(a.rel.as_str()) else { continue };
                    let spec = a.args.iter().enumerate().find_map(|(c, t)| {
                        let g = grammar_of(t.var()?)?;
                        (!cols.get(c).is_some_and(|f| f.scoped && f.grammar.as_deref() == Some(g.as_str()))).then_some((c, g))
                    });
                    let Some((c, g)) = spec else { continue };
                    let (orig, mut cols) = specialized.get(a.rel.as_str()).cloned().unwrap_or_else(|| (a.rel.clone(), BTreeMap::new()));
                    cols.insert(c, g);
                    let name = format!("rr_in_{}{orig}", cols.iter().map(|(c, g)| format!("{}_{c}_", g.replace(|ch: char| !ch.is_ascii_alphanumeric(), "_"))).collect::<String>());
                    if made.insert(name.clone()) {
                        for &j in delta.heads.get(orig.as_str()).into_iter().flatten() {
                            let mut copy = rules[j].clone();
                            copy.head = name.clone();
                            let mut seed = Facts::new();
                            for (c, g) in &cols {
                                if let Some(S::Var(x)) = copy.head_args.get(*c) {
                                    seed.insert(*x, Fact { scoped: true, grammar: Some(g.clone()), definite: true });
                                }
                            }
                            copies.push((copy, seed));
                        }
                        if let Some(d) = prog.columns_of(orig.as_str()) {
                            decls.push((name.clone(), d));
                        }
                        specialized.insert(name.clone(), (orig, cols));
                    }
                    if a.rel != name {
                        edits.push((i, k, Edit::Rel(name)));
                        changed = true;
                    }
                }
            }
            let edited: BTreeSet<usize> = edits.iter().map(|(i, _, _)| *i).collect();
            for (i, k, edit) in edits {
                match (&mut rules[i].body[k].1, edit) {
                    (L::Nearest { set, .. }, Edit::Kindset(name)) => *set = name,
                    (L::Atom(a) | L::Agg(a), Edit::Rel(name)) => a.rel = name,
                    (L::Cmp { l, r, .. }, Edit::Str(from, to)) => {
                        for t in [l, r] {
                            if matches!(t, S::Str(s) if *s == from) {
                                *t = S::Str(to.clone());
                            }
                        }
                    }
                    _ => unreachable!("an edit is of the literal it was recorded for"),
                }
            }
            let first_copy = rules.len();
            for (rule, seed) in copies {
                rules.push(rule);
                seeds.push(seed);
            }
            touched = edited.into_iter().chain(first_copy..rules.len()).collect();
            delta.changed(&rules, &touched);
            for (name, cols) in decls {
                declare_columns(&mut prog.b, &name, &cols);
            }
            all_kindsets.extend(kindsets);
            for (name, orig, kinds) in kind_facts {
                if let Some(d) = prog.columns_of(orig.as_str()) {
                    declare_columns(&mut prog.b, &name, &d);
                }
                let rows: Vec<front::List<Value>> = kinds.iter().map(|k| [Value::Str(k.as_str().into())].into()).collect();
                prog.b.set_facts(&name, &rows);
            }
            for axis in axes {
                prog.b.add_input(&axis);
            }
            if !changed {
                break;
            }
        }
        // Each rule bounded on its own (`hiding::hide`'s last stage), then the aggregates' helpers.
        let bounding = crate::code_rule_dsl::logic::profile::span(|| "hiding: each rule bounded".to_owned());
        let carried = std::mem::take(&mut delta.carried);
        drop(delta);
        let bound = self.bound;
        let bound_of = |doc_column: bool| if doc_column { i64::from(bound) } else { i64::from(bound) << 32 };
        let reaches = reaching(&rules, &seeds);
        let asking = asking(&prog, &rules);
        let mut finals: Vec<Final> = vec![Final::default(); rules.len()];
        for (i, rule) in rules.iter().enumerate() {
            if VIEWS.contains(&rule.head.as_str()) || asking.contains(&rule.head) {
                continue;
            }
            let src = v.rule(rule.src).expect("a rule of the input");
            let name = |s: Slot| v.text(src.var(s)).to_owned();
            let facts: Facts = if reaches[i] { analyse(rule, &base_locations, &carried, &seeds[i]) } else { Facts::new() };
            let fin = &mut finals[i];
            let mut fresh = 0;
            let mut bounded: BTreeMap<String, bool> = BTreeMap::new();
            for (_, l) in &rule.body {
                let L::Atom(a) = l else { continue };
                let Some(cols) = base_locations.get(a.rel.as_str()) else { continue };
                if a.neg {
                    let grammar = cols.iter().find_map(|&c| a.args.get(c).and_then(S::var).and_then(|x| facts.get(&x)).filter(|f| f.scoped && f.definite).and_then(|f| f.grammar.clone()).filter(|g| !g.is_empty()));
                    if let Some(g) = grammar {
                        fin.qualify(a, dialect_label(&g));
                    }
                    continue;
                }
                let doc_col = crate::code_rule_dsl::typing::file_local::document_column(&a.rel);
                let atom_scoped = cols.iter().any(|&c| a.args.get(c).and_then(S::var).is_some_and(|x| facts.get(&x).is_some_and(|f| f.scoped)));
                if atom_scoped {
                    let grammar = cols.iter().find_map(|&c| a.args.get(c).and_then(S::var).and_then(|x| facts.get(&x)).filter(|f| f.definite).and_then(|f| f.grammar.clone()).filter(|g| !g.is_empty()));
                    if let Some(g) = grammar {
                        fin.qualify(a, dialect_label(&g));
                    }
                    continue;
                }
                for &c in cols {
                    match a.args.get(c) {
                        Some(S::Var(x)) => {
                            bounded.entry(name(*x)).or_insert(Some(c) == doc_col);
                        }
                        Some(S::Wild) => {
                            let x = format!("rr_shown{fresh}");
                            fresh += 1;
                            fin.args.insert((a.node, c), x.clone());
                            bounded.insert(x, Some(c) == doc_col);
                        }
                        _ => {}
                    }
                }
            }
            fin.bounds = bounded.into_iter().map(|(x, doc)| (x, bound_of(doc))).collect();
        }
        drop(bounding);
        // An aggregate's atom is its own scope: one over a base atom reads it through a bounded helper.
        let mut helpers: Vec<Helper> = Vec::new();
        for (i, rule) in rules.iter().enumerate() {
            if VIEWS.contains(&rule.head.as_str()) {
                continue;
            }
            let src = v.rule(rule.src).expect("a rule of the input");
            for (_, l) in &rule.body {
                let L::Agg(atom) = l else { continue };
                let Some(cols) = base_locations.get(atom.rel.as_str()) else { continue };
                if cols.is_empty() {
                    continue;
                }
                let decl = prog.columns_of(atom.rel.as_str()).unwrap_or_default();
                let doc_col = crate::code_rule_dsl::typing::file_local::document_column(&atom.rel);
                // Every wildcard a column of the helper, named; the other arguments the atom's own.
                let inner: Vec<Option<String>> = atom
                    .args
                    .iter()
                    .enumerate()
                    .map(|(c, t)| match t {
                        S::Wild => Some(format!("rr_shown_agg_c{c}")),
                        S::Var(x) => Some(v.text(src.var(*x)).to_owned()),
                        _ => None,
                    })
                    .collect();
                let bounds: Vec<(String, i64)> = cols.iter().filter_map(|&c| inner.get(c).cloned().flatten().map(|x| (x, bound_of(Some(c) == doc_col)))).collect();
                let mut vars: Vec<(String, String)> = Vec::new();
                for (c, x) in inner.iter().enumerate() {
                    if let Some(x) = x
                        && !vars.iter().any(|(y, _)| y == x)
                    {
                        vars.push((x.clone(), decl.get(c).map_or_else(|| "int".to_owned(), |(_, ty)| ty.to_string())));
                    }
                }
                let name = format!("rr_shown_agg{}", helpers.len());
                let outer: Vec<Option<String>> = vars.iter().map(|(x, _)| (!x.starts_with("rr_shown_agg_c")).then(|| x.clone())).collect();
                finals[i].aggs.insert(atom.node, (name.clone(), outer));
                helpers.push(Helper { name, vars, src: rule.src, atom: atom.node, rel: atom.rel.clone(), inner, bounds });
            }
        }
        for h in &helpers {
            let cols: Vec<(&str, &str)> = h.vars.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
            prog.b.declare(&h.name, &cols);
        }
        // The version: every rule copied from its source with its edits, then the helpers, then the fields.
        let b = w.derive()?;
        b.state_fields_of(&prog.b);
        if all_kindsets != v.kindsets() {
            b.set_field(crate::code_rule_dsl::logic::program_facts::Field::Kindsets(&all_kindsets));
        }
        let asked = asked_after(v, &rules);
        for (rule, fin) in rules.iter().zip(&finals) {
            let src = v.rule(rule.src).expect("a rule of the input");
            let mut rw = Writing::new(b, v, rule, fin);
            let bounds: Vec<(AtomId, i64)> = fin.bounds.iter().map(|(x, n)| (b.atom(x), *n)).collect();
            let mut out = b.rule().source(rule.src);
            out = out.head_with(v, src, src.head(), &mut rw);
            for l in src.body() {
                rw.cur = l.node();
                out = out.lit_with(v, src, l, &mut rw);
            }
            for (x, n) in bounds {
                out = out.cmp(T::VarAtom(x), CmpOp::Lt, T::Int(n));
            }
            out.finish()?;
        }
        for h in &helpers {
            let src = v.rule(h.src).expect("a rule of the input");
            let atom = src.body().find_map(|l| match l.view() {
                Lit::Agg { atom, .. } if atom.node() == h.atom => Some(atom),
                _ => None,
            }).expect("the aggregate's atom");
            let head: Vec<T<'static>> = h.vars.iter().map(|(x, _)| T::VarAtom(b.atom(x))).collect();
            let args: Vec<T<'static>> = atom
                .args()
                .zip(&h.inner)
                .map(|(t, x)| match x {
                    Some(x) => T::VarAtom(b.atom(x)),
                    None => super::simplify::Tm::of(v, src, t).write(b),
                })
                .collect();
            let bounds: Vec<(AtomId, i64)> = h.bounds.iter().map(|(x, n)| (b.atom(x), *n)).collect();
            let mut out = b.rule().head(h.name.as_str(), &head).atom_with(h.rel.as_str(), &args, atom.neg(), false);
            for (x, n) in bounds {
                out = out.cmp(T::VarAtom(x), CmpOp::Lt, T::Int(n));
            }
            out.finish()?;
        }
        if let Some(asked) = asked {
            b.set_asked(&asked);
        }
        Ok(())
    }
}

/// The request record (decision 0331) as the tree's hiding leaves it (`Program::push_rule_from`): a request's rule,
/// copied or edited, serves the readers its source served; a copy of a reader under another name reads what the
/// reader read, so it is a reader wherever its source is. `None` where the input has no record.
fn asked_after(v: &Version, rules: &[W]) -> Option<crate::code_rule_dsl::logic::program_image::about::Asked> {
    let was = v.asked();
    if was.is_empty() {
        return None;
    }
    let mut by: HashMap<RuleId, BTreeSet<String>> = HashMap::default();
    for (req, readers) in &was {
        let Some(rel) = v.rel(req) else { continue };
        let ids: Vec<RuleId> = v.heads(rel).map(|r| r.id()).collect();
        if ids.len() == readers.len() {
            by.extend(ids.into_iter().zip(readers.iter().cloned()));
        }
    }
    let mut out: crate::code_rule_dsl::logic::program_image::about::Asked = was.keys().map(|req| (req.clone(), Vec::new())).collect();
    for rule in rules {
        let src = v.rel_name(v.rule(rule.src).expect("a rule of the input").head().rel());
        if let Some(readers) = out.get_mut(&rule.head) {
            let serves = by.get(&rule.src).filter(|_| src == rule.head).cloned().unwrap_or_else(|| BTreeSet::from([rule.head.clone()]));
            readers.push(serves);
        }
        if src != rule.head {
            for readers in out.values_mut().flatten().chain(by.values_mut()).filter(|r| r.contains(src)) {
                readers.insert(rule.head.clone());
            }
        }
    }
    out.retain(|_, by| !by.is_empty());
    Some(out)
}

/// What the last stage adds to a rule: kind or field names qualified, wildcards named, the bounds, and the aggregates
/// read through their helpers.
#[derive(Clone, Default)]
struct Final {
    /// By (atom node, column): a string qualified, or a wildcard's new variable.
    strs: HashMap<(u32, usize), String>,
    args: HashMap<(u32, usize), String>,
    bounds: Vec<(String, i64)>,
    /// By aggregate atom node: the helper, and its arguments (a variable's name, or a wildcard).
    aggs: HashMap<u32, (String, Vec<Option<String>>)>,
}

impl Final {
    /// `hiding::qualify`: a kind or field constant of `a` named in `label`'s grammar, unless it names one already.
    fn qualify(&mut self, a: &A, label: &str) {
        let col = match a.rel.as_str() {
            "kind" => Some(1),
            "node" => Some(2),
            "field" => Some(1),
            _ => None,
        };
        if let Some(c) = col
            && let Some(S::Str(s)) = a.args.get(c)
            && qualified(s).is_none()
        {
            self.strs.insert((a.node, c), format!("{label}:{s}"));
        }
    }
}

/// An aggregate's helper: `name(vars) :- rel(inner), bounds`, its atom the aggregate's (`src`'s, at node `atom`).
struct Helper {
    name: String,
    vars: Vec<(String, String)>,
    src: RuleId,
    atom: u32,
    rel: String,
    inner: Vec<Option<String>>,
    bounds: Vec<(String, i64)>,
}

/// The copy of one rule from its source: its head, each atom's relation, each `nearest`'s kind set, each
/// comparison's kind name, and the last stage's additions.
struct Writing<'a> {
    v: &'a Version,
    rule: &'a W,
    /// The literal being written (its node).
    cur: u32,
    atoms: HashMap<u32, &'a A>,
    sets: HashMap<u32, &'a str>,
    /// By literal node: a comparison's strings, (as written, now), interned.
    cmps: HashMap<u32, Vec<(String, AtomId)>>,
    strs: HashMap<(u32, usize), AtomId>,
    args: HashMap<(u32, usize), AtomId>,
    aggs: HashMap<u32, (String, Vec<T<'static>>)>,
}

impl<'a> Writing<'a> {
    fn new(b: &mut crate::code_rule_dsl::logic::program_image::Builder<'_>, v: &'a Version, rule: &'a W, fin: &'a Final) -> Writing<'a> {
        let src = v.rule(rule.src).expect("a rule of the input");
        let mut atoms = HashMap::default();
        let mut sets = HashMap::default();
        let mut cmps: HashMap<u32, Vec<(String, AtomId)>> = HashMap::default();
        let was: HashMap<u32, L> = W::of(v, src).body.into_iter().collect();
        for (node, l) in &rule.body {
            match l {
                L::Atom(a) | L::Agg(a) => {
                    atoms.insert(a.node, a);
                }
                L::Nearest { set, .. } => {
                    sets.insert(*node, set.as_str());
                }
                L::Cmp { l, r, .. } => {
                    if let Some(L::Cmp { l: l0, r: r0, .. }) = was.get(node) {
                        for (now, then) in [(l, l0), (r, r0)] {
                            if let (S::Str(now), S::Str(then)) = (now, then)
                                && now != then
                            {
                                cmps.entry(*node).or_default().push((then.clone(), b.atom(now)));
                            }
                        }
                    }
                }
                L::Other => {}
            }
        }
        let strs = fin.strs.iter().map(|(k, s)| (*k, b.atom(s))).collect();
        let args = fin.args.iter().map(|(k, s)| (*k, b.atom(s))).collect();
        let aggs = fin.aggs.iter().map(|(k, (name, outer))| (*k, (name.clone(), outer.iter().map(|x| x.as_ref().map_or(T::Wild, |x| T::VarAtom(b.atom(x)))).collect()))).collect();
        Writing { v, rule, cur: 0, atoms, sets, cmps, strs, args, aggs }
    }
}

impl Rewrite for Writing<'_> {
    fn head_as(&mut self, _a: AtomRef<'_>) -> Option<String> {
        Some(self.rule.head.clone())
    }
    fn atom_as(&mut self, a: AtomRef<'_>) -> Option<(String, bool, bool)> {
        if let Some((name, _)) = self.aggs.get(&a.node()) {
            return Some((name.clone(), a.neg(), false));
        }
        self.atoms.get(&a.node()).map(|x| (x.rel.clone(), a.neg(), a.gated()))
    }
    fn prefix(&mut self, a: AtomRef<'_>, out: &mut Vec<T<'static>>) {
        if let Some((_, args)) = self.aggs.get(&a.node()) {
            out.extend(args.iter().cloned());
        }
    }
    fn drop_arg(&mut self, a: AtomRef<'_>, _c: usize) -> bool {
        self.aggs.contains_key(&a.node())
    }
    fn arg(&mut self, a: AtomRef<'_>, c: usize, _t: TermRef<'_>) -> Option<T<'static>> {
        let at = (a.node(), c);
        self.strs.get(&at).map(|s| T::StrAtom(*s)).or_else(|| self.args.get(&at).map(|x| T::VarAtom(*x)))
    }
    fn term(&mut self, site: Site<'_>, t: TermRef<'_>) -> Option<T<'static>> {
        let Site::Cmp(l) = site else { return None };
        let TermRef::Str(a) = t else { return None };
        // The side that named the kind as written (`hiding::hide`'s `Edit::Str`).
        let text = self.v.text(a);
        self.cmps.get(&l.node())?.iter().find(|(then, _)| then == text).map(|(_, now)| T::StrAtom(*now))
    }
    fn kindset(&mut self, _set: AtomId) -> Option<String> {
        self.sets.get(&self.cur).map(|s| (*s).to_owned())
    }
}

/// `hiding::reaching` over the rules here.
fn reaching(rules: &[W], seeds: &[Facts]) -> Vec<bool> {
    let mut readers: HashMap<&str, Vec<usize>> = HashMap::default();
    let mut out = vec![false; rules.len()];
    let mut work = Vec::new();
    for (i, rule) in rules.iter().enumerate() {
        let mut enters = seeds.get(i).is_some_and(|s| !s.is_empty());
        for (_, l) in &rule.body {
            if let Some(a) = l.positive() {
                enters |= ENTRIES.iter().any(|(e, ..)| *e == a.rel);
                readers.entry(a.rel.as_str()).or_default().push(i);
            }
        }
        if enters {
            out[i] = true;
            work.push(i);
        }
    }
    let mut made: BTreeSet<&str> = BTreeSet::new();
    while let Some(i) = work.pop() {
        let head = rules[i].head.as_str();
        if made.insert(head) {
            for &j in readers.get(head).into_iter().flatten() {
                if !out[j] {
                    out[j] = true;
                    work.push(j);
                }
            }
        }
    }
    out
}

/// `hiding::asking` over the rules here: the guest demand and the contexts only it reads, transitively.
fn asking(prog: &dyn ProgramFields, rules: &[W]) -> BTreeSet<String> {
    let declared = prog.declared_names();
    let context = |r: &str| {
        let own = r.strip_prefix('p').and_then(|t| t.find('_').filter(|&i| i > 0 && t[..i].bytes().all(|b| b.is_ascii_digit())).map(|i| &t[i + 1..]));
        own.unwrap_or(r).starts_with("rr_ctx")
    };
    let mut asking: BTreeSet<String> = BTreeSet::from([super::super::guests::DEMAND.to_owned()]);
    loop {
        let more: Vec<String> = declared
            .iter()
            .filter(|r| context(r) && !asking.contains(r.as_str()))
            .filter(|r| {
                let mut readers = rules.iter().filter(|x| x.body.iter().any(|(_, l)| l.atom().is_some_and(|a| a.rel == r.as_str()))).peekable();
                readers.peek().is_some() && readers.all(|x| asking.contains(x.head.as_str()))
            })
            .map(|r| r.to_string())
            .collect();
        if more.is_empty() {
            return asking;
        }
        asking.extend(more);
    }
}

/// `hiding::Delta` over the rules here.
struct Delta {
    heads: HashMap<String, Vec<usize>>,
    readers: HashMap<String, Vec<usize>>,
    any_readers: HashMap<String, Vec<usize>>,
    reaches: Vec<bool>,
    made: std::collections::HashSet<String>,
    gained: HashMap<String, Vec<Fact>>,
    carried: HashMap<String, Vec<Fact>>,
    listed: usize,
    first: bool,
}

impl Delta {
    fn new(rules: &[W]) -> Delta {
        let mut d = Delta { heads: HashMap::default(), readers: HashMap::default(), any_readers: HashMap::default(), reaches: Vec::new(), made: Default::default(), gained: HashMap::default(), carried: HashMap::default(), listed: 0, first: true };
        d.list(rules, &(0..rules.len()).collect::<Vec<_>>());
        d
    }

    fn list(&mut self, rules: &[W], which: &[usize]) {
        for &i in which {
            let rule = &rules[i];
            if i >= self.listed {
                self.heads.entry(rule.head.clone()).or_default().push(i);
            }
            for (_, l) in &rule.body {
                let Some(a) = l.atom() else { continue };
                let any = self.any_readers.entry(a.rel.clone()).or_default();
                if !any.contains(&i) {
                    any.push(i);
                }
                if let Some(a) = l.positive() {
                    let list = self.readers.entry(a.rel.clone()).or_default();
                    if !list.contains(&i) {
                        list.push(i);
                    }
                }
            }
        }
        self.listed = self.listed.max(rules.len());
    }

    fn changed(&mut self, rules: &[W], touched: &[usize]) {
        self.list(rules, touched);
    }

    fn round(&mut self, rules: &[W], prog: &dyn ProgramFields, seeds: &[Facts], base: &HashMap<String, Vec<usize>>, view: &dyn Fn(&str) -> bool, touched: &[usize]) -> Vec<usize> {
        let first = std::mem::replace(&mut self.first, false);
        self.reaches.resize(rules.len(), false);
        let mut newly: Vec<usize> = Vec::new();
        let mut work: Vec<usize> = Vec::new();
        for &i in touched {
            let rule = &rules[i];
            let enters = seeds.get(i).is_some_and(|s| !s.is_empty())
                || rule.body.iter().any(|(_, l)| l.positive().is_some_and(|a| ENTRIES.iter().any(|(e, ..)| *e == a.rel) || self.made.contains(a.rel.as_str())));
            if enters && !self.reaches[i] {
                self.reaches[i] = true;
                newly.push(i);
                work.push(i);
            }
        }
        while let Some(i) = work.pop() {
            let head = &rules[i].head;
            if self.made.insert(head.clone()) {
                for &j in self.readers.get(head.as_str()).into_iter().flatten() {
                    if !self.reaches[j] {
                        self.reaches[j] = true;
                        newly.push(j);
                        work.push(j);
                    }
                }
            }
        }
        for r in touched.iter().map(|&i| &rules[i]) {
            if !self.gained.contains_key(r.head.as_str()) && !view(&r.head) {
                let n = prog.columns_of(r.head.as_str()).map_or(0, |c| c.len());
                self.gained.insert(r.head.clone(), vec![Fact::default(); n]);
            }
        }
        let start: BTreeSet<usize> = touched.iter().chain(&newly).copied().collect();
        let mut grew: BTreeSet<String> = BTreeSet::new();
        fixpoint(rules, base, seeds, &self.reaches, &self.readers, &mut self.gained, start.iter().copied().collect(), &mut grew, &|facts, t, slot| match t {
            S::Var(x) => facts.get(x).is_some_and(|f| slot.join(f)),
            _ => false,
        });
        let mut cone: BTreeSet<String> = BTreeSet::new();
        if first {
            cone.extend(self.gained.keys().cloned());
        } else {
            let mut work: Vec<String> = start.iter().map(|&i| rules[i].head.clone()).chain(grew.iter().cloned()).filter(|r| self.gained.contains_key(r.as_str())).collect();
            while let Some(rel) = work.pop() {
                if !cone.insert(rel.clone()) {
                    continue;
                }
                for &j in self.readers.get(rel.as_str()).into_iter().flatten() {
                    let head = &rules[j].head;
                    if self.gained.contains_key(head.as_str()) && !cone.contains(head.as_str()) {
                        work.push(head.clone());
                    }
                }
            }
        }
        let mut before: HashMap<String, Vec<Fact>> = HashMap::default();
        for rel in &cone {
            let mut cols = self.gained[rel.as_str()].clone();
            for f in cols.iter_mut() {
                f.definite = f.scoped;
            }
            if let Some(old) = self.carried.insert(rel.clone(), cols) {
                before.insert(rel.clone(), old);
            }
        }
        let which: Vec<usize> = cone.iter().flat_map(|r| self.heads.get(r.as_str()).into_iter().flatten().copied()).collect::<BTreeSet<usize>>().into_iter().collect();
        let mut lost: BTreeSet<String> = BTreeSet::new();
        fixpoint(rules, base, seeds, &self.reaches, &self.readers, &mut self.carried, which, &mut lost, &|facts, t, slot| {
            let definite = matches!(t, S::Var(x) if facts.get(x).is_some_and(|f| f.definite));
            if slot.definite && !definite {
                slot.definite = false;
                true
            } else {
                false
            }
        });
        if first {
            return (0..rules.len()).collect();
        }
        let mut looked: BTreeSet<usize> = start;
        for rel in &cone {
            if before.get(rel.as_str()) != self.carried.get(rel.as_str()) {
                looked.extend(self.any_readers.get(rel.as_str()).into_iter().flatten().copied());
            }
        }
        looked.into_iter().collect()
    }
}

/// `hiding::fixpoint` over the rules here.
#[allow(clippy::too_many_arguments)]
fn fixpoint(
    rules: &[W],
    base: &HashMap<String, Vec<usize>>,
    seeds: &[Facts],
    reaches: &[bool],
    readers: &HashMap<String, Vec<usize>>,
    carried: &mut HashMap<String, Vec<Fact>>,
    start: Vec<usize>,
    changed: &mut BTreeSet<String>,
    step: &dyn Fn(&Facts, &S, &mut Fact) -> bool,
) {
    let mut round = start;
    while !round.is_empty() {
        let now: &HashMap<String, Vec<Fact>> = carried;
        let analysed: Vec<Facts> = round.iter().map(|&i| if reaches[i] && now.contains_key(rules[i].head.as_str()) { analyse(&rules[i], base, now, &seeds[i]) } else { Facts::new() }).collect();
        let mut next: BTreeSet<usize> = BTreeSet::new();
        for (&i, facts) in round.iter().zip(analysed) {
            let rule = &rules[i];
            let Some(cols) = carried.get_mut(rule.head.as_str()) else { continue };
            let mut grew = false;
            for (c, t) in rule.head_args.iter().enumerate() {
                if let Some(slot) = cols.get_mut(c) {
                    grew |= step(&facts, t, slot);
                }
            }
            if grew {
                changed.insert(rule.head.clone());
                next.extend(readers.get(rule.head.as_str()).into_iter().flatten().copied());
            }
        }
        round = next.into_iter().collect();
    }
}

/// `hiding::analyse` over a rule here: its variables that are scoped, and their grammars.
fn analyse(rule: &W, base: &HashMap<String, Vec<usize>>, carried: &HashMap<String, Vec<Fact>>, seed: &Facts) -> Facts {
    let mut facts: Facts = seed.clone();
    let positive = || rule.body.iter().filter_map(|(_, l)| l.positive());
    for a in positive() {
        let Some(&(_, cols, grammar, tape)) = ENTRIES.iter().find(|(e, ..)| *e == a.rel) else { continue };
        let grammar = grammar.and_then(|g| match a.args.get(g) {
            Some(S::Str(s)) => Some(s.clone()),
            _ => None,
        });
        let fact = Fact { scoped: true, grammar, definite: true };
        for &c in cols.iter().chain(tape.iter()) {
            if let Some(x) = a.args.get(c).and_then(S::var) {
                facts.entry(x).or_default().join(&fact);
            }
        }
    }
    for a in positive() {
        if let Some(cols) = carried.get(a.rel.as_str()) {
            for (c, f) in cols.iter().enumerate() {
                if (f.scoped || f.grammar.is_some())
                    && let Some(x) = a.args.get(c).and_then(S::var)
                {
                    let grammar = f.grammar.clone().filter(|g| !g.is_empty());
                    facts.entry(x).or_default().join(&Fact { definite: f.definite && f.scoped, grammar, ..f.clone() });
                }
            }
        }
    }
    let mut groups: Vec<Vec<Slot>> = Vec::new();
    const PAIR: &[usize] = &[0, 1];
    for a in positive() {
        let cols: &[usize] = match a.rel.as_str() {
            "tape_of" | "doc_root" | "tape" => PAIR,
            r if front::is_axis(r) => PAIR,
            r => base.get(r).map_or(&[], Vec::as_slice),
        };
        let vars: Vec<Slot> = cols.iter().filter_map(|&c| a.args.get(c).and_then(S::var)).collect();
        if vars.len() > 1 {
            groups.push(vars);
        }
    }
    for (_, l) in &rule.body {
        match l {
            L::Nearest { n, a, .. } => groups.push(vec![*n, *a]),
            L::Cmp { eq: true, l: S::Var(a), r: S::Var(b) } => groups.push(vec![*a, *b]),
            _ => {}
        }
    }
    loop {
        let mut changed = false;
        for g in &groups {
            let mut all = Fact::default();
            for x in g {
                if let Some(f) = facts.get(x) {
                    all.join(f);
                }
            }
            if all == Fact::default() {
                continue;
            }
            for x in g {
                changed |= facts.entry(*x).or_default().join(&all);
            }
        }
        if !changed {
            break;
        }
    }
    facts
}

#[cfg(test)]
mod tests {
    use crate::code_rule_dsl::logic::program_image::Builder;
    use crate::code_rule_dsl::logic::program_image::manager::Pipeline;

    /// A kind set read negated over a guest's node (`kind(r, k), !ok_kind(k)`) reads its qualified copy on the
    /// version as on the tree (`hiding::hide`): the version's pass left negated reads alone, the bare set holds none of
    /// the guest's kinds, and the Python pack's combine stopped on the debug check that the two agree.
    #[test]
    fn a_negated_kind_set_over_a_guest_reads_its_copy_as_on_the_tree() {
        let text = ".decl parsed(s: node, g: symbol, r: node)\n.decl kind(n: node, k: symbol)\n.input parsed, kind\n\
                    .decl ok_kind(k: symbol)\nok_kind(\"select\").\n\
                    .decl out(r: node)\n.output out\nout(r) :- parsed(_, \"sql\", r), kind(r, k), !ok_kind(k).\n";
        let mut tree = crate::code_rule_dsl::logic::rr::parse(text).unwrap();
        let (v, _) = Builder::of_program(&tree).unwrap().finish().unwrap();
        let mut p = Pipeline::new("test", v);
        p.program(&mut super::Hide { bound: 1 << 30 });
        let (out, _) = p.finish().unwrap();
        crate::code_rule_dsl::logic::plan::corpus::hiding::hide(&mut tree, 1 << 30);
        let (want, _) = Builder::of_program(&tree).unwrap().finish().unwrap();
        assert!(want.show().contains("!rr_in_sql_rows_ok_kind("), "the tree reads the copy: {}", want.show());
        assert_eq!((out.size(), out.digest()), (want.size(), want.digest()), "{}\n---\n{}", out.show(), want.show());
    }
}

/// Declares `rel` with `cols` on the staged fields.
fn declare_columns(b: &mut (impl FieldsMut + ?Sized), rel: &str, cols: &[(String, String)]) {
    let pairs: Vec<(&str, &str)> = cols.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
    b.declare(rel, &pairs);
}
