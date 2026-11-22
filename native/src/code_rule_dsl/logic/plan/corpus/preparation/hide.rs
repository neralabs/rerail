//! Hiding (decision 0058 §1, on the version since 0281's step 7a-1): the program's unscoped location
//! variables bounded to its shown documents, the relations read with a guest's node copied for its grammar, and the
//! kinds and fields of a guest's nodes named in its grammar. `hiding` says what the documents are and which
//! variables are scoped; the comments here say what each part of the pass does and why.
//!
//! The rounds edit rules and add copies, each round reading the rules as it began. Each rule here is its source rule
//! in the input version and what the rounds changed of it (`W`: the relation each atom reads, a `nearest`'s kind
//! set, a kind name a comparison qualified, and for a copy its head); the version is written once at the end, each
//! rule copied from its source with those edits, the bounds the last stage adds, and the aggregates' helpers.

use rerail_blocks::vector::Vector;
use super::super::hiding::{ENTRIES, Fact, VIEWS, dialect_label, qualified};
use crate::code_rule_dsl::logic::front::{self, Value};
use crate::code_rule_dsl::logic::program_image::fields::{Edit as Staged, FieldsMut, ProgramFields};
use crate::code_rule_dsl::logic::program_image::build::{Rewrite, Site, T};
use crate::code_rule_dsl::logic::program_image::manager::{Cx, Last, ProgramPass, ProgramW, Reads, Row};
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

/// Bounds `prog`'s unscoped location variables to its shown documents (`bound`), and names the kinds and fields of
/// scoped nodes in their grammar.
/// With `bound` from `hiding::bound`.
pub struct Hide {
    bound: u32,
    /// What `again` reads back (decision 0393, `ProgramPass::plan`).
    kept: Vec<Row>,
}

impl Hide {
    pub fn new(bound: u32) -> Hide {
        Hide { bound, kept: Vec::new() }
    }
}

/// A round's edit of one literal (by its place in the rule), applied once the round has read every rule.
enum Edit {
    Kindset(String),
    Rel(String),
    /// A comparison's kind name, qualified in a guest's grammar: (as written, qualified).
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
        let (views, base_locations) = bases(&prog);
        let view = |rel: &str| views.contains(rel);
        let mut seeds: Vec<Facts> = vec![Facts::new(); rules.len()];
        // A derived relation read with a scoped variable of a known grammar is specialized to it (decision 0058 §2, O98):
        // `rr_in_<grammar>_<column>_<relation>`, its rules copied with that head column seeded, so a node set's kinds are
        // the guest's grammar's and its guest rows are not bounded away. A kind set (`.kindset`) read by `nearest` over
        // such a node is copied as `rr_in_<grammar>_<set>`, with its kinds named in that grammar. To a fixpoint: a copy reads relations too.
        let mut made: BTreeSet<String> = BTreeSet::new();
        // Each copy's relation and seeded columns, with their grammars.
        let mut specialized: BTreeMap<String, (String, BTreeMap<usize, String>)> = BTreeMap::new();
        // Rules split once already: a variable of theirs that may be a guest's node has its clone (O110).
        let mut split: BTreeSet<usize> = BTreeSet::new();
        // The clones, by their place among the rules.
        let mut clones: BTreeSet<usize> = BTreeSet::new();
        // The analysis kept across the rounds, each round's change applied to it (decision 0246 §3).
        let mut delta = Delta::new(&rules);
        // The rules the round before changed or added (every rule, at first).
        let mut touched: Vector<usize> = (0..rules.len()).collect();
        loop {
            let _round = crate::code_rule_dsl::logic::profile::span(|| "hiding: a round".to_owned());
            let looked = delta.round(&rules, &prog.b, &seeds, &base_locations, &view, &touched);
            let reaches = &delta.reaches;
            let carried = &delta.carried;
            // The round reads the rules as they were when it began, and records its edits, applied after it: a copy is
            // made from its original as it was (no snapshot of every rule each round, which was a fifth of hiding a pack).
            let mut edits: Vec<(usize, usize, Edit)> = Vec::new();
            // The relations rules derive: a relation of stated facts alone is a kind set when a kind is read through it.
            let ruled = &delta.heads;
            // Each copy with the rule it was made from (the request record follows it, `asked_after`).
            let mut copies: Vec<(W, Facts, bool)> = Vec::new();
            let mut decls: Vec<(String, Vec<(String, String)>)> = Vec::new();
            // The axes over a guest grammar's kind set made this round: inputs the transducer makes, as the run's are.
            let mut axes: Vec<String> = Vec::new();
            let mut kindsets: Vec<(String, Vec<String>)> = Vec::new();
            // Kind sets read by value over a guest's kinds, copied in its grammar: (copy, original, kinds).
            let mut kind_facts: Vec<(String, String, Vec<String>)> = Vec::new();
            let mut changed = false;
            // Each rule's analysis reads the round's start alone, used in order below. Only the
            // rules `round` names: any other's analysis is the round before's, and its edits are made.
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
                // A variable that may be a guest's node of a known grammar, and may be a host's: the rule stays as it is
                // for the host's, and a clone with the variable seeded as the guest's reads the guest's copies. Copying
                // in place of the atom would drop the host's rows (`tv`'s nodes are both, and `tbv`'s `live_asg` read
                // only the annotation copy, O110).
                let maybe: Facts = facts
                    .iter()
                    .filter(|(_, f)| f.scoped && !f.definite && f.grammar.as_ref().is_some_and(|g| !g.is_empty()))
                    .map(|(v, f)| (*v, Fact { definite: true, ..f.clone() }))
                    .collect();
                if !maybe.is_empty() && split.insert(i) {
                    let mut seed = seeds[i].clone();
                    seed.extend(maybe);
                    copies.push((rule.clone(), seed, true));
                    changed = true;
                }
                let grammar_of = |s: Slot| facts.get(&s).filter(|f| f.scoped && f.definite).and_then(|f| f.grammar.clone()).filter(|g| !g.is_empty());
                // A kind read off a guest's node is its grammar's (decision 0222): `kind(f, k), fn_scope(k)` over a view's
                // node reads the kind set named in the view's grammar, and `k = "arrow_function"` names that grammar's
                // kind. Unqualified, both are the run language's, and a library over another grammar (tsx/names.rerail
                // read over a Vue component's view) finds none of its kinds.
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
                            // Negated as well: `kind(s, k), !def_or_class(k)` over a guest's node
                            // reads the set's qualified copy; the bare set holds none of the guest's kinds.
                            L::Atom(a)
                                // Not `rr_in_<g>_<set>`, `nearest`'s copy of the set: that one is a kind set the transducer
                                // answers, and holds no rows.
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
                    // `has(r, f, "field")` and `inside(n, p, "field")` over a guest's node: the axis's kind set is named in
                    // the guest's grammar, as `nearest`'s is (`rr_has_rr_in_<grammar>_<set>`). Unqualified, the kind is
                    // the run language's, and the axis finds no guest node.
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
                    // A column scoped already is copied too where it knows no grammar, or another: a util whose node
                    // reaches a guest relation (`host_node(p, h)` over a view's node, decision 0222) is scoped there, and
                    // read with a view's node of a known grammar it must name that grammar's kinds.
                    let spec = a.args.iter().enumerate().find_map(|(c, t)| {
                        let g = grammar_of(t.var()?)?;
                        (!cols.get(c).is_some_and(|f| f.scoped && f.grammar.as_deref() == Some(g.as_str()))).then_some((c, g))
                    });
                    let Some((c, g)) = spec else { continue };
                    // A copy read with another column scoped is a copy of the same relation with both seeded, not a copy
                    // of the copy: one per relation and set of seeded columns, so the copies end.
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
                            copies.push((copy, seed, false));
                        }
                        if let Some(d) = prog.columns_of(orig.as_str()) {
                            decls.push((name.clone(), d));
                        }
                        specialized.insert(name.clone(), (orig, cols));
                    }
                    // An atom that reads the copy already is not a change: a copy whose head holds a constant at the
                    // seeded column is never carried as scoped there, so the same copy is found each round (a loop
                    // that did not end on `ruff-perf401`).
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
                    // The string side of the comparison the edit was recorded for: the one that named `from`.
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
            for (rule, seed, clone) in copies {
                if clone {
                    clones.insert(rules.len());
                }
                rules.push(rule);
                seeds.push(seed);
            }
            touched = edited.into_iter().chain(first_copy..rules.len()).collect();
            delta.changed(&rules, &touched);
            for (name, cols) in decls {
                declare_columns(&mut prog.b.fields(), &name, &cols);
            }
            all_kindsets.extend(kindsets);
            for (name, orig, kinds) in kind_facts {
                if let Some(d) = prog.columns_of(orig.as_str()) {
                    declare_columns(&mut prog.b.fields(), &name, &d);
                }
                let rows: Vec<front::List<Value>> = kinds.iter().map(|k| [Value::Str(k.as_str().into())].into()).collect();
                prog.b.fields().set_facts(&name, &rows);
            }
            for axis in axes {
                prog.b.fields().add_input(&axis);
            }
            if !changed {
                break;
            }
        }
        // Each rule bounded on its own, then the aggregates' helpers.
        let bounding = crate::code_rule_dsl::logic::profile::span(|| "hiding: each rule bounded".to_owned());
        let carried = std::mem::take(&mut delta.carried);
        let reached: BTreeSet<String> = delta.made.iter().cloned().collect();
        drop(delta);
        let bound = self.bound;
        let bound_of = |doc_column: bool| if doc_column { i64::from(bound) } else { i64::from(bound) << 32 };
        let reaches = reaching(&rules, &seeds);
        // What `parsed` asks for (`rr_embed_req`, and the contexts `rr_ctx<k>` only it reads) is not bounded: asking is
        // not seeing. A string in an annotation is in a document not shown, and its rule's demand is made of inputs
        // alone (`rr_ctx3(s) :- kind(s, "string")`), so bounded, the string inside `"Opt['Baz']"` was never asked for,
        // and no rule reached it through the annotation either.
        let asking = asking(&prog, &rules);
        let mut finals: Vec<Final> = vec![Final::default(); rules.len()];
        // An annotation is a tape value (decision 0431 §3), shown to no rule that does not name a guest (decision
        // 0165): a clone whose head would carry an annotation's node into a column an output shows (`shown_columns`)
        // goes. A clone that reads one inside its rule stays: a call's target through a quoted `n: "Node"`. Kept, a
        // policy's own rules found calls inside string annotations through `builtin_at` beside a library that reads
        // them (C408), placed at nodes no file holds (R9), where alone they found none.
        let shown = shown_columns(&rules, &v.output_names());
        let mut gone: BTreeSet<usize> = BTreeSet::new();
        for (i, rule) in rules.iter().enumerate() {
            if VIEWS.contains(&rule.head.as_str()) || asking.contains(&rule.head) {
                continue;
            }
            let facts: Facts = if reaches[i] { analyse(rule, &base_locations, &carried, &seeds[i]) } else { Facts::new() };
            if clones.contains(&i) && shows_an_annotation(rule, &facts, &shown) {
                gone.insert(i);
                continue;
            }
            finals[i] = bounded(v, rule, &facts, &base_locations, bound);
        }
        drop(bounding);
        // An aggregate's atom is its own scope: one over a base atom reads it through a bounded helper,
        // `rr_shown_agg<k>(vars) :- atom, bounds`.
        let mut helpers: Vec<Helper> = Vec::new();
        for (i, rule) in rules.iter().enumerate() {
            if VIEWS.contains(&rule.head.as_str()) || gone.contains(&i) {
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
                // Every wildcard a column of the helper, named: the aggregate counts the atom's rows, not their
                // projection. The other arguments are the atom's own.
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
                // Named by what it holds (`helper_names`), not by its place among the program's helpers.
                let args: Vec<String> = src
                    .body()
                    .find_map(|l| match l.view() {
                        Lit::Agg { atom: a, .. } if a.node() == atom.node => Some(a.args().zip(&inner).map(|(t, x)| x.as_ref().map_or_else(|| v.show_term(src, t), |x| format!("${x}"))).collect()),
                        _ => None,
                    })
                    .expect("the aggregate's atom");
                let key = helper_key(&atom.rel, &args, &bounds, &vars);
                let outer: Vec<Option<String>> = vars.iter().map(|(x, _)| (!x.starts_with("rr_shown_agg_c")).then(|| x.clone())).collect();
                finals[i].aggs.insert(atom.node, (key.clone(), outer));
                helpers.push(Helper { name: key, vars, src: rule.src, atom: atom.node, rel: atom.rel.clone(), inner, bounds });
            }
        }
        // Each helper named by its rank among the helpers' texts: two of one text are one helper, and an edit that moves a
        // rule leaves every other helper's name (decision 0393).
        let names = helper_names(helpers.iter().map(|h| h.name.clone()));
        for fin in &mut finals {
            for (name, _) in fin.aggs.values_mut() {
                *name = names[name.as_str()].clone();
            }
        }
        let mut seen: BTreeSet<String> = BTreeSet::new();
        helpers.retain_mut(|h| {
            h.name = names[h.name.as_str()].clone();
            seen.insert(h.name.clone())
        });
        for h in &helpers {
            let cols: Vec<(&str, &str)> = h.vars.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
            prog.b.fields().declare(&h.name, &cols);
        }
        // The version: every rule copied from its source with its edits, then the helpers, then the fields.
        let b = w.derive()?;
        b.fields().state_fields_of(&prog.b);
        if all_kindsets != v.kindsets() {
            b.fields().set_field(crate::code_rule_dsl::logic::program_facts::Field::Kindsets(&all_kindsets));
        }
        let asked = asked_after(v, &rules);
        for (i, (rule, fin)) in rules.iter().zip(&finals).enumerate() {
            if !gone.contains(&i) {
                write_rule(b, v, rule, fin)?;
            }
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
            b.fields().set_asked(&asked);
        }
        self.kept = plan_rows(&reached, &made, &specialized, &asking, &carried);
        Ok(())
    }

    fn plan(&self) -> Vec<Row> {
        self.kept.clone()
    }

    /// Over an edit (decision 0393): the edit's added rules hidden alone, onto the last output, where the edit leaves
    /// the rounds' fixpoint as it was. That holds when no rule of the edit reads or defines what the guests' copies
    /// and helpers are made of (a view, a context or the guest demand, a relation copied for a grammar, an
    /// aggregate over a base relation), none is recursive, an added rule reaches only a relation reached already and
    /// a removed one leaves its head reached as it was, each changed head carries the facts it carried, and no added
    /// rule's variable has a guest's grammar (so the rounds edit none of its literals). Otherwise it runs whole.
    fn again(&mut self, cx: &Cx<'_>, last: &Last<'_>, w: &mut ProgramW<'_>) -> Result<bool> {
        if last.delta.touched != Reads::RULES || last.plan.is_empty() {
            return Ok(false);
        }
        let Some(plan) = Plan::of(last.plan) else { return Ok(false) };
        let v = cx.v();
        let (views, base_locations) = bases(&**v);
        let mut removed: Vec<(RuleId, W)> = Vec::new();
        for id in last.delta.removed.iter() {
            let r = last.input.rule(id).ok_or("a removed rule is not its last input's")?;
            removed.push((id, W::of(last.input, r)));
        }
        let mut added: Vec<W> = Vec::new();
        for &id in &last.delta.added {
            added.push(W::of(v, v.rule(id).ok_or("an added rule is not its input's")?));
        }
        let enters = |w: &W| w.body.iter().any(|(_, l)| l.positive().is_some_and(|a| ENTRIES.iter().any(|(e, ..)| *e == a.rel) || plan.reached.contains(&a.rel)));
        let demand = super::super::guests::DEMAND;
        for w in removed.iter().map(|(_, w)| w).chain(&added) {
            let h = w.head.as_str();
            if views.contains(h) || h == demand || is_context(h) || plan.asking.contains(h) || plan.origs.contains(h) || plan.copies.contains(h) {
                return Ok(false);
            }
            for (_, l) in &w.body {
                let Some(a) = l.atom() else { continue };
                if a.rel == h || is_context(&a.rel) || plan.asking.contains(&a.rel) || (matches!(l, L::Agg(_)) && base_locations.get(a.rel.as_str()).is_some_and(|c| !c.is_empty())) {
                    return Ok(false);
                }
            }
        }
        let heads: BTreeSet<String> = removed.iter().map(|(_, w)| w).chain(&added).map(|w| w.head.clone()).collect();
        // No head of the edit reads itself through others.
        let mut up: BTreeSet<String> = BTreeSet::new();
        let mut todo: Vec<String> = heads.iter().cloned().collect();
        while let Some(rel) = todo.pop() {
            for of in [&**v, &**last.input] {
                let Some(id) = of.rel(&rel) else { continue };
                for (r, _) in of.readers(id) {
                    let h = of.rel_name(r.head().rel());
                    if up.insert(h.to_owned()) {
                        todo.push(h.to_owned());
                    }
                }
            }
        }
        if heads.iter().any(|h| up.contains(h)) {
            return Ok(false);
        }
        let rules_of = |h: &str| -> Vec<W> { v.rel(h).into_iter().flat_map(|id| v.heads(id)).map(|r| W::of(v, r)).collect() };
        for w in &added {
            if enters(w) && !plan.reached.contains(&w.head) {
                return Ok(false);
            }
        }
        for (_, w) in &removed {
            if enters(w) && !rules_of(&w.head).iter().any(&enters) {
                return Ok(false);
            }
        }
        // Each changed head's facts, as the rounds make them from its rules over the others' (which the edit leaves).
        for h in &heads {
            let rules = rules_of(h);
            let n = v.columns_of(h.as_str()).map_or(0, |c| c.len());
            let mut cols = vec![Fact::default(); n];
            let facts: Vec<Facts> = rules.iter().map(|r| if enters(r) { analyse(r, &base_locations, &plan.carried, &Facts::new()) } else { Facts::new() }).collect();
            for (r, facts) in rules.iter().zip(&facts) {
                for (c, t) in r.head_args.iter().enumerate() {
                    if let (Some(slot), S::Var(x)) = (cols.get_mut(c), t)
                        && let Some(f) = facts.get(x)
                    {
                        slot.join(f);
                    }
                }
            }
            for f in cols.iter_mut() {
                f.definite = f.scoped;
            }
            for (r, facts) in rules.iter().zip(&facts) {
                for (c, t) in r.head_args.iter().enumerate() {
                    let definite = matches!(t, S::Var(x) if facts.get(x).is_some_and(|f| f.definite));
                    if let Some(slot) = cols.get_mut(c)
                        && slot.definite
                        && !definite
                    {
                        slot.definite = false;
                    }
                }
            }
            let was = plan.carried.get(h.as_str());
            let same = if rules.is_empty() { was.is_none_or(|w| w.iter().all(|f| *f == Fact::default())) } else { was.map_or_else(|| cols.iter().all(|f| *f == Fact::default()), |w| *w == cols) };
            if !same {
                return Ok(false);
            }
        }
        // The added rules' last stage, by their facts: none may name a guest's grammar.
        let mut finals: Vec<Final> = Vec::new();
        for w in &added {
            let facts = if enters(w) { analyse(w, &base_locations, &plan.carried, &Facts::new()) } else { Facts::new() };
            if facts.values().any(|f| f.grammar.as_ref().is_some_and(|g| !g.is_empty())) {
                return Ok(false);
            }
            finals.push(bounded(v, w, &facts, &base_locations, self.bound));
        }
        // Each removed rule made one rule of the last output (none was copied: no head of the edit is).
        let mut gone = Vec::new();
        for (id, _) in &removed {
            let made: Vec<RuleId> = last.output.derived(*id).map(|r| r.id()).collect();
            if made.len() != 1 {
                return Ok(false);
            }
            gone.extend(made);
        }
        let b = w.on()?;
        for id in gone {
            b.remove(id);
        }
        for (rule, fin) in added.iter().zip(&finals) {
            write_rule(b, v, rule, fin)?;
        }
        self.kept = last.plan.to_vec();
        Ok(true)
    }
}

/// What a run of `Hide` keeps for `again` (`Hide::plan`): the relations its rounds reached, the names they made,
/// the relations copied for a grammar and their copies, the relations the guest demand asks for, and each
/// relation's carried facts (those not the default).
struct Plan {
    reached: BTreeSet<String>,
    copies: BTreeSet<String>,
    origs: BTreeSet<String>,
    asking: BTreeSet<String>,
    carried: HashMap<String, Vec<Fact>>,
}

impl Plan {
    fn of(rows: &[Row]) -> Option<Plan> {
        let text = |x: &Value| if let Value::Str(s) = x { Some(s.to_string()) } else { None };
        let int = |x: &Value| if let Value::Int(n) = x { Some(*n) } else { None };
        let mut p = Plan { reached: BTreeSet::new(), copies: BTreeSet::new(), origs: BTreeSet::new(), asking: BTreeSet::new(), carried: HashMap::default() };
        let mut ran = false;
        for r in rows {
            match (text(r.first()?)?.as_str(), &r[1..]) {
                ("ran", []) => ran = true,
                ("reached", [x]) => drop(p.reached.insert(text(x)?)),
                ("made", [_]) => {}
                ("copy", [x, o]) => {
                    p.copies.insert(text(x)?);
                    p.origs.insert(text(o)?);
                }
                ("asking", [x]) => drop(p.asking.insert(text(x)?)),
                ("carried", [x, n, c, scoped, definite, g @ ..]) => {
                    let cols = p.carried.entry(text(x)?).or_insert_with(|| vec![Fact::default(); int(n).and_then(|n| usize::try_from(n).ok()).unwrap_or(0)]);
                    let f = cols.get_mut(usize::try_from(int(c)?).ok()?)?;
                    *f = Fact { scoped: int(scoped)? != 0, definite: int(definite)? != 0, grammar: match g { [g] => Some(text(g)?), _ => None } };
                }
                _ => return None,
            }
        }
        ran.then_some(p)
    }
}

/// `Hide::plan`'s rows of a run (`Plan`).
fn plan_rows(reached: &BTreeSet<String>, made: &BTreeSet<String>, specialized: &BTreeMap<String, (String, BTreeMap<usize, String>)>, asking: &BTreeSet<String>, carried: &HashMap<String, Vec<Fact>>) -> Vec<Row> {
    let s = |x: &str| Value::Str(x.to_owned().into());
    let n = |k: i64| Value::Int(k);
    let mut rows = vec![vec![s("ran")]];
    rows.extend(reached.iter().map(|r| vec![s("reached"), s(r)]));
    rows.extend(made.iter().map(|r| vec![s("made"), s(r)]));
    rows.extend(specialized.iter().map(|(x, (o, _))| vec![s("copy"), s(x), s(o)]));
    rows.extend(asking.iter().map(|r| vec![s("asking"), s(r)]));
    let mut held: Vec<(&String, &Vec<Fact>)> = carried.iter().filter(|(_, cols)| cols.iter().any(|f| *f != Fact::default())).collect();
    held.sort_by_key(|(r, _)| *r);
    for (rel, cols) in held {
        for (c, f) in cols.iter().enumerate().filter(|(_, f)| **f != Fact::default()) {
            let mut row = vec![s("carried"), s(rel), n(cols.len() as i64), n(c as i64), n(i64::from(f.scoped)), n(i64::from(f.definite))];
            if let Some(g) = &f.grammar {
                row.push(s(g));
            }
            rows.push(row);
        }
    }
    rows
}

/// A positive atom of a base relation (`bases`' locations): its columns that hold locations in one document, and its
/// document column. `combining::one_document` reads a document's rows by them too.
pub(super) fn located<'b>(rel: &str, base: &'b HashMap<String, rerail_blocks::vector::Vector<usize>>) -> (&'b [usize], Option<usize>) {
    const PAIR: &[usize] = &[0, 1];
    match rel {
        // A node's parent is in its document (`tape(n, par, size, depth)`, the climbs' walk).
        "tape_of" | "tape" => (PAIR, None),
        // A tree axis's node and its ancestor (`rr_has_<set>(n, a)`, `rr_inside_<set>(n, a)`): `has(r, c, "field")`
        // over a guest's root r makes c the guest's node.
        r if front::is_axis(r) => (PAIR, None),
        r => (base.get(r).map_or(&[], |v| v.as_slice()), front::document_column(r)),
    }
}

/// The views the lowering writes as rules (those the program does not take as inputs), and each base relation's
/// location columns (its node columns and its document's).
pub(super) fn bases(prog: &dyn ProgramFields) -> (BTreeSet<String>, HashMap<String, Vector<usize>>) {
    let inputs = prog.input_names();
    let views: BTreeSet<String> = VIEWS.iter().filter(|r| !inputs.iter().any(|i| i == *r)).map(|r| (*r).to_owned()).collect();
    let base: BTreeSet<String> = inputs.iter().cloned().chain(VIEWS.iter().filter(|r| views.contains(**r)).map(|r| (*r).to_owned())).filter(|r| !ENTRIES.iter().any(|(e, ..)| e == r)).collect();
    let locations = |rel: &str| -> Vector<usize> {
        let mut cols: Vector<usize> = prog.columns_of(rel).map_or_else(rerail_blocks::vector::Vector::new, |c| c.iter().enumerate().filter(|(_, (_, t))| t == "node").map(|(i, _)| i).collect());
        if let Some(c) = crate::code_rule_dsl::typing::file_local::document_column(rel)
            && !cols.contains(&c)
        {
            cols.push(c);
        }
        cols
    };
    let base_locations = base.iter().map(|r| (r.clone(), locations(r))).collect();
    (views, base_locations)
}

/// One rule of the output: `rule` copied from its source with its edits and its last stage's additions (`fin`), its
/// source recorded (so an edit's removed rule finds what it made, `Version::derived`, decision 0393).
fn write_rule(b: &mut crate::code_rule_dsl::logic::program_image::Builder<'_>, v: &Version, rule: &W, fin: &Final) -> Result<()> {
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
    Ok(())
}

/// Hiding's last stage for one rule: its kind and field names qualified, its wildcards named, and its bounds
/// (`bound`), by its facts (`analyse`; none where it is not reached).
fn bounded(v: &Version, rule: &W, facts: &Facts, base_locations: &HashMap<String, Vector<usize>>, bound: u32) -> Final {
    let bound_of = |doc_column: bool| if doc_column { i64::from(bound) } else { i64::from(bound) << 32 };
    let src = v.rule(rule.src).expect("a rule of the input");
    let name = |s: Slot| v.text(src.var(s)).to_owned();
    let mut out = Final::default();
    let fin = &mut out;
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
    out
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
    /// A kind or field constant of `a` named in `label`'s grammar, unless it names one already.
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

/// An aggregate helper's text (the helpers `Hide` makes): its atom's relation and arguments, its bounds and its
/// columns. Two helpers of one text hold the same rows.
pub(in crate::code_rule_dsl::logic::plan::corpus) fn helper_key(rel: &str, args: &[String], bounds: &[(String, i64)], vars: &[(String, String)]) -> String {
    format!("{rel}({}) {bounds:?} {vars:?}", args.join(", "))
}

/// Each helper's name by its text (`helper_key`): `rr_shown_agg<k>`, `k` its text's rank among the program's.
pub(in crate::code_rule_dsl::logic::plan::corpus) fn helper_names(keys: impl Iterator<Item = String>) -> HashMap<String, String> {
    let keys: BTreeSet<String> = keys.collect();
    keys.into_iter().enumerate().map(|(k, key)| (key, format!("rr_shown_agg{k}"))).collect()
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
        // The side that named the kind as written (the round's `Edit::Str`).
        let text = self.v.text(a);
        self.cmps.get(&l.node())?.iter().find(|(then, _)| then == text).map(|(_, now)| T::StrAtom(*now))
    }
    fn kindset(&mut self, _set: AtomId) -> Option<String> {
        self.sets.get(&self.cur).map(|s| (*s).to_owned())
    }
}

/// Which rules' analysis can find anything (O-combine-hiding-single): one with a seed, or a positive atom of a guest
/// relation (`ENTRIES`) or of a relation such a rule makes, transitively. Any other rule reads no scoped column and
/// enters no guest, so its analysis is empty, and is not made: in a policy most rules are such.
fn reaching(rules: &[W], seeds: &[Facts]) -> Vec<bool> {
    // A worklist over each relation's readers: a rule is looked at once, and again only when it is marked.
    let mut readers: HashMap<&str, Vector<usize>> = HashMap::default();
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

/// Whether `rule`'s head, by its analysis (`facts`), holds an annotation's node at a column an output shows.
fn shows_an_annotation(rule: &W, facts: &Facts, shown: &HashMap<String, BTreeSet<usize>>) -> bool {
    let Some(cols) = shown.get(rule.head.as_str()) else { return false };
    // Scoped is unbounded (`bounded`): a node that may be an annotation's is shown as one that is.
    let annotation = |x: Slot| facts.get(&x).is_some_and(|f| f.scoped && f.grammar.as_deref() == Some("annotation"));
    rule.head_args.iter().enumerate().any(|(c, t)| cols.contains(&c) && t.var().is_some_and(annotation))
}

/// The columns an output shows, by relation: an output's every column and a finding's place (what `rr_unplaced`
/// reads), and each column of a relation a rule reads into a column its head shows, transitively. A node that reaches
/// one is shown to the output's reader.
fn shown_columns(rules: &[W], outputs: &[String]) -> HashMap<String, BTreeSet<usize>> {
    let mut rules_of: HashMap<&str, Vec<&W>> = HashMap::default();
    for r in rules {
        rules_of.entry(r.head.as_str()).or_default().push(r);
    }
    let mut shown: HashMap<String, BTreeSet<usize>> = HashMap::default();
    let mut todo: Vec<&str> = Vec::new();
    for o in outputs {
        let arity = rules_of.get(o.as_str()).and_then(|rs| rs.first()).map_or(0, |r| r.head_args.len());
        shown.insert(o.clone(), (0..arity).collect());
        todo.push(o.as_str());
    }
    // A finding's place: the column its `rr_unplaced` rule reads a node of (decision 0431 R9).
    for w in rules.iter().filter(|w| w.head == crate::code_rule_dsl::logic::rr::UNPLACED) {
        let placed: BTreeSet<Slot> = w.body.iter().filter_map(|(_, l)| l.positive()).filter(|a| a.rel == "node").filter_map(|a| a.args.get(1).and_then(S::var)).collect();
        for a in w.body.iter().filter_map(|(_, l)| l.positive()).filter(|a| a.rel != "node") {
            let cols: Vec<usize> = a.args.iter().enumerate().filter(|(_, t)| t.var().is_some_and(|x| placed.contains(&x))).map(|(c, _)| c).collect();
            if !cols.is_empty() {
                shown.entry(a.rel.clone()).or_default().extend(cols);
                todo.push(a.rel.as_str());
            }
        }
    }
    while let Some(rel) = todo.pop() {
        let cols = shown.get(rel).cloned().unwrap_or_default();
        for w in rules_of.get(rel).into_iter().flatten() {
            let vars: BTreeSet<Slot> = cols.iter().filter_map(|&c| w.head_args.get(c).and_then(S::var)).collect();
            for a in w.body.iter().filter_map(|(_, l)| l.positive()) {
                let more: Vec<usize> = a.args.iter().enumerate().filter(|(_, t)| t.var().is_some_and(|x| vars.contains(&x))).map(|(c, _)| c).collect();
                let entry = shown.entry(a.rel.clone()).or_default();
                let before = entry.len();
                entry.extend(more);
                if entry.len() != before {
                    todo.push(a.rel.as_str());
                }
            }
        }
    }
    shown
}

/// A context relation (`rr_ctx…`, a policy's copy too): what `asking` adds.
pub(super) fn is_context(r: &str) -> bool {
    let own = r.strip_prefix('p').and_then(|t| t.find('_').filter(|&i| i > 0 && t[..i].bytes().all(|b| b.is_ascii_digit())).map(|i| &t[i + 1..]));
    own.unwrap_or(r).starts_with("rr_ctx")
}

/// `hiding::asking` over the rules here: the guest demand and the contexts only it reads, transitively.
fn asking(prog: &dyn ProgramFields, rules: &[W]) -> BTreeSet<String> {
    let declared = prog.declared_names();
    let context = is_context;
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

/// Hiding's analysis kept across its rounds (decision 0246 §3, by delta): which rules reach a guest, and what each
/// derived relation's columns carry. A round changes some rules (its edits) and adds others (its copies); the next
/// round looks at those, and at what their change reaches, not at every rule again.
///
/// - **Reaching a guest** only grows: an edit gives an atom a copy, whose rules are seeded and so reach one, or a
///   relation of facts or an axis, which no rule derives.
/// - **What the columns gain** (scoped, a grammar) only grows: a copy carries at least what its original did, and a
///   rule added adds. The round before's fixpoint is below this round's, so it starts from there, from the rules
///   changed.
/// - **Which columns are definite** is a greatest fixpoint: made again over the cone the change reaches (the
///   relations the changed rules derive, those whose gain grew, and every relation read from them), from every
///   scoped column definite. A relation outside the cone reads nothing that changed, and keeps its columns.
/// - **A round's analysis** of a rule reads its body, its seed, whether it reaches a guest, and what its atoms'
///   columns carry. Only the rules whose one of those changed are analysed again: another's is the round before's, its
///   edits made.
struct Delta {
    /// Each relation's rules, by head.
    heads: HashMap<String, Vector<usize>>,
    /// The rules reading each relation in a positive atom: what reaching and the fixpoints follow. An edited rule is
    /// listed under the relation it read too (a superset: following it looks at one rule more, and changes nothing).
    readers: HashMap<String, Vector<usize>>,
    /// The rules reading each relation in any literal: whose analysis a change of its columns changes.
    any_readers: HashMap<String, Vector<usize>>,
    reaches: Vec<bool>,
    /// The relations a rule reaching a guest derives.
    made: std::collections::HashSet<String>,
    /// What the columns gain, the first fixpoint's result.
    gained: HashMap<String, Vec<Fact>>,
    /// What the columns carry: the gain, with which columns are definite.
    carried: HashMap<String, Vec<Fact>>,
    /// The rules listed so far.
    listed: usize,
    first: bool,
}

impl Delta {
    fn new(rules: &[W]) -> Delta {
        let mut d = Delta { heads: HashMap::default(), readers: HashMap::default(), any_readers: HashMap::default(), reaches: Vec::new(), made: Default::default(), gained: HashMap::default(), carried: HashMap::default(), listed: 0, first: true };
        d.list(rules, &(0..rules.len()).collect::<Vec<_>>());
        d
    }

    /// Lists rules `rules` (new ones, or edited) under their head and the relations they read.
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

    /// The round's edits made (rules `touched`, edited or new).
    fn changed(&mut self, rules: &[W], touched: &[usize]) {
        self.list(rules, touched);
    }

    /// Brings the analysis to the rules as the round begins, `touched` being those changed since the round before (every
    /// rule at first); returns the rules the round analyses, in order.
    fn round(&mut self, rules: &[W], prog: &dyn ProgramFields, seeds: &[Facts], base: &HashMap<String, Vector<usize>>, view: &dyn Fn(&str) -> bool, touched: &[usize]) -> Vector<usize> {
        let first = std::mem::replace(&mut self.first, false);
        self.reaches.resize(rules.len(), false);
        // Reaching a guest, from the rules changed: one that enters, or reads a relation a reaching rule derives.
        let mut newly: Vector<usize> = rerail_blocks::vector::Vector::new();
        let mut work: Vector<usize> = rerail_blocks::vector::Vector::new();
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
        // What the columns gain, from the round before's, starting at the rules changed.
        let start: BTreeSet<usize> = touched.iter().chain(&newly).copied().collect();
        let mut grew: BTreeSet<String> = BTreeSet::new();
        fixpoint(rules, base, seeds, &self.reaches, &self.readers, &mut self.gained, start.iter().copied().collect(), &mut grew, &|facts, t, slot| match t {
            S::Var(x) => facts.get(x).is_some_and(|f| slot.join(f)),
            _ => false,
        });
        // Which columns are definite, made again over the cone of the change.
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
        let which: Vector<usize> = cone.iter().flat_map(|r| self.heads.get(r.as_str()).into_iter().flatten().copied()).collect::<BTreeSet<usize>>().into_iter().collect();
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

/// A fixpoint over what the columns carry (`Delta`), from rules `start`: in rounds, each round's rules analysed
/// against what the columns carried as it began, and applied in order. Both of hiding's fixpoints are
/// monotone (columns only gain, then only lose being definite), so the rounds reach what a rule at a time does. The
/// relations whose columns changed are added to `changed`.
#[allow(clippy::too_many_arguments)]
fn fixpoint(
    rules: &[W],
    base: &HashMap<String, Vector<usize>>,
    seeds: &[Facts],
    reaches: &[bool],
    readers: &HashMap<String, Vector<usize>>,
    carried: &mut HashMap<String, Vec<Fact>>,
    start: Vector<usize>,
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

/// A rule's variables that are scoped, and their grammars; `seed` holds what a specialized copy's head is.
fn analyse(rule: &W, base: &HashMap<String, Vector<usize>>, carried: &HashMap<String, Vec<Fact>>, seed: &Facts) -> Facts {
    let mut facts: Facts = seed.clone();
    let positive = || rule.body.iter().filter_map(|(_, l)| l.positive());
    // Seeds: the guest relations' guest columns, with the grammar a constant names.
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
    // Derived relations' carried columns.
    for a in positive() {
        if let Some(cols) = carried.get(a.rel.as_str()) {
            for (c, f) in cols.iter().enumerate() {
                // A column rules fill from guests of several grammars (`""`: a rule per dialect of a view, decision
                // 0222) says nothing of this rule's: a guest relation here names it.
                if (f.scoped || f.grammar.is_some())
                    && let Some(x) = a.args.get(c).and_then(S::var)
                {
                    let grammar = f.grammar.clone().filter(|g| !g.is_empty());
                    facts.entry(x).or_default().join(&Fact { definite: f.definite && f.scoped, grammar, ..f.clone() });
                }
            }
        }
    }
    // A tape's documents and a document's nodes are the tape's, and a base atom's locations are in one document.
    // A document column is one way (decision 0431 §4): a document's nodes are its guest's (a tape value's, its own
    // document), but a node's document may be its file (a program's nodes are the file's), so a node gives it scope,
    // not its grammar. `(from, to)`: from's facts flow to `to`, its grammar too unless `to` is the document.
    let mut groups: Vec<Vec<Slot>> = Vec::new();
    let mut to_document: Vec<(Slot, Slot)> = Vec::new();
    for a in positive() {
        let (cols, document) = located(a.rel.as_str(), base);
        let doc = document.filter(|d| cols.contains(d)).and_then(|d| a.args.get(d).and_then(S::var));
        let vars: Vec<Slot> = cols.iter().filter(|&&c| Some(c) != document).filter_map(|&c| a.args.get(c).and_then(S::var)).collect();
        if let Some(d) = doc {
            for &x in vars.iter().filter(|&&x| x != d) {
                groups.push(vec![d, x]);
                to_document.push((x, d));
            }
        }
        if vars.len() > 1 {
            groups.push(vars);
        }
    }
    for (_, l) in &rule.body {
        match l {
            L::Nearest { n, a, .. } => groups.push(vec![*n, *a]),
            // `t = rr_tape1`: a hoisted value is the variable it is named by.
            L::Cmp { eq: true, l: S::Var(a), r: S::Var(b) } => groups.push(vec![*a, *b]),
            _ => {}
        }
    }
    loop {
        let mut changed = false;
        for g in &groups {
            // A node and its document: the document's facts flow to the node, the node's to the document without
            // its grammar.
            let one_way = match g.as_slice() {
                [d, x] if to_document.contains(&(*x, *d)) => Some((*d, *x)),
                _ => None,
            };
            if let Some((d, x)) = one_way {
                if let Some(f) = facts.get(&d).cloned() {
                    changed |= facts.entry(x).or_default().join(&f);
                }
                if let Some(f) = facts.get(&x).cloned() {
                    changed |= facts.entry(d).or_default().join(&Fact { grammar: None, ..f });
                }
                continue;
            }
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


/// Declares `rel` with `cols` on the staged fields.
fn declare_columns(b: &mut (impl FieldsMut + ?Sized), rel: &str, cols: &[(String, String)]) {
    let pairs: Vec<(&str, &str)> = cols.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
    b.declare(rel, &pairs);
}

// cratify: `tests` is rerail-logic's (it mounts it: it reads the language, `rr::parse_version`, or the corpus's root)
