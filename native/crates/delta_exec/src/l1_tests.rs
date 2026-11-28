//! B6's checks: L1 over a copy of l0 on disk.

use super::*;
use crate::L0;
use crate::boot::{DocSet, Pins};
use crate::contract::{Bench, Global, Instance, oracle_rows, reference};
use crate::dir::Directory;
use crate::files::{Doc, Store, World};
use crate::files::tests::{past_racy_window, racy_window, root, write};
use crate::sym::Atoms;
use rerail_blocks::maps::hash_bytes;

/// The keys (column 0) of `t`'s rows at its head: the oracle's, by meaning.
fn keys(dir: &Directory, t: &crate::trace::Trace) -> BTreeSet<i64> {
    oracle_rows(dir, t, t.head()).unwrap().into_iter().map(|r| r[0]).collect()
}
use std::collections::BTreeSet;
use std::path::PathBuf;

const PATH: &str = "bootstrap-l0.rerail";
const MODULE: &str = "bootstrap-l0";

/// L1 over l0 on disk: a root holding a copy of l0, the document set the file layer reads, the atoms (the run's
/// globals), L1 and its instance, and the last run's pins.
struct Disk {
    root: PathBuf,
    docs: DocSet,
    atoms: Atoms,
    l1: L1,
    inst: Instance,
    last: Option<Pins>,
}

impl Disk {
    fn new(bench: &mut Bench, name: &str) -> Disk {
        let (root, store) = root(name);
        let atoms = Atoms::new(&mut bench.dir);
        let world = World::new(root.clone(), store, &bench.dir, [None; 3]).unwrap();
        Disk::over(bench, root, world, atoms, None)
    }

    /// L1 over the document set of `world`, at `pins` and the pins block they lie in (a kept run's; `None`: none).
    fn over(bench: &mut Bench, root: PathBuf, world: World, atoms: Atoms, pins: Option<(BlockId, Pins)>) -> Disk {
        let l1 = L1;
        let place = bench.dir.lasting();
        let inst = Instance::new(&l1, &mut bench.dir).unwrap().resumed(place, pins.as_ref().map_or(&[][..], |p| &p.1.outs), pins.as_ref().and_then(|p| p.1.want), pins.as_ref().map_or(&[][..], |p| &p.1.asks));
        let docs = DocSet::new(world, &bench.dir, pins.as_ref().and_then(|p| p.1.tape), pins.as_ref().map(|p| p.0)).unwrap();
        let last = pins.map(|(_, p)| Pins { atoms: Global { block: atoms.0, ..p.atoms }, ..p });
        Disk { root, docs, atoms, l1, inst, last }
    }

    /// B§8's run, its first two lines, from `last`: its pins.
    fn run_from(&mut self, bench: &mut Bench, last: Option<&Pins>) -> Result<(Pins, Doc, Work), String> {
        crate::boot::run(&self.l1, &mut self.inst, PATH, MODULE, self.atoms, &mut self.docs, bench, last)
    }

    /// B§8's run from the last run's pins, its pins kept as the last.
    fn run(&mut self, bench: &mut Bench) -> Result<(Doc, Work), String> {
        let last = self.last.take();
        let (pins, doc, work) = self.run_from(bench, last.as_ref())?;
        self.last = Some(pins);
        Ok((doc, work))
    }

    /// The atoms at their count now.
    fn globals(&self, bench: &Bench) -> Global {
        Global { block: self.atoms.0, count: u64::from(self.atoms.count(&bench.dir)) }
    }

    fn edit(&self, text: &str, ago: u64) {
        write(&self.root.join(PATH), text, ago);
    }
}

/// l0 with a rule added, a rule changed and a rule removed.
fn edits() -> [String; 3] {
    crate::boot::chain_edits()
}

#[test]
fn the_harness_passes_l1_over_edits_of_l0_on_disk() {
    let mut bench = Bench::new().unwrap();
    let (root, _) = root("b6-chain");
    crate::boot::harness_over_edits(&mut bench, root.parent().unwrap(), false, 1..=3).unwrap_or_else(|e| panic!("{e}"));
}

/// D1 on L1 over the same chain of edits on disk: under a want that names all of l0's relations, two of them, none,
/// in turn, its output restricted to the want is the full output restricted (its grain is the document: a wanted
/// relation of l0 is all of l0's, over-demand); l0 asked of its tape port while wanted, and on the inputs it asks, the
/// same wanted output; a newly wanted l0 an initial run of it, and one no longer wanted its rows retracted.
#[test]
fn the_harness_passes_l1_under_a_changing_want() {
    let mut bench = Bench::new().unwrap();
    let (root, _) = root("b6-want");
    crate::boot::harness_over_edits(&mut bench, root.parent().unwrap(), true, 4..=5).unwrap_or_else(|e| panic!("{e}"));
}

/// Demand reaches the world as reads (P§7.1): a run whose productions name another document asks for it and lets l0
/// go: l0 is not read, its row leaves the document set (X → ∅) and its rows leave L1's output, which is the plain
/// function's on the set as it now is.
#[test]
fn a_document_no_longer_asked_goes_and_is_not_read() {
    let _racy = racy_window();
    let mut bench = Bench::new().unwrap();
    let mut disk = Disk::new(&mut bench, "b6-demand");
    write(&disk.root.join("other.rerail"), &edits()[0], 60);
    past_racy_window();
    disk.run(&mut bench).unwrap();
    let (reads, l0) = (disk.docs.world.asked.reads, disk.docs.key(&bench.dir, disk.atoms, PATH).unwrap());
    let last = disk.last.take();
    let (pins, doc, _) = crate::boot::run(&disk.l1, &mut disk.inst, "other.rerail", "other", disk.atoms, &mut disk.docs, &mut bench, last.as_ref()).unwrap();
    disk.last = Some(pins);
    assert!(!doc.is_identity());
    assert_eq!(disk.docs.world.asked.reads, reads + 1, "the other document read; l0, asked by no one, not read");
    let other = disk.docs.key(&bench.dir, disk.atoms, "other.rerail").unwrap();
    assert_ne!(other, l0);
    assert_eq!(keys(&bench.dir, &disk.docs.tape), BTreeSet::from([other]), "l0 gone from the document set");
    assert_eq!(keys(&bench.dir, disk.inst.asks_of(0).expect("L1 asks of its tape port")), BTreeSet::from([other]), "L1 asks for the other alone");
    let outs: Vec<BTreeSet<Vec<i64>>> = disk.inst.outs.iter().map(|t| oracle_rows(&bench.dir, t, t.head()).unwrap()).collect();
    assert!(outs.iter().flatten().all(|r| r[0] == other), "no row of l0 left in L1's output");
    let globals = [disk.globals(&bench)];
    let plain = crate::contract::apply(&disk.l1, &mut bench.cx(), &[(&disk.docs.tape, disk.docs.tape.head())], &globals).unwrap();
    assert_eq!(outs, plain, "L1's output is the plain function's on the document set as it is");
    // Asked again, l0 comes back as an initial run of it, from the store (its content answered before).
    let parses = disk.docs.world.asked.parses;
    let last = disk.last.take();
    let (pins, _, _) = crate::boot::run(&disk.l1, &mut disk.inst, PATH, MODULE, disk.atoms, &mut disk.docs, &mut bench, last.as_ref()).unwrap();
    disk.last = Some(pins);
    assert_eq!(keys(&bench.dir, &disk.docs.tape), BTreeSet::from([l0]));
    assert_eq!(disk.docs.world.asked.parses, parses, "l0's content is answered before: not parsed again");
    let outs: Vec<BTreeSet<Vec<i64>>> = disk.inst.outs.iter().map(|t| oracle_rows(&bench.dir, t, t.head()).unwrap()).collect();
    assert_eq!(outs[0].len(), 14, "l0's 14 rules again");
    assert!(outs.iter().flatten().all(|r| r[0] == l0), "no row of the other left");
    let globals = [disk.globals(&bench)];
    let plain = crate::contract::apply(&disk.l1, &mut bench.cx(), &[(&disk.docs.tape, disk.docs.tape.head())], &globals).unwrap();
    assert_eq!(outs, plain);
}

/// The digest of a block's bytes, where they lie.
fn digest(dir: &Directory, b: BlockId) -> u64 {
    hash_bytes(dir.read("digest", &[b]).unwrap().block(0))
}

#[test]
fn an_unchanged_file_is_identity_checks_only_and_not_read() {
    let _racy = racy_window();
    let mut bench = Bench::new().unwrap();
    let mut disk = Disk::new(&mut bench, "b6-same");
    past_racy_window();
    let (_, cold) = disk.run(&mut bench).unwrap();
    assert_eq!(cold.rows_read, 1, "a cold run reads l0 whole: {cold:?}");
    let (asked, outs, cache) = (disk.docs.world.asked, disk.inst.heads(), digest(&bench.dir, disk.inst.cache.0));
    let (doc, work) = disk.run(&mut bench).unwrap();
    assert!(doc.is_identity());
    assert_eq!(disk.docs.world.asked, asked, "the file is not read, nor loaded, nor parsed");
    assert!(work.is_checks_only(), "{work:?}");
    assert_eq!(disk.inst.heads(), outs, "the output is the identity");
    assert_eq!(digest(&bench.dir, disk.inst.cache.0), cache, "the cache is unchanged");
}

#[test]
fn an_edited_files_delta_is_the_diff_of_two_cold_runs() {
    let mut bench = Bench::new().unwrap();
    let mut disk = Disk::new(&mut bench, "b6-edit");
    disk.run(&mut bench).unwrap();
    let [_, changed, _] = edits();
    disk.edit(&changed, 20);
    let (tape0, atoms0) = (disk.docs.tape.head(), disk.globals(&bench).count);
    // The run's before, held as a reader holds a generation: the run rolls it up at its end (`Trace::roll_up_run`).
    let _before = bench.dir.block(tape0.unwrap()).unwrap();
    // The output's rows before the run (its layers may be rolled up by the run's commit).
    let before: Vec<BTreeSet<Vec<i64>>> = disk.inst.outs.iter().map(|t| oracle_rows(&bench.dir, t, t.head()).unwrap()).collect();
    let (doc, work) = disk.run(&mut bench).unwrap();
    assert!(!doc.is_identity());
    assert_eq!(work.rows_read, 2, "the gone document's kept rows retracted, the new tape read whole: {work:?}");
    let ins = [(&disk.docs.tape, tape0, disk.docs.tape.head())];
    let g = [crate::contract::GlobalIn { block: disk.atoms.0, before: Some(atoms0), after: disk.globals(&bench).count }];
    let want = reference(&disk.l1, &mut bench.cx(), &ins, &g).unwrap();
    let mut changes = 0;
    for (k, t) in disk.inst.outs.iter().enumerate() {
        let (b, a) = (&before[k], oracle_rows(&bench.dir, t, t.head()).unwrap());
        let (added, gone): (BTreeSet<_>, BTreeSet<_>) = (a.difference(b).cloned().collect(), b.difference(&a).cloned().collect());
        assert_eq!((&added, &gone), (&want[k].added, &want[k].gone), "{}", RELATIONS[k].0);
        changes += added.len() + gone.len();
    }
    assert!(changes > 0);
}

#[test]
fn l1_cold_on_l0_gives_its_14_rules() {
    let mut bench = Bench::new().unwrap();
    let mut disk = Disk::new(&mut bench, "b6-cold");
    disk.run(&mut bench).unwrap();
    let o = &disk.inst.outs[0];
    let rules = oracle_rows(&bench.dir, o, o.head()).unwrap();
    assert_eq!(rules.len(), 14);
    let heads: BTreeSet<String> = rules.iter().map(|r| disk.atoms.name(&bench.dir, r[2])).collect();
    assert!(heads.iter().all(|h| RELATIONS.iter().any(|(n, _)| n == h)), "{heads:?}");
}

/// `run` takes and returns pins (B§8): where the run left each trace. A run from the last pins is the delta since
/// them; pins of another state are refused (a trace has two states); a run with none is cold, its outputs from ∅.
#[test]
fn a_run_takes_and_returns_pins() {
    let mut bench = Bench::new().unwrap();
    let mut disk = Disk::new(&mut bench, "b6-pins");
    let [added, ..] = edits();
    let (p0, _, cold) = disk.run_from(&mut bench, None).unwrap();
    assert_eq!((p0.tape, p0.outs.clone()), (disk.docs.tape.head(), disk.inst.heads()), "where the run left its traces");
    assert_eq!(cold.rows_read, 1);
    disk.edit(&added, 30);
    let (p1, doc, work) = disk.run_from(&mut bench, Some(&p0)).unwrap();
    assert!(!doc.is_identity());
    assert_eq!(work.rows_read, 2, "the delta since p0: l0's kept rows retracted, the edit read: {work:?}");
    let warm: Vec<BTreeSet<Vec<i64>>> = disk.inst.outs.iter().map(|t| oracle_rows(&bench.dir, t, t.head()).unwrap()).collect();
    // p0 is no longer where the traces are.
    let e = disk.run_from(&mut bench, Some(&p0)).unwrap_err();
    assert!(e.starts_with("pins of another state"), "{e}");
    // From p1, nothing changed: the identity.
    let (p2, _, work) = disk.run_from(&mut bench, Some(&p1)).unwrap();
    assert!(work.is_checks_only(), "{work:?}");
    assert_eq!(p2, p1);
    // Cold: every bistate from ∅, the same rows.
    let (_, _, work) = disk.run_from(&mut bench, None).unwrap();
    assert_eq!(work.rows_read, 1, "a cold run reads the document whole: {work:?}");
    let cold: Vec<BTreeSet<Vec<i64>>> = disk.inst.outs.iter().map(|t| oracle_rows(&bench.dir, t, t.head()).unwrap()).collect();
    assert_eq!(cold, warm);
}

/// A construct outside K0, refused through L1 as a delta operator, naming its node; no row written.
#[test]
fn k0_pass_refuses_an_aggregate_through_l1() {
    let mut bench = Bench::new().unwrap();
    let mut disk = Disk::new(&mut bench, "b6-agg");
    disk.edit("e(1, 2).\nn(c) :- c = count : { e(_, _) }.\n", 30);
    let e = disk.run(&mut bench).unwrap_err();
    assert!(e.contains("not K0") && e.contains(" rule "), "{e}");
    assert!(disk.inst.heads().iter().all(Option::is_none), "a refused document writes no row");
}

/// L1's cache (Operations §6.1; Bootstrap, Figure 2): each document's sizes, kept by `k0_pass`. A run's room is each
/// read document's nodes (a bound on its rows) and each gone one's kept rows, by the cache: a cold run, from ∅, opens
/// its runs at l0's nodes, in one attempt; the cache then names l0's sizes, and an edit's run retracts them and reads
/// the edit, in one attempt. With the cache dropped, the edit's retraction is bounded by the kept relations' rows: one
/// attempt still, and its change is the same, the reference's (L5).
#[test]
fn l1s_runs_take_one_attempt_with_its_cache_or_without() {
    use crate::contract::OUTGROWN;
    let outgrown = |w: &Work| w.excess.iter().find(|e| e.0 == OUTGROWN).map_or(0, |e| e.1);
    let mut bench = Bench::new().unwrap();
    let mut disk = Disk::new(&mut bench, "b6-cache");
    let (_, cold) = disk.run(&mut bench).unwrap();
    assert_eq!(outgrown(&cold), 0, "a cold run's room is l0's nodes: {cold:?}");
    // The document's key (decision 0052 §3), its tape row's first column.
    let d = oracle_rows(&bench.dir, &disk.docs.tape, disk.docs.tape.head()).unwrap().first().unwrap()[0] as u32;
    {
        let l = bench.dir.read("t", &[disk.inst.cache.0]).unwrap();
        let sizes = rerail_delta_twins::cache::sizes(l.block(0), d).expect("l0's sizes");
        let rows: Vec<u32> = disk.inst.outs.iter().map(|t| oracle_rows(&bench.dir, t, t.head()).unwrap().len() as u32).collect();
        assert_eq!(sizes, &rows[..], "the rows k0_pass gave l0, by relation");
    }
    let [added, changed, _] = edits();
    disk.edit(&added, 30);
    let (_, work) = disk.run(&mut bench).unwrap();
    assert_eq!(outgrown(&work), 0, "the runs opened at the room the cache names: {work:?}");
    // Dropped: the next edit's run costs an attempt, and its change is the reference's.
    disk.edit(&changed, 20);
    disk.inst.drop_cache(&mut bench.dir);
    let (tape0, atoms0) = (disk.docs.tape.head(), disk.globals(&bench).count);
    let _before = bench.dir.block(tape0.unwrap()).unwrap();
    let before: Vec<BTreeSet<Vec<i64>>> = disk.inst.outs.iter().map(|t| oracle_rows(&bench.dir, t, t.head()).unwrap()).collect();
    let (_, work) = disk.run(&mut bench).unwrap();
    assert_eq!(outgrown(&work), 0, "from ∅, the edit's retraction bounded by the kept rows: {work:?}");
    let ins = [(&disk.docs.tape, tape0, disk.docs.tape.head())];
    let g = [crate::contract::GlobalIn { block: disk.atoms.0, before: Some(atoms0), after: disk.globals(&bench).count }];
    let want = reference(&disk.l1, &mut bench.cx(), &ins, &g).unwrap();
    for (k, t) in disk.inst.outs.iter().enumerate() {
        let a = oracle_rows(&bench.dir, t, t.head()).unwrap();
        let got = (a.difference(&before[k]).cloned().collect::<BTreeSet<_>>(), before[k].difference(&a).cloned().collect::<BTreeSet<_>>());
        assert_eq!((&got.0, &got.1), (&want[k].added, &want[k].gone), "{}", RELATIONS[k].0);
    }
}

/// The residency's policy, as it is (`rerail_slabs`): a run retires nothing while the slabs fit the budget; past it,
/// the least recently used slab goes, the live document's last of all (each run asks for it). A document whose slab
/// went keeps its content hit: back to an earlier text, it is loaded from the store's objects, not parsed. All of it
/// is dropped `DROP_AFTER` after the last run, and the next run loads the live document again.
#[test]
fn a_run_retires_the_least_recently_used_slab_past_the_budget_and_keeps_the_content_hit() {
    let mut bench = Bench::new().unwrap();
    // Slabs of no more than a document's need, and each version of l0 64 KB longer than the last (a comment): its
    // room, at the tape's bound, is past what the last one's slab has left, so each lies in a slab of its own.
    let pad = |k: usize| format!("// {}\n", "x".repeat(64 << 10)).repeat(k + 1);
    let [added, changed, removed] = edits();
    let texts = [format!("{added}{}", pad(0)), format!("{changed}{}", pad(1)), format!("{removed}{}", pad(2))];
    let dir = Directory::default().with_slab_bytes(1);
    // Room for l0's slab and the first two edits', not the third's: the third edit retires one, l0's, the least
    // recently used.
    let slabs: usize = [L0, &texts[0], &texts[1], &texts[2]].iter().map(|t| dir.slab_room_bytes(crate::files::room(t.len()))).sum();
    let budget = slabs - 1;
    bench.dir = dir.with_residency(budget, std::time::Duration::from_secs(3600));
    let mut disk = Disk::new(&mut bench, "b6-retire");
    let (cold, _) = disk.run(&mut bench).unwrap();
    let rules = |disk: &Disk, bench: &Bench| oracle_rows(&bench.dir, &disk.inst.outs[0], disk.inst.outs[0].head()).unwrap().len();
    for (k, text) in texts.iter().enumerate() {
        disk.edit(text, 40 - 10 * k as u64);
        disk.run(&mut bench).unwrap();
        let held: usize = bench.dir.slabs().iter().map(|&b| bench.dir.block(b).unwrap().reserved()).sum();
        assert!(held <= budget, "edit {k}: within the budget ({held} of {budget})");
    }
    let (retired, _) = bench.dir.residency();
    assert!(retired >= 1, "past the budget, a slab retired");
    assert!(bench.blobs.get(&bench.dir, &cold.after.tape).unwrap().is_none(), "l0's, the least recently used");
    assert_eq!(rules(&disk, &bench), 14, "the live document resident, its rows read");
    // Unchanged runs retire nothing more and read nothing.
    for _ in 0..3 {
        let (doc, work) = disk.run(&mut bench).unwrap();
        assert!(doc.is_identity() && work.is_checks_only());
    }
    assert_eq!(bench.dir.residency().0, retired);
    // Back to l0, whose slab was retired first: loaded from the store's objects, not parsed again.
    let asked = disk.docs.world.asked;
    disk.edit(L0, 5);
    let (doc, _) = disk.run(&mut bench).unwrap();
    assert!(!doc.is_identity());
    assert_eq!(disk.docs.world.asked.parses, asked.parses, "a content hit: not parsed");
    assert_eq!(disk.docs.world.asked.loads, asked.loads + 1, "loaded where the store holds it");
    assert_eq!(rules(&disk, &bench), 14);
}

/// Everything resident is dropped `DROP_AFTER` after the last run (the residency's drop); the next run finds the live
/// document gone and loads it from the store's objects (no read past its stat, no parse), and its delta is the
/// identity.
#[test]
fn the_residency_drops_all_after_a_run_and_the_next_run_loads_again() {
    let _racy = racy_window();
    let mut bench = Bench::new().unwrap();
    bench.dir = Directory::default().with_residency(1 << 40, std::time::Duration::from_millis(30));
    let mut disk = Disk::new(&mut bench, "b6-drop");
    past_racy_window();
    disk.run(&mut bench).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(400));
    assert_eq!(bench.dir.residency().1, 1, "dropped after the run");
    assert!(bench.dir.slabs().is_empty(), "no slab held");
    let asked = disk.docs.world.asked;
    let (doc, work) = disk.run(&mut bench).unwrap();
    assert!(doc.is_identity() && work.is_checks_only(), "{work:?}");
    assert_eq!((disk.docs.world.asked.parses, disk.docs.world.asked.loads), (asked.parses, asked.loads + 1));
    assert_eq!(oracle_rows(&bench.dir, &disk.inst.outs[0], disk.inst.outs[0].head()).unwrap().len(), 14);
}

/// What a run keeps lasts across processes (*Operations* §5, P§11.4): its traces' layers, the symbols blocks and the
/// pins in the store's kept folder, the atoms in the store's atoms file, sources and tapes in its objects. A later
/// process opens them: its first run over an unchanged root is L4's identity, with no file read past its stat and
/// nothing parsed or interned; an edit then is the same delta as in the first process.
#[test]
fn a_later_process_starts_where_the_last_run_left_off() {
    let _racy = racy_window();
    let (root, store) = root("b6-restart");
    let folder = store.folder.clone();
    let open = |root: &PathBuf| {
        let store = Store::open(&folder).unwrap();
        let (mut bench, pins) = Bench::open(&store.kept()).unwrap();
        let atoms = Atoms::open(&mut bench.dir, &store.atoms()).unwrap();
        let world = World::new(root.clone(), store, &bench.dir, pins.as_ref().map_or([None; 3], |p| p.1.world)).unwrap();
        let disk = Disk::over(&mut bench, root.clone(), world, atoms, pins);
        (bench, disk)
    };
    drop(store);
    past_racy_window();
    let (rows, names) = {
        let (mut bench, mut disk) = open(&root);
        assert!(disk.last.is_none(), "a new store: no pins");
        let (doc, _) = disk.run(&mut bench).unwrap();
        assert_eq!(doc.before, None);
        let rows: Vec<BTreeSet<Vec<i64>>> = disk.inst.outs.iter().map(|t| oracle_rows(&bench.dir, t, t.head()).unwrap()).collect();
        let names: Vec<String> = rows[0].iter().map(|r| disk.atoms.name(&bench.dir, r[2])).collect();
        (rows, names)
    };
    // The process's observer forgotten, as a new process has none.
    rerail_observe::content_store::observe::forget(&rerail_observe::content_store::observe::store_beside(&folder.join("objects.db")));
    let (mut bench, mut disk) = open(&root);
    assert!(disk.last.is_some(), "the last run's pins");
    let again: Vec<BTreeSet<Vec<i64>>> = disk.inst.outs.iter().map(|t| oracle_rows(&bench.dir, t, t.head()).unwrap()).collect();
    assert_eq!(again, rows, "L1's outputs, where the last run left them");
    let c0 = disk.docs.world.counts();
    let (doc, work) = disk.run(&mut bench).unwrap();
    assert!(doc.is_identity(), "{doc:?}");
    assert!(work.is_checks_only(), "{work:?}");
    let (a, c) = (disk.docs.world.asked, disk.docs.world.counts().since(&c0));
    assert_eq!((a.parses, a.interns), (0, 0), "nothing parsed or interned: {a:?}");
    // The observer trusts the file on its stat, as it last answered it; it hashes one trusted file in 64 again (its
    // sampled check): the only read, and it finds the bytes unchanged.
    assert_eq!((a.reads, c.mismatched), (c.validated, 0), "no read but the observer's sampled check: {a:?} {c:?}");
    assert_eq!(a.loads, 1, "the document loaded from the store's objects, where its places say");
    let names_now: Vec<String> = again[0].iter().map(|r| disk.atoms.name(&bench.dir, r[2])).collect();
    assert_eq!(names_now, names, "the atoms keep their ids");
    // An edit: the same delta as a cold run's difference.
    let [added, ..] = edits();
    disk.edit(&added, 30);
    let (doc, work) = disk.run(&mut bench).unwrap();
    assert!(!doc.is_identity());
    assert_eq!(work.rows_read, 2, "{work:?}");
    assert_eq!(oracle_rows(&bench.dir, &disk.inst.outs[0], disk.inst.outs[0].head()).unwrap().len(), 15, "l0's 14 rules and the one added");
}

/// l0's cold boot run, `RERAIL_BOOT_RUNS` times (default 1), each on a bench and root of its own: what a profiler
/// samples the boot run by (`cargo test ... -- --ignored --exact l1::tests::the_cold_boot_run_repeated`).
#[test]
#[ignore = "a profiler's load, not a check"]
fn the_cold_boot_run_repeated() {
    let n: usize = std::env::var("RERAIL_BOOT_RUNS").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    let base = std::env::temp_dir().join(format!("rerail-boot-profile-{}", std::process::id()));
    for k in 0..n {
        let mut bench = Bench::new().unwrap();
        crate::boot::cold_k_form(&mut bench, &base.join(k.to_string())).unwrap();
    }
}

#[test]
fn the_boot_runs_as_a_delta() {
    let mut bench = Bench::new().unwrap();
    let base = std::env::temp_dir().join(format!("rerail-chain-{}", std::process::id()));
    let versions = crate::boot::chain_versions();
    let runs = crate::boot::boot_chain(&mut bench, &base, &versions.each_ref().map(String::as_str)).unwrap_or_else(|e| panic!("{e}"));
    let atoms = crate::sym::Atoms(runs[0].after.atoms.block);
    crate::boot::check_boot_chain(&runs, |x| atoms.find(&bench.dir, x));
}

/// The laws' drivers' scratch roots, one a test.
fn laws_base(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("rerail-laws-{}-{name}", std::process::id()))
}

/// L6: the boot run threaded over l0's edits and the comments' versions is, run by run, the composed plain functions
/// run cold on that version's text; and a stage whose input is the identity cuts off: l0 touched (its bytes the same)
/// is answered by its load alone, not parsed nor interned, and L1, given the identity, does its checks alone.
#[test]
fn a_run_is_the_composed_reference_and_cuts_off() {
    let mut bench = Bench::new().unwrap();
    let [added, changed, removed] = edits();
    let [_, _, comments, commented_rule] = crate::boot::chain_versions();
    let versions = [L0, &added, &changed, &removed, &comments, &commented_rule];
    let c = crate::laws::composition_and_cutoff(&mut bench, &laws_base("l6"), &versions).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(c.same_as_cold, vec![true; versions.len()], "each run is the cold composed run on its text");
    let (u, t) = (&c.unchanged, &c.touched);
    assert!(u.doc.is_identity() && t.doc.is_identity());
    assert_eq!(t.asked, crate::files::Asked { reads: 1, loads: 0, parses: 0, interns: 0 }, "touched: read and found the same content, not parsed");
    assert!(t.work.is_checks_only(), "{:?}", t.work);
    assert_eq!(t.before.as_ref(), Some(&t.after), "every pin where it was");
    assert_eq!((t.launches, t.cache, &t.kform), (1, u.cache, &u.kform), "its one launch the load's answer; the cache and output unchanged");
}

/// L7: one module edited among three: the edit's step changes the edited module's partitions alone, the others' rows
/// stay, and its work is the edited module's alone (the same edit in a set of it alone).
#[test]
fn one_edited_document_among_several_reads_and_writes_only_its_own() {
    let among = crate::laws::edit_among(&mut Bench::new().unwrap(), &laws_base("l7-among"), 3).unwrap_or_else(|e| panic!("{e}"));
    let alone = crate::laws::edit_among(&mut Bench::new().unwrap(), &laws_base("l7-alone"), 0).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(among.work, alone.work, "the edited module's work alone");
    assert_eq!(among.asked, crate::files::Asked { reads: 1, loads: 0, parses: 1, interns: 1 }, "the edited file alone read, parsed and interned");
}

/// L8: two edits of l0 read as one change and stepped once give what stepping each gives; an edit and its undo stepped
/// once leave the output as it was, its change the identity, though the input's change is the identity by value only
/// (its tape row gone and back the same values): the module is read again (O3), and the pairs cancel.
#[test]
fn edits_coalesce_two_as_one_and_an_undo_as_the_identity() {
    let c = crate::laws::coalescing(&mut Bench::new().unwrap(), &laws_base("l8")).unwrap_or_else(|e| panic!("{e}"));
    assert!(c.once_is_twice, "two edits stepped once are the two steps");
    assert!(c.once.rows_written >= c.least, "{:?} against the least change's {}", c.once, c.least);
    assert!(c.undo_rows_same && c.undo_out_identity, "an edit and its undo: the output as it was, its change the identity");
    assert_eq!(c.undo_in_identity, (false, true), "the input's change: not the identity by its counts, the identity by value");
}

/// P1: the work of one edit does not grow with the rest: one module edited among 1, 2, 4 and 8 unchanged ones, L1's
/// step reads, writes and launches the same; and the world's read of the set reads the edited file alone.
#[test]
fn the_work_of_one_edit_does_not_grow_with_the_rest() {
    let runs: Vec<crate::laws::EditAmong> = [1, 2, 4, 8].iter().map(|&n| crate::laws::edit_among(&mut Bench::new().unwrap(), &laws_base(&format!("p1-{n}")), n).unwrap_or_else(|e| panic!("{n}: {e}"))).collect();
    for (r, n) in runs.iter().zip([1, 2, 4, 8]) {
        assert_eq!((&r.work, r.launches, r.asked), (&runs[0].work, runs[0].launches, runs[0].asked), "{n} unchanged modules beside the edit");
    }
}
