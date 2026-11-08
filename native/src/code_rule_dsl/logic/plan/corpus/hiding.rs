//! Which documents a rule sees (decision 0058 §1): a base atom ranges over the policy's shown documents unless the
//! rule reached it from a guest.
//!
//! A run's documents are, by key (`syntax::GUEST_KEYS`, `syntax::HIDDEN_KEYS`):
//! - files, `1..2^30`;
//! - guests in the run's language (a Python string annotation in a Python run), `2^30..2^30 + 2^29`: shown, unless
//!   the program says `.guests off`;
//! - every other document (a guest in another grammar, a value `@parse` or `@splice` made), up to `2^31`: never
//!   shown.
//!
//! A node's id is its document's key `<< 32` and its index, so "shown" is one bound on a location value.
//!
//! A variable is *scoped* when the rule reached it from a guest relation (`guest`, `parsed`, `doc_root`, `doc_role`,
//! `tape_of`, `host_node`): it is a location in a guest column of one, or it shares a base atom's location columns
//! with a scoped variable (they are in one document), or it is read from a derived relation's column that some rule
//! fills from a scoped variable. Every other variable in a positive base atom's location column gets the bound
//! (`n < bound << 32`, `d < bound`). The bound is a comparison on a value the join already has: no row is copied,
//! and a run that makes no guests is not changed at all.
//!
//! A scoped variable also knows its grammar when the guest relation names it (`parsed(s, "sql", r)`, `guest(g, _,
//! _, "sql")`, a tape from `@parse("sql", …)`). A kind or field constant on such a node is resolved in that grammar
//! (`"sql:select"`, `plan::lower::Constants`), so a Python policy can name SQL kinds inside its SQL guests.

use crate::code_rule_dsl::logic::front::{Atom, Literal, Program, Term};
use std::collections::{BTreeMap, BTreeSet};

/// The relations a rule enters a guest through, and their columns that are the guest's: (relation, guest location
/// columns, grammar column, tape column).
const ENTRIES: [(&str, &[usize], Option<usize>, Option<usize>); 9] = [
    ("guest", &[0], Some(3), None),
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
    use crate::code_rule_dsl::facts::vtab::syntax::{GUEST_KEYS, GUEST_POLICY, HIDDEN_KEYS};
    let off = prog.facts.get(GUEST_POLICY).into_iter().flatten().any(|row| {
        matches!(row.as_slice(), [crate::code_rule_dsl::logic::front::Value::Str(k), crate::code_rule_dsl::logic::front::Value::Str(v)] if k == "library" && v == "off")
    });
    if off { GUEST_KEYS } else { HIDDEN_KEYS }
}

/// What the analysis knows of a variable: scoped, and its grammar (`Some("")` when two disagree).
#[derive(Clone, Default, PartialEq)]
struct Fact {
    scoped: bool,
    grammar: Option<String>,
}

impl Fact {
    fn join(&mut self, other: &Fact) -> bool {
        let before = self.clone();
        self.scoped |= other.scoped;
        self.grammar = match (self.grammar.take(), &other.grammar) {
            (None, g) => g.clone(),
            (g, None) => g,
            (Some(a), Some(b)) if a == *b => Some(a),
            _ => Some(String::new()),
        };
        *self != before
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
    let base_locations: BTreeMap<String, Vec<usize>> = base.iter().map(|r| (r.clone(), locations(r))).collect();
    // Each rule's seeds: a specialized copy's head column, scoped in its grammar.
    let mut seeds: Vec<BTreeMap<String, Fact>> = vec![BTreeMap::new(); prog.rules.len()];
    // A derived relation read with a scoped variable of a known grammar is specialized to it (decision 0058 §2, O98):
    // `rr_in_<grammar>_<column>_<relation>`, its rules copied with that head column seeded, so a node set's kinds are
    // the guest's grammar's and its guest rows are not bounded away. A kind set (`.kindset`) read by `nearest` over
    // such a node is copied as `rr_in_<grammar>_<set>`, with its kinds named in that grammar. To a fixpoint: a copy reads relations too.
    let mut made: BTreeSet<String> = BTreeSet::new();
    // Each copy's relation and seeded columns, with their grammars.
    let mut specialized: BTreeMap<String, (String, BTreeMap<usize, String>)> = BTreeMap::new();
    let mut carried;
    loop {
        carried = carry(prog, &base_locations, &seeds, &view);
        let snapshot = prog.rules.clone();
        let mut copies: Vec<(crate::code_rule_dsl::logic::front::Rule, BTreeMap<String, Fact>)> = Vec::new();
        let mut decls: Vec<(String, Vec<(String, String)>)> = Vec::new();
        let mut kindsets: Vec<(String, Vec<String>)> = Vec::new();
        let mut changed = false;
        for (i, rule) in prog.rules.iter_mut().enumerate() {
            if view(&rule.head.rel) {
                continue;
            }
            let facts = analyse(rule, &base_locations, &carried, &seeds[i]);
            let grammar_of = |v: &str| facts.get(v).filter(|f| f.scoped).and_then(|f| f.grammar.clone()).filter(|g| !g.is_empty());
            for l in &mut rule.body {
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
                        *kindset = name;
                        changed = true;
                    }
                    continue;
                }
                let a = match l {
                    Literal::Atom(a) | Literal::Agg { atom: a, .. } => a,
                    _ => continue,
                };
                let Some(cols) = carried.get(&a.rel) else { continue };
                let spec = a.args.iter().enumerate().find_map(|(c, t)| match t {
                    Term::Var(v) if !cols.get(c).is_some_and(|f| f.scoped) => grammar_of(v).map(|g| (c, g)),
                    _ => None,
                });
                let Some((c, g)) = spec else { continue };
                // A copy read with another column scoped is a copy of the same relation with both seeded, not a copy
                // of the copy: one per relation and set of seeded columns, so the copies end.
                let (orig, mut cols) = specialized.get(&a.rel).cloned().unwrap_or_else(|| (a.rel.clone(), BTreeMap::new()));
                cols.insert(c, g);
                let name = format!("rr_in_{}{orig}", cols.iter().map(|(c, g)| format!("{}_{c}_", g.replace(|ch: char| !ch.is_ascii_alphanumeric(), "_"))).collect::<String>());
                if made.insert(name.clone()) {
                    for r in snapshot.iter().filter(|r| r.head.rel == orig) {
                        let mut copy = r.clone();
                        copy.head.rel = name.clone();
                        let mut seed = BTreeMap::new();
                        for (c, g) in &cols {
                            if let Some(Term::Var(v)) = copy.head.args.get(*c) {
                                seed.insert(v.clone(), Fact { scoped: true, grammar: Some(g.clone()) });
                            }
                        }
                        copies.push((copy, seed));
                    }
                    if let Some(d) = prog.decls.get(&orig) {
                        decls.push((name.clone(), d.clone()));
                    }
                    specialized.insert(name.clone(), (orig, cols));
                }
                a.rel = name;
                changed = true;
            }
        }
        for (rule, seed) in copies {
            prog.rules.push(rule);
            seeds.push(seed);
        }
        prog.decls.extend(decls);
        prog.kindsets.extend(kindsets);
        if !changed {
            break;
        }
    }
    let bound_of = |doc_column: bool| if doc_column { i64::from(bound) } else { i64::from(bound) << 32 };
    for (rule, seed) in prog.rules.iter_mut().zip(&seeds).filter(|(r, _)| !VIEWS.contains(&r.head.rel.as_str())) {
        let facts = analyse(rule, &base_locations, &carried, seed);
        // Wildcards in an unscoped base atom's location columns become variables, so they can be bounded.
        let mut fresh = 0;
        let mut bounded: BTreeMap<String, bool> = BTreeMap::new();
        for l in &mut rule.body {
            let Literal::Atom(a) = l else { continue };
            if a.neg {
                continue;
            }
            let Some(cols) = base_locations.get(&a.rel) else { continue };
            let doc_col = crate::code_rule_dsl::typing::file_local::document_column(&a.rel);
            let atom_scoped = cols.iter().any(|&c| matches!(a.args.get(c), Some(Term::Var(v)) if facts.get(v).is_some_and(|f| f.scoped)));
            if atom_scoped {
                let grammar = cols.iter().find_map(|&c| match a.args.get(c) {
                    Some(Term::Var(v)) => facts.get(v).and_then(|f| f.grammar.clone()).filter(|g| !g.is_empty()),
                    _ => None,
                });
                if let Some(g) = grammar {
                    qualify(a, dialect_label(&g));
                }
                continue;
            }
            for &c in cols {
                match a.args.get_mut(c) {
                    Some(Term::Var(v)) => {
                        bounded.entry(v.clone()).or_insert(Some(c) == doc_col);
                    }
                    Some(t @ Term::Wild) => {
                        let v = format!("rr_shown{fresh}");
                        fresh += 1;
                        *t = Term::Var(v.clone());
                        bounded.insert(v, Some(c) == doc_col);
                    }
                    _ => {}
                }
            }
        }
        for (v, doc_column) in bounded {
            rule.body.push(Literal::Cmp { l: Term::Var(v), op: "<", r: Term::Int(bound_of(doc_column)) });
        }
    }
    // An aggregate's atom is its own scope: one over a base atom reads it through a bounded helper,
    // `rr_shown_agg<k>(vars) :- atom, bounds`.
    let mut helpers: Vec<(Atom, Vec<(String, String)>, Vec<Literal>)> = Vec::new();
    for rule in prog.rules.iter_mut().filter(|r| !VIEWS.contains(&r.head.rel.as_str())) {
        for l in &mut rule.body {
            let Literal::Agg { atom, .. } = l else { continue };
            let Some(cols) = base_locations.get(&atom.rel) else { continue };
            if cols.is_empty() {
                continue;
            }
            let decl = prog.decls.get(&atom.rel).cloned().unwrap_or_default();
            let doc_col = crate::code_rule_dsl::typing::file_local::document_column(&atom.rel);
            // Every wildcard becomes a column of the helper: the aggregate counts the atom's rows, not their
            // projection.
            let mut inner = atom.clone();
            for (c, t) in inner.args.iter_mut().enumerate() {
                if matches!(t, Term::Wild) {
                    *t = Term::Var(format!("rr_shown_agg_c{c}"));
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
                    vars.push((v.clone(), decl.get(c).map_or_else(|| "int".to_owned(), |(_, ty)| ty.clone())));
                }
            }
            let name = format!("rr_shown_agg{}", helpers.len());
            let head = Atom { rel: name, args: vars.iter().map(|(v, _)| Term::Var(v.clone())).collect(), neg: false, gated: false };
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
        prog.decls.insert(head.rel.clone(), vars);
        prog.rules.push(crate::code_rule_dsl::logic::front::Rule { head, body });
    }
}

/// What each derived relation's columns carry, to a fixpoint over the rules.
fn carry(prog: &Program, base: &BTreeMap<String, Vec<usize>>, seeds: &[BTreeMap<String, Fact>], view: &dyn Fn(&str) -> bool) -> BTreeMap<String, Vec<Fact>> {
    let mut carried: BTreeMap<String, Vec<Fact>> =
        prog.rules.iter().map(|r| r.head.rel.clone()).filter(|r| !view(r)).map(|r| { let n = prog.decls.get(&r).map_or(0, Vec::len); (r, vec![Fact::default(); n]) }).collect();
    loop {
        let mut changed = false;
        for (rule, seed) in prog.rules.iter().zip(seeds) {
            let facts = analyse(rule, base, &carried, seed);
            let Some(cols) = carried.get_mut(&rule.head.rel) else { continue };
            for (c, t) in rule.head.args.iter().enumerate() {
                if let (Term::Var(v), Some(slot)) = (t, cols.get_mut(c))
                    && let Some(f) = facts.get(v)
                {
                    changed |= slot.join(f);
                }
            }
        }
        if !changed {
            return carried;
        }
    }
}

/// A rule's variables that are scoped, and their grammars; `seed` holds what a specialized copy's head is.
fn analyse(rule: &crate::code_rule_dsl::logic::front::Rule, base: &BTreeMap<String, Vec<usize>>, carried: &BTreeMap<String, Vec<Fact>>, seed: &BTreeMap<String, Fact>) -> BTreeMap<String, Fact> {
    let mut facts: BTreeMap<String, Fact> = seed.clone();
    let var = |t: Option<&Term>| match t {
        Some(Term::Var(v)) => Some(v.clone()),
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
            Some(Term::Str(s)) => Some(s.clone()),
            _ => None,
        });
        let fact = Fact { scoped: true, grammar };
        for &c in cols.iter().chain(tape.iter()) {
            if let Some(v) = var(a.args.get(c)) {
                facts.entry(v).or_default().join(&fact);
            }
        }
    }
    // Derived relations' carried columns.
    for a in positive() {
        if let Some(cols) = carried.get(&a.rel) {
            for (c, f) in cols.iter().enumerate() {
                if (f.scoped || f.grammar.is_some()) && let Some(v) = var(a.args.get(c)) {
                    facts.entry(v).or_default().join(f);
                }
            }
        }
    }
    // A tape's documents and a document's nodes are the tape's, and a base atom's locations are in one document.
    let mut groups: Vec<Vec<String>> = Vec::new();
    for a in positive() {
        let cols: Vec<usize> = match a.rel.as_str() {
            "tape_of" => vec![0, 1],
            "doc_root" => vec![0, 1],
            // A node's parent is in its document (`tape(n, par, size, depth)`, the climbs' walk).
            "tape" => vec![0, 1],
            r => base.get(r).cloned().unwrap_or_default(),
        };
        let vars: Vec<String> = cols.iter().filter_map(|&c| var(a.args.get(c))).collect();
        if vars.len() > 1 {
            groups.push(vars);
        }
    }
    for l in &rule.body {
        match l {
            Literal::Nearest { n, a, .. } => groups.push(vec![n.clone(), a.clone()]),
            // `t = rr_tape1`: a hoisted value is the variable it is named by.
            Literal::Cmp { l: Term::Var(a), op: "=", r: Term::Var(b) } => groups.push(vec![a.clone(), b.clone()]),
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
                changed |= facts.entry(v.clone()).or_default().join(&all);
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
    if let Some(Term::Str(s)) = col.and_then(|c| a.args.get_mut(c))
        && qualified(s).is_none()
    {
        *s = format!("{label}:{s}");
    }
}

/// `"sql:select"` as its grammar's label and the name, when the label is a registry language's.
pub(in crate::code_rule_dsl) fn qualified(s: &str) -> Option<(&str, &str)> {
    let (label, name) = s.split_once(':')?;
    (!label.is_empty() && !name.is_empty() && crate::code_rule_dsl::language::dialect_by_label(label).is_some()).then_some((label, name))
}

#[cfg(test)]
mod tests {
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
}
