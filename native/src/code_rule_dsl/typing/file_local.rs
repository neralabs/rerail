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

/// No node: a root's parent, or a node with no ancestor in the set.
pub(crate) const NONE: u32 = u32::MAX;
/// The flat lattice's absent and top values (`logic::native::BOT`, `TOP`).
const BOT: i64 = i64::MIN;
const TOP: i64 = 1 << 62;

/// Where the file-local FIX runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Binding {
    Cpu,
    /// The device work queue; the CPU where there is no device.
    Metal,
}

impl Binding {
    pub(crate) fn of(backend: crate::code_rule_dsl::transducer::packed::ExecutionBackend) -> Self {
        use crate::code_rule_dsl::transducer::packed::ExecutionBackend as B;
        match backend {
            B::MetalInterpreter | B::MetalJit => Binding::Metal,
            _ => Binding::Cpu,
        }
    }
}

/// A chunk's trees: per node (a cell of the chunk), its parent's cell or `NONE`, and its kind atom.
pub(crate) struct Trees<'a> {
    pub(crate) parent: &'a [u32],
    pub(crate) kind: &'a [i64],
}

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
    #[cfg(target_os = "macos")]
    if binding == Binding::Metal {
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
        use crate::code_rule_dsl::transducer::packed::{Error, metal_nearest};
        // `RERAIL_NEAREST_BUFFERS=fresh`: buffers made per piece, to measure the pre-pinned blocks against.
        let how = if std::env::var("RERAIL_NEAREST_BUFFERS").is_ok_and(|v| v == "fresh") { metal_nearest::Buffers::Fresh } else { metal_nearest::Buffers::Blocks };
        match metal_nearest::solve_with(n, &target, &done, how) {
            Ok(solved) => {
                let kernel = "nearest ancestors (pointer jumping)";
                profile::device(kernel, solved.launches as u64, solved.gpu, solved.wall, target.len() as u64);
                profile::device_phases(kernel, solved.phases, solved.made, solved.reused);
                crate::code_rule_dsl::prof::count("nearest.device_steps", solved.rounds as usize);
                return Ok(solved.targets.chunks(n.max(1)).map(<[u32]>::to_vec).collect());
            }
            Err(Error::UnsupportedBackend) => {
                profile::decision("fallback", || "nearest ancestors: the Metal binding found no device and solved on the CPU".into());
            }
            Err(error) => return Err(StoreError::Invalid(format!("nearest on the device: {error:?}"))),
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

/// A base relation's document column, including raw text views.
pub(crate) fn document_column(relation: &str) -> Option<usize> {
    match relation {
        "node" | "doc" | "pkg" | "path_part" | "file_name" | "syntax_error" |
        "source" | "src_line" | "split_line" | "lf_line" | "line_count" | "parsed"
        // Decision 0055's: a guest and its host are one file (`corpus::guests` attributes its rows to the host).
        | "doc_role" | "doc_root" | "tape_of" | "guest" | "host_node" => Some(0),
        "docof" => Some(1),
        _ => None,
    }
}

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
    use crate::code_rule_dsl::logic::front::{Literal, Term};
    use std::collections::{BTreeMap, BTreeSet};
    let derived: BTreeSet<&str> = prog.rules.iter().map(|r| r.head.rel.as_str()).collect();
    // A base relation's location columns: node columns, and the document column of `node`, `doc`, `pkg` and
    // `syntax_error`.
    let base_locations = |rel: &str| -> Vec<usize> {
        let mut columns: Vec<usize> = prog.decls.get(rel).map_or_else(Vec::new, |cols| cols.iter().enumerate().filter(|(_, (_, t))| t == "node").map(|(i, _)| i).collect());
        if let Some(column) = document_column(rel) {
            columns.push(column);
        }
        columns
    };
    // A derived relation's location columns: those every rule fills from a location.
    let mut locations: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let location_vars = |rule: &crate::code_rule_dsl::logic::front::Rule, locations: &BTreeMap<String, Vec<usize>>| -> BTreeSet<String> {
        let mut out = BTreeSet::new();
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
        out
    };
    // Location columns to a fixpoint (a column is one if every rule's head fills it from a location variable).
    let mut changed = true;
    for rel in &derived {
        let width = prog.decls.get(*rel).map_or(0, Vec::len);
        locations.insert((*rel).to_owned(), (0..width).collect());
    }
    while changed {
        changed = false;
        for rel in &derived {
            let rules: Vec<_> = prog.rules.iter().filter(|r| r.head.rel == *rel).collect();
            let kept: Vec<usize> = locations[*rel]
                .iter()
                .copied()
                .filter(|&c| rules.iter().all(|r| matches!(r.head.args.get(c), Some(Term::Var(v)) if location_vars(r, &locations).contains(v))))
                .collect();
            if kept != locations[*rel] {
                locations.insert((*rel).to_owned(), kept);
                changed = true;
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
    loop {
        let kept: BTreeSet<String> = filters
            .iter()
            .filter(|rel| {
                prog.rules.iter().filter(|r| &r.head.rel == *rel).all(|r| {
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
            })
            .cloned()
            .collect();
        if kept == filters {
            break;
        }
        filters = kept;
    }
    // A relation made natively (`dfs_first`) reads the whole program's rows: never file-local.
    let mut local: BTreeSet<String> = derived.iter().filter(|r| !prog.natives.contains_key(**r)).map(|r| (*r).to_owned()).collect();
    loop {
        let rule_local = |rule: &crate::code_rule_dsl::logic::front::Rule| -> bool {
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
                    Literal::Ast { .. } => return false,
                    Literal::Nearest { n, a, .. } => {
                        groups.push(BTreeSet::from([n.clone(), a.clone()]));
                        continue;
                    }
                    Literal::Cmp { .. } => continue,
                };
                let constant = !derived.contains(atom.rel.as_str()) && prog.facts.contains_key(&atom.rel) && !prog.inputs.contains(&atom.rel);
                // A tape operation's rows (decision 0058 §4) are made in the chunk that asked: they place nothing.
                let chunk_value = !derived.contains(atom.rel.as_str()) && CHUNK_VALUES.contains(&atom.rel.as_str());
                if constant || chunk_value || test_relation(&atom.rel) || filters.contains(&atom.rel) || filters.contains(&rule.head.rel) {
                    continue;
                }
                if derived.contains(atom.rel.as_str()) && !local.contains(&atom.rel) {
                    return false;
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
                    return false;
                }
                groups.push(vars);
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
            merged.len() == 1 || (merged.is_empty() && filters.contains(&rule.head.rel))
        };
        let before = local.len();
        let failing: Vec<String> = prog.rules.iter().filter(|r| local.contains(&r.head.rel) && !rule_local(r)).map(|r| r.head.rel.clone()).collect();
        for rel in failing {
            local.remove(&rel);
        }
        if local.len() == before {
            let mut same: BTreeMap<String, Vec<usize>> =
                prog.inputs.iter().filter(|i| !derived.contains(i.as_str())).map(|i| (i.clone(), base_locations(i))).collect();
            same.extend(local.iter().map(|rel| (rel.clone(), locations[rel].clone())));
            return (local, locations, same);
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
    let (local, locations, same_doc) = split_located(prog);
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
    let cross_reads = read(&mut prog.rules.iter().filter(|r| in_cross(&r.head.rel)));
    let exports: Vec<String> = cross_reads.iter().filter(|r| is_local(r) && !projected.contains(*r)).cloned().collect();
    let base_read: Vec<String> = prog.inputs.iter().filter(|i| cross_reads.contains(*i)).cloned().collect();
    let mut base = base_read.clone();
    for decode in ["node", "doc"] {
        if prog.inputs.iter().any(|i| i == decode) && !base.iter().any(|b| b == decode) {
            base.push(decode.to_owned());
        }
    }
    let mut l = prog.clone();
    l.rules.retain(|r| is_local(&r.head.rel));
    l.lattice.retain(|rel, _| is_local(rel));
    l.gates.retain(|(rel, _)| is_local(rel));
    l.outputs = prog.outputs.iter().filter(|o| is_local(o)).cloned().chain(exports.iter().cloned()).collect::<BTreeSet<_>>().into_iter().collect();
    let mut c = prog.clone();
    c.rules.retain(|r| in_cross(&r.head.rel));
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
/// A component that is one atom of only variables filters nothing and is left; so is one that reads no base
/// relation. A made relation the split does not find file-local is undone. Returns the relations made.
pub(crate) fn push_local_joins(prog: &mut crate::code_rule_dsl::logic::front::Program) -> Vec<String> {
    use crate::code_rule_dsl::logic::front::{Atom, Literal, Rule, Term};
    use std::collections::{BTreeMap, BTreeSet};
    // An atom such as field(parent, name, child) connects its node columns. Document equality
    // and comparisons of node attributes do not: enumerating their independent node groups
    // before the remaining candidate predicates creates quadratic or cubic intermediate relations.
    let independent_node_groups = |atoms: &[&Atom]| {
        let mut groups: Vec<BTreeSet<String>> = Vec::new();
        for atom in atoms {
            let Some(columns) = prog.decls.get(&atom.rel) else { continue };
            let nodes: BTreeSet<String> = atom.args.iter().zip(columns).filter_map(|(term, (_, ty))| {
                if matches!(ty.as_str(), "node" | "loc" | "span") && let Term::Var(value) = term { Some(value.clone()) } else { None }
            }).collect();
            if nodes.is_empty() { continue; }
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
    loop {
        let local = split(prog);
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
            // Components of the candidates by shared variables.
            let idx: Vec<usize> = (0..rule.body.len()).filter(|&j| candidate(&rule.body[j])).collect();
            let mut comps: Vec<(BTreeSet<usize>, BTreeSet<String>)> = Vec::new();
            for &j in &idx {
                let vars = rule.body[j].vars();
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
                if !atoms.iter().any(|a| base(&a.rel)) {
                    continue;
                }
                if independent_node_groups(&atoms) > 1 {
                    continue;
                }
                if let [a] = atoms[..] {
                    let named: Vec<&String> = a.args.iter().filter_map(|t| if let Term::Var(v) = t { Some(v) } else { None }).collect();
                    let distinct: BTreeSet<&String> = named.iter().copied().collect();
                    if a.args.iter().all(|t| matches!(t, Term::Var(_) | Term::Wild)) && distinct.len() == named.len() {
                        continue;
                    }
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
            return Vec::new();
        }
        next.rules.extend(helpers);
        let now = split(&next);
        let failed: Vec<usize> = made.iter().filter(|(_, n)| !now.contains(n)).map(|(i, _)| *i).collect();
        if failed.is_empty() {
            *prog = next;
            return made.into_iter().map(|(_, n)| n).collect();
        }
        for i in failed {
            *level.entry(i).or_default() += 1;
        }
    }
}

/// Gives every relation the chunks hand on (an export, or a file-local output) a location column, so each of its
/// rows names its document (decision 0052): a relation without one, `r(p, l)`, is derived by the chunks as
/// `r_at(d, p, l)`, `d` a document its rules bind (from `node`, `doc`, …, or a node through `node(d, n, …)`), and
/// the cross-file part reads `r(p, l) :- r_at(_, p, l)`. A relation that file-local rules also read, a lattice, or
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
    let doc_type = prog.decls.get("node").and_then(|cols| cols.first()).map(|(_, t)| t.clone()).unwrap_or_else(|| "number".to_owned());
    let (mut located, mut blocked) = (Vec::new(), Vec::new());
    let mut rules = std::mem::take(&mut prog.rules);
    for rel in &targets {
        let read_locally = rules.iter().any(|r| local.contains(r.head.rel.as_str()) && r.head.rel != *rel && r.body.iter().any(|l| l.atom().is_some_and(|a| a.rel == *rel)));
        let Some(cols) = prog.decls.get(rel).cloned().filter(|_| !read_locally && !prog.lattice.contains_key(rel)) else {
            blocked.push(rel.clone());
            continue;
        };
        // Each rule's document: a document column it binds, else one through a node it binds.
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
            let doc = match doc {
                Some(d) => d,
                None => {
                    let Some(node) = positive().find_map(|a| a.args.iter().enumerate().find_map(|(c, t)| match t {
                        Term::Var(v) if node_typed(&a.rel, c) => Some(v.clone()),
                        _ => None,
                    })) else {
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
            head.rel = format!("{rel}_at");
            head.args.insert(0, Term::Var(doc));
            made.push(Rule { head, body });
        }
        if made.len() != rules.iter().filter(|r| r.head.rel == *rel).count() {
            blocked.push(rel.clone());
            continue;
        }
        let at = format!("{rel}_at");
        let mut at_cols = vec![("d".to_owned(), doc_type.clone())];
        at_cols.extend(cols.iter().cloned());
        prog.decls.insert(at.clone(), at_cols);
        let vars: Vec<Term> = (0..cols.len()).map(|i| Term::Var(format!("__c{i}"))).collect();
        let mut args = vec![Term::Wild];
        args.extend(vars.iter().cloned());
        rules.retain(|r| r.head.rel != *rel);
        rules.extend(made);
        rules.push(Rule { head: Atom { rel: rel.clone(), args: vars, neg: false, gated: false }, body: vec![Literal::Atom(Atom { rel: at, args, neg: false, gated: false })] });
        located.push(rel.clone());
    }
    prog.rules = rules;
    if blocked.is_empty() { Ok(located) } else { Err(blocked) }
}
