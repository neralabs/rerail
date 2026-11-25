//! A relation's trace: its history as layers in one block, read at two states (decision 0686 B2).
//!
//! P§12's trace, version, bistate and open layer, as the user narrowed them (2026-11-25, on the rule of 2026-11-24,
//! "Nothing reads more than two versions"): a trace has two states, the committed one and it with the run's layer.
//! There are no other frontiers, no forks and no named pins. Layers are only how one state is stored: O5's
//! geometric roll-up keeps about log n of them (the roll-up comes after L1).
//!
//! A layer is a `Relation` as it is (P§13: "Keep"). The first is a full relation; each later one continues the one
//! below (`Relation::try_continue`): its ids follow theirs, its own rows are the rows its run added, and its dead bits
//! its kills of rows below. The state is `View::full` of the last layer, read down the chain; the last commit's change
//! is its `View::bistate`, 0678's `Bistate`, the one definition. Its gone rows are the layer's `changed` ids, so the
//! change is read in its size, and its identity from its counts alone.
//!
//! Beside each layer lie its partitions' change (P§12's partition digests), derived facts written once where they are
//! computed (`consolidate`):
//!
//! - `parts(part, digest, added, gone)`: one row a partition the layer changed, its digest moved by the rows added
//!   less the rows gone (`row_digest`, summed, so no order is needed);
//! - `gone(part, id)`: the layer's kills, by partition.
//!
//! With the partition index on the layer's own rows, a change restricted to one partition (`restrict`) reads that
//! partition's rows only. `Trace` is the host's bookkeeping, the block and its layers' offsets (an operation graph's,
//! which host Rust may keep); `consolidate` is the work, run by the `consolidate` operation.

use crate::block::Block;
use crate::bistate::Bistate;
use crate::relation::{RelW, Relation, Spec, View, freeze_in, leading, row_digest};
use crate::scratch::Scratch;
use crate::key_hash;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The run's open layer: rows pushed, rows of the state killed (P§12's `LayerW`). `consolidate` freezes it.
pub type LayerW<'b> = RelW<'b>;

/// A layer's relations in the trace's block, by offset: its rows, and its partitions' change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layer {
    pub rows: usize,
    pub parts: usize,
    pub gone: usize,
}

/// What `consolidate` freezes: a layer, the layer below it (none for the first), its rows' sort column and partition
/// column (`NONE` for none).
#[derive(Clone, Copy, Debug)]
pub struct Consolidate {
    pub layer: Layer,
    pub below: Option<usize>,
    pub sort: Option<u16>,
    pub partition: Option<u16>,
}

/// `parts`' columns: the partition, its digest's move, rows added, rows gone.
pub fn parts_spec() -> Spec {
    Spec { widths: vec![8, 8, 4, 4], sort: None, indexes: vec![vec![0]], min_log: crate::rows::MIN_LOG }
}

/// `gone`'s columns: the partition, the id killed.
pub fn gone_spec() -> Spec {
    Spec { widths: vec![8, 4], sort: None, indexes: vec![vec![0]], min_log: crate::rows::MIN_LOG }
}

/// The relation at `at` in `block`.
fn rel_at(block: &Block, at: usize) -> &Relation {
    // SAFETY: `at` is a relation's descriptor in `block` (a trace's layer, made by `Trace::open`), kept with it.
    unsafe { &*block.ptr::<Relation>(at) }
}

/// The trace's spec for its layers: `widths`, sorted on `sort`, indexed on its partition.
fn layer_spec(widths: Vec<u32>, sort: Option<u16>, partition: Option<u16>, min_log: u32) -> Spec {
    Spec { widths, sort, indexes: partition.map(|p| vec![vec![p]]).unwrap_or_default(), min_log }
}

/// Freezes a trace's open layer where it lies (set semantics against the state below, the order, the partition
/// index, its row index, its dead bits and change), then writes its partitions' change (`parts`, `gone`) and freezes
/// them. Everything it reads and writes lies in `block`; its temporaries are `scratch`'s.
pub fn consolidate(block: &Block, c: &Consolidate, scratch: &Scratch) {
    let rel = rel_at(block, c.layer.rows);
    let below = c.below.map(|b| rel_at(block, b));
    let a = rel.arity as usize;
    let widths = (0..a).map(|col| rel.rows.col(col).width).collect();
    freeze_in(block, rel, &layer_spec(widths, c.sort, c.partition, rel.rows.min_log), below, scratch);

    let part = |id: u32| c.partition.map_or(0, |p| rel.get(id, p as usize));
    let digest = |id: u32| row_digest((0..a).map(|col| rel.get(id, col)));
    let mut parts: HashMap<i64, (u64, u32, u32)> = HashMap::new();
    for id in rel.ids() {
        let e = parts.entry(part(id)).or_default();
        e.0 = e.0.wrapping_add(digest(id));
        e.1 += 1;
    }
    let lock = Mutex::new(());
    let (pr, gr) = (rel_at(block, c.layer.parts), rel_at(block, c.layer.gone));
    let gw = RelW { block, rel: gr, lock: &lock };
    for &id in rel.changed.as_slice() {
        let p = part(id);
        let e = parts.entry(p).or_default();
        e.0 = e.0.wrapping_sub(digest(id));
        e.2 += 1;
        gw.push(&[p, i64::from(id)]);
    }
    let pw = RelW { block, rel: pr, lock: &lock };
    for (p, (d, added, gone)) in parts {
        pw.push(&[p, d as i64, i64::from(added), i64::from(gone)]);
    }
    freeze_in(block, pr, &parts_spec(), None, scratch);
    freeze_in(block, gr, &gone_spec(), None, scratch);
}

/// The ids of `rel`'s own rows whose column `col` is `v`, by its index on `col` (a range when it is the sort
/// column's, else a bucket checked against `v`), or a scan of its own rows when it has none.
fn own_with(rel: &Relation, col: u16, v: i64) -> Vec<u32> {
    match rel.find_index(&[col], rel.rows.len()) {
        Some(x) if rel.index_at(x).ordered == 1 => {
            let r = leading(rel, &[col], &[v]);
            (r.start + rel.base_rows..r.end + rel.base_rows).collect()
        }
        Some(x) => rel.group(x, key_hash(&[v])).iter().copied().filter(|&id| rel.own(id, col as usize) == v).collect(),
        None => rel.ids().filter(|&id| rel.own(id, col as usize) == v).collect(),
    }
}

/// One partition's part of the last commit's change (`Trace::restrict`): its ids added and gone, and how many ids its
/// reads touched.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PartChange {
    pub added: Vec<u32>,
    pub gone: Vec<u32>,
    pub read: usize,
}

/// One relation's trace in one block: its layers, oldest first, and the run's open layer.
pub struct Trace {
    block: Arc<Block>,
    widths: Vec<u32>,
    sort: Option<u16>,
    partition: Option<u16>,
    layers: Vec<Layer>,
    open: Option<Layer>,
    lock: Mutex<()>,
}

impl Trace {
    /// An empty trace in `block` of rows of `widths`, sorted on `sort` and partitioned on `partition`.
    pub fn new(block: Arc<Block>, widths: Vec<u32>, sort: Option<u16>, partition: Option<u16>) -> Trace {
        Trace { block, widths, sort, partition, layers: Vec::new(), open: None, lock: Mutex::new(()) }
    }

    pub fn block(&self) -> &Arc<Block> {
        &self.block
    }

    pub fn partition(&self) -> Option<u16> {
        self.partition
    }

    /// Its committed layers.
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    fn spec(&self) -> Spec {
        layer_spec(self.widths.clone(), self.sort, self.partition, crate::rows::MIN_LOG)
    }

    /// The committed state: every committed layer read as one. `None` before the first commit (the state ∅).
    pub fn state(&self) -> Option<View<'_>> {
        self.layers.last().map(|l| View::full(rel_at(&self.block, l.rows)))
    }

    /// The last commit's change: before, the state it found (∅ for the first, a cold run); after, the state it left.
    pub fn change(&self) -> Option<Bistate<View<'_>>> {
        self.state().map(|v| v.bistate())
    }

    /// Opens the run's layer over the committed state: its rows, and its partitions' change, made empty in the block.
    /// `None` when the block has no room.
    pub fn open(&mut self) -> Option<Layer> {
        assert!(self.open.is_none(), "one open layer at a time");
        let block: &Block = &self.block;
        let spec = self.spec();
        let rows = match self.layers.last() {
            None => Relation::try_create(block, &spec, None)?,
            Some(l) => {
                let top = rel_at(block, l.rows);
                Relation::try_continue(block, &spec, top, top.len_all())?
            }
        };
        let parts = Relation::try_create(block, &parts_spec(), None)?;
        let gone = Relation::try_create(block, &gone_spec(), None)?;
        let at = |r: &Relation| block.offset_of(std::ptr::from_ref(r));
        let layer = Layer { rows: at(rows), parts: at(parts), gone: at(gone) };
        self.open = Some(layer);
        Some(layer)
    }

    /// The open layer's writer.
    pub fn writer(&self) -> LayerW<'_> {
        let l = self.open.expect("an open layer");
        RelW { block: &self.block, rel: rel_at(&self.block, l.rows), lock: &self.lock }
    }

    /// What `consolidate` freezes for the open layer.
    pub fn to_consolidate(&self) -> Consolidate {
        Consolidate { layer: self.open.expect("an open layer"), below: self.layers.last().map(|l| l.rows), sort: self.sort, partition: self.partition }
    }

    /// Takes the open layer, consolidated, as the committed state's last.
    pub fn commit(&mut self) {
        let l = self.open.take().expect("an open layer");
        assert!(rel_at(&self.block, l.rows).is_frozen(), "a layer is committed once consolidated");
        self.layers.push(l);
    }

    /// The partitions the last commit changed: its `parts` rows' partitions.
    pub fn changed_parts(&self) -> Vec<i64> {
        let Some(l) = self.layers.last() else { return Vec::new() };
        let p = rel_at(&self.block, l.parts);
        p.ids().map(|id| p.own(id, 0)).collect()
    }

    /// Partition `p`'s digest in the committed state: the sum of its layers' moves (`parts`).
    pub fn part_digest(&self, p: i64) -> u64 {
        self.layers.iter().fold(0u64, |d, l| {
            let parts = rel_at(&self.block, l.parts);
            own_with(parts, 0, p).into_iter().fold(d, |d, id| d.wrapping_add(parts.own(id, 1) as u64))
        })
    }

    /// The last commit's change in partition `p` alone: the layer's own rows in `p` (its partition index), and its
    /// kills in `p` (`gone`'s index). No other partition's row is read.
    pub fn restrict(&self, p: i64) -> PartChange {
        let Some(l) = self.layers.last() else { return PartChange::default() };
        let (rows, gone) = (rel_at(&self.block, l.rows), rel_at(&self.block, l.gone));
        let pc = self.partition.expect("a partitioned trace");
        let added = own_with(rows, pc, p);
        let gone_ids: Vec<u32> = own_with(gone, 0, p).into_iter().map(|g| gone.own(g, 1) as u32).collect();
        let read = added.len() + gone_ids.len();
        PartChange { added, gone: gone_ids, read }
    }

    /// Partition `p`'s live ids in the committed state: each layer's own rows in `p`, by its partition index, live
    /// in the state.
    pub fn part_ids(&self, p: i64) -> Vec<u32> {
        let Some(top) = self.layers.last().map(|l| rel_at(&self.block, l.rows)) else { return Vec::new() };
        let pc = self.partition.expect("a partitioned trace");
        self.layers.iter().flat_map(|l| own_with(rel_at(&self.block, l.rows), pc, p)).filter(|&id| top.live_down(id)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bistate::{self, Read, RowId};
    use crate::block::kind;
    use std::collections::{BTreeMap, BTreeSet};

    const C0: crate::bistate::ColumnIndex = crate::bistate::ColumnIndex::new(0);
    const C1: crate::bistate::ColumnIndex = crate::bistate::ColumnIndex::new(1);

    /// xorshift: the tests' rounds, the same every run.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self, n: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % n
        }
    }

    /// The live rows of a state, as a set of values.
    fn rows_of<S: Read>(s: &S) -> BTreeSet<(i64, i64)> {
        s.live_ids().map(|id| (s.get(id, C0), s.get(id, C1))).collect()
    }

    /// A round: rows pushed, rows of the state killed, then consolidated (its own `Scratch`) and committed.
    fn round(t: &mut Trace, push: &[(i64, i64)], kill: &[u32]) {
        t.open().expect("room");
        let w = t.writer();
        for &(p, v) in push {
            w.push(&[p, v]);
        }
        for &id in kill {
            w.kill(id);
        }
        let c = t.to_consolidate();
        consolidate(t.block(), &c, &Scratch::anon(1 << 30));
        t.commit();
    }

    /// B2's check: a random chain of layers reads as a flat set at both states, its change is 0678's bistate (L1 and
    /// the least change), a partition's change reads that partition's rows alone, and its digest is its rows'.
    fn random_chain(sort: Option<u16>, seed: u64) {
        let block = Arc::new(Block::anon(kind::FACTS, 64 << 20).with_free_list());
        let mut t = Trace::new(block, vec![8, 8], sort, Some(0));
        let mut rng = Rng(seed);
        let mut flat: BTreeSet<(i64, i64)> = BTreeSet::new();
        for r in 0..24 {
            let before = flat.clone();
            let live: Vec<u32> = t.state().map(|s| s.live_ids().map(RowId::get).collect()).unwrap_or_default();
            let mut kill = Vec::new();
            for &id in &live {
                if rng.next(5) == 0 {
                    kill.push(id);
                }
            }
            // Rows new, rows the state holds (dropped), and rows just killed (brought back).
            let mut push: Vec<(i64, i64)> = (0..rng.next(12)).map(|_| (rng.next(4) as i64, rng.next(24) as i64)).collect();
            if let Some(s) = t.state() {
                for &id in kill.iter().take(2) {
                    push.push((s.get(id, 0), s.get(id, 1)));
                }
                for &id in &kill {
                    flat.remove(&(s.get(id, 0), s.get(id, 1)));
                }
            }
            flat.extend(push.iter().copied());
            round(&mut t, &push, &kill);

            assert_eq!(t.layers().len(), r + 1);
            let after = t.state().expect("committed");
            assert_eq!(rows_of(&after), flat, "round {r}: the state reads as the flat set");
            let change = t.change().expect("committed");
            assert_eq!(change.before().map(rows_of).unwrap_or_default(), before, "round {r}: before is the state it found");
            bistate::check(&change);
            assert!(bistate::is_least(&change), "round {r}: a row killed and pushed again is brought back");

            for p in 0..4 {
                let part = |id: &RowId| after.get(id.get(), 0) == p;
                let want_added: Vec<u32> = change.added().filter(part).map(RowId::get).collect();
                let want_gone: Vec<u32> = change.gone().filter(|id| change.before().expect("before").get(id.get(), 0) == p).map(RowId::get).collect();
                let got = t.restrict(p);
                assert_eq!((got.added, got.gone), (want_added.clone(), want_gone.clone()), "round {r}: partition {p}'s change");
                assert_eq!(got.read, want_added.len() + want_gone.len(), "round {r}: partition {p}'s change reads its rows alone");
                let digest = flat.iter().filter(|x| x.0 == p).fold(0u64, |d, &(a, b)| d.wrapping_add(row_digest([a, b].into_iter())));
                assert_eq!(t.part_digest(p), digest, "round {r}: partition {p}'s digest is its rows'");
                let ids: BTreeSet<(i64, i64)> = t.part_ids(p).into_iter().map(|id| (after.get(id, 0), after.get(id, 1))).collect();
                assert_eq!(ids, flat.iter().copied().filter(|x| x.0 == p).collect::<BTreeSet<_>>(), "round {r}: partition {p}'s rows");
            }
            let mut changed: Vec<i64> = t.changed_parts();
            changed.sort_unstable();
            let want: BTreeSet<i64> = change.added().chain(change.gone()).map(|id| change.after().get(id.get(), 0)).collect();
            assert_eq!(changed, want.into_iter().collect::<Vec<_>>(), "round {r}: the partitions it changed");
        }
    }

    #[test]
    fn a_random_chain_of_layers_reads_as_a_flat_set_at_both_states() {
        for seed in [1, 7, 42, 0x5eed] {
            random_chain(None, seed);
        }
    }

    #[test]
    fn a_sorted_chain_reads_as_a_flat_set_too() {
        for seed in [3, 11, 0xbeef] {
            random_chain(Some(0), seed);
        }
    }

    /// A round that changes nothing (no row, only rows the state holds, a row killed and pushed again) is the
    /// identity, said by its counts: no id flipped and no row of its own.
    #[test]
    fn a_round_that_changes_nothing_is_the_identity_by_its_counts() {
        let block = Arc::new(Block::anon(kind::FACTS, 16 << 20).with_free_list());
        let mut t = Trace::new(block, vec![8, 8], None, Some(0));
        round(&mut t, &[(0, 1), (1, 2), (2, 3)], &[]);
        assert!(!t.change().expect("committed").is_identity());
        round(&mut t, &[], &[]);
        assert!(t.change().expect("committed").is_identity());
        round(&mut t, &[(1, 2), (0, 1)], &[]);
        assert!(t.change().expect("committed").is_identity());
        round(&mut t, &[(2, 3)], &[2]);
        let c = t.change().expect("committed");
        assert!(c.is_identity());
        assert_eq!(c.after().end(), c.before().expect("before").end(), "no row of its own");
        assert!(t.changed_parts().is_empty());
        assert_eq!(t.layers().len(), 4);
        let mut by: BTreeMap<i64, u64> = BTreeMap::new();
        for (p, v) in [(0, 1), (1, 2), (2, 3)] {
            *by.entry(p).or_default() = by.get(&p).copied().unwrap_or(0).wrapping_add(row_digest([p, v].into_iter()));
        }
        for (p, d) in by {
            assert_eq!(t.part_digest(p), d);
        }
    }
}
