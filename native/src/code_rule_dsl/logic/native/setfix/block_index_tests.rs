//! Decision 0468 step 5 (R2, R5): a relation's index is in its block, the one the device finds and builds, and the
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
    let n = 4096;
    let rel = InRel::from_flat(rows(n), n, vec![b'i'; 2]);
    let _ = probed(&rel, 1);
    let ix = rel.index(&[0]);
    assert!(ix.table.is_empty() && ix.ids.is_empty() && ix.order.get().is_none(), "no table, ids or order");
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

/// Decision 0468 step 5b (the user, 2026-11-19: one sorted order): a frozen relation's kept orders are one sorted
/// order each in its block's index directory, found there by key, as the device can find them; the relation's own
/// handles to them hold nothing of their own.
#[test]
fn a_frozen_relations_orders_are_in_its_blocks_directory() {
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
    let n = 3000;
    let old = frozen(n);
    let at: Vec<*const u32> = old.chains.iter().map(|o| o.first_ids()).collect();
    let based = super::Rel::based("edge", &[b'i', b'i'], &old, None).expect("based on the old relation");
    assert_eq!(based.idx.iter().map(|o| o.first_ids()).collect::<Vec<_>>(), at, "the same ids, where they lie");
}
