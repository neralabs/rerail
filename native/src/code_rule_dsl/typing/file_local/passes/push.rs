//! `PushLocalJoins` (0281's path, step 5), reading the split from the version (`FileLocal`). Variables are read by
//! name, so each component, interface and helper name (`fl<h>_<head>`, from its content: `helper_name`) is fixed by
//! the program; each round's program is a version of its own, split again to check the helpers stay file-local.

use super::super::{FileLocal, close_under, push_equal_columns, var_equalities_in};
use super::{BodyItem, Made, NOwned, Round, lit_vars, name, term_vars, write};
use crate::code_rule_dsl::logic::plan::fields::{Fields, FieldsData};
use crate::code_rule_dsl::logic::program_image::manager::{self, Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::{AtomRef, Builder, CmpOp, Lit as L, LitRef, RuleRef, TermRef, Version};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

/// Pushes the file-local part of each cross-file rule into the chunks: the rule's positive reads of base relations
/// and file-local relations, joined by their variables, with the comparisons over only their variables, become a
/// file-local relation of the variables the rest of the rule uses (`fl<h>_<head>`), and the rule reads that. So do
/// its negated reads of those relations whose variables the component binds.
///
/// Without this, a cross-file rule (one that negates or joins a cross-file relation) reads its base relations
/// whole: every chunk's `within`, `node`, `child` rows are kept for the cross part, memory that grows with the
/// corpus and with the rules run at once. With it, the chunk joins them and exports the few rows that pass.
///
/// A component that is one atom of only variables filters nothing and is left; so is one atom of a file-local
/// relation alone. A made relation the split does not find file-local is undone. `made`: the relations made.
///
/// A component that narrows no node, only lookups of a node's attributes (`docof(n, d), doc(d, p, _), line(n, l)`,
/// a finding's location) or relations of documents (`p9_path_module(d, m)`), is a row per node of the chunk: on 200
/// files, 611,865 rows for each such rule, every one held by the cross part. It is pushed only when that keeps the
/// cross part off the base relations (so the per-file memo works); when the cross part reads them anyway, it reads
/// these lookups there too. So is a component that enumerates subtrees (`descends`): `within(n, a)` with `n`
/// narrowed only by its attributes.
#[derive(Default)]
pub struct PushLocalJoins {
    pub made: Vec<String>,
    /// Whether the cross-file part reads a base relation after the first push: the program's mode (a part's, its
    /// share of the run's, decision 0345 stage 2).
    pub reads_base: bool,
}

const ATTRIBUTES: [&str; 8] = ["docof", "line", "end_line", "col", "visit_index", "node_order", "node", "doc"];

/// All of `a`'s terms are variables or wildcards, each variable once.
fn plain(v: &Version, rule: RuleRef<'_>, a: AtomRef<'_>) -> bool {
    let named: Vec<String> = a.args().filter_map(|t| t.var()).map(|s| name(v, rule, s)).collect();
    a.args().all(|t| matches!(t, TermRef::Var(_) | TermRef::Wild)) && named.iter().collect::<BTreeSet<_>>().len() == named.len()
}

fn has_var(v: &Version, rule: RuleRef<'_>, a: AtomRef<'_>, x: &str) -> bool {
    a.args().any(|t| matches!(t, TermRef::Var(s) if name(v, rule, s) == x))
}

/// Whether nothing in `atoms` narrows a node: each is a plain lookup of a node's attributes (no constant, no repeated
/// variable) or has no node column, and some node is read.
fn unrestricted(v: &Version, f: Fields<'_>, rule: RuleRef<'_>, atoms: &[AtomRef<'_>]) -> bool {
    let rel = |a: &AtomRef<'_>| v.rel_name(a.rel());
    atoms.iter().any(|a| matches!(rel(a), "docof" | "line" | "end_line" | "col" | "visit_index" | "node_order" | "node"))
        && atoms.iter().all(|a| {
            let attribute = ATTRIBUTES.contains(&rel(a));
            (attribute && plain(v, rule, *a)) || (!attribute && !f.columns(rel(a)).is_some_and(|cols| cols.iter().any(|(_, t)| matches!(*t, "node" | "loc" | "span"))))
        })
}

/// Whether a component enumerates subtrees: `within(n, a)` with `n` read by no other atom of it but a node's
/// attributes (`docof`, `visit_index`, `line`, …). Its rows are every node under each `a` it narrows, whatever reads
/// it: `c4_shadow`'s `binding(s, x, site, _), within(n, s), earlier(site, n)` made 922,044 rows on the Python pack's
/// fixtures for a cross-file reader that binds `n` from a few hundred asks (`c4_builtin_ask(n, x)`).
fn descends(v: &Version, rule: RuleRef<'_>, atoms: &[AtomRef<'_>]) -> bool {
    let attribute = |rel: &str| matches!(rel, "docof" | "line" | "end_line" | "col" | "visit_index" | "node_order" | "ast_depth");
    atoms.iter().enumerate().any(|(wi, w)| {
        let args: Vec<TermRef<'_>> = w.args().collect();
        match (v.rel_name(w.rel()), args.as_slice()) {
            ("within", [TermRef::Var(n), a]) if !matches!(a, TermRef::Var(m) if m == n) => {
                let n = name(v, rule, *n);
                atoms.iter().enumerate().filter(|(bi, b)| *bi != wi && has_var(v, rule, **b, &n)).all(|(_, b)| attribute(v.rel_name(b.rel())))
            }
            _ => false,
        }
    })
}

/// `push_with`'s `independent_node_groups`.
fn independent_node_groups(v: &Version, f: Fields<'_>, rule: RuleRef<'_>, atoms: &[AtomRef<'_>], equal: &[(String, String)]) -> usize {
    let documents: BTreeSet<String> = atoms
        .iter()
        .filter_map(|a| {
            let args: Vec<TermRef<'_>> = a.args().collect();
            match (v.rel_name(a.rel()), args.as_slice()) {
                ("docof", [_, TermRef::Var(d)]) => Some(name(v, rule, *d)),
                ("node" | "doc", [TermRef::Var(d), ..]) => Some(name(v, rule, *d)),
                _ => None,
            }
        })
        .collect();
    let mut groups: Vec<BTreeSet<String>> = Vec::new();
    for (ai, atom) in atoms.iter().enumerate() {
        let Some(columns) = f.columns(v.rel_name(atom.rel())) else { continue };
        let mut nodes: BTreeSet<String> = atom
            .args()
            .zip(&columns)
            .filter_map(|(term, (_, ty))| if matches!(*ty, "node" | "loc" | "span") && let TermRef::Var(s) = term { Some(name(v, rule, s)) } else { None })
            .collect();
        if nodes.is_empty() {
            let own: BTreeSet<String> = atom.args().filter_map(|t| t.var()).map(|s| name(v, rule, s)).filter(|x| !documents.contains(x)).collect();
            let met = atoms.iter().enumerate().any(|(bi, b)| bi != ai && b.args().any(|t| matches!(t, TermRef::Var(s) if own.contains(&name(v, rule, s)))));
            if own.len() < 2 || met {
                continue;
            }
            nodes = own;
        }
        close_under(&mut nodes, equal);
        let mut connected = nodes;
        let mut separate = Vec::new();
        for group in groups {
            if group.is_disjoint(&connected) {
                separate.push(group);
            } else {
                connected.extend(group);
            }
        }
        separate.push(connected);
        groups = separate;
    }
    groups.len()
}

/// A round of `push_with` that made helpers: its program, the helpers by rule, and the program's split.
struct Pushed<'v> {
    round: Round<'v>,
    made: Vec<String>,
    local: BTreeSet<String>,
    candidate: Arc<Version>,
}

/// A node's text or value read, `x = @rr_node_text(v)` (either side, `syntax::NODE_READS`): `(x, v)`.
fn node_read(v: &Version, rule: RuleRef<'_>, l: LitRef<'_>) -> Option<(String, String)> {
    let L::Cmp { l, op: CmpOp::Eq, r } = l.view() else { return None };
    let read = |x: TermRef<'_>, c: TermRef<'_>| match (x, c) {
        (TermRef::Var(x), TermRef::Call(f, mut args)) if crate::code_rule_dsl::facts::vtab::syntax::NODE_READS.contains(&v.text(f)) => {
            match (args.next(), args.next()) {
                (Some(TermRef::Var(n)), None) => Some((name(v, rule, x), name(v, rule, n))),
                _ => None,
            }
        }
        _ => None,
    };
    read(l, r).or_else(|| read(r, l))
}

/// A round of `PushLocalJoins`: the relations made, and the split of the program as it is left; `None` where it makes
/// nothing.
fn push_with<'v>(v: &'v Arc<Version>, f: Fields<'v>, local: &BTreeSet<String>, keep_unrestricted: bool) -> Result<Option<Pushed<'v>>, String> {
    let equal_columns = push_equal_columns();
    let rules: Vec<RuleRef<'v>> = v.rules().collect();
    let decl_names: HashSet<String> = f.decls().iter().map(|d| v.text(d.name()).to_owned()).collect();
    let base = |rel: &str| f.is_input(rel);
    let mut level: BTreeMap<usize, u8> = BTreeMap::new();
    loop {
        let mut used = std::collections::HashMap::new();
        let mut decls: Vec<(String, Vec<(String, String)>)> = Vec::new();
        let mut made: Vec<(usize, String)> = Vec::new();
        let mut program: Vec<Made<'v>> = Vec::with_capacity(rules.len());
        let mut helpers: Vec<Made<'v>> = Vec::new();
        for (i, &rule) in rules.iter().enumerate() {
            let at = level.get(&i).copied().unwrap_or(0);
            let keep = |program: &mut Vec<Made<'v>>| program.push(Made { src: rule, head: None, body: Vec::new(), kept: true });
            let head_rel = v.rel_name(rule.head().rel());
            if local.contains(head_rel) || at >= 2 {
                keep(&mut program);
                continue;
            }
            let body: Vec<LitRef<'v>> = rule.body().collect();
            let candidate = |l: LitRef<'_>| matches!(l.view(), L::Atom(a) if !a.neg() && !f.lattice(v.rel_name(a.rel())) && (base(v.rel_name(a.rel())) || local.contains(v.rel_name(a.rel()))));
            let idx: Vec<usize> = (0..body.len()).filter(|&j| candidate(body[j])).collect();
            let equal: Vec<(String, String)> = var_equalities_in(v, rule).into_iter().map(|(a, b)| (name(v, rule, a), name(v, rule, b))).collect();
            let mut comps: Vec<(BTreeSet<usize>, BTreeSet<String>)> = Vec::new();
            for &j in &idx {
                let mut vars = lit_vars(v, rule, body[j]);
                close_under(&mut vars, &equal);
                let (joined, rest): (Vec<_>, Vec<_>) = comps.into_iter().partition(|(_, x)| !x.is_disjoint(&vars));
                let mut all = (BTreeSet::from([j]), vars);
                for (l, x) in joined {
                    all.0.extend(l);
                    all.1.extend(x);
                }
                comps = rest;
                comps.push(all);
            }
            let mut taken: BTreeSet<usize> = BTreeSet::new();
            let mut replaced: Vec<(usize, BodyItem<'v>)> = Vec::new();
            for (mut lits, mut vars) in comps {
                let atoms: Vec<AtomRef<'v>> = lits.iter().filter_map(|&j| body[j].atom()).collect();
                let rel = |a: &AtomRef<'_>| v.rel_name(a.rel());
                // A node's text or value read of a node this component binds (`t = @rr_node_text(v)`, a long-text
                // twin's, decision 0109) goes with it: only the chunk's batch holds the node's text, so the read is
                // answered there and its value handed on. Such a component is pushed whatever else would leave it
                // (decision 0292 §3).
                let node_reads: Vec<String> = body.iter().filter_map(|l| node_read(v, rule, *l)).filter(|(_, n)| vars.contains(n)).map(|(x, _)| x).collect();
                vars.extend(node_reads.iter().cloned());
                let must = !node_reads.is_empty();
                if !must && !atoms.iter().any(|a| base(rel(a))) && atoms.len() < 2 {
                    continue;
                }
                if independent_node_groups(v, f, rule, &atoms, &equal) > 1 {
                    continue;
                }
                let attribute = |a: &AtomRef<'_>| ATTRIBUTES.contains(&rel(a)) && plain(v, rule, *a);
                let rest: Vec<&AtomRef<'v>> = atoms.iter().filter(|a| !attribute(a)).collect();
                if let [a] = rest[..]
                    && !must
                    && plain(v, rule, *a)
                {
                    let own: BTreeSet<String> = a.args().filter_map(|t| t.var()).map(|s| name(v, rule, s)).collect();
                    let of_atom = atoms.iter().filter(|b| attribute(b)).all(|b| {
                        let key = if rel(b) == "node" { b.args().nth(1) } else { b.args().next() };
                        matches!(key, Some(TermRef::Var(s)) if own.contains(&name(v, rule, s)))
                    });
                    if of_atom {
                        continue;
                    }
                }
                if let [a] = atoms[..]
                    && !must
                    && plain(v, rule, a)
                {
                    continue;
                }
                if !must && keep_unrestricted && (unrestricted(v, f, rule, &atoms) || descends(v, rule, &atoms)) {
                    continue;
                }
                for (j, l) in body.iter().enumerate() {
                    let with = match l.view() {
                        L::Cmp { .. } => true,
                        L::Atom(a) => a.neg() && !a.gated() && !f.lattice(rel(&a)) && (base(rel(&a)) || (at == 0 && local.contains(rel(&a)))),
                        _ => false,
                    };
                    let lv = lit_vars(v, rule, *l);
                    if with && !lv.is_empty() && lv.is_subset(&vars) {
                        lits.insert(j);
                    }
                }
                let mut outside: BTreeSet<String> = BTreeSet::new();
                rule.head().args().for_each(|t| outside.extend(term_vars(v, rule, t)));
                for (j, l) in body.iter().enumerate() {
                    if !lits.contains(&j) {
                        outside.extend(lit_vars(v, rule, *l));
                    }
                }
                let mut iface: Vec<String> = Vec::new();
                for &j in &lits {
                    for x in lit_vars(v, rule, body[j]) {
                        if outside.contains(&x) && !iface.contains(&x) {
                            iface.push(x);
                        }
                    }
                }
                if iface.is_empty() {
                    continue;
                }
                let joined_away = vars.iter().any(|x| !iface.contains(x) && atoms.iter().filter(|a| has_var(v, rule, **a, x)).count() >= 2);
                let filtered = body.iter().enumerate().any(|(j, l)| {
                    !lits.contains(&j)
                        && matches!(l.view(), L::Atom(a) if !a.neg() && !f.lattice(rel(&a)) && !candidate(*l)
                            && iface.iter().all(|x| has_var(v, rule, a, x)))
                });
                if !must && joined_away && filtered {
                    continue;
                }
                let typed: Option<Vec<(String, String)>> = iface
                    .iter()
                    .map(|x| {
                        let column_type = |var: &str| {
                            if node_reads.iter().any(|read| read == var) {
                                return Some("sym".to_owned());
                            }
                            atoms.iter().find_map(|a| {
                                let c = a.args().position(|t| matches!(t, TermRef::Var(s) if name(v, rule, s) == var))?;
                                f.column_type(rel(a), c).map(str::to_owned)
                            })
                        };
                        let ty = column_type(x).or_else(|| {
                            if !equal_columns {
                                return None;
                            }
                            let equal: Vec<_> = lits.iter().filter_map(|&j| match body[j].view() {
                                L::Cmp { l: TermRef::Var(a), op: CmpOp::Eq, r: TermRef::Var(b) } => {
                                    Some((name(v, rule, a), name(v, rule, b)))
                                }
                                _ => None,
                            }).collect();
                            let mut available = BTreeSet::from([x.clone()]);
                            close_under(&mut available, &equal);
                            available.iter().find_map(|var| column_type(var))
                        })?;
                        Some((x.clone(), ty))
                    })
                    .collect();
                let Some(typed) = typed else { continue };
                let content = format!("{head_rel}({}) :- {}", iface.join(", "), lits.iter().map(|&j| v.show_lit(rule, body[j])).collect::<Vec<_>>().join(", "));
                let helper = super::super::helper_name("fl", head_rel, &content, |n| decl_names.contains(n), &mut used);
                decls.retain(|(r, _)| *r != helper);
                decls.push((helper.clone(), typed));
                let args: Vec<NOwned<'v>> = iface.iter().map(|x| NOwned::V(x.clone())).collect();
                helpers.push(Made { src: rule, head: Some((helper.clone(), args.clone())), body: lits.iter().map(|&j| BodyItem::Orig(body[j])).collect(), kept: false });
                replaced.push((*lits.iter().next().expect("a literal"), BodyItem::New(helper.clone(), args)));
                taken.extend(lits);
                made.push((i, helper));
            }
            if taken.is_empty() {
                keep(&mut program);
                continue;
            }
            let mut out = Vec::new();
            for (j, l) in body.iter().enumerate() {
                if let Some((_, r)) = replaced.iter().find(|(at, _)| *at == j) {
                    out.push(r.clone());
                } else if !taken.contains(&j) {
                    out.push(BodyItem::Orig(*l));
                }
            }
            program.push(Made { src: rule, head: None, body: out, kept: false });
        }
        if made.is_empty() {
            return Ok(None);
        }
        program.extend(helpers);
        let round = Round { decls, undeclared: Vec::new(), rules: program, no_inline: Vec::new(), nullary: Vec::new() };
        let (candidate, split) = super::candidate(v, &round)?;
        let now = split.local.clone();
        let failed: Vec<usize> = made.iter().filter(|(_, n)| !now.contains(n)).map(|(i, _)| *i).collect();
        if failed.is_empty() {
            return Ok(Some(Pushed { round, made: made.into_iter().map(|(_, n)| n).collect(), local: now, candidate }));
        }
        for i in failed {
            *level.entry(i).or_default() += 1;
        }
    }
}

/// The rules outside the chunks (`local`) that read a base relation, each with the relation.
fn cross_reads_base(v: &Version, inputs: &BTreeSet<&str>, local: &BTreeSet<String>) -> Vec<(String, String)> {
    v.rules()
        .filter(|r| !local.contains(v.rel_name(r.head().rel())))
        .filter_map(|r| r.body().find_map(|l| l.atom().map(|a| v.rel_name(a.rel())).filter(|rel| inputs.contains(rel))).map(|rel| (v.show_rule(r), rel.to_owned())))
        .collect()
}

impl ProgramPass for PushLocalJoins {
    const NAME: &'static str = "push the file-local joins into the chunks";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let v: &Arc<Version> = cx.v();
        let d = cx.get::<FieldsData>()?;
        let f = Fields::over(v, &d);
        let split = cx.get::<FileLocal>()?;
        let inputs: BTreeSet<&str> = f.inputs().collect();
        let first = push_with(v, f, &split.local, false)?;
        let reads_base = match &first {
            Some(p) => cross_reads_base(&p.candidate, &inputs, &p.local),
            None => cross_reads_base(v, &inputs, &split.local),
        };
        if !reads_base.is_empty() {
            crate::code_rule_dsl::logic::profile::decision("file-local joins pushed into the chunks", || {
                let first: Vec<String> = reads_base.iter().take(4).map(|(rule, rel)| format!("{rule} reads {rel}")).collect();
                format!("the cross-file part reads base relations after the first push, {} rules ({}): a finding's location lookups are read there, not pushed", reads_base.len(), first.join("; "))
            });
        }
        let reads_base = !reads_base.is_empty();
        self.reads_base = reads_base;
        // A part pushes in the run's mode (decision 0345 stage 2): every part's reads together decide it.
        use crate::code_rule_dsl::logic::plan::fields::{PART_WHOLE, PUSH_BASE, PUSH_DECIDED};
        let mode = if f.qualified(PART_WHOLE, PUSH_DECIDED) { f.qualified(PART_WHOLE, PUSH_BASE) } else { reads_base };
        let pushed = if mode { push_with(v, f, &split.local, true)? } else { first };
        if let Some(p) = pushed {
            self.made = p.made;
            write(w.derive()?, v, &p.round)?;
        }
        Ok(())
    }
}
