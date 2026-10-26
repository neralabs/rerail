use super::*;
use std::hash::{Hash, Hasher};

#[derive(Default)]
struct Bytes(usize);
impl Hasher for Bytes {
    fn write(&mut self, bytes: &[u8]) {
        self.0 += bytes.len();
    }
    fn finish(&self) -> u64 {
        0
    }
}
fn versions(n: usize) -> Versions {
    let files = (0..n)
        .map(|i| {
            let path = format!("project/long/module/path/file_{i}.py");
            FileInput::new(
                path.clone(),
                "python".into(),
                Arc::new("pass".into()),
                None,
                None,
                &path,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    Versions::new(files.iter().map(|f| (f, [42; 32])))
}
fn key(name: &str, definition: Arc<str>, versions: Versions) -> Key {
    Key::Evidence {
        definition,
        definition_digest: "same-digest".into(),
        clause: "clause".into(),
        name: name.into(),
        versions,
        domain: None,
        nonce: 0,
    }
}
fn hashed_bytes(value: &impl Hash) -> usize {
    let mut hash = Bytes::default();
    value.hash(&mut hash);
    hash.0
}
#[test]
fn repeated_version_identity_hashing_does_not_reread_the_repository() {
    let small = versions(1);
    let large = versions(4096);
    assert!(
        hashed_bytes(&large) <= hashed_bytes(&small) + 64,
        "every key lookup rehashes all source identities"
    );
}
#[test]
fn definition_digest_collisions_still_require_full_equality() {
    let versions = versions(2);
    let a = key("input", Arc::from("first"), versions.clone());
    let b = key("input", Arc::from("second"), versions);
    let hash = |k: &Key| {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        k.hash(&mut h);
        h.finish()
    };
    assert_eq!(
        hash(&a),
        hash(&b),
        "canonical bodies are hashed again despite the cached digest"
    );
    let mut bindings = Bindings::default();
    let a = bindings.intern(a);
    let b = bindings.intern(b);
    assert_ne!(
        a, b,
        "a digest collision coalesced different producer definitions"
    );
}
#[test]
fn metadata_charges_shared_definition_and_versions_once() {
    let versions = versions(128);
    let definition: Arc<str> = Arc::from("canonical policy body ".repeat(1000));
    let mut bindings = Bindings::default();
    bindings.intern(key("first", definition.clone(), versions.clone()));
    let first = bindings.bytes();
    for i in 0..64 {
        bindings.intern(key(
            &format!("input_{i}"),
            definition.clone(),
            versions.clone(),
        ));
    }
    assert!(
        bindings.bytes() - first < 64 * 1024,
        "shared owners are charged once per input: {} -> {}",
        first,
        bindings.bytes()
    );
    bindings.forget();
    assert_eq!(bindings.bytes(), 0);
}
