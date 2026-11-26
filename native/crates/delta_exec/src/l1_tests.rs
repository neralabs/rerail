//! B6's checks: L1 over a copy of l0 on disk.

use super::*;
use crate::L0;
use crate::boot::Tape;
use crate::contract::{Bench, Instance, harness_on, reference, rows_at};
use crate::dir::Directory;
use crate::files::Doc;
use crate::files::tests::{past_racy_window, racy_window, root, write};
use crate::intern::InternOps;
use crate::sym::Atoms;
use rerail_blocks::maps::hash_bytes;
use std::collections::BTreeSet;
use std::path::PathBuf;

const PATH: &str = "bootstrap-l0.rerail";

/// L1's world on disk: a root holding a copy of l0, its tape ports as the world reads them, and L1.
struct Disk {
    root: PathBuf,
    tape: Tape,
    l1: L1,
}

impl Disk {
    fn new(bench: &mut Bench, name: &str) -> Disk {
        let (root, files) = root(name);
        let iops = InternOps::declare(&mut bench.reg).unwrap();
        let ops = K0Ops::declare(&mut bench.reg).unwrap();
        let atoms = Atoms::new(&mut bench.dir);
        Disk { root, tape: Tape::new(files, iops, atoms).unwrap(), l1: L1 { ops } }
    }

    /// l0 read by the world (B§8's run, its first line).
    fn answer(&mut self, bench: &mut Bench) -> Doc {
        self.tape.read(&bench.reg, &bench.traces, &mut bench.dir, &mut bench.blobs, PATH).unwrap()
    }

    /// B§8's run, its first two lines.
    fn run(&mut self, bench: &mut Bench, inst: &mut Instance) -> Result<(Doc, Work), String> {
        crate::boot::run(&mut self.tape, &self.l1, inst, bench, PATH)
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
    disk.answer(&mut bench);
    let [added, changed, removed] = edits();
    // Each edit's stat differs from the one before (its size, its modification time).
    let chain = [None, Some(added.as_str()), Some(changed.as_str()), None, Some(removed.as_str()), Some(L0)];
    for (k, e) in chain.into_iter().enumerate() {
        if let Some(text) = e {
            disk.edit(text, 50 - 5 * k as u64);
        }
        disk.answer(&mut bench);
    }
    assert_eq!(disk.tape.heads[1], disk.tape.heads[2], "an unchanged file is the identity");
    let Disk { l1, tape: Tape { mut ins, heads, globals, .. }, .. } = disk;
    for drop_at in [1, 3, 5] {
        harness_on(&l1, &mut bench, &mut ins, &heads, &globals, drop_at).unwrap_or_else(|b| panic!("{b:?}"));
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
    let mut l1 = Instance::new(&disk.l1, &mut bench.dir).unwrap();
    let (_, cold) = disk.run(&mut bench, &mut l1).unwrap();
    assert_eq!(cold.rows_read, 1, "a cold run reads l0 whole: {cold:?}");
    let (reads, outs, cache) = (disk.tape.files.reads, l1.heads(), digest(&bench.dir, l1.cache.0));
    let (doc, work) = disk.run(&mut bench, &mut l1).unwrap();
    assert!(doc.is_identity());
    assert_eq!(disk.tape.files.reads, reads, "the file is not read");
    assert!(work.is_checks_only(), "{work:?}");
    assert_eq!(l1.heads(), outs, "the output is the identity");
    assert_eq!(digest(&bench.dir, l1.cache.0), cache, "the cache is unchanged");
}

#[test]
fn an_edited_files_delta_is_the_diff_of_two_cold_runs() {
    let mut bench = Bench::new().unwrap();
    let mut disk = Disk::new(&mut bench, "b6-edit");
    let mut l1 = Instance::new(&disk.l1, &mut bench.dir).unwrap();
    disk.run(&mut bench, &mut l1).unwrap();
    let [_, changed, _] = edits();
    disk.edit(&changed, 20);
    // The output's rows before the run (its layers may be rolled up by the run's commit).
    let before: Vec<BTreeSet<Vec<i64>>> = l1.outs.iter().map(|t| rows_at(&bench.dir, t, t.head()).unwrap()).collect();
    let (doc, work) = disk.run(&mut bench, &mut l1).unwrap();
    assert!(!doc.is_identity());
    assert_eq!(work.rows_read, 2, "the old tape and the new one, each read whole: {work:?}");
    let want = reference(&disk.l1, &mut bench.cx(), &disk.tape.since(2), &disk.tape.globals_since(2)).unwrap();
    let mut changes = 0;
    for (k, t) in l1.outs.iter().enumerate() {
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
    let mut l1 = Instance::new(&disk.l1, &mut bench.dir).unwrap();
    disk.run(&mut bench, &mut l1).unwrap();
    let rules = rows_at(&bench.dir, &l1.outs[0], l1.outs[0].head()).unwrap();
    assert_eq!(rules.len(), 14);
    let heads: BTreeSet<String> = rules.iter().map(|r| disk.tape.atoms.name(&bench.dir, r[2])).collect();
    assert!(heads.iter().all(|h| RELATIONS.iter().any(|(n, _)| n == h)), "{heads:?}");
}

/// A construct outside K0, refused through L1 as a delta operator, naming its node; no row written.
#[test]
fn k0_pass_refuses_an_aggregate_through_l1() {
    let mut bench = Bench::new().unwrap();
    let mut disk = Disk::new(&mut bench, "b6-agg");
    disk.edit("e(1, 2).\nn(c) :- c = count : { e(_, _) }.\n", 30);
    let mut l1 = Instance::new(&disk.l1, &mut bench.dir).unwrap();
    let e = disk.run(&mut bench, &mut l1).unwrap_err();
    assert!(e.contains("not K0") && e.contains(" rule "), "{e}");
    assert!(l1.heads().iter().all(Option::is_none), "a refused document writes no row");
}
