//! The interner's own tests (the store's tests of the atoms open a store: `content_store/atoms/tests.rs`).
use super::*;

/// Decision 0478 step 2a: the run's pending atoms are in atoms blocks, the stored chain's format, each string where
/// a block holds it (a device reads it there): none in a heap buffer. The chain grows past its first block, every
/// atom is found by its string and read by its id, and the blocks of atoms stored since are let go.
#[test]
fn pending_atoms_live_in_atoms_blocks() {
    let mut i = Interner::default();
    let n = 3 * pending_first_cap() as usize;
    let strings: Vec<String> = (0..n).map(|k| format!("atom-{k}-{}", "x".repeat(k % 37))).collect();
    let ids: Vec<Sym> = strings.iter().map(|s| i.intern(s)).collect();
    assert_eq!(ids, (1..=n as Sym).collect::<Vec<_>>(), "dense ids from the first");
    let blocks = i.pending_regions();
    assert!(blocks.len() >= 2, "the chain grew past its first block: {blocks:?}");
    for (s, &id) in strings.iter().zip(&ids) {
        let got = i.str(id).expect("a pending atom's string");
        assert_eq!(got, s);
        let at = got.as_ptr() as usize;
        assert!(blocks.iter().any(|&(lo, hi)| at >= lo && at + got.len() <= hi), "{s}: read where an atoms block holds it");
        assert_eq!(i.get(s), Some(id));
        assert_eq!(i.hash_of(id), Some(hash_str(s)));
    }
    assert_eq!(i.intern(&strings[5]), ids[5], "interned again: the same id");
    assert_eq!(i.pending(), (n, strings.iter().map(String::len).sum()));
    assert_eq!(i.len(), n + 1);
}
