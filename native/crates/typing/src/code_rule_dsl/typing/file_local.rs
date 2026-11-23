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

pub use crate::code_rule_dsl::facts::nearest::{Binding, NONE, Trees};
/// The flat lattice's absent and top values (`logic::native::BOT`, `TOP`).
const BOT: i64 = i64::MIN;
const TOP: i64 = 1 << 62;



/// The flat-lattice system: seeds, and copies `(cell, source)`.
pub struct System {
    pub seeds: Vector<i64>,
    pub copies: Vec<(u32, u32)>,
}

pub fn system(trees: &Trees<'_>, sets: &[Vector<i64>]) -> StoreResult<System> {
    let n = trees.parent.len();
    let cells = n.checked_mul(sets.len()).filter(|&c| c < u32::MAX as usize).ok_or_else(|| StoreError::Invalid(format!("nearest: {n} nodes in {} sets", sets.len())))?;
    let mut seeds = rerail_blocks::vector![BOT; cells];
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
pub fn solve_cpu(system: &System) -> Vector<i64> {
    let n = system.seeds.len();
    // Sources and dependents as CSR (a cell has one source here; the solver does not rely on it).
    let csr = |edges: &mut Vec<(u32, u32)>| -> (Vector<u32>, Vector<u32>) {
        edges.sort_unstable();
        let mut start = rerail_blocks::vector![0_u32; n + 1];
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
    let mut work: Vector<u32> = (0..n as u32).filter(|&c| values[c as usize] != BOT).collect();
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
use rerail_blocks::vector::Vector;

pub fn nearest(trees: &Trees<'_>, sets: &[Vector<i64>], binding: Binding) -> StoreResult<Vec<Vector<u32>>> {
    let n = trees.parent.len();
    if n == 0 || sets.is_empty() {
        return Ok((0..sets.len()).map(|_| Vector::new()).collect());
    }
    let mut out: Vec<Vector<u32>> = (0..sets.len()).map(|_| Vector::with_capacity(n)).collect();
    let per = (CELLS / sets.len()).max(1);
    let mut start = 0;
    while start < n {
        // The next piece ends at a root (a document's first node) once it holds `per` nodes; a tree is never split.
        let mut end = (start + per).min(n);
        while end < n && trees.parent[end] != NONE {
            end += 1;
        }
        let parent: Vector<u32> = trees.parent[start..end].iter().map(|&p| if p == NONE { NONE } else { p - start as u32 }).collect();
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
fn solve(trees: &Trees<'_>, sets: &[Vector<i64>], binding: Binding) -> StoreResult<Vec<Vector<u32>>> {
    use crate::code_rule_dsl::logic::profile;
    let n = trees.parent.len();
    let member = |k: usize, node: u32| sets[k].contains(&trees.kind[node as usize]);
    crate::code_rule_dsl::prof::count("nearest.cells", n * sets.len());
    if let Some(dev) = crate::code_rule_dsl::logic::device::solver().filter(|_| binding == Binding::Metal) {
        let _span = profile::span(|| "nearest on Metal (pointer jumping)".into());
        profile::decision("nearest ancestors", || format!("solved per piece of at most {CELLS} cells on Metal by pointer jumping, one dispatch and one wait"));
        let mut target: Vector<u32> = Vector::with_capacity(n * sets.len());
        let mut done: Vector<u8> = Vector::with_capacity(n * sets.len());
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
                return Ok(solved.targets.chunks(n.max(1)).map(Vector::from).collect());
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
                let mut found: Vector<u32> = rerail_blocks::vector![NONE; n];
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

mod on_version;
pub use on_version::FileLocal;
mod passes;
pub use passes::{FactorCaches, FactorizeExports, Locate, MoveCrossFile, PushLocalJoins, RequestsFileLocal, factorize_intermediates, locate_all};
#[cfg(any(test, feature = "test-hooks"))]
pub use passes::Readers;
pub mod export_demand;
pub use export_demand::DemandExports;
use crate::code_rule_dsl::logic::program_image::passes::Parts as _;
// cratify: `tests` is rerail-device-metal's (it mounts it)

/// The inputs whose rows are the run's options (`facts::repository::options`): the same beside every chunk and the
/// cross-file part, and named by the corpus memo's key (`corpus::memo::with_options`), so constants to the split.
pub fn option_input(rel: &str) -> bool {
    matches!(rel, "option" | "option_item")
}

/// An input with one value for the whole run, as a fact has: the run's options (`option_input`), or an earlier
/// phase's export (`corpus::phases`), bound once its rows are known. A rule reading one joins no document by it.
pub fn run_constant(prog: &(impl crate::code_rule_dsl::logic::program_image::fields::ProgramFields + ?Sized), rel: &str) -> bool {
    option_input(rel) || (crate::code_rule_dsl::logic::plan::phased::is_phased(rel) && prog.input_names().iter().any(|i| i == rel))
}

/// Rule `rule`'s `a = b` between two variables, and its `d = @doc(n)` (decision 0091's scope guards), by their slots:
/// d is n's document, so the two place one document, as a document column and a node of one atom do.
fn var_equalities_in(v: &crate::code_rule_dsl::logic::program_image::Version, rule: crate::code_rule_dsl::logic::program_image::RuleRef<'_>) -> Vec<(crate::code_rule_dsl::logic::program_image::Slot, crate::code_rule_dsl::logic::program_image::Slot)> {
    use crate::code_rule_dsl::logic::program_image::{CmpOp, Lit, TermRef};
    // `@doc(x)`'s variable.
    let doc_of = |t: TermRef<'_>| match t {
        TermRef::Call(f, mut xs) if v.text(f) == "doc" => match (xs.next(), xs.next()) {
            (Some(TermRef::Var(b)), None) => Some(b),
            _ => None,
        },
        _ => None,
    };
    rule.body()
        .filter_map(|l| match l.view() {
            Lit::Cmp { l: TermRef::Var(a), op: CmpOp::Eq, r: TermRef::Var(b) } => Some((a, b)),
            Lit::Cmp { l: TermRef::Var(a), op: CmpOp::Eq, r } => doc_of(r).map(|b| (a, b)),
            // `@doc(a) = @doc(b)`: a and b are of one document (a rule written for one file's nodes).
            Lit::Cmp { l, op: CmpOp::Eq, r } => doc_of(l).zip(doc_of(r)),
            _ => None,
        })
        .collect()
}

/// Adds to `vars` every variable equal to one in it, to a fixpoint.
fn close_under<T: Ord + Clone>(vars: &mut std::collections::BTreeSet<T>, equal: &[(T, T)]) {
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

/// Whether a pushed local join exports a column bound by equality (`RERAIL_PUSH_EQUAL_COLUMNS`, on unless `off`): public, as
/// the split's tests read it in every crate that compiles them (re-exported whole).
pub fn push_equal_columns() -> bool {
    std::env::var("RERAIL_PUSH_EQUAL_COLUMNS").map_or(true, |value| value != "off")
}

pub use crate::code_rule_dsl::logic::front::document_column;

/// Base relations whose rows a chunk makes for itself and that name no document (decision 0058 §4).
const CHUNK_VALUES: [&str; 3] = ["parse_of", "unparse_of", "splice_of"];

use std::collections::BTreeMap;

/// How deep the request narrowing (`RequestsFileLocal`) unfolds cross-file relations, and how many alternative bodies a request may have.
const UNFOLD: usize = 3;
const ALTERNATIVES: usize = 8;

/// A program split at its file-local relations (`FileLocal`): `local` derives them, run per chunk of documents over the
/// chunk's base relations; `cross` derives the rest, run once over what the chunks wrote: `exports` (the file-local
/// relations a cross-file rule reads) and `base` (the base relations a cross-file rule reads, and `node` and `doc`,
/// which decode findings). In `cross`, both are inputs. `same_doc` names, for each file-local and base relation, the
/// columns whose values in one row lie in one document (a join through them crosses no file: the cross memo's guards).
#[derive(Clone)]
pub struct Programs {
    pub exports: Vec<String>,
    pub base: Vec<String>,
    /// Each derived relation's location columns (`FileLocal::locations`).
    pub locations: std::collections::BTreeMap<String, Vector<usize>>,
    /// The same-document columns of each file-local or base relation (`FileLocal::same_doc`): what a cross-file hop reads.
    pub same_doc: std::collections::BTreeMap<String, Vector<usize>>,
    /// What the split found, and the parts are made of (`assemble`): the file-local relations, and the relations of
    /// constants derived in both parts. A prepared program keeps these, not the two parts: each is the program's own
    /// rules, a part of them, so the parts are made from the program again when it is read.
    pub file_local: std::collections::BTreeSet<String>,
    pub constants: std::collections::BTreeSet<String>,
    /// The parts' versions (decision 0244): each a view of the program's version, its fields overlaid. No part has a
    /// tree (decision 0281's path, step 4).
    pub versions: Parts,
    /// The program's version the parts are views of: what preparation's analyses of the whole program read (the
    /// static check, decision 0364 §5: a part holds only some of the rules a step reads through).
    pub program: std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>,
    /// Why each derived relation is cross-file (`cross_causes`), as the split found it; empty for a split read from
    /// the store (`assemble` of what a prepared program kept), which keeps no causes.
    pub causes: std::collections::BTreeMap<String, String>,
}

/// The versions of a split's parts (`Programs::versions`): the file-local and the cross-file part.
#[derive(Clone)]
pub struct Parts {
    pub local: std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>,
    pub cross: std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>,
}

/// The split of the program `version` is (decision 0281's path, step 5): what it found read from the version
/// (`FileLocal`, an analysis of it), and the parts views of it.
pub fn programs_of(version: std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>) -> Result<Programs, String> {
    let split = crate::code_rule_dsl::logic::program_image::manager::analysis::<FileLocal>(&version)?;
    let (local, constants) = split.found();
    let mut programs = assemble(version, local, constants, split.locations.clone(), split.same_doc.clone())?;
    programs.causes = split.causes.clone();
    Ok(programs)
}

/// `prog` split at its file-local relations `local` (`programs_of`), deriving the relations of `constants` in both parts:
/// the parts and what passes between them. What is costly to find is the arguments; this only selects (`programs_of`
/// finds them, a prepared program keeps them).
pub fn assemble(
    version: std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>,
    local: std::collections::BTreeSet<String>,
    constants: std::collections::BTreeSet<String>,
    locations: std::collections::BTreeMap<String, Vector<usize>>,
    same_doc: std::collections::BTreeMap<String, Vector<usize>>,
) -> Result<Programs, String> {
    let placed = crate::code_rule_dsl::logic::plan::body::Placed::whole(version.clone());
    let f = placed.fields().map_err(|e| e.to_string())?;
    let on_version::Crossing { exports, base_read, base, projected } = on_version::crossing(&version, f, &local);
    let is_local = |rel: &str| local.contains(rel);
    let in_cross = |rel: &str| !is_local(rel) || projected.contains(rel);
    // Each part's inputs and outputs; its rules and other fields are a view of the program's version (below), an
    // overlay over it (decision 0244).
    let local_outputs: Vec<String> = f.outputs().filter(|o| is_local(o)).map(str::to_owned).chain(exports.iter().cloned()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    let local_inputs: Vec<String> = f.inputs().map(str::to_owned).collect();
    // An output the cross part reads too (an export: a pack's `rr_found_through` over a file-local finding) is handed
    // on by it, read where its input is bound: the file-local part keeps its exports in the engine for the cross part,
    // not decoded, so an exported output is the cross part's to write.
    let cross_outputs: Vec<String> = f.outputs().filter(|o| !is_local(o) || exports.iter().any(|x| x == o)).map(str::to_owned).collect();
    let cross_inputs: Vec<String> = base_read.into_iter().chain(exports.iter().cloned()).collect();
    let versions = {
        use crate::code_rule_dsl::logic::program_image::PartFields;
        let local_rule = |rel: &str| is_local(rel) || constants.contains(rel);
        let not_local = |rel: &str| !is_local(rel);
        let local = version.part(local_rule, &local_inputs, &local_outputs, PartFields { natives: &is_local, lattice: &is_local, gates: &is_local })?;
        let cross = version.part(in_cross, &cross_inputs, &cross_outputs, PartFields { natives: &in_cross, lattice: &not_local, gates: &not_local })?;
        Parts { local, cross }
    };
    crate::code_rule_dsl::logic::version_trace::note("split.local", &version, &versions.local);
    crate::code_rule_dsl::logic::version_trace::note("split.cross", &version, &versions.cross);
    // The split's constants are the file-local part's own (its split's, read from its version): checked in a debug
    // build, as `corpus` reads `constants` for the part.
    #[cfg(debug_assertions)]
    {
        let part = crate::code_rule_dsl::logic::program_image::manager::analysis::<FileLocal>(&versions.local)?;
        for r in exports.iter().chain(local_outputs.iter()) {
            assert_eq!(part.constants.contains(r.as_str()), constants.contains(r.as_str()), "the split's constants and the file-local part's disagree on {r}");
        }
    }
    Ok(Programs { exports, base, locations, same_doc, file_local: local, constants, versions, program: version, causes: Default::default() })
}

/// Whether the factorized store is on (decision 0099): it is, unless `RERAIL_FACTORIZE=off` or the run's option
/// `factorize: off` turns it off, for comparison. The prepared program's key names it (`corpus`'s `prepared_key`).
pub fn factorize_on(options: &serde_json::Value) -> bool {
    std::env::var("RERAIL_FACTORIZE").map_or(true, |v| v != "off") && options.get("factorize").and_then(|v| v.as_str()) != Some("off")
}

/// A helper relation's name from its content (decision 0281's path, step 5): `<prefix><h>_<rel>`, `h` a stable hash of
/// the text of the rule that makes it (its head's relation and interface, its body as the tree writes it), so an
/// edit renames only the helpers of the rules it changes, where a counter renumbered every helper after them. The
/// digits keep the shape the profile's and the run's readers parse (`fl<digits>_<head>`). A name the program already
/// declares (`existing`), or one this round gave other content (`used`), is hashed again with a salt.
pub fn helper_name(prefix: &str, rel: &str, content: &str, existing: impl Fn(&str) -> bool, used: &mut std::collections::HashMap<String, String>) -> String {
    let mut salt = 0u32;
    loop {
        // FNV-1a over the salt and the text: the same in every process and build.
        let mut h: u32 = 0x811c_9dc5;
        for b in salt.to_le_bytes().iter().chain(content.as_bytes()) {
            h ^= u32::from(*b);
            h = h.wrapping_mul(0x0100_0193);
        }
        let name = format!("{prefix}{h}_{rel}");
        let free = match used.get(&name) {
            Some(c) => c == content,
            None => !existing(&name),
        };
        if free {
            used.insert(name.clone(), content.to_owned());
            return name;
        }
        salt += 1;
    }
}

/// At most this many rules of a relation stored factorized, and reader copies made of its reads.
const FACTOR_RULES: usize = 4;
const FACTOR_COPIES: usize = 8;

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
pub fn semijoin_bases(cross: &crate::code_rule_dsl::logic::program_image::Version, base: &[String]) -> Option<std::collections::BTreeSet<String>> {
    use crate::code_rule_dsl::logic::program_image::{AtomRef, Lit, Slot, TermRef};
    use std::collections::{BTreeMap, BTreeSet};
    let v = cross;
    let name = |a: AtomRef<'_>| v.rel_name(a.rel());
    let types_of = |rel: &str| -> Option<Vec<&str>> { v.decl_of(rel).map(|d| d.cols().map(|(_, t)| v.text(t)).collect()) };
    // Each attribute relation's node column: the base ones, and a view the cross part derives from one
    // (`kind($n, $k) :- node(_, $n, $k, _, _)`), keyed where the node lands in its head.
    let mut key: BTreeMap<String, usize> = BTreeMap::new();
    for rel in base {
        let (Some(columns), Some(types)) = (crate::code_rule_dsl::logic::sql::select::input_columns(rel), types_of(rel)) else { continue };
        let Some(k) = columns.iter().position(|c| *c == "n") else { continue };
        // Another column of nodes: by its type, or by its name where the type is a number (`tape`'s parent `par`, an
        // ancestor `a`, `ast_match`'s pattern root `p`, `host_node`'s host `h`); a walk up `tape(p, par)` reaches
        // parents no export holds.
        let others_nodes = types.iter().enumerate().any(|(i, t)| i != k && (matches!(*t, "node" | "loc" | "span") || matches!(columns[i], "par" | "a" | "p" | "h")));
        if !others_nodes && columns.len() == types.len() {
            key.insert(rel.clone(), k);
        }
    }
    // A view's base: a view is a semi-join exactly when its base is.
    let mut view_of: BTreeMap<String, String> = BTreeMap::new();
    let mut heads: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for rule in v.rules() {
        *heads.entry(name(rule.head())).or_default() += 1;
    }
    for rule in v.rules() {
        let mut body = rule.body();
        let (Some(only), None) = (body.next(), body.next()) else { continue };
        let Lit::Atom(a) = only.view() else { continue };
        let Some(TermRef::Var(n)) = key.get(name(a)).and_then(|&k| a.args().nth(k)).filter(|_| !a.neg()) else { continue };
        let head = rule.head();
        let Some(at) = head.args().position(|t| t.var() == Some(n)) else { continue };
        let sole = heads.get(name(head)) == Some(&1);
        if sole && !key.contains_key(name(head)) && !view_of.contains_key(name(a)) {
            key.insert(name(head).to_owned(), at);
            view_of.insert(name(head).to_owned(), name(a).to_owned());
        }
    }
    let root = |rel: &str| view_of.get(rel).cloned().unwrap_or_else(|| rel.to_owned());
    // A relation with no node column reaches no node.
    let reaches_nodes = |rel: &str| types_of(rel).is_none_or(|ts| ts.iter().any(|t| matches!(*t, "node" | "loc" | "span")));
    let is_base = |rel: &str| base.iter().any(|b| b == rel);
    let about = crate::code_rule_dsl::logic::program_image::About::of(v);
    let natives = &about.natives;
    let outputs: Vec<&str> = v.outputs().map(|a| v.text(a)).collect();
    // The attributes still taken as semi-joins (by their base relation), and the relations whose nodes may not trace
    // back to an export.
    let mut semi: BTreeSet<String> = key.keys().filter(|r| !view_of.contains_key(*r)).cloned().collect();
    let mut tainted: BTreeSet<String> = BTreeSet::new();
    loop {
        let (semi_before, tainted_before) = (semi.len(), tainted.len());
        // A source of tainted nodes: a base relation that reaches nodes and is no attribute, or a tainted relation.
        let source = |a: AtomRef<'_>, tainted: &BTreeSet<String>| !a.neg() && !key.contains_key(name(a)) && (tainted.contains(name(a)) || (is_base(name(a)) && reaches_nodes(name(a))));
        for native in natives.iter() {
            for r in native.reads.iter().map(|r| v.rel_name(*r)) {
                if key.contains_key(r) {
                    semi.remove(&root(r));
                }
                if tainted.contains(r) || (is_base(r) && reaches_nodes(r)) || key.contains_key(r) {
                    tainted.insert(v.rel_name(native.rel).to_owned());
                }
            }
        }
        for rule in v.rules().filter(|r| !view_of.contains_key(name(r.head()))) {
            // Held variables: bound by a positive read of a relation that is no attribute and holds no tainted node.
            let positive: Vec<AtomRef<'_>> = rule.body().filter_map(|l| if let Lit::Atom(a) = l.view() { Some(a) } else { None }).filter(|a| !a.neg()).collect();
            let vars = |a: AtomRef<'_>| a.args().filter_map(TermRef::var).collect::<Vec<Slot>>();
            // A variable equal to a held one is held; one equal to a loose one is loose (`m = n`).
            let equal = var_equalities_in(v, rule);
            let mut held: BTreeSet<Slot> = positive.iter().filter(|a| !key.contains_key(name(**a)) && !source(**a, &tainted)).flat_map(|a| vars(*a)).collect();
            close_under(&mut held, &equal);
            let mut loose: BTreeSet<Slot> = positive.iter().filter(|a| source(**a, &tainted)).flat_map(|a| vars(*a)).collect();
            for l in rule.body() {
                let (a, in_agg) = match l.view() {
                    Lit::Atom(a) => (a, false),
                    Lit::Agg { atom: a, .. } => (a, true),
                    _ => continue,
                };
                let Some(&k) = key.get(name(a)) else { continue };
                match a.args().nth(k) {
                    Some(TermRef::Var(s)) if held.contains(&s) => {}
                    found => {
                        // A lookup of a node that may not be held, or an enumeration: the attribute is kept whole.
                        semi.remove(&root(name(a)));
                        // Its node, not held, is tainted or new (an enumeration).
                        if let (Some(TermRef::Var(s)), false, false) = (found, a.neg(), in_agg) {
                            loose.insert(s);
                        }
                    }
                }
            }
            close_under(&mut loose, &equal);
            let head = rule.head();
            let head_tainted = head.args().any(|t| t.var().is_some_and(|s| loose.contains(&s) && !held.contains(&s)));
            if head_tainted {
                tainted.insert(name(head).to_owned());
            }
        }
        if outputs.iter().any(|o| tainted.contains(*o)) {
            semi.remove("node");
        }
        if semi.len() == semi_before && tainted.len() == tainted_before {
            break;
        }
    }
    Some(semi)
}

/// The qualifier `locate_all` gives a relation whose rows name no document: the split holds it cross-file (decision 0290).
pub const CROSS_FILE: &str = "cross_file";

/// The relations `locate_all` moved to the cross-file part (`CROSS_FILE`), read from the program's version.
pub fn moved_in<'v>(f: &crate::code_rule_dsl::logic::plan::fields::Fields<'v>) -> Vec<&'v str> {
    f.decls_by_name().into_iter().map(|(r, _)| r).filter(|r| f.qualified(r, CROSS_FILE)).collect()
}

