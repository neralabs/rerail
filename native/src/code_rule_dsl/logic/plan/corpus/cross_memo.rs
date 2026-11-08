//! A corpus run's cross-file rows, memoised per import component (decision 0076).
//!
//! - A program asks with `.pragma "cross-memo" "a b …"`. That names the cross-file relations whose rows are about
//!   the document of their first node column and are a function of that document's import closure; `"*"` names
//!   every relation the contract's static half admits.
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
use super::{Error, Result, front};
use crate::content_store::{ContentStore, HashKey};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

pub(super) const PRAGMA: &str = "cross-memo";
pub(super) const COMPONENT: &str = "rr_cx_component";
const EDGE: &str = "rr_cx_edge";
const PREFIX: &str = "rr_cx_";
pub(super) const SUFFIX: &str = "__cross";
pub(super) const DOCS: &str = "rr_cross_doc";
const SCHEMA: &str = "rerail.cross-rows/v1";
/// The stored rows' encoding version: bumped when its meaning changes.
const ROWS_SCHEMA: u32 = 1;

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
    // Only the engine's own rules (`rr_` heads: `rr_cx_*` and what lowering made for `components`), not what
    // `core.rr` brings with it: a program that declares a base relation itself keeps its own declaration.
    let rules: Vec<front::Rule> = components.rules.into_iter().filter(|r| r.head.rel.starts_with("rr_")).collect();
    let mut needed: BTreeSet<String> = BTreeSet::new();
    for r in &rules {
        needed.insert(r.head.rel.clone());
        needed.extend(r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.clone()));
    }
    let natives: Vec<_> = components.natives.into_iter().filter(|(rel, _)| needed.contains(rel)).collect();
    for (rel, native) in natives {
        needed.extend(native.reads().into_iter().map(str::to_owned));
        prog.natives.entry(rel).or_insert(native);
    }
    // A relation the program declares stays as the program has it: its own columns, and an input or a rule as its
    // lowering made it (`docof` over nodes is a rule over `node`, not an input). Only what it lacks is added.
    let missing: BTreeSet<String> = needed.iter().filter(|rel| !prog.decls.contains_key(*rel)).cloned().collect();
    for (rel, cols) in components.decls.into_iter().filter(|(rel, _)| missing.contains(rel)) {
        prog.decls.insert(rel, cols);
    }
    for (rel, rows) in components.facts.into_iter().filter(|(rel, _)| missing.contains(rel)) {
        prog.facts.entry(rel).or_insert(rows);
    }
    for input in components.inputs.into_iter().filter(|rel| missing.contains(rel)) {
        if !prog.inputs.contains(&input) {
            prog.inputs.push(input);
        }
    }
    for rule in rules {
        if !prog.rules.contains(&rule) {
            prog.rules.push(rule);
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
/// The cross-file relations a memoised relation may read as if they were base facts:
/// - projections of an input that keep a node column (`kind`, `docof` over nodes): every rule copies one row of a
///   base relation or an export, so each row is about that row's node;
/// - the run's constants: relations whose rules read only the program's own facts (settings, `world`'s answers) and
///   other constants, such as `py_version`. The facts are in the plan's digest, so a row that reads them is still a
///   function of the closure under that plan.
fn free(cross: &front::Program, derived: &BTreeSet<String>) -> BTreeSet<String> {
    use front::Literal;
    fn rules_of<'a>(cross: &'a front::Program, rel: &'a str) -> impl Iterator<Item = &'a front::Rule> + 'a {
        cross.rules.iter().filter(move |r| r.head.rel == rel)
    }
    let input = |rel: &str| cross.inputs.iter().any(|i| i == rel);
    let mut free: BTreeSet<String> = derived
        .iter()
        .filter(|rel| !cross.natives.contains_key(*rel))
        // A head that drops the node (`isqmod(q) :- modname(_, q)`) is a set over every document, not a view.
        .filter(|rel| cross.decls.get(*rel).is_some_and(|c| c.iter().any(|(_, t)| t == "node")))
        .filter(|rel| rules_of(cross, rel).all(|r| matches!(r.body.as_slice(), [Literal::Atom(a)] if !a.neg && input(&a.rel) && !a.rel.ends_with("__cross"))))
        .cloned()
        .collect();
    // Declared, made by no rule and bound from no chunk: the program's own rows, if any.
    let fact = |rel: &str| cross.decls.contains_key(rel) && !derived.contains(rel) && !input(rel);
    let mut constants: BTreeSet<String> = BTreeSet::new();
    loop {
        let more: Vec<String> = derived
            .iter()
            .filter(|r| !constants.contains(*r) && !cross.natives.contains_key(*r))
            .filter(|rel| rules_of(cross, rel).all(|r| r.body.iter().filter_map(|l| l.atom()).all(|a| fact(&a.rel) || constants.contains(&a.rel))))
            .cloned()
            .collect();
        if more.is_empty() {
            break;
        }
        constants.extend(more);
    }
    free.extend(constants);
    free
}

/// The largest set of cross-file relations that a `cross-memo` pragma could name: each has a node column and is
/// made by rules that read, of what is derived across files, only the components' facts and the set itself.
fn closed(cross: &front::Program, derived: &BTreeSet<String>) -> BTreeSet<String> {
    let mut set: BTreeSet<String> = derived
        .iter()
        .filter(|r| !r.starts_with(PREFIX) && !cross.natives.contains_key(*r) && cross.decls.get(*r).is_some_and(|c| c.iter().any(|(_, t)| t == "node")))
        .cloned()
        .collect();
    loop {
        let out: Vec<String> = cross
            .rules
            .iter()
            .filter(|r| set.contains(&r.head.rel))
            .filter(|r| r.body.iter().filter_map(|l| l.atom()).any(|a| derived.contains(&a.rel) && !set.contains(&a.rel) && !a.rel.starts_with(PREFIX)))
            .map(|r| r.head.rel.clone())
            .collect();
        if out.is_empty() {
            return set;
        }
        for r in out {
            set.remove(&r);
        }
    }
}

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
    // What the rest reads of the cone (`rr_cx_reach` reads the edges) stays in the cross-file part too, derived again
    // there: a few thousand rows over the documents, not nodes.
    let mut shared: BTreeSet<String> = BTreeSet::new();
    let mut todo: Vec<String> = cross
        .rules
        .iter()
        .filter(|r| !cone.contains(&r.head.rel))
        .flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.clone()))
        .filter(|rel| cone.contains(rel))
        .collect();
    while let Some(rel) = todo.pop() {
        if shared.insert(rel.clone()) {
            todo.extend(cross.rules.iter().filter(|r| r.head.rel == rel).flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.clone())).filter(|x| cone.contains(x)));
        }
    }
    let mut components = cross.clone();
    components.rules.retain(|r| cone.contains(&r.head.rel));
    components.natives.retain(|rel, _| cone.contains(rel));
    components.outputs = vec![COMPONENT.to_owned()];
    cross.rules.retain(|r| !cone.contains(&r.head.rel) || shared.contains(&r.head.rel));
    cross.natives.retain(|rel, _| !cone.contains(rel) || shared.contains(rel));
    cross.outputs.retain(|o| o != COMPONENT);
    // The plan: the engine, the grammars, the file-local part (what the exports are) and the cross-file part.
    let mut hash = Sha256::new();
    hash.update(SCHEMA.as_bytes());
    hash.update(env!("RERAIL_ENGINE_BUILD_ID").as_bytes());
    hash.update(crate::code_rule_dsl::workspace::grammar_version().to_le_bytes());
    hash.update(local);
    // A relation with no facts is left out: a program prepared afresh has `dir_entry: []` where the one read back from
    // the store (decision 0073) has nothing, and the two must store under one plan.
    let facts: Vec<_> = cross.facts.iter().filter(|(_, rows)| !rows.is_empty()).collect();
    hash.update(format!("{:?}{:?}{:?}{:?}{:?}", cross.decls, cross.rules, facts, cross.natives, cross.lattice).as_bytes());
    let digest: [u8; 32] = hash.finalize().into();
    // The contract's static half: a memoised relation has a node column, and its rules read only what is not derived
    // across files, the components' facts, and other memoised relations.
    // A native relation of the cross-file part is derived across files too, and is not replayed: it has no rules to
    // copy into or guard.
    // What a memoised rule may read freely is not counted: the run's constants and the views of `node` (`free`).
    let all: BTreeSet<String> = cross.rules.iter().map(|r| r.head.rel.clone()).chain(cross.natives.keys().cloned()).collect();
    let free = free(cross, &all);
    let derived: BTreeSet<String> = all.difference(&free).cloned().collect();
    let closed = closed(cross, &derived);
    crate::code_rule_dsl::logic::profile::decision("cross-file memo: the static contract", || {
        // What keeps the others out: cross-file relations that cannot be declared (no node column, or native), with
        // how many rules of node-column relations read each.
        let mut blockers: BTreeMap<&str, usize> = BTreeMap::new();
        for r in cross.rules.iter().filter(|r| !closed.contains(&r.head.rel) && cross.decls.get(&r.head.rel).is_some_and(|c| c.iter().any(|(_, t)| t == "node"))) {
            for a in r.body.iter().filter_map(|l| l.atom()) {
                let declarable = cross.decls.get(&a.rel).is_some_and(|c| c.iter().any(|(_, t)| t == "node")) && !cross.natives.contains_key(&a.rel);
                if derived.contains(&a.rel) && !declarable && !a.rel.starts_with(PREFIX) {
                    *blockers.entry(a.rel.as_str()).or_default() += 1;
                }
            }
        }
        let mut blockers: Vec<_> = blockers.into_iter().collect();
        blockers.sort_by(|a, b| b.1.cmp(&a.1));
        format!(
            "{} of {} cross-file relations could be declared (a node column, reading only each other): {}; kept out by {}",
            closed.len(),
            derived.len(),
            closed.iter().cloned().collect::<Vec<_>>().join(" "),
            blockers.iter().map(|(r, n)| format!("{r}{}{n}", if cross.natives.contains_key(*r) { " (native) ×" } else { " ×" })).collect::<Vec<_>>().join(", ")
        )
    });
    // `*`: every relation the static half admits.
    let declared: &BTreeSet<String> = if declared.contains("*") { &closed } else { declared };
    let mut memo = BTreeMap::new();
    for rel in declared.iter().filter(|r| derived.contains(*r)) {
        if cross.natives.contains_key(rel) {
            return Err(Error::Plan(format!("{rel}: a native relation cannot be declared `{PRAGMA}`: it is made whole, not by rules")));
        }
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
        let copy = rule.body.iter().any(|l| matches!(l, Literal::Atom(a) if a.rel.ends_with(SUFFIX)));
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

/// Drops the memoised relations' rules but their copies from `<rel>__cross`: for a run where every component was
/// found, whose guards (`!rr_cross_doc`) every other rule's rows fail.
/// What only those rules read is dropped too (`rr_cx_reach`); `kept` names what the run reads besides its outputs.
pub(super) fn keep_copies(cross: &mut front::Program, plan: &Plan, kept: &BTreeSet<String>) {
    cross.rules.retain(|r| !plan.memo.contains_key(&r.head.rel) || r.body.iter().any(|l| matches!(l, front::Literal::Atom(a) if a.rel.ends_with(SUFFIX))));
    loop {
        let read: BTreeSet<&str> = cross.rules.iter().flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.as_str())).collect();
        let unread = |rel: &str| !read.contains(rel) && !kept.contains(rel) && !cross.outputs.iter().any(|o| o == rel);
        let dead: BTreeSet<String> = cross.rules.iter().map(|r| r.head.rel.clone()).chain(cross.natives.keys().cloned()).filter(|r| unread(r)).collect();
        if dead.is_empty() {
            return;
        }
        cross.rules.retain(|r| !dead.contains(&r.head.rel));
        cross.natives.retain(|rel, _| !dead.contains(rel));
    }
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
    pub(super) hits: BTreeMap<i64, Rows>,
    /// The components that missed.
    pub(super) misses: BTreeSet<i64>,
    /// The hits' stored sources, which the run's manifest names so collection keeps them (decision 0053).
    pub(super) sources: Vec<crate::content_store::SourceId>,
}

impl Found {
    /// The inputs to bind: `rr_cross_doc` (the hits' documents) and each memoised relation's replayed rows. Empty
    /// when verifying, so everything is derived.
    pub(super) fn inputs(&self, plan: &Plan, components: &Components, mode: Mode) -> Vec<(String, usize, Vec<i64>)> {
        // Nothing memoised: `split` added no replay inputs to bind.
        if plan.memo.is_empty() {
            return Vec::new();
        }
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
        let stored = lookup_source(store, &name(&plan.digest, &key)).and_then(|(id, b)| Rows::decode(&b).map(|r| (id, r)));
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
            // A found component's rows are its replayed copies: nothing to check or store unless verifying.
            if mode == Mode::On && found.hits.contains_key(c) {
                continue;
            }
            let reach = components.closure(*c, &mut closures);
            for &n in &nodes {
                if let Some((e, _)) = components.of.get(&doc_of(row[n]))
                    && !reach.contains(e)
                {
                    return Err(Error::Plan(format!(
                        "{rel}: a row about {} names a node of {}, outside its import closure, so the relation's rows are not a function of the closure (decision 0076): remove it from `.pragma \"{PRAGMA}\"`",
                        paths.get(&doc_of(row[*first])).map_or("?", String::as_str),
                        // A guest has no path: named by its key and its component's first file.
                        paths.get(&doc_of(row[n])).cloned().unwrap_or_else(|| format!(
                            "document {} (component of {})",
                            doc_of(row[n]),
                            components.members().get(e).and_then(|(_, ds)| ds.iter().find_map(|d| paths.get(d))).map_or("no file", String::as_str)
                        ))
                    )));
                }
            }
            by.entry(*c).or_default().entry(rel.clone()).or_default().extend_from_slice(row);
        }
    }
    let members = components.members();
    let mut recorded = Recorded::default();
    let mut differ: Vec<String> = Vec::new();
    for (c, (key, docs)) in &members {
        let derived = by.remove(c).unwrap_or_default();
        let mut stored = Rows { keys: docs.iter().filter_map(|d| paths.get(d).map(|p| (p.clone(), *d))).collect(), rels: BTreeMap::new() };
        for (rel, &(_, width)) in &plan.memo {
            stored.rels.insert(rel.clone(), (width, derived.get(rel).cloned().unwrap_or_default()));
        }
        if let Some(hit) = found.hits.get(c) {
            if mode == Mode::Verify {
                for (rel, (width, values)) in &stored.rels {
                    let had = hit.rels.get(rel).map(|(_, v)| v.clone()).unwrap_or_default();
                    let set = |v: &[i64]| v.chunks(*width).map(<[i64]>::to_vec).collect::<BTreeSet<_>>();
                    let (had, now) = (set(&had), set(values));
                    if had != now {
                        differ.push(format!(
                            "{rel} about the component of {} ({} stored, {} derived, {} only stored, {} only derived)",
                            docs.first().and_then(|d| paths.get(d)).map_or("?", String::as_str),
                            had.len(),
                            now.len(),
                            had.difference(&now).count(),
                            now.difference(&had).count()
                        ));
                    }
                }
                recorded.verified += 1;
            }
            continue;
        }
        recorded.rows += stored.rels.values().map(|(w, v)| v.len() / w).sum::<usize>();
        recorded.sources.push(record_named(store, &name(&plan.digest, key), &plan.digest, &stored.encode())?);
        recorded.stored += 1;
    }
    if !differ.is_empty() {
        return Err(Error::Plan(format!("RERAIL_CROSS_MEMO=verify: stored rows differ from those derived: {}", differ.join("; "))));
    }
    Ok(recorded)
}

/// The component sub-program's relations a run holds: the components and the edges.
pub(super) const HELD: [&str; 2] = [COMPONENT, EDGE];

/// A component's stored rows: each of its documents' keys (by path), and each relation's width and rows (flat).
#[derive(Default, Debug, PartialEq)]
pub(super) struct Rows {
    pub(super) keys: BTreeMap<String, i64>,
    pub(super) rels: BTreeMap<String, (usize, Vec<i64>)>,
}

impl Rows {
    pub(super) fn encode(&self) -> Vec<u8> {
        let mut b = Vec::new();
        let put_str = |b: &mut Vec<u8>, s: &str| {
            b.extend_from_slice(&(s.len() as u32).to_le_bytes());
            b.extend_from_slice(s.as_bytes());
        };
        b.extend_from_slice(&ROWS_SCHEMA.to_le_bytes());
        b.extend_from_slice(&(self.keys.len() as u32).to_le_bytes());
        for (path, key) in &self.keys {
            put_str(&mut b, path);
            b.extend_from_slice(&key.to_le_bytes());
        }
        b.extend_from_slice(&(self.rels.len() as u32).to_le_bytes());
        for (rel, (width, values)) in &self.rels {
            put_str(&mut b, rel);
            b.extend_from_slice(&(*width as u32).to_le_bytes());
            b.extend_from_slice(&(values.len() as u64).to_le_bytes());
            for v in values {
                b.extend_from_slice(&v.to_le_bytes());
            }
        }
        zstd::encode_all(b.as_slice(), 3).expect("zstd over memory")
    }

    pub(super) fn decode(bytes: &[u8]) -> Option<Rows> {
        let b = zstd::decode_all(bytes).ok()?;
        let mut at = 0usize;
        let mut take = |n: usize| -> Option<&[u8]> {
            let s = b.get(at..at + n)?;
            at += n;
            Some(s)
        };
        let u32_ = |s: &[u8]| u32::from_le_bytes(s.try_into().expect("4 bytes"));
        if u32_(take(4)?) != ROWS_SCHEMA {
            return None;
        }
        let mut rows = Rows::default();
        for _ in 0..u32_(take(4)?) {
            let n = u32_(take(4)?) as usize;
            let path = String::from_utf8(take(n)?.to_vec()).ok()?;
            let key = i64::from_le_bytes(take(8)?.try_into().ok()?);
            rows.keys.insert(path, key);
        }
        for _ in 0..u32_(take(4)?) {
            let n = u32_(take(4)?) as usize;
            let rel = String::from_utf8(take(n)?.to_vec()).ok()?;
            let width = u32_(take(4)?) as usize;
            let len = u64::from_le_bytes(take(8)?.try_into().ok()?) as usize;
            let raw = take(len.checked_mul(8)?)?;
            let values = raw.chunks_exact(8).map(|c| i64::from_le_bytes(c.try_into().expect("8 bytes"))).collect();
            rows.rels.insert(rel, (width, values));
        }
        Some(rows)
    }
}

/// The stored source a derivation's output `name` binds, and its bytes.
fn lookup_source(store: &ContentStore, name: &str) -> Option<(crate::content_store::SourceId, Vec<u8>)> {
    let digest = store.binding(name).ok()??;
    let source = store.source_by_hash(HashKey::from_sha256_digest(digest)).ok()??;
    store.get_source(source).ok().map(|bytes| (source, bytes))
}

/// Stores `bytes` as the derivation `name` of `producer`, with no named input: `name` is a function of its inputs.
fn record_named(store: &ContentStore, name: &str, producer: &[u8; 32], bytes: &[u8]) -> Result<crate::content_store::SourceId> {
    use crate::content_store::DerivationRecord;
    let plan = |e: crate::content_store::StoreError| Error::Plan(e.to_string());
    let source = store.put_source(bytes, &[]).map_err(plan)?;
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let record = DerivationRecord { output: name.to_owned(), producer: *producer, inputs: Vec::new(), digest, status: 0, cost: 0 };
    store.record_derivations(&[record]).map_err(plan)?;
    Ok(source)
}

