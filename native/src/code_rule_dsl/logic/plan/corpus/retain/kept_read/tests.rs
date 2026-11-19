use super::*;
use crate::code_rule_dsl::facts::rows::{Rows, Shared};
use crate::content_store::KeptRel;

const SPACE: [u8; 32] = [5; 32];

/// F-R1 (decision 0401): a kept relation bound by a new reader reads its rows, dead bits and levels where they lie
/// in the space's mapped block, not in copies.
#[test]
fn a_kept_relation_is_read_where_it_lies() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    // Two relations: one of three rows with their levels; one of three rows, the second dead, with their levels.
    let whole = InRel::held_shared(Rows::frozen(2, vec![1, 10, 2, 20, 3, 30], 3), 3, vec![b'i', b'i'], Shared::default(), vec![1u32, 2, 3].into());
    let some_dead = InRel::held_shared(Rows::frozen(2, vec![1, 10, 2, 20, 3, 30], 3), 3, vec![b'i', b'i'], vec![0b010u64].into(), vec![1u32, 2, 3].into());
    let name = |r: &str| KeptRel { name: r.into(), ident: [3; 32] };
    let puts = [("whole", &whole), ("some_dead", &some_dead)].map(|(r, held)| super::super::kept_rows(&name(r), 77, vec![("q".into(), 9)], vec![b'i', b'i'], held).expect("a relation in a block"));
    store.queue_kept(SPACE, puts.into_iter().collect());
    let bound = Bound::of(&store, &SPACE).unwrap().expect("the space keeps blocks");
    let view = bound.view();
    let entry = |r: &str| view.get(&rerail_blocks::kept::FactKey { rel: r, ident: &[3; 32], part: super::super::DOCS_PART }).expect("its entry");
    let kept = entry("whole");
    assert_eq!((kept.digest, kept.reads().collect::<Vec<_>>()), (77, vec![("q", 9)]));
    let rel = bound.rel(&kept, vec![b'i', b'i']).expect("bound");
    // Its rows are the entry's relation in the mapping, not a copy of it.
    let at = rel.in_block().expect("its rows, one relation");
    assert!(std::ptr::eq(at.relation(), kept.rows), "the bound rows are the mapped relation");
    assert!(!at.facts.block.is_writable(), "bound over a read-only mapping");
    assert_eq!(rel.levels().as_ptr(), kept.levels.as_ptr());
    assert_eq!(rel.live_rows().unwrap(), vec![1, 10, 2, 20, 3, 30]);
    // Dead rows: the dead bits and levels are the entry's, where they lie. Its rows with their kills are not one
    // relation of the mapping (the delta of kills would be written into it, which is read only): `None`.
    let kept = entry("some_dead");
    let rel = bound.rel(&kept, vec![b'i', b'i']).expect("bound");
    assert_eq!(rel.dead_bits().as_ptr(), kept.dead.as_ptr());
    assert_eq!(rel.levels().as_ptr(), kept.levels.as_ptr());
    assert!(rel.in_block().is_none(), "no kills written into the mapping");
    assert_eq!(rel.live_rows().unwrap(), vec![1, 10, 3, 30]);
    assert_eq!(rel.live_levels().unwrap(), vec![1, 3]);
    // Bound again with no commit between: the same mapping.
    let again = Bound::of(&store, &SPACE).unwrap().unwrap();
    assert!(Arc::ptr_eq(&bound, &again));
    // Dropped, the relation gives nothing back to the mapping (it is read only), and the mapping outlives the bound.
    drop(view);
    drop(bound);
    drop(again);
    assert_eq!(rel.live_rows().unwrap(), vec![1, 10, 3, 30]);
}

/// Rows started from a kept relation bound over its read-only mapping (`Rows::shared`, as a maintained relation goes
/// on from its old rows) read the bound rows, then the rows pushed after them, at the ids past them. Nothing is written
/// in the mapping and its rows are not moved out (0353): the tail is a relation of its own past the mapped segment. A
/// tail moved holding every row (`Part::moved_all`) once kept the mapped segment too, read a second copy of the old
/// rows at the new rows' ids, and lost the rows pushed.
#[test]
fn rows_started_from_a_bound_relation_read_the_rows_pushed_after_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    let held = InRel::held_shared(Rows::frozen(2, vec![1, 10, 2, 20, 3, 30], 3), 3, vec![b'i', b'i'], Shared::default(), Shared::default());
    let name = KeptRel { name: "r".into(), ident: [3; 32] };
    store.queue_kept(SPACE, vec![super::super::kept_rows(&name, 1, vec![], vec![b'i', b'i'], &held).expect("a relation in a block")]);
    let bound = Bound::of(&store, &SPACE).unwrap().unwrap();
    let view = bound.view();
    let kept = view.get(&rerail_blocks::kept::FactKey { rel: "r", ident: &[3; 32], part: super::super::DOCS_PART }).expect("its entry");
    let mut rows = Rows::living(2, Life::Kept);
    rows.append_part(bound.part(&kept).expect("its rows' part"));
    let rows = rows.sealed();
    let mut next = rows.shared(3);
    // copy: not block data: a test's rows
    next.extend_from_slice(&[4, 40, 5, 50]);
    let all: Vec<Vec<i64>> = (0..5).map(|i| next.row(i, 2).to_vec()).collect();
    assert_eq!(all, [[1, 10], [2, 20], [3, 30], [4, 40], [5, 50]].map(|r| r.to_vec()), "the bound rows, then the rows pushed");
    let parts: Vec<&crate::code_rule_dsl::facts::rows::Part> = next.parts().collect();
    assert_eq!(parts.len(), 2, "the mapped segment, then a tail of its own");
    assert!(std::ptr::eq(parts[0].rel(), kept.rows), "the old rows where they lie, not moved");
}
