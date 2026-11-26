//! The harness on a toy operator, and on four toys each built to break one law (B3).

use super::*;
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
            OpCache(c).order(cx.reg, cx.caches, cx.dir, layer, &[1])?;
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
    let mut reg = Registry::default();
    let traces = TraceOps::declare(&mut reg).unwrap();
    let caches = CacheOps::declare(&mut reg).unwrap();
    let mut dir = Directory::default();
    let mut t = Trace::new(&in_shape()).unwrap().with_roll_up(RollUp::Never);
    let commit = |dir: &mut Directory, t: &mut Trace, pairs: &[(Vec<i64>, i32)]| {
        let id = t.open(dir, pairs.len());
        let b = dir.block(id).unwrap();
        let pin = pin_block(&b, "test", true).unwrap();
        pairs.iter().for_each(|(r, d)| assert!(RunsW::new(&pin).push(r, *d).is_some()));
        drop(pin);
        t.commit(&reg, &traces, dir, id, Tiling::WHOLE).unwrap()
    };
    let h1 = commit(&mut dir, &mut t, &[(vec![1, 10], 1), (vec![2, 20], 1), (vec![3, 30], 1)]);
    let h2 = commit(&mut dir, &mut t, &[(vec![1, 10], -1), (vec![4, 40], 1), (vec![6, 60], 1)]);
    let mut cx = Cx { reg: &reg, dir: &mut dir, blobs: &crate::files::BlobRows::new(&traces), traces: &traces, caches: &caches };
    let r = reference(&Toy(Flaw::None), &mut cx, &[(&t, Some(h1), Some(h2))], &[]).unwrap();
    assert_eq!(r[0].added, [vec![40, 4]].into_iter().collect());
    assert_eq!(r[0].gone, [vec![10, 1]].into_iter().collect());
}

/// An instance's identity run writes nothing: no layer committed, the cache's header as it was, no work.
#[test]
fn an_identity_run_is_its_checks_alone() {
    let mut reg = Registry::default();
    let traces = TraceOps::declare(&mut reg).unwrap();
    let caches = CacheOps::declare(&mut reg).unwrap();
    let mut dir = Directory::default();
    let mut t = Trace::new(&in_shape()).unwrap().with_roll_up(RollUp::Never);
    let id = t.open(&mut dir, 2);
    {
        let b = dir.block(id).unwrap();
        let pin = pin_block(&b, "test", true).unwrap();
        assert!(RunsW::new(&pin).push(&[1, 10], 1).is_some());
    }
    let h = t.commit(&reg, &traces, &mut dir, id, Tiling::WHOLE).unwrap();
    let mut cx = Cx { reg: &reg, dir: &mut dir, blobs: &crate::files::BlobRows::new(&traces), traces: &traces, caches: &caches };
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
    let mut reg = Registry::default();
    let traces = TraceOps::declare(&mut reg).unwrap();
    let caches = CacheOps::declare(&mut reg).unwrap();
    let mut dir = Directory::default();
    let c = OpCache::new(&mut dir);
    let cx = Cx { reg: &reg, dir: &mut dir, blobs: &crate::files::BlobRows::new(&traces), traces: &traces, caches: &caches };
    c.open(cx.reg, cx.caches, cx.dir, 0).unwrap();
    c.commit(cx.reg, cx.caches, cx.dir, 7).unwrap();
    assert_eq!(CacheW::<()>::new(c, 7).read(&cx).unwrap(), Some(c.0));
    assert_eq!(CacheW::<()>::new(c, 8).read(&cx).unwrap(), None, "another state");
    c.open(cx.reg, cx.caches, cx.dir, 7).unwrap();
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
