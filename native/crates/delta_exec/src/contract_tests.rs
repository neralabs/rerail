//! The harness on a toy operator, and on four toys each built to break one law (B3).

use super::*;
use rerail_blocks::bistate::RowId;
use rerail_blocks::layer::ColType;

/// What a toy gets wrong.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Flaw {
    /// Nothing: the good toy.
    None,
    /// Drops the rows gone from its input: breaks the square (L1), not a null start.
    Kills,
    /// Writes a row of its own on a cold run: breaks the null start (L2).
    Cold,
    /// Reads its whole input when nothing changed: breaks "identity is free" (L4), its answers right.
    Busy,
    /// Drops the rows gone when its cache reads as ∅: its answer hangs on its cache (L5).
    Cache,
    /// Under a want, writes a row of its own on a cold run, where nothing is wanted: past its want, a row the full
    /// output lacks (D1).
    Past,
}

/// `out(b, a) :- in(a, b), a % 3 != 0`, its added rows read in the order its cache keeps of its input's newest layer.
struct Toy(Flaw);

fn keep(a: i64) -> bool {
    a % 3 != 0
}

fn in_shape() -> Shape {
    Shape::run(vec![ColType::I32, ColType::I64], vec![0, 1], None)
}

fn out_shape() -> Shape {
    Shape::run(vec![ColType::I64, ColType::I32], vec![0, 1], None)
}

fn cell(s: &impl Read, id: RowId, c: u16) -> i64 {
    s.get(id, ColumnIndex::new(c))
}

fn live_ids(s: &impl Read) -> impl Iterator<Item = RowId> + '_ {
    (0..s.end().get()).map(RowId::new).filter(|&id| s.live(id))
}

/// The toys' port set: one input, one output.
struct ToyPorts;

const IN: PortIn<ToyPorts> = PortIn::new(0);
const OUT: PortOut<ToyPorts> = PortOut::new(0);

impl PortSet for ToyPorts {
    const INS: usize = 1;
    const GLOBALS: usize = 0;
    const OUTS: usize = 1;

    fn ports() -> Ports {
        Ports { ins: vec![Port { name: "in", shape: in_shape() }], globals: Vec::new(), outs: vec![Port { name: "out", shape: out_shape() }] }
    }
}

impl DeltaOp for Toy {
    type Ports = ToyPorts;
    type Cache = crate::cache::Orders;

    fn delta(&self, cx: &mut Cx<'_>, ins: &In<'_, ToyPorts>, want: Option<&Bistate<View<'_>>>, cache: &mut CacheW<crate::cache::Orders>, out: &mut Out<ToyPorts>, _: &mut Asks<ToyPorts>) -> Result<Work, String> {
        let x = ins.port(IN);
        let mut work = Work::default();
        if ins.is_identity() {
            if self.0 == Flaw::Busy {
                work.rows_read = live_ids(x.after()).count() as u64;
            }
            return Ok(work);
        }
        let held = cache.read(cx)?.is_some();
        let w = out.writer(cx.dir, OUT)?;
        let skip_gone = self.0 == Flaw::Kills || (self.0 == Flaw::Cache && !held && x.before().is_some());
        if let Some(b) = x.before().filter(|_| !skip_gone) {
            for id in x.gone() {
                work.rows_read += 1;
                let a = cell(b, id, 0);
                if keep(a) {
                    w.push(&[cell(b, id, 1), a], -1);
                    work.rows_written += 1;
                }
            }
        }
        if self.0 == Flaw::Cold && x.before().is_none() {
            w.push(&[-1, -1], 1);
        }
        if self.0 == Flaw::Past && want.is_some() && x.before().is_none() {
            w.push(&[-2, -2], 1);
        }
        // The added rows, each layer's between the two states read in its order by the second column, from the
        // cache: made once a layer, and kept while the cache holds its state.
        let added = x.added().count();
        let mut layers = Vec::new();
        {
            let (before, after) = ins.heads(IN);
            let heads: Vec<BlockId> = after.into_iter().collect();
            let l = cx.dir.read("the toy's layers", &heads)?;
            let table = l.table();
            let mut at = after.map(|h| h.0);
            while let Some(id) = at.filter(|&id| Some(BlockId(id)) != before) {
                layers.push(BlockId(id));
                at = rerail_blocks::layer::Layer::new(table.block(id).ok_or("a layer below")?)?.below().map(|(b, _)| b);
            }
        }
        if layers.is_empty() {
            return Ok(work);
        }
        let c = cache.write(cx)?;
        let a = x.after();
        let mut seen = 0;
        for layer in layers {
            OpCache(c).order(cx.ops, cx.dir, layer, &[1])?;
            let read = cache.body(cx, |orders| -> Result<(), String> {
                let order = orders.order(layer, &[1]).ok_or("the order just added")?;
                for &id in order.ids {
                    let id = RowId::new(id);
                    if !Read::live(a, id) || x.before().is_some_and(|b| Read::live(b, id)) {
                        continue;
                    }
                    seen += 1;
                    work.probes += 1;
                    if keep(cell(a, id, 0)) {
                        w.push(&[cell(a, id, 1), cell(a, id, 0)], 1);
                        work.rows_written += 1;
                    }
                }
                Ok(())
            })?;
            read.ok_or("the cache this run writes, read as ∅")??;
        }
        if seen != added {
            return Err(format!("the newest layer's order holds {seen} of the {added} rows added"));
        }
        Ok(work)
    }

    fn run(&self, cx: &mut Cx<'_>, ins: &At<'_, ToyPorts>, out: &mut Out<ToyPorts>) -> Result<(), String> {
        let x = ins.port(IN);
        let w = out.writer(cx.dir, OUT)?;
        for id in live_ids(x) {
            let a = cell(x, id, 0);
            if keep(a) {
                w.push(&[cell(x, id, 1), a], 1);
            }
        }
        Ok(())
    }
}

fn row(w: u64) -> Vec<i64> {
    vec![(w % 40) as i64, ((w >> 8) % 1000) as i64]
}

fn chain(seed: u64) -> Chain {
    random_chain(1, 9, seed, &row)
}

#[test]
fn the_harness_passes_a_toy_operator_over_random_edit_chains() {
    for seed in 0..6 {
        for drop_at in [1, 2, 5, 8] {
            if let Err(b) = harness(&Toy(Flaw::None), &chain(seed), drop_at) {
                panic!("seed {seed}, cache dropped at {drop_at}: {b:?}");
            }
        }
    }
}

/// Each broken toy fails, on its law and no other, for some chain (a law is broken only where an edit reaches it).
fn fails_on(flaw: Flaw, law: &str) {
    let mut failed = 0;
    for seed in 0..6 {
        for drop_at in [2, 3, 5] {
            match harness(&Toy(flaw), &chain(seed), drop_at) {
                Ok(()) => {}
                Err(b) => {
                    assert_eq!(b.law, law, "{flaw:?}, seed {seed}: {b:?}");
                    failed += 1;
                }
            }
        }
    }
    assert!(failed > 0, "{flaw:?} passed every chain; it breaks {law}");
}

#[test]
fn a_toy_that_drops_its_kills_fails_the_square() {
    fails_on(Flaw::Kills, "L1");
}

#[test]
fn a_toy_with_a_row_of_its_own_on_a_cold_run_fails_the_null_start() {
    fails_on(Flaw::Cold, "L2");
}

#[test]
fn a_toy_that_reads_when_nothing_changed_fails_identity_is_free() {
    fails_on(Flaw::Busy, "L4");
}

#[test]
fn a_toy_whose_answer_hangs_on_its_cache_fails_the_dropped_cache() {
    fails_on(Flaw::Cache, "L5");
}

/// The reference is the plain function run on both sides, diffed: a row in both is in neither side of the change.
#[test]
fn the_reference_is_both_sides_run_and_diffed() {
    let reg = crate::op::Runner::new().unwrap();
    let mut dir = Directory::default();
    let mut t = Trace::new(&in_shape()).unwrap().with_roll_up(RollUp::Never);
    let commit = |dir: &mut Directory, t: &mut Trace, pairs: &[(Vec<i64>, i32)]| {
        let id = t.open(dir, pairs.len());
        let b = dir.block(id).unwrap();
        let pin = pin_block(&b, "test", true).unwrap();
        pairs.iter().for_each(|(r, d)| assert!(RunsW::new(&pin).push(r, *d).is_some()));
        drop(pin);
        t.commit(&reg, dir, id, Tiling::WHOLE).unwrap()
    };
    let h1 = commit(&mut dir, &mut t, &[(vec![1, 10], 1), (vec![2, 20], 1), (vec![3, 30], 1)]);
    let h2 = commit(&mut dir, &mut t, &[(vec![1, 10], -1), (vec![4, 40], 1), (vec![6, 60], 1)]);
    let mut cx = Cx { ops: &reg, dir: &mut dir, blobs: &crate::files::BlobRows::new() };
    let r = reference(&Toy(Flaw::None), &mut cx, &[(&t, Some(h1), Some(h2))], &[]).unwrap();
    assert_eq!(r[0].added, [vec![40, 4]].into_iter().collect());
    assert_eq!(r[0].gone, [vec![10, 1]].into_iter().collect());
}

/// An instance's identity run writes nothing: no layer committed, the cache's header as it was, no work.
#[test]
fn an_identity_run_is_its_checks_alone() {
    let reg = crate::op::Runner::new().unwrap();
    let mut dir = Directory::default();
    let mut t = Trace::new(&in_shape()).unwrap().with_roll_up(RollUp::Never);
    let id = t.open(&mut dir, 2);
    {
        let b = dir.block(id).unwrap();
        let pin = pin_block(&b, "test", true).unwrap();
        assert!(RunsW::new(&pin).push(&[1, 10], 1).is_some());
    }
    let h = t.commit(&reg, &mut dir, id, Tiling::WHOLE).unwrap();
    let mut cx = Cx { ops: &reg, dir: &mut dir, blobs: &crate::files::BlobRows::new() };
    let mut inst = Instance::new(&Toy(Flaw::None), cx.dir).unwrap();
    inst.step(&Toy(Flaw::None), &mut cx, &[(&t, None, Some(h))], &[], None).unwrap();
    let (heads, cache) = (inst.heads(), cache_facts(&cx.dir.read("t", &[inst.cache.0]).unwrap().table(), 0, cx.dir.read("t", &[inst.cache.0]).unwrap().block(0)));
    let w = inst.step(&Toy(Flaw::None), &mut cx, &[(&t, Some(h), Some(h))], &[], None).unwrap();
    assert!(w.is_checks_only(), "{w:?}");
    assert_eq!(inst.heads(), heads);
    assert_eq!(cache_facts(&cx.dir.read("t", &[inst.cache.0]).unwrap().table(), 0, cx.dir.read("t", &[inst.cache.0]).unwrap().block(0)), cache);
}

/// A run's cache reads as ∅ when its header was left open (a run that died part-way) or names another state than the
/// run's before; and as the cache when it names that state.
#[test]
fn a_runs_cache_left_open_or_of_another_state_reads_as_empty() {
    let reg = crate::op::Runner::new().unwrap();
    let mut dir = Directory::default();
    let c = OpCache::new(&mut dir);
    let cx = Cx { ops: &reg, dir: &mut dir, blobs: &crate::files::BlobRows::new() };
    c.open(cx.ops, cx.dir, 0).unwrap();
    c.commit(cx.ops, cx.dir, 7).unwrap();
    assert_eq!(CacheW::<()>::new(c, 7).read(&cx).unwrap(), Some(c.0));
    assert_eq!(CacheW::<()>::new(c, 8).read(&cx).unwrap(), None, "another state");
    c.open(cx.ops, cx.dir, 7).unwrap();
    assert_eq!(CacheW::<()>::new(c, 7).read(&cx).unwrap(), None, "left open");
}

/// A port set whose declared ports are not its counts (P§12's `type Ports`): its operator's instance is refused.
#[test]
fn a_port_set_whose_ports_are_not_its_counts_is_refused() {
    struct Two;
    impl PortSet for Two {
        const INS: usize = 2;
        const GLOBALS: usize = 0;
        const OUTS: usize = 1;
        fn ports() -> Ports {
            ToyPorts::ports()
        }
    }
    struct Bad;
    impl DeltaOp for Bad {
        type Ports = Two;
        type Cache = ();
        fn delta(&self, _: &mut Cx<'_>, _: &In<'_, Two>, _: Option<&Bistate<View<'_>>>, _: &mut CacheW<()>, _: &mut Out<Two>, _: &mut Asks<Two>) -> Result<Work, String> {
            Err("never run".into())
        }
        fn run(&self, _: &mut Cx<'_>, _: &At<'_, Two>, _: &mut Out<Two>) -> Result<(), String> {
            Err("never run".into())
        }
    }
    let e = Instance::new(&Bad, &mut Directory::default()).map(|_| ()).unwrap_err();
    assert!(e.contains("not its counts"), "{e}");
}

/// An operator of port set `P` that never runs: what `Instance::new` says of its ports.
fn ports_verdict<P: PortSet>() -> Result<(), String> {
    struct Never<P>(std::marker::PhantomData<P>);
    impl<P: PortSet> DeltaOp for Never<P> {
        type Ports = P;
        type Cache = ();
        fn delta(&self, _: &mut Cx<'_>, _: &In<'_, P>, _: Option<&Bistate<View<'_>>>, _: &mut CacheW<()>, _: &mut Out<P>, _: &mut Asks<P>) -> Result<Work, String> {
            Err("never run".into())
        }
        fn run(&self, _: &mut Cx<'_>, _: &At<'_, P>, _: &mut Out<P>) -> Result<(), String> {
            Err("never run".into())
        }
    }
    Instance::new(&Never::<P>(std::marker::PhantomData), &mut Directory::default()).map(|_| ())
}

/// L9 by column types: an id local to a partition (a node of its document's tape) is a column of a partitioned
/// relation, not its partition; an operator across partitions whose ports carry one is refused, its input's or its
/// output's; a single-file wave's ports are partitioned. L1, a single-file wave, carries the K-form's node ids.
#[test]
fn a_shape_carrying_a_node_out_of_its_partition_is_refused() {
    use crate::kform::shape;
    use rerail_blocks::layer::{ColType, Shape};
    fn two() -> Shape {
        Shape::run(vec![ColType::I32, ColType::I32], vec![0, 1], Some(0))
    }
    assert!(two().with_local(&[1]).encode().is_ok());
    assert!(Shape::run(vec![ColType::I32, ColType::I32], vec![0, 1], None).with_local(&[1]).encode().unwrap_err().contains("L9"), "a local id with no partition");
    assert!(two().with_local(&[0]).encode().unwrap_err().contains("L9"), "the partition column is no local id");
    assert!(two().with_local(&[2]).encode().unwrap_err().contains("L9"), "past the columns");
    // Across partitions, a K-form relation (its rule ids) out, or in.
    struct Out1;
    impl PortSet for Out1 {
        const INS: usize = 1;
        const GLOBALS: usize = 0;
        const OUTS: usize = 1;
        fn ports() -> Ports {
            Ports { ins: vec![Port { name: "values", shape: two() }], globals: vec![], outs: vec![Port { name: "k_rule", shape: shape(0) }] }
        }
    }
    struct In1;
    impl PortSet for In1 {
        const INS: usize = 1;
        const GLOBALS: usize = 0;
        const OUTS: usize = 1;
        fn ports() -> Ports {
            Ports { ins: vec![Port { name: "k_arg", shape: shape(2) }], globals: vec![], outs: vec![Port { name: "values", shape: two() }] }
        }
    }
    let e = ports_verdict::<Out1>().unwrap_err();
    assert!(e.contains("L9") && e.contains("k_rule"), "{e}");
    let e = ports_verdict::<In1>().unwrap_err();
    assert!(e.contains("L9") && e.contains("k_arg") && e.contains("[1, 5]"), "{e}");
    // A single-file wave: the same ports pass; one not partitioned is refused.
    struct Wave;
    impl PortSet for Wave {
        const INS: usize = 1;
        const GLOBALS: usize = 0;
        const OUTS: usize = 1;
        const PER_PARTITION: bool = true;
        fn ports() -> Ports {
            Out1::ports()
        }
    }
    ports_verdict::<Wave>().unwrap();
    struct Flat;
    impl PortSet for Flat {
        const INS: usize = 1;
        const GLOBALS: usize = 0;
        const OUTS: usize = 1;
        const PER_PARTITION: bool = true;
        fn ports() -> Ports {
            Ports { ins: vec![Port { name: "values", shape: Shape::run(vec![ColType::I32], vec![0], None) }], globals: vec![], outs: vec![Port { name: "k_rule", shape: shape(0) }] }
        }
    }
    assert!(ports_verdict::<Flat>().unwrap_err().contains("partitioned"));
    ports_verdict::<crate::l1::L1Ports>().unwrap();
}

/// L9's columns where a run's traces lie: the K-form's node ids (each relation's rule and term ids, `ID_COLS`) are its
/// shapes' local columns; what crosses partitions or comes from the world carries none: the document set, the blobs,
/// the world's leaves, answers and places, the want and the asks.
#[test]
fn node_ids_are_local_columns_of_the_kform_alone() {
    for k in 0..crate::kform::RELATIONS.len() {
        let want: Vec<u16> = crate::kform::ID_COLS[k].iter().map(|&c| c as u16).collect();
        assert_eq!(crate::kform::shape(k).local, want, "{}", crate::kform::RELATIONS[k].0);
    }
    let crossing = [
        ("the document set", crate::l1::tape_shape()),
        ("the blobs", crate::files::blobs_shape()),
        ("the leaves", crate::files::leaves_shape()),
        ("the answers", crate::files::answers_shape()),
        ("the places", crate::files::places_shape()),
        ("the want", crate::contract::want_shape()),
        ("the asks", crate::contract::asks_shape()),
    ];
    for (name, s) in crossing {
        assert!(s.local.is_empty(), "{name} carries ids local to a partition: {:?}", s.local);
        s.encode().unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

/// The want at each of a chain's ten states: the toy's one output wanted, then not, in turn by `seed`.
fn wants(seed: u64, steps: usize) -> Vec<BTreeSet<(usize, i64)>> {
    (0..=steps).map(|k| if rerail_blocks::layer::mix(seed + k as u64).is_multiple_of(3) { BTreeSet::new() } else { BTreeSet::from([(0, 0)]) }).collect()
}

/// D1 on the toy: under a want that comes and goes, its output restricted to the want is the full output restricted,
/// over-demand changes nothing asked, and every other law holds on the restricted outputs.
#[test]
fn the_harness_passes_a_toy_operator_under_a_changing_want() {
    for seed in 0..6 {
        for drop_at in [1, 5] {
            let w = wants(seed, 9);
            if let Err(b) = harness_under(&Toy(Flaw::None), &chain(seed), drop_at, Some(&w)) {
                panic!("seed {seed}, cache dropped at {drop_at}: {b:?}");
            }
        }
    }
}

/// A toy that writes a row past its want, one the full output lacks, fails D1 and no other law.
#[test]
fn a_toy_that_writes_past_its_want_fails_d1() {
    let none = vec![BTreeSet::new(); 10];
    let b = harness_under(&Toy(Flaw::Past), &chain(0), 2, Some(&none)).unwrap_err();
    assert_eq!(b.law, "D1", "{b:?}");
    harness_under(&Toy(Flaw::Past), &chain(0), 2, None).unwrap();
}

/// `out(p, v) :- in(p, v)` per partition `p`, and three flawed copies.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    /// Nothing: the good toy.
    None,
    /// Writes a row of partition 99 at each warm step, a partition its input's change never names (L9, checked in the
    /// validation mode as it runs).
    Other,
    /// Its plain function also gives each row's partition the count of every partition's rows: it reads across
    /// partitions, though its ports' types allow it (L9 by meaning, `by_partition`).
    Leak,
    /// Reads every live row of its input on each warm step, its answer right: its work grows with the partitions an
    /// edit leaves alone (L7's work).
    Scan,
}

struct PerPart(Part);

struct PerPartPorts;

fn parted() -> Shape {
    Shape::run(vec![ColType::I32, ColType::I32], vec![0, 1], Some(0))
}

impl PortSet for PerPartPorts {
    const INS: usize = 1;
    const GLOBALS: usize = 0;
    const OUTS: usize = 1;
    const PER_PARTITION: bool = true;

    fn ports() -> Ports {
        Ports { ins: vec![Port { name: "in", shape: parted() }], globals: Vec::new(), outs: vec![Port { name: "out", shape: parted() }] }
    }
}

impl DeltaOp for PerPart {
    type Ports = PerPartPorts;
    type Cache = crate::cache::Orders;

    fn delta(&self, cx: &mut Cx<'_>, ins: &In<'_, PerPartPorts>, _: Option<&Bistate<View<'_>>>, _: &mut CacheW<crate::cache::Orders>, out: &mut Out<PerPartPorts>, _: &mut Asks<PerPartPorts>) -> Result<Work, String> {
        let x = ins.port(PortIn::new(0));
        let w = out.writer(cx.dir, PortOut::new(0))?;
        if let Some(b) = x.before() {
            for id in x.gone() {
                w.push(&[cell(b, id, 0), cell(b, id, 1)], -1);
            }
        }
        for id in x.added() {
            w.push(&[cell(x.after(), id, 0), cell(x.after(), id, 1)], 1);
        }
        if self.0 == Part::Other && x.before().is_some() {
            w.push(&[99, Read::end(x.after()).get() as i64], 1);
        }
        if self.0 == Part::Scan && x.before().is_some() {
            std::hint::black_box(live_ids(x.after()).map(|id| cell(x.after(), id, 1)).sum::<i64>());
        }
        Ok(Work::default())
    }

    fn run(&self, cx: &mut Cx<'_>, ins: &At<'_, PerPartPorts>, out: &mut Out<PerPartPorts>) -> Result<(), String> {
        let x = ins.port(PortIn::new(0));
        let w = out.writer(cx.dir, PortOut::new(0))?;
        let n = live_ids(x).count() as i64;
        let mut parts = BTreeSet::new();
        for id in live_ids(x) {
            w.push(&[cell(x, id, 0), cell(x, id, 1)], 1);
            if self.0 == Part::Leak && parts.insert(cell(x, id, 0)) {
                w.push(&[cell(x, id, 0), -n], 1);
            }
        }
        Ok(())
    }
}

/// The validation mode refuses a per-partition operator whose step changes a partition its inputs' change does not
/// name (L9, checked as it runs), and passes the same operator without the flaw.
#[test]
fn a_per_partition_step_changing_another_partition_is_refused_in_the_validation_mode() {
    for flawed in [false, true] {
        let mut bench = Bench::new().unwrap();
        bench.dir.validate(true);
        let mut t = Trace::new(&parted()).unwrap().with_roll_up(RollUp::Never);
        let op = PerPart(if flawed { Part::Other } else { Part::None });
        let mut inst = Instance::new(&op, &mut bench.dir).unwrap();
        t.write(&bench.ops, &mut bench.dir, &[(vec![1, 10], 1), (vec![2, 20], 1), (vec![3, 30], 1)]).unwrap();
        Trace::commit_open(&bench.ops, &mut bench.dir, &mut [&mut t], Tiling::WHOLE).unwrap();
        let h1 = t.head();
        inst.step(&op, &mut bench.cx(), &[(&t, None, h1)], &[], None).unwrap();
        t.write(&bench.ops, &mut bench.dir, &[(vec![2, 20], -1), (vec![2, 21], 1)]).unwrap();
        Trace::commit_open(&bench.ops, &mut bench.dir, &mut [&mut t], Tiling::WHOLE).unwrap();
        let ran = inst.step(&op, &mut bench.cx(), &[(&t, h1, t.head())], &[], None);
        match flawed {
            false => ran.unwrap(),
            true => {
                let e = ran.unwrap_err();
                assert!(e.contains("L9") && e.contains("[99]") && e.contains("{2}"), "{e}");
                continue;
            }
        };
    }
}

fn part_row(w: u64) -> Vec<i64> {
    vec![(w % 5) as i64, ((w >> 8) % 1000) as i64]
}

/// The harness passes the per-partition toy, L9 by meaning among its laws, and fails the one whose plain function
/// reads across partitions, on L9 and no other.
#[test]
fn a_per_partition_toy_passes_and_one_reading_across_partitions_fails_l9() {
    for seed in 0..4 {
        let c = random_chain(1, 8, seed, &part_row);
        harness(&PerPart(Part::None), &c, 3).unwrap_or_else(|b| panic!("seed {seed}: {b:?}"));
        let b = harness(&PerPart(Part::Leak), &c, 3).unwrap_err();
        assert!(b.law == "L9" && b.what.contains("reads across partitions"), "seed {seed}: {b:?}");
    }
}

/// L7's work, by the validation mode's ledger (the operator's host reads among it): an edit of one partition beside
/// 4 to 64 others costs the per-partition toy what it costs beside 4; the toy that scans its input grows with them.
#[test]
fn a_per_partition_operators_work_does_not_grow_with_the_partitions_an_edit_leaves() {
    let sizes = [4i64, 8, 16, 32, 64];
    let row = |_: usize, q: i64, i: u64| vec![q, i as i64];
    for (flaw, grows) in [(Part::None, false), (Part::Scan, true)] {
        let runs: Vec<Vec<crate::dir::PhaseWork>> = sizes.iter().map(|&n| partition_work(&PerPart(flaw), n, 4, &row).unwrap()).collect();
        assert!(runs[0].iter().any(|w| w.op == "PerPart" && w.phase == "host" && w.reads > 0), "the toy's own reads counted: {:?}", runs[0]);
        let mut grew = Vec::new();
        for (r, n) in runs.iter().zip(sizes) {
            grew.extend(crate::laws::work_grows(&runs[0], r, (n / sizes[0]).ilog2() as u64));
        }
        assert_eq!(!grew.is_empty(), grows, "{flaw:?}: {grew:?}");
        if grows {
            assert!(grew.iter().any(|g| g.starts_with("PerPart.host")), "{grew:?}");
        }
    }
}

/// `out(a, b) :- in(b, a), b % 2 == 0`: Toy's output read back into Toy's input shape.
struct Back;

struct BackPorts;

impl PortSet for BackPorts {
    const INS: usize = 1;
    const GLOBALS: usize = 0;
    const OUTS: usize = 1;

    fn ports() -> Ports {
        Ports { ins: vec![Port { name: "in", shape: out_shape() }], globals: Vec::new(), outs: vec![Port { name: "out", shape: in_shape() }] }
    }
}

impl DeltaOp for Back {
    type Ports = BackPorts;
    type Cache = crate::cache::Orders;

    fn delta(&self, cx: &mut Cx<'_>, ins: &In<'_, BackPorts>, _: Option<&Bistate<View<'_>>>, _: &mut CacheW<crate::cache::Orders>, out: &mut Out<BackPorts>, _: &mut Asks<BackPorts>) -> Result<Work, String> {
        let mut work = Work::default();
        if ins.is_identity() {
            return Ok(work);
        }
        let x = ins.port(PortIn::new(0));
        let w = out.writer(cx.dir, PortOut::new(0))?;
        if let Some(b) = x.before() {
            for id in x.gone() {
                work.rows_read += 1;
                if cell(b, id, 0) % 2 == 0 {
                    w.push(&[cell(b, id, 1), cell(b, id, 0)], -1);
                }
            }
        }
        for id in x.added() {
            work.rows_read += 1;
            if cell(x.after(), id, 0) % 2 == 0 {
                w.push(&[cell(x.after(), id, 1), cell(x.after(), id, 0)], 1);
            }
        }
        Ok(work)
    }

    fn run(&self, cx: &mut Cx<'_>, ins: &At<'_, BackPorts>, out: &mut Out<BackPorts>) -> Result<(), String> {
        let x = ins.port(PortIn::new(0));
        let w = out.writer(cx.dir, PortOut::new(0))?;
        for id in live_ids(x).filter(|&id| cell(x, id, 0) % 2 == 0) {
            w.push(&[cell(x, id, 1), cell(x, id, 0)], 1);
        }
        Ok(())
    }
}

/// `out(0, a, b) :- l(a, b)`, `out(1, a, b) :- r(a, b)`: two inputs of Toy's input shape, tagged into one.
struct Union2;

struct Union2Ports;

impl PortSet for Union2Ports {
    const INS: usize = 2;
    const GLOBALS: usize = 0;
    const OUTS: usize = 1;

    fn ports() -> Ports {
        let out = Shape::run(vec![ColType::I32, ColType::I32, ColType::I64], vec![0, 1, 2], None);
        Ports { ins: vec![Port { name: "l", shape: in_shape() }, Port { name: "r", shape: in_shape() }], globals: Vec::new(), outs: vec![Port { name: "out", shape: out }] }
    }
}

impl DeltaOp for Union2 {
    type Ports = Union2Ports;
    type Cache = crate::cache::Orders;

    fn delta(&self, cx: &mut Cx<'_>, ins: &In<'_, Union2Ports>, _: Option<&Bistate<View<'_>>>, _: &mut CacheW<crate::cache::Orders>, out: &mut Out<Union2Ports>, _: &mut Asks<Union2Ports>) -> Result<Work, String> {
        let mut work = Work::default();
        if ins.is_identity() {
            return Ok(work);
        }
        let w = out.writer(cx.dir, PortOut::new(0))?;
        for tag in 0..2 {
            let x = ins.port(PortIn::new(tag));
            if let Some(b) = x.before() {
                for id in x.gone() {
                    work.rows_read += 1;
                    w.push(&[tag as i64, cell(b, id, 0), cell(b, id, 1)], -1);
                }
            }
            for id in x.added() {
                work.rows_read += 1;
                w.push(&[tag as i64, cell(x.after(), id, 0), cell(x.after(), id, 1)], 1);
            }
        }
        Ok(work)
    }

    fn run(&self, cx: &mut Cx<'_>, ins: &At<'_, Union2Ports>, out: &mut Out<Union2Ports>) -> Result<(), String> {
        let w = out.writer(cx.dir, PortOut::new(0))?;
        for tag in 0..2 {
            let x = ins.port(PortIn::new(tag));
            for id in live_ids(x) {
                w.push(&[tag as i64, cell(x, id, 0), cell(x, id, 1)], 1);
            }
        }
        Ok(())
    }
}

/// An edit chain of Toy's input whose every fourth step (from the third) changes only rows Toy drops (`a % 3 == 0`),
/// so the stages after Toy see the identity, and every fourth (from the fourth) is the identity itself.
fn piped_chain(seed: u64, steps: usize) -> Chain {
    let mut z = seed;
    let mut next = move || {
        z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
        rerail_blocks::layer::mix(z)
    };
    let mut live: BTreeSet<Vec<i64>> = BTreeSet::new();
    let mut chain = Vec::with_capacity(steps);
    for s in 0..steps {
        let mut step: Vec<(Vec<i64>, i32)> = Vec::new();
        let dropped_only = s % 4 == 2;
        if s % 4 != 3 {
            let gone: Vec<Vec<i64>> = live.iter().filter(|r| (!dropped_only || r[0] % 3 == 0) && next().is_multiple_of(3)).cloned().collect();
            for r in gone {
                live.remove(&r);
                step.push((r, -1));
            }
            for _ in 0..(next() % 20 + 4) {
                let mut r = row(next());
                if dropped_only {
                    r[0] = r[0] / 3 * 3;
                }
                if !live.contains(&r) && !step.iter().any(|(x, _)| *x == r) {
                    live.insert(r.clone());
                    step.push((r, 1));
                }
            }
        }
        chain.push(vec![step]);
    }
    chain
}

/// Runs the pipeline of `stages` wired by `wires` over a piped chain of one input.
fn piped(seed: u64, make: &dyn Fn(&mut Directory) -> Vec<Box<dyn Stage + 'static>>, wires: &[Vec<Wire>]) -> Result<Piped, Broken> {
    let mut bench = Bench::new().unwrap();
    let (ins, heads) = inputs_of(&mut bench, &[in_shape()], &piped_chain(seed, 12)).unwrap();
    let mut stages = make(&mut bench.dir);
    pipeline_laws(&mut bench, &ins, &heads, &mut stages, wires)
}

static TOY: Toy = Toy(Flaw::None);
static BUSY: Toy = Toy(Flaw::Busy);
static KILLS: Toy = Toy(Flaw::Kills);

fn staged<O: DeltaOp>(op: &'static O, dir: &mut Directory) -> Box<dyn Stage + 'static> {
    Box::new(Staged::new(op, dir).unwrap())
}

/// L6 beyond L1: a chain of three stages (Toy, Back, Toy) and a diamond (Toy, Back, and a union of the input with
/// Back's output) are each their composed function at every state, and a stage whose inputs an edit left alone cuts
/// off (here, past Toy when an edit changes only rows Toy drops).
#[test]
fn pipelines_of_operators_are_their_composed_functions_and_cut_off() {
    let chain_wires = [vec![Wire::Input(0)], vec![Wire::Stage(0, 0)], vec![Wire::Stage(1, 0)]];
    let diamond_wires = [vec![Wire::Input(0)], vec![Wire::Stage(0, 0)], vec![Wire::Input(0), Wire::Stage(1, 0)]];
    for seed in 0..3 {
        let p = piped(seed, &|d| vec![staged(&TOY, d), staged(&Back, d), staged(&TOY, d)], &chain_wires).unwrap_or_else(|b| panic!("the chain, seed {seed}: {b:?}"));
        // 13 states (∅, then 12 edits). Every stage cuts off at the 3 identity steps; Back and the last Toy at the 3 steps of rows Toy drops too.
        assert_eq!((p.steps, p.cutoffs), (13, 3 * 3 + 3 * 2), "the chain, seed {seed}");
        let p = piped(seed, &|d| vec![staged(&TOY, d), staged(&Back, d), staged(&Union2, d)], &diamond_wires).unwrap_or_else(|b| panic!("the diamond, seed {seed}: {b:?}"));
        // The union reads the input itself: it cuts off at the identity steps alone.
        assert_eq!((p.steps, p.cutoffs), (13, 3 * 3 + 3), "the diamond, seed {seed}");
    }
}

/// A pipeline with a stage that breaks a law fails L6: a stage that reads when its input is the identity does not
/// cut off, and one that drops its kills is not its composed function.
#[test]
fn a_pipeline_with_a_broken_stage_fails_l6() {
    let wires = [vec![Wire::Input(0)], vec![Wire::Stage(0, 0)], vec![Wire::Stage(1, 0)]];
    let b = piped(0, &|d| vec![staged(&TOY, d), staged(&Back, d), staged(&BUSY, d)], &wires).unwrap_err();
    assert!(b.law == "L6" && b.what.contains("did not cut off"), "{b:?}");
    let b = piped(0, &|d| vec![staged(&TOY, d), staged(&Back, d), staged(&KILLS, d)], &wires).unwrap_err();
    assert!(b.law == "L6" && b.what.contains("composed function"), "{b:?}");
}
