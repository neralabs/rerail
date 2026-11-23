use super::super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn store(label: &str) -> ContentStore {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("rerail-tree-diff-{label}-{}-{nonce}.sqlite", std::process::id()));
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

fn change(path: &str, tree: bool, n: Option<u8>) -> TreeChange {
    TreeChange { path: path.to_owned(), tree, digest: n.map_or(ABSENT, |n| tree_diff::digest(hash(n).lanes)) }
}

/// Root 100 = { src/ (10: a.py=1, b.py=2), lib/ (20: c.py=3), README=4 }.
fn base(store: &ContentStore) -> (GitTree, GitTree) {
    let src = tree(10, vec![file("a.py", 1), file("b.py", 2)]);
    let lib = tree(20, vec![file("c.py", 3)]);
    let root = tree(100, vec![dir("src", &src), dir("lib", &lib), file("README", 4)]);
    store.put_git_trees_bulk(&[src.clone(), lib.clone(), root]).unwrap();
    (src, lib)
}

#[test]
fn an_edit_reads_only_the_trees_on_its_path() {
    let store = store("edit");
    let (_, lib) = base(&store);
    let src = tree(11, vec![file("a.py", 5), file("b.py", 2)]);
    let root = tree(101, vec![dir("src", &src), dir("lib", &lib), file("README", 4)]);
    store.put_git_trees_bulk(&[src, root]).unwrap();
    let diff = store.tree_changes(Some(hash(100)), hash(101)).unwrap();
    assert_eq!(
        diff.changes,
        [change("", true, Some(101)), change("src", true, Some(11)), change("src/a.py", false, Some(5))]
    );
    // Both roots, then both `src` trees; `lib` is the same tree on both sides and is never read.
    assert_eq!(diff.trees_read, 4);
}

#[test]
fn the_first_observation_lists_every_path() {
    let store = store("first");
    base(&store);
    let diff = store.tree_changes(None, hash(100)).unwrap();
    assert_eq!(
        diff.changes,
        [
            change("", true, Some(100)),
            change("README", false, Some(4)),
            change("lib", true, Some(20)),
            change("lib/c.py", false, Some(3)),
            change("src", true, Some(10)),
            change("src/a.py", false, Some(1)),
            change("src/b.py", false, Some(2)),
        ]
    );
}

#[test]
fn a_removed_directory_unbinds_it_and_everything_under_it() {
    let store = store("remove");
    let (src, _) = base(&store);
    let root = tree(102, vec![dir("src", &src), file("README", 4)]);
    store.put_git_trees_bulk(&[root]).unwrap();
    let diff = store.tree_changes(Some(hash(100)), hash(102)).unwrap();
    assert_eq!(
        diff.changes,
        [change("", true, Some(102)), change("lib", true, None), change("lib/c.py", false, None)]
    );
}

#[test]
fn a_file_replaced_by_a_directory_changes_both_paths() {
    let store = store("switch");
    let (src, lib) = base(&store);
    let readme = tree(30, vec![file("index.md", 6)]);
    let root = tree(103, vec![dir("src", &src), dir("lib", &lib), dir("README", &readme)]);
    store.put_git_trees_bulk(&[readme, root]).unwrap();
    let diff = store.tree_changes(Some(hash(100)), hash(103)).unwrap();
    assert_eq!(
        diff.changes,
        [
            change("", true, Some(103)),
            change("README", false, None),
            change("README", true, Some(30)),
            change("README/index.md", false, Some(6)),
        ]
    );
}

#[test]
fn the_same_root_changes_nothing_and_an_unknown_root_is_an_error() {
    let store = store("same");
    base(&store);
    assert_eq!(store.tree_changes(Some(hash(100)), hash(100)).unwrap(), TreeDiff::default());
    assert!(store.tree_changes(Some(hash(100)), hash(250)).is_err());
}
