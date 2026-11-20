//! Which documents a rule sees (decision 0058 §1): a base atom ranges over the policy's shown documents unless the
//! rule reached it from a guest.
//!
//! A run's documents are, by key (`syntax::GUEST_KEYS`, `syntax::HIDDEN_KEYS`):
//! - files, `1..2^30`;
//! - guests of code in the run's language (Python in a Python string, embedded by a library), `2^30..2^30 + 2^28`:
//!   shown, unless the program says `.guests off`;
//! - annotations (a type expression, Python's in a Python run), `..2^30 + 2^29`: not shown to a rule that does not
//!   name them, but kept in a relation they reach once its copies for them are folded into it (`fold_own`);
//! - every other document (a guest in another grammar, a value `@parse` or `@splice` made), up to `2^31`: never
//!   shown.
//!
//! A node's id is its document's key `<< 32` and its index, so "shown" is one bound on a location value.
//!
//! A variable is *scoped* when the rule reached it from a guest relation (`guest`, `parsed`, `doc_root`, `doc_role`,
//! `tape_of`, `host_node`, `string_annotation`): it is a location in a guest column of one, or it shares a base atom's location columns
//! with a scoped variable (they are in one document), or it is read from a derived relation's column that some rule
//! fills from a scoped variable. Every other variable in a positive base atom's location column gets the bound
//! (`n < bound << 32`, `d < bound`). The bound is a comparison on a value the join already has: no row is copied,
//! and a run that makes no guests is not changed at all.
//!
//! A scoped variable also knows its grammar when the guest relation names it (`parsed(s, "sql", r)`, `guest(g, _,
//! _, "sql")`, a tape from `@parse("sql", …)`). A kind or field constant on such a node is resolved in that grammar
//! (`"sql:select"`, `plan::lower::Constants`), so a Python policy can name SQL kinds inside its SQL guests.

use crate::code_rule_dsl::logic::front::{Atom, Literal, Name, Program, Term};
use rustc_hash::FxHashMap as HashMap;
use std::collections::{BTreeMap, BTreeSet};

/// The relations a rule enters a guest through, and their columns that are the guest's: (relation, guest location
/// columns, grammar column, tape column).
/// `string_annotation(s)` is one: Ruff's strings that are type definitions, nested ones too, whose nodes are in an
/// annotation's document (not shown, `guests::key`).
const ENTRIES: [(&str, &[usize], Option<usize>, Option<usize>); 10] = [
    ("guest", &[0], Some(3), None),
    ("string_annotation", &[0], None, None),
    ("parsed", &[2], Some(1), None),
    ("sub_tape", &[], Some(1), Some(2)),
    ("doc_root", &[0, 1], None, None),
    ("doc_role", &[0], None, None),
    ("tape_of", &[0], None, Some(1)),
    ("host_node", &[0], None, None),
    ("parse_of", &[], Some(1), Some(2)),
    ("splice_of", &[], None, Some(3)),
];

/// Views of base relations the lowering writes as rules.
const VIEWS: [&str; 3] = ["kind", "docof", "precedes"];

/// The grammar a guest's kinds are named in: a dialect label (`annotation` is Python's expression).
pub(super) fn dialect_label(grammar: &str) -> &str {
    if grammar == "annotation" { "python" } else { grammar }
}

/// The bound of `prog`'s shown documents: files, and the run language's guests unless `.guests off`.
pub(super) fn bound(prog: &Program) -> u32 {
    use crate::code_rule_dsl::facts::vtab::syntax::{ANNOTATION_KEYS, GUEST_KEYS, GUEST_POLICY};
    let off = prog.facts.get(GUEST_POLICY).into_iter().flatten().any(|row| {
        matches!(row.as_slice(), [crate::code_rule_dsl::logic::front::Value::Str(k), crate::code_rule_dsl::logic::front::Value::Str(v)] if k == "library" && v == "off")
    });
    if off { GUEST_KEYS } else { ANNOTATION_KEYS }
}

/// What the analysis knows of a variable: scoped, and its grammar (`Some("")` when two disagree). `definite`: it is
/// a guest's node, not only maybe one (O110). A variable read from a derived column that host rows fill too is
/// scoped (no bound on it) but not definite: it is specialized to a guest grammar only in a clone of its rule.
/// For a derived relation's column, `definite` says every rule fills it with a definite variable.
#[derive(Clone, Default, PartialEq)]
struct Fact {
    scoped: bool,
    grammar: Option<String>,
    definite: bool,
}

impl Fact {
    fn join(&mut self, other: &Fact) -> bool {
        let mut changed = (other.scoped && !self.scoped) || (other.definite && !self.definite);
        self.scoped |= other.scoped;
        self.definite |= other.definite;
        // Two grammars join to none known (`""`).
        match (&self.grammar, &other.grammar) {
            (_, None) => {}
            (None, Some(b)) => {
                self.grammar = Some(b.clone());
                changed = true;
            }
            (Some(a), Some(b)) if a == b || a.is_empty() => {}
            (Some(_), Some(_)) => {
                self.grammar = Some(String::new());
                changed = true;
            }
        }
        changed
    }
}

/// Bounds `prog`'s unscoped location variables to its shown documents (`bound`), and names the kinds and fields of
/// scoped nodes in their grammar.
pub(super) fn hide(prog: &mut Program, bound: u32) {
    // `kind`, `docof` and `precedes` are views of `node` and `child` the lowering writes as rules (`rr::lower`): they
    // are read as base relations, and their own rules are left whole.
    let views: BTreeSet<String> = VIEWS.iter().filter(|r| !prog.inputs.iter().any(|i| i == *r)).map(|r| (*r).to_owned()).collect();
    let view = |rel: &str| views.contains(rel);
    let base: BTreeSet<String> = prog
        .inputs
        .iter()
        .cloned()
        .chain(VIEWS.iter().filter(|r| view(r)).map(|r| (*r).to_owned()))
        .filter(|r| !ENTRIES.iter().any(|(e, ..)| e == r))
        .collect();
    let locations = |rel: &str| -> Vec<usize> {
        let mut cols: Vec<usize> = prog.decls.get(rel).map_or_else(Vec::new, |c| c.iter().enumerate().filter(|(_, (_, t))| t == "node").map(|(i, _)| i).collect());
        if let Some(c) = crate::code_rule_dsl::typing::file_local::document_column(rel)
            && !cols.contains(&c)
        {
            cols.push(c);
        }
        cols
    };
    let base_locations: HashMap<String, Vec<usize>> = base.iter().map(|r| (r.clone(), locations(r))).collect();
    // Each rule's seeds: a specialized copy's head column, scoped in its grammar.
    let mut seeds: Vec<BTreeMap<String, Fact>> = vec![BTreeMap::new(); prog.rules.len()];
    // A derived relation read with a scoped variable of a known grammar is specialized to it (decision 0058 §2, O98):
    // `rr_in_<grammar>_<column>_<relation>`, its rules copied with that head column seeded, so a node set's kinds are
    // the guest's grammar's and its guest rows are not bounded away. A kind set (`.kindset`) read by `nearest` over
    // such a node is copied as `rr_in_<grammar>_<set>`, with its kinds named in that grammar. To a fixpoint: a copy reads relations too.
    let mut made: BTreeSet<String> = BTreeSet::new();
    // Each copy's relation and seeded columns, with their grammars.
    let mut specialized: BTreeMap<String, (String, BTreeMap<usize, String>)> = BTreeMap::new();
    // Rules split once already: a variable of theirs that may be a guest's node has its clone (O110).
    let mut split: BTreeSet<usize> = BTreeSet::new();
    // The analysis kept across the rounds, each round's change applied to it (decision 0246 §3).
    let mut delta = Delta::new(prog, &seeds);
    // The rules the round before changed or added (every rule, at first).
    let mut touched: Vec<usize> = (0..prog.rules.len()).collect();
    loop {
        let _round = crate::code_rule_dsl::logic::profile::span(|| "hiding: a round".to_owned());
        let looked = delta.round(prog, &seeds, &base_locations, &view, &touched);
        let reaches = &delta.reaches;
        let carried = &delta.carried;
        // The round reads the rules as they were when it began, and records its edits, applied after it: a copy is
        // made from its original as it was (no snapshot of every rule each round, which was a fifth of hiding a pack).
        let mut edits: Vec<(usize, usize, Edit)> = Vec::new();
        // The relations rules derive: a relation of stated facts alone is a kind set when a kind is read through it.
        let ruled = &delta.heads;
        let mut copies: Vec<(crate::code_rule_dsl::logic::front::Rule, BTreeMap<String, Fact>)> = Vec::new();
        let mut decls: Vec<(crate::code_rule_dsl::logic::front::Name, crate::code_rule_dsl::logic::front::List<(crate::code_rule_dsl::logic::front::Name, crate::code_rule_dsl::logic::front::Name)>)> = Vec::new();
        // The axes over a guest grammar's kind set made this round: inputs the transducer makes, as the run's are.
        let mut axes: Vec<String> = Vec::new();
        let mut kindsets: Vec<(String, Vec<String>)> = Vec::new();
        // Kind sets read by value over a guest's kinds, copied in its grammar: (copy, original, kinds).
        let mut kind_facts: Vec<(String, String, Vec<String>)> = Vec::new();
        let mut changed = false;
        // Each rule's analysis reads the round's start alone, used in order below. Only the
        // rules `round` names: any other's analysis is the round before's, and its edits are made.
        let analysed: Vec<BTreeMap<String, Fact>> = looked
            .iter()
            .map(|&i| {
                let rule = &prog.rules[i];
                if view(&rule.head.rel) || !reaches[i] {
                    return BTreeMap::new();
                }
                analyse(rule, &base_locations, carried, &seeds[i]).into_iter().map(|(v, f)| (v.to_owned(), f)).collect()
            })
            .collect();
        for (i, facts) in looked.iter().copied().zip(analysed) {
            let rule = &prog.rules[i];
            if view(&rule.head.rel) {
                continue;
            }
            // A variable that may be a guest's node of a known grammar, and may be a host's: the rule stays as it is
            // for the host's, and a clone with the variable seeded as the guest's reads the guest's copies. Copying
            // in place of the atom would drop the host's rows (`tv`'s nodes are both, and `tbv`'s `live_asg` read
            // only the annotation copy, O110).
            let maybe: BTreeMap<String, Fact> = facts
                .iter()
                .filter(|(_, f)| f.scoped && !f.definite && f.grammar.as_ref().is_some_and(|g| !g.is_empty()))
                .map(|(v, f)| ((*v).to_owned(), Fact { definite: true, ..f.clone() }))
                .collect();
            if !maybe.is_empty() && split.insert(i) {
                let mut seed = seeds[i].clone();
                seed.extend(maybe);
                copies.push((rule.clone(), seed));
                changed = true;
            }
            let grammar_of = |v: &str| facts.get(v).filter(|f| f.scoped && f.definite).and_then(|f| f.grammar.clone()).filter(|g| !g.is_empty());
            // A kind read off a guest's node is its grammar's (decision 0222): `kind(f, k), fn_scope(k)` over a view's
            // node reads the kind set named in the view's grammar, and `k = "arrow_function"` names that grammar's
            // kind. Unqualified, both are the run language's, and a library over another grammar (tsx/names.rerail
            // read over a Vue component's view) finds none of its kinds.
            let kind_of: BTreeMap<String, String> = rule
                .body
                .iter()
                .filter_map(|l| match l {
                    Literal::Atom(a) if !a.neg => match (a.rel.as_str(), a.args.as_slice()) {
                        ("kind", [Term::Var(x), Term::Var(k)]) | ("node", [_, Term::Var(x), Term::Var(k), _, _]) => grammar_of(x.as_str()).map(|g| (k.to_string(), g)),
                        _ => None,
                    },
                    _ => None,
                })
                .collect();
            for (k, l) in rule.body.iter().enumerate() {
                if !kind_of.is_empty() {
                    match l {
                        Literal::Cmp { l: Term::Var(v), r: Term::Str(s), .. } | Literal::Cmp { l: Term::Str(s), r: Term::Var(v), .. }
                            if qualified(s.as_str()).is_none() && kind_of.contains_key(v.as_str()) =>
                        {
                            edits.push((i, k, Edit::Str(s.as_str().to_owned(), format!("{}:{s}", dialect_label(&kind_of[v.as_str()])))));
                            changed = true;
                            continue;
                        }
                        Literal::Atom(a)
                            if !a.rel.starts_with("rr_in_")
                                && !ruled.contains_key(a.rel.as_str())
                                && let [Term::Var(v)] = a.args.as_slice()
                                && let Some(g) = kind_of.get(v.as_str())
                                && let Some(rows) = prog.facts.get(a.rel.as_str()) =>
                        {
                            // Not `rr_in_<g>_<set>`, `nearest`'s copy of the set: that one is a kind set the transducer
                            // answers, and holds no rows.
                            let name = format!("rr_in_{}_rows_{}", g.replace(|ch: char| !ch.is_ascii_alphanumeric(), "_"), a.rel);
                            if made.insert(name.clone()) {
                                let label = dialect_label(g);
                                let names = rows.iter().filter_map(|row| match row.as_slice() {
                                    [crate::code_rule_dsl::logic::front::Value::Str(k)] => Some(k.as_str()),
                                    _ => None,
                                });
                                let kinds: Vec<String> = names.map(|k| if qualified(k).is_some() { k.to_owned() } else { format!("{label}:{k}") }).collect();
                                kind_facts.push((name.clone(), a.rel.to_string(), kinds));
                            }
                            edits.push((i, k, Edit::Rel(name)));
                            changed = true;
                            continue;
                        }
                        _ => {}
                    }
                }
                if let Literal::Nearest { n, kindset, .. } = l {
                    if let Some(g) = grammar_of(n)
                        && !kindset.starts_with("rr_in_")
                    {
                        let name = format!("rr_in_{g}_{kindset}");
                        if made.insert(name.clone()) {
                            let label = dialect_label(&g);
                            let kinds = prog.kindsets.get(kindset.as_str()).cloned().unwrap_or_default();
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
                if let Literal::Atom(a) | Literal::Agg { atom: a, .. } = l
                    && let Some((set, descendants)) = crate::code_rule_dsl::logic::front::axis(&a.rel)
                    && !set.starts_with("rr_in_")
                    && prog.kindsets.contains_key(set)
                    && let Some(g) = a.args.iter().find_map(|t| match t {
                        Term::Var(v) => grammar_of(v),
                        _ => None,
                    })
                {
                    let copy = format!("rr_in_{}_{set}", g.replace(|ch: char| !ch.is_ascii_alphanumeric(), "_"));
                    let prefix = if descendants { crate::code_rule_dsl::logic::front::DESCENDANTS } else { crate::code_rule_dsl::logic::front::AXIS };
                    let name = format!("{prefix}{copy}");
                    if made.insert(name.clone()) {
                        let label = dialect_label(&g);
                        let kinds = prog.kindsets.get(set).cloned().unwrap_or_default();
                        kindsets.push((copy, kinds.into_iter().map(|k| if qualified(&k).is_some() { k } else { format!("{label}:{k}") }).collect()));
                        if let Some(d) = prog.decls.get(a.rel.as_str()) {
                            decls.push((name.clone().into(), d.clone()));
                        }
                        axes.push(name.clone());
                    }
                    edits.push((i, k, Edit::Rel(name)));
                    changed = true;
                    continue;
                }
                let a = match l {
                    Literal::Atom(a) | Literal::Agg { atom: a, .. } => a,
                    _ => continue,
                };
                let Some(cols) = carried.get(a.rel.as_str()) else { continue };
                // A column scoped already is copied too where it knows no grammar, or another: a util whose node
                // reaches a guest relation (`host_node(p, h)` over a view's node, decision 0222) is scoped there, and
                // read with a view's node of a known grammar it must name that grammar's kinds.
                let spec = a.args.iter().enumerate().find_map(|(c, t)| match t {
                    Term::Var(v) => grammar_of(v).filter(|g| !cols.get(c).is_some_and(|f| f.scoped && f.grammar.as_deref() == Some(g.as_str()))).map(|g| (c, g)),
                    _ => None,
                });
                let Some((c, g)) = spec else { continue };
                // A copy read with another column scoped is a copy of the same relation with both seeded, not a copy
                // of the copy: one per relation and set of seeded columns, so the copies end.
                let (orig, mut cols) = specialized.get(a.rel.as_str()).cloned().unwrap_or_else(|| (a.rel.to_string(), BTreeMap::new()));
                cols.insert(c, g);
                let name = format!("rr_in_{}{orig}", cols.iter().map(|(c, g)| format!("{}_{c}_", g.replace(|ch: char| !ch.is_ascii_alphanumeric(), "_"))).collect::<String>());
                if made.insert(name.clone()) {
                    for &j in delta.heads.get(orig.as_str()).into_iter().flatten() {
                        let mut copy = prog.rules[j].clone();
                        copy.head.rel = name.clone().into();
                        let mut seed = BTreeMap::new();
                        for (c, g) in &cols {
                            if let Some(Term::Var(v)) = copy.head.args.get(*c) {
                                seed.insert(v.to_string(), Fact { scoped: true, grammar: Some(g.clone()), definite: true });
                            }
                        }
                        copies.push((copy, seed));
                    }
                    if let Some(d) = prog.decls.get(orig.as_str()) {
                        decls.push((name.clone().into(), d.clone()));
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
            match (&mut prog.rules[i].body.make_mut()[k], edit) {
                (Literal::Nearest { kindset, .. }, Edit::Kindset(name)) => *kindset = name,
                (Literal::Atom(a) | Literal::Agg { atom: a, .. }, Edit::Rel(name)) => a.rel = name.into(),
                // The string side of the comparison the edit was recorded for: the one that named `from`.
                (Literal::Cmp { l, r, .. }, Edit::Str(from, to)) => {
                    for t in [l, r] {
                        if matches!(t, Term::Str(s) if s.as_str() == from) {
                            *t = Term::Str(to.clone().into());
                        }
                    }
                }
                _ => unreachable!("an edit is of the literal it was recorded for"),
            }
        }
        let first_copy = prog.rules.len();
        for (rule, seed) in copies {
            prog.rules.push(rule);
            seeds.push(seed);
        }
        touched = edited.into_iter().chain(first_copy..prog.rules.len()).collect();
        delta.changed(prog, &touched);
        prog.decls.extend(decls);
        prog.kindsets.extend(kindsets);
        for (name, orig, kinds) in kind_facts {
            if let Some(d) = prog.decls.get(orig.as_str()).cloned() {
                prog.decls.insert(name.as_str().into(), d);
            }
            prog.facts.insert(name, kinds.iter().map(|k| [crate::code_rule_dsl::logic::front::Value::Str(k.as_str().into())].into()).collect());
        }
        for axis in axes {
            if !prog.inputs.contains(&axis) {
                prog.inputs.push(axis);
            }
        }
        if !changed {
            break;
        }
    }
    let bounding = crate::code_rule_dsl::logic::profile::span(|| "hiding: each rule bounded".to_owned());
    let carried = std::mem::take(&mut delta.carried);
    drop(delta);
    let bound_of = |doc_column: bool| if doc_column { i64::from(bound) } else { i64::from(bound) << 32 };
    let reaches = reaching(prog, &seeds);
    // What `parsed` asks for (`rr_embed_req`, and the contexts `rr_ctx<k>` only it reads) is not bounded: asking is
    // not seeing. A string in an annotation is in a document not shown, and its rule's demand is made of inputs
    // alone (`rr_ctx3(s) :- kind(s, "string")`), so bounded, the string inside `"Opt['Baz']"` was never asked for,
    // and no rule reached it through the annotation either.
    let asking = asking(prog);
    let asks = |rel: &str| asking.contains(rel);
    // Each rule bounded on its own.
    prog.rules.iter_mut().zip(&seeds).zip(reaches).filter(|((r, _), _)| !VIEWS.contains(&r.head.rel.as_str()) && !asks(&r.head.rel)).for_each(|((rule, seed), reaches)| {
        let facts: BTreeMap<String, Fact> = if reaches { analyse(rule, &base_locations, &carried, seed).into_iter().map(|(v, f)| (v.to_owned(), f)).collect() } else { BTreeMap::new() };
        // Wildcards in an unscoped base atom's location columns become variables, so they can be bounded.
        let mut fresh = 0;
        let mut bounded: BTreeMap<String, bool> = BTreeMap::new();
        for l in &mut rule.body {
            let Literal::Atom(a) = l else { continue };
            let Some(cols) = base_locations.get(a.rel.as_str()) else { continue };
            // A negation over a guest's node names its kind in the guest's grammar too (`!kind(d, "import_specifier")`
            // over a view's node, decision 0222); it bounds nothing.
            if a.neg {
                let grammar = cols.iter().find_map(|&c| match a.args.get(c) {
                    Some(Term::Var(v)) => facts.get(v.as_str()).filter(|f| f.scoped && f.definite).and_then(|f| f.grammar.clone()).filter(|g| !g.is_empty()),
                    _ => None,
                });
                if let Some(g) = grammar {
                    qualify(a, dialect_label(&g));
                }
                continue;
            }
            let doc_col = crate::code_rule_dsl::typing::file_local::document_column(&a.rel);
            let atom_scoped = cols.iter().any(|&c| matches!(a.args.get(c), Some(Term::Var(v)) if facts.get(v.as_str()).is_some_and(|f| f.scoped)));
            if atom_scoped {
                // Named in the guest's grammar only where the node is the guest's (a clone's seed), not maybe one.
                let grammar = cols.iter().find_map(|&c| match a.args.get(c) {
                    Some(Term::Var(v)) => facts.get(v.as_str()).filter(|f| f.definite).and_then(|f| f.grammar.clone()).filter(|g| !g.is_empty()),
                    _ => None,
                });
                if let Some(g) = grammar {
                    qualify(a, dialect_label(&g));
                }
                continue;
            }
            for &c in cols {
                match a.args.make_mut().get_mut(c) {
                    Some(Term::Var(v)) => {
                        bounded.entry(v.to_string()).or_insert(Some(c) == doc_col);
                    }
                    Some(t @ Term::Wild) => {
                        let v = format!("rr_shown{fresh}");
                        fresh += 1;
                        *t = Term::Var(v.clone().into());
                        bounded.insert(v, Some(c) == doc_col);
                    }
                    _ => {}
                }
            }
        }
        for (v, doc_column) in bounded {
            rule.body.push(Literal::Cmp { l: Term::Var(v.into()), op: "<", r: Term::Int(bound_of(doc_column)) });
        }
    });
    drop(bounding);
    // An aggregate's atom is its own scope: one over a base atom reads it through a bounded helper,
    // `rr_shown_agg<k>(vars) :- atom, bounds`.
    let mut helpers: Vec<(Atom, Vec<(String, String)>, Vec<Literal>)> = Vec::new();
    for rule in prog.rules.iter_mut().filter(|r| !VIEWS.contains(&r.head.rel.as_str())) {
        for l in &mut rule.body {
            let Literal::Agg { atom, .. } = l else { continue };
            let Some(cols) = base_locations.get(atom.rel.as_str()) else { continue };
            if cols.is_empty() {
                continue;
            }
            let decl = prog.decls.get(atom.rel.as_str()).cloned().unwrap_or_default();
            let doc_col = crate::code_rule_dsl::typing::file_local::document_column(&atom.rel);
            // Every wildcard becomes a column of the helper: the aggregate counts the atom's rows, not their
            // projection.
            let mut inner = atom.clone();
            for (c, t) in inner.args.iter_mut().enumerate() {
                if matches!(t, Term::Wild) {
                    *t = Term::Var(format!("rr_shown_agg_c{c}").into());
                }
            }
            let bounds: Vec<Literal> = cols
                .iter()
                .filter_map(|&c| match inner.args.get(c) {
                    Some(Term::Var(v)) => Some(Literal::Cmp { l: Term::Var(v.clone()), op: "<", r: Term::Int(bound_of(Some(c) == doc_col)) }),
                    _ => None,
                })
                .collect();
            let mut vars: Vec<(String, String)> = Vec::new();
            for (c, t) in inner.args.iter().enumerate() {
                if let Term::Var(v) = t
                    && !vars.iter().any(|(x, _)| x == v)
                {
                    vars.push((v.to_string(), decl.get(c).map_or_else(|| "int".to_owned(), |(_, ty)| ty.to_string())));
                }
            }
            let name = format!("rr_shown_agg{}", helpers.len());
            let head = Atom { rel: name.into(), args: vars.iter().map(|(v, _)| Term::Var(v.clone().into())).collect(), neg: false, gated: false };
            // The aggregate's own wildcards stay wildcards: its named variables are correlated.
            let args = head.args.iter().map(|t| match t {
                Term::Var(v) if v.starts_with("rr_shown_agg_c") => Term::Wild,
                t => t.clone(),
            }).collect();
            *atom = Atom { rel: head.rel.clone(), args, neg: atom.neg, gated: false };
            helpers.push((head, vars, std::iter::once(Literal::Atom(inner)).chain(bounds).collect()));
        }
    }
    for (head, vars, body) in helpers {
        prog.decls.insert(head.rel.clone(), vars.iter().map(|(c, t)| (c.into(), t.into())).collect());
        prog.rules.push(crate::code_rule_dsl::logic::front::Rule { head, body: body.into() });
    }
}

/// The guest demand (`guests::DEMAND`) and the contexts (`rr_ctx<k>`, `rr::lower`) that only it reads, transitively.
fn asking(prog: &Program) -> BTreeSet<String> {
    // A combined run's program prefix (`p<i>_`, `corpus::combine`) comes first.
    let context = |r: &str| {
        let own = r.strip_prefix('p').and_then(|t| t.find('_').filter(|&i| i > 0 && t[..i].bytes().all(|b| b.is_ascii_digit())).map(|i| &t[i + 1..]));
        own.unwrap_or(r).starts_with("rr_ctx")
    };
    let mut asking: BTreeSet<String> = BTreeSet::from([super::guests::DEMAND.to_owned()]);
    loop {
        let more: Vec<String> = prog
            .decls
            .keys()
            .filter(|r| context(r) && !asking.contains(r.as_str()))
            .filter(|r| {
                let mut readers = prog.rules.iter().filter(|x| x.body.iter().any(|l| l.atom().is_some_and(|a| a.rel == r.as_str()))).peekable();
                readers.peek().is_some() && readers.all(|x| asking.contains(x.head.rel.as_str()))
            })
            .map(|r| r.to_string())
            .collect();
        if more.is_empty() {
            return asking;
        }
        asking.extend(more);
    }
}

/// Folds the copies hiding made for guests in the run's own language back into their originals, once the run's
/// language (`label`) is known (decision 0058). A code guest's documents are shown (`GUEST_KEYS..ANNOTATION_KEYS`,
/// `guests::key`) and its kinds are the run's (`python:x` resolves as `x`), so a copy specialized to it holds its
/// original's rows: its rules differ only by the seeded column's bound, which those documents are under, and by the
/// qualified kinds. An annotation's documents are not shown (decision 0165): the original of a copy made for one is
/// widened to take them in, and the rules that read it and are not its own read it as hiding left it (`keep_narrow`).
/// Its readers read the original, its rules go, and a qualified kind of `label` is written plainly; a copied kind set
/// goes with its readers, `nearest`'s and the tree axes' (`rr_inside_rr_in_<g>_<set>` reads `rr_inside_<set>`); a clone
/// (O110) is then its rule again, dropped as a repeat by `merge_and_drop_repeats`. With guests not shown (`bound`,
/// `.guests off`) nothing is folded. Gives the copies folded. `RERAIL_SHOWN_COPIES=on` keeps them (to measure what
/// they cost).
pub(super) fn fold_own(prog: &mut Program, label: &str, bound: u32) -> usize {
    use crate::code_rule_dsl::facts::vtab::syntax::ANNOTATION_KEYS;
    if bound != ANNOTATION_KEYS || std::env::var("RERAIL_SHOWN_COPIES").is_ok_and(|v| v == "on") {
        return 0;
    }
    // `rr_in_<g>_<c>_…<orig>`, every `<g>` the run's own (as `hide` names them: `g` with non-alphanumerics as `_`).
    let grammars: Vec<&str> = ["annotation", label].into_iter().filter(|g| dialect_label(g) == label).collect();
    let fold = Fold::of(prog, &grammars);
    if fold.is_empty() {
        return 0;
    }
    fold.apply(prog, label);
    fold.folded.len() + fold.sets.len()
}

/// What `fold_own` folds: each copied relation's original (`folded`), and each copied kind set's (`sets`).
struct Fold {
    folded: HashMap<String, String>,
    sets: HashMap<String, String>,
}

impl Fold {
    fn of(prog: &Program, grammars: &[&str]) -> Fold {
        let folded = prog.decls.keys().filter_map(|rel| copied_relation(prog, grammars, rel).map(|orig| (rel.to_string(), orig.to_owned()))).collect();
        let sets = prog.kindsets.keys().filter_map(|k| copied_set(prog, grammars, k).map(|set| (k.clone(), set.to_owned()))).collect();
        Fold { folded, sets }
    }

    fn is_empty(&self) -> bool {
        self.folded.is_empty() && self.sets.is_empty()
    }

    /// A tree axis over a copied set (`hide`'s `rr_inside_rr_in_<g>_<set>`, `rr_has_…`) reads the original set's axis:
    /// the copied set goes, and an axis left on it names no set, so the transducer made it with no rows and its
    /// negation held for every node (`!inside(t, _, S)` and `within_stop` in a clone over an annotation's node).
    fn axis(&self, rel: &str) -> Option<String> {
        use crate::code_rule_dsl::logic::front::{axis, AXIS, DESCENDANTS};
        let (set, has) = axis(rel)?;
        self.sets.get(set).map(|orig| format!("{}{orig}", if has { DESCENDANTS } else { AXIS }))
    }

    fn apply(&self, prog: &mut Program, label: &str) {
        prog.rules.retain(|r| !self.folded.contains_key(r.head.rel.as_str()));
        keep_narrow(prog, &self.annotated());
        self.widen_originals(prog);
        let plain = format!("{label}:");
        for rule in &mut prog.rules {
            for l in &mut rule.body {
                self.literal(l, &plain);
            }
        }
        for rel in self.folded.keys() {
            prog.decls.remove(rel.as_str());
            prog.qualifiers.remove(rel);
        }
        for k in self.sets.keys() {
            prog.kindsets.remove(k);
        }
        self.fold_axes(prog);
    }

    /// The originals of copies made for an annotation. A copy for a code guest of the run's language holds rows under
    /// the narrow bound already; one for an annotation holds an annotation's, which its original takes in once widened.
    fn annotated(&self) -> BTreeSet<&str> {
        let for_annotation = |copy: &str, orig: &str| {
            let mid = &copy["rr_in_".len()..copy.len() - orig.len()];
            mid.starts_with("annotation_") || mid.contains("_annotation_")
        };
        self.folded.iter().filter(|(copy, orig)| for_annotation(copy, orig)).map(|(_, orig)| orig.as_str()).collect()
    }

    /// An original holds its copies' rows: the bounds of an annotation's take the annotations in (`ANNOTATION_KEYS` to
    /// `HIDDEN_KEYS`), as its copies' seeded columns did. A rule that does not reach an annotation keeps the bound.
    fn widen_originals(&self, prog: &mut Program) {
        let originals = self.annotated();
        for rule in prog.rules.iter_mut().filter(|r| originals.contains(r.head.rel.as_str())) {
            for l in &mut rule.body {
                widen(l);
            }
        }
    }

    /// A literal reads the originals, and a kind of the run's language (`plain`, `python:`) is written plainly.
    fn literal(&self, l: &mut Literal, plain: &str) {
        match l {
            Literal::Atom(a) | Literal::Agg { atom: a, .. } => {
                self.rename(a);
                unqualify(a, plain);
            }
            Literal::Nearest { kindset, .. } => {
                if let Some(set) = self.sets.get(kindset.as_str()) {
                    *kindset = set.clone();
                }
            }
            _ => {}
        }
    }

    fn rename(&self, a: &mut Atom) {
        if let Some(orig) = self.folded.get(a.rel.as_str()) {
            a.rel = orig.clone().into();
        } else if let Some(orig) = self.axis(&a.rel) {
            a.rel = orig.into();
        }
    }

    /// The axes the copied sets name: `hide` lists each as an input, and declares it where its original was declared.
    /// Each is declared and listed as its original.
    fn fold_axes(&self, prog: &mut Program) {
        let copied: BTreeMap<String, String> = prog.decls.keys().map(|r| r.as_str()).chain(prog.inputs.iter().map(String::as_str)).filter_map(|rel| self.axis(rel).map(|orig| (rel.to_owned(), orig))).collect();
        for (copy, orig) in &copied {
            if let Some(d) = prog.decls.remove(copy.as_str()) {
                prog.decls.entry(orig.as_str().into()).or_insert(d);
            }
        }
        for list in [&mut prog.inputs, &mut prog.outputs, &mut prog.demand] {
            self.relist(list, &copied);
        }
    }

    /// A list of relations names the originals; an axis copy's place is its original's, unless the original is listed
    /// already.
    fn relist(&self, list: &mut Vec<String>, copied: &BTreeMap<String, String>) {
        for rel in list.iter_mut() {
            if let Some(orig) = self.folded.get(rel.as_str()) {
                *rel = orig.clone();
            }
        }
        for (copy, orig) in copied {
            let Some(i) = list.iter().position(|r| r == copy) else { continue };
            if list.contains(orig) {
                list.remove(i);
            } else {
                list[i] = orig.clone();
            }
        }
    }
}

/// The name of `orig` as hiding left it, for the rules that read it and are not its own (`keep_narrow`).
fn narrow_name(orig: &str) -> String {
    format!("rr_narrow_{orig}")
}

/// Keeps the originals `widened` as hiding left them for the rules that read them and are not their own (decision
/// 0165 §3). Such a rule reached no annotation (it would read a copy), so it reads the original under the narrow
/// bound, `rr_narrow_<orig>`: the original's rules before they are widened, each reading the others' narrow ones too.
/// Without it a rule's rows depended on what else the run held: a program whose rules read a copy widened the original
/// another program's rule read, and that rule saw the annotations (RUF060 found `x: "1 in []"` beside C417 only).
fn keep_narrow(prog: &mut Program, widened: &BTreeSet<&str>) {
    let narrowing = Narrowing { widened, asking: asking(prog) };
    let read = narrowing.reads(prog);
    let narrow: Vec<_> = prog.rules.iter().filter(|r| read.contains(r.head.rel.as_str())).cloned().collect();
    prog.rules.extend(narrow.into_iter().map(|mut r| {
        r.head.rel = narrow_name(&r.head.rel).into();
        r
    }));
    for orig in &read {
        declare_narrow(prog, orig);
    }
    narrowing.redirect(prog, &read);
}

/// A literal that reads an original kept narrow (`read`) reads its narrow copy.
fn read_narrow(l: &mut Literal, read: &BTreeSet<String>) {
    if let Literal::Atom(a) | Literal::Agg { atom: a, .. } = l
        && read.contains(a.rel.as_str())
    {
        a.rel = narrow_name(&a.rel).into();
    }
}

/// Which rules read the widened originals narrow (`keep_narrow`).
struct Narrowing<'w> {
    widened: &'w BTreeSet<&'w str>,
    /// The guest demand and the contexts only it reads (`asking`).
    asking: BTreeSet<String>,
}

impl Narrowing<'_> {
    /// Whether the rules of `head` read the originals narrow: not a widened original's own, and not asking for what
    /// every reader is answered from.
    fn sees(&self, head: &str) -> bool {
        !self.widened.contains(head) && !self.asks(head)
    }

    /// Whether `rel` is kept narrow for the rules that read it narrow.
    fn narrowed(&self, rel: &str) -> bool {
        self.widened.contains(rel) && !self.asks(rel)
    }

    /// Whether `rel` asks for what every reader is answered from, so it reads the originals widened: a guest request
    /// and the contexts only it reads (`guests::REQUESTS`, `asking`; asking is not seeing, decision 0165 §4), or the
    /// on-request rewrite's attributes, their requests, eager halves and widened copies (decision 0174). Read narrow, a
    /// request asks too little for the widened original's readers.
    fn asks(&self, rel: &str) -> bool {
        use crate::code_rule_dsl::logic::rr::lower::{made_on_request, on_request_rule};
        self.asking.contains(rel) || super::guests::REQUESTS.contains(&rel) || on_request_rule(rel) || made_on_request(rel)
    }

    /// The originals kept narrow that a rule reading them narrow reads, and those their rules read in turn.
    fn reads(&self, prog: &Program) -> BTreeSet<String> {
        let mut rules_of: HashMap<&str, Vec<&crate::code_rule_dsl::logic::front::Rule>> = HashMap::default();
        for r in &prog.rules {
            rules_of.entry(r.head.rel.as_str()).or_default().push(r);
        }
        let mut todo: Vec<_> = prog.rules.iter().filter(|r| self.sees(r.head.rel.as_str())).collect();
        let mut read = BTreeSet::new();
        while let Some(rule) = todo.pop() {
            for rel in rule.body.iter().filter_map(Literal::atom).map(|a| a.rel.as_str()).filter(|r| self.narrowed(r)) {
                if read.insert(rel.to_owned()) {
                    todo.extend(rules_of.get(rel).into_iter().flatten().copied());
                }
            }
        }
        read
    }

    /// Points the rules that read narrow, the narrow copies' among them, at the narrow copies of the originals they
    /// read (`read`), and their native operators too: a native's rows are made from the relations its fields name,
    /// its rule only orders it after them.
    fn redirect(&self, prog: &mut Program, read: &BTreeSet<String>) {
        let reads = |r: &crate::code_rule_dsl::logic::front::Rule| r.body.iter().filter_map(Literal::atom).any(|a| read.contains(a.rel.as_str()));
        for rule in prog.rules.iter_mut().filter(|r| self.sees(r.head.rel.as_str()) && reads(r)) {
            for l in &mut rule.body {
                read_narrow(l, read);
            }
        }
        let rename = |r: &str| if read.contains(r) { narrow_name(r) } else { r.to_owned() };
        for (_, native) in prog.natives.iter_mut().filter(|(head, _)| self.sees(head)) {
            native.rename_reads(rename);
        }
    }
}

/// What the program says of `orig` by name, said of `rr_narrow_<orig>` too: its declaration, qualifiers, tape
/// columns, lattice, facts, native operator, join shapes, demand and the request rules that serve its readers.
fn declare_narrow(prog: &mut Program, orig: &str) {
    let name = narrow_name(orig);
    copy_entry(&mut prog.decls, orig, &name);
    copy_entry(&mut prog.qualifiers, orig, &name);
    copy_entry(&mut prog.tapes, orig, &name);
    copy_entry(&mut prog.lattice, orig, &name);
    copy_entry(&mut prog.facts, orig, &name);
    copy_entry(&mut prog.natives, orig, &name);
    if prog.nullary.contains(orig) {
        prog.nullary.insert(name.clone());
    }
    if prog.demand.iter().any(|d| d == orig) {
        prog.demand.push(name.clone());
    }
    let shapes: Vec<_> = prog.shapes.iter().filter(|s| s.head == orig).map(|s| crate::code_rule_dsl::logic::front::Shape { head: name.clone(), ..s.clone() }).collect();
    prog.shapes.extend(shapes);
    for by in prog.asked_by.values_mut().flatten().filter(|by| by.contains(orig)) {
        by.insert(name.clone());
    }
}

/// `map`'s entry of `from`, copied to `to`.
fn copy_entry<K: Ord + std::borrow::Borrow<str> + From<String>, V: Clone>(map: &mut BTreeMap<K, V>, from: &str, to: &str) {
    if let Some(v) = map.get(from).cloned() {
        map.insert(to.to_owned().into(), v);
    }
}

/// The original of `rel`, a copy `hide` made for guests of the run's own grammars (`rr_in_<g>_<c>_…<orig>`): the longest
/// original that leaves an own prefix.
fn copied_relation<'p>(prog: &Program, grammars: &[&str], rel: &'p str) -> Option<&'p str> {
    let tail = rel.strip_prefix("rr_in_")?;
    (0..tail.len())
        .filter(|&i| tail.is_char_boundary(i))
        .map(|i| tail.split_at(i))
        .find(|(mid, orig)| !orig.starts_with("rr_in_") && prog.decls.contains_key(*orig) && own_mid(grammars, mid))
        .map(|(_, orig)| orig)
}

/// The original of kind set `k`, a copy `hide` made for a guest of the run's own grammars (`rr_in_<g>_<set>`).
fn copied_set<'p>(prog: &Program, grammars: &[&str], k: &'p str) -> Option<&'p str> {
    let tail = k.strip_prefix("rr_in_")?;
    own_grammar(grammars, tail).filter(|set| prog.kindsets.contains_key(*set))
}

/// `s` less a leading `<g>_`, `g` one of `grammars`.
fn own_grammar<'s>(grammars: &[&str], s: &'s str) -> Option<&'s str> {
    grammars.iter().find_map(|g| s.strip_prefix(*g).and_then(|r| r.strip_prefix('_')))
}

/// Whether `mid` is a copy's prefix of own grammars and seeded columns, `<g>_<c>_` once or more.
fn own_mid(grammars: &[&str], mut mid: &str) -> bool {
    if mid.is_empty() {
        return false;
    }
    while !mid.is_empty() {
        let Some(rest) = own_grammar(grammars, mid) else { return false };
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        let Some(rest) = rest[digits..].strip_prefix('_').filter(|_| digits > 0) else { return false };
        mid = rest;
    }
    true
}

/// A bound of the shown annotations (`ANNOTATION_KEYS`, a document's or a node's) widened to every annotation's
/// (`HIDDEN_KEYS`).
fn widen(l: &mut Literal) {
    use crate::code_rule_dsl::facts::vtab::syntax::{ANNOTATION_KEYS, HIDDEN_KEYS};
    let Literal::Cmp { op: "<", r: Term::Int(b), .. } = l else { return };
    let (narrow, wide) = (i64::from(ANNOTATION_KEYS), i64::from(HIDDEN_KEYS));
    if *b == narrow << 32 {
        *b = wide << 32;
    } else if *b == narrow {
        *b = wide;
    }
}

/// A kind or field constant of `plain`'s language (`python:identifier`) written plainly.
fn unqualify(a: &mut Atom, plain: &str) {
    let col = match a.rel.as_str() {
        "kind" | "field" => Some(1),
        "node" => Some(2),
        _ => None,
    };
    if let Some(Term::Str(s)) = col.and_then(|c| a.args.make_mut().get_mut(c))
        && let Some(name) = s.strip_prefix(plain)
    {
        *s = name.to_owned().into();
    }
}

/// Which rules' analysis can find anything (O-combine-hiding-single): one with a seed, or a positive atom of a guest
/// relation (`ENTRIES`) or of a relation such a rule makes, transitively. Any other rule reads no scoped column and
/// enters no guest, so its analysis is empty, and is not made: in a policy most rules are such.
fn reaching(prog: &Program, seeds: &[BTreeMap<String, Fact>]) -> Vec<bool> {
    // A worklist over each relation's readers: a rule is looked at once, and again only when it is marked.
    let mut readers: HashMap<&str, Vec<usize>> = HashMap::default();
    let mut out = vec![false; prog.rules.len()];
    let mut work = Vec::new();
    for (i, rule) in prog.rules.iter().enumerate() {
        let mut enters = seeds.get(i).is_some_and(|s| !s.is_empty());
        for l in &rule.body {
            if let Literal::Atom(a) = l
                && !a.neg
            {
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
        let head = prog.rules[i].head.rel.as_str();
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

/// A round's edit of one literal, applied once the round has read every rule.
enum Edit {
    Kindset(String),
    Rel(String),
    /// A comparison's kind name, qualified in a guest's grammar: (as written, qualified).
    Str(String, String),
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
    heads: HashMap<Name, Vec<usize>>,
    /// The rules reading each relation in a positive atom: what reaching and the fixpoints follow. An edited rule is
    /// listed under the relation it read too (a superset: following it looks at one rule more, and changes nothing).
    readers: HashMap<Name, Vec<usize>>,
    /// The rules reading each relation in any literal: whose analysis a change of its columns changes.
    any_readers: HashMap<Name, Vec<usize>>,
    reaches: Vec<bool>,
    /// The relations a rule reaching a guest derives.
    made: std::collections::HashSet<Name>,
    /// What the columns gain, the first fixpoint's result.
    gained: HashMap<Name, Vec<Fact>>,
    /// What the columns carry: the gain, with which columns are definite.
    carried: HashMap<Name, Vec<Fact>>,
    /// The rules listed so far.
    listed: usize,
    first: bool,
}

impl Delta {
    fn new(prog: &Program, _seeds: &[BTreeMap<String, Fact>]) -> Delta {
        let mut d = Delta { heads: HashMap::default(), readers: HashMap::default(), any_readers: HashMap::default(), reaches: Vec::new(), made: Default::default(), gained: HashMap::default(), carried: HashMap::default(), listed: 0, first: true };
        d.list(prog, &(0..prog.rules.len()).collect::<Vec<_>>());
        d
    }

    /// Lists rules `rules` (new ones, or edited) under their head and the relations they read.
    fn list(&mut self, prog: &Program, rules: &[usize]) {
        for &i in rules {
            let rule = &prog.rules[i];
            if i >= self.listed {
                self.heads.entry(rule.head.rel.clone()).or_default().push(i);
            }
            for l in &rule.body {
                let (Literal::Atom(a) | Literal::Agg { atom: a, .. }) = l else { continue };
                let any = self.any_readers.entry(a.rel.clone()).or_default();
                if !any.contains(&i) {
                    any.push(i);
                }
                if let Literal::Atom(a) = l
                    && !a.neg
                {
                    let list = self.readers.entry(a.rel.clone()).or_default();
                    if !list.contains(&i) {
                        list.push(i);
                    }
                }
            }
        }
        self.listed = self.listed.max(prog.rules.len());
    }

    /// The round's edits made (rules `touched`, edited or new).
    fn changed(&mut self, prog: &Program, touched: &[usize]) {
        self.list(prog, touched);
    }

    /// Brings the analysis to the rules as the round begins, `touched` being those changed since the round before (every
    /// rule at first); returns the rules the round analyses, in order.
    fn round(&mut self, prog: &Program, seeds: &[BTreeMap<String, Fact>], base: &HashMap<String, Vec<usize>>, view: &(dyn Fn(&str) -> bool + Sync), touched: &[usize]) -> Vec<usize> {
        let first = std::mem::replace(&mut self.first, false);
        // Reaching a guest, from the rules changed: one that enters, or reads a relation a reaching rule derives.
        let reach_span = crate::code_rule_dsl::logic::profile::span(|| "hiding: the rules reaching a guest".to_owned());
        self.reaches.resize(prog.rules.len(), false);
        let mut newly: Vec<usize> = Vec::new();
        let mut work: Vec<usize> = Vec::new();
        for &i in touched {
            let rule = &prog.rules[i];
            let enters = seeds.get(i).is_some_and(|s| !s.is_empty())
                || rule.body.iter().any(|l| matches!(l, Literal::Atom(a) if !a.neg && (ENTRIES.iter().any(|(e, ..)| *e == a.rel) || self.made.contains(a.rel.as_str()))));
            if enters && !self.reaches[i] {
                self.reaches[i] = true;
                newly.push(i);
                work.push(i);
            }
        }
        while let Some(i) = work.pop() {
            let head = &prog.rules[i].head.rel;
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
        drop(reach_span);
        let _carry_span = crate::code_rule_dsl::logic::profile::span(|| "hiding: what the columns carry".to_owned());
        for r in touched.iter().map(|&i| &prog.rules[i]) {
            if !self.gained.contains_key(r.head.rel.as_str()) && !view(&r.head.rel) {
                let n = prog.decls.get(r.head.rel.as_str()).map_or(0, |c| c.len());
                self.gained.insert(r.head.rel.clone(), vec![Fact::default(); n]);
            }
        }
        // What the columns gain, from the round before's, starting at the rules changed.
        let start: BTreeSet<usize> = touched.iter().chain(&newly).copied().collect();
        let mut grew: BTreeSet<Name> = BTreeSet::new();
        fixpoint(prog, base, seeds, &self.reaches, &self.readers, &mut self.gained, start.iter().copied().collect(), &mut grew, &|facts, t, slot| match t {
            Term::Var(v) => facts.get(v.as_str()).is_some_and(|f| slot.join(f)),
            _ => false,
        });
        // Which columns are definite, made again over the cone of the change.
        let mut cone: BTreeSet<Name> = BTreeSet::new();
        if first {
            cone.extend(self.gained.keys().cloned());
        } else {
            let mut work: Vec<Name> = start.iter().map(|&i| prog.rules[i].head.rel.clone()).chain(grew.iter().cloned()).filter(|r| self.gained.contains_key(r.as_str())).collect();
            while let Some(rel) = work.pop() {
                if !cone.insert(rel.clone()) {
                    continue;
                }
                for &j in self.readers.get(rel.as_str()).into_iter().flatten() {
                    let head = &prog.rules[j].head.rel;
                    if self.gained.contains_key(head.as_str()) && !cone.contains(head.as_str()) {
                        work.push(head.clone());
                    }
                }
            }
        }
        let mut before: HashMap<Name, Vec<Fact>> = HashMap::default();
        for rel in &cone {
            let mut cols = self.gained[rel.as_str()].clone();
            for f in cols.iter_mut() {
                f.definite = f.scoped;
            }
            if let Some(old) = self.carried.insert(rel.clone(), cols) {
                before.insert(rel.clone(), old);
            }
        }
        let rules: Vec<usize> = cone.iter().flat_map(|r| self.heads.get(r.as_str()).into_iter().flatten().copied()).collect::<BTreeSet<usize>>().into_iter().collect();
        let mut lost: BTreeSet<Name> = BTreeSet::new();
        fixpoint(prog, base, seeds, &self.reaches, &self.readers, &mut self.carried, rules, &mut lost, &|facts, t, slot| {
            let definite = matches!(t, Term::Var(v) if facts.get(v.as_str()).is_some_and(|f| f.definite));
            if slot.definite && !definite {
                slot.definite = false;
                true
            } else {
                false
            }
        });
        if first {
            return (0..prog.rules.len()).collect();
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
    prog: &Program,
    base: &HashMap<String, Vec<usize>>,
    seeds: &[BTreeMap<String, Fact>],
    reaches: &[bool],
    readers: &HashMap<Name, Vec<usize>>,
    carried: &mut HashMap<Name, Vec<Fact>>,
    start: Vec<usize>,
    changed: &mut BTreeSet<Name>,
    step: &(dyn Fn(&BTreeMap<&str, Fact>, &Term, &mut Fact) -> bool + Sync),
) {
    let mut round = start;
    while !round.is_empty() {
        let now: &HashMap<Name, Vec<Fact>> = carried;
        let analysed: Vec<BTreeMap<&str, Fact>> = round
            .iter()
            .map(|&i| if reaches[i] && now.contains_key(prog.rules[i].head.rel.as_str()) { analyse(&prog.rules[i], base, now, &seeds[i]) } else { BTreeMap::new() })
            .collect();
        let mut next: BTreeSet<usize> = BTreeSet::new();
        for (&i, facts) in round.iter().zip(analysed) {
            let rule = &prog.rules[i];
            let Some(cols) = carried.get_mut(rule.head.rel.as_str()) else { continue };
            let mut grew = false;
            for (c, t) in rule.head.args.iter().enumerate() {
                if let Some(slot) = cols.get_mut(c) {
                    grew |= step(&facts, t, slot);
                }
            }
            if grew {
                changed.insert(rule.head.rel.clone());
                next.extend(readers.get(rule.head.rel.as_str()).into_iter().flatten().copied());
            }
        }
        round = next.into_iter().collect();
    }
}

/// A rule's variables that are scoped, and their grammars; `seed` holds what a specialized copy's head is.
/// Its variables are the rule's (and the seed's) own strings, borrowed: none is copied.
fn analyse<'r>(rule: &'r crate::code_rule_dsl::logic::front::Rule, base: &HashMap<String, Vec<usize>>, carried: &HashMap<Name, Vec<Fact>>, seed: &'r BTreeMap<String, Fact>) -> BTreeMap<&'r str, Fact> {
    let mut facts: BTreeMap<&'r str, Fact> = seed.iter().map(|(v, f)| (v.as_str(), f.clone())).collect();
    let var = |t: Option<&'r Term>| match t {
        Some(Term::Var(v)) => Some(v.as_str()),
        _ => None,
    };
    let positive = || rule.body.iter().filter_map(|l| match l {
        Literal::Atom(a) if !a.neg => Some(a),
        _ => None,
    });
    // Seeds: the guest relations' guest columns, with the grammar a constant names.
    for a in positive() {
        let Some(&(_, cols, grammar, tape)) = ENTRIES.iter().find(|(e, ..)| *e == a.rel) else { continue };
        let grammar = grammar.and_then(|g| match a.args.get(g) {
            Some(Term::Str(s)) => Some(s.to_string()),
            _ => None,
        });
        let fact = Fact { scoped: true, grammar, definite: true };
        for &c in cols.iter().chain(tape.iter()) {
            if let Some(v) = var(a.args.get(c)) {
                facts.entry(v).or_default().join(&fact);
            }
        }
    }
    // Derived relations' carried columns.
    for a in positive() {
        if let Some(cols) = carried.get(a.rel.as_str()) {
            for (c, f) in cols.iter().enumerate() {
                // A column rules fill from guests of several grammars (`""`: a rule per dialect of a view, decision
                // 0222) says nothing of this rule's: a guest relation here names it.
                if (f.scoped || f.grammar.is_some()) && let Some(v) = var(a.args.get(c)) {
                    let grammar = f.grammar.clone().filter(|g| !g.is_empty());
                    facts.entry(v).or_default().join(&Fact { definite: f.definite && f.scoped, grammar, ..f.clone() });
                }
            }
        }
    }
    // A tape's documents and a document's nodes are the tape's, and a base atom's locations are in one document.
    let mut groups: Vec<Vec<&'r str>> = Vec::new();
    const PAIR: &[usize] = &[0, 1];
    for a in positive() {
        let cols: &[usize] = match a.rel.as_str() {
            "tape_of" => PAIR,
            "doc_root" => PAIR,
            // A node's parent is in its document (`tape(n, par, size, depth)`, the climbs' walk).
            "tape" => PAIR,
            // A tree axis's node and its ancestor (`rr_has_<set>(n, a)`, `rr_inside_<set>(n, a)`): `has(r, c, "field")`
            // over a guest's root r makes c the guest's node.
            r if crate::code_rule_dsl::logic::front::is_axis(r) => PAIR,
            r => base.get(r).map_or(&[], Vec::as_slice),
        };
        let vars: Vec<&'r str> = cols.iter().filter_map(|&c| var(a.args.get(c))).collect();
        if vars.len() > 1 {
            groups.push(vars);
        }
    }
    for l in &rule.body {
        match l {
            Literal::Nearest { n, a, .. } => groups.push(vec![n.as_str(), a.as_str()]),
            // `t = rr_tape1`: a hoisted value is the variable it is named by.
            Literal::Cmp { l: Term::Var(a), op: "=", r: Term::Var(b) } => groups.push(vec![a.as_str(), b.as_str()]),
            _ => {}
        }
    }
    loop {
        let mut changed = false;
        for g in &groups {
            let mut all = Fact::default();
            for v in g {
                if let Some(f) = facts.get(v) {
                    all.join(f);
                }
            }
            if all == Fact::default() {
                continue;
            }
            for v in g {
                changed |= facts.entry(v).or_default().join(&all);
            }
        }
        if !changed {
            break;
        }
    }
    facts
}

/// A kind or field constant of `a` named in `label`'s grammar (`"sql:select"`), unless it names one already.
fn qualify(a: &mut Atom, label: &str) {
    let col = match a.rel.as_str() {
        "kind" => Some(1),
        "node" => Some(2),
        "field" => Some(1),
        _ => None,
    };
    if let Some(Term::Str(s)) = col.and_then(|c| a.args.make_mut().get_mut(c))
        && qualified(s).is_none()
    {
        *s = format!("{label}:{s}").into();
    }
}

/// `"sql:select"` as its grammar's label and the name, when the label is a registry language's.
pub fn qualified(s: &str) -> Option<(&str, &str)> {
    let (label, name) = s.split_once(':')?;
    (!label.is_empty() && !name.is_empty() && crate::code_rule_dsl::language::dialect_by_label(label).is_some()).then_some((label, name))
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A relation read with guest variables in two columns its rules bind apart is copied once, with both seeded:
    /// copying the copy lost the first seed, so its reader read it at that column again, and schema-architecture's
    /// `rr_agg36` nested `rr_in_annotation_1_rr_in_annotation_0_…` without end.
    #[test]
    fn a_copy_read_at_another_guest_column_is_one_copy_of_the_relation() {
        let mut p = crate::code_rule_dsl::logic::rr::parse(concat!(
            ".decl parsed(s: node, g: symbol, r: node)\n.decl e(a: node)\n.decl f(b: node)\n.input parsed, e, f\n",
            ".decl pair(a: node, b: node)\npair(a, b) :- e(a), f(b).\n",
            ".decl out(r: node)\n.output out\nout(r) :- parsed(_, \"sql\", r), parsed(_, \"sql\", s), pair(r, s).\n",
        ))
        .unwrap();
        hide(&mut p, 1 << 30);
        let copies: BTreeSet<&str> = p.rules.iter().map(|r| r.head.rel.as_str()).filter(|r| r.starts_with("rr_in_")).collect();
        assert!(copies.iter().all(|c| c.matches("rr_in_").count() == 1), "{copies:?}");
        assert!(copies.contains("rr_in_sql_0_sql_1_pair"), "{copies:?}");
        let out = p.rules.iter().find(|r| r.head.rel == "out").unwrap().to_string();
        assert!(out.contains("rr_in_sql_0_sql_1_pair("), "{out}");
    }

    /// O110: a column filled by host and guest nodes alike (`calls.rerail`'s `tv`) is not a guest's column. A rule reading
    /// a relation with a variable from it keeps reading the relation for the host's nodes, and a clone reads the
    /// guest's copy; before, the rule read only the copy, and `tbv` lost every host assignment.
    #[test]
    fn a_maybe_guest_variable_keeps_the_host_rows_and_clones_for_the_guest() {
        let mut p = crate::code_rule_dsl::logic::rr::parse(concat!(
            ".decl parsed(s: node, g: symbol, r: node)\n.decl e(a: node)\n.input parsed, e\n",
            ".decl val(n: node)\nval(n) :- e(n).\nval(r) :- parsed(_, \"sql\", r).\n",
            ".decl asg(n: node)\nasg(n) :- e(n).\n",
            ".decl only(n: node)\nonly(r) :- parsed(_, \"sql\", r).\n",
            ".decl out(n: node)\n.output out\nout(n) :- val(n), asg(n).\nout(n) :- only(n), asg(n).\n",
        ))
        .unwrap();
        hide(&mut p, 1 << 30);
        let outs: Vec<String> = p.rules.iter().filter(|r| r.head.rel == "out").map(|r| r.to_string()).collect();
        assert!(outs.iter().any(|r| r.contains("val(") && r.contains(" asg(")), "the host's rows stay: {outs:#?}");
        assert!(outs.iter().any(|r| r.contains("val(") && r.contains("rr_in_sql_0_asg(")), "a clone reads the guest's copy: {outs:#?}");
        // A column only guest nodes fill is the guest's: its reader is specialized, not split.
        assert!(outs.iter().any(|r| r.contains("only(") && r.contains("rr_in_sql_0_asg(")), "{outs:#?}");
        assert!(!outs.iter().any(|r| r.contains("only(") && r.contains(" asg(")), "{outs:#?}");
    }

    /// A copy whose head holds a constant at the seeded column (`unset` is -1) is never carried as scoped there, so
    /// every round found the same copy again and counted it a change: `ruff-perf401` did not end (decision 0084's
    /// dump found it). Reading the copy already is no change, and the rounds end.
    #[test]
    fn a_copy_with_a_constant_at_its_seeded_column_ends() {
        let mut p = crate::code_rule_dsl::logic::rr::parse(concat!(
            ".decl parsed(s: node, g: symbol, r: node)\n.input parsed\n",
            ".decl none(n: node)\nnone(-1) :- parsed(_, _, _).\n",
            ".decl out(r: node)\n.output out\nout(r) :- parsed(_, \"sql\", r), none(r).\n",
        ))
        .unwrap();
        let (done, wait) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            hide(&mut p, 1 << 30);
            let _ = done.send(p);
        });
        let p = wait.recv_timeout(std::time::Duration::from_secs(20)).expect("hide ends");
        let out = p.rules.iter().find(|r| r.head.rel == "out").unwrap().to_string();
        assert!(out.contains("rr_in_sql_0_none("), "{out}");
    }
    /// A copy for a guest in the run's own language (an annotation in a Python run, its guests shown) is its
    /// original: folded once the language is known, its reader reads the original and its clone repeats its rule.
    /// In another language's run, or with guests hidden, the copies stay.
    #[test]
    fn copies_for_a_guest_in_the_runs_own_language_are_folded_into_their_originals() {
        let text = |g: &str| {
            format!(concat!(
                ".decl parsed(s: node, g: symbol, r: node)\n.decl e(a: node)\n.input parsed, e\n",
                ".decl val(n: node)\nval(n) :- e(n).\nval(r) :- parsed(_, \"{g}\", r).\n",
                ".decl asg(n: node)\nasg(n) :- e(n), kind(n, \"identifier\").\n",
                ".decl only(n: node)\nonly(r) :- parsed(_, \"{g}\", r).\n",
                ".decl out(n: node)\n.output out\nout(n) :- val(n), asg(n).\nout(n) :- only(n), asg(n).\n",
            ), g = g)
        };
        let hidden = |grammar: &str, label: &str, bound: u32| {
            let mut p = crate::code_rule_dsl::logic::rr::parse(&text(grammar)).unwrap();
            hide(&mut p, bound);
            let made = p.decls.keys().filter(|r| r.starts_with("rr_in_")).count();
            let folded = fold_own(&mut p, label, bound);
            super::super::merge_and_drop_repeats(&mut p);
            (made, folded, p)
        };
        let shown = crate::code_rule_dsl::facts::vtab::syntax::ANNOTATION_KEYS;
        let (made, folded, p) = hidden("python", "python", shown);
        assert!(made > 0, "hiding made the guest's copies");
        assert_eq!(folded, made);
        assert!(p.rules.iter().all(|r| !r.to_string().contains("rr_in_") && !r.to_string().contains("python:")), "{:#?}", p.rules.iter().map(|r| r.to_string()).collect::<Vec<_>>());
        let outs: Vec<String> = p.rules.iter().filter(|r| r.head.rel == "out").map(|r| r.to_string()).collect();
        assert_eq!(outs.len(), 2, "each clone repeats its rule: {outs:#?}");
        assert_eq!(hidden("python", "rust", shown).1, 0, "another language's run keeps them");
        assert_eq!(hidden("python", "python", 1 << 30).1, 0, "guests hidden keep them");
        // An annotation is not shown to a rule that does not name it, so its copies fold into originals whose
        // bounds take annotations in; a rule no annotation reaches keeps the narrow bound.
        let (made, folded, p) = hidden("annotation", "python", shown);
        assert!(made > 0 && folded == made, "{made} made, {folded} folded");
        let wide = (i64::from(crate::code_rule_dsl::facts::vtab::syntax::HIDDEN_KEYS) << 32).to_string();
        let narrow = (i64::from(shown) << 32).to_string();
        let rules: Vec<String> = p.rules.iter().map(|r| r.to_string()).collect();
        assert!(rules.iter().any(|r| r.starts_with("asg(") && r.contains(&wide)), "{rules:#?}");
        assert!(rules.iter().any(|r| r.starts_with("out(") && r.contains(&narrow)) || rules.iter().filter(|r| r.starts_with("out(")).all(|r| !r.contains(&wide)), "{rules:#?}");
    }

    /// A tree axis read over a node that may be an annotation's is the clone's copy of the axis, over a copy of its kind
    /// set (`rr_inside_rr_in_annotation_<set>`). Folding the copied set folds the axis too: before, the set went and
    /// the axis stayed, naming no set, so the transducer made it with no rows, and the clone's `!inside(t, _, S)` held
    /// for every node, below S too (the Django rules' `within_stop` over a block found through `sym_of`).
    #[test]
    fn an_axis_over_a_folded_kind_set_reads_the_original_set() {
        let mut p = crate::code_rule_dsl::logic::rr::parse(concat!(
            ".decl parsed(s: node, g: symbol, r: node)\n.decl e(a: node)\n.input parsed, e\n",
            ".decl body(b: node)\nbody(b) :- e(b).\nbody(r) :- parsed(_, \"annotation\", r).\n",
            ".decl attempt(t: node)\nattempt(t) :- e(t).\n",
            ".decl out(t: node)\n.output out\n",
            "out(t) :- attempt(t), body(b), within(t, b), !inside(t, _, \"function_definition\").\n",
        ))
        .unwrap();
        let shown = crate::code_rule_dsl::facts::vtab::syntax::ANNOTATION_KEYS;
        hide(&mut p, shown);
        let axes = |p: &Program| -> Vec<String> {
            p.rules.iter().flat_map(|r| r.body.iter()).filter_map(Literal::atom).map(|a| a.rel.to_string()).filter(|r| crate::code_rule_dsl::logic::front::is_axis(r)).collect()
        };
        assert!(axes(&p).iter().any(|r| r.contains("rr_in_annotation_")), "the clone reads the annotation's axis: {:?}", axes(&p));
        assert!(fold_own(&mut p, "python", shown) > 0);
        super::super::merge_and_drop_repeats(&mut p);
        for rel in axes(&p).iter().chain(p.inputs.iter().filter(|r| crate::code_rule_dsl::logic::front::is_axis(r))) {
            let (set, _) = crate::code_rule_dsl::logic::front::axis(rel).unwrap();
            assert!(p.kindsets.contains_key(set), "{rel} names no kind set of the program: {:?}", p.kindsets);
        }
        let outs: Vec<String> = p.rules.iter().filter(|r| r.head.rel == "out").map(|r| r.to_string()).collect();
        assert!(!outs.is_empty() && outs.iter().all(|r| r.contains("!rr_inside_rr_kind_function_definition(") && !r.contains("rr_in_")), "{outs:#?}");
    }

    /// A native operator over an original folding widens reads it narrow when its relation reads narrow: its rows are
    /// made from the relation its fields name, not from its rule, so its fields name the narrow copy too.
    #[test]
    fn a_native_over_a_widened_original_reads_its_narrow_copy() {
        use crate::code_rule_dsl::logic::front::Native;
        let mut p = crate::code_rule_dsl::logic::rr::parse(concat!(
            ".decl parsed(s: node, g: symbol, r: node)\n.decl e(a: node)\n.input parsed, e\n",
            ".decl asg(n: node)\nasg(n) :- e(n), kind(n, \"identifier\").\n",
            ".decl only(n: node)\nonly(r) :- parsed(_, \"annotation\", r).\n",
            ".decl named(n: node)\nnamed(n) :- asg(n).\n",
            ".decl out(n: node)\n.output out\nout(n) :- only(n), asg(n).\nout(n) :- named(n).\n",
        ))
        .unwrap();
        let shown = crate::code_rule_dsl::facts::vtab::syntax::ANNOTATION_KEYS;
        hide(&mut p, shown);
        p.natives.insert("named".into(), Native::TextTest { input: "asg".into(), column: 0, op: "match".into(), patterns: Vec::new() });
        assert!(fold_own(&mut p, "python", shown) > 0);
        assert_eq!(p.natives["named"].reads(), ["rr_narrow_asg"]);
        let named: Vec<String> = p.rules.iter().filter(|r| r.head.rel == "named").map(|r| r.to_string()).collect();
        assert!(named.iter().all(|r| r.contains("rr_narrow_asg(")), "{named:#?}");
    }
}
