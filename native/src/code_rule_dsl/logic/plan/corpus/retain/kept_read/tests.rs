use super::*;
use crate::code_rule_dsl::facts::rows::{BlockVec, Rows};
use crate::content_store::KeptRel;
use crate::content_store::kept_facts::{KeptPut, KeptWhat};

const SPACE: [u8; 32] = [5; 32];

fn name(r: &str) -> KeptRel {
    KeptRel { name: r.into(), ident: [3; 32] }
}

/// F-R1, K-R8a (decisions 0401 F3, 0456 step 6b): a kept relation bound by a new reader is a view of its entry where
/// it lies in the space's mapped chain, its rows in canonical order, not a copy; one whose entry keeps its levels
/// (decision 0090) is read in with them, in the entry's rows' order. A relation's dead rows are not kept (0456 K3:
/// its live rows are packed). Bound again with no commit between, the space is the same mapping; the relation
/// outlives the binding it came from.
#[test]
fn a_kept_relation_is_read_where_it_lies() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    // Two relations of three rows in a block: one with no levels; one with its levels, its second row dead.
    let whole = InRel::held_shared(Rows::frozen(2, vec![3, 30, 1, 10, 2, 20], 3), 3, vec![b'i', b'i'], BlockVec::new(), BlockVec::new());
    let some_dead = InRel::held_shared(Rows::frozen(2, vec![3, 30, 2, 20, 1, 10], 3), 3, vec![b'i', b'i'], BlockVec::from_iter_in(None, [0b010u64]), BlockVec::from_iter_in(None, [5u32, 6, 7]));
    let puts = [("whole", &whole), ("some_dead", &some_dead)].map(|(r, held)| super::super::kept_rows(&name(r), 77, vec![("q".into(), 9)], vec![b'i', b'i'], held).expect("a relation in a block"));
    store.queue_kept(SPACE, puts.into_iter().collect());
    let bound = Bound::of(&store, &SPACE).unwrap().expect("the space keeps facts");
    let atoms = store.atoms_handle();
    let entry = |r: &str| bound.entry(r, &[3; 32], super::super::DOCS_PART).expect("its entry");
    let whole = entry("whole");
    assert_eq!((whole.digest(), whole.reads(), whole.len(), whole.width()), (77, vec![("q".to_owned(), 9)], 3, 2));
    let in_chain = bound.view().entry(&rerail_blocks::kept::FactKey { rel: "whole", ident: &[3; 32], part: super::super::DOCS_PART }).expect("kept").rows;
    assert!(std::ptr::eq(whole.packed(), in_chain), "the entry's rows where they lie in the chain");
    let rel = whole.rel(&atoms).expect("bound");
    assert!(rel.view_kind().is_some() && rel.in_block().is_none(), "a view of the entry, no rows of its own");
    let src = rel.source(vec!["a".into(), "b".into()]);
    let rows: Vec<[i64; 2]> = (0..src.len()).map(|r| [src.value(r, 0), src.value(r, 1)]).collect();
    assert_eq!(rows, [[1, 10], [2, 20], [3, 30]], "its rows in canonical order");
    // With levels: its live rows read in, each with its level.
    let rel = entry("some_dead").rel(&atoms).expect("bound");
    assert!(rel.view_kind().is_none());
    assert_eq!((rel.live_rows().unwrap(), rel.live_levels().unwrap()), (vec![1, 10, 3, 30], vec![7, 5]), "the live rows, their levels in the entry's order");
    let again = Bound::of(&store, &SPACE).unwrap().unwrap();
    assert!(Arc::ptr_eq(&bound, &again), "the same mapping");
    let view = whole.rel(&atoms).unwrap();
    drop((bound, again, whole));
    assert_eq!(view.len(), 3, "the view holds the mapping");
}

/// A kept input's granules (decision 0401 F3; put as given, 0456 K6) bound as one relation: a view of each granule's
/// entry where it lies, one after another, every row in the order put; a probe of any column finds its rows across
/// the granules. Granules of another width are not gathered.
#[test]
fn a_kept_inputs_granules_are_one_view_where_they_lie() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();
    let granule = |part: i64, rows: Vec<i64>| KeptPut { rel: "input".into(), ident: [3; 32], part, digest: 0, reads: Vec::new(), types: vec![b'i', b'i'], what: KeptWhat::Picked(Box::new(crate::content_store::kept_facts::BlockFlat::of_rows(&rows, 2, true))) };
    store.queue_kept(SPACE, vec![granule(1, vec![9, 1, 8, 1]), granule(2, vec![7, 2, 9, 2, 6, 2])]);
    let bound = Bound::of(&store, &SPACE).unwrap().unwrap();
    let parts: Vec<Entry> = [1, 2].iter().map(|&p| bound.entry("input", &[3; 32], p).expect("a granule")).collect();
    let rel = gathered(&parts, 2, &store.atoms_handle()).expect("gathered");
    assert!(rel.view_kind().is_some(), "a view of the granules");
    let src = rel.source(vec!["a".into(), "b".into()]);
    let rows: Vec<[i64; 2]> = (0..src.len()).map(|r| [src.value(r, 0), src.value(r, 1)]).collect();
    assert_eq!(rows, [[9, 1], [8, 1], [7, 2], [9, 2], [6, 2]], "each granule's rows in the order put, the granules in theirs");
    let mut found = src.lookup(0, &[9]).unwrap();
    found.sort_unstable();
    assert_eq!(found, [0, 3], "a probe across the granules");
    assert!(gathered(&parts, 3, &store.atoms_handle()).is_none(), "another width");
}
