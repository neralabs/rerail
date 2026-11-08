//! A corpus run's cross-file rows, memoised per import component (decision 0063).
//!
//! - A program asks with `.pragma "cross-memo" "a b …"`. That names the cross-file relations whose rows are about
//!   the document of their first node column and are a function of that document's import closure.
//! - The run adds `components.rr` (`add`). Its file-local rules export each document's imports, module and own
//!   digest, which the per-file memo keeps. Its cross-file rules make `rr_cx_component(d, c, key)`.
//! - `split` takes those rules out of the cross-file part into a sub-program run first. It checks the contract's
//!   static half, and gives each memoised relation an input `<rel>__cross`, a rule `rel(…) :- rel__cross(…)`, and a
//!   guard `!rr_cross_doc(@doc(first node))` on its other rules.
//! - `lookup` finds each component's stored rows by its key. The hits are replayed; the misses are derived.
//! - `record` stores the misses' rows under `cross-rows/<plan>/<key>`. It checks the contract's other half: each
//!   node of a stored row lies in the closure of its first node's component.
//! - With `RERAIL_CROSS_MEMO=verify` nothing is replayed, and the hits' stored rows are compared with what was
//!   derived.
use super::{Error, Result, front, library};
use crate::content_store::ContentStore;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

pub(super) const PRAGMA: &str = "cross-memo";
pub(super) const COMPONENT: &str = "rr_cx_component";
const EDGE: &str = "rr_cx_edge";
const PREFIX: &str = "rr_cx_";
pub(super) const SUFFIX: &str = "__cross";
pub(super) const DOCS: &str = "rr_cross_doc";
const SCHEMA: &str = "rerail.cross-rows/v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Off,
    On,
    Verify,
}

/// `RERAIL_CROSS_MEMO`: `off`, `verify`, or on.
pub(super) fn mode() -> Mode {
    match std::env::var("RERAIL_CROSS_MEMO").as_deref() {
        Ok("off") => Mode::Off,
        Ok("verify") => Mode::Verify,
        _ => Mode::On,
    }
}

/// The relations the program's `cross-memo` pragmas name; `None` without one.
pub(super) fn declared(prog: &front::Program) -> Option<BTreeSet<String>> {
    let mut out: Option<BTreeSet<String>> = None;
    for (key, text) in &prog.settings {
        if key != "pragma" {
            continue;
        }
        let words: Vec<&str> = text.split('"').map(str::trim).filter(|w| !w.is_empty()).collect();
        if let [PRAGMA, rels] = words.as_slice() {
            out.get_or_insert_default().extend(rels.split_whitespace().map(str::to_owned));
        }
    }
    out
}

/// Adds `components.rr` to `prog`: its declarations, natives and rules, and `rr_cx_component` as an output. Core's
/// relations and rules are the engine's, taken once.
pub(super) fn add(prog: &mut front::Program) -> Result<()> {
    let path = crate::code_rule_dsl::logic::rr::library().join("components.rr");
    let components = crate::code_rule_dsl::logic::rr::parse_file(&path).map_err(|e| Error::Plan(format!("{}: {e:?}", path.display())))?;
    for (rel, cols) in components.decls {
        prog.decls.entry(rel).or_insert(cols);
    }
    for (rel, native) in components.natives {
        prog.natives.entry(rel).or_insert(native);
    }
    for (rel, rows) in components.facts {
        prog.facts.entry(rel).or_insert(rows);
    }
    for rule in components.rules {
        if !prog.rules.contains(&rule) {
            prog.rules.push(rule);
        }
    }
    for input in components.inputs {
        if !prog.inputs.contains(&input) {
            prog.inputs.push(input);
        }
    }
    if !prog.outputs.iter().any(|o| o == COMPONENT) {
        prog.outputs.push(COMPONENT.to_owned());
    }
    Ok(())
}

/// What `split` makes of the cross-file part.
#[derive(Clone)]
pub(super) struct Plan {
    /// The rules that make `rr_cx_component`, run before the rest.
    pub(super) components: front::Program,
    /// The memoised relations: the column of each one's first node, and its width.
    pub(super) memo: BTreeMap<String, (usize, usize)>,
    /// The digest every stored row is a function of besides its component's key.
    pub(super) digest: [u8; 32],
}

/// Takes the component rules out of `cross`, checks the declared relations, and adds their replay inputs, copies
/// and guards. `local` is the file-local program's digest (`memo::program_digest`).
pub(super) fn split(cross: &mut front::Program, declared: &BTreeSet<String>, local: &[u8; 32]) -> Result<Option<Plan>> {
    use front::{Atom, Literal, Rule, Term};
    let heads: BTreeSet<String> = cross.rules.iter().map(|r| r.head.rel.clone()).collect();
    if !heads.contains(COMPONENT) {
        return Ok(None);
    }
    // The component rules: what `rr_cx_component` reads, transitively, among the cross-file rules.
    let mut cone = BTreeSet::from([COMPONENT.to_owned()]);
    loop {
        let before = cone.len();
        let reads: Vec<String> = cross
            .rules
            .iter()
            .filter(|r| cone.contains(&r.head.rel))
            .flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.clone()))
            .filter(|rel| heads.contains(rel))
            .collect();
        cone.extend(reads);
        if cone.len() == before {
            break;
        }
    }
    let mut components = cross.clone();
    components.rules.retain(|r| cone.contains(&r.head.rel));
    components.natives.retain(|rel, _| cone.contains(rel));
    components.outputs = vec![COMPONENT.to_owned()];
    cross.rules.retain(|r| !cone.contains(&r.head.rel));
    cross.natives.retain(|rel, _| !cone.contains(rel));
    cross.outputs.retain(|o| o != COMPONENT);
    // The plan: the engine, the grammars, the file-local part (what the exports are) and the cross-file part.
    let mut hash = Sha256::new();
    hash.update(SCHEMA.as_bytes());
    hash.update(env!("RERAIL_ENGINE_BUILD_ID").as_bytes());
    hash.update(crate::code_rule_dsl::workspace::grammar_version().to_le_bytes());
    hash.update(local);
    hash.update(format!("{:?}{:?}{:?}{:?}{:?}", cross.decls, cross.rules, cross.facts, cross.natives, cross.lattice).as_bytes());
    let digest: [u8; 32] = hash.finalize().into();
    // The contract's static half: a memoised relation has a node column, and its rules read only what is not derived
    // across files, the components' facts, and other memoised relations.
    let derived: BTreeSet<String> = cross.rules.iter().map(|r| r.head.rel.clone()).collect();
    let mut memo = BTreeMap::new();
    for rel in declared.iter().filter(|r| derived.contains(*r)) {
        let cols = &cross.decls[rel];
        let first = cols.iter().position(|(_, t)| t == "node").ok_or_else(|| Error::Plan(format!("{rel}: a `{PRAGMA}` relation needs a node column, the document its rows are about")))?;
        memo.insert(rel.clone(), (first, cols.len()));
    }
    for r in cross.rules.iter().filter(|r| memo.contains_key(&r.head.rel)) {
        for a in r.body.iter().filter_map(|l| l.atom()) {
            if derived.contains(&a.rel) && !memo.contains_key(&a.rel) && !a.rel.starts_with(PREFIX) {
                return Err(Error::Plan(format!(
                    "{}: a `{PRAGMA}` relation reads {}, a cross-file relation the pragma does not name, whose rows may depend on files outside the import closure",
                    r.head.rel, a.rel
                )));
            }
        }
    }
    if memo.is_empty() {
        return Ok(Some(Plan { components, memo, digest }));
    }
    // Replay: an input and a copy per memoised relation; its other rules derive no row about a replayed document.
    cross.decls.insert(DOCS.to_owned(), vec![("d".to_owned(), "int".to_owned())]);
    cross.inputs.push(DOCS.to_owned());
    for rel in memo.keys() {
        let cols = cross.decls[rel].clone();
        let input = format!("{rel}{SUFFIX}");
        let vars: Vec<Term> = (0..cols.len()).map(|i| Term::Var(format!("rr_x{i}"))).collect();
        cross.decls.insert(input.clone(), cols);
        cross.inputs.push(input.clone());
        cross.rules.push(Rule {
            head: Atom { rel: rel.clone(), args: vars.clone(), neg: false, gated: false },
            body: vec![Literal::Atom(Atom { rel: input, args: vars, neg: false, gated: false })],
        });
    }
    for rule in &mut cross.rules {
        let Some(&(first, _)) = memo.get(&rule.head.rel) else { continue };
        let copy = rule.body.iter().any(|l| matches!(l, Literal::Atom(a) if a.rel.ends_with(SUFFIX) || a.rel.ends_with(library::SUFFIX)));
        if copy {
            continue;
        }
        if let Some(Term::Var(v)) = rule.head.args.get(first) {
            let d = "rr_xd".to_owned();
            rule.body.push(Literal::Cmp { l: Term::Var(d.clone()), op: "=", r: Term::Call("doc".to_owned(), vec![Term::Var(v.clone())]) });
            rule.body.push(Literal::Atom(Atom { rel: DOCS.to_owned(), args: vec![Term::Var(d)], neg: true, gated: false }));
        }
    }
    Ok(Some(Plan { components, memo, digest }))
}

/// Each document's component and key, and the components' edges, from the component sub-program's rows.
pub(super) struct Components {
    pub(super) of: HashMap<i64, (i64, String)>,
    out: HashMap<i64, BTreeSet<i64>>,
}

impl Components {
    pub(super) fn new(rows: Vec<(i64, i64, String)>, edges: &[(i64, i64)]) -> Self {
        let of: HashMap<i64, (i64, String)> = rows.into_iter().map(|(d, c, k)| (d, (c, k))).collect();
        let mut out: HashMap<i64, BTreeSet<i64>> = HashMap::new();
        for (a, b) in edges {
            if let (Some((ca, _)), Some((cb, _))) = (of.get(a), of.get(b))
                && ca != cb
            {
                out.entry(*ca).or_default().insert(*cb);
            }
        }
        Self { of, out }
    }

    /// The components and their members.
    fn members(&self) -> BTreeMap<i64, (String, Vec<i64>)> {
        let mut m: BTreeMap<i64, (String, Vec<i64>)> = BTreeMap::new();
        for (d, (c, k)) in &self.of {
            m.entry(*c).or_insert_with(|| (k.clone(), Vec::new())).1.push(*d);
        }
        for (_, docs) in m.values_mut() {
            docs.sort_unstable();
        }
        m
    }

    /// The components `c` reaches, itself included.
    fn closure(&self, c: i64, memo: &mut HashMap<i64, HashSet<i64>>) -> HashSet<i64> {
        if let Some(r) = memo.get(&c) {
            return r.clone();
        }
        let mut seen = HashSet::from([c]);
        let mut todo = vec![c];
        while let Some(x) = todo.pop() {
            for y in self.out.get(&x).into_iter().flatten() {
                if seen.insert(*y) {
                    todo.push(*y);
                }
            }
        }
        memo.insert(c, seen.clone());
        seen
    }
}

fn name(plan: &[u8; 32], key: &str) -> String {
    format!("cross-rows/{}/{key}", plan.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// What `lookup` found.
#[derive(Default)]
pub(super) struct Found {
    /// The components found, by component, with their stored rows.
    pub(super) hits: BTreeMap<i64, library::Rows>,
    /// The components that missed.
    pub(super) misses: BTreeSet<i64>,
    /// The hits' stored sources, which the run's manifest names so collection keeps them (decision 0053).
    pub(super) sources: Vec<crate::content_store::SourceId>,
}

impl Found {
    /// The inputs to bind: `rr_cross_doc` (the hits' documents) and each memoised relation's replayed rows. Empty
    /// when verifying, so everything is derived.
    pub(super) fn inputs(&self, plan: &Plan, components: &Components, mode: Mode) -> Vec<(String, usize, Vec<i64>)> {
        let replay = mode == Mode::On;
        let docs: Vec<i64> = if replay { components.of.iter().filter(|(_, (c, _))| self.hits.contains_key(c)).map(|(d, _)| *d).collect() } else { Vec::new() };
        let mut inputs = vec![(DOCS.to_owned(), 1, docs)];
        for (rel, &(_, width)) in &plan.memo {
            let mut values = Vec::new();
            if replay {
                for rows in self.hits.values() {
                    if let Some((w, v)) = rows.rels.get(rel)
                        && *w == width
                    {
                        values.extend_from_slice(v);
                    }
                }
            }
            inputs.push((format!("{rel}{SUFFIX}"), width, values));
        }
        inputs
    }
}

/// Each component's stored rows under `plan`, if its documents' keys are the ones they were stored under.
pub(super) fn lookup(store: &ContentStore, plan: &Plan, components: &Components, paths: &HashMap<i64, String>) -> Found {
    let mut found = Found::default();
    for (c, (key, docs)) in components.members() {
        let stored = library::lookup_source(store, &name(&plan.digest, &key)).and_then(|(id, b)| library::Rows::decode(&b).map(|r| (id, r)));
        let keys: BTreeMap<String, i64> = docs.iter().filter_map(|d| paths.get(d).map(|p| (p.clone(), *d))).collect();
        match stored {
            Some((id, rows)) if rows.keys == keys => {
                found.sources.push(id);
                found.hits.insert(c, rows);
            }
            _ => {
                found.misses.insert(c);
            }
        }
    }
    found
}

/// What `record` did.
#[derive(Default)]
pub(super) struct Recorded {
    /// The stored sources, for the run's manifest.
    pub(super) sources: Vec<crate::content_store::SourceId>,
    pub(super) stored: usize,
    pub(super) rows: usize,
    pub(super) verified: usize,
}

/// Stores the misses' rows of the memoised relations (`rows`: each relation's derived rows), checking that each
/// row's nodes lie in the closure of its first node's component. When verifying, compares the hits' stored rows with
/// what was derived instead of trusting them.
#[allow(clippy::too_many_arguments)]
pub(super) fn record(
    store: &ContentStore,
    plan: &Plan,
    components: &Components,
    found: &Found,
    mode: Mode,
    rows: &BTreeMap<String, Vec<Vec<i64>>>,
    node_columns: &BTreeMap<String, Vec<usize>>,
    paths: &HashMap<i64, String>,
) -> Result<Recorded> {
    let doc_of = |v: i64| if v >= 1 << 32 { v >> 32 } else { v };
    let mut closures = HashMap::new();
    let mut by: BTreeMap<i64, BTreeMap<String, Vec<i64>>> = BTreeMap::new();
    for (rel, (first, _)) in &plan.memo {
        let nodes = node_columns.get(rel).cloned().unwrap_or_default();
        let mut sorted: Vec<&Vec<i64>> = rows.get(rel).into_iter().flatten().collect();
        sorted.sort();
        sorted.dedup();
        for row in sorted {
            let Some((c, _)) = components.of.get(&doc_of(row[*first])) else { continue };
            let reach = components.closure(*c, &mut closures);
            for &n in &nodes {
                if let Some((e, _)) = components.of.get(&doc_of(row[n]))
                    && !reach.contains(e)
                {
                    return Err(Error::Plan(format!(
                        "{rel}: a row about {} names a node of {}, outside its import closure, so the relation's rows are not a function of the closure (decision 0063): remove it from `.pragma \"{PRAGMA}\"`",
                        paths.get(&doc_of(row[*first])).map_or("?", String::as_str),
                        paths.get(&doc_of(row[n])).map_or("?", String::as_str)
                    )));
                }
            }
            by.entry(*c).or_default().entry(rel.clone()).or_default().extend_from_slice(row);
        }
    }
    let members = components.members();
    let mut recorded = Recorded::default();
    for (c, (key, docs)) in &members {
        let derived = by.remove(c).unwrap_or_default();
        let mut stored = library::Rows { keys: docs.iter().filter_map(|d| paths.get(d).map(|p| (p.clone(), *d))).collect(), rels: BTreeMap::new() };
        for (rel, &(_, width)) in &plan.memo {
            stored.rels.insert(rel.clone(), (width, derived.get(rel).cloned().unwrap_or_default()));
        }
        if let Some(hit) = found.hits.get(c) {
            if mode == Mode::Verify {
                for (rel, (width, values)) in &stored.rels {
                    let had = hit.rels.get(rel).map(|(_, v)| v.clone()).unwrap_or_default();
                    let set = |v: &[i64]| v.chunks(*width).map(<[i64]>::to_vec).collect::<BTreeSet<_>>();
                    if set(&had) != set(values) {
                        return Err(Error::Plan(format!(
                            "RERAIL_CROSS_MEMO=verify: {rel}'s stored rows about the component of {} differ from those derived ({} stored, {} derived)",
                            docs.first().and_then(|d| paths.get(d)).map_or("?", String::as_str),
                            set(&had).len(),
                            set(values).len()
                        )));
                    }
                }
                recorded.verified += 1;
            }
            continue;
        }
        recorded.rows += stored.rels.values().map(|(w, v)| v.len() / w).sum::<usize>();
        recorded.sources.push(library::record_named(store, &name(&plan.digest, key), &plan.digest, &stored.encode())?);
        recorded.stored += 1;
    }
    Ok(recorded)
}

/// The component sub-program's relations a run holds: the components and the edges.
pub(super) const HELD: [&str; 2] = [COMPONENT, EDGE];
