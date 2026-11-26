//! A seed join placed on the device and refused there runs on the CPU over the relations it was parsed with (t50
//! step 9).
use super::spec::hex;
use super::*;

/// A seed parsed for the device has no CPU indexes; refused, it gets them on the relations it holds (`indexed`).
/// Parsed again from its spec instead, it looked each relation up by name on the connection: a stratum that waited
/// for its wave is finished later, on the run's thread, after other chunks' jobs had bound their own relations
/// under those names, so it could read another chunk's rows, or find none ("no native relation", Metal at 32 MB).
#[test]
fn a_refused_seed_reads_the_relations_it_was_parsed_with() {
    let path = crate::code_rule_dsl::fixtures::temp();
    drop(crate::content_store::ContentStore::open(&path).unwrap());
    let db = Connection::open(&path).unwrap();
    crate::code_rule_dsl::logic::native::register(&db).unwrap();
    hold_rows(&db, "r", vec![b'i'], rerail_blocks::vector![1, 2, 3], 3);
    let parsed = on(&db).derived.lock().get("r").cloned().expect("r held");
    // A scan of `r`, then a probe of it by the scan's value.
    let r = format!("r:{}", hex("r"));
    let spec = JoinSpec::parse(&format!("pos {r} v0\npos {r} v0\nhead v0")).unwrap().encode();
    let shared = symbols(&db).unwrap();
    let mut syms = Interner::for_loading(&shared);
    let mut j = join_rule_with(&db, &spec, &mut syms, false).unwrap();
    let held = |j: &JoinRule| -> Vec<(Arc<InRel>, Arc<Csr>)> {
        j.steps.iter().filter_map(|s| match s { Step::Atom { src: Src::In(rel, ix), .. } => { let (rel, ix) = j.src_in(rel, ix); Some((Arc::clone(rel), Arc::clone(ix))) } _ => None }).collect()
    };
    let unindexed = held(&j)[1].1.clone();
    // Another chunk's job binds its own `r` on the connection meanwhile.
    hold_rows(&db, "r", vec![b'i'], rerail_blocks::vector![7], 1);
    j.indexed();
    let now = held(&j);
    assert_eq!(now.len(), 2);
    for (rel, _) in &now {
        assert!(Arc::ptr_eq(rel, &parsed), "the seed reads the relation it was parsed with, not the one bound since");
    }
    // The probe's index is the one a parse with the CPU's indexes gives its relation.
    let (rel, ix) = &now[1];
    assert!(Arc::ptr_eq(ix, &rel.index(&ix.cols)), "the probe has the relation's index of its key");
    assert!(!Arc::ptr_eq(ix, &unindexed), "an index made, not the device's unindexed one");
}

/// A refused seed's count over an input gets its index too, its keys in that index's column order (t50 step 9). Only
/// its atoms did: a count kept the device's unindexed probe, which finds no rows, so every count was 0 (on Metal,
/// `up_argc`'s arguments counted 0, and the bench gave 829 and 711 rows for 463).
#[test]
fn a_refused_seeds_count_reads_its_inputs_index() {
    let path = crate::code_rule_dsl::fixtures::temp();
    drop(crate::content_store::ContentStore::open(&path).unwrap());
    let db = Connection::open(&path).unwrap();
    crate::code_rule_dsl::logic::native::register(&db).unwrap();
    hold_rows(&db, "t", vec![b'i', b'i'], rerail_blocks::vector![1, 10, 2, 20], 2);
    hold_rows(&db, "r", vec![b'i', b'i'], rerail_blocks::vector![1, 10, 1, 10, 2, 20, 2, 21], 4);
    let r_rel = on(&db).derived.lock().get("r").cloned().expect("r held");
    // Another join made `r`'s index by its columns in the other order first: a probe reads its key in that order.
    let first = r_rel.index(&[1, 0]);
    let (t, r) = (format!("r:{}", hex("t")), format!("r:{}", hex("r")));
    let spec = JoinSpec::parse(&format!("pos {t} v0 v1\ncount {r} v0 v1 v2\nhead v0 v2")).unwrap().encode();
    let shared = symbols(&db).unwrap();
    let mut syms = Interner::for_loading(&shared);
    let mut j = join_rule_with(&db, &spec, &mut syms, false).unwrap();
    j.indexed();
    let store = Store { rels: Vec::new(), syms: shared };
    let Step::Scalar { src: src @ Src::In(rel, ix), keys, kind: ScalarKind::Count, .. } = &j.steps[1] else { panic!("a count of an input") };
    let (rel, ix) = j.src_in(rel, ix);
    assert!(Arc::ptr_eq(rel, &r_rel));
    assert!(Arc::ptr_eq(ix, &first), "the count has the relation's index of its key");
    assert_eq!(keys.iter().map(|&(c, _)| c).collect::<Vec<_>>(), ix.cols, "its keys in the index's column order");
    // Each of `t`'s rows' counts in `r`, read through that index as the step reads it.
    for (key, want) in [([1i64, 10], 2usize), ([2, 20], 1)] {
        let mut probe = [0i64; 2];
        for (at, &(c, _)) in keys.iter().enumerate() {
            probe[at] = key[c];
        }
        let (rows, cands) = cands::candidates(&j, src, &store, &probe, keys, &mut [0; runs::HINTS], &mut LastProbe::default(), 0, None, false);
        let mut n = 0;
        cands.any(|id| {
            let row = rows.row(id);
            n += usize::from(row.at(0) == key[0] && row.at(1) == key[1]);
            false
        });
        assert_eq!(n, want, "the count of {key:?}");
    }
}

/// An aggregate's input gets its index too: left with the device's unindexed one (whose `get` holds no rows), a
/// refused stratum's `min` over it found nothing, and its head lost every row of a chunk (t50 step 9, Metal at 512 MB:
/// `rf_dec_code`, empty in the chunks whose device run was refused).
#[test]
fn a_refused_seeds_aggregate_gets_its_index() {
    let path = crate::code_rule_dsl::fixtures::temp();
    drop(crate::content_store::ContentStore::open(&path).unwrap());
    let db = Connection::open(&path).unwrap();
    crate::code_rule_dsl::logic::native::register(&db).unwrap();
    // (key, j): key 1's least j is 3, key 2's is 7.
    hold_rows(&db, "r", vec![b'i', b'i'], rerail_blocks::vector![1, 5, 1, 3, 2, 7], 3);
    let r = format!("r:{}", hex("r"));
    // Each row whose j is its key's least: `pos r v0 v1`, then the least j of `r` by v0, into v1 (a test).
    let spec = JoinSpec::parse(&format!("pos {r} v0 v1\nmin {r} v0 a0 1 v1\nhead v0 v1")).unwrap().encode();
    let shared = symbols(&db).unwrap();
    let mut syms = Interner::for_loading(&shared);
    let mut j = join_rule_with(&db, &spec, &mut syms, false).unwrap();
    let scalar = |j: &JoinRule| j.steps.iter().find_map(|s| match s { Step::Scalar { src: Src::In(rel, ix), .. } => { let (rel, ix) = j.src_in(rel, ix); Some((Arc::clone(rel), Arc::clone(ix))) }, _ => None }).expect("the aggregate reads r");
    let (_, unindexed) = scalar(&j);
    assert!(unindexed.get(&scalar(&j).0, &[1]).is_empty(), "parsed for the device, the aggregate's index holds no rows");
    j.indexed();
    let (rel, ix) = scalar(&j);
    assert!(!Arc::ptr_eq(&ix, &unindexed), "an index made, not the device's unindexed one");
    assert!(Arc::ptr_eq(&ix, &rel.index(&ix.cols)), "the aggregate has its relation's index of its key");
}
