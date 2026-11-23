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
    drop(crate::code_rule_dsl::fixtures::open(&path));
    let db = Connection::open(&path).unwrap();
    crate::code_rule_dsl::logic::native::register(&db).unwrap();
    hold_rows(&db, "r", vec![b'i'], vec![1, 2, 3], 3);
    let parsed = on(&db).derived.lock().get("r").cloned().expect("r held");
    // A scan of `r`, then a probe of it by the scan's value.
    let r = format!("r:{}", hex("r"));
    let spec = JoinSpec::parse(&format!("pos {r} v0\npos {r} v0\nhead v0")).unwrap();
    let shared = symbols(&db).unwrap();
    let mut syms = Interner::for_loading(&shared);
    let mut j = join_rule_with(&db, &spec, &mut syms, false).unwrap();
    let held = |j: &JoinRule| -> Vec<(Arc<InRel>, Arc<Csr>)> {
        j.steps.iter().filter_map(|s| match s { Step::Atom { src: Src::In(rel, ix), .. } => Some((Arc::clone(rel), Arc::clone(ix))), _ => None }).collect()
    };
    let unindexed = held(&j)[1].1.clone();
    // Another chunk's job binds its own `r` on the connection meanwhile.
    hold_rows(&db, "r", vec![b'i'], vec![7], 1);
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
