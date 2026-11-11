//! File-local strata in the transducer (03 §13; 05 Phase 10 row 1, "split file-local strata"; list step
//! 29), the part list step 22 needs: `nearest` (O12, option a, decided 2026-11-03).
//!
//! `nearest($n, $a, set)` holds when `$a` is `$n`'s nearest strict ancestor whose kind is in the set. Tree
//! structure is not a relation (§12), so the strata do not climb the tree: the relation is made per chunk,
//! file-locally, and the strata read its rows as an input (`nearest_<set>(n, a)`,
//! `facts::vtab::syntax::with_nearest`).
//!
//! It is a pure function of the tree, solved per (set, node) cell (decision 0038). On the device by pointer
//! jumping in one dispatch (`transducer::packed::metal_nearest`, decision 0040): each cell's target starts at
//! its parent and takes its target's current state until final, one command buffer and one wait a chunk. On the CPU by one pass in
//! preorder, which is the reference. It was a flat-lattice system on the device work queue (list step 17), at
//! about 2.5 us a cell visit, one tree level at a time; `system` and `solve_cpu` remain for a piece whose
//! parents do not precede their children, and for comparison (`nearest_costs_by_solver`).

use crate::content_store::{StoreError, StoreResult};

pub(crate) use crate::code_rule_dsl::facts::nearest::{Binding, NONE, Trees};
/// The flat lattice's absent and top values (`logic::native::BOT`, `TOP`).
const BOT: i64 = i64::MIN;
const TOP: i64 = 1 << 62;



/// The flat-lattice system: seeds, and copies `(cell, source)`.
struct System {
    seeds: Vec<i64>,
    copies: Vec<(u32, u32)>,
}

fn system(trees: &Trees<'_>, sets: &[Vec<i64>]) -> StoreResult<System> {
    let n = trees.parent.len();
    let cells = n.checked_mul(sets.len()).filter(|&c| c < u32::MAX as usize).ok_or_else(|| StoreError::Invalid(format!("nearest: {n} nodes in {} sets", sets.len())))?;
    let mut seeds = vec![BOT; cells];
    let mut copies = Vec::new();
    for (k, set) in sets.iter().enumerate() {
        let base = k * n;
        for (node, &parent) in trees.parent.iter().enumerate() {
            if parent == NONE {
                continue;
            }
            if set.contains(&trees.kind[parent as usize]) {
                seeds[base + node] = i64::from(parent);
            } else {
                copies.push(((base + node) as u32, (base + parent as usize) as u32));
            }
        }
    }
    Ok(System { seeds, copies })
}

/// The system's least fixpoint on the CPU: a cell's value is its seed joined with its sources', evaluated
/// again whenever a source rises.
fn solve_cpu(system: &System) -> Vec<i64> {
    let n = system.seeds.len();
    // Sources and dependents as CSR (a cell has one source here; the solver does not rely on it).
    let csr = |edges: &mut Vec<(u32, u32)>| -> (Vec<u32>, Vec<u32>) {
        edges.sort_unstable();
        let mut start = vec![0_u32; n + 1];
        for &(a, _) in edges.iter() {
            start[a as usize + 1] += 1;
        }
        for at in 0..n {
            start[at + 1] += start[at];
        }
        (start, edges.iter().map(|e| e.1).collect())
    };
    let (source_start, sources) = csr(&mut system.copies.clone());
    let (dependent_start, dependents) = csr(&mut system.copies.iter().map(|&(cell, source)| (source, cell)).collect());
    let join = |a: i64, b: i64| if a == BOT { b } else if b == BOT || a == b { a } else { TOP };
    let mut values = system.seeds.clone();
    let mut work: Vec<u32> = (0..n as u32).filter(|&c| values[c as usize] != BOT).collect();
    while let Some(cell) = work.pop() {
        for &dependent in &dependents[dependent_start[cell as usize] as usize..dependent_start[cell as usize + 1] as usize] {
            let d = dependent as usize;
            let value = sources[source_start[d] as usize..source_start[d + 1] as usize].iter().fold(system.seeds[d], |v, &s| join(v, values[s as usize]));
            if value != values[d] {
                values[d] = value;
                work.push(dependent);
            }
        }
    }
    values
}

/// Cells of one system at most: a chunk's (list step 17's queue is sized per launch). Larger batches are split at
/// their documents' roots, a system and a launch each.
const CELLS: usize = 1 << 18;

/// Per set, per node of `trees`: its nearest strict ancestor whose kind is in the set, or `NONE`.
pub(crate) fn nearest(trees: &Trees<'_>, sets: &[Vec<i64>], binding: Binding) -> StoreResult<Vec<Vec<u32>>> {
    let n = trees.parent.len();
    if n == 0 || sets.is_empty() {
        return Ok(vec![Vec::new(); sets.len()]);
    }
    let mut out = vec![Vec::with_capacity(n); sets.len()];
    let per = (CELLS / sets.len()).max(1);
    let mut start = 0;
    while start < n {
        // The next piece ends at a root (a document's first node) once it holds `per` nodes; a tree is never split.
        let mut end = (start + per).min(n);
        while end < n && trees.parent[end] != NONE {
            end += 1;
        }
        let parent: Vec<u32> = trees.parent[start..end].iter().map(|&p| if p == NONE { NONE } else { p - start as u32 }).collect();
        let piece = Trees { parent: &parent, kind: &trees.kind[start..end] };
        for (set, found) in out.iter_mut().zip(solve(&piece, sets, binding)?) {
            set.extend(found.into_iter().map(|a| if a == NONE { NONE } else { a + start as u32 }));
        }
        start = end;
    }
    Ok(out)
}

/// `nearest` of one piece (decision 0038). On the device: pointer jumping, a cell per (set, node) whose target
/// starts at the parent and jumps to its target's target until final (`metal_nearest`). On the CPU: one pass in
/// preorder, where a node reads its parent's answer once; the worklist over the lattice system where a parent does
/// not precede its children.
fn solve(trees: &Trees<'_>, sets: &[Vec<i64>], binding: Binding) -> StoreResult<Vec<Vec<u32>>> {
    use crate::code_rule_dsl::logic::profile;
    let n = trees.parent.len();
    let member = |k: usize, node: u32| sets[k].contains(&trees.kind[node as usize]);
    crate::code_rule_dsl::prof::count("nearest.cells", n * sets.len());
    if let Some(dev) = crate::code_rule_dsl::logic::device::solver().filter(|_| binding == Binding::Metal) {
        let _span = profile::span(|| "nearest on Metal (pointer jumping)".into());
        profile::decision("nearest ancestors", || format!("solved per piece of at most {CELLS} cells on Metal by pointer jumping, one dispatch and one wait"));
        let mut target = Vec::with_capacity(n * sets.len());
        let mut done = Vec::with_capacity(n * sets.len());
        for k in 0..sets.len() {
            for &p in trees.parent {
                target.push(p);
                done.push(u8::from(p == NONE || member(k, p)));
            }
        }
        use crate::code_rule_dsl::logic::device::{Refused, nearest};
        // `RERAIL_NEAREST_BUFFERS=fresh`: buffers made per piece, to measure the pre-pinned blocks against.
        let how = if std::env::var("RERAIL_NEAREST_BUFFERS").is_ok_and(|v| v == "fresh") { nearest::Buffers::Fresh } else { nearest::Buffers::Blocks };
        match dev.nearest(n, &target, &done, how) {
            Ok(solved) => {
                let kernel = "nearest ancestors (pointer jumping)";
                profile::device(kernel, solved.launches as u64, solved.gpu, solved.wall, target.len() as u64);
                profile::device_phases(kernel, solved.phases, solved.made, solved.reused);
                crate::code_rule_dsl::prof::count("nearest.device_steps", solved.rounds as usize);
                return Ok(solved.targets.chunks(n.max(1)).map(<[u32]>::to_vec).collect());
            }
            Err(Refused::NoDevice) => {
                profile::decision("fallback", || "nearest ancestors: the Metal binding found no device and solved on the CPU".into());
            }
            Err(Refused::Failed(error)) => return Err(StoreError::Invalid(format!("nearest on the device: {error}"))),
        }
    }
    let _ = binding;
    let _span = profile::span(|| "nearest on the CPU".into());
    profile::work(profile::Unit::Cell, (trees.parent.len() * sets.len()) as u64);
    if trees.parent.iter().enumerate().all(|(v, &p)| p == NONE || (p as usize) < v) {
        profile::decision("nearest ancestors", || format!("solved per piece of at most {CELLS} cells on the CPU, one pass in preorder"));
        return Ok((0..sets.len())
            .map(|k| {
                let mut found = vec![NONE; n];
                for v in 0..n {
                    let p = trees.parent[v];
                    if p != NONE {
                        found[v] = if member(k, p) { p } else { found[p as usize] };
                    }
                }
                found
            })
            .collect());
    }
    profile::decision("nearest ancestors", || "a piece whose parents do not precede their children: the CPU worklist".into());
    let values = solve_cpu(&system(trees, sets)?);
    values
        .chunks(n)
        .map(|set| {
            set.iter()
                .map(|&v| match v {
                    BOT => Ok(NONE),
                    TOP => Err(StoreError::Invalid("nearest: a node with two nearest ancestors".to_owned())),
                    v => Ok(v as u32),
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests;

/// The relations a program derives within one document (list step 29's split, R0): a relation is file-local when
/// each of its rules joins only one document's rows. A rule is when every relation it reads (positive, negated or
/// aggregated) is a base relation, a constant one, or file-local, each with a location (a node, or a document) of the
/// rule's one component: locations are joined within an atom of a base or file-local relation (`node(d, n, …)`,
/// `child(p, c, _)`, `nearest_<set>(n, a)`), and nothing else joins them. A symbol shared by two atoms (`text($a,
/// $x), text($b, $x)`) does not: those may be two files. The relations are the greatest set closed under this, so a
/// recursion within a document is file-local. Evaluated per chunk of documents, a file-local relation's rows are the
/// union of the chunks'.
pub(crate) fn split(prog: &crate::code_rule_dsl::logic::front::Program) -> std::collections::BTreeSet<String> {
    split_located(prog).0
}

/// The inputs whose rows are the run's options (`facts::repository::options`): the same beside every chunk and the
/// cross-file part, and named by the corpus memo's key (`corpus::memo::with_options`), so constants to the split.
pub(crate) fn option_input(rel: &str) -> bool {
    matches!(rel, "option" | "option_item")
}

/// An input with one value for the whole run, as a fact has: the run's options (`option_input`), or an earlier
/// phase's export (`corpus::phases`), bound once its rows are known. A rule reading one joins no document by it.
pub(crate) fn run_constant(prog: &crate::code_rule_dsl::logic::front::Program, rel: &str) -> bool {
    option_input(rel) || (crate::code_rule_dsl::logic::plan::corpus::phases::is_phased(rel) && prog.inputs.iter().any(|i| i == rel))
}

/// Whether no rule of `prog` reads a document: every relation a rule reads is derived, a fact, or an input with one
/// value for the run (`run_constant`), and no rule matches an AST pattern or asks for the nearest of a node. Such a
/// program reads an earlier phase's exports and the run's options at most, and reads at least one of them.
pub(crate) fn reads_no_document(prog: &crate::code_rule_dsl::logic::front::Program) -> bool {
    use crate::code_rule_dsl::logic::front::Literal;
    let derived: std::collections::BTreeSet<&str> = prog.rules.iter().map(|r| r.head.rel.as_str()).collect();
    let mut phased = false;
    let none = prog.natives.is_empty()
        && prog.rules.iter().flat_map(|r| &r.body).all(|l| match l {
            Literal::Atom(a) | Literal::Agg { atom: a, .. } => {
                let constant = run_constant(prog, &a.rel);
                phased |= constant && !option_input(&a.rel);
                derived.contains(a.rel.as_str()) || constant || (prog.facts.contains_key(&a.rel) && !prog.inputs.contains(&a.rel))
            }
            Literal::Cmp { .. } => true,
            _ => false,
        });
    none && phased
}

/// Derived relations made from the program's constants alone (`la_side_queue(x) :- la_set("…", x)`): every rule
/// reads only facts, the run's options (`option_input`), other such relations and comparisons. They are constants to
/// the split, like facts: a rule reading one joins no document by it, and both parts derive them.
pub(crate) fn constant_relations(prog: &crate::code_rule_dsl::logic::front::Program) -> std::collections::BTreeSet<String> {
    use crate::code_rule_dsl::logic::front::Literal;
    let derived: std::collections::BTreeSet<&str> = prog.rules.iter().map(|r| r.head.rel.as_str()).collect();
    let mut constant: std::collections::BTreeSet<String> =
        derived.iter().filter(|r| !prog.natives.contains_key(**r) && !prog.lattice.contains_key(**r) && !prog.inputs.iter().any(|i| i == *r)).map(|r| (*r).to_owned()).collect();
    // A greatest fixpoint: a relation stays while every rule of it reads only constants. Each relation is checked
    // again only when one it reads leaves (a pack's program has tens of thousands of rules).
    let index = RuleIndex::of(prog);
    let mut queue: std::collections::VecDeque<&str> = derived.iter().copied().filter(|r| constant.contains(*r)).collect();
    let mut queued: std::collections::BTreeSet<&str> = queue.iter().copied().collect();
    while let Some(rel) = queue.pop_front() {
        queued.remove(rel);
        if !constant.contains(rel) {
            continue;
        }
        let still = index.rules_of(rel).all(|r| {
            r.body.iter().all(|l| match l {
                Literal::Atom(a) | Literal::Agg { atom: a, .. } => {
                    constant.contains(&a.rel) || run_constant(prog, &a.rel) || (!derived.contains(a.rel.as_str()) && prog.facts.contains_key(&a.rel) && !prog.inputs.contains(&a.rel))
                }
                Literal::Cmp { .. } => true,
                _ => false,
            })
        });
        if !still {
            constant.remove(rel);
            for reader in index.heads_reading(rel) {
                if constant.contains(reader) && queued.insert(reader) {
                    queue.push_back(reader);
                }
            }
        }
    }
    constant
}

/// A program's rules by head, and by the relations their bodies read: what a fixpoint over relations re-checks when
/// one changes, instead of every rule each round.
pub(crate) struct RuleIndex<'p> {
    rules: &'p [crate::code_rule_dsl::logic::front::Rule],
    by_head: std::collections::HashMap<&'p str, Vec<usize>>,
    by_read: std::collections::HashMap<&'p str, Vec<usize>>,
}

impl<'p> RuleIndex<'p> {
    pub(crate) fn of(prog: &'p crate::code_rule_dsl::logic::front::Program) -> Self {
        use crate::code_rule_dsl::logic::front::Literal;
        let mut by_head: std::collections::HashMap<&str, Vec<usize>> = Default::default();
        let mut by_read: std::collections::HashMap<&str, Vec<usize>> = Default::default();
        for (i, r) in prog.rules.iter().enumerate() {
            by_head.entry(r.head.rel.as_str()).or_default().push(i);
            for l in &r.body {
                if let Literal::Atom(a) | Literal::Agg { atom: a, .. } = l {
                    let readers = by_read.entry(a.rel.as_str()).or_default();
                    if readers.last() != Some(&i) {
                        readers.push(i);
                    }
                }
            }
        }
        RuleIndex { rules: &prog.rules, by_head, by_read }
    }

    /// `rel`'s rules, in program order.
    pub(crate) fn rules_of(&self, rel: &str) -> impl Iterator<Item = &'p crate::code_rule_dsl::logic::front::Rule> + '_ {
        self.by_head.get(rel).into_iter().flatten().map(|&i| &self.rules[i])
    }

    /// The indices of the rules whose bodies read `rel`, in program order.
    pub(crate) fn reading(&self, rel: &str) -> &[usize] {
        self.by_read.get(rel).map_or(&[], Vec::as_slice)
    }

    /// The heads of the rules whose bodies read `rel`.
    pub(crate) fn heads_reading(&self, rel: &str) -> impl Iterator<Item = &'p str> + '_ {
        self.reading(rel).iter().map(|&i| self.rules[i].head.rel.as_str())
    }
}

/// The rule's `a = b` between two variables, and its `d = @doc(n)` (decision 0091's scope guards): d is n's
/// document, so the two place one document, as a document column and a node of one atom do.
fn var_equalities(rule: &crate::code_rule_dsl::logic::front::Rule) -> Vec<(String, String)> {
    use crate::code_rule_dsl::logic::front::{Literal, Term};
    rule.body
        .iter()
        .filter_map(|l| match l {
            Literal::Cmp { l: Term::Var(a), op: "=", r: Term::Var(b) } => Some((a.clone(), b.clone())),
            Literal::Cmp { l: Term::Var(a), op: "=", r: Term::Call(f, xs) } if f == "doc" && let [Term::Var(b)] = xs.as_slice() => Some((a.clone(), b.clone())),
            // `@doc(a) = @doc(b)`: a and b are of one document (a rule written for one file's nodes).
            Literal::Cmp { l: Term::Call(f, xs), op: "=", r: Term::Call(g, ys) }
                if f == "doc" && g == "doc" && let ([Term::Var(a)], [Term::Var(b)]) = (xs.as_slice(), ys.as_slice()) =>
            {
                Some((a.clone(), b.clone()))
            }
            _ => None,
        })
        .collect()
}

/// Adds to `vars` every variable equal to one in it, to a fixpoint.
fn close_under(vars: &mut std::collections::BTreeSet<String>, equal: &[(String, String)]) {
    let mut changed = true;
    while changed {
        changed = false;
        for (a, b) in equal {
            if vars.contains(a) != vars.contains(b) {
                vars.insert(a.clone());
                vars.insert(b.clone());
                changed = true;
            }
        }
    }
}

pub(crate) use crate::code_rule_dsl::logic::front::document_column;

/// Base relations whose rows a chunk makes for itself and that name no document (decision 0058 §4).
const CHUNK_VALUES: [&str; 3] = ["parse_of", "unparse_of", "splice_of"];

use std::collections::BTreeMap;

/// The value tapes of a rule (decision 0058 §4, O96) and where they stand. A value is made in the chunk of the
/// document whose row gave its text, and is that document's: `parse_of(x, g, t)`'s t stands with the one atom that
/// binds x (two atoms binding x join two documents on a string, which is no one document's), and `splice_of(_, n,
/// _, t3)`'s t3 with node n.
struct ValueTapes {
    /// Each value tape (its class's first variable) and the body index of the atom that gave its text.
    by_binder: BTreeMap<String, usize>,
    /// Each spliced tape's node and tape.
    splices: Vec<(String, String)>,
    /// Each variable's class under the rule's `a = b`.
    class: BTreeMap<String, String>,
}

impl ValueTapes {
    /// `v`'s value tape, when it is one.
    fn canonical(&self, v: &str) -> Option<String> {
        let c = self.class.get(v).cloned().unwrap_or_else(|| v.to_owned());
        (self.by_binder.contains_key(&c) || self.splices.iter().any(|(_, t)| *t == c)).then_some(c)
    }
}

fn value_tapes(rule: &crate::code_rule_dsl::logic::front::Rule) -> ValueTapes {
    use crate::code_rule_dsl::logic::front::{Literal, Term};
    // `t = rr_tape1`: one class, named by its least variable.
    let mut class: BTreeMap<String, String> = BTreeMap::new();
    let find = |class: &BTreeMap<String, String>, v: &str| {
        let mut v = v.to_owned();
        while let Some(p) = class.get(&v).filter(|p| **p != v) {
            v = p.clone();
        }
        v
    };
    for l in &rule.body {
        if let Literal::Cmp { l: Term::Var(a), op: "=", r: Term::Var(b) } = l {
            let (a, b) = (find(&class, a), find(&class, b));
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            class.insert(hi, lo.clone());
            class.entry(lo.clone()).or_insert(lo);
        }
    }
    let vars: Vec<String> = class.keys().cloned().collect();
    let class: BTreeMap<String, String> = vars.into_iter().map(|v| (v.clone(), find(&class, &v))).collect();
    let canon = |v: &str| class.get(v).cloned().unwrap_or_else(|| v.to_owned());
    let mut out = ValueTapes { by_binder: BTreeMap::new(), splices: Vec::new(), class: class.clone() };
    for l in &rule.body {
        let Literal::Atom(a) = l else { continue };
        if a.neg {
            continue;
        }
        match (a.rel.as_str(), a.args.as_slice()) {
            ("parse_of", [Term::Var(x), _, Term::Var(t)]) => {
                let x = canon(x);
                let binders: Vec<usize> = rule
                    .body
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| matches!(l, Literal::Atom(b) if !b.neg && !CHUNK_VALUES.contains(&b.rel.as_str()) && b.rel != "tape_of" && b.args.iter().any(|t| matches!(t, Term::Var(v) if canon(v) == x))))
                    .map(|(i, _)| i)
                    .collect();
                if let [b] = binders[..] {
                    out.by_binder.insert(canon(t), b);
                }
            }
            ("splice_of", [_, Term::Var(n), _, Term::Var(t3)]) => out.splices.push((n.clone(), canon(t3))),
            _ => {}
        }
    }
    out
}

/// [`split`], with each derived relation's location columns: those every rule fills from a location (a node, or a
/// document of `node`, `doc`, …), and the same-document columns of each file-local or base relation: its location
/// columns, whose values in one row are all of one document. A row's document is read off any of them (decision 0052).
pub(crate) fn split_located(prog: &crate::code_rule_dsl::logic::front::Program) -> (std::collections::BTreeSet<String>, std::collections::BTreeMap<String, Vec<usize>>, std::collections::BTreeMap<String, Vec<usize>>) {
    let (local, locations, same, _) = split_explained(prog);
    (local, locations, same)
}

/// Why each derived relation is cross-file, by the first rule the split found not to join one document: a root
/// cause ("joins 2 documents: {n} {m}", "reads field with no location", "made natively") or "reads cross-file r".
/// Read through the second kind to the first to see what makes a check run over the whole corpus.
pub(crate) fn cross_causes(prog: &crate::code_rule_dsl::logic::front::Program) -> std::collections::BTreeMap<String, String> {
    split_explained(prog).3
}

#[allow(clippy::type_complexity)]
fn split_explained(prog: &crate::code_rule_dsl::logic::front::Program) -> (std::collections::BTreeSet<String>, std::collections::BTreeMap<String, Vec<usize>>, std::collections::BTreeMap<String, Vec<usize>>, std::collections::BTreeMap<String, String>) {
    use crate::code_rule_dsl::logic::front::{Literal, Term};
    use std::collections::{BTreeMap, BTreeSet};
    let derived: BTreeSet<&str> = prog.rules.iter().map(|r| r.head.rel.as_str()).collect();
    // A base relation's location columns: node columns, the document column (`document_column`: `node`, `doc`, `pkg`,
    // `syntax_error`, `cpython_syntax_error`, the source text's, the path's, the guests') and `docof(n, d)`'s
    // document. The reference graph's (`reference_edge`, `reference_open`) have none: an edge's ends are in any
    // files, and the graph is made of every file's.
    let base_locations = |rel: &str| -> Vec<usize> {
        if matches!(rel, "reference_edge" | "reference_open") {
            return Vec::new();
        }
        let mut columns: Vec<usize> = prog.decls.get(rel).map_or_else(Vec::new, |cols| cols.iter().enumerate().filter(|(_, (_, t))| t == "node").map(|(i, _)| i).collect());
        if let Some(column) = document_column(rel) {
            columns.push(column);
        }
        if rel == "docof" && !columns.contains(&1) {
            columns.push(1);
        }
        if rel == "docof" {
            columns.push(1);
        }
        columns
    };
    // A derived relation's location columns: those every rule fills from a location.
    let mut locations: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let location_vars = |rule: &crate::code_rule_dsl::logic::front::Rule, locations: &BTreeMap<String, Vec<usize>>| -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        // `v = v0` (a disjunction's branch, as lowering leaves it): v is a location where v0 is.
        let equal = var_equalities(rule);
        for literal in &rule.body {
            if let Some(a) = literal.atom().filter(|a| !a.neg) {
                let columns = if derived.contains(a.rel.as_str()) { locations.get(&a.rel).cloned().unwrap_or_default() } else { base_locations(&a.rel) };
                for c in columns {
                    if let Some(Term::Var(v)) = a.args.get(c) {
                        out.insert(v.clone());
                    }
                }
            }
        }
        close_under(&mut out, &equal);
        out
    };
    // Location columns to a fixpoint (a column is one if every rule's head fills it from a location variable).
    for rel in &derived {
        let width = prog.decls.get(*rel).map_or(0, Vec::len);
        locations.insert((*rel).to_owned(), (0..width).collect());
    }
    // A greatest fixpoint (columns only leave), so any order reaches the same one: a relation is checked again only
    // when the locations of one its rules read changed (a pack's program has tens of thousands of rules).
    let index = RuleIndex::of(prog);
    let mut queue: std::collections::VecDeque<&str> = derived.iter().copied().collect();
    let mut queued: BTreeSet<&str> = derived.clone();
    while let Some(rel) = queue.pop_front() {
        queued.remove(rel);
        let rules: Vec<&crate::code_rule_dsl::logic::front::Rule> = index.rules_of(rel).collect();
        let vars: Vec<BTreeSet<String>> = rules.iter().map(|r| location_vars(r, &locations)).collect();
        let kept: Vec<usize> = locations[rel]
            .iter()
            .copied()
            .filter(|&c| rules.iter().zip(&vars).all(|(r, vs)| matches!(r.head.args.get(c), Some(Term::Var(v)) if vs.contains(v))))
            .collect();
        if kept != locations[rel] {
            locations.insert(rel.to_owned(), kept);
            for reader in index.heads_reading(rel) {
                if queued.insert(reader) {
                    queue.push_back(reader);
                }
            }
        }
    }
    // String filters (decision 0037): a text test's relation (`strtest_<k>`), and a derived relation whose every
    // rule reads only text relations and such filters, each head string from a text column. A chunk's rows hold
    // every string of the chunk that passes, so a rule that joins one on a string its chunk gave stays file-local.
    let text_columns = |rel: &str| crate::code_rule_dsl::logic::strings::TEXT_COLUMNS.iter().find(|(r, _)| *r == rel).map(|(_, c)| *c);
    let test_relation = |rel: &str| rel.starts_with(crate::code_rule_dsl::logic::strings::RELATION);
    // To a fixpoint: a filter may read other filters.
    let mut filters: BTreeSet<String> = derived.iter().map(|r| (*r).to_owned()).collect();
    // A greatest fixpoint: a relation is checked again only when one its rules read leaves.
    let mut queue: std::collections::VecDeque<&str> = derived.iter().copied().collect();
    let mut queued: BTreeSet<&str> = derived.clone();
    while let Some(rel) = queue.pop_front() {
        queued.remove(rel);
        if !filters.contains(rel) {
            continue;
        }
        let still = {
            {
                index.rules_of(rel).all(|r| {
                    let mut strings = BTreeSet::new();
                    let reads_only_text = r.body.iter().all(|l| match l {
                        Literal::Atom(a) if test_relation(&a.rel) || filters.contains(&a.rel) => true,
                        Literal::Atom(a) if !a.neg && let Some(cols) = text_columns(&a.rel) => {
                            strings.extend(cols.iter().filter_map(|&c| if let Some(Term::Var(v)) = a.args.get(c) { Some(v.clone()) } else { None }));
                            true
                        }
                        Literal::Cmp { .. } => true,
                        _ => false,
                    });
                    reads_only_text && r.head.args.iter().all(|t| matches!(t, Term::Var(v) if strings.contains(v)))
                })
            }
        };
        if !still {
            filters.remove(rel);
            for reader in index.heads_reading(rel) {
                if filters.contains(reader) && queued.insert(reader) {
                    queue.push_back(reader);
                }
            }
        }
    }
    // Python fact operators preserve each demanded node's document. Their proof rules must still
    // pass the ordinary dependency/location checks below. Other native operators remain cross-file.
    let constants = constant_relations(prog);
    let cross_native = |r: &str| prog.natives.get(r).is_some_and(|n| match n {
        crate::code_rule_dsl::logic::front::Native::PythonCandidates { targets, .. } => !constants.contains(targets) && !(prog.facts.contains_key(targets) && !prog.inputs.contains(targets)),
        crate::code_rule_dsl::logic::front::Native::Python { targets, .. } => targets.as_ref().is_some_and(|t|
            !constants.contains(t) && !(prog.facts.contains_key(t) && !prog.inputs.contains(t))),
        _ => true,
    });
    let mut local: BTreeSet<String> = derived.iter().filter(|r| !cross_native(r)).map(|r| (*r).to_owned()).collect();
    let mut causes: BTreeMap<String, String> = derived.iter().filter(|r| cross_native(r)).map(|r| ((*r).to_owned(), "made natively".to_owned())).collect();
    let mut dirty: Option<BTreeSet<usize>> = None;
    loop {
        let rule_local = |rule: &crate::code_rule_dsl::logic::front::Rule| -> Result<(), String> {
            // Each read relation's location variables; the rule's locations must form one component.
            let mut groups: Vec<BTreeSet<String>> = Vec::new();
            let placed = value_tapes(rule);
            let tape = |v: &str| format!("rr_value_tape:{v}");
            for (n, t) in &placed.splices {
                groups.push(BTreeSet::from([n.clone(), tape(t)]));
            }
            for (index, literal) in rule.body.iter().enumerate() {
                let atom = match literal {
                    Literal::Atom(a) | Literal::Agg { atom: a, .. } => a,
                    Literal::Ast { .. } => return Err("an AST pattern".to_owned()),
                    Literal::Nearest { n, a, .. } => {
                        groups.push(BTreeSet::from([n.clone(), a.clone()]));
                        continue;
                    }
                    Literal::Cmp { .. } => continue,
                };
                let constant = (!derived.contains(atom.rel.as_str()) && prog.facts.contains_key(&atom.rel) && !prog.inputs.contains(&atom.rel)) || constants.contains(&atom.rel) || run_constant(prog, &atom.rel);
                // A tape operation's rows (decision 0058 §4) are made in the chunk that asked: they place nothing.
                let chunk_value = !derived.contains(atom.rel.as_str()) && CHUNK_VALUES.contains(&atom.rel.as_str());
                if constant || chunk_value || test_relation(&atom.rel) || filters.contains(&atom.rel) || filters.contains(&rule.head.rel) {
                    continue;
                }
                if derived.contains(atom.rel.as_str()) && !local.contains(&atom.rel) {
                    return Err(format!("reads cross-file {}", atom.rel));
                }
                let columns = if derived.contains(atom.rel.as_str()) { locations[&atom.rel].clone() } else { base_locations(&atom.rel) };
                let mut vars: BTreeSet<String> = columns.iter().filter_map(|&c| if let Some(Term::Var(v)) = atom.args.get(c) { Some(v.clone()) } else { None }).collect();
                // A value's text binder whose location is a wildcard (`q(_, x)`, as simplification leaves an
                // existence check) still reads one document's row: the value's tape stands there.
                let binder = matches!(literal, Literal::Atom(_)) && placed.by_binder.values().any(|b| *b == index);
                if vars.is_empty() && binder && !columns.is_empty() {
                    vars.insert(format!("rr_value_binder:{index}"));
                }
                if !vars.is_empty() {
                    // A value's tape stands with the atom that gave its text, and `tape_of(d, t)` puts d there too.
                    vars.extend(placed.by_binder.iter().filter(|(_, b)| **b == index).map(|(t, _)| tape(t)));
                    if atom.rel == "tape_of"
                        && let Some(Term::Var(t)) = atom.args.get(1)
                        && let Some(t) = placed.canonical(t)
                    {
                        vars.insert(tape(&t));
                    }
                }
                if vars.is_empty() {
                    // A read with no location variable (`field(_, "left", _)`'s rows, a count over them): any file's.
                    return Err(format!("reads {} with no location", atom.rel));
                }
                groups.push(vars);
            }
            // `a = b` joins a's group and b's (it makes no group of its own: two symbols equal place nothing).
            let equal = var_equalities(rule);
            for g in &mut groups {
                close_under(g, &equal);
            }
            // Merge groups sharing a variable; one group left is one document.
            let mut merged: Vec<BTreeSet<String>> = Vec::new();
            for group in groups {
                let (joined, rest): (Vec<_>, Vec<_>) = merged.into_iter().partition(|m| !m.is_disjoint(&group));
                let mut all = group;
                for j in joined {
                    all.extend(j);
                }
                merged = rest;
                merged.push(all);
            }
            // A constant-only rule has no document owner. Run it once in the cross-file phase,
            // including an empty corpus. Vocabulary filters remain per-chunk producers.
            if merged.len() == 1 || (merged.is_empty() && filters.contains(&rule.head.rel)) {
                Ok(())
            } else if merged.is_empty() {
                Err("reads no location".to_owned())
            } else {
                let parts: Vec<String> = merged.iter().map(|m| m.iter().filter(|v| !v.starts_with("rr_value")).cloned().collect::<Vec<_>>().join(" ")).collect();
                Err(format!("joins {} documents: {{{}}}", merged.len(), parts.join("} {")))
            }
        };
        let before = local.len();
        // In rounds, as before, so the causes are the same: a rule can newly fail only by reading a relation that left
        // in the round before; the first round checks every rule.
        let failing: Vec<(String, String)> = match &dirty {
            None => prog.rules.iter().filter(|r| local.contains(&r.head.rel)).filter_map(|r| rule_local(r).err().map(|why| (r.head.rel.clone(), why))).collect(),
            Some(rules) => rules.iter().map(|&i| &prog.rules[i]).filter(|r| local.contains(&r.head.rel)).filter_map(|r| rule_local(r).err().map(|why| (r.head.rel.clone(), why))).collect(),
        };
        let mut next: BTreeSet<usize> = BTreeSet::new();
        for (rel, why) in failing {
            if local.remove(&rel) {
                next.extend(index.reading(&rel));
            }
            causes.entry(rel).or_insert(why);
        }
        dirty = Some(next);
        if local.len() == before {
            let mut same: BTreeMap<String, Vec<usize>> =
                prog.inputs.iter().filter(|i| !derived.contains(i.as_str())).map(|i| (i.clone(), base_locations(i))).collect();
            same.extend(local.iter().map(|rel| (rel.clone(), locations[rel].clone())));
            return (local, locations, same, causes);
        }
    }
}

/// A program split at its file-local relations (`split`): `local` derives them, run per chunk of documents over the
/// chunk's base relations; `cross` derives the rest, run once over what the chunks wrote: `exports` (the file-local
/// relations a cross-file rule reads) and `base` (the base relations a cross-file rule reads, and `node` and `doc`,
/// which decode findings). In `cross`, both are inputs. `same_doc` names, for each file-local and base relation, the
/// columns whose values in one row lie in one document (`cross_rounds::bound`: a join through them crosses no file).
#[derive(Clone)]
pub(crate) struct Programs {
    pub(crate) local: crate::code_rule_dsl::logic::front::Program,
    pub(crate) cross: crate::code_rule_dsl::logic::front::Program,
    pub(crate) exports: Vec<String>,
    pub(crate) base: Vec<String>,
    /// Each derived relation's location columns (`split_located`).
    pub(crate) locations: std::collections::BTreeMap<String, Vec<usize>>,
    /// The same-document columns of each file-local or base relation (`split_located`): what a cross-file hop reads.
    pub(crate) same_doc: std::collections::BTreeMap<String, Vec<usize>>,
}

pub(crate) fn programs(prog: &crate::code_rule_dsl::logic::front::Program) -> Programs {
    use crate::code_rule_dsl::logic::front::Literal;
    use std::collections::BTreeSet;
    let (mut local, locations, same_doc) = split_located(prog);
    // A program that reads no document (a later phase over an earlier one's exports, `corpus::phases`) has no
    // file-local part: its relations made from those exports alone would be constants, derived again whole in the
    // file-local part each run and handed to the cross part as changed inputs. In the cross part a session maintains
    // them from the exports' delta (t28 step 7).
    let no_document = reads_no_document(prog);
    if no_document {
        local.clear();
    }
    let is_local = |rel: &str| local.contains(rel);
    let read = |rules: &mut dyn Iterator<Item = &crate::code_rule_dsl::logic::front::Rule>| -> BTreeSet<String> {
        rules
            .flat_map(|r| &r.body)
            .filter_map(|l| match l {
                Literal::Atom(a) | Literal::Agg { atom: a, .. } => Some(a.rel.clone()),
                _ => None,
            })
            .collect()
    };
    let cross_reads = read(&mut prog.rules.iter().filter(|r| !is_local(&r.head.rel)));
    // A file-local relation that only projects a base input (each rule one positive input atom: `kind` of `node`)
    // is not exported: the cross-file part derives it from the stored input (a view), not from a copy of it.
    let projection = |rel: &str| {
        let mut rules = prog.rules.iter().filter(|r| r.head.rel == rel).peekable();
        rules.peek().is_some()
            && !prog.lattice.contains_key(rel)
            && rules.all(|r| matches!(r.body.as_slice(), [Literal::Atom(a)] if !a.neg && prog.inputs.contains(&a.rel)))
    };
    let projected: BTreeSet<String> = cross_reads.iter().filter(|r| is_local(r) && projection(r)).cloned().collect();
    let in_cross = |rel: &str| !is_local(rel) || projected.contains(rel);
    // A relation made natively (`dfs_first`) is cross-file, and reads its inputs there: they are exported too.
    let native_reads = |mut reads: BTreeSet<String>| {
        reads.extend(prog.natives.iter().filter(|(rel, _)| in_cross(rel)).flat_map(|(_, n)| n.reads()).map(str::to_owned));
        reads
    };
    let cross_reads = native_reads(read(&mut prog.rules.iter().filter(|r| in_cross(&r.head.rel))));
    let exports: Vec<String> = cross_reads.iter().filter(|r| is_local(r) && !projected.contains(*r)).cloned().collect();
    let base_read: Vec<String> = prog.inputs.iter().filter(|i| cross_reads.contains(*i)).cloned().collect();
    let mut base = base_read.clone();
    for decode in ["node", "doc"] {
        if prog.inputs.iter().any(|i| i == decode) && !base.iter().any(|b| b == decode) {
            base.push(decode.to_owned());
        }
    }
    let mut l = prog.clone();
    // A relation of constants is derived in both parts (`constant_relations`), but where no document is read: the
    // file-local part then has nothing to give them to.
    let constants = if no_document { Default::default() } else { constant_relations(prog) };
    l.rules.retain(|r| is_local(&r.head.rel) || constants.contains(&r.head.rel));
    l.natives.retain(|rel, _| is_local(rel));
    l.lattice.retain(|rel, _| is_local(rel));
    l.gates.retain(|(rel, _)| is_local(rel));
    l.outputs = prog.outputs.iter().filter(|o| is_local(o)).cloned().chain(exports.iter().cloned()).collect::<BTreeSet<_>>().into_iter().collect();
    // The reference graph is bound for the cross-file part only (no rule reading it is file-local): no chunk has it.
    l.inputs.retain(|i| !matches!(i.as_str(), "reference_edge" | "reference_open"));
    let mut c = prog.clone();
    c.rules.retain(|r| in_cross(&r.head.rel));
    c.natives.retain(|rel, _| in_cross(rel));
    c.lattice.retain(|rel, _| !is_local(rel));
    c.gates.retain(|(rel, _)| !is_local(rel));
    c.outputs.retain(|o| !is_local(o));
    c.inputs = base_read.into_iter().chain(exports.iter().cloned()).collect();
    Programs { local: l, cross: c, exports, base, locations, same_doc }
}

/// Pushes the file-local part of each cross-file rule into the chunks: the rule's positive reads of base relations
/// and file-local relations, joined by their variables, with the comparisons over only their variables, become a
/// file-local relation of the variables the rest of the rule uses (`fl<k>_<head>`), and the rule reads that. So do
/// its negated reads of those relations whose variables the component binds.
///
/// Without this, a cross-file rule (one that negates or joins a cross-file relation) reads its base relations
/// whole: every chunk's `within`, `node`, `child` rows are kept for the cross part, memory that grows with the
/// corpus and with the rules run at once. With it, the chunk joins them and exports the few rows that pass.
///
/// A component that is one atom of only variables filters nothing and is left; so is one atom of a file-local
/// relation alone. A made relation the split does not find file-local is undone. Returns the relations made.
///
/// A component that narrows no node, only lookups of a node's attributes (`docof(n, d), doc(d, p, _), line(n, l)`,
/// a finding's location) or relations of documents (`p9_path_module(d, m)`), is a row per node of the chunk: on 200
/// files, 611,865 rows for each such rule, every one held by the cross part. It is pushed only when that keeps the
/// cross part off the base relations (so the per-file memo works); when the cross part reads them anyway, it reads
/// these lookups there too. So is a component that enumerates subtrees (`descends`): `within(n, a)` with `n`
/// narrowed only by its attributes.
pub(crate) fn push_local_joins(prog: &mut crate::code_rule_dsl::logic::front::Program) -> Vec<String> {
    let original = prog.clone();
    let (made, local) = push_with(prog, false);
    if cross_reads_base(prog, &local) {
        *prog = original;
        return push_with(prog, true).0;
    }
    made
}

/// Whether the factorized store is on (decision 0099): it is, unless `RERAIL_FACTORIZE=off` or the run's option
/// `factorize: off` turns it off, for comparison. The prepared program's key names it (`corpus`'s `prepared_key`).
pub(crate) fn factorize_on(options: &serde_json::Value) -> bool {
    std::env::var("RERAIL_FACTORIZE").map_or(true, |v| v != "off") && options.get("factorize").and_then(|v| v.as_str()) != Some("off")
}

/// At most this many rules of a relation stored factorized, and reader copies made of its reads.
const FACTOR_RULES: usize = 4;
const FACTOR_COPIES: usize = 8;

/// The factorized store across the chunk boundary (decision 0099). An export (a file-local relation only the
/// cross-file part reads) whose every rule joins parts that meet only at a variable its head projects away is handed
/// on as the parts, not as their product: `fl853_p4_outer(f, x) :- rr_inside_ks5(f, p), bound_in(p, x)` held 295 k
/// rows over the 486 fixture files, and is handed on as `fz_a(f, p)` and `fz_b(p, x)`, each about the size of its
/// own part. The cross-file readers read the parts where they read the product (`p4_outer(f, x) :- p4_read(_, x, f,
/// _), fz_a(f, p), fz_b(p, x)`): their join is the cross part's ordinary join, per key, from the columns they bind.
///
/// - **Each part** keeps its own comparisons and negations (`keeps::factors`), and is reduced by the others' keys
///   (`fz_a(f, p) :- …, fzk_b(p)`, `fzk_b(p) :- bound_in(p, x)`): every row stored then joins, so a part never holds
///   more rows than the product it replaces.
/// - **When:** decided from the program alone, before the first chunk: the per-file memo's key is a digest of the
///   file-local part (`corpus::memo::program_digest`), which names the exports, so the choice must not depend on a
///   chunk's rows. A changed choice (another engine, `RERAIL_FACTORIZE=off`) is another key, a memo miss.
///   The relation is factorized when at least two of its parts may hold many rows per key (a part that is a node's
///   attributes, `parent(n, p), text(p, t)`, holds one, and the product is no larger than the other part); when every
///   read of it is positive, in a cross-file rule that binds a column of it (so the reader's join starts from a
///   bound part, not the whole product again); and when the readers' copies stay few (one per rule of the relation,
///   at most `FACTOR_COPIES`).
///
/// Returns a line per relation factorized. A relation whose parts the split does not find file-local is left whole.
pub(crate) fn factorize_exports(prog: &mut crate::code_rule_dsl::logic::front::Program) -> Vec<String> {
    use crate::code_rule_dsl::logic::front::{Atom, Literal, Rule, Term};
    use std::collections::{BTreeMap, BTreeSet};
    let local = split(prog);
    let natives_read: BTreeSet<&str> = prog.natives.values().flat_map(|n| n.reads()).collect();
    let keys = relation_keys(prog);
    // Each relation's reads, and the relations read in a way the parts cannot stand in for.
    let mut reads: BTreeMap<&str, Vec<(usize, usize)>> = BTreeMap::new();
    let mut unfit: BTreeSet<&str> = BTreeSet::new();
    for (ri, r) in prog.rules.iter().enumerate() {
        for (li, l) in r.body.iter().enumerate() {
            match l {
                Literal::Atom(a) => {
                    if a.neg || a.gated || local.contains(&r.head.rel) || a.args.iter().any(|t| matches!(t, Term::Call(..) | Term::Top)) {
                        unfit.insert(a.rel.as_str());
                    } else {
                        reads.entry(a.rel.as_str()).or_default().push((ri, li));
                    }
                }
                Literal::Agg { atom, .. } => {
                    unfit.insert(atom.rel.as_str());
                }
                _ => {}
            }
        }
    }
    let mut rules_of: BTreeMap<&str, Vec<&Rule>> = BTreeMap::new();
    for r in &prog.rules {
        rules_of.entry(r.head.rel.as_str()).or_default().push(r);
    }
    // A plan per relation: per rule, its key and parts.
    let mut chosen: BTreeMap<String, Vec<(String, Vec<Vec<Literal>>)>> = BTreeMap::new();
    for (rel, read) in &reads {
        let rel = *rel;
        let engine = rel.starts_with("magic_") || rel.starts_with("done_") || rel.contains("__");
        if !local.contains(rel)
            || unfit.contains(rel)
            || engine
            || prog.outputs.iter().any(|o| o == rel)
            || prog.inputs.iter().any(|i| i == rel)
            || prog.lattice.contains_key(rel)
            || prog.natives.contains_key(rel)
            || natives_read.contains(rel)
            || prog.facts.get(rel).is_some_and(|f| !f.is_empty())
            || prog.gates.iter().any(|(g, _)| g == rel)
            || prog.answers.contains_key(rel)
            || prog.answers.values().flatten().any(|a| a == rel)
        {
            continue;
        }
        let Some(rules) = rules_of.get(rel) else { continue };
        if rules.len() > FACTOR_RULES || read.len() * rules.len() > FACTOR_COPIES {
            continue;
        }
        let mut plan = Vec::new();
        for rule in rules {
            // The head: distinct variables, so a read's terms stand for them one for one.
            let head: Option<Vec<&String>> = rule.head.args.iter().map(|t| if let Term::Var(v) = t { Some(v) } else { None }).collect();
            let Some(head) = head.filter(|h| h.iter().collect::<BTreeSet<_>>().len() == h.len()) else { break };
            let Some((key, parts)) = crate::code_rule_dsl::logic::plan::keeps::factors(rule) else { break };
            // Each part reads the key in a positive atom: it is a part of the key's join, not a product of its own.
            let keyed = |part: &Vec<Literal>| part.iter().any(|l| matches!(l, Literal::Atom(a) if !a.neg && a.vars().contains(&key)));
            if !parts.iter().all(keyed) {
                break;
            }
            let fans = parts.iter().filter(|part| !functional(part, &key, &head, &keys)).count();
            if fans < 2 {
                break;
            }
            plan.push((key, parts));
        }
        if plan.len() != rules.len() {
            continue;
        }
        // Every read binds a column of the relation: a constant, or a variable another positive atom of the reader names.
        let binds = read.iter().all(|&(ri, li)| {
            let reader = &prog.rules[ri];
            let Literal::Atom(a) = &reader.body[li] else { return false };
            a.args.iter().any(|t| match t {
                Term::Int(_) | Term::Str(_) => true,
                Term::Var(v) => reader.body.iter().enumerate().any(|(j, l)| j != li && matches!(l, Literal::Atom(b) if !b.neg && b.vars().contains(v))),
                _ => false,
            })
        });
        if binds {
            chosen.insert(rel.to_owned(), plan);
        }
    }
    // Made, checked by the split, and made again without the relations whose parts it does not find file-local.
    loop {
        if chosen.is_empty() {
            return Vec::new();
        }
        let mut next = prog.clone();
        let mut lines = Vec::new();
        let mut made: BTreeMap<String, Vec<String>> = BTreeMap::new();
        // Per relation, per rule: the parts' atoms over the rule's own variables.
        let mut stands: BTreeMap<String, Vec<(Vec<String>, Vec<Atom>)>> = BTreeMap::new();
        for (rel, plan) in &chosen {
            let rules = &rules_of[rel.as_str()];
            let mut shown = Vec::new();
            let mut per_rule = Vec::new();
            let mut ok = true;
            for (rule, (key, parts)) in rules.iter().zip(plan) {
                let head: Vec<String> = rule.head.args.iter().filter_map(|t| if let Term::Var(v) = t { Some(v.clone()) } else { None }).collect();
                let typed = |part: &[Literal], v: &str| {
                    part.iter().find_map(|l| match l {
                        Literal::Atom(a) if !a.neg => {
                            let c = a.args.iter().position(|t| matches!(t, Term::Var(x) if x == v))?;
                            prog.decls.get(&a.rel)?.get(c).map(|(_, t)| (v.to_owned(), t.clone()))
                        }
                        _ => None,
                    })
                };
                // Each part's interface: the head's variables it names, in the head's order, then the key.
                let mut atoms = Vec::new();
                let mut keys = Vec::new();
                let mut bodies = Vec::new();
                for part in parts {
                    let mut vars = BTreeSet::new();
                    part.iter().for_each(|l| vars.extend(l.vars()));
                    let mut iface: Vec<String> = head.iter().filter(|v| vars.contains(*v)).cloned().collect();
                    iface.push(key.clone());
                    let Some(cols) = iface.iter().map(|v| typed(part, v)).collect::<Option<Vec<_>>>() else {
                        ok = false;
                        break;
                    };
                    let name = format!("fz{}_{rel}", next.decls.len());
                    next.decls.insert(name.clone(), cols.clone());
                    let kname = format!("fzk{}_{rel}", next.decls.len());
                    next.decls.insert(kname.clone(), vec![cols.last().cloned().expect("the key")]);
                    atoms.push(Atom { rel: name, args: iface.iter().map(|v| Term::Var(v.clone())).collect(), neg: false, gated: false });
                    keys.push(Atom { rel: kname, args: vec![Term::Var(key.clone())], neg: false, gated: false });
                    bodies.push(part.clone());
                }
                if !ok {
                    break;
                }
                for (i, (atom, body)) in atoms.iter().zip(&bodies).enumerate() {
                    next.rules.push(Rule { head: keys[i].clone(), body: body.clone() });
                    let mut reduced = body.clone();
                    reduced.extend(keys.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, k)| Literal::Atom(k.clone())));
                    next.rules.push(Rule { head: atom.clone(), body: reduced });
                    made.entry(rel.clone()).or_default().extend([atom.rel.clone(), keys[i].rel.clone()]);
                }
                shown.push(format!(
                    "by ${key}: {}",
                    atoms.iter().zip(&bodies).map(|(a, b)| format!("{a} :- {}", b.iter().map(ToString::to_string).collect::<Vec<_>>().join(", "))).collect::<Vec<_>>().join(" | ")
                ));
                per_rule.push((head, atoms));
            }
            if !ok {
                continue;
            }
            lines.push(format!("{rel}: handed on as parts, each reduced by the others' keys, read in {} cross-file rules; {}", reads[rel.as_str()].len(), shown.join("; ")));
            stands.insert(rel.clone(), per_rule);
        }
        // The relation's rules go; each read becomes the parts of each of its rules, the key and the variables the
        // read leaves open named apart.
        let mut fresh = 0usize;
        let mut rules = Vec::with_capacity(next.rules.len());
        for rule in std::mem::take(&mut next.rules) {
            if stands.contains_key(&rule.head.rel) {
                continue;
            }
            let mut work = vec![rule];
            while let Some(rule) = work.pop() {
                let at = rule.body.iter().position(|l| matches!(l, Literal::Atom(a) if !a.neg && stands.contains_key(&a.rel)));
                let Some(at) = at else {
                    rules.push(rule);
                    continue;
                };
                let Literal::Atom(read) = &rule.body[at] else { unreachable!() };
                for (head, atoms) in &stands[&read.rel] {
                    fresh += 1;
                    let to = |v: &str| -> Term {
                        match head.iter().position(|h| h == v) {
                            Some(i) => read.args[i].clone(),
                            None => Term::Var(format!("fz{fresh}_{v}")),
                        }
                    };
                    let parts = atoms.iter().map(|a| Literal::Atom(Atom { rel: a.rel.clone(), args: a.args.iter().map(|t| if let Term::Var(v) = t { to(v) } else { t.clone() }).collect(), neg: false, gated: false }));
                    let mut copy = rule.clone();
                    copy.body.splice(at..=at, parts);
                    work.push(copy);
                }
            }
        }
        // Readers first in program order, as they were.
        next.rules = rules;
        for rel in stands.keys() {
            next.decls.remove(rel);
        }
        let now = split(&next);
        let failed: Vec<String> = made.iter().filter(|(_, parts)| parts.iter().any(|p| !now.contains(p))).map(|(r, _)| r.clone()).collect();
        if failed.is_empty() {
            *prog = next;
            return lines;
        }
        for r in failed {
            chosen.remove(&r);
        }
    }
}

/// Whether `part` holds at most one row per value of `key`: its head variables (`head`) follow from the key through
/// atoms read by a key of their relation (`keys`, `relation_keys`), equalities and nearest ancestors.
fn functional(part: &[crate::code_rule_dsl::logic::front::Literal], key: &str, head: &[&String], keys: &std::collections::BTreeMap<String, Vec<Vec<usize>>>) -> bool {
    let mut vars = std::collections::BTreeSet::new();
    part.iter().for_each(|l| vars.extend(l.vars()));
    let known = determined(part, std::collections::BTreeSet::from([key.to_owned()]), keys);
    head.iter().filter(|h| vars.contains(h.as_str())).all(|h| known.contains(h.as_str()))
}

/// The variables `body` determines from `known`: an atom whose relation has a key all of whose columns are known (or
/// constants) determines its other variables; so does an equality, and a nearest ancestor its node.
fn determined(body: &[crate::code_rule_dsl::logic::front::Literal], mut known: std::collections::BTreeSet<String>, keys: &std::collections::BTreeMap<String, Vec<Vec<usize>>>) -> std::collections::BTreeSet<String> {
    use crate::code_rule_dsl::logic::front::{Literal, Term};
    loop {
        let before = known.len();
        for l in body {
            match l {
                Literal::Atom(a) if !a.neg => {
                    let fixed = |c: &usize| match a.args.get(*c) {
                        Some(Term::Var(v)) => known.contains(v),
                        Some(Term::Int(_) | Term::Str(_)) => true,
                        _ => false,
                    };
                    if keys.get(&a.rel).is_some_and(|ks| ks.iter().any(|k| k.iter().all(fixed))) {
                        known.extend(a.vars());
                    }
                }
                Literal::Cmp { l, op: "=", r } => {
                    for (x, y) in [(l, r), (r, l)] {
                        let mut ys = std::collections::BTreeSet::new();
                        y.vars(&mut ys);
                        if let Term::Var(x) = x
                            && ys.is_subset(&known)
                        {
                            known.insert(x.clone());
                        }
                    }
                }
                Literal::Nearest { n, a, .. } if known.contains(n) => {
                    known.insert(a.clone());
                }
                _ => {}
            }
        }
        if known.len() == before {
            return known;
        }
    }
}

/// Keys of the program's relations, as far as the program shows them: a node's attributes by the node (`docof`,
/// `text`, `node` by its node, and a view of one); the tree's edges by the child, and by the parent with the field or
/// the index (`child(p, c, i)`, `field(p, f, c)`); a lattice by its key columns; and a derived relation by a column
/// that, in every rule, determines the head (`determined`), found to a fixpoint. They price the factorized store's
/// test only (a part of one row a key holds no more rows than the product): a wrong key costs rows, never findings.
fn relation_keys(prog: &crate::code_rule_dsl::logic::front::Program) -> std::collections::BTreeMap<String, Vec<Vec<usize>>> {
    use crate::code_rule_dsl::logic::front::{Literal, Term};
    let mut keys: std::collections::BTreeMap<String, Vec<Vec<usize>>> = std::collections::BTreeMap::new();
    for rel in &prog.inputs {
        let (Some(columns), Some(types)) = (crate::code_rule_dsl::logic::sql::select::input_columns(rel), prog.decls.get(rel)) else { continue };
        if columns.len() != types.len() {
            continue;
        }
        let at = |name: &str| columns.iter().position(|c| *c == name);
        match (rel.as_str(), at("p"), at("c")) {
            ("child" | "field", Some(p), Some(c)) => {
                let other = (0..columns.len()).find(|&i| i != p && i != c);
                keys.insert(rel.clone(), std::iter::once(vec![c]).chain(other.map(|o| vec![p, o])).collect());
                continue;
            }
            _ => {}
        }
        let Some(k) = at("n") else { continue };
        let others_nodes = types.iter().enumerate().any(|(i, (_, t))| i != k && (matches!(t.as_str(), "node" | "loc" | "span") || matches!(columns.get(i), Some(&("par" | "a" | "p" | "h")))));
        if !others_nodes {
            keys.insert(rel.clone(), vec![vec![k]]);
        }
    }
    for (rel, l) in &prog.lattice {
        keys.insert(rel.clone(), vec![(0..l.keys).collect()]);
    }
    let derived: std::collections::BTreeSet<&str> = prog.rules.iter().map(|r| r.head.rel.as_str()).filter(|r| !keys.contains_key(*r)).collect();
    let mut rules_of: std::collections::BTreeMap<&str, Vec<&crate::code_rule_dsl::logic::front::Rule>> = std::collections::BTreeMap::new();
    for r in &prog.rules {
        if derived.contains(r.head.rel.as_str()) {
            rules_of.entry(r.head.rel.as_str()).or_default().push(r);
        }
    }
    // A key proven stays proven as more are found (each proof reads keys only positively), so the rounds rise.
    for _ in 0..6 {
        let mut found = false;
        for (rel, rules) in &rules_of {
            let width = rules[0].head.args.len();
            for c in 0..width {
                if keys.get(*rel).is_some_and(|ks| ks.contains(&vec![c])) {
                    continue;
                }
                let proven = rules.iter().all(|r| {
                    let mut head = std::collections::BTreeSet::new();
                    r.head.args.iter().for_each(|t| t.vars(&mut head));
                    let start = match r.head.args.get(c) {
                        Some(Term::Var(v)) => std::collections::BTreeSet::from([v.clone()]),
                        _ => return false,
                    };
                    r.body.iter().all(|l| matches!(l, Literal::Atom(_) | Literal::Cmp { .. } | Literal::Nearest { .. })) && head.is_subset(&determined(&r.body, start, &keys))
                });
                if proven {
                    keys.entry((*rel).to_owned()).or_default().push(vec![c]);
                    found = true;
                }
            }
        }
        if !found {
            break;
        }
    }
    keys
}

/// The base relations the cross-file part can take as a semi-join: of each, a chunk keeps only the rows of the nodes
/// its exports' rows hold, not every node's. Decided per relation, so that in a pack one policy's cross-file rule that
/// walks the tree does not make every other policy's lookups hold every node's rows.
///
/// A node attribute (`docof`, `line`, `text`, `node`: a node `n` and no other node) can be a semi-join when every read
/// of it looks up a node that is *held*: bound, in the same body, by a positive read of a relation whose nodes all trace
/// back to an export's row. Nodes that do not trace back are *tainted*. They come from a base relation that reaches
/// nodes and is not an attribute (`child(p, c)`, `within`), from a native step over one (`dfs_first`), from an
/// attribute read with an unbound node (`node(d, n, ...)` enumerating a document's nodes: that attribute is then kept
/// whole), and from any relation a rule derives from such nodes. A finding's decoding reads `node` of the findings'
/// nodes, so `node` is kept whole when a tainted relation is an output. The sets shrink to a fixpoint. `Some` always:
/// the empty set where no attribute qualifies.
///
/// On the full data-layer tree, `lazy-architecture`'s cross part held 673 MB of these for about ten thousand
/// lookups; on 200 files, the 66-rule pack's held 1.1M rows of each of `line`, `col` and `docof`, because one rule's
/// walk turned the semi-join off for all of them.
pub(crate) fn semijoin_bases(cross: &crate::code_rule_dsl::logic::front::Program, base: &[String]) -> Option<std::collections::BTreeSet<String>> {
    use crate::code_rule_dsl::logic::front::{Atom, Literal, Term};
    use std::collections::{BTreeMap, BTreeSet};
    // Each attribute relation's node column: the base ones, and a view the cross part derives from one
    // (`kind($n, $k) :- node(_, $n, $k, _, _)`), keyed where the node lands in its head.
    let mut key: BTreeMap<String, usize> = BTreeMap::new();
    for rel in base {
        let (Some(columns), Some(types)) = (crate::code_rule_dsl::logic::sql::select::input_columns(rel), cross.decls.get(rel)) else { continue };
        let Some(k) = columns.iter().position(|c| *c == "n") else { continue };
        // Another column of nodes: by its type, or by its name where the type is a number (`tape`'s parent `par`, an
        // ancestor `a`, `ast_match`'s pattern root `p`, `host_node`'s host `h`); a walk up `tape(p, par)` reaches
        // parents no export holds.
        let others_nodes = types.iter().enumerate().any(|(i, (_, t))| i != k && (matches!(t.as_str(), "node" | "loc" | "span") || matches!(columns[i], "par" | "a" | "p" | "h")));
        if !others_nodes && columns.len() == types.len() {
            key.insert(rel.clone(), k);
        }
    }
    // A view's base: a view is a semi-join exactly when its base is.
    let mut view_of: BTreeMap<String, String> = BTreeMap::new();
    for rule in &cross.rules {
        let [Literal::Atom(a)] = rule.body.as_slice() else { continue };
        let Some(Term::Var(n)) = key.get(&a.rel).and_then(|&k| a.args.get(k)).filter(|_| !a.neg) else { continue };
        let Some(at) = rule.head.args.iter().position(|t| matches!(t, Term::Var(v) if v == n)) else { continue };
        let sole = cross.rules.iter().filter(|r| r.head.rel == rule.head.rel).count() == 1;
        if sole && !key.contains_key(&rule.head.rel) && !view_of.contains_key(&a.rel) {
            key.insert(rule.head.rel.clone(), at);
            view_of.insert(rule.head.rel.clone(), a.rel.clone());
        }
    }
    let root = |rel: &str| view_of.get(rel).cloned().unwrap_or_else(|| rel.to_owned());
    // A relation with no node column reaches no node.
    let reaches_nodes = |rel: &str| cross.decls.get(rel).is_none_or(|cols| cols.iter().any(|(_, t)| matches!(t.as_str(), "node" | "loc" | "span")));
    let is_base = |rel: &str| base.iter().any(|b| b == rel);
    // The attributes still taken as semi-joins (by their base relation), and the relations whose nodes may not trace
    // back to an export.
    let mut semi: BTreeSet<String> = key.keys().filter(|r| !view_of.contains_key(*r)).cloned().collect();
    let mut tainted: BTreeSet<String> = BTreeSet::new();
    loop {
        let (semi_before, tainted_before) = (semi.len(), tainted.len());
        // A source of tainted nodes: a base relation that reaches nodes and is no attribute, or a tainted relation.
        let source = |a: &Atom, tainted: &BTreeSet<String>| !a.neg && !key.contains_key(&a.rel) && (tainted.contains(&a.rel) || (is_base(&a.rel) && reaches_nodes(&a.rel)));
        for (rel, native) in &cross.natives {
            for r in native.reads() {
                if key.contains_key(r) && semi.remove(&root(r)) {
                }
                if tainted.contains(r) || (is_base(r) && reaches_nodes(r)) || key.contains_key(r) {
                    tainted.insert(rel.clone());
                }
            }
        }
        for rule in cross.rules.iter().filter(|r| !view_of.contains_key(&r.head.rel)) {
            // Held variables: bound by a positive read of a relation that is no attribute and holds no tainted node.
            let positive: Vec<&Atom> = rule.body.iter().filter_map(|l| if let Literal::Atom(a) = l { Some(a) } else { None }).filter(|a| !a.neg).collect();
            let vars = |a: &Atom| a.args.iter().filter_map(|t| if let Term::Var(v) = t { Some(v.clone()) } else { None }).collect::<Vec<_>>();
            // A variable equal to a held one is held; one equal to a loose one is loose (`m = n`).
            let equal = var_equalities(rule);
            let mut held: BTreeSet<String> = positive.iter().filter(|a| !key.contains_key(&a.rel) && !source(a, &tainted)).flat_map(|a| vars(a)).collect();
            close_under(&mut held, &equal);
            let mut loose: BTreeSet<String> = positive.iter().filter(|a| source(a, &tainted)).flat_map(|a| vars(a)).collect();
            for l in &rule.body {
                let (a, in_agg) = match l {
                    Literal::Atom(a) => (a, false),
                    Literal::Agg { atom: a, .. } => (a, true),
                    _ => continue,
                };
                let Some(&k) = key.get(&a.rel) else { continue };
                match a.args.get(k) {
                    Some(Term::Var(v)) if held.contains(v) => {}
                    found => {
                        // A lookup of a node that may not be held, or an enumeration: the attribute is kept whole.
                        semi.remove(&root(&a.rel));
                        // Its node, not held, is tainted or new (an enumeration).
                        if let (Some(Term::Var(v)), false, false) = (found, a.neg, in_agg) {
                            loose.insert(v.clone());
                        }
                    }
                }
            }
            close_under(&mut loose, &equal);
            let head_tainted = rule.head.args.iter().any(|t| matches!(t, Term::Var(v) if loose.contains(v) && !held.contains(v)));
            if head_tainted {
                tainted.insert(rule.head.rel.clone());
            }
        }
        if cross.outputs.iter().any(|o| tainted.contains(o)) && semi.remove("node") {
        }
        if semi.len() == semi_before && tainted.len() == tainted_before {
            break;
        }
    }
    Some(semi)
}

/// Whether a rule outside the chunks (`local`: `split(prog)`) reads a base relation.
fn cross_reads_base(prog: &crate::code_rule_dsl::logic::front::Program, local: &std::collections::BTreeSet<String>) -> bool {
    prog.rules.iter().filter(|r| !local.contains(&r.head.rel)).any(|r| {
        r.body.iter().any(|l| l.atom().is_some_and(|a| prog.inputs.iter().any(|i| *i == a.rel)))
    })
}

/// Whether nothing in `atoms` narrows a node: each is a plain lookup of a node's attributes (no constant, no repeated
/// variable) or has no node column, and some node is read.
fn unrestricted(prog: &crate::code_rule_dsl::logic::front::Program, atoms: &[&crate::code_rule_dsl::logic::front::Atom]) -> bool {
    use crate::code_rule_dsl::logic::front::Term;
    let attribute = |rel: &str| matches!(rel, "docof" | "line" | "end_line" | "col" | "visit_index" | "node_order" | "node" | "doc");
    let plain = |a: &crate::code_rule_dsl::logic::front::Atom| {
        let named: Vec<&String> = a.args.iter().filter_map(|t| if let Term::Var(x) = t { Some(x) } else { None }).collect();
        a.args.iter().all(|t| matches!(t, Term::Var(_) | Term::Wild)) && named.iter().collect::<std::collections::BTreeSet<_>>().len() == named.len()
    };
    atoms.iter().any(|a| matches!(a.rel.as_str(), "docof" | "line" | "end_line" | "col" | "visit_index" | "node_order" | "node"))
        && atoms.iter().all(|a| (attribute(&a.rel) && plain(a)) || (!attribute(&a.rel) && !prog.decls.get(&a.rel).is_some_and(|cols| cols.iter().any(|(_, t)| matches!(t.as_str(), "node" | "loc" | "span")))))
}

/// Whether a component enumerates subtrees: `within(n, a)` with `n` read by no other atom of it but a node's
/// attributes (`docof`, `visit_index`, `line`, …). Its rows are every node under each `a` it narrows, whatever reads
/// it: `c4_shadow`'s `binding(s, x, site, _), within(n, s), earlier(site, n)` made 922,044 rows on the Python pack's
/// fixtures for a cross-file reader that binds `n` from a few hundred asks (`c4_builtin_ask(n, x)`).
fn descends(atoms: &[&crate::code_rule_dsl::logic::front::Atom]) -> bool {
    use crate::code_rule_dsl::logic::front::Term;
    let attribute = |rel: &str| matches!(rel, "docof" | "line" | "end_line" | "col" | "visit_index" | "node_order" | "ast_depth");
    atoms.iter().any(|w| match (w.rel.as_str(), w.args.as_slice()) {
        ("within", [Term::Var(n), a]) if a != &Term::Var(n.clone()) => atoms
            .iter()
            .filter(|b| !std::ptr::eq(**b, *w) && b.args.iter().any(|t| matches!(t, Term::Var(x) if x == n)))
            .all(|b| attribute(&b.rel)),
        _ => false,
    })
}

/// The relations made, and the split of the program as it is left (`split(prog)`).
fn push_with(prog: &mut crate::code_rule_dsl::logic::front::Program, keep_unrestricted: bool) -> (Vec<String>, std::collections::BTreeSet<String>) {
    use crate::code_rule_dsl::logic::front::{Atom, Literal, Rule, Term};
    use std::collections::{BTreeMap, BTreeSet};
    // An atom such as field(parent, name, child) connects its node columns. Document equality
    // and comparisons of node attributes do not: enumerating their independent node groups
    // before the remaining candidate predicates creates quadratic or cubic intermediate relations.
    // An atom with no node column that meets the rest only through the document, with two or more columns of its
    // own (a document's imports, `la_imp(d, name, symbol)`), is a table per document: joined to the document's
    // nodes it multiplies them by its rows, so it is a group of its own.
    let independent_node_groups = |atoms: &[&Atom], equal: &[(String, String)]| {
        let documents: BTreeSet<&String> = atoms
            .iter()
            .filter_map(|a| match (a.rel.as_str(), a.args.as_slice()) {
                ("docof", [_, Term::Var(d)]) => Some(d),
                ("node" | "doc", [Term::Var(d), ..]) => Some(d),
                _ => None,
            })
            .collect();
        let mut groups: Vec<BTreeSet<String>> = Vec::new();
        for atom in atoms {
            let Some(columns) = prog.decls.get(&atom.rel) else { continue };
            let mut nodes: BTreeSet<String> = atom.args.iter().zip(columns).filter_map(|(term, (_, ty))| {
                if matches!(ty.as_str(), "node" | "loc" | "span") && let Term::Var(value) = term { Some(value.clone()) } else { None }
            }).collect();
            if nodes.is_empty() {
                let own: BTreeSet<String> = atom.args.iter().filter_map(|t| if let Term::Var(v) = t { Some(v.clone()) } else { None }).filter(|v| !documents.contains(v)).collect();
                let met = atoms.iter().any(|b| !std::ptr::eq(*b, *atom) && b.args.iter().any(|t| matches!(t, Term::Var(v) if own.contains(v))));
                if own.len() < 2 || met {
                    continue;
                }
                nodes = own;
            }
            // `a = b` names one node twice.
            close_under(&mut nodes, equal);
            let mut connected = nodes;
            let mut separate = Vec::new();
            for group in groups {
                if group.is_disjoint(&connected) { separate.push(group); } else { connected.extend(group); }
            }
            separate.push(connected);
            groups = separate;
        }
        groups.len()
    };
    // Per rule, how much it takes: 0 negated base and file-local reads, 1 negated base reads only, 2 nothing (the
    // split did not find the join file-local: a negation read through no location column, say).
    let mut level: BTreeMap<usize, u8> = BTreeMap::new();
    // The program is changed only when a round succeeds, so its split is found once.
    let local = split(prog);
    loop {
        let mut next = prog.clone();
        let mut made: Vec<(usize, String)> = Vec::new();
        let mut helpers: Vec<Rule> = Vec::new();
        for (i, rule) in prog.rules.iter().enumerate() {
            let at = level.get(&i).copied().unwrap_or(0);
            if local.contains(&rule.head.rel) || at >= 2 {
                continue;
            }
            let base = |rel: &str| prog.inputs.iter().any(|r| r == rel);
            let candidate = |l: &Literal| matches!(l, Literal::Atom(a) if !a.neg && !prog.lattice.contains_key(&a.rel) && (base(&a.rel) || local.contains(&a.rel)));
            // Components of the candidates by shared variables. `a = b` between two variables (a disjunction's
            // branch, as lowering leaves it: `(e0 = d ; …)`) names one value, so it joins the atoms on each side; apart,
            // each would be pushed alone and their join left to the cross-file part (decision 0093 §4, O140).
            let idx: Vec<usize> = (0..rule.body.len()).filter(|&j| candidate(&rule.body[j])).collect();
            let equal = var_equalities(rule);
            let mut comps: Vec<(BTreeSet<usize>, BTreeSet<String>)> = Vec::new();
            for &j in &idx {
                let mut vars = rule.body[j].vars();
                close_under(&mut vars, &equal);
                let (joined, rest): (Vec<_>, Vec<_>) = comps.into_iter().partition(|(_, v)| !v.is_disjoint(&vars));
                let mut all = (BTreeSet::from([j]), vars);
                for (l, v) in joined {
                    all.0.extend(l);
                    all.1.extend(v);
                }
                comps = rest;
                comps.push(all);
            }
            let mut body = rule.body.clone();
            let mut taken: BTreeSet<usize> = BTreeSet::new();
            let mut replaced: Vec<(usize, Literal)> = Vec::new();
            for (mut lits, vars) in comps {
                let atoms: Vec<&Atom> = lits.iter().filter_map(|&j| rule.body[j].atom()).collect();
                // A join of file-local relations alone is pushed too: exported, each would be held whole by the
                // cross part (`la_leaf_target`, a row per node, for the 311 rows its join with `la_target` makes).
                if !atoms.iter().any(|a| base(&a.rel)) && atoms.len() < 2 {
                    continue;
                }
                if independent_node_groups(&atoms, &equal) > 1 {
                    continue;
                }
                // One plain atom, alone or widened by plain attribute lookups of its nodes (`within(n, b)` and n's
                // start), narrows nothing: pushed, it would be a copy of the atom with more columns.
                let plain = |a: &Atom| {
                    let named: Vec<&String> = a.args.iter().filter_map(|t| if let Term::Var(v) = t { Some(v) } else { None }).collect();
                    let distinct: BTreeSet<&String> = named.iter().copied().collect();
                    a.args.iter().all(|t| matches!(t, Term::Var(_) | Term::Wild)) && distinct.len() == named.len()
                };
                let attribute = |a: &Atom| matches!(a.rel.as_str(), "docof" | "line" | "end_line" | "col" | "visit_index" | "node_order" | "node" | "doc") && plain(a);
                let rest: Vec<&&Atom> = atoms.iter().filter(|a| !attribute(a)).collect();
                if let [a] = rest[..]
                    && plain(a)
                {
                    let own: BTreeSet<String> = a.args.iter().filter_map(|t| if let Term::Var(v) = t { Some(v.clone()) } else { None }).collect();
                    // Each lookup reads a node of the atom (its first column for `node`, `docof`, `line`, …).
                    let of_atom = atoms.iter().filter(|b| attribute(b)).all(|b| {
                        let key = if b.rel == "node" { b.args.get(1) } else { b.args.first() };
                        matches!(key, Some(Term::Var(v)) if own.contains(v))
                    });
                    if of_atom {
                        continue;
                    }
                }
                if let [a] = atoms[..]
                    && plain(a)
                {
                    continue;
                }
                // A component that holds a row for every node of the chunk (nothing in it narrows its nodes) is left
                // when the cross part reads base relations anyway (`push_local_joins`).
                if keep_unrestricted && (unrestricted(prog, &atoms) || descends(&atoms)) {
                    continue;
                }
                // Comparisons and negated file-local reads over only this component's variables go with it.
                for (j, l) in rule.body.iter().enumerate() {
                    let with = match l {
                        Literal::Cmp { .. } => true,
                        Literal::Atom(a) => a.neg && !a.gated && !prog.lattice.contains_key(&a.rel) && (base(&a.rel) || (at == 0 && local.contains(&a.rel))),
                        _ => false,
                    };
                    if with && !l.vars().is_empty() && l.vars().is_subset(&vars) {
                        lits.insert(j);
                    }
                }
                // The head's variables, those inside a call (`@cat(p, s)`) too.
                let mut outside: BTreeSet<String> = BTreeSet::new();
                rule.head.args.iter().for_each(|t| t.vars(&mut outside));
                for (j, l) in rule.body.iter().enumerate() {
                    if !lits.contains(&j) {
                        outside.extend(l.vars());
                    }
                }
                // The interface in the order the variables first appear.
                let mut iface: Vec<String> = Vec::new();
                for &j in &lits {
                    for v in rule.body[j].vars() {
                        if outside.contains(&v) && !iface.contains(&v) {
                            iface.push(v);
                        }
                    }
                }
                if iface.is_empty() {
                    continue;
                }
                // A composition the rule filters from outside is left (t31 A1): its atoms join on a variable the
                // rule projects away, and a positive read the chunks do not have (a cross-file relation) binds every
                // variable of its interface. Pushed, it is exported unfiltered: F823's
                // `outer(f, x) :- outer_cand(f, x), inside(f, p, fscope), bound_in(p, x)` exported every function x
                // every name bound around it, 15 M rows for one 291 KB file, 55% of the per-file memo. Left, the
                // cross part starts from the filter and probes the two.
                let joined_away = vars.iter().any(|v| {
                    !iface.contains(v) && atoms.iter().filter(|a| a.args.iter().any(|t| matches!(t, Term::Var(x) if x == v))).count() >= 2
                });
                let filtered = rule.body.iter().enumerate().any(|(j, l)| {
                    !lits.contains(&j)
                        && matches!(l, Literal::Atom(a) if !a.neg && !prog.lattice.contains_key(&a.rel) && !candidate(l)
                            && iface.iter().all(|v| a.args.iter().any(|t| matches!(t, Term::Var(x) if x == v))))
                });
                if joined_away && filtered {
                    continue;
                }
                let typed: Option<Vec<(String, String)>> = iface
                    .iter()
                    .map(|v| {
                        atoms.iter().find_map(|a| {
                            let c = a.args.iter().position(|t| matches!(t, Term::Var(x) if x == v))?;
                            prog.decls.get(&a.rel)?.get(c).map(|(_, t)| (v.clone(), t.clone()))
                        })
                    })
                    .collect();
                let Some(typed) = typed else { continue };
                let name = format!("fl{}_{}", next.decls.len(), rule.head.rel);
                next.decls.insert(name.clone(), typed);
                let head = Atom { rel: name.clone(), args: iface.iter().map(|v| Term::Var(v.clone())).collect(), neg: false, gated: false };
                helpers.push(Rule { head: head.clone(), body: lits.iter().map(|&j| rule.body[j].clone()).collect() });
                replaced.push((*lits.iter().next().expect("a literal"), Literal::Atom(head)));
                taken.extend(lits);
                made.push((i, name));
            }
            if taken.is_empty() {
                continue;
            }
            let mut out = Vec::new();
            for (j, l) in body.drain(..).enumerate() {
                if let Some((_, r)) = replaced.iter().find(|(at, _)| *at == j) {
                    out.push(r.clone());
                } else if !taken.contains(&j) {
                    out.push(l);
                }
            }
            next.rules[i].body = out;
        }
        if made.is_empty() {
            return (Vec::new(), local);
        }
        next.rules.extend(helpers);
        let now = split(&next);
        let failed: Vec<usize> = made.iter().filter(|(_, n)| !now.contains(n)).map(|(i, _)| *i).collect();
        if failed.is_empty() {
            *prog = next;
            return (made.into_iter().map(|(_, n)| n).collect(), now);
        }
        for i in failed {
            *level.entry(i).or_default() += 1;
        }
    }
}

/// Gives every relation the chunks hand on (an export, or a file-local output) a location column, so each of its
/// rows names its document (decision 0052): a relation without one, `r(p, l)`, is derived by the chunks as
/// `r_at(d, p, l)` (`r_at_` where the program has an `r_at` of its own), `d` a document its rules bind (from `node`,
/// `doc`, `tape_line`, …, a document column left unnamed included, or a node through `node(d, n, …)`), and the
/// cross-file part reads `r(p, l) :- r_at(_, p, l)`. A relation that file-local rules also read, a lattice, or
/// one whose rule binds no location is left; its name is returned in the `Err`: its rows are not attributable.
pub(crate) fn locate(prog: &mut crate::code_rule_dsl::logic::front::Program) -> Result<Vec<String>, Vec<String>> {
    use crate::code_rule_dsl::logic::front::{Atom, Literal, Rule, Term};
    use std::collections::BTreeSet;
    let split = programs(prog);
    let local: BTreeSet<&str> = split.local.rules.iter().map(|r| r.head.rel.as_str()).collect();
    let targets: BTreeSet<String> = split
        .exports
        .iter()
        .chain(split.local.outputs.iter())
        .filter(|r| local.contains(r.as_str()) && split.locations.get(*r).is_none_or(Vec::is_empty))
        .cloned()
        .collect();
    // Spans are located through docof, as are ordinary nodes.
    let docof = prog.inputs.iter().any(|r| r == "docof");
    let decls = prog.decls.clone();
    let node_typed = |rel: &str, c: usize| decls.get(rel).and_then(|cols| cols.get(c)).is_some_and(|(_, t)| t == "node");
    let doc_type = prog.decls.get("node").and_then(|cols| cols.first()).map(|(_, t)| t.clone()).unwrap_or_else(|| "int".to_owned());
    let (mut located, mut blocked) = (Vec::new(), Vec::new());
    let mut rules = std::mem::take(&mut prog.rules);
    // How many rules of a file-local head read each relation (not their own): a target read so is blocked. Kept as
    // the rules change (a located relation's rules move to `<rel>_at`, no file-local head), not counted again per
    // target over every rule.
    let reads = |rule: &Rule| -> BTreeSet<String> { rule.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.clone()).filter(|a| *a != rule.head.rel).collect() };
    let mut readers: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for rule in rules.iter().filter(|r| local.contains(r.head.rel.as_str())) {
        for a in reads(rule) {
            *readers.entry(a).or_default() += 1;
        }
    }
    for rel in &targets {
        let read_locally = readers.get(rel).is_some_and(|&n| n > 0);
        let Some(cols) = prog.decls.get(rel).cloned().filter(|_| !read_locally && !prog.lattice.contains_key(rel)) else {
            blocked.push(rel.clone());
            continue;
        };
        // The located relation: `r_at`, unless the program has a relation of that name (then `r_at_`, …).
        let mut at = format!("{rel}_at");
        while prog.decls.contains_key(&at) {
            at.push('_');
        }
        // Each rule's document: a document column it binds, or one it leaves unnamed (`tape_line(_, l, sp)`, which
        // then names it), else one through a node it binds.
        let mut made = Vec::new();
        for rule in rules.iter().filter(|r| r.head.rel == *rel) {
            let positive = || rule.body.iter().filter_map(|l| match l {
                Literal::Atom(a) if !a.neg => Some(a),
                _ => None,
            });
            let doc = positive().find_map(|a| match document_column(&a.rel).and_then(|column| a.args.get(column)) {
                Some(Term::Var(v)) => Some(v.clone()),
                _ => None,
            });
            let mut body = rule.body.clone();
            let unnamed = rule.body.iter().enumerate().find_map(|(i, l)| match l {
                Literal::Atom(a) if !a.neg => document_column(&a.rel).filter(|&c| matches!(a.args.get(c), Some(Term::Wild))).map(|c| (i, c)),
                _ => None,
            });
            let doc = match (doc, unnamed) {
                (Some(d), _) => d,
                (None, Some((i, c))) => {
                    let d = "__located_d".to_owned();
                    if let Literal::Atom(a) = &mut body[i] {
                        a.args[c] = Term::Var(d.clone());
                    }
                    d
                }
                (None, None) => {
                    // Through `docof`, or `node` where the program declares it (one that declares neither, as a
                    // program of a few base relations may, is not located).
                    let Some(node) = positive().find_map(|a| a.args.iter().enumerate().find_map(|(c, t)| match t {
                        Term::Var(v) if node_typed(&a.rel, c) => Some(v.clone()),
                        _ => None,
                    })).filter(|_| docof || decls.contains_key("node")) else {
                        break;
                    };
                    let d = "__located_d".to_owned();
                    body.push(Literal::Atom(if docof {
                        Atom { rel: "docof".into(), args: vec![Term::Var(node), Term::Var(d.clone())], neg: false, gated: false }
                    } else {
                        Atom { rel: "node".into(), args: vec![Term::Var(d.clone()), Term::Var(node), Term::Wild, Term::Wild, Term::Wild], neg: false, gated: false }
                    }));
                    d
                }
            };
            let mut head = rule.head.clone();
            head.rel = at.clone();
            head.args.insert(0, Term::Var(doc));
            made.push(Rule { head, body });
        }
        if made.len() != rules.iter().filter(|r| r.head.rel == *rel).count() {
            blocked.push(rel.clone());
            continue;
        }
        let mut at_cols = vec![("d".to_owned(), doc_type.clone())];
        at_cols.extend(cols.iter().cloned());
        prog.decls.insert(at.clone(), at_cols);
        let vars: Vec<Term> = (0..cols.len()).map(|i| Term::Var(format!("__c{i}"))).collect();
        let mut args = vec![Term::Wild];
        args.extend(vars.iter().cloned());
        for rule in rules.iter().filter(|r| r.head.rel == *rel && local.contains(r.head.rel.as_str())) {
            for a in reads(rule) {
                if let Some(n) = readers.get_mut(&a) {
                    *n -= 1;
                }
            }
        }
        rules.retain(|r| r.head.rel != *rel);
        for rule in made.iter().filter(|r| local.contains(r.head.rel.as_str())) {
            for a in reads(rule) {
                *readers.entry(a).or_default() += 1;
            }
        }
        rules.extend(made);
        let read_at = Rule { head: Atom { rel: rel.clone(), args: vars, neg: false, gated: false }, body: vec![Literal::Atom(Atom { rel: at, args, neg: false, gated: false })] };
        for a in reads(&read_at) {
            *readers.entry(a).or_default() += 1;
        }
        rules.push(read_at);
        located.push(rel.clone());
    }
    prog.rules = rules;
    if blocked.is_empty() { Ok(located) } else { Err(blocked) }
}
