// The sketch alone: what its relation reads of it (`small`) is not this binary's.
#[allow(dead_code)]
#[path = "../../../src/code_rule_dsl/logic/native/setfix/sketch.rs"]
mod sketch;
#[test]
fn compact_integer_domains_keep_exact_counts_past_64_distinct_values() {
    let mut s=sketch::Sketch::default();
    let mut sq=0;
    for value in 0..200 { for _ in 0..value+1 { s.add(value); } sq+=(value as u64+1).pow(2); }
    assert_eq!(s.ndv(),200,"bounded labels must retain exact cardinality");
    assert_eq!(s.sq(),sq,"bounded labels must retain exact posting counts");
    assert_eq!(s.exact_counts().unwrap().iter().copied().collect::<std::collections::HashMap<_,_>>()[&137],138);
    assert!(s.bytes()<20_000,"histogram memory must be bounded");
    s.add(1_i64<<40);
    assert!(s.exact_counts().is_none(),"an escaped domain must fall back, not claim a complete histogram");
}
#[test]
fn sparse_label_domains_also_keep_exact_counts_with_bounded_memory() {
    let mut s=sketch::Sketch::default();
    for i in 0..200 {for _ in 0..3 {s.add(i*293);}}
    assert_eq!(s.exact_counts().unwrap().len(),200);
    assert_eq!(s.sq(),1800);
    assert!(s.bytes()<65536);
    for i in 200..2000 {s.add((i*293)%60000);}
    assert!(s.exact_counts().is_none(),"large domains must return to bounded sketches");
}
#[test]
fn label_ids_do_not_lose_histograms_when_the_atom_arena_grows() {
    let mut s=sketch::Sketch::default();
    for i in 0..200 {s.add(600_000+i*293);}
    assert_eq!(s.exact_counts().unwrap().len(),200);
}
