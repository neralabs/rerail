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
//! - `lookup` finds every component's stored rows by its descriptor's digest (plan, key, documents' keys), in one
//!   query. The hits are replayed, their rows read as columns by one more query; the misses are derived.
//! - `record` stores the misses' rows in the store's `cross_rows` table. It checks the contract's other half: each
//!   node of a stored row lies in the closure of its first node's component.
//! - With `RERAIL_CROSS_MEMO=verify` nothing is replayed, and the hits' stored rows are compared with what was
//!   derived.
use super::{Error, Result, front};
use crate::content_store::ContentStore;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

pub(super) const PRAGMA: &str = "cross-memo";
pub(super) const COMPONENT: &str = "rr_cx_component";
const EDGE: &str = "rr_cx_edge";
const PREFIX: &str = "rr_cx_";
pub(super) const SUFFIX: &str = "__cross";
pub(super) const DOCS: &str = "rr_cross_doc";
/// The facts `world` gives a run (its answers), not in the plan's digest.
const WORLD_FACTS: [&str; 3] = ["dir_entry", "file_line", "loaded"];
const SCHEMA: &str = "rerail.cross-rows/v1";
/// The stored rows' encoding version: bumped when its meaning changes.
const ROWS_SCHEMA: u32 = 2;

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
        needed.insert(r.head.rel.to_string());
        needed.extend(r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.to_string()));
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
    // And what it keeps (`.decl rr_cx_edge(…) no_inline`, decision 0099).
    for (rel, words) in components.qualifiers.into_iter().filter(|(rel, _)| missing.contains(rel)) {
        prog.qualifiers.entry(rel).or_default().extend(words);
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
            .filter(|rel| rules_of(cross, rel).all(|r| r.body.iter().filter_map(|l| l.atom()).all(|a| fact(&a.rel) || constants.contains(a.rel.as_str()))))
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
            .filter(|r| set.contains(r.head.rel.as_str()))
            .filter(|r| r.body.iter().filter_map(|l| l.atom()).any(|a| derived.contains(a.rel.as_str()) && !set.contains(a.rel.as_str()) && !a.rel.starts_with(PREFIX)))
            .map(|r| r.head.rel.to_string())
            .collect();
        if out.is_empty() {
            return set;
        }
        for r in out {
            set.remove(&r);
        }
    }
}

pub(super) fn split(cross: &mut front::Program, declared: &BTreeSet<String>, local: &[u8; 32], same_doc: &BTreeMap<String, Vec<usize>>) -> Result<Option<Plan>> {
    use front::{Atom, Literal, Rule, Term};
    let heads: BTreeSet<String> = cross.rules.iter().map(|r| r.head.rel.to_string()).collect();
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
            .filter(|r| cone.contains(r.head.rel.as_str()))
            .flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.to_string()))
            .filter(|rel| heads.contains(rel.as_str()))
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
        .filter(|r| !cone.contains(r.head.rel.as_str()))
        .flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.to_string()))
        .filter(|rel| cone.contains(rel.as_str()))
        .collect();
    while let Some(rel) = todo.pop() {
        if shared.insert(rel.to_string()) {
            todo.extend(cross.rules.iter().filter(|r| r.head.rel == rel).flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.to_string())).filter(|x| cone.contains(x.as_str())));
        }
    }
    let mut components = cross.clone();
    components.rules.retain(|r| cone.contains(r.head.rel.as_str()));
    components.natives.retain(|rel, _| cone.contains(rel));
    components.outputs = vec![COMPONENT.to_owned()];
    cross.rules.retain(|r| !cone.contains(r.head.rel.as_str()) || shared.contains(r.head.rel.as_str()));
    cross.natives.retain(|rel, _| !cone.contains(rel) || shared.contains(rel));
    cross.outputs.retain(|o| o != COMPONENT);
    // The plan: the engine, the grammars, the file-local part (what the exports are) and the cross-file part.
    let mut hash = Sha256::new();
    hash.update(SCHEMA.as_bytes());
    hash.update(crate::code_rule_dsl::build_ids::engine().as_bytes());
    hash.update(crate::code_rule_dsl::workspace::grammar_version().to_le_bytes());
    hash.update(local);
    // A relation with no facts is left out: a program prepared afresh has `dir_entry: []` where the one read back from
    // the store (decision 0073) has nothing, and the two must store under one plan.
    // The file system's answers (`world`) are left out too: a loaded document has its own key, and a resolution
    // decides the edges, so a component's key covers what its rows read of them. In the digest, a file added
    // anywhere (a directory's entries) or a load fewer would miss every component. `RERAIL_CROSS_MEMO=verify` over an
    // edited corpus is the check (O114).
    let facts: Vec<_> = cross.facts.iter().filter(|(rel, rows)| !rows.is_empty() && !WORLD_FACTS.contains(&rel.as_str())).collect();
    hash.update(format!("{:?}{:?}{:?}{:?}{:?}", cross.decls, cross.rules, facts, cross.natives, cross.lattice).as_bytes());
    let digest: [u8; 32] = hash.finalize().into();
    // The contract's static half: a memoised relation has a node column, and its rules read only what is not derived
    // across files, the components' facts, and other memoised relations.
    // A native relation of the cross-file part is derived across files too, and is not replayed: it has no rules to
    // copy into or guard.
    // What a memoised rule may read freely is not counted: the run's constants and the views of `node` (`free`).
    let all: BTreeSet<String> = cross.rules.iter().map(|r| r.head.rel.to_string()).chain(cross.natives.keys().cloned()).collect();
    let free = free(cross, &all);
    let derived: BTreeSet<String> = all.difference(&free).cloned().collect();
    let closed = closed(cross, &derived);
    crate::code_rule_dsl::logic::profile::decision("cross-file memo: the static contract", || {
        // What keeps the others out: cross-file relations that cannot be declared (no node column, or native), with
        // how many rules of node-column relations read each.
        let mut blockers: BTreeMap<&str, usize> = BTreeMap::new();
        for r in cross.rules.iter().filter(|r| !closed.contains(r.head.rel.as_str()) && cross.decls.get(r.head.rel.as_str()).is_some_and(|c| c.iter().any(|(_, t)| t == "node"))) {
            for a in r.body.iter().filter_map(|l| l.atom()) {
                let declarable = cross.decls.get(a.rel.as_str()).is_some_and(|c| c.iter().any(|(_, t)| t == "node")) && !cross.natives.contains_key(a.rel.as_str());
                if derived.contains(a.rel.as_str()) && !declarable && !a.rel.starts_with(PREFIX) {
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
        memo.insert(rel.to_string(), (first, cols.len()));
    }
    for r in cross.rules.iter().filter(|r| memo.contains_key(r.head.rel.as_str())) {
        for a in r.body.iter().filter_map(|l| l.atom()) {
            if derived.contains(a.rel.as_str()) && !memo.contains_key(a.rel.as_str()) && !a.rel.starts_with(PREFIX) {
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
        // Stored, never inlined into its readers (decision 0099): its derived rows are read from its table to be
        // recorded (`record`), and replayed into it.
        cross.qualifiers.entry(rel.clone()).or_default().insert("no_inline".to_owned());
        let cols = cross.decls[rel].clone();
        let input = format!("{rel}{SUFFIX}");
        let vars: Vec<Term> = (0..cols.len()).map(|i| Term::Var(format!("rr_x{i}"))).collect();
        cross.decls.insert(input.clone(), cols);
        cross.inputs.push(input.clone());
        cross.rules.push(Rule {
            head: Atom { rel: rel.clone().into(), args: vars.clone(), neg: false, gated: false },
            body: vec![Literal::Atom(Atom { rel: input.into(), args: vars, neg: false, gated: false })],
        });
    }
    for rule in &mut cross.rules {
        let Some(&(first, _)) = memo.get(rule.head.rel.as_str()) else { continue };
        let copy = rule.body.iter().any(|l| matches!(l, Literal::Atom(a) if a.rel.ends_with(SUFFIX)));
        if copy {
            continue;
        }
        if let Some(Term::Var(v)) = rule.head.args.get(first) {
            // The guard on the head's node, and on every node the body's same-document atoms join to it
            // (`cross_rounds::same_document`): each has the head's document, or a guest or host of it, which shares
            // its component (`components.rr`), so each guard admits the same rows. The planner applies each as soon
            // as its node is bound, so a found document's rows stop at the first atom that binds one, before the
            // rest of the join (O114).
            let nodes: BTreeSet<&str> = rule
                .body
                .iter()
                .filter_map(|l| l.atom().filter(|a| !a.neg))
                .flat_map(|a| {
                    let decl = cross.decls.get(a.rel.as_str());
                    a.args.iter().enumerate().filter_map(move |(i, t)| match (t, decl.and_then(|d| d.get(i))) {
                        (Term::Var(x), Some((_, ty))) if ty == "node" => Some(x.as_str()),
                        _ => None,
                    })
                })
                .collect();
            let mut guarded: Vec<String> = vec![v.clone()];
            for x in crate::code_rule_dsl::typing::cross_rounds::same_document(&rule.body, same_doc, v) {
                if x != *v && nodes.contains(x.as_str()) {
                    guarded.push(x);
                }
            }
            for (i, x) in guarded.into_iter().enumerate() {
                let d = if i == 0 { "rr_xd".to_owned() } else { format!("rr_xd{i}") };
                rule.body.push(Literal::Cmp { l: Term::Var(d.clone()), op: "=", r: Term::Call("doc".to_owned(), vec![Term::Var(x)]) });
                rule.body.push(Literal::Atom(Atom { rel: DOCS.to_owned().into(), args: vec![Term::Var(d)], neg: true, gated: false }));
            }
        }
    }
    Ok(Some(Plan { components, memo, digest }))
}

/// Drops the guards `split` added (`$rr_xd = @doc(x), !rr_cross_doc($rr_xd)`): for a run that found no component,
/// whose `rr_cross_doc` is empty, so every guard passes. Each guard was two steps over every row its rule's delta
/// gave (calls_cx, cold: before the selective join of most rules of its largest stratum). Returns the guards dropped.
pub(super) fn drop_guards(cross: &mut front::Program) -> usize {
    use front::{Literal, Term};
    let mut dropped = 0;
    for rule in &mut cross.rules {
        let guards: BTreeSet<String> = rule
            .body
            .iter()
            .filter_map(|l| match l {
                Literal::Atom(a) if a.rel == DOCS && a.neg => match a.args.as_slice() {
                    [Term::Var(d)] => Some(d.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        if guards.is_empty() {
            continue;
        }
        let before = rule.body.len();
        rule.body.retain(|l| match l {
            Literal::Atom(a) => !(a.rel == DOCS && a.neg),
            Literal::Cmp { l: Term::Var(d), op: "=", r: Term::Call(f, _) } => !(f == "doc" && guards.contains(d)),
            _ => true,
        });
        dropped += (before - rule.body.len()) / 2;
    }
    dropped
}

/// Drops the memoised relations' rules but their copies from `<rel>__cross`: for a run where every component was
/// found, whose guards (`!rr_cross_doc`) every other rule's rows fail.
/// What only those rules read is dropped too (`rr_cx_reach`); `kept` names what the run reads besides its outputs.
pub(super) fn keep_copies(cross: &mut front::Program, plan: &Plan, kept: &BTreeSet<String>) {
    cross.rules.retain(|r| !plan.memo.contains_key(r.head.rel.as_str()) || r.body.iter().any(|l| matches!(l, front::Literal::Atom(a) if a.rel.ends_with(SUFFIX))));
    loop {
        let read: BTreeSet<&str> = cross.rules.iter().flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.as_str())).collect();
        let unread = |rel: &str| !read.contains(rel) && !kept.contains(rel) && !cross.outputs.iter().any(|o| o == rel);
        let dead: BTreeSet<String> = cross.rules.iter().map(|r| r.head.rel.to_string()).chain(cross.natives.keys().cloned()).filter(|r| unread(r)).collect();
        if dead.is_empty() {
            return;
        }
        cross.rules.retain(|r| !dead.contains(r.head.rel.as_str()));
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

/// A component's descriptor (the plan, its key and its documents' keys by path), which its stored source holds, and
/// the descriptor's digest, which names it (`cross_components`).
fn descriptor(plan: &[u8; 32], key: &str, keys: &BTreeMap<String, i64>) -> (String, Vec<u8>) {
    let mut b = Vec::new();
    let put_str = |b: &mut Vec<u8>, s: &str| {
        b.extend_from_slice(&(s.len() as u32).to_le_bytes());
        b.extend_from_slice(s.as_bytes());
    };
    put_str(&mut b, SCHEMA);
    b.extend_from_slice(&ROWS_SCHEMA.to_le_bytes());
    b.extend_from_slice(plan);
    put_str(&mut b, key);
    b.extend_from_slice(&(keys.len() as u32).to_le_bytes());
    for (path, k) in keys {
        put_str(&mut b, path);
        b.extend_from_slice(&k.to_le_bytes());
    }
    let digest: [u8; 32] = Sha256::digest(&b).into();
    (digest.iter().map(|x| format!("{x:02x}")).collect(), b)
}

/// A component's documents' keys, by path (a guest has none: its host's key covers it).
fn doc_keys(docs: &[i64], paths: &HashMap<i64, String>) -> BTreeMap<String, i64> {
    docs.iter().filter_map(|d| paths.get(d).map(|p| (p.clone(), *d))).collect()
}

/// What `lookup` found.
#[derive(Default)]
pub(super) struct Found {
    /// The components found, by component, with the source their rows are stored under.
    pub(super) hits: BTreeMap<i64, crate::content_store::SourceId>,
    /// The components that missed.
    pub(super) misses: BTreeSet<i64>,
    /// The hits' stored sources, which the run's manifest names so collection keeps them (decision 0053).
    pub(super) sources: Vec<crate::content_store::SourceId>,
}

impl Found {
    /// The inputs to bind, as columns: `rr_cross_doc` (the hits' documents) and the replayed rows of each memoised
    /// relation `need` admits, read from the store by one query. Empty when verifying, so everything is derived.
    pub(super) fn inputs(
        &self,
        store: &ContentStore,
        plan: &Plan,
        components: &Components,
        mode: Mode,
        need: impl Fn(&str) -> bool,
    ) -> Result<Vec<(String, usize, Vec<Vec<i64>>)>> {
        // Nothing memoised: `split` added no replay inputs to bind.
        if plan.memo.is_empty() {
            return Ok(Vec::new());
        }
        let replay = mode == Mode::On;
        let docs: Vec<i64> = if replay { components.of.iter().filter(|(_, (c, _))| self.hits.contains_key(c)).map(|(d, _)| *d).collect() } else { Vec::new() };
        let mut inputs = vec![(DOCS.to_owned(), 1, vec![docs])];
        let rels: Vec<(String, usize)> = plan.memo.iter().filter(|(r, _)| need(r)).map(|(r, &(_, w))| (r.clone(), w)).collect();
        let mut rows = if replay && !self.hits.is_empty() {
            store.cross_rows(&self.hits.values().copied().collect::<Vec<_>>(), &rels, false).map_err(|e| Error::Plan(e.to_string()))?
        } else {
            BTreeMap::new()
        };
        for (rel, width) in rels {
            let cols = rows.remove(&rel).unwrap_or_else(|| vec![Vec::new(); width]);
            inputs.push((format!("{rel}{SUFFIX}"), width, cols));
        }
        Ok(inputs)
    }
}

/// Finds each component's stored rows under `plan`, by its descriptor's digest (its documents' keys included), with
/// one query.
pub(super) fn lookup(store: &ContentStore, plan: &Plan, components: &Components, paths: &HashMap<i64, String>) -> Result<Found> {
    let members = components.members();
    let digests: Vec<(i64, String)> = members.iter().map(|(c, (key, docs))| (*c, descriptor(&plan.digest, key, &doc_keys(docs, paths)).0)).collect();
    let stored = store.cross_lookup(&digests.iter().map(|(_, d)| d.clone()).collect::<Vec<_>>()).map_err(|e| Error::Plan(e.to_string()))?;
    let mut found = Found::default();
    for (c, digest) in digests {
        match stored.get(&digest) {
            Some(&id) => {
                found.sources.push(id);
                found.hits.insert(c, id);
            }
            None => {
                found.misses.insert(c);
            }
        }
    }
    Ok(found)
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
/// row's nodes lie in the closure of its first node's component. When verifying, compares the hits' stored rows
/// (read by one query) with what was derived instead of trusting them.
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
    let grouping = crate::code_rule_dsl::logic::profile::span(|| "group by component and check closures".into());
    // Each (component, component a row names) pair is checked against the closure once, not once a row.
    let mut checked: HashSet<(i64, i64)> = HashSet::new();
    for (rel, (first, _)) in &plan.memo {
        let nodes = node_columns.get(rel).cloned().unwrap_or_default();
        // Each row tagged with its component, then sorted and deduplicated (a relation's rows can come from several
        // parts of the run).
        let mut tagged: Vec<(i64, &Vec<i64>)> = Vec::new();
        let mut last: Option<(i64, Option<i64>)> = None;
        let mut seen: Vec<Option<(i64, i64)>> = vec![None; nodes.len()];
        for row in rows.get(rel).into_iter().flatten() {
            let d = doc_of(row[*first]);
            let c = match last {
                Some((ld, c)) if ld == d => c,
                _ => {
                    let c = components.of.get(&d).map(|(c, _)| *c);
                    last = Some((d, c));
                    c
                }
            };
            let Some(c) = c else { continue };
            // A found component's rows are its replayed copies: nothing to check or store unless verifying.
            if mode == Mode::On && found.hits.contains_key(&c) {
                continue;
            }
            for (k, &n) in nodes.iter().enumerate() {
                // Consecutive rows mostly name the same document in a column: its component and check are kept.
                let dn = doc_of(row[n]);
                if seen[k] == Some((c, dn)) {
                    continue;
                }
                let Some((e, _)) = components.of.get(&dn) else { continue };
                if *e == c || checked.contains(&(c, *e)) {
                    seen[k] = Some((c, dn));
                    continue;
                }
                if !components.closure(c, &mut closures).contains(e) {
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
                checked.insert((c, *e));
                seen[k] = Some((c, dn));
            }
            tagged.push((c, row));
        }
        // One sort by (component, row): each component's rows in order, duplicates adjacent.
        tagged.sort_unstable();
        tagged.dedup();
        for group in tagged.chunk_by(|a, b| a.0 == b.0) {
            let out = by.entry(group[0].0).or_default().entry(rel.clone()).or_default();
            out.reserve(group.iter().map(|(_, r)| r.len()).sum());
            for (_, row) in group {
                out.extend_from_slice(row);
            }
        }
    }
    drop(grouping);
    let _putting = crate::code_rule_dsl::logic::profile::span(|| "put components' blocks".into());
    let members = components.members();
    let mut recorded = Recorded::default();
    let mut differ: Vec<String> = Vec::new();
    // Verifying: every hit's stored rows, by source and relation, from one query.
    let mut had: HashMap<(i64, String), BTreeSet<Vec<i64>>> = HashMap::new();
    if mode == Mode::Verify && !found.hits.is_empty() {
        let rels: Vec<(String, usize)> = plan.memo.iter().map(|(r, &(_, w))| (r.clone(), w)).collect();
        let stored = store.cross_rows(&found.hits.values().copied().collect::<Vec<_>>(), &rels, true).map_err(|e| Error::Plan(e.to_string()))?;
        for (rel, cols) in stored {
            let (source, cols) = cols.split_last().expect("a source column");
            for (i, s) in source.iter().enumerate() {
                had.entry((*s, rel.clone())).or_default().insert(cols.iter().map(|c| c[i]).collect());
            }
        }
    }
    for (c, (key, docs)) in &members {
        let derived = by.remove(c).unwrap_or_default();
        let keys = doc_keys(docs, paths);
        let mut rels: BTreeMap<String, (usize, Vec<i64>)> = BTreeMap::new();
        for (rel, &(_, width)) in &plan.memo {
            rels.insert(rel.clone(), (width, derived.get(rel).cloned().unwrap_or_default()));
        }
        if let Some(hit) = found.hits.get(c) {
            if mode == Mode::Verify {
                for (rel, (width, values)) in &rels {
                    let empty = BTreeSet::new();
                    let had = had.get(&(hit.raw(), rel.clone())).unwrap_or(&empty);
                    let now: BTreeSet<Vec<i64>> = values.chunks(*width).map(<[i64]>::to_vec).collect();
                    if *had != now {
                        differ.push(format!(
                            "{rel} about the component of {} ({} stored, {} derived, {} only stored, {} only derived)",
                            docs.first().and_then(|d| paths.get(d)).map_or("?", String::as_str),
                            had.len(),
                            now.len(),
                            had.difference(&now).count(),
                            now.difference(had).count()
                        ));
                    }
                }
                recorded.verified += 1;
            }
            continue;
        }
        recorded.rows += rels.values().map(|(w, v)| v.len() / w).sum::<usize>();
        let (digest, bytes) = descriptor(&plan.digest, key, &keys);
        recorded.sources.push(store.put_cross_rows(&digest, &bytes, &rels).map_err(|e| Error::Plan(e.to_string()))?);
        recorded.stored += 1;
    }
    if !differ.is_empty() {
        return Err(Error::Plan(format!("RERAIL_CROSS_MEMO=verify: stored rows differ from those derived: {}", differ.join("; "))));
    }
    Ok(recorded)
}

/// The component sub-program's relations a run holds: the components and the edges.
pub(super) const HELD: [&str; 2] = [COMPONENT, EDGE];
