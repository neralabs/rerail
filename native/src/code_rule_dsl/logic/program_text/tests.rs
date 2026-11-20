use crate::code_rule_dsl::logic::rr::parse_version;

/// A stored program is kept as its image alone (decisions 0173, 0246 §6), placed in a segment and read there, not
/// copied (decision 0480); one kept as text alone (a store written before images) is not read: a miss, which its run
/// compiles again.
#[test]
fn a_stored_program_is_read_from_its_image() {
    use crate::content_store::{ContentStore, HashKey};
    let dir = std::env::temp_dir().join(format!("rerail-program-image-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = ContentStore::open(&dir.join("store.sqlite")).unwrap();
    let prog = parse_version(".decl e(x: number)\n.decl p(x: number)\ne(1).\np(x) :- e(x), x < 3.\n.output p\n").unwrap();
    let encoded = super::encode_version(&prog).expect("kept");
    assert!(encoded.image, "an image");
    let digest = encoded.digest;
    let sources = super::store_encoded(&store, &digest, encoded.image, (*encoded.bytes).as_ref()).unwrap();
    assert_eq!(sources.len(), 1, "its image alone");
    assert!(store.source_by_alias(super::image_key(&digest)).unwrap().is_some());
    let found = super::stored(&store, &[digest]).unwrap();
    let (bytes, ids) = &found[&digest];
    assert_eq!((**bytes).as_ref(), (*encoded.bytes).as_ref(), "read from its image");
    assert!(store.source_by_alias_in_place(super::image_key(&digest)).unwrap().expect("held").in_place(), "placed in a segment, read there");
    assert_eq!(ids, &sources, "the image's source");
    let (back, _) = super::load_image(&store, &digest).unwrap().expect("loads");
    let shown = |v: &crate::code_rule_dsl::logic::program_image::Version| v.rules().map(|r| v.show_rule(r)).collect::<Vec<_>>();
    assert_eq!(shown(&back), shown(&prog));
    assert_eq!(back.output_names(), ["p"]);
    // Kept as text alone (a store written before images): not read.
    let text = ".decl p(x: int)\n.rules\n";
    store.put_source_hashed(text.as_bytes(), HashKey::sha256(text.as_bytes()), &[]).unwrap();
    let d2: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(text.as_bytes()).into();
    assert!(super::stored(&store, &[d2]).unwrap().is_empty());
    assert!(super::load_image(&store, &d2).unwrap().is_none());
    assert_eq!(super::sources(&store, &[d2]).unwrap().len(), 1, "its source is named, so collection keeps it with its run");
    drop(store);
    let _ = std::fs::remove_dir_all(&dir);
}
