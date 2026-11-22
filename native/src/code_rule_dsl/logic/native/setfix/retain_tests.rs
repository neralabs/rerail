//! setfix's relations under the planner's retention (`plan::corpus::retain`): a relation's digest and kept rows, read
//! where its rows lie. Its tests read the planner, a crate above setfix: `rerail-logic`'s facade of `native::setfix`
//! mounts this file.
use super::*;

/// A relation over a source (a kept relation bound in place, decision 0456 step 6b) has no rows of its own: its
/// values, its live rows and their batches are read through it, as a held relation's are (a value of it once
/// panicked, "row 0 of a relation of 0 rows", and its live rows were none, so a kept stratum's put held no rows).
#[test]
fn a_relation_over_a_source_reads_its_rows_through_it() {
    let src = super::columns_source(vec!["a".into(), "b".into()], vec![Some(vec![1, 2, 5]), Some(vec![3, 4, 0])]);
    let r = InRel::over_source(src, b"ii".to_vec());
    assert_eq!((r.len(), r.rows()), (3, 3));
    assert_eq!((0..3).map(|i| [r.value(i, 0), r.value(i, 1)]).collect::<Vec<_>>(), [[1, 3], [2, 4], [5, 0]], "its values");
    assert_eq!(r.live_rows().as_deref(), Some(&[1, 3, 2, 4, 5, 0][..]), "its live rows");
    let mut batches = Vec::new();
    assert!(r.each_live_batch(1, 9, |b| batches.push(b.to_vec())).is_some());
    assert_eq!(batches, [vec![2, 4, 5, 0]], "rows 1.. in one batch, bounded by its rows");
    let cells = crate::code_rule_dsl::logic::plan::Cells::plain(2);
    // Its digest is its rows', as the same rows held flat have (a sum, blind to their order): a kept relation
    // bound in place and passed on unchanged is its entry, not re-kept. Re-kept (its rows changed), every row is
    // put, read through it: rr-align's 4e3d12466 re-kept views by a frame and `kept_rows`, which take a relation
    // in a block, and lost their rows (7d8103eed's warm-run losses, which it met by naming a view by its reads).
    use crate::code_rule_dsl::logic::plan::corpus::retain;
    let flat = InRel::from_flat(vec![5, 0, 1, 3, 2, 4], 3, b"ii".to_vec());
    let d = retain::held_digest(&cells, &r);
    assert!(d.is_some() && d == retain::held_digest(&cells, &flat), "a view's digest is its rows'");
    let put = retain::kept_put(&crate::content_store::KeptRel { name: "r".into(), ident: [1; 32] }, 0, Vec::new(), b"ii".to_vec(), &r);
    match put.what {
        // Read through it where they lie as the writer packs them (decision 0353 step 8), not copied flat.
        crate::content_store::kept_facts::KeptWhat::Read { rows, .. } => {
            use rerail_blocks::packed::RowsIn as _;
            let got: Vec<i64> = (0..rows.n()).flat_map(|r| (0..rows.arity()).map(move |c| (r, c))).map(|(r, c)| rows.get(r, c)).collect();
            assert_eq!((got, rows.n()), (vec![1, 3, 2, 4, 5, 0], 3), "every row of the view kept");
        }
        _ => panic!("a view is kept as its rows, read through it"),
    }
}

/// A relation's digest taken from its rows where they lie (`retain::held_digest`) is the digest of a copy of its
/// live rows (`rows_digest`), dead rows and batch edges included.
#[test]
fn a_digest_read_in_place_is_the_digest_of_the_copied_rows() {
    use crate::code_rule_dsl::logic::plan::{Cells, rows_digest};
    use crate::code_rule_dsl::logic::plan::corpus::retain::held_digest;
    let n = 3 * LIVE_BATCH + 17;
    let mut rel = Rel::new("t", b"iii");
    for i in 0..n as i64 {
        rel.push(&[i * 3 - 7, (i * 7919) % 1001 - 500, -i]);
    }
    let old = InRel::moved(rel);
    let mut gone = Rel::new("g", b"iii");
    for id in [0usize, 5, LIVE_BATCH - 1, LIVE_BATCH, 2 * LIVE_BATCH + 3, n - 1] {
        let r = old.row(id).to_vec();
        gone.push(&r);
    }
    let dead = InRel::moved(Rel::based("t", b"iii", &old, Some(&InRel::moved(gone))).expect("few dead"));
    for held in [&old, &dead] {
        let live = held.live_rows().expect("flat");
        let cells = Cells::plain(3);
        assert_eq!(held_digest(&cells, held), Some(rows_digest(&cells, &live, held.rows())), "{} dead", held.ndead);
    }
}
