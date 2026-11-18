//! A corpus run's cross-file rows, memoised per import component (decision 0076).
//!
//! - A program asks with `.pragma "cross-memo" "a b …"`. That names the cross-file relations whose rows are about
//!   the document of their first node column and are a function of that document's import closure; `"*"` names
//!   every relation the contract's static half admits.
//! - The run adds `components.rr` (`add_to`). Its file-local rules export each document's imports, module and own
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
use super::{Error, Result, front, lower};
use crate::code_rule_dsl::logic::program_image::{AtomRef, Builder, CmpOp, Lit, LitRef, PartFields, Rewrite, RuleRef, T, TermRef, Version};
use crate::code_rule_dsl::logic::program_image::Slot;
use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
use super::super::fields::Fields;
use std::sync::Arc;
use crate::content_store::ContentStore;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use crate::code_rule_dsl::logic::program_image::passes::Parts as _;

pub(super) const PRAGMA: &str = "cross-memo";
pub(super) const COMPONENT: &str = "rr_cx_component";
const EDGE: &str = "rr_cx_edge";
const PREFIX: &str = "rr_cx_";
pub(super) const SUFFIX: &str = "__cross";
pub(super) const DOCS: &str = "rr_cross_doc";
/// The facts `world` gives a run (its answers), not in the plan's digest.
const WORLD_FACTS: [&str; 3] = ["dir_entry", "file_line", "loaded"];
const SCHEMA: &str = "rerail.cross-rows/v2";
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
pub(super) fn declared(prog: &(impl ProgramFields + ?Sized)) -> Option<BTreeSet<String>> {
    let mut out: Option<BTreeSet<String>> = None;
    for text in prog.setting_values("pragma") {
        let words: Vec<&str> = text.split('"').map(str::trim).filter(|w| !w.is_empty()).collect();
        if let [PRAGMA, rels] = words.as_slice() {
            out.get_or_insert_default().extend(rels.split_whitespace().map(str::to_owned));
        }
    }
    out
}

/// Adds `components.rr` to the program's version: its natives, declarations, qualifiers, facts and inputs the program
/// lacks, `rr_cx_component` as an output, and its rules copied from its image, each one the program does not hold
/// already. Only the engine's own rules count (`rr_` heads: `rr_cx_*` and what lowering made for `components`), not
/// what `core.rr` brings with it: a program that declares a base relation itself keeps its own declaration, and its
/// input or rule as its lowering made it (`docof` over nodes is a rule over `node`, not an input), and what it keeps
/// (`.decl rr_cx_edge(…) no_inline`, decision 0099).
pub(super) fn add_to(v: &Arc<Version>) -> Result<Arc<Version>> {
    use crate::code_rule_dsl::logic::program_facts::Field;
    let path = crate::code_rule_dsl::logic::rr::library().join("components.rr");
    let image = crate::code_rule_dsl::logic::rr::parse_file_version(&path).map_err(|e| Error::Plan(format!("{}: {e:?}", path.display())))?;
    let own = |r: &crate::code_rule_dsl::logic::program_image::RuleRef<'_>| image.rel_name(r.head().rel()).starts_with("rr_");
    let mut needed: BTreeSet<String> = BTreeSet::new();
    for r in image.rules().filter(own) {
        needed.insert(image.rel_name(r.head().rel()).to_owned());
        needed.extend(r.body().filter_map(|l| match l.view() {
            Lit::Atom(a) => Some(image.rel_name(a.rel()).to_owned()),
            _ => None,
        }));
    }
    let mut b = Builder::on(v);
    // The fields first: a copied rule's relations are declared.
    let mut natives = v.native_map().map_err(Error::Plan)?;
    let natives_were = natives.len();
    let theirs = image.native_map().map_err(Error::Plan)?;
    let taken: Vec<_> = theirs.iter().filter(|(rel, _)| needed.contains(*rel)).collect();
    for (rel, native) in taken {
        needed.extend(native.reads().into_iter().map(str::to_owned));
        natives.entry(rel.clone()).or_insert_with(|| native.clone());
    }
    if natives.len() != natives_were {
        b.set_field(Field::Natives(&natives));
    }
    let missing: BTreeSet<String> = needed.iter().filter(|rel| !v.declares(rel)).cloned().collect();
    for rel in &missing {
        let Some(cols) = image.columns_of(rel) else { continue };
        let pairs: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
        b.declare(rel, &pairs);
    }
    let mut qualifiers = v.qualifier_map().map_err(Error::Plan)?;
    let mut qualified = false;
    for (rel, words) in image.qualifier_map().map_err(Error::Plan)?.iter().filter(|(rel, _)| missing.contains(*rel)) {
        let had = qualifiers.entry(rel.clone()).or_default();
        let n = had.len();
        had.extend(words.iter().cloned());
        qualified |= had.len() != n;
    }
    if qualified {
        b.set_field(Field::Qualifiers(&qualifiers));
    }
    for rel in ProgramFields::fact_names(&*image).iter().filter(|rel| missing.contains(*rel)) {
        if !ProgramFields::has_fact_rows(&**v, rel) {
            b.add_facts(rel, &ProgramFields::fact_rows(&*image, rel));
        }
    }
    for input in image.input_names().iter().filter(|rel| missing.contains(*rel)) {
        b.add_input(input);
    }
    b.add_output(COMPONENT);
    // A rule the program holds already is taken once (by its hash and its text).
    let held: std::collections::HashMap<u64, Vec<String>> = v.rules().fold(std::collections::HashMap::new(), |mut m, r| {
        m.entry(r.hash()).or_insert_with(Vec::new).push(v.show_rule(r));
        m
    });
    let mut added: std::collections::HashSet<String> = std::collections::HashSet::new();
    for r in image.rules().filter(own) {
        let text = image.show_rule(r);
        if held.get(&r.hash()).is_some_and(|ts| ts.contains(&text)) || !added.insert(text) {
            continue;
        }
        b.copy(&image, r);
    }
    b.finish().map(|(v, _)| v).map_err(Error::Plan)
}

/// What `split` makes of the cross-file part.
#[derive(Clone)]
pub(super) struct Plan {
    /// The rules that make `rr_cx_component`, run before the rest.
    pub(super) components: Arc<Version>,
    /// The memoised relations: the column of each one's first node, and its width.
    pub(super) memo: BTreeMap<String, (usize, usize)>,
    /// Each memoised relation's atom columns (`lower::atom_columns`): stored as the store numbers its atoms (decision 0170).
    pub(super) syms: BTreeMap<String, Vec<bool>>,
    /// The digest every stored row is a function of besides its component's key.
    pub(super) digest: [u8; 32],
}

/// The cross-file relations a memoised relation may read as if they were base facts:
/// - projections of an input that keep a node column (`kind`, `docof` over nodes): every rule copies one row of a
///   base relation or an export, so each row is about that row's node;
/// - the run's constants: relations whose rules read only the program's own facts (settings, `world`'s answers) and
///   other constants, such as `py_version`. The facts are in the plan's digest, so a row that reads them is still a
///   function of the closure under that plan.
fn free(v: &Version, f: &Fields<'_>, derived: &BTreeSet<String>) -> BTreeSet<String> {
    let rules_of = |rel: &str| v.rel(rel).into_iter().flat_map(|r| v.heads(r));
    let node_column = |rel: &str| f.types(rel).contains(&"node");
    let mut free: BTreeSet<String> = derived
        .iter()
        .filter(|rel| f.native(rel).is_none())
        // A head that drops the node (`isqmod(q) :- modname(_, q)`) is a set over every document, not a view.
        .filter(|rel| node_column(rel))
        .filter(|rel| {
            rules_of(rel).all(|r| {
                let mut body = r.body();
                match (body.next().map(|l| l.view()), body.next()) {
                    (Some(Lit::Atom(a)), None) => {
                        let read = v.rel_name(a.rel());
                        !a.neg() && f.is_input(read) && !read.ends_with(SUFFIX)
                    }
                    _ => false,
                }
            })
        })
        .cloned()
        .collect();
    // Declared, made by no rule and bound from no chunk: the program's own rows, if any.
    let fact = |rel: &str| f.declared(rel) && !derived.contains(rel) && !f.is_input(rel);
    let mut constants: BTreeSet<String> = BTreeSet::new();
    loop {
        let more: Vec<String> = derived
            .iter()
            .filter(|r| !constants.contains(*r) && f.native(r).is_none())
            .filter(|rel| rules_of(rel).all(|r| r.reads().all(|a| { let read = v.rel_name(a.rel()); fact(read) || constants.contains(read) })))
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
fn closed(v: &Version, f: &Fields<'_>, derived: &BTreeSet<String>) -> BTreeSet<String> {
    let mut set: BTreeSet<String> = derived
        .iter()
        .filter(|r| !r.starts_with(PREFIX) && f.native(r).is_none() && f.types(r).contains(&"node"))
        .cloned()
        .collect();
    loop {
        let out: Vec<String> = v
            .rules()
            .filter(|r| set.contains(v.rel_name(r.head().rel())))
            .filter(|r| r.reads().map(|a| v.rel_name(a.rel())).any(|a| derived.contains(a) && !set.contains(a) && !a.starts_with(PREFIX)))
            .map(|r| v.rel_name(r.head().rel()).to_owned())
            .collect();
        if out.is_empty() {
            return set;
        }
        for r in out {
            set.remove(&r);
        }
    }
}

/// The variables of rule `r` on the document of variable `x`: `x`, and every variable an atom with same-document
/// columns (`same_doc`) joins to one of them, transitively.
fn same_document(v: &Version, r: RuleRef<'_>, same_doc: &BTreeMap<String, Vec<usize>>, x: Slot) -> BTreeSet<Slot> {
    let groups: Vec<BTreeSet<Slot>> = r
        .reads()
        .filter(|a| !a.neg())
        .filter_map(|a| {
            let cols = same_doc.get(v.rel_name(a.rel()))?;
            let args: Vec<TermRef<'_>> = a.args().collect();
            Some(cols.iter().filter_map(|&c| args.get(c).and_then(|t| t.var())).collect())
        })
        .collect();
    let mut reached: BTreeSet<Slot> = BTreeSet::from([x]);
    loop {
        let before = reached.len();
        for g in &groups {
            if !g.is_disjoint(&reached) {
                reached.extend(g.iter().copied());
            }
        }
        if reached.len() == before {
            return reached;
        }
    }
}

/// Keeps every literal of a rule copied whole.
struct Whole;
impl Rewrite for Whole {}

/// The cross-file part's version `cross` with the component rules split off (decision 0076), the declared relations
/// checked, and their replay inputs, copies and guards added: a part of `cross` (decision 0244's form), then where
/// relations are memoised a derivation of that part (decision 0241's form), each guarded rule rewritten, the copies
/// added, with the replay's declarations, inputs and qualifiers. `local` is the file-local program's digest
/// (`memo::program_digest`). A part with no component rules is returned as it is, with no plan.
pub(super) fn split(
    cross: &Arc<Version>,
    domains: &lower::Domains,
    declared: &BTreeSet<String>,
    local: &[u8; 32],
    same_doc: &BTreeMap<String, Vec<usize>>,
) -> Result<(Option<Plan>, Arc<Version>)> {
    let name = |r: RuleRef<'_>| cross.rel_name(r.head().rel());
    let heads: BTreeSet<&str> = cross.rules().map(name).collect();
    if !heads.contains(COMPONENT) {
        return Ok((None, Arc::clone(cross)));
    }
    // The component rules: what `rr_cx_component` reads, transitively, among the cross-file rules.
    let mut cone = BTreeSet::from([COMPONENT.to_owned()]);
    loop {
        let before = cone.len();
        let reads: Vec<String> = cross
            .rules()
            .filter(|r| cone.contains(name(*r)))
            .flat_map(|r| r.reads().map(|a| cross.rel_name(a.rel())))
            .filter(|rel| heads.contains(rel))
            .map(str::to_owned)
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
        .rules()
        .filter(|r| !cone.contains(name(*r)))
        .flat_map(|r| r.reads().map(|a| cross.rel_name(a.rel())))
        .filter(|rel| cone.contains(*rel))
        .map(str::to_owned)
        .collect();
    while let Some(rel) = todo.pop() {
        if shared.insert(rel.clone()) {
            todo.extend(cross.rel(&rel).into_iter().flat_map(|h| cross.heads(h)).flat_map(|r| r.reads().map(|a| cross.rel_name(a.rel()))).filter(|x| cone.contains(*x)).map(str::to_owned));
        }
    }
    let all_rels = |_: &str| true;
    let in_cone = |rel: &str| cone.contains(rel);
    let inputs: Vec<&str> = cross.inputs().map(|a| cross.text(a)).collect();
    let outputs: Vec<&str> = cross.outputs().map(|a| cross.text(a)).collect();
    let components = cross
        .part(&in_cone, &inputs, &[COMPONENT], PartFields { natives: &in_cone, lattice: &all_rels, gates: &all_rels })
        .map_err(Error::Plan)?;
    let kept = |rel: &str| !cone.contains(rel) || shared.contains(rel);
    let kept_outputs: Vec<&str> = outputs.iter().copied().filter(|o| *o != COMPONENT).collect();
    let part = cross.part(&kept, &inputs, &kept_outputs, PartFields { natives: &kept, lattice: &all_rels, gates: &all_rels }).map_err(Error::Plan)?;
    let placed = super::super::body::Placed::whole(Arc::clone(&part));
    let f = placed.fields()?;
    // The plan: the engine, the grammars, the file-local part (what the exports are) and the cross-file part.
    let mut hash = Sha256::new();
    hash.update(SCHEMA.as_bytes());
    hash.update(crate::code_rule_dsl::build_ids::engine().as_bytes());
    hash.update(crate::code_rule_dsl::build_ids::grammar_version().to_le_bytes());
    hash.update(local);
    // The part's digest (its rules, declarations, facts and fields), less the file system's answers (`world`) and the
    // relations with no facts: a program prepared afresh has `dir_entry: []` where the one read back from the store
    // (decision 0073) has nothing, and the two must store under one plan. A loaded document has its own key, and a
    // resolution decides the edges, so a component's key covers what its rows read of them. In the digest, a file
    // added anywhere (a directory's entries) or a load fewer would miss every component. `RERAIL_CROSS_MEMO=verify`
    // over an edited corpus is the check (O114).
    let digested = {
        let mut b = Builder::on(&part);
        for facts in part.all_facts() {
            let rel = part.text(facts.name());
            if !facts.about() && (facts.rows().len() == 0 || WORLD_FACTS.contains(&rel)) {
                b.drop_facts(rel);
            }
        }
        b.finish_overlay().map_err(Error::Plan)?
    };
    hash.update(digested.digest().to_le_bytes());
    let digest: [u8; 32] = hash.finalize().into();
    // The contract's static half: a memoised relation has a node column, and its rules read only what is not derived
    // across files, the components' facts, and other memoised relations.
    // A native relation of the cross-file part is derived across files too, and is not replayed: it has no rules to
    // copy into or guard.
    // What a memoised rule may read freely is not counted: the run's constants and the views of `node` (`free`).
    let all: BTreeSet<String> = part.rules().map(|r| part.rel_name(r.head().rel()).to_owned()).chain(f.natives().iter().map(|n| f.rel_name(n.rel).to_owned())).collect();
    let free = free(&part, &f, &all);
    let derived: BTreeSet<String> = all.difference(&free).cloned().collect();
    let closed = closed(&part, &f, &derived);
    let node_column = |rel: &str| f.types(rel).contains(&"node");
    crate::code_rule_dsl::logic::profile::decision("cross-file memo: the static contract", || {
        // What keeps the others out: cross-file relations that cannot be declared (no node column, or native), with
        // how many rules of node-column relations read each.
        let mut blockers: BTreeMap<&str, usize> = BTreeMap::new();
        for r in part.rules().filter(|r| { let h = part.rel_name(r.head().rel()); !closed.contains(h) && node_column(h) }) {
            for a in r.reads().map(|a| part.rel_name(a.rel())) {
                let declarable = node_column(a) && f.native(a).is_none();
                if derived.contains(a) && !declarable && !a.starts_with(PREFIX) {
                    *blockers.entry(a).or_default() += 1;
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
            blockers.iter().map(|(r, n)| format!("{r}{}{n}", if f.native(r).is_some() { " (native) ×" } else { " ×" })).collect::<Vec<_>>().join(", ")
        )
    });
    // `*`: every relation the static half admits.
    let declared: &BTreeSet<String> = if declared.contains("*") { &closed } else { declared };
    let mut memo = BTreeMap::new();
    for rel in declared.iter().filter(|r| derived.contains(*r)) {
        if f.native(rel).is_some() {
            return Err(Error::Plan(format!("{rel}: a native relation cannot be declared `{PRAGMA}`: it is made whole, not by rules")));
        }
        let types = f.types(rel);
        let first = types.iter().position(|t| *t == "node").ok_or_else(|| Error::Plan(format!("{rel}: a `{PRAGMA}` relation needs a node column, the document its rows are about")))?;
        memo.insert(rel.to_string(), (first, types.len()));
    }
    // Those of its `sym` columns that hold names: a kind's or field's grammar code is no atom (`lower::atom_columns`).
    let syms: BTreeMap<String, Vec<bool>> = memo.keys().map(|rel| (rel.clone(), lower::atom_columns(&f.types(rel), domains, rel))).collect();
    for r in part.rules().filter(|r| memo.contains_key(part.rel_name(r.head().rel()))) {
        for a in r.reads().map(|a| part.rel_name(a.rel())) {
            if derived.contains(a) && !memo.contains_key(a) && !a.starts_with(PREFIX) {
                return Err(Error::Plan(format!(
                    "{}: a `{PRAGMA}` relation reads {a}, a cross-file relation the pragma does not name, whose rows may depend on files outside the import closure",
                    part.rel_name(r.head().rel())
                )));
            }
        }
    }
    if memo.is_empty() {
        return Ok((Some(Plan { components, memo, syms, digest }), part));
    }
    // Replay: an input and a copy per memoised relation; its other rules derive no row about a replayed document. Its
    // version: the part's rules copied by their handles, each guarded one rewritten, then the copies; the replay's
    // declarations, inputs and qualifiers stated.
    let mut b = Builder::derive(&part);
    for r in part.rules() {
        let guarded = memo.get(part.rel_name(r.head().rel())).and_then(|&(first, _)| {
            // A copy reads its replay input, and is not guarded.
            let copy = r.reads().any(|a| part.rel_name(a.rel()).ends_with(SUFFIX));
            let x = r.head().args().nth(first).and_then(|t| t.var()).filter(|_| !copy)?;
            // The guard on the head's node, and on every node the body's same-document atoms join to it
            // (`same_document`): each has the head's document, or a guest or host of it, which shares its component
            // (`components.rr`), so each guard admits the same rows. The planner applies each as soon as its node is
            // bound, so a found document's rows stop at the first atom that binds one, before the rest of the join
            // (O114).
            let nodes: BTreeSet<Slot> = r
                .reads()
                .filter(|a| !a.neg())
                .flat_map(|a| {
                    let types = f.types(part.rel_name(a.rel()));
                    a.args().enumerate().filter_map(move |(i, t)| t.var().filter(|_| types.get(i) == Some(&"node"))).collect::<Vec<_>>()
                })
                .collect();
            let mut guarded = vec![x];
            guarded.extend(same_document(&part, r, same_doc, x).into_iter().filter(|y| *y != x && nodes.contains(y)));
            Some(guarded)
        });
        match guarded {
            None => {
                b.copy(&part, r);
            }
            Some(guarded) => {
                let mut rule = b.rewrite(&part, r, &mut Whole);
                for (i, x) in guarded.into_iter().enumerate() {
                    let d = if i == 0 { "rr_xd".to_owned() } else { format!("rr_xd{i}") };
                    rule = rule.cmp(T::Var(&d), CmpOp::Eq, T::Call("doc", vec![T::Var(part.text(r.var(x)))])).not(DOCS, &[T::Var(&d)]);
                }
                rule.finish().map_err(Error::Plan)?;
            }
        }
    }
    let mut inputs: Vec<String> = inputs.iter().map(|s| (*s).to_owned()).collect();
    b.declare(DOCS, &[("d", "int")]);
    inputs.push(DOCS.to_owned());
    for rel in memo.keys() {
        let input = format!("{rel}{SUFFIX}");
        let cols = f.columns(rel).unwrap_or_default();
        b.declare(&input, &cols);
        inputs.push(input.clone());
        let vars: Vec<String> = (0..cols.len()).map(|i| format!("rr_x{i}")).collect();
        let vars: Vec<T<'_>> = vars.iter().map(|x| T::Var(x)).collect();
        b.rule().head(rel.as_str(), &vars).atom(input.as_str(), &vars).finish().map_err(Error::Plan)?;
    }
    b.set_inputs(&inputs);
    // Stored, never inlined into its readers (decision 0099): its derived rows are read from its table to be
    // recorded (`record`), and replayed into it.
    let words: Vec<(&str, &str)> = memo.keys().map(|rel| (rel.as_str(), "no_inline")).collect();
    b.add_qualifiers(&words);
    let (version, _) = b.finish().map_err(Error::Plan)?;
    Ok((Some(Plan { components, memo, syms, digest }), version))
}

/// Drops the guards `split` added (`$rr_xd = @doc(x), !rr_cross_doc($rr_xd)`): for a run that found no component,
/// whose `rr_cross_doc` is empty, so every guard passes. Each guard was two steps over every row its rule's delta
/// gave (calls_cx, cold: before the selective join of most rules of its largest stratum). Returns the guards dropped.
pub(super) fn drop_guards(cross: &mut super::lower::Lowered) -> Result<usize> {
    let v = cross.version.clone();
    let is_docs = |a: AtomRef<'_>| v.rel_name(a.rel()) == DOCS && a.neg();
    // Each rule's guards: the variables its `!rr_cross_doc(d)` atoms read.
    let guards_of = |r: RuleRef<'_>| -> BTreeSet<Slot> {
        r.reads().filter(|a| is_docs(*a)).filter_map(|a| {
            let mut args = a.args();
            match (args.next(), args.next()) {
                (Some(TermRef::Var(d)), None) => Some(d),
                _ => None,
            }
        }).collect()
    };
    /// Drops a rule's guards: its `!rr_cross_doc` atoms, and the `d = @doc(x)` that bind their variables.
    struct Unguard<'a> {
        v: &'a Version,
        guards: BTreeSet<Slot>,
        dropped: usize,
    }
    impl Rewrite for Unguard<'_> {
        fn keep(&mut self, l: LitRef<'_>) -> bool {
            let drop = match l.view() {
                Lit::Atom(a) => self.v.rel_name(a.rel()) == DOCS && a.neg(),
                Lit::Cmp { l: TermRef::Var(d), op: CmpOp::Eq, r: TermRef::Call(f, _) } => self.v.text(f) == "doc" && self.guards.contains(&d),
                _ => false,
            };
            self.dropped += drop as usize;
            !drop
        }
    }
    // Its version: a derivation of the part's (decision 0241's form), each rule that keeps its guards copied by its
    // handle, each that drops them written again without them, in order.
    let mut dropped = 0;
    let mut b = Builder::derive(&v);
    for r in v.rules() {
        let guards = guards_of(r);
        if guards.is_empty() {
            b.copy(&v, r);
            continue;
        }
        let mut rw = Unguard { v: &v, guards, dropped: 0 };
        b.rewrite(&v, r, &mut rw).finish().map_err(Error::Plan)?;
        dropped += rw.dropped / 2;
    }
    let (made, _) = b.finish().map_err(Error::Plan)?;
    cross.version = made;
    Ok(dropped)
}

/// Drops the memoised relations' rules but their copies from `<rel>__cross`: for a run where every component was
/// found, whose guards (`!rr_cross_doc`) every other rule's rows fail.
/// What only those rules read is dropped too (`rr_cx_reach`); `kept` names what the run reads besides its outputs.
/// Its version is a view of the part's, its natives an overlay (decision 0244's form).
pub(super) fn keep_copies(cross: &mut super::lower::Lowered, plan: &Plan, kept: &BTreeSet<String>) -> Result<()> {
    let v = cross.version.clone();
    let placed = super::super::body::Placed::whole(Arc::clone(&v));
    let f = placed.fields()?;
    let rules: Vec<RuleRef<'_>> = v.rules().collect();
    let head = |r: &RuleRef<'_>| v.rel_name(r.head().rel());
    let mut keep: Vec<bool> = rules.iter().map(|r| !plan.memo.contains_key(head(r)) || r.reads().any(|a| v.rel_name(a.rel()).ends_with(SUFFIX))).collect();
    let mut natives_gone: BTreeSet<String> = BTreeSet::new();
    loop {
        let kept_rules = || rules.iter().zip(&keep).filter(|(_, k)| **k).map(|(r, _)| r);
        let read: BTreeSet<&str> = kept_rules().flat_map(|r| r.reads().map(|a| v.rel_name(a.rel()))).collect();
        let unread = |rel: &str| !read.contains(rel) && !kept.contains(rel) && !f.is_output(rel);
        let dead: BTreeSet<String> = kept_rules()
            .map(|r| head(r).to_owned())
            .chain(f.natives().iter().map(|n| f.rel_name(n.rel).to_owned()).filter(|n| !natives_gone.contains(n)))
            .filter(|r| unread(r))
            .collect();
        if dead.is_empty() {
            break;
        }
        for (r, k) in rules.iter().zip(keep.iter_mut()) {
            *k &= !dead.contains(head(r));
        }
        natives_gone.extend(dead.into_iter().filter(|r| f.native(r).is_some()));
    }
    let mut b = Builder::on(&v);
    for (r, k) in rules.iter().zip(&keep) {
        if !k {
            b.remove(r.id());
        }
    }
    let (mut made, _) = b.finish().map_err(Error::Plan)?;
    if !natives_gone.is_empty() {
        let mut b = Builder::on(&made);
        b.keep_relation_fields(&|r| !natives_gone.contains(r), &|_| true, &|_| true);
        made = b.finish_overlay().map_err(Error::Plan)?;
    }
    cross.version = made;
    Ok(())
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
            store.cross_rows(&self.hits.values().copied().collect::<Vec<_>>(), &rels, &plan.syms, false).map_err(|e| Error::Plan(e.to_string()))?
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
        let stored = store.cross_rows(&found.hits.values().copied().collect::<Vec<_>>(), &rels, &plan.syms, true).map_err(|e| Error::Plan(e.to_string()))?;
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
        recorded.sources.push(store.put_cross_rows(&digest, &bytes, &rels, &plan.syms).map_err(|e| Error::Plan(e.to_string()))?);
        recorded.stored += 1;
    }
    if !differ.is_empty() {
        return Err(Error::Plan(format!("RERAIL_CROSS_MEMO=verify: stored rows differ from those derived: {}", differ.join("; "))));
    }
    Ok(recorded)
}

/// The component sub-program's relations a run holds: the components and the edges.
pub(super) const HELD: [&str; 2] = [COMPONENT, EDGE];
