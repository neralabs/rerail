use super::*;
use crate::content_store::{ABSENT, DerivationInput, GitTree, GitTreeEntry};
use std::time::{SystemTime, UNIX_EPOCH};

fn store() -> ContentStore {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("rerail-observe-{}-{nonce}.sqlite", std::process::id()));
    ContentStore::open(&path).unwrap()
}

fn hash(n: u8) -> HashKey {
    HashKey::git_sha1([n; 20])
}

fn file(name: &str, n: u8) -> GitTreeEntry {
    GitTreeEntry { name: name.to_owned(), mode: 0o100644, target: hash(n) }
}

fn dir(name: &str, tree: &GitTree) -> GitTreeEntry {
    GitTreeEntry { name: name.to_owned(), mode: 0o040000, target: tree.id }
}

fn tree(n: u8, entries: Vec<GitTreeEntry>) -> GitTree {
    GitTree { id: hash(n), prefix: String::new(), entries }
}

const WS: &str = "/repo";

/// Records `output` as reading `names` with the digests they are bound to now.
fn record(store: &ContentStore, output: &str, names: &[String], digest: u8) -> DerivationRecord {
    let inputs = names
        .iter()
        .map(|name| DerivationInput { name: Some(name.clone()), digest: store.binding(name).unwrap().unwrap_or(ABSENT) })
        .collect();
    DerivationRecord { output: output.to_owned(), producer: [9; 32], inputs, digest: [digest; 32], status: 0, cost: 1 }
}

#[test]
fn a_blob_edit_reruns_only_what_read_its_path_or_the_trees_above_it() {
    let store = store();
    let src = tree(10, vec![file("a.py", 1), file("b.py", 2)]);
    let lib = tree(20, vec![file("c.py", 3)]);
    let root = tree(100, vec![dir("src", &src), dir("lib", &lib)]);
    store.put_git_trees_bulk(&[src, lib.clone(), root]).unwrap();
    let first = push_root(&store, WS, hash(100), 8, |_| unreachable!()).unwrap();
    assert_eq!(first.rebound, 6);

    // One tape per file, and one result page per directory (Tempo's per-tree pages).
    let readers = [
        ("tape:src/a.py", vec![path_name(WS, "src/a.py")]),
        ("tape:src/b.py", vec![path_name(WS, "src/b.py")]),
        ("tape:lib/c.py", vec![path_name(WS, "lib/c.py")]),
        ("page:src", vec![tree_name(WS, "src")]),
        ("page:lib", vec![tree_name(WS, "lib")]),
    ];
    let records = readers.iter().map(|(o, names)| record(&store, o, names, 1)).collect::<Vec<_>>();
    store.record_derivations(&records).unwrap();

    let src = tree(11, vec![file("a.py", 5), file("b.py", 2)]);
    let root = tree(101, vec![dir("src", &src), dir("lib", &lib)]);
    store.put_git_trees_bulk(&[src, root]).unwrap();
    let mut runs = Vec::new();
    let push = push_root(&store, WS, hash(101), 8, |frontier| {
        runs.push(frontier.iter().map(|d| d.output.clone()).collect::<Vec<_>>());
        Ok(frontier
            .iter()
            .map(|d| {
                let (_, names) = readers.iter().find(|(o, _)| *o == d.output).unwrap();
                record(&store, &d.output, names, 1)
            })
            .collect())
    })
    .unwrap();
    assert_eq!(runs, [vec!["page:src", "tape:src/a.py"]]);
    assert_eq!(push, Push { trees_read: 4, rebound: 3, work: Work { waves: 1, rerun: 2, changed: 0 } });
}

#[test]
fn scopes_do_not_share_names() {
    let store = store();
    store.put_git_trees_bulk(&[tree(100, vec![file("a.py", 1)])]).unwrap();
    let (first, _) = observe_root(&store, "/one", hash(100)).unwrap();
    let (second, _) = observe_root(&store, "/two", hash(100)).unwrap();
    assert_eq!(first, [tree_name("/one", ""), path_name("/one", "a.py")]);
    assert_eq!(second, [tree_name("/two", ""), path_name("/two", "a.py")]);
}

#[test]
fn a_collected_last_root_relists_every_path_but_rebinds_only_changes() {
    let store = store();
    store.put_git_trees_bulk(&[tree(100, vec![file("a.py", 1), file("b.py", 2)])]).unwrap();
    observe_root(&store, WS, hash(100)).unwrap();
    // The scope last saw root 99, whose trees are no longer stored.
    store.bind_names(&[Binding { name: tree_name(WS, ""), digest: [7; 32] }]).unwrap();
    store.put_git_trees_bulk(&[tree(101, vec![file("a.py", 1), file("b.py", 3)])]).unwrap();
    let (changed, trees_read) = observe_root(&store, WS, hash(101)).unwrap();
    assert_eq!(changed, [tree_name(WS, ""), path_name(WS, "b.py")]);
    assert_eq!(trees_read, 1);
}
