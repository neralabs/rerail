//! B6's checks: L1 over a copy of l0 on disk.

use super::*;
use crate::L0;
use crate::boot::{DocSet, Pins};
use crate::contract::{Bench, Global, Instance, harness_on, reference, rows_at};
use crate::dir::Directory;
use crate::files::Doc;
use crate::files::tests::{past_racy_window, racy_window, root, write};
use crate::intern::InternOps;
use crate::sym::Atoms;
use rerail_blocks::maps::hash_bytes;
use std::collections::BTreeSet;
use std::path::PathBuf;

const PATH: &str = "bootstrap-l0.rerail";

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
        let (root, files) = root(name);
        let iops = InternOps::declare(&mut bench.reg).unwrap();
        let ops = K0Ops::declare(&mut bench.reg).unwrap();
        let drops = crate::drop::DropOps::declare(&mut bench.reg).unwrap();
        let atoms = Atoms::new(&mut bench.dir);
        let l1 = L1 { ops, drops };
        let inst = Instance::new(&l1, &mut bench.dir).unwrap();
        Disk { root, docs: DocSet::new(files, iops, drops).unwrap(), atoms, l1, inst, last: None }
    }

    /// l0 read by the file layer (B§8's run, its first line).
    fn answer(&mut self, bench: &mut Bench) -> Doc {
        self.docs.read(bench, self.atoms, PATH).unwrap()
    }

    /// B§8's run, its first two lines, from `last`: its pins.
    fn run_from(&mut self, bench: &mut Bench, last: Option<&Pins>) -> Result<(Pins, Doc, Work), String> {
        crate::boot::run(&self.l1, &mut self.inst, PATH, self.atoms, &mut self.docs, bench, last)
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
    let added = format!("{L0}\nb6_added(c, h) :- k_rule(_, c, h), !k_wild(_, c).\n");
    let changed = added.replace("k_wild(d, t) :- kind(d, t, \"wildcard\").", "k_wild(d, t) :- kind(d, t, \"wildcard\"), node(d, t, _, _, _).");
    let line = changed.lines().find(|l| l.starts_with("k_num(")).unwrap().to_string();
    let removed = changed.replace(&format!("{line}\n"), "");
    assert!(added != L0 && changed != added && removed != changed);
    [added, changed, removed]
}

#[test]
fn the_harness_passes_l1_over_edits_of_l0_on_disk() {
    let mut bench = Bench::new().unwrap();
    let mut disk = Disk::new(&mut bench, "b6-chain");
    let (mut heads, mut globals) = (vec![vec![None]], vec![vec![Global { block: disk.atoms.0, count: 0 }]]);
    let [added, changed, removed] = edits();
    // Each edit's stat differs from the one before (its size, its modification time).
    let chain = [None, None, Some(added.as_str()), Some(changed.as_str()), None, Some(removed.as_str()), Some(L0)];
    for (k, e) in chain.into_iter().enumerate() {
        if let Some(text) = e {
            disk.edit(text, 50 - 5 * k as u64);
        }
        disk.answer(&mut bench);
        heads.push(vec![disk.docs.tape.head()]);
        globals.push(vec![disk.globals(&bench)]);
    }
    assert_eq!(heads[1], heads[2], "an unchanged file is the identity");
    let Disk { l1, mut docs, .. } = disk;
    // L5: the cache dropped at a random step of the chain, a seed each.
    for seed in 1..=3u64 {
        let drop_at = 1 + (rerail_blocks::layer::mix(seed) % (heads.len() as u64 - 1)) as usize;
        harness_on(&l1, &mut bench, std::slice::from_mut(&mut docs.tape), &heads, &globals, drop_at).unwrap_or_else(|b| panic!("{b:?}"));
    }
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
    let (reads, outs, cache) = (disk.docs.files.reads, disk.inst.heads(), digest(&bench.dir, disk.inst.cache.0));
    let (doc, work) = disk.run(&mut bench).unwrap();
    assert!(doc.is_identity());
    assert_eq!(disk.docs.files.reads, reads, "the file is not read");
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
    // The output's rows before the run (its layers may be rolled up by the run's commit).
    let before: Vec<BTreeSet<Vec<i64>>> = disk.inst.outs.iter().map(|t| rows_at(&bench.dir, t, t.head()).unwrap()).collect();
    let (doc, work) = disk.run(&mut bench).unwrap();
    assert!(!doc.is_identity());
    assert_eq!(work.rows_read, 2, "the gone document's kept rows retracted, the new tape read whole: {work:?}");
    let ins = [(&disk.docs.tape, tape0, disk.docs.tape.head())];
    let g = [crate::contract::GlobalIn { block: disk.atoms.0, before: Some(atoms0), after: disk.globals(&bench).count }];
    let want = reference(&disk.l1, &mut bench.cx(), &ins, &g).unwrap();
    let mut changes = 0;
    for (k, t) in disk.inst.outs.iter().enumerate() {
        let (b, a) = (&before[k], rows_at(&bench.dir, t, t.head()).unwrap());
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
    let rules = rows_at(&bench.dir, o, o.head()).unwrap();
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
    let warm: Vec<BTreeSet<Vec<i64>>> = disk.inst.outs.iter().map(|t| rows_at(&bench.dir, t, t.head()).unwrap()).collect();
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
    let cold: Vec<BTreeSet<Vec<i64>>> = disk.inst.outs.iter().map(|t| rows_at(&bench.dir, t, t.head()).unwrap()).collect();
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

/// L1's cache (Operations §6.1; Bootstrap, Figure 2): each document's sizes, kept by `k0_pass`. A cold run, from ∅,
/// outgrows l0's runs and runs again; the cache then names l0's sizes, and an edit's run opens its runs at the room
/// they name, in one attempt. With the cache dropped, the edit's run costs an attempt more and its change is the
/// same, the reference's (L5).
#[test]
fn l1s_cache_sizes_its_runs_and_dropped_costs_an_attempt_not_an_answer() {
    use crate::contract::OUTGROWN;
    let outgrown = |w: &Work| w.excess.iter().find(|e| e.0 == OUTGROWN).map_or(0, |e| e.1);
    let mut bench = Bench::new().unwrap();
    let mut disk = Disk::new(&mut bench, "b6-cache");
    let (_, cold) = disk.run(&mut bench).unwrap();
    assert!(outgrown(&cold) > 0, "l0's k_arg rows outgrow a first room: {cold:?}");
    let d = disk.atoms.find(&bench.dir, "bootstrap-l0").unwrap() as u32;
    {
        let l = bench.dir.read("t", &[disk.inst.cache.0]).unwrap();
        let sizes = rerail_delta_twins::cache::sizes(l.block(0), d).expect("l0's sizes");
        let rows: Vec<u32> = disk.inst.outs.iter().map(|t| rows_at(&bench.dir, t, t.head()).unwrap().len() as u32).collect();
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
    let before: Vec<BTreeSet<Vec<i64>>> = disk.inst.outs.iter().map(|t| rows_at(&bench.dir, t, t.head()).unwrap()).collect();
    let (_, work) = disk.run(&mut bench).unwrap();
    assert!(outgrown(&work) > 0, "from ∅, the edit outgrows a first room: {work:?}");
    let ins = [(&disk.docs.tape, tape0, disk.docs.tape.head())];
    let g = [crate::contract::GlobalIn { block: disk.atoms.0, before: Some(atoms0), after: disk.globals(&bench).count }];
    let want = reference(&disk.l1, &mut bench.cx(), &ins, &g).unwrap();
    for (k, t) in disk.inst.outs.iter().enumerate() {
        let a = rows_at(&bench.dir, t, t.head()).unwrap();
        let got = (a.difference(&before[k]).cloned().collect::<BTreeSet<_>>(), before[k].difference(&a).cloned().collect::<BTreeSet<_>>());
        assert_eq!((&got.0, &got.1), (&want[k].added, &want[k].gone), "{}", RELATIONS[k].0);
    }
}
