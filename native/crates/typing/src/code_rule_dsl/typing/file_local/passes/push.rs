//! `PushLocalJoins` (0281's path, step 5), reading the split from the version (`FileLocal`). Variables are read by
//! name, so each component, interface and helper name (`fl<h>_<head>`, from its content: `helper_name`) is fixed by
//! the program; each round's program is a version of its own, split again to check the helpers stay file-local.

use rerail_blocks::vector::Vector;
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
    /// The rounds the last push took (`push_with`): a rule held back a level a round, at most two levels each.
    pub rounds: usize,
}

/// A node's attributes (and a document's, `doc`): a row a node, read by its key. `text(n, s)` is one: a component of
/// `text`, `docof` and a document's module (`rf_ref`'s) was a row for every node with text, 4.3 M on t50's product
/// run, made and handed on by every chunk for a cross-file rule that reads a few of them.
const ATTRIBUTES: [&str; 9] = ["docof", "line", "end_line", "col", "visit_index", "node_order", "node", "doc", "text"];

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

/// The variables of `atom` in its node columns (`node`, `loc`, `span`): what puts it in a node group.
fn node_vars(v: &Version, f: Fields<'_>, rule: RuleRef<'_>, atom: AtomRef<'_>) -> BTreeSet<String> {
    let Some(columns) = f.columns(v.rel_name(atom.rel())) else { return BTreeSet::new() };
    atom.args()
        .zip(&columns)
        .filter_map(|(term, (_, ty))| if matches!(*ty, "node" | "loc" | "span") && let TermRef::Var(s) = term { Some(name(v, rule, s)) } else { None })
        .collect()
}

/// `push_with`'s `independent_node_groups`, each group's variables.
fn node_groups(v: &Version, f: Fields<'_>, rule: RuleRef<'_>, atoms: &[AtomRef<'_>], equal: &[(String, String)]) -> Vec<BTreeSet<String>> {
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
        if f.columns(v.rel_name(atom.rel())).is_none() {
            continue;
        }
        let mut nodes = node_vars(v, f, rule, *atom);
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
    groups
}

/// `comp` (its literals, of `body`, and their variables), one component of each of its independent node groups
/// (`node_groups`): two files' nodes joined by a value (`text(a, t), text(b, t)`, a cross-file join by a string) are
/// each a file's, joined in the cross part by that value, their interface. An atom of node columns goes with its
/// group; one of none (a text test of `t`) with every group that binds all its variables, a filter of each; one that
/// joins two groups by values alone stays in the rule. A component of one group is itself.
fn by_node_group(v: &Version, f: Fields<'_>, rule: RuleRef<'_>, body: &[LitRef<'_>], comp: (BTreeSet<usize>, BTreeSet<String>), equal: &[(String, String)]) -> Vec<(BTreeSet<usize>, BTreeSet<String>)> {
    let atoms: Vec<(usize, AtomRef<'_>)> = comp.0.iter().filter_map(|&j| body[j].atom().map(|a| (j, a))).collect();
    let groups = node_groups(v, f, rule, &atoms.iter().map(|(_, a)| *a).collect::<Vec<_>>(), equal);
    if groups.len() < 2 {
        return vec![comp];
    }
    // A part's variables are its literals' own, not closed under the rule's equalities: an equality across two
    // groups (`dr = ds`, two digests' test) is no part's, and stays in the rule to join them.
    let vars_of = |j: usize| lit_vars(v, rule, body[j]);
    let mut parts: Vec<(BTreeSet<usize>, BTreeSet<String>)> = groups.into_iter().map(|g| (BTreeSet::new(), g)).collect();
    let mut rest = Vec::new();
    for &(j, a) in &atoms {
        let mut nodes = node_vars(v, f, rule, a);
        close_under(&mut nodes, equal);
        match parts.iter_mut().find(|(_, g)| !nodes.is_empty() && !g.is_disjoint(&nodes)) {
            Some(part) => {
                part.0.insert(j);
                part.1.extend(vars_of(j));
            }
            None => rest.push(j),
        }
    }
    for j in rest {
        let x = vars_of(j);
        for part in parts.iter_mut().filter(|(_, vars)| x.is_subset(vars)) {
            part.0.insert(j);
        }
    }
    // A group that narrows no node (a node's attributes alone, `docof(a, d), visit_index(a, ai)`) or enumerates
    // subtrees would be a row of each: it stays in the rule, as the component did whole.
    parts.retain(|(lits, _)| {
        let atoms: Vec<AtomRef<'_>> = lits.iter().filter_map(|&j| body[j].atom()).collect();
        !atoms.is_empty() && !unrestricted(v, f, rule, &atoms) && !descends(v, rule, &atoms)
    });
    parts
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

/// The helpers a round made, by the rules that read them (`push_with`). A helper of the same content is made once and
/// read by every rule whose component it is, so a helper the split does not take holds back all of its readers in one
/// round. Held against its first reader alone, each other reader made it again the next round and failed again: a
/// round per reader, each splitting the whole program (on main's Python pack the two pushes never ended; 3 rounds each
/// now).
#[derive(Default)]
pub struct Readers {
    reads: Vec<(usize, String)>,
    seen: HashSet<(usize, String)>,
}

impl Readers {
    /// Rule `rule` reads `helper`.
    pub fn read(&mut self, rule: usize, helper: String) {
        if self.seen.insert((rule, helper.clone())) {
            self.reads.push((rule, helper));
        }
    }
    pub fn is_empty(&self) -> bool {
        self.reads.is_empty()
    }
    /// Each (rule, helper) read, in the order read.
    pub fn reads(&self) -> &[(usize, String)] {
        &self.reads
    }
    /// The rules that read a helper `taken` lacks: each once, however many of its helpers failed (a level a round).
    pub fn refused(&self, taken: &BTreeSet<String>) -> BTreeSet<usize> {
        self.reads.iter().filter(|(_, n)| !taken.contains(n)).map(|(i, _)| *i).collect()
    }
    /// Each helper once, in the order made.
    pub fn names(self) -> Vec<String> {
        let mut seen: HashSet<&str> = HashSet::with_capacity(self.reads.len());
        self.reads.iter().filter(|(_, n)| seen.insert(n.as_str())).map(|(_, n)| n.clone()).collect()
    }
}

/// The file-local relations a helper cannot read and stay file-local (`placeless`): those with no location column
/// (`FileLocal::locations`), not constants or filters, which the split lets any rule read. Each file's rows of one are
/// part of a relation of every file's, so a rule that reads one joins no document by it and is cross-file ("reads r
/// with no location"): a join pushed with it is refused every round, and its reader left reading the base relations.
fn placeless(split: &FileLocal) -> BTreeSet<String> {
    let located = |rel: &String| split.locations.get(rel).is_some_and(|c| !c.is_empty());
    split.local.iter().filter(|rel| !located(rel) && !split.constants.contains(*rel) && !split.filters.contains(*rel)).cloned().collect()
}

/// A round of `PushLocalJoins`: the relations made, and the split of the program as it is left; `None` where it makes
/// nothing. `split`'s placeless relations (`placeless`) are the reader's, as a cross-file relation is.
/// `rounds`: the rounds it took, each a split of the whole program.
fn push_with<'v>(v: &'v Arc<Version>, f: Fields<'v>, split: &FileLocal, keep_unrestricted: bool, rounds: &mut usize) -> Result<Option<Pushed<'v>>, String> {
    let local = &split.local;
    let rules: Vec<RuleRef<'v>> = v.rules().collect();
    let heads: HashSet<&str> = rules.iter().map(|r| v.rel_name(r.head().rel())).collect();
    let rules_of = |rel: &str| heads.contains(rel);
    let placeless = placeless(split);
    // A relation of stated facts alone (`effect_kind("call")`, no rule of it) is every chunk's too, as a constant of
    // the split is: a component takes it, so its variable stays inside rather than every row's value being handed on
    // to the cross part.
    let stated = |rel: &str| f.facts(rel).is_some_and(|x| !x.about()) && !rules_of(rel);
    let joinable = |rel: &str| (local.contains(rel) && !placeless.contains(rel)) || split.constants.contains(rel) || stated(rel);
    let equal_columns = push_equal_columns();
    let decl_names: HashSet<String> = f.decls().iter().map(|d| v.text(d.name()).to_owned()).collect();
    let base = |rel: &str| f.is_input(rel);
    let mut level: BTreeMap<usize, u8> = BTreeMap::new();
    *rounds = 0;
    loop {
        // A round: every rule pushed at its level, the program split again, and the rules whose push the split did not
        // take held back a level. Its rules held back name it, so a profile counts the rounds.
        let _round = crate::code_rule_dsl::logic::profile::span(|| format!("a round of the push ({} rules held back)", level.len()));
        *rounds += 1;
        let mut used = std::collections::HashMap::new();
        let mut by_content: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut decls: Vec<(String, Vec<(String, String)>)> = Vec::new();
        let mut made = Readers::default();
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
            let candidate = |l: LitRef<'_>| matches!(l.view(), L::Atom(a) if !a.neg() && !f.lattice(v.rel_name(a.rel())) && (base(v.rel_name(a.rel())) || joinable(v.rel_name(a.rel()))));
            let idx: Vector<usize> = (0..body.len()).filter(|&j| candidate(body[j])).collect();
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
            let comps: Vec<(BTreeSet<usize>, BTreeSet<String>)> = comps.into_iter().flat_map(|c| by_node_group(v, f, rule, &body, c, &equal)).collect();
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
                if node_groups(v, f, rule, &atoms, &equal).len() > 1 {
                    continue;
                }
                // The comparisons and negations the component takes with it (those over its variables alone): they
                // filter its rows in the chunk.
                let goes_with = |l: LitRef<'_>| match l.view() {
                    L::Cmp { .. } => true,
                    L::Atom(a) => a.neg() && !a.gated() && !f.lattice(rel(&a)) && (base(rel(&a)) || (at == 0 && joinable(rel(&a)))),
                    _ => false,
                };
                let filtered = body.iter().enumerate().any(|(j, l)| {
                    let lv = lit_vars(v, rule, *l);
                    !lits.contains(&j) && goes_with(*l) && !lv.is_empty() && lv.is_subset(&vars)
                });
                let attribute = |a: &AtomRef<'_>| ATTRIBUTES.contains(&rel(a)) && plain(v, rule, *a);
                let rest: Vec<&AtomRef<'v>> = atoms.iter().filter(|a| !attribute(a)).collect();
                // One plain atom and lookups of its nodes' attributes (`child(p, f, _), text(p, s)`) is a row a node the
                // cross part reads as well where it is: left, unless a test of its variables goes with it
                // (`!bound(p, s)`), which the chunk answers and the cross part would otherwise read whole.
                if let [a] = rest[..]
                    && !must
                    && !filtered
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
                    let lv = lit_vars(v, rule, *l);
                    if goes_with(*l) && !lv.is_empty() && lv.is_subset(&vars) {
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
                // A bound (`$x < k`) is the reader's where its variable is handed on anyway: two rules that differ only
                // in their bounds (a narrow copy, its original and its candidates, decision 0575) then push one join.
                // A bound of a variable nothing else outside reads stays in the join: handing the variable on for it
                // alone made a row of every value (`p297_effect`'s `$n < …`: every node under each `e`, 7 rows a node
                // of the 300-file bench).
                // The literals in one order whatever the rule's (by their text), and the interface by first sight in
                // it: two rules that push the same join with their literals written in another order (a recheck and
                // its original, decision 0575) name one helper.
                let mut in_order: Vec<usize> = lits.iter().copied().collect();
                in_order.sort_by_cached_key(|&j| v.show_lit(rule, body[j]));
                let mut iface: Vec<String> = Vec::new();
                for &j in &in_order {
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
                // A join on a variable it projects away whose interface a cross-file read binds whole stays in the cross
                // part, which starts from the filter (ed8dd88c04: F823's pairs, 15.4 M rows for one file). An interface
                // of one variable is no product: at most a row a value. Where the join narrows its nodes (a kind of
                // `effect_kind`'s), the chunk makes it once and the memo keeps it; one that narrows none
                // (`docof(n, d), lead(d)`) is a row of every node, and stays the filter's.
                if !must && joined_away && filtered && (iface.len() > 1 || unrestricted(v, f, rule, &atoms)) {
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
                // A test of the interface's values alone is the reader's: an order test (a bound, `t < k`), or a
                // negation of a derived relation (the cross part holds no tree, so not of a base one). The join's rows
                // are the same without it, so two rules that differ only in such tests (a narrow copy and its
                // original, decision 0165) push one join, made once.
                let in_iface: BTreeSet<String> = iface.iter().cloned().collect();
                let reader_test = |l: LitRef<'_>| {
                    let test = match l.view() {
                        L::Cmp { op: CmpOp::Lt | CmpOp::Le | CmpOp::Gt | CmpOp::Ge, l: a, r: b } => [a, b].iter().all(|t| matches!(t, TermRef::Var(_) | TermRef::Int(_))),
                        L::Atom(a) => a.neg() && !a.gated() && !base(v.rel_name(a.rel())) && a.args().all(|t| matches!(t, TermRef::Var(_) | TermRef::Int(_) | TermRef::Str(_))),
                        _ => false,
                    };
                    test && lit_vars(v, rule, l).is_subset(&in_iface)
                };
                lits.retain(|&j| !reader_test(body[j]));
                // Named by the join alone, not the rule's head: every rule that pushes the same join reads one helper.
                let mut shown: Vec<String> = lits.iter().map(|&j| v.show_lit(rule, body[j])).collect();
                shown.sort();
                let content = format!("({}) :- {}", iface.join(", "), shown.join(", "));
                let helper = match by_content.get(&content) {
                    Some(h) => h.clone(),
                    None => {
                        let h = super::super::helper_name("fl", head_rel, &content, |n| decl_names.contains(n), &mut used);
                        by_content.insert(content.clone(), h.clone());
                        h
                    }
                };
                let args: Vec<NOwned<'v>> = iface.iter().map(|x| NOwned::V(x.clone())).collect();
                // A helper of the same content (two files' parts of one join, `by_node_group`; a narrow copy's and its
                // original's join, decision 0575) is the one made: read again, not made again.
                if !helpers.iter().any(|h| h.head.as_ref().is_some_and(|(name, _)| *name == helper)) {
                    decls.retain(|(r, _)| *r != helper);
                    decls.push((helper.clone(), typed));
                    helpers.push(Made { src: rule, head: Some((helper.clone(), args.clone())), body: lits.iter().map(|&j| BodyItem::Orig(body[j])).collect(), kept: false });
                }
                // Where the helper's read goes: its first literal no other helper's read took (a filter copied into
                // several components is the first literal of more than one).
                let at = lits.iter().copied().find(|j| !replaced.iter().any(|(at, _)| at == j)).expect("a literal no other helper's read took");
                replaced.push((at, BodyItem::New(helper.clone(), args)));
                taken.extend(lits);
                // Read by this rule, as by every other rule it was made for (`Readers`).
                made.read(i, helper);
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
        let round = Round { decls, undeclared: Vec::new(), rules: program, no_inline: Vec::new(), strategy: Vec::new(), nullary: Vec::new() };
        let split_span = crate::code_rule_dsl::logic::profile::span(|| "the round's program, split again".into());
        let (candidate, split) = super::candidate(v, &round)?;
        drop(split_span);
        let now = split.local.clone();
        // Each rule once, however many of its helpers failed: a level a round.
        let failed = made.refused(&now);
        if failed.is_empty() {
            return Ok(Some(Pushed { round, made: made.names(), local: now, candidate }));
        }
        crate::code_rule_dsl::logic::profile::decision("a pushed join the split did not take: its rules held back a level", || {
            let gone: Vec<String> = made.reads().iter().filter(|(_, n)| !now.contains(n)).take(3).map(|(i, n)| format!("{n} of {}", v.show_rule(rules[*i]))).collect();
            // The split's cause for the first helper it did not take.
            let why = made.reads().iter().find(|(_, n)| !now.contains(n)).and_then(|(_, n)| split.causes.get(n)).map_or("", String::as_str);
            format!("{} rules ({}): {}; the first's cause: {why}", failed.len(), level.len(), gone.join("; "))
        });
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
        let first = push_with(v, f, &split, false, &mut self.rounds)?;
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
        let pushed = if mode { push_with(v, f, &split, true, &mut self.rounds)? } else { first };
        if let Some(p) = pushed {
            self.made = p.made;
            write(w.derive()?, v, &p.round)?;
        }
        Ok(())
    }
}
