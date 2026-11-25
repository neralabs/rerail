//! B2's checks (*0686: Operations* §7, §10; P§1, P§11–§12): a trace of random runs against a brute-force set, its
//! states and changes read where the layers lie; the committed state readable while the next layer is written; a
//! change's partitions in its size; compose and invert; the sketches; consolidate's refusals; a roll-up, and the
//! merged generations let go (a referenced generation lives until its referrer is rolled up); the same layer at
//! every tiling and order.

use super::*;
use crate::op::{Registry, Status, Tiling};
use rerail_blocks::bistate::{ColumnIndex, Read, check};
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
        if r() % 4 == 0 {
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
    let (reg, ops, mut dir, mut t) = setup();
    let mut r = rng(42);
    let mut sets: Vec<Rows> = vec![Rows::new()];
    let mut heads: Vec<Option<BlockId>> = vec![None];
    let tilings = [Tiling::WHOLE, Tiling { tile: 1, seed: 9 }, Tiling { tile: 5, seed: 0 }, Tiling { tile: 2, seed: 4 }];
    for step in 0..6 {
        let (pairs, next) = random_run(&mut r, sets.last().unwrap());
        let id = runs(&mut dir, &t, &pairs);
        let h = t.commit(&reg, &ops, &mut dir, id, tilings[step % tilings.len()]).unwrap_or_else(|e| panic!("step {step}: {e}"));
        sets.push(next);
        heads.push(Some(h));
    }
    let l = t.read(&dir, &[t.head()]).unwrap();
    let table = l.table();
    for (k, set) in sets.iter().enumerate() {
        let s = t.state(&table, heads[k]).unwrap();
        assert_eq!(&rows_of(&s), set, "state {k} is its set");
        for p in parts_of(set).into_iter().chain([3, 99]) {
            let want: Rows = set.iter().filter(|x| x[0] == p).cloned().collect();
            assert_eq!(rows_of(&rerail_blocks::bistate::States::restrict(&s, p)), want, "state {k}, partition {p}");
        }
        if let Some(x) = set.iter().next() {
            let cols: Vec<ColumnIndex> = ColumnIndex::all(3).collect();
            let found: Vec<RowId> = s.matching(&cols, x).collect();
            assert_eq!(found.len(), 1, "a probe finds the one live row");
            assert_eq!(&row(&s, found[0]), x);
        }
        for c in ColumnIndex::all(3) {
            let st = s.stats(c);
            assert_eq!(st.count, set.len() as i64, "state {k}: the sketch counts the live rows");
            if let Some(m) = set.iter().map(|x| x[c.get() as usize]).min() {
                assert!(st.min <= m, "the sketch's min bounds the live rows'");
            }
        }
    }
    for k in 1..sets.len() {
        for j in 0..k {
            let b = t.change(&table, heads[j], heads[k]).unwrap();
            check(&b);
            let gone: Rows = b.gone().map(|id| row(b.before().unwrap(), id)).collect();
            let added: Rows = b.added().map(|id| row(b.after(), id)).collect();
            assert_eq!(gone, sets[j].difference(&sets[k]).cloned().collect::<Rows>(), "{j}→{k}: gone");
            assert_eq!(added, sets[k].difference(&sets[j]).cloned().collect::<Rows>(), "{j}→{k}: added");
            assert_eq!(b.counts(), (gone.len(), added.len()));
            let mut ps = Vec::new();
            b.changed_parts(&mut |p| ps.push(p));
            ps.sort();
            assert_eq!(ps, changed(&sets[j], &sets[k]), "{j}→{k}: the changed partitions");
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
    let layer = s.layers().next().unwrap();
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

/// What a trace cannot store yet is refused by name; a partition not first in the order is refused.
#[test]
fn a_shape_not_built_is_refused_by_name() {
    let mut s = shape();
    s.levels = true;
    assert!(Trace::new(&s).err().unwrap().contains("not built"));
    let s = Shape::run(vec![ColType::I32, ColType::I64], vec![1, 0], Some(0));
    assert!(Trace::new(&s).err().unwrap().contains("first"));
}
