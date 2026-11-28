//! The delta laws the boot's run and L1 keep, as drivers on any bench (0686: Shape and Invariants, §2 and §4), so the
//! CPU's tests and the device's run the same checks: L6 (composition and cutoff), L7 (partition), L8 (edits coalesce)
//! and P1 (cost follows the deltas). Each returns what its test asserts on; a broken law is an error naming it.

use crate::boot::DocSet;
use crate::contract::{Bench, GlobalIn, Instance, Work, apply, oracle_rows};
use crate::files::{Store, World};
use crate::kform::RELATIONS;
use crate::l1::L1;
use crate::sym::Atoms;
use rerail_blocks::bistate::{ColumnIndex, Read};
use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, SystemTime};

type Rows = Vec<BTreeSet<Vec<i64>>>;

/// Writes `text` at `path`, its modification time `ago` seconds back (past the racy window).
fn write(path: &Path, text: &str, ago: u64) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| e.to_string())?;
    let f = std::fs::File::options().write(true).open(path).map_err(|e| e.to_string())?;
    f.set_modified(SystemTime::now() - Duration::from_secs(ago)).map_err(|e| e.to_string())
}

/// L1's K-form at its outputs' heads.
fn outs(bench: &Bench, inst: &Instance) -> Result<Rows, String> {
    inst.outs.iter().map(|t| oracle_rows(&bench.dir, t, t.head())).collect()
}

/// The composed reference (L6): l0's boot run cold on `text`, from nothing (a root, a store and an instance of its own
/// under `base`), by the same atoms, so its rows read as a threaded run's: its K-form.
pub fn cold_on(bench: &mut Bench, atoms: Atoms, base: &Path, text: &str) -> Result<Rows, String> {
    const PATH: &str = "bootstrap-l0.rerail";
    let root = base.join("root");
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    write(&root.join(PATH), text, 600)?;
    let world = World::new(root, Store::open(&base.join("store"))?, &bench.dir, [None; 3])?;
    let mut inst = Instance::new(&L1, &mut bench.dir)?;
    let mut docs = DocSet::new(world, &bench.dir, None, None)?;
    crate::boot::run(&L1, &mut inst, PATH, "bootstrap-l0", atoms, &mut docs, bench, None)?;
    outs(bench, &inst)
}

/// What L6's driver saw.
pub struct Composed {
    /// Per version: whether the threaded run's K-form is the cold run's on its text.
    pub same_as_cold: Vec<bool>,
    /// The last version run again unchanged (which takes any roll-up the edit before left), then touched: its bytes
    /// unchanged under a new modification time.
    pub unchanged: crate::boot::Ran,
    pub touched: crate::boot::Ran,
}

/// L6 on `bench`: the boot run threaded over `versions` (`boot_chain`) is, run by run, the composed plain functions
/// (load, parse, intern, L1) run cold on that version's text; and a stage whose input comes out the identity cuts off:
/// the last version touched (its bytes unchanged, a new modification time) is loaded, found the same content and not
/// parsed, and L1, given the identity, does checks alone.
pub fn composition_and_cutoff(bench: &mut Bench, base: &Path, versions: &[&str]) -> Result<Composed, String> {
    let mut all: Vec<&str> = versions.to_vec(); // copy: not block data (the chain's versions)
    let last = versions.last().ok_or("a version")?;
    all.extend([last, last]);
    let mut runs = crate::boot::boot_chain_touched(bench, &base.join("chain"), &all, &[versions.len() + 1])?;
    let touched = runs.pop().expect("the touched run");
    let unchanged = runs.pop().expect("the unchanged run");
    let atoms = Atoms(runs[0].after.atoms.block);
    let mut same_as_cold = Vec::with_capacity(runs.len()); // copy: not block data (the test's verdicts)
    for (k, (r, v)) in runs.iter().zip(versions).enumerate() {
        same_as_cold.push(r.kform == cold_on(bench, atoms, &base.join(format!("cold-{k}")), v)?);
    }
    Ok(Composed { same_as_cold, unchanged, touched })
}

/// What one edit among several documents did (L7, P1): L1's step over the set's change, alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditAmong {
    /// L1's work and launches for the step.
    pub work: Work,
    pub launches: u64,
    /// The world's reads, loads and parses, and its launches, reading the set again (every document's stat).
    pub asked: crate::files::Asked,
    pub read_launches: u64,
}

/// The edited module's two versions, and the `i`-th unchanged module's text.
const EDITED: [&str; 2] = ["e(1, 2).\np(x) :- e(x, _).\n", "e(1, 2).\np(x) :- e(x, _).\nq(x) :- p(x), !e(x, 1).\n"];
fn other(i: usize) -> String {
    format!("e({i}, {}).\np(x) :- e(x, _), !q(x).\nr(x, y) :- e(x, y), p(y).\n", i + 1)
}

/// L7 and P1 on `bench`: a set of `others` unchanged modules and one edited one, read and stepped cold, then the edit
/// read and stepped. Refused where L7 breaks: an output's changed partitions other than the edited module's, an
/// unchanged module's rows moved, or L1's output not the plain function's on the set. What the edit's step cost.
pub fn edit_among(bench: &mut Bench, base: &Path, others: usize) -> Result<EditAmong, String> {
    let root = base.join("root");
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let paths: Vec<(String, String)> = std::iter::once(("e.rerail".to_string(), "e".to_string())).chain((0..others).map(|i| (format!("m{i}.rerail"), format!("m{i}")))).collect();
    write(&root.join("e.rerail"), EDITED[0], 600)?;
    for i in 0..others {
        write(&root.join(format!("m{i}.rerail")), &other(i), 600)?;
    }
    let atoms = Atoms::new(&mut bench.dir);
    let world = World::new(root.clone(), Store::open(&base.join("store"))?, &bench.dir, [None; 3])?;
    let mut docs = DocSet::new(world, &bench.dir, None, None)?;
    let mut inst = Instance::new(&L1, &mut bench.dir)?;
    // Every document read (its stat; loaded, parsed and interned when new or changed), then L1 stepped over the set.
    let read_all = |bench: &mut Bench, docs: &mut DocSet| -> Result<(), String> {
        // A run of the world (decision 0105): the journal asked once what changed under the root, the rest trusted.
        docs.world.begin();
        let mut leaves = docs.world.leaves_w(&mut bench.dir);
        let named: Vec<(&str, &str)> = paths.iter().map(|(p, m)| (p.as_str(), m.as_str())).collect();
        docs.read_all(bench, &mut leaves, atoms, &named)?;
        docs.world.commit_leaves(&bench.ops, &mut bench.dir, leaves)?;
        docs.world.end();
        docs.resident(bench)
    };
    let step = |bench: &mut Bench, docs: &DocSet, inst: &mut Instance, from: (Option<crate::dir::BlockId>, Option<u64>)| -> Result<Work, String> {
        let count = u64::from(atoms.count(&bench.dir));
        inst.step(&L1, &mut bench.cx(), &[(&docs.tape, from.0, docs.tape.head())], &[GlobalIn { block: atoms.0, before: from.1, after: count }], None)
    };
    read_all(bench, &mut docs).map_err(|e| format!("the cold read: {e}"))?;
    step(bench, &docs, &mut inst, (None, None)).map_err(|e| format!("the cold step: {e}"))?;
    let (h1, c1, o1) = (docs.tape.head(), u64::from(atoms.count(&bench.dir)), inst.heads());
    // The outputs' befores, held as a reader holds a generation: the step may roll them up.
    let _held: Vec<_> = o1.iter().flatten().filter_map(|&h| bench.dir.block(h)).collect();
    let rows1 = outs(bench, &inst)?;
    // The edit.
    write(&root.join("e.rerail"), EDITED[1], 300)?;
    let (asked0, launched) = (docs.world.asked, bench.ops.launches());
    read_all(bench, &mut docs).map_err(|e| format!("the edit's read: {e}"))?;
    let a = docs.world.asked;
    let asked = crate::files::Asked { reads: a.reads - asked0.reads, loads: a.loads - asked0.loads, parses: a.parses - asked0.parses, interns: a.interns - asked0.interns };
    let read_launches = bench.ops.launches() - launched;
    let launched = bench.ops.launches();
    let work = step(bench, &docs, &mut inst, (h1, Some(c1))).map_err(|e| format!("the edit's step: {e}"))?;
    let launches = bench.ops.launches() - launched;
    // L7: only the edited module's partitions changed, in every output.
    let e = docs.key(&bench.dir, atoms, "e.rerail")?;
    for (k, (t, before)) in inst.outs.iter().zip(&o1).enumerate() {
        let l = t.read(&bench.dir, &[*before, t.head()])?;
        let table = l.table();
        let b = t.change(&table, *before, t.head())?;
        let mut parts = Vec::new(); // copy: derived (the changed partitions, the answer)
        b.changed_parts(&mut |p| parts.push(p));
        if parts.iter().any(|&p| p != e) {
            return Err(format!("L7: {}'s changed partitions {parts:?}, not the edited module's ({e}) alone", RELATIONS[k].0));
        }
    }
    let rows2 = outs(bench, &inst)?;
    for (k, (r1, r2)) in rows1.iter().zip(&rows2).enumerate() {
        let (a, b): (BTreeSet<_>, BTreeSet<_>) = (r1.iter().filter(|r| r[0] != e).collect(), r2.iter().filter(|r| r[0] != e).collect());
        if a != b {
            return Err(format!("L7: {}'s rows of the unchanged modules moved", RELATIONS[k].0));
        }
    }
    let globals = [crate::contract::Global { block: atoms.0, count: u64::from(atoms.count(&bench.dir)) }];
    if rows2 != apply(&L1, &mut bench.cx(), &[(&docs.tape, docs.tape.head())], &globals)? {
        return Err("L1: the output is not the plain function's on the set".into());
    }
    Ok(EditAmong { work, launches, asked, read_launches })
}

/// What L8's driver saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coalesced {
    /// The two edits stepped once (l0 → added → changed, read as one change): its output and work, against the two
    /// steps' output.
    pub once_is_twice: bool,
    pub once: Work,
    /// The least change of the output across the two edits, by value: what the one step must write.
    pub least: u64,
    /// An edit and its undo stepped once (l0 → added → l0): the work, and whether the output stayed as it was.
    pub undo: Work,
    /// Its output rows as at l0, and its output's change the identity (a bistate between the heads before and after).
    pub undo_rows_same: bool,
    pub undo_out_identity: bool,
    /// The input's change across the edit and its undo: the identity by its counts, and by value (its rows gone and
    /// added the same values).
    pub undo_in_identity: (bool, bool),
}

/// L8 on `bench`: l0's document set read at l0, with a rule added, with that rule's neighbour changed, and at l0
/// again; L1 stepped edit by edit, and over each span as one change.
pub fn coalescing(bench: &mut Bench, base: &Path) -> Result<Coalesced, String> {
    const PATH: &str = "bootstrap-l0.rerail";
    let root = base.join("root");
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let atoms = Atoms::new(&mut bench.dir);
    let world = World::new(root.clone(), Store::open(&base.join("store"))?, &bench.dir, [None; 3])?;
    let mut docs = DocSet::new(world, &bench.dir, None, None)?;
    let [added, changed, _] = crate::boot::chain_edits();
    let mut heads = vec![(None, None)]; // copy: not block data (the set's states)
    for (k, text) in [crate::L0, added.as_str(), changed.as_str(), crate::L0].into_iter().enumerate() {
        write(&root.join(PATH), text, 600 - 10 * k as u64)?;
        let mut leaves = docs.world.leaves_w(&mut bench.dir);
        docs.read(bench, &mut leaves, atoms, PATH, "bootstrap-l0")?;
        docs.world.commit_leaves(&bench.ops, &mut bench.dir, leaves)?;
        heads.push((docs.tape.head(), Some(u64::from(atoms.count(&bench.dir)))));
    }
    docs.resident(bench)?;
    let count = u64::from(atoms.count(&bench.dir));
    // Each step from state `i` to state `j` (the atoms at the last state's count: append-only, every name kept).
    let step = |bench: &mut Bench, inst: &mut Instance, i: usize, j: usize| -> Result<Work, String> {
        let ((hi, ci), (hj, _)) = (heads[i], heads[j]);
        inst.step(&L1, &mut bench.cx(), &[(&docs.tape, hi, hj)], &[GlobalIn { block: atoms.0, before: ci, after: count }], None)
    };
    let mut twice = Instance::new(&L1, &mut bench.dir)?;
    step(bench, &mut twice, 0, 1)?;
    let at_l0 = outs(bench, &twice)?;
    step(bench, &mut twice, 1, 2)?;
    step(bench, &mut twice, 2, 3)?;
    let at_changed = outs(bench, &twice)?;
    let mut once = Instance::new(&L1, &mut bench.dir)?;
    step(bench, &mut once, 0, 1)?;
    let once_work = step(bench, &mut once, 1, 3)?;
    let least = at_l0.iter().zip(&at_changed).map(|(a, b)| a.symmetric_difference(b).count() as u64).sum();
    let once_is_twice = outs(bench, &once)? == at_changed;
    let mut undo = Instance::new(&L1, &mut bench.dir)?;
    step(bench, &mut undo, 0, 1)?;
    let o = undo.heads();
    let _held: Vec<_> = o.iter().flatten().filter_map(|&h| bench.dir.block(h)).collect();
    let undo_work = step(bench, &mut undo, 1, 4)?;
    let undo_rows_same = outs(bench, &undo)? == at_l0;
    let mut undo_out_identity = true;
    for (t, before) in undo.outs.iter().zip(&o) {
        let l = t.read(&bench.dir, &[*before, t.head()])?;
        let table = l.table();
        undo_out_identity &= t.change(&table, *before, t.head())?.is_identity();
    }
    let l = docs.tape.read(&bench.dir, &[heads[1].0, heads[4].0])?;
    let table = l.table();
    let b = docs.tape.change(&table, heads[1].0, heads[4].0)?;
    let row = |s: &rerail_blocks::relation::View<'_>, id| -> Vec<i64> { ColumnIndex::all(Read::arity(s)).map(|c| Read::get(s, id, c)).collect() };
    let gone: BTreeSet<Vec<i64>> = b.gone().map(|id| row(b.before().expect("a before"), id)).collect();
    let added: BTreeSet<Vec<i64>> = b.added().map(|id| row(b.after(), id)).collect();
    let undo_in_identity = (b.is_identity(), gone == added);
    Ok(Coalesced { once_is_twice, once: once_work, least, undo: undo_work, undo_rows_same, undo_out_identity, undo_in_identity })
}

/// D5 on `bench` (P§7.1, demand reaches the world as reads): l0 and another document under a root; a run naming l0,
/// then one naming the other, then l0 again, each from the last's pins. The second asks for the other alone and lets
/// l0 go: l0 is not read, its row leaves the document set (X → ∅) and its rows leave L1's output. The third brings l0
/// back as an initial run of it, from the store (its content answered before: not parsed again). Every run's output is
/// the plain function's on the set as it is. Refused naming what broke.
pub fn demand_moves(bench: &mut Bench, base: &Path) -> Result<(), String> {
    const L0: &str = "bootstrap-l0.rerail";
    let root = base.join("root");
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    write(&root.join(L0), crate::L0, 600)?;
    write(&root.join("other.rerail"), &crate::boot::chain_edits()[0], 600)?;
    let atoms = Atoms::new(&mut bench.dir);
    let world = World::new(root, Store::open(&base.join("store"))?, &bench.dir, [None; 3])?;
    let mut docs = DocSet::new(world, &bench.dir, None, None)?;
    let mut inst = Instance::new(&L1, &mut bench.dir)?;
    let keys = |bench: &Bench, t: &crate::trace::Trace| -> Result<BTreeSet<i64>, String> { Ok(oracle_rows(&bench.dir, t, t.head())?.into_iter().map(|r| r[0]).collect()) };
    let plain = |bench: &mut Bench, docs: &DocSet, inst: &Instance| -> Result<(), String> {
        let globals = [crate::contract::Global { block: atoms.0, count: u64::from(atoms.count(&bench.dir)) }];
        if outs(bench, inst)? != apply(&L1, &mut bench.cx(), &[(&docs.tape, docs.tape.head())], &globals)? {
            return Err("L1's output is not the plain function's on the document set".into());
        }
        Ok(())
    };
    let (pins, ..) = crate::boot::run(&L1, &mut inst, L0, "bootstrap-l0", atoms, &mut docs, bench, None)?;
    let (reads, l0) = (docs.world.asked.reads, docs.key(&bench.dir, atoms, L0)?);
    let (pins, doc, _) = crate::boot::run(&L1, &mut inst, "other.rerail", "other", atoms, &mut docs, bench, Some(&pins))?;
    let other = docs.key(&bench.dir, atoms, "other.rerail")?;
    if doc.is_identity() || other == l0 {
        return Err("the other document read as a new one".into());
    }
    if docs.world.asked.reads != reads + 1 {
        return Err(format!("D5: {} reads, not the other's alone (l0, asked by no one, read)", docs.world.asked.reads - reads));
    }
    if keys(bench, &docs.tape)? != BTreeSet::from([other]) {
        return Err("D5: l0 not gone from the document set".into());
    }
    if keys(bench, inst.asks_of(0).ok_or("L1 asks of its tape port")?)? != BTreeSet::from([other]) {
        return Err("D5: L1 asks for more than the other".into());
    }
    if outs(bench, &inst)?.iter().flatten().any(|r| r[0] != other) {
        return Err("D5: a row of l0 left in L1's output".into());
    }
    plain(bench, &docs, &inst)?;
    let parses = docs.world.asked.parses;
    crate::boot::run(&L1, &mut inst, L0, "bootstrap-l0", atoms, &mut docs, bench, Some(&pins))?;
    if keys(bench, &docs.tape)? != BTreeSet::from([l0]) || docs.world.asked.parses != parses {
        return Err("D5: l0 asked again is not back, from the store, with no parse".into());
    }
    let o = outs(bench, &inst)?;
    if o[0].len() != 14 || o.iter().flatten().any(|r| r[0] != l0) {
        return Err("D5: l0 asked again is not its 14 rules alone".into());
    }
    plain(bench, &docs, &inst)
}
