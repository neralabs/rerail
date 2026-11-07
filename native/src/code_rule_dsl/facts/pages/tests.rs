use super::*;
use crate::code_rule_dsl::facts::observe::{observe_root, push_root};
use crate::code_rule_dsl::facts::overlay::{Limits, Overlay};
use crate::content_store::{ContentStore, GitTree, GitTreeEntry, HashKey};
use std::cell::RefCell;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

fn hash(n: u8) -> HashKey {
    HashKey::git_sha1([n; 20])
}

fn file(name: &str, n: u8) -> GitTreeEntry {
    GitTreeEntry { name: name.to_owned(), mode: 0o100644, target: hash(n) }
}

fn tree(n: u8, entries: Vec<GitTreeEntry>) -> GitTree {
    GitTree { id: hash(n), prefix: String::new(), entries }
}

fn dir(name: &str, tree: &GitTree) -> GitTreeEntry {
    GitTreeEntry { name: name.to_owned(), mode: 0o040000, target: tree.id }
}

/// root n = { src/ (src), lib/ (lib) }.
fn root(store: &ContentStore, n: u8, src: u8, lib: u8) -> HashKey {
    let src = tree(src, vec![file("a.py", src)]);
    let lib = tree(lib, vec![file("c.py", lib)]);
    let root = tree(n, vec![dir("src", &src), dir("lib", &lib)]);
    store.put_git_trees_bulk(&[src, lib, root.clone()]).unwrap();
    root.id
}

fn pages<'a>(scope: &'a str, prefix: &'a str) -> DirectoryPages<'a> {
    DirectoryPages { scope, prefix, plan: [7; 32], context: [0; 32] }
}

#[test]
fn a_subtree_one_workspace_evaluated_is_a_lookup_for_another_and_edits_elsewhere_skip_it() {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let base = std::env::temp_dir().join(format!("rerail-pages-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let store = Arc::new(ContentStore::open(&base.join("store.sqlite")).unwrap());
    let limits = Limits { top: 1 << 20, resident: 1 << 20, behind: 4, spill: base.join("spill"), background: true };
    let overlay = Overlay::new(Arc::clone(&store), limits).unwrap();
    let computed = RefCell::new(Vec::<String>::new());
    let evaluate = |scope: &str, prefix: &str| {
        let directory = pages(scope, prefix);
        evaluate_pages(&overlay, directory, || {
            computed.borrow_mut().push(format!("{}:{}", directory.scope, directory.prefix));
            let tree = overlay.binding(&tree_name(directory.scope, directory.prefix)).unwrap().unwrap();
            Ok(vec![TreePage { file_count: 1, bytes: tree.to_vec() }])
        })
        .unwrap()
    };

    let first = root(&store, 100, 10, 20);
    observe_root(&overlay, "/one", first).unwrap();
    observe_root(&overlay, "/two", first).unwrap();
    for scope in ["/one", "/two"] {
        for prefix in ["src", "lib"] {
            let evaluated = evaluate(scope, prefix);
            overlay.record_derivations(&[evaluated.record]).unwrap();
        }
    }
    // /two holds the same subtrees: its pages were lookups.
    assert_eq!(computed.borrow().as_slice(), ["/one:src", "/one:lib"]);

    // An edit under src in /one re-runs /one's src page only; lib's is not in any frontier.
    let edited = root(&store, 101, 11, 20);
    let mut waves = Vec::new();
    let push = push_root(&overlay, "/one", edited, 8, |frontier| {
        waves.push(frontier.iter().map(|f| f.output.clone()).collect::<Vec<_>>());
        Ok(frontier
            .iter()
            .map(|f| evaluate("/one", f.output.rsplit(':').next().unwrap()).record)
            .collect())
    })
    .unwrap();
    assert_eq!(waves, [vec![pages("/one", "src").output()]]);
    assert_eq!((push.trees_read, push.rebound, push.work.rerun, push.work.changed), (4, 3, 1, 1));
    assert_eq!(computed.borrow().len(), 3);

    // /two then makes the same edit: the src page is a lookup of what /one computed.
    let push = push_root(&overlay, "/two", edited, 8, |frontier| {
        Ok(frontier.iter().map(|_| evaluate("/two", "src").record).collect())
    })
    .unwrap();
    assert_eq!(push.work.rerun, 1);
    assert_eq!(computed.borrow().len(), 3);
    overlay.flush().unwrap();
    for scope in ["/one", "/two"] {
        for prefix in ["src", "lib"] {
            assert!(store.derivation_valid(&pages(scope, prefix).output()).unwrap());
        }
    }
}
