//! The operator cache's checks (*0686: Operations* §13, "the operator's cache"; P§11.3): a header left open, or naming
//! another state, reads as ∅; an order is the layer's ids sorted by its columns, and a second run builds no index the
//! first built; the cache grows within its reservation, its bytes where they lie; two operations writing one cache
//! block cannot run together; a cache is not a trace's layers. (L5, a run whose cache is dropped giving the same
//! facts, is the operator harness's: B3.)

use super::*;
use crate::trace::{RollUp, Trace, TraceOps};
use rerail_blocks::layer::{ColType, Layer, Retention, RunsW, Shape};
use rerail_blocks::region::pin_block;

fn setup() -> (Registry, CacheOps, TraceOps, Directory) {
    let mut reg = Registry::default();
    let c = CacheOps::declare(&mut reg).unwrap();
    let t = TraceOps::declare(&mut reg).unwrap();
    (reg, c, t, Directory::default())
}

fn shape() -> Shape {
    Shape::run(vec![ColType::I32, ColType::I64, ColType::I32], vec![0, 1, 2], Some(0))
}

/// A committed layer of `n` rows: `(k % 7, (k * 7919) % 1000, k)`.
fn layer(reg: &Registry, ops: &TraceOps, dir: &mut Directory, n: i64) -> BlockId {
    let mut t = Trace::new(&shape()).unwrap().with_roll_up(RollUp::Never);
    let id = t.open(dir, n as usize);
    {
        let b = dir.block(id).unwrap();
        let pin = pin_block(&b, "writer", true).unwrap();
        let w = RunsW::new(&pin);
        for k in 0..n {
            w.push(&[k % 7, (k * 7919) % 1000, k], 1).unwrap();
        }
    }
    t.commit(reg, ops, dir, id, Tiling::WHOLE).unwrap()
}

fn bytes(dir: &Directory, c: OpCache) -> (usize, usize, u64) {
    let l = dir.read("t", &[c.0]).unwrap();
    let b = l.block(0);
    (twin::count(b), twin::used(b), twin::state(b))
}

#[test]
fn a_cache_open_or_of_another_state_reads_as_empty() {
    let (reg, ops, tops, mut dir) = setup();
    let l = layer(&reg, &tops, &mut dir, 100);
    let c = OpCache::new(&mut dir);
    assert!(c.open(&reg, &ops, &dir, 0).unwrap(), "a new cache holds ∅, state 0");
    c.order(&reg, &ops, &dir, l, &[2]).unwrap();
    c.commit(&reg, &ops, &dir, 0xd1).unwrap();
    assert_eq!(bytes(&dir, c).0, 1);
    // The state it holds: kept.
    assert!(c.open(&reg, &ops, &dir, 0xd1).unwrap());
    assert_eq!(bytes(&dir, c).0, 1);
    // That run died before its commit: the header is left open, and the next run's cache is ∅.
    assert!(!c.open(&reg, &ops, &dir, 0xd1).unwrap(), "left open: ∅");
    assert_eq!(bytes(&dir, c), (0, 0, 0));
    c.order(&reg, &ops, &dir, l, &[1]).unwrap();
    c.commit(&reg, &ops, &dir, 0xd2).unwrap();
    // Another state's: ∅.
    assert!(!c.open(&reg, &ops, &dir, 0xd3).unwrap(), "another state's: ∅");
    assert_eq!(bytes(&dir, c), (0, 0, 0));
}

#[test]
fn an_order_is_its_layers_ids_by_its_columns_and_is_built_once() {
    let (reg, ops, tops, mut dir) = setup();
    let l = layer(&reg, &tops, &mut dir, 500);
    let c = OpCache::new(&mut dir);
    c.open(&reg, &ops, &dir, 0).unwrap();
    c.order(&reg, &ops, &dir, l, &[1, 2]).unwrap();
    let once = bytes(&dir, c);
    {
        let r = dir.read("t", &[l, c.0]).unwrap();
        let layer = Layer::new(r.block(0)).unwrap();
        let mut want: Vec<u32> = layer.own.ids().collect();
        want.sort_by_key(|&id| (layer.get(id, 1), layer.get(id, 2), id));
        let o = twin::order(r.block(1), l.0, &[1, 2]).expect("the order, held");
        assert_eq!(o.ids, &want[..]);
        let fences: Vec<i64> = want.iter().step_by(twin::FENCE).map(|&id| layer.get(id, 1)).collect();
        assert_eq!(o.fences, &fences[..]);
        assert!(twin::order(r.block(1), l.0, &[2, 1]).is_none(), "another order: not held");
    }
    // A second run over the same layer builds no index the first built.
    c.order(&reg, &ops, &dir, l, &[1, 2]).unwrap();
    assert_eq!(bytes(&dir, c), once);
}

#[test]
fn a_cache_grows_within_its_reservation() {
    let (reg, ops, tops, mut dir) = setup();
    // 30,000 ids are 120,000 bytes: past the first 64 KiB of capacity.
    let l = layer(&reg, &tops, &mut dir, 30_000);
    let c = OpCache::new(&mut dir);
    c.open(&reg, &ops, &dir, 0).unwrap();
    assert_eq!(dir.capacity(c.0), CACHE_CAPACITY);
    c.order(&reg, &ops, &dir, l, &[0, 1]).unwrap();
    assert!(dir.capacity(c.0) > 120_000, "grown to {}", dir.capacity(c.0));
    let r = dir.read("t", &[c.0]).unwrap();
    assert_eq!(twin::order(r.block(0), l.0, &[0, 1]).expect("held").ids.len(), 30_000);
}

#[test]
fn two_writers_of_one_cache_cannot_run_together() {
    let (reg, ops, tops, mut dir) = setup();
    let l = layer(&reg, &tops, &mut dir, 10);
    let c = OpCache::new(&mut dir);
    c.open(&reg, &ops, &dir, 0).unwrap();
    let b = dir.block(c.0).unwrap();
    let held = pin_block(&b, "another writer", true).unwrap();
    let e = c.order(&reg, &ops, &dir, l, &[1]).unwrap_err();
    assert!(e.contains("order could not start") && e.contains("another writer"), "{e}");
    drop(held);
    c.order(&reg, &ops, &dir, l, &[1]).unwrap();
}

#[test]
fn a_cache_is_not_a_traces_layers() {
    let mut s = shape();
    s.retention = Retention::Cache;
    let e = Trace::new(&s).err().unwrap();
    assert!(e.contains("an operator's cache"), "{e}");
}
