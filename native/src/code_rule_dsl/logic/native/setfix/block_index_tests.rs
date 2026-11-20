//! Decision 0478 step 5 (R2, R5): a relation's index is in its block, the one the device finds and builds, and the
//! CPU probes it there. Its dead rows are its block's.

use super::InRel;

/// `n` rows of two columns: `(id % 7, id)`.
fn rows(n: usize) -> Vec<i64> {
    (0..n as i64).flat_map(|id| [id % 7, id]).collect()
}

/// The rows a probe of column 0 by `v` finds, by their column 1.
fn probed(rel: &InRel, v: i64) -> Vec<i64> {
    let mut got = Vec::new();
    rel.probe_each(&[0], &[v], &mut |row| got.push(row(1)));
    got.sort_unstable();
    got
}

/// Indexes built in the relation's block, and its rows' relation there.
fn built(rel: &InRel) -> usize {
    let ib = rel.in_block().expect("its rows lie in a block");
    ib.relation().nindex.load(std::sync::atomic::Ordering::Acquire) as usize
}

/// R2, the CPU's side: the CPU's first probe builds the index into the relation's block, where the device's
/// `find_index` finds it, and keeps no table of its own.
#[test]
fn a_cpu_probe_builds_its_index_in_the_relations_block() {
    // A run of its own (decision 0442): its open blocks and counts are the run's.
    let _run = crate::code_rule_dsl::logic::profile::test_run();
    let n = 1000;
    let rel = InRel::from_flat(rows(n), n, vec![b'i'; 2]);
    assert_eq!(built(&rel), 0);
    assert_eq!(probed(&rel, 3), (0..n as i64).filter(|id| id % 7 == 3).collect::<Vec<_>>());
    let ib = rel.in_block().unwrap();
    assert!(ib.relation().find_index(&[0], n as u32).is_some(), "the index is in the block, for the device");
    assert!(rel.index(&[0]).block.is_some(), "the CPU probes the block's index");
    assert_eq!(built(&rel), 1);
}

/// R2, the device's side: an index already in the block (as the device builds it: `find_index` over the same rows)
/// is the one the CPU probes; it builds none.
#[test]
fn a_cpu_probe_reads_the_index_the_device_built() {
    // A run of its own (decision 0442): its open blocks and counts are the run's.
    let _run = crate::code_rule_dsl::logic::profile::test_run();
    let n = 1000;
    let rel = InRel::from_flat(rows(n), n, vec![b'i'; 2]);
    let ib = rel.in_block().unwrap();
    let i = ib.own_index(&[0]).expect("room for an index");
    assert_eq!(built(&rel), 1);
    assert_eq!(rel.index(&[0]).block.as_ref().map(|b| b.1), Some(i), "the block's index");
    assert_eq!(probed(&rel, 5).len(), (0..n).filter(|id| id % 7 == 5).count());
    assert_eq!(built(&rel), 1, "no second index");
}

/// Dead rows: the index is over the relation's rows in its block, dead ones too, as the device's (`DevRel::dead`);
/// a probe skips them by the dead bits.
#[test]
fn a_probe_skips_the_dead_rows_of_the_blocks_index() {
    // A run of its own (decision 0442): its open blocks and counts are the run's.
    let _run = crate::code_rule_dsl::logic::profile::test_run();
    let n: usize = 700;
    let mut dead = vec![0u64; n.div_ceil(64)];
    for id in (0..n).filter(|id| id % 3 == 0) {
        dead[id / 64] |= 1 << (id % 64);
    }
    let built_rows = crate::code_rule_dsl::facts::rows::Rows::frozen(2, rows(n), n);
    let rel = InRel::held(built_rows, n, vec![b'i'; 2], dead);
    assert_eq!(probed(&rel, 2), (0..n as i64).filter(|id| id % 7 == 2 && id % 3 != 0).collect::<Vec<_>>());
    let ib = rel.in_block().expect("in its block, its dead rows a delta");
    assert_ne!(ib.dead, 0);
    assert!(ib.relation().find_index(&[0], n as u32).is_some(), "the index of the rows, dead ones too");
    assert!(rel.index(&[0]).block.is_some());
}

/// R5: a relation's index keeps nothing on the heap: no table, no order of its own.
#[test]
fn a_relations_index_holds_no_table_of_its_own() {
    // A run of its own (decision 0442): its open blocks and counts are the run's.
    let _run = crate::code_rule_dsl::logic::profile::test_run();
    let n = 4096;
    let rel = InRel::from_flat(rows(n), n, vec![b'i'; 2]);
    let _ = probed(&rel, 1);
    let ix = rel.index(&[0]);
    assert!(ix.block.is_some() && ix.order.get().is_none(), "its block's index; no order");
}

/// A relation `Rel` derived of `n` rows `(id % 7, id)`, asked for an order on column 1 too, frozen into an input.
fn frozen(n: usize) -> std::sync::Arc<InRel> {
    let mut rel = super::Rel::new("edge", &[b'i', b'i']);
    rel.want(&[1]);
    for r in rows(n).chunks_exact(2) {
        rel.push(r);
    }
    std::sync::Arc::new(InRel::moved(rel))
}

/// The block relation that holds a frozen relation's last rows: its orders' directory.
fn directory(rel: &InRel) -> crate::code_rule_dsl::facts::rows::InBlock {
    rel.rows.last_in_block().expect("its rows in a block")
}

/// Decision 0478 step 5b (the user, 2026-11-19: one sorted order): a frozen relation's kept orders are one sorted
/// order each in its block's index directory, found there by key, as the device can find them; the relation's own
/// handles to them hold nothing of their own.
#[test]
fn a_frozen_relations_orders_are_in_its_blocks_directory() {
    // A run of its own (decision 0442): its open blocks and counts are the run's.
    let _run = crate::code_rule_dsl::logic::profile::test_run();
    let n = 5000;
    let rel = frozen(n);
    let ib = directory(&rel);
    let r = ib.relation();
    // Column 1 asked for, the dedup order became `[1, 0]` (`Rel::want`): one order, found by its key.
    let i = r.find_order(&[1, 0], n as u32).expect("the order on [1, 0] in the directory");
    assert_eq!(rerail_blocks::relation::Relation::order_rows(r.index(i)), n, "every row, in one run");
    assert_eq!(r.orders().count(), rel.chains.len(), "every order kept is in the directory");
    assert!(!rel.chains.is_empty() && rel.chains.iter().all(|o| o.runs() == 1 && o.borrowed()), "handles to the block's orders, one run each");
    // A probe through the order on column 1 finds its row.
    let mut got = Vec::new();
    rel.probe_each(&[1], &[1234], &mut |row| got.push((row(0), row(1))));
    assert_eq!(got, vec![(1234 % 7, 1234)]);
}

/// The orders a maintained relation restarts from (`Rel::based`) are the block's, read where they lie: nothing of
/// them is copied, and the old relation's block holds them while the new one reads them.
#[test]
fn a_based_relation_reads_its_old_orders_in_the_block() {
    // A run of its own (decision 0442): its open blocks and counts are the run's.
    let _run = crate::code_rule_dsl::logic::profile::test_run();
    let n = 3000;
    let old = frozen(n);
    let at: Vec<*const u32> = old.chains.iter().map(|o| o.first_ids()).collect();
    let based = super::Rel::based("edge", &[b'i', b'i'], &old, None).expect("based on the old relation");
    assert_eq!(based.idx.iter().map(|o| o.first_ids()).collect::<Vec<_>>(), at, "the same ids, where they lie");
}

/// Decision 0478 step 5c: a frozen relation's row ranges known to be in an order are in its block's descriptor,
/// read there (`Relation::known`); it keeps no list of them.
#[test]
fn a_frozen_relations_known_sorted_ranges_are_in_its_block() {
    // A run of its own (decision 0442): its open blocks and counts are the run's.
    let _run = crate::code_rule_dsl::logic::profile::test_run();
    let n = 4000;
    let mut rel = super::Rel::new("edge", &[b'i', b'i']);
    let cols = rel.idx[0].cols.clone();
    for id in 0..n as i64 {
        rel.push(&[0, id]);
    }
    // Its rows (one value of column 0, then ascending) were written in the order on its first order's columns (a wave's, `Fresh::settle`).
    let data = rel.data.shared(n);
    rel.sorted.note(&data, 2, &cols, 0, (n / 2) as u32);
    let rel = InRel::moved(rel);
    assert!(rel.sorted.in_block(), "no ranges of its own");
    let ib = directory(&rel);
    let known = ib.relation().known();
    assert_eq!(known.iter().map(|kn| (kn.lo, kn.hi)).collect::<Vec<_>>(), vec![(0, (n / 2) as u32)]);
    assert_eq!(rel.sorted.within(&cols, 2, 100, 200), vec![(100, 200)], "read from the block");
}

/// Decision 0478 step 5d: a frozen relation's dead rows are its block relation's (`own_dead`), read there by the CPU,
/// and its kills delta, the device's dead rows, reads the same words: none copied.
#[test]
fn a_frozen_relations_dead_rows_are_its_blocks_and_the_devices() {
    // A run of its own (decision 0442): its open blocks and counts are the run's.
    let _run = crate::code_rule_dsl::logic::profile::test_run();
    let n = 1000;
    let mut rel = super::Rel::new("edge", &[b'i', b'i']);
    for r in rows(n).chunks_exact(2) {
        rel.push(r);
    }
    for id in (0..n as u32).step_by(5) {
        if super::kill(&mut rel.dead, id) {
            rel.ndead += 1;
        }
    }
    let rel = InRel::moved(rel);
    let ib = directory(&rel);
    let own = ib.relation().own_dead();
    assert!(!own.is_empty() && std::ptr::eq(own, &rel.dead[..]), "the relation's dead rows are its block relation's");
    let dev = rel.in_block().expect("in its block, its dead rows a delta");
    assert_ne!(dev.dead, 0);
    // SAFETY: `dev.dead` is a delta's descriptor in `dev`'s block, which `dev` keeps.
    let delta = unsafe { &*dev.facts.block.ptr::<rerail_blocks::relation::Relation>(dev.dead) };
    assert!(std::ptr::eq(delta.dead.as_slice(), own), "the delta reads the same words");
    assert_eq!(probed(&rel, 0), (0..n as i64).filter(|id| id % 7 == 0 && id % 5 != 0).collect::<Vec<_>>());
}

/// Decision 0478 step 5e: a derived input's order made at its first probe (one with no block index: its rows in
/// several parts) is in its last part's block directory, as a frozen relation's are; the probe reads it there.
#[test]
fn an_order_made_at_a_probe_is_in_the_blocks_directory() {
    // A run of its own (decision 0442): its open blocks and counts are the run's.
    let _run = crate::code_rule_dsl::logic::profile::test_run();
    let n = 3000;
    let k = 2;
    // Half the rows frozen, the rest another relation's appended after them (`append_part`): two parts. A tail
    // written after a frozen segment continues it and reads as one relation (decision 0384 steps 7 and 8).
    let mut rows_ = crate::code_rule_dsl::facts::rows::Rows::frozen(k, rows(n / 2), n / 2);
    let mut rest = crate::code_rule_dsl::facts::rows::Rows::new(k);
    rest.extend_from_slice(&rows(n)[n..]);
    rows_.append_part(rest.parts().last().expect("its rows' part").clone());
    let rows_ = rows_.freeze(n);
    let rel = InRel::held(rows_, n, vec![b'i'; 2], Vec::new());
    assert!(rel.rows.in_one_block().is_none(), "two parts");
    let before = rel.rows.last_in_block().unwrap().relation().orders().count();
    let mut got = Vec::new();
    rel.probe_each(&[1], &[2000], &mut |row| got.push((row(0), row(1))));
    assert_eq!(got, vec![(2000 % 7, 2000)]);
    assert_eq!(rel.rows.last_in_block().unwrap().relation().orders().count(), before + 1, "the order is in the directory");
}

/// Decision 0478 step 6: a derived input's value counts and measured statistics are written once in a block, and
/// its rows' values are not copied to count them.
#[test]
fn an_inputs_value_counts_and_statistics_are_in_a_block() {
    // A run of its own (decision 0442): its open blocks and counts are the run's.
    let _run = crate::code_rule_dsl::logic::profile::test_run();
    use crate::code_rule_dsl::facts::vtab::Source;
    let n = 700;
    let rel = std::sync::Arc::new(InRel::from_flat(rows(n), n, vec![b'i'; 2]));
    let src = super::Derived::new(std::sync::Arc::clone(&rel), None);
    let want = |v: i64| (0..n as i64).filter(|id| id % 7 == v).count() as i64;
    assert_eq!(src.value_counts(&[(0, 3), (0, 6), (0, 9)]), Some(vec![want(3), want(6), 0]));
    {
        let counted = rel.counted.lock().unwrap();
        assert_eq!(counted.len(), 1, "one column counted");
        assert!(counted[0].1.facts().is_some() && counted[0].1.len() == 7, "its seven values' counts, in a block");
    }
    assert!(src.statistics().is_some());
    assert!(rel.measured.get().is_some_and(|m| m.facts().is_some() && m.len() == 2), "a column's statistics each, in a block");
}
