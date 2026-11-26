//! B2's checks (*0686: Operations* §7, §10; P§1, P§11–§12): a trace of random runs against a brute-force set, its
//! states and changes read where the layers lie; the committed state readable while the next layer is written; a
//! change's partitions in its size; compose and invert; the sketches; consolidate's refusals; a roll-up, and the
//! merged generations let go (a referenced generation lives until its referrer is rolled up); the same layer at
//! every tiling and order.

use super::*;
use crate::op::{Registry, Status, Tiling};
use rerail_blocks::bistate::{ColumnIndex, Read, States, check};
use rerail_blocks::layer::{ColType, RunsW};
use rerail_blocks::region::pin_block;
use rerail_blocks::ids::RowId;
use rerail_blocks::layer::mix;
use std::collections::BTreeSet;

type Rows = BTreeSet<Vec<i64>>;

/// Three columns: a partition (32 bits), a 64-bit value, a 32-bit value; sorted partition first.
fn shape() -> Shape {
    Shape::run(vec![ColType::I32, ColType::I64, ColType::I32], vec![0, 1, 2], Some(0))
}

fn setup() -> (Registry, TraceOps, Directory, Trace) {
    let mut reg = Registry::default();
    let ops = TraceOps::declare(&mut reg).unwrap();
    (reg, ops, Directory::default(), Trace::new(&shape()).unwrap())
}

fn rng(seed: u64) -> impl FnMut() -> u64 {
    let mut z = seed;
    move || {
        z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
        mix(z)
    }
}

/// A run's δ runs: `pairs` pushed by a writer, in the order given.
fn runs(dir: &mut Directory, t: &Trace, pairs: &[(Vec<i64>, i32)]) -> BlockId {
    let id = t.open(dir, pairs.len());
    let b = dir.block(id).unwrap();
    let pin = pin_block(&b, "writer", true).unwrap();
    let w = RunsW::new(&pin);
    for (r, d) in pairs {
        w.push(r, *d).expect("room");
    }
    id
}

fn row(s: &impl Read, id: RowId) -> Vec<i64> {
    ColumnIndex::all(s.arity()).map(|c| s.get(id, c)).collect()
}

fn rows_of(s: &impl Read) -> Rows {
    s.scan().map(|id| row(s, id)).collect()
}

/// A random run against `set`: some rows gone, some new rows added, and a new row added and taken back (+1, −1).
fn random_run(r: &mut impl FnMut() -> u64, set: &Rows) -> (Vec<(Vec<i64>, i32)>, Rows) {
    let mut next = set.clone();
    let mut pairs = Vec::new();
    for x in set.iter() {
        if r().is_multiple_of(4) {
            pairs.push((x.clone(), -1));
            next.remove(x);
        }
    }
    for _ in 0..(r() % 300 + 1) {
        let x = vec![(r() % 7) as i64, (r() % 100_000) as i64 - 50_000, (r() % 1000) as i64];
        if !set.contains(&x) && next.insert(x.clone()) {
            pairs.push((x, 1));
        }
    }
    let undo = vec![99, 1, 1];
    if !set.contains(&undo) && !next.contains(&undo) {
        pairs.push((undo.clone(), 1));
        pairs.push((undo, -1));
    }
    // The writers' order: shuffled.
    for i in (1..pairs.len()).rev() {
        pairs.swap(i, (r() % (i as u64 + 1)) as usize);
    }
    (pairs, next)
}

/// `random_run`, and the cases a chain's dedup is for: rows held killed and added again in the same run (no change),
/// and rows killed in an earlier round added back (`gone_before`: each one's dead copy lies in a layer below, and the
/// new one is live).
fn random_run_with_history(r: &mut impl FnMut() -> u64, set: &Rows, gone_before: &Rows) -> (Vec<(Vec<i64>, i32)>, Rows) {
    let (mut pairs, mut next) = random_run(r, set);
    for x in set.iter() {
        if next.contains(x) && r().is_multiple_of(9) {
            pairs.push((x.clone(), -1));
            pairs.push((x.clone(), 1));
        }
    }
    for x in gone_before.iter() {
        if !set.contains(x) && !next.contains(x) && r().is_multiple_of(3) {
            pairs.push((x.clone(), 1));
            next.insert(x.clone());
        }
    }
    for i in (1..pairs.len()).rev() {
        pairs.swap(i, (r() % (i as u64 + 1)) as usize);
    }
    (pairs, next)
}

fn parts_of(rows: &Rows) -> BTreeSet<i64> {
    rows.iter().map(|x| x[0]).collect()
}

/// The partitions whose rows differ between two sets.
fn changed(a: &Rows, b: &Rows) -> Vec<i64> {
    let mut p: Vec<i64> = a.symmetric_difference(b).map(|x| x[0]).collect();
    p.sort();
    p.dedup();
    p
}

/// A random chain of runs against a brute-force set: each state's rows, each change's rows gone and added and its
/// counts, each partition restricted, the changed partitions, a probe, the sketches' counts, at varied tilings.
#[test]
fn a_random_trace_reads_as_its_set() {
    // The rebuild plan's B2: "24 rounds of pushes and kills, 7 seeds".
    std::thread::scope(|sc| {
        for seed in [42, 7, 13, 99, 1234, 5, 77] {
            sc.spawn(move || random_trace(seed, 24));
        }
    });
}

fn random_trace(seed: u64, rounds: usize) {
    let (reg, ops, mut dir, mut t) = setup();
    let mut r = rng(seed);
    let mut sets: Vec<Rows> = vec![Rows::new()];
    let mut heads: Vec<Option<BlockId>> = vec![None];
    let mut gone_before = Rows::new();
    let tilings = [Tiling::WHOLE, Tiling { tile: 1, seed: 9 }, Tiling { tile: 5, seed: 0 }, Tiling { tile: 2, seed: 4 }];
    for step in 0..rounds {
        let set = sets.last().unwrap().clone();
        let (pairs, next) = random_run_with_history(&mut r, &set, &gone_before);
        gone_before.extend(set.difference(&next).cloned());
        let id = runs(&mut dir, &t, &pairs);
        let h = t.commit(&reg, &ops, &mut dir, id, tilings[step % tilings.len()]).unwrap_or_else(|e| panic!("seed {seed}, step {step}: {e}"));
        sets.push(next);
        heads.push(Some(h));
    }
    let l = t.read(&dir, &[t.head()]).unwrap();
    let table = l.table();
    for (k, set) in sets.iter().enumerate() {
        let s = t.state(&table, heads[k]).unwrap();
        assert_eq!(&rows_of(&s), set, "seed {seed}: state {k} is its set");
        for p in parts_of(set).into_iter().chain([3, 99]) {
            let want: Rows = set.iter().filter(|x| x[0] == p).cloned().collect();
            assert_eq!(rows_of(&rerail_blocks::bistate::States::restrict(&s, p)), want, "seed {seed}: state {k}, partition {p}");
        }
        if let Some(x) = set.iter().next() {
            let cols: Vec<ColumnIndex> = ColumnIndex::all(3).collect();
            let found: Vec<RowId> = s.matching(&cols, x).collect();
            assert_eq!(found.len(), 1, "a probe finds the one live row");
            assert_eq!(&row(&s, found[0]), x);
        }
        for c in ColumnIndex::all(3) {
            let st = s.stats(c);
            assert_eq!(st.rows, set.len() as u64, "seed {seed}: state {k}: the sketch counts the live rows");
            let ndv = set.iter().map(|x| x[c.get()]).collect::<BTreeSet<_>>().len() as f64;
            // Exact while the sketch keeps exact counts; past them a HyperLogLog's, about 6.5% off (the sketch's own
            // test allows 20%).
            assert!((st.ndv as f64 - ndv).abs() <= 0.2 * ndv, "state {k}: the sketch's distinct values, {} against {ndv}", st.ndv);
            if let Some(m) = set.iter().map(|x| x[c.get()]).min() {
                assert!(st.min <= m, "the sketch's min bounds the live rows'");
            }
        }
    }
    // Each round's change from the one before (its before the previous state), and changes across several layers.
    for k in 1..sets.len() {
        let spans = [k - 1, k.saturating_sub(6)];
        for j in spans.into_iter().collect::<BTreeSet<_>>() {
            let b = t.change(&table, heads[j], heads[k]).unwrap();
            check(&b);
            let gone: Rows = b.gone().map(|id| row(b.before().unwrap(), id)).collect();
            let added: Rows = b.added().map(|id| row(b.after(), id)).collect();
            assert_eq!(b.counts(), (gone.len(), added.len()));
            // A round's change (a trace's two states) is the least one. A change across several layers is by id: a
            // row killed in one and added back in a later one is gone under its old id and added under its new one,
            // the same values; by value, the two cancel.
            let back: Rows = gone.intersection(&added).cloned().collect();
            if j == k - 1 {
                // `bistate::is_least`, by the values' sets: no row gone has the values of a row added.
                assert!(back.is_empty(), "seed {seed}: {j}→{k} is the least change");
            }
            assert!(back.iter().all(|x| sets[j].contains(x) && sets[k].contains(x)), "seed {seed}: {j}→{k}: a row gone and added is live at both");
            assert_eq!(gone.difference(&back).cloned().collect::<Rows>(), sets[j].difference(&sets[k]).cloned().collect::<Rows>(), "seed {seed}: {j}→{k}: gone");
            assert_eq!(added.difference(&back).cloned().collect::<Rows>(), sets[k].difference(&sets[j]).cloned().collect::<Rows>(), "seed {seed}: {j}→{k}: added");
            let mut ps = Vec::new();
            b.changed_parts(&mut |p| ps.push(p));
            ps.sort();
            // The partitions changed by digest: a row gone and back moves its partition's digest by nothing.
            assert_eq!(ps, changed(&sets[j], &sets[k]), "seed {seed}: {j}→{k}: the changed partitions");
            // Each partition's restriction, of each round's change (a span's is the same read over more layers).
            if j + 1 < k {
                continue;
            }
            for p in parts_of(&sets[j]).union(&parts_of(&sets[k])) {
                let rb = b.restrict(*p);
                check(&rb);
                let want = sets[k].iter().filter(|x| x[0] == *p).count() + sets[j].iter().filter(|x| x[0] == *p).count();
                let (g, a) = rb.counts();
                assert!(g + a <= want, "a restriction's change is within its partition");
            }
        }
    }
}

/// P§1: compose and invert. Two changes in turn compose to the change across both; a change then its inverse is the
/// identity; an inverse swaps gone and added.
#[test]
fn changes_compose_and_invert() {
    let (reg, ops, mut dir, mut t) = setup();
    let mut r = rng(5);
    let mut set = Rows::new();
    let mut heads = vec![None];
    for _ in 0..3 {
        let (pairs, next) = random_run(&mut r, &set);
        let id = runs(&mut dir, &t, &pairs);
        heads.push(Some(t.commit(&reg, &ops, &mut dir, id, Tiling::WHOLE).unwrap()));
        set = next;
    }
    let l = t.read(&dir, &[t.head()]).unwrap();
    let table = l.table();
    let ch = |a: usize, b: usize| t.change(&table, heads[a], heads[b]).unwrap();
    let ids = |b: &Bistate<_>| (b.gone().collect::<Vec<_>>(), b.added().collect::<Vec<_>>());
    let c = ch(1, 2).compose(ch(2, 3)).unwrap();
    check(&c);
    assert_eq!(ids(&c), ids(&ch(1, 3)), "1→2 then 2→3 is 1→3");
    assert!(ch(1, 2).compose(ch(1, 3)).is_err(), "a composed change starts where the first ends");
    let inv = ch(1, 3).invert();
    check(&inv);
    let (g, a) = ids(&ch(1, 3));
    assert_eq!(ids(&inv), (a, g), "the inverse swaps gone and added");
    let undo = ch(1, 3).compose(ch(1, 3).invert()).unwrap();
    check(&undo);
    assert!(undo.is_identity(), "a change and its undo compose to the identity");
    let cold = ch(0, 2).compose(ch(2, 3)).unwrap();
    check(&cold);
    assert_eq!(rows_of(cold.after()), set);
}

/// The committed state stays readable while the next layer is written: a reader's launch holds the head while a commit
/// runs over it, and reads the same rows after.
#[test]
fn the_committed_state_is_readable_while_the_next_is_written() {
    let (reg, ops, mut dir, mut t) = setup();
    let mut r = rng(11);
    let (pairs, set) = random_run(&mut r, &Rows::new());
    let id = runs(&mut dir, &t, &pairs);
    let h1 = t.commit(&reg, &ops, &mut dir, id, Tiling::WHOLE).unwrap();
    let reader = t.read(&dir, &[Some(h1)]).unwrap();
    let (pairs, next) = random_run(&mut r, &set);
    let id = runs(&mut dir, &t, &pairs);
    let h2 = t.commit(&reg, &ops, &mut dir, id, Tiling { tile: 3, seed: 1 }).expect("a commit beside a reader of the state below");
    let table = reader.table();
    assert_eq!(rows_of(&t.state(&table, Some(h1)).unwrap()), set, "the reader's state is as it was");
    drop(reader);
    let l = t.read(&dir, &[Some(h2)]).unwrap();
    assert_eq!(rows_of(&t.state(&l.table(), Some(h2)).unwrap()), next);
}

/// consolidate refuses what is not a set change, into control, and the head stays: a row gone that is not live, a row
/// added that is, a row's diffs past ±1; a run that outgrew its runs block is refused with the size to rerun at.
#[test]
fn consolidate_refuses_what_is_not_a_set_change() {
    let (reg, ops, mut dir, mut t) = setup();
    let id = runs(&mut dir, &t, &[(vec![1, 2, 3], 1), (vec![1, 2, 4], 1)]);
    let h = t.commit(&reg, &ops, &mut dir, id, Tiling::WHOLE).unwrap();
    for (pairs, why) in [
        (vec![(vec![1, 2, 5], -1)], "a row gone is not live"),
        (vec![(vec![1, 2, 3], 1)], "a row added is already live"),
        (vec![(vec![1, 2, 6], 1), (vec![1, 2, 6], 1)], "diffs sum past ±1"),
    ] {
        let id = runs(&mut dir, &t, &pairs);
        let e = t.commit(&reg, &ops, &mut dir, id, Tiling::WHOLE).unwrap_err();
        assert!(e.contains(why), "{e}");
        assert_eq!(t.head(), Some(h), "the head stays");
    }
    let id = t.open(&mut dir, 1);
    {
        let b = dir.block(id).unwrap();
        let pin = pin_block(&b, "writer", true).unwrap();
        let w = RunsW::new(&pin);
        assert!(w.push(&[1, 1, 1], 1).is_some());
        assert!(w.push(&[1, 1, 2], 1).is_none(), "past its room a push is counted, not written");
        assert!(w.push(&[1, 1, 3], 1).is_none());
    }
    let e = t.commit(&reg, &ops, &mut dir, id, Tiling::WHOLE).unwrap_err();
    assert!(e.contains("rerun at 3"), "{e}");
    let l = t.read(&dir, &[t.head()]).unwrap();
    assert_eq!(rows_of(&t.state(&l.table(), t.head()).unwrap()), [vec![1, 2, 3], vec![1, 2, 4]].into_iter().collect());
}

/// A roll-up merges the newest layers into one over the layer below them, with the same state, an add and its later
/// kill cancelled. A referenced generation lives until its referrer is rolled up; then it lives only while a reader
/// holds it.
#[test]
fn a_roll_up_merges_layers_and_lets_the_merged_generations_go() {
    let (reg, ops, mut dir, mut t) = setup();
    let mut r = rng(77);
    let mut set = Rows::new();
    let mut heads = Vec::new();
    for _ in 0..4 {
        let (pairs, next) = random_run(&mut r, &set);
        let id = runs(&mut dir, &t, &pairs);
        heads.push(t.commit(&reg, &ops, &mut dir, id, Tiling::WHOLE).unwrap());
        set = next;
    }
    assert!(heads.iter().all(|&h| dir.alive(h)), "each generation lives: the head refers to the one below, and so on");
    let reader = t.read(&dir, &[Some(heads[3])]).unwrap();
    let m = t.roll_up(&reg, &ops, &mut dir, 3, Tiling { tile: 2, seed: 8 }).unwrap();
    assert_eq!(dir.refs(m), vec![heads[0]], "the merged layer refers to the layer below the merged ones");
    assert!(dir.alive(heads[0]));
    assert!(dir.alive(heads[3]) && dir.alive(heads[1]), "a reader holds the merged generations");
    drop(reader);
    dir.sweep();
    assert!(!dir.alive(heads[3]) && !dir.alive(heads[2]) && !dir.alive(heads[1]), "rolled up, nothing holds them");
    let l = t.read(&dir, &[Some(m)]).unwrap();
    let table = l.table();
    let s = t.state(&table, Some(m)).unwrap();
    assert_eq!(rows_of(&s), set, "the roll-up's state is the state it rolled up");
    let (_, layer) = s.layers().next().unwrap();
    assert!(layer.n() <= set.len() + rows_of(&t.state(&table, Some(heads[0])).unwrap()).len(), "adds and their kills cancelled");
    check(&t.change(&table, Some(heads[0]), Some(m)).unwrap());
    drop(l);
    let m2 = t.roll_up(&reg, &ops, &mut dir, 2, Tiling::WHOLE).unwrap();
    assert!(!dir.alive(heads[0]) && !dir.alive(m), "a referenced generation lives until its referrer is rolled up");
    let l = t.read(&dir, &[Some(m2)]).unwrap();
    assert_eq!(rows_of(&t.state(&l.table(), Some(m2)).unwrap()), set);
}

/// O§13 for consolidate: the same layer, by its facts, at two tilings and with its tiles in shuffled order: the
/// check a version is registered by.
#[test]
fn consolidate_writes_the_same_layer_at_every_tiling() {
    let mut reg = Registry::default();
    let ops = TraceOps::declare(&mut reg).unwrap();
    let mut r = rng(3);
    let (pairs, _) = random_run(&mut r, &Rows::new());
    let s = shape();
    let tiled = Version { status: Status::Default, ..Version::twin(ops.consolidate, "consolidate, tiled", rerail_delta_twins::trace::CONSOLIDATE, &PIPELINE) };
    let checked = reg.check(&tiled, &consolidate_case(&s, &pairs), &[Tiling::WHOLE, Tiling { tile: 1, seed: 2 }, Tiling { tile: 4, seed: 6 }]).unwrap();
    reg.register(tiled, checked).unwrap();
}

/// A run block's trace refuses what the plan puts elsewhere, saying where: levels (Δfix, after L1), encodings other
/// than Open (a block that leaves its run), kept files and pinned slabs (after L1), the operator cache's retention
/// (B3) and History's named frontiers (two states). A partition not first in the order, or wider than 32 bits, is
/// refused.
#[test]
fn a_shape_the_plan_puts_elsewhere_is_refused_saying_where() {
    use rerail_blocks::layer::{Encoding, Placement, Retention};
    let refused = |f: &dyn Fn(&mut Shape), why: &str| {
        let mut s = shape();
        f(&mut s);
        let e = Trace::new(&s).err().unwrap();
        assert!(e.contains(why), "{e}");
    };
    refused(&|s| s.levels = true, "Δfix");
    refused(&|s| s.encoding = Encoding::Packed, "leaves its run");
    refused(&|s| s.placement = Placement::KeptFile, "kept stores");
    refused(&|s| s.placement = Placement::PinnedSlab, "device executor");
    refused(&|s| s.retention = Retention::Cache, "operator cache");
    refused(&|s| s.retention = Retention::History, "two states");
    refused(&|s| s.indexes = vec![rerail_blocks::layer::IndexSpec::Perm(vec![2])], "packed's permutation");
    let s = Shape::run(vec![ColType::I32, ColType::I64], vec![1, 0], Some(0));
    assert!(Trace::new(&s).err().unwrap().contains("first"));
    let s = Shape::run(vec![ColType::I64, ColType::I32], vec![0, 1], Some(0));
    assert!(Trace::new(&s).err().unwrap().contains("32 bits"));
}

/// The shape's indexes (P§11.2): an order with fences and prints, a hashed index and a Bloom filter,
/// written by consolidate and merge, at any tiling, and read by the probes: every probe of a random trace finds the
/// brute-force set's rows; a Bloom filter holds every key of its layer and rules out most absent keys.
#[test]
fn every_index_kind_answers_its_probes() {
    use rerail_blocks::layer::{IndexSpec, Layer};
    let mut s = shape();
    s.indexes = vec![
        IndexSpec::Order { key: vec![1], fences: true, prints: true },
        IndexSpec::Hash(vec![2, 1]),
        IndexSpec::Hash(vec![2]),
        IndexSpec::Bloom(vec![1, 2]),
    ];
    let mut reg = Registry::default();
    let ops = TraceOps::declare(&mut reg).unwrap();
    let mut dir = Directory::default();
    let mut t = Trace::new(&s).unwrap();
    let mut r = rng(21);
    let mut set = Rows::new();
    let tilings = [Tiling::WHOLE, Tiling { tile: 1, seed: 5 }, Tiling { tile: 3, seed: 0 }];
    for step in 0..5 {
        let (mut pairs, next) = random_run(&mut r, &set);
        // Many rows sharing a value of column 1, so its runs cross fences.
        for k in 0..200 {
            let x = vec![1, 7, k];
            if !set.contains(&x) && !next.contains(&x) && step == 0 {
                pairs.push((x, 1));
            }
        }
        let next: Rows = if step == 0 { next.into_iter().chain((0..200).map(|k| vec![1, 7, k])).collect() } else { next };
        let id = runs(&mut dir, &t, &pairs);
        t.commit(&reg, &ops, &mut dir, id, tilings[step % 3]).unwrap();
        set = next;
    }
    let check_probes = |dir: &Directory, t: &Trace, set: &Rows| {
        let l = t.read(dir, &[t.head()]).unwrap();
        let table = l.table();
        let st = t.state(&table, t.head()).unwrap();
        let probes: [&[u16]; 5] = [&[1], &[2, 1], &[2], &[1, 2], &[0, 1]];
        let mut keys: Vec<Vec<i64>> = set.iter().take(40).cloned().collect();
        keys.push(vec![1, 7, 3]);
        keys.push(vec![5, 123_456, 999]);
        for cols in probes {
            let cs: Vec<ColumnIndex> = cols.iter().map(|&c| ColumnIndex::new(c)).collect();
            for x in &keys {
                let key: Vec<i64> = cols.iter().map(|&c| x[c as usize]).collect();
                let got: Rows = st.matching(&cs, &key).map(|id| row(&st, id)).collect();
                let want: Rows = set.iter().filter(|y| cols.iter().zip(&key).all(|(&c, &k)| y[c as usize] == k)).cloned().collect();
                assert_eq!(got, want, "probe {cols:?} = {key:?}");
            }
        }
        // The Bloom filter: every key of a layer may be held; most absent keys are ruled out.
        let (mut present, mut ruled_out) = (0, 0);
        for (_, layer) in st.layers() {
            let layer: Layer<'_> = layer;
            for i in layer.rows.ids() {
                assert!(layer.may_hold(&[1, 2], &[layer.get(i, 1), layer.get(i, 2)]), "a layer's own key");
                present += 1;
            }
            ruled_out += (0..1000).filter(|k| !layer.may_hold(&[1, 2], &[-1_000_000 - k, 5])).count();
        }
        assert!(present > 0 && ruled_out > 900 * st.layers().count(), "most absent keys ruled out: {ruled_out}");
    };
    check_probes(&dir, &t, &set);
    t.roll_up(&reg, &ops, &mut dir, 3, Tiling { tile: 2, seed: 9 }).unwrap();
    check_probes(&dir, &t, &set);

    // The same indexed layer at every tiling and order.
    let (pairs, _) = random_run(&mut rng(8), &Rows::new());
    let tiled = Version { status: Status::Default, ..Version::twin(ops.consolidate, "consolidate, tiled", rerail_delta_twins::trace::CONSOLIDATE, &PIPELINE) };
    reg.check(&tiled, &consolidate_case(&s, &pairs), &[Tiling::WHOLE, Tiling { tile: 1, seed: 2 }, Tiling { tile: 5, seed: 6 }]).unwrap();
}

/// A chain of any depth is read whole: 14 layers with no roll-up scan, probe and change as their set does, and a
/// roll-up of 12 layers into one keeps the state.
#[test]
fn a_deep_chain_reads_as_its_set() {
    let (reg, ops, mut dir, mut t) = setup();
    let mut r = rng(99);
    let mut sets = vec![Rows::new()];
    let mut heads = vec![None];
    for _ in 0..14 {
        let (pairs, next) = random_run(&mut r, sets.last().unwrap());
        let id = runs(&mut dir, &t, &pairs);
        heads.push(Some(t.commit(&reg, &ops, &mut dir, id, Tiling::WHOLE).unwrap()));
        sets.push(next);
    }
    {
        let l = t.read(&dir, &[t.head()]).unwrap();
        let table = l.table();
        assert_eq!(rows_of(&t.state(&table, t.head()).unwrap()), sets[14]);
        for j in [0, 1, 5] {
            let b = t.change(&table, heads[j], heads[14]).unwrap();
            check(&b);
            let mut ps = Vec::new();
            b.changed_parts(&mut |p| ps.push(p));
            ps.sort();
            assert_eq!(ps, changed(&sets[j], &sets[14]), "{j}→14 across {} layers", 14 - j);
        }
    }
    t.roll_up(&reg, &ops, &mut dir, 12, Tiling { tile: 4, seed: 1 }).unwrap();
    let l = t.read(&dir, &[t.head()]).unwrap();
    assert_eq!(rows_of(&t.state(&l.table(), t.head()).unwrap()), sets[14]);
}

/// O§13 for merge: the same rolled-up layer at every tiling and with its tiles in shuffled order, by the registry's
/// check, over layers with every index kind.
#[test]
fn merge_writes_the_same_layer_at_every_tiling() {
    use rerail_blocks::layer::IndexSpec;
    let mut s = shape();
    s.indexes = vec![IndexSpec::Order { key: vec![1], fences: true, prints: true }, IndexSpec::Hash(vec![2]), IndexSpec::Bloom(vec![1, 2])];
    let mut r = rng(31);
    let (mut set, mut runs) = (Rows::new(), Vec::new());
    for _ in 0..4 {
        let (pairs, next) = random_run(&mut r, &set);
        runs.push(pairs);
        set = next;
    }
    let mut reg = Registry::default();
    let ops = TraceOps::declare(&mut reg).unwrap();
    let tiled = Version { status: Status::Default, ..Version::twin(ops.merge, "merge, tiled", rerail_delta_twins::trace::MERGE, &PIPELINE) };
    let tilings = [Tiling::WHOLE, Tiling { tile: 1, seed: 4 }, Tiling { tile: 3, seed: 17 }];
    let checked = reg.check(&tiled, &merge_case(&s, &runs, 3), &tilings).unwrap();
    reg.register(tiled, checked).unwrap();
}

/// The trace of `runs` committed in turn: its heads, and each state's set.
fn committed(runs_of: &[Vec<(Vec<i64>, i32)>]) -> (Registry, TraceOps, Directory, Trace, Vec<Option<BlockId>>) {
    let (reg, ops, mut dir, mut t) = setup();
    let mut heads = vec![None];
    for (k, pairs) in runs_of.iter().enumerate() {
        let id = runs(&mut dir, &t, pairs);
        heads.push(Some(t.commit(&reg, &ops, &mut dir, id, Tiling { tile: 3, seed: k as u64 }).unwrap_or_else(|e| panic!("run {k}: {e}"))));
    }
    (reg, ops, dir, t, heads)
}

/// P§1, P§11: a restriction reads only its partition's rows. Over a trace of a thousand rows in seven partitions and
/// a few in partition 50, the restricted state's scan and probe and the restricted change's rows gone and added touch
/// only partition 50's rows of each layer and its kills, counted where the reads are made (`View::counted`): rows,
/// kills, liveness words and search steps.
#[test]
fn a_restriction_reads_only_its_partitions_rows() {
    let mut r = rng(61);
    let mut set = Rows::new();
    let mut all = Vec::new();
    let mut sets = vec![Rows::new()];
    for step in 0..5i64 {
        let (mut pairs, mut next) = random_run(&mut r, &set);
        for k in 0..3 {
            let x = vec![50, step * 10 + k, k];
            pairs.push((x.clone(), 1));
            next.insert(x);
        }
        if step > 0 {
            let x = vec![50, (step - 1) * 10, 0];
            pairs.push((x.clone(), -1));
            next.remove(&x);
        }
        all.push(pairs);
        set = next;
        sets.push(set.clone());
    }
    let (_, _, dir, t, heads) = committed(&all);
    let l = t.read(&dir, &[heads[5]]).unwrap();
    let table = l.table();
    let reads = std::sync::atomic::AtomicU64::new(0);
    let whole = t.state(&table, heads[5]).unwrap();
    let rows_total: usize = whole.layers().map(|(_, l)| l.n()).sum();
    // The partition index (`parts`): each partition's range is its own rows exactly, its kills its kills of rows below.
    for (_, l) in whole.layers() {
        for p in (0..7).chain([50, 99, 1_000]) {
            let r = l.part_rows(p);
            assert!(r.clone().all(|id| l.get(id, 0) == p), "partition {p}'s range holds its rows only");
            assert_eq!(r.len(), (l.start()..l.end()).filter(|&id| l.get(id, 0) == p).count(), "and all of them");
            let k = l.part_kills(p);
            assert_eq!(k.len(), l.kills().iter().filter(|&&id| whole.get(id, 0) == p).count(), "partition {p}'s kills");
            assert!(k.windows(2).all(|w| w[0] < w[1]) && k.iter().all(|id| l.kills().binary_search(id).is_ok()));
        }
    }
    let s = whole.counted(&reads).restrict(50);
    let want: Rows = set.iter().filter(|x| x[0] == 50).cloned().collect();
    assert_eq!(rows_of(&s), want);
    let own: u64 = whole.layers().map(|(_, l)| l.part_rows(50).len() as u64).sum();
    let kills: u64 = whole.layers().map(|(_, l)| l.part_kills(50).len() as u64).sum();
    let scan = reads.swap(0, std::sync::atomic::Ordering::Relaxed);
    // And the searches that place the scan's cursors in the partition's kills of each layer above another (`Scan`):
    // each step a kill of partition 50's.
    let pk: Vec<u64> = whole.layers().map(|(_, l)| l.part_kills(50).len() as u64).collect();
    let log = |n: u64| u64::from(u64::BITS - n.leading_zeros());
    let searches: u64 = (0..pk.len()).map(|k| (0..k).map(|j| log(pk[j]) + 1).sum::<u64>()).sum();
    assert!(scan <= own + kills + searches, "a restricted scan read {scan} rows and kills, its partition's own rows being {own} (of {rows_total}), its kills {kills}, its searches {searches}");
    let cols: Vec<ColumnIndex> = ColumnIndex::all(3).collect();
    let x = want.iter().next().unwrap();
    assert_eq!(s.matching(&cols, x).count(), 1);
    let probe = reads.swap(0, std::sync::atomic::Ordering::Relaxed);
    // A search a layer among its partition's rows (two bounds), then a search of the partition's kills above the match.
    let psearch: u64 = whole.layers().map(|(_, l)| 2 * (log(l.part_rows(50).len() as u64) + 1)).sum();
    let ksearch: u64 = pk.iter().map(|&n| log(n) + 1).sum();
    assert!(probe <= psearch + 1 + ksearch, "a restricted probe read {probe} rows and kills: its searches {psearch} and {ksearch}");
    let before = t.state(&table, heads[3]).unwrap().counted(&reads).restrict(50);
    let b = Bistate::new(before, s);
    let gone: Rows = b.gone().map(|id| row(b.before().unwrap(), id)).collect();
    let added: Rows = b.added().map(|id| row(b.after(), id)).collect();
    let of50 = |s: &Rows| s.iter().filter(|x| x[0] == 50).cloned().collect::<Rows>();
    assert_eq!(gone, of50(&sets[3]).difference(&of50(&sets[5])).cloned().collect(), "partition 50's rows gone, 3→5");
    assert_eq!(added, of50(&sets[5]).difference(&of50(&sets[3])).cloned().collect(), "and added");
    let change = reads.swap(0, std::sync::atomic::Ordering::Relaxed);
    assert!(change <= 4 * (own + kills), "a restricted change read {change} rows and words, its partition's being {}", own + kills);
    // Each read above is partition 50's: its rows, its kills, and searches within its ranges (each bound in its terms
    // alone), the partition index's own entries aside. Together they were 80 of the trace's 770 rows (2026-11-25).
}

/// P§1: whether a change is the identity is read from a layer's counts, touching no row: a layer of rows, a layer
/// whose run cancelled (an add and its undo: no rows), a layer that only kills, each whole and restricted.
#[test]
fn is_identity_reads_a_layers_counts_only() {
    let (one, set) = random_run(&mut rng(4), &Rows::new());
    let cancel = vec![(vec![2, 5, 5], 1), (vec![2, 5, 5], -1)];
    let gone: Vec<Vec<i64>> = set.iter().take(4).cloned().collect();
    let kill_only: Vec<(Vec<i64>, i32)> = gone.iter().map(|x| (x.clone(), -1)).collect();
    let (_, _, dir, t, heads) = committed(&[one, cancel, kill_only]);
    let l = t.read(&dir, &[heads[3]]).unwrap();
    let table = l.table();
    let reads = std::sync::atomic::AtomicU64::new(0);
    let st = |k: usize| t.state(&table, heads[k]).unwrap().counted(&reads);
    for (k, identity) in [(1, false), (2, true), (3, false)] {
        let b = Bistate::new(st(k - 1), st(k));
        assert_eq!(b.is_identity(), identity, "layer {k}");
        let p = gone[0][0];
        let restricted = b.restrict(p);
        assert_eq!(restricted.is_identity(), k == 2, "layer {k}, partition {p}");
        assert!(b.restrict(1_000).is_identity(), "layer {k}, a partition it does not hold");
        assert_eq!(reads.load(std::sync::atomic::Ordering::Relaxed), 0, "layer {k}: is_identity read no row");
    }
    let l3 = st(3);
    let (_, top) = l3.layers().next().unwrap();
    assert_eq!((top.n(), top.kills().len()), (0, 4), "a layer that only kills");
    let after: Rows = set.iter().filter(|x| !gone.contains(x)).cloned().collect();
    assert_eq!(rows_of(&l3), after);
    let b = t.change(&table, heads[2], heads[3]).unwrap();
    check(&b);
    assert_eq!(b.gone().map(|id| row(b.before().unwrap(), id)).collect::<Vec<_>>(), gone);
}

/// The kill search's cost (consolidate's `kills`: a probe of the state below on the trace's canonical order, its first
/// live match): over a trace of eight runs, each row of the state's probe reads, counted where they are made (each
/// row it compares in a layer's binary search, each candidate, each step of a search of a layer's kills above it),
/// at most two searches a layer and a kill search a candidate's layer above: O(layers · log rows), not the state's
/// rows.
#[test]
fn a_kill_search_reads_a_search_a_layer() {
    let mut r = rng(83);
    let mut set = Rows::new();
    let mut all = Vec::new();
    for _ in 0..8 {
        let (pairs, next) = random_run(&mut r, &set);
        all.push(pairs);
        set = next;
    }
    let (_, _, dir, t, heads) = committed(&all);
    let l = t.read(&dir, &[heads[8]]).unwrap();
    let table = l.table();
    let whole = t.state(&table, heads[8]).unwrap();
    let layers: Vec<(usize, usize)> = whole.layers().map(|(_, l)| (l.n(), l.kills().len())).collect();
    let log = |n: usize| (usize::BITS - n.leading_zeros()) as u64;
    let search: u64 = layers.iter().map(|&(n, _)| 2 * (log(n) + 1)).sum();
    let kill_search: u64 = layers.iter().map(|&(_, k)| log(k) + 1).sum();
    let bound = search + layers.len() as u64 * (1 + kill_search);
    let rows_total: usize = layers.iter().map(|&(n, _)| n).sum();
    let (_, top) = whole.layers().next().unwrap();
    let cols: Vec<ColumnIndex> = top.head.shape.order().iter().map(|&c| ColumnIndex::new(u16::from(c))).collect();
    let reads = std::sync::atomic::AtomicU64::new(0);
    let s = whole.counted(&reads);
    let (mut sum, mut max) = (0u64, 0u64);
    for x in &set {
        let key: Vec<i64> = cols.iter().map(|c| x[c.get()]).collect();
        let id = s.matching(&cols, &key).next().expect("a live row's live copy");
        assert_eq!(row(&whole, id), *x);
        let n = reads.swap(0, std::sync::atomic::Ordering::Relaxed);
        assert!(n <= bound, "a kill search read {n}, past {bound} (layers {layers:?})");
        sum += n;
        max = max.max(n);
    }
    let mean = sum / set.len() as u64;
    eprintln!("kill search: {} layers {layers:?}, {rows_total} rows; reads a key: mean {mean}, max {max}, bound {bound}", layers.len());
}
