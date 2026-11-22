//! The git workspace's tests (`git_workspace`), mounted by the store (rerail-store): they publish a workspace's
//! snapshot to a `ContentStore` and collect it.
use crate::git_workspace::*;
// `git_workspace.rs`'s own imports, which the tests read as its children did.
use rerail_store::Canonical as _;
use crate::content_store::{GitTree, GitTreeEntry, HashKey};
#[allow(unused_imports)]
use std::collections::{BTreeMap, BTreeSet, HashSet};
#[allow(unused_imports)]
use std::fmt;
use std::path::{Path, PathBuf};
#[cfg(unix)]
#[allow(unused_imports)]
use std::os::unix::ffi::OsStrExt;
use crate::content_store::{ContentStore, SnapshotId, SourceId, StoreResult, WorkspaceId, WorkspaceRegistration};
use crate::content_store::test_support::TemporaryDatabase;

/// A repository is discovered from a directory inside it, with no `git` process: its files in order, its
/// ignored ones left out, an edit an overlay of the clean trees, a source read and aliased to its blob, and its
/// snapshot published once. Over a repository of its own: the checkout the test was built in need not be one (a
/// farm host's copy of the tree has no `.git`).
#[test]
fn discovers_current_repository_without_a_git_process() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    std::fs::create_dir_all(root.join("native/src")).unwrap();
    initialize_repository(root, &[
        ("native/build.rs", b"fn main() {}\n"), ("native/src/lib.rs", b"pub fn run() {}\n"),
        (".gitignore", b"tmp/\n"),
    ]);
    std::fs::write(root.join("native/src/lib.rs"), b"pub fn edited() {}\n").unwrap();
    std::fs::create_dir(root.join("tmp")).unwrap();
    std::fs::write(root.join("tmp/ignored"), b"ignored").unwrap();
    let workspace = discover(&root.join("native/src")).unwrap();
    assert!(workspace.worktree_root.is_absolute());
    assert!(workspace.common_directory.is_absolute());
    assert!(!workspace.trees.is_empty());
    assert!(
        workspace
            .base_files
            .windows(2)
            .all(|pair| pair[0].path < pair[1].path)
    );
    assert!(
        workspace
            .base_files
            .iter()
            .any(|file| file.path == "native/src/lib.rs"),
        "source files: {:?}",
        workspace
            .base_files
            .iter()
            .filter(|file| file.path.starts_with("native/"))
            .take(20)
            .map(|file| &file.path)
            .collect::<Vec<_>>()
    );
    assert!(
        workspace
            .base_files
            .iter()
            .all(|file| !file.path.starts_with("tmp/"))
    );
    let clean_trees = workspace.clean_trees();
    assert!(clean_trees.iter().all(|(prefix, _)| {
        !workspace.overlays.iter().any(|overlay| {
            prefix.is_empty()
                || overlay.path == *prefix
                || overlay.path.starts_with(&format!("{prefix}/"))
        })
    }));
    let indexed = workspace.index_files();
    assert!(indexed.windows(2).all(|pair| pair[0].0 < pair[1].0));
    assert!(workspace.overlays.iter().all(|overlay| {
        overlay.kind == OverlayKind::Deleted
            || indexed
                .iter()
                .any(|(path, clean)| path == &overlay.path && !clean)
    }));
    let source = workspace.read_source("native/build.rs", u64::MAX).unwrap();
    assert!(!source.is_empty());
    let alias = workspace
        .source_alias("native/build.rs", &source)
        .expect("unchanged source has Git alias");

    let (_database, store) = temporary_store("discovery");
    let registration = published_workspace();
    let published = workspace
        .publish_snapshot(&store, 1, HashKey::sha256(b"grammar"), 1, &registration)
        .unwrap();
    assert_eq!(
        workspace
            .publish_snapshot(&store, 1, HashKey::sha256(b"grammar"), 1, &registration)
            .unwrap(),
        published
    );
    let source_id = workspace.store_source(&store, "native/build.rs").unwrap();
    assert_eq!(store.source_by_alias(alias).unwrap(), Some(source_id));
    assert!(workspace.overlays.iter().any(|overlay| overlay.path == "native/src/lib.rs"), "the edit is an overlay");
    let _ = std::fs::remove_dir_all(root);
}

fn run_git(directory: &Path, arguments: &[&str]) {
    // The machine's git settings (a signing key, hooks, a default branch) are not the test's.
    let output = std::process::Command::new("git")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git -C {} {} failed:\n{}",
        directory.display(),
        arguments.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn initialize_repository(root: &Path, files: &[(&str, &[u8])]) {
    std::fs::create_dir_all(root).unwrap();
    run_git(root, &["init", "--initial-branch=main"]);
    run_git(root, &["config", "user.email", "rerail-tests@example.com"]);
    run_git(root, &["config", "user.name", "Rerail tests"]);
    for (path, contents) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }
    let mut paths = vec!["add", "--"];
    paths.extend(files.iter().map(|(path, _)| *path));
    run_git(root, &paths);
    run_git(root, &["commit", "-m", "initial"]);
}

fn fixture_directory(name: &str) -> PathBuf {
    static NEXT_FIXTURE: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "rerail-git-workspace-{name}-{}-{}",
        std::process::id(),
        NEXT_FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ))
}

/// Takes away this account's right to list `folder`, as another account has it on a records
/// folder several accounts share: it may still pass through and make what it makes there.
#[cfg(unix)]
fn deny_listing(folder: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o300)).unwrap();
}

#[cfg(unix)]
fn allow_listing(folder: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700)).unwrap();
}

/// The same on Windows: an entry that denies this account listing `folder` and nothing more.
#[cfg(windows)]
fn deny_listing(folder: &Path) {
    icacls_for_this_user(folder, "/deny", ":(RD)");
}

#[cfg(windows)]
fn allow_listing(folder: &Path) {
    icacls_for_this_user(folder, "/remove:d", "");
}

#[cfg(windows)]
fn icacls_for_this_user(folder: &Path, action: &str, rights: &str) {
    let user = std::env::var("USERNAME").unwrap();
    let status = std::process::Command::new("icacls")
        .arg(folder)
        .arg(action)
        .arg(format!("{user}{rights}"))
        .status()
        .unwrap();
    assert!(status.success());
}

/// What `git status --porcelain --untracked-files=all` prints, as (two-letter code, path)
/// sorted by path.
fn porcelain(root: &Path) -> Vec<(String, String)> {
    let output = std::process::Command::new("git")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
        .unwrap();
    assert!(output.status.success());
    // Git prints the changed paths first, then the untracked ones.
    let mut lines: Vec<(String, String)> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| (line[..2].to_owned(), line[3..].to_owned()))
        .collect();
    lines.sort_by(|left, right| left.1.cmp(&right.1));
    lines
}

/// The overlays as `git status --porcelain` codes them, for the kinds the tests below make.
fn overlay_codes(workspace: &GitWorkspace) -> Vec<(String, String)> {
    workspace
        .overlays
        .iter()
        .map(|overlay| {
            let code = match overlay.kind {
                OverlayKind::Untracked => "??",
                OverlayKind::Unstaged => " M",
                OverlayKind::Staged => "M ",
                OverlayKind::StagedAndUnstaged => "MM",
                OverlayKind::Deleted => " D",
            };
            (code.to_owned(), overlay.path.clone())
        })
        .collect()
}

#[test]
fn a_records_folder_this_account_may_not_list_is_skipped_as_git_skips_it() {
    let root = fixture_directory("unlistable-records");
    initialize_repository(
        &root,
        &[("kept.py", b"kept\n"), ("changed.py", b"before\n")],
    );
    std::fs::write(root.join("changed.py"), b"after\n").unwrap();
    std::fs::write(root.join("new.py"), b"new\n").unwrap();
    let records = root.join(".rerail").join("sessions");
    std::fs::create_dir_all(records.join("another-accounts-session")).unwrap();
    std::fs::write(records.join(".gitignore"), b"*\n").unwrap();
    std::fs::write(
        records.join("another-accounts-session/events.jsonl"),
        b"{}\n",
    )
    .unwrap();
    std::fs::write(root.join(".rerail/beside.txt"), b"beside\n").unwrap();

    deny_listing(&records);
    let discovered = discover(&root);
    let status = porcelain(&root);
    allow_listing(&records);

    let workspace = discovered.unwrap();
    assert_eq!(overlay_codes(&workspace), status);
    assert_eq!(
        status,
        [
            ("??", ".rerail/beside.txt"),
            (" M", "changed.py"),
            ("??", "new.py")
        ]
        .map(|(code, path)| (code.to_owned(), path.to_owned()))
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// Takes away this account's right to read `file`'s attributes, as another account has it on a
/// file in a records folder after the folder's repair; on Unix its folder's mode does that.
#[cfg(windows)]
fn deny_attributes(file: &Path) {
    icacls_for_this_user(file, "/deny", ":(RA)");
}

#[cfg(windows)]
fn allow_attributes(file: &Path) {
    icacls_for_this_user(file, "/remove:d", "");
}

#[cfg(unix)]
fn deny_attributes(file: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let folder = file.parent().unwrap();
    std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o000)).unwrap();
}

#[cfg(unix)]
fn allow_attributes(file: &Path) {
    allow_listing(file.parent().unwrap());
}

#[test]
fn a_records_folder_git_made_whose_file_this_account_may_not_read_is_left_out() {
    let root = fixture_directory("unreadable-records");
    initialize_repository(&root, &[("kept.py", b"kept\n")]);
    let records = root.join(".rerail").join("sessions");
    std::fs::create_dir_all(&records).unwrap();
    std::fs::write(records.join("notes.md"), b"kept\n").unwrap();
    run_git(&root, &["add", "--", ".rerail/sessions/notes.md"]);
    run_git(&root, &["commit", "-m", "a file in the records folder"]);
    std::fs::write(records.join(".gitignore"), b"*\n").unwrap();
    std::fs::write(root.join("new.py"), b"new\n").unwrap();
    let status = porcelain(&root);

    deny_attributes(&records.join("notes.md"));
    let discovered = discover(&root);
    allow_attributes(&records.join("notes.md"));

    let workspace = discovered.unwrap();
    assert_eq!(overlay_codes(&workspace), status);
    assert_eq!(status, [("??".to_owned(), "new.py".to_owned())]);
    assert!(
        workspace
            .base_files
            .iter()
            .all(|file| !file.path.starts_with(".rerail/"))
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_folder_git_ignores_that_this_account_may_not_list_is_never_listed() {
    let root = fixture_directory("unlistable-ignored");
    initialize_repository(
        &root,
        &[(".gitignore", b"build/\n"), ("kept.py", b"kept\n")],
    );
    let build = root.join("build");
    std::fs::create_dir(&build).unwrap();
    std::fs::write(build.join("out.o"), b"out\n").unwrap();

    deny_listing(&build);
    let discovered = discover(&root);
    allow_listing(&build);

    assert!(discovered.unwrap().overlays.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_folder_git_does_not_ignore_that_this_account_may_not_list_still_stops_discovery() {
    let root = fixture_directory("unlistable-untracked");
    initialize_repository(&root, &[("kept.py", b"kept\n")]);
    let folder = root.join("private");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("notes.txt"), b"notes\n").unwrap();

    deny_listing(&folder);
    let discovered = discover(&root);
    allow_listing(&folder);

    let error = discovered
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(error.contains("cannot read Git status item"), "{error}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn staged_rename_removes_the_source_path() {
    let root = fixture_directory("staged-rename");
    initialize_repository(&root, &[("old.py", b"print('old')\n")]);
    run_git(&root, &["mv", "old.py", "new.py"]);

    let workspace = discover(&root).unwrap();

    assert_eq!(workspace.index_files(), vec![("new.py".to_owned(), false)]);
    assert!(workspace.read_source("old.py", u64::MAX).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn discovers_clean_php_sources() {
    let root = fixture_directory("php");
    initialize_repository(&root, &[("source.php", b"<?php echo 'rerail';\n")]);

    let workspace = discover(&root).unwrap();

    assert_eq!(
        workspace.index_files(),
        vec![("source.php".to_owned(), true)]
    );
    assert_eq!(
        workspace.read_source("source.php", u64::MAX).unwrap(),
        b"<?php echo 'rerail';\n"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn reads_worktree_bytes_when_a_clean_file_changes_after_discovery() {
    let root = fixture_directory("changed-after-discovery");
    initialize_repository(&root, &[("source.py", b"print('before')\n")]);
    let workspace = discover(&root).unwrap();
    std::fs::write(root.join("source.py"), b"print('after')\n").unwrap();

    assert_eq!(
        workspace.read_source("source.py", u64::MAX).unwrap(),
        b"print('after')\n"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn reads_checkout_bytes_for_clean_eol_transformed_sources() {
    let root = fixture_directory("eol");
    initialize_repository(
        &root,
        &[
            (".gitattributes", b"*.py text eol=crlf\n"),
            ("source.py", b"print('crlf')\r\n"),
        ],
    );
    let workspace = discover(&root).unwrap();

    assert_eq!(
        workspace.read_source("source.py", u64::MAX).unwrap(),
        b"print('crlf')\r\n"
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// Discovery compares the index with the HEAD tree it walked: a commit that lands after the
/// walk overlays the files it changed, so their walked blobs never alias the new bytes.
#[test]
fn a_commit_during_discovery_overlays_the_files_it_changed() {
    let root = fixture_directory("commit-during-discovery");
    initialize_repository(
        &root,
        &[
            ("changed.py", b"print('a')\n"),
            ("kept.py", b"print('kept')\n"),
        ],
    );
    let committer = root.clone();
    AFTER_HEAD_TREE_VISIT.set(Some(Box::new(move || {
        std::fs::write(committer.join("changed.py"), b"print('b')\n").unwrap();
        run_git(&committer, &["commit", "-qam", "during discovery"]);
    })));

    let workspace = discover(&root).unwrap();

    assert!(
        AFTER_HEAD_TREE_VISIT.take().is_none(),
        "the commit landed inside discovery"
    );
    assert_eq!(
        workspace.overlays,
        [WorkspaceOverlay {
            path: "changed.py".to_owned(),
            kind: OverlayKind::Staged,
        }],
        "the file the commit changed differs from the walked HEAD tree"
    );
    assert_eq!(
        workspace.clean_source_alias("changed.py"),
        None,
        "a file committed after the walk is not restored from the walked blob"
    );
    assert!(workspace.clean_source_alias("kept.py").is_some());
    std::fs::remove_dir_all(root).unwrap();
}

/// An untracked scratch repository without commits cannot be discovered; the outer worktree
/// still is, with the scratch directory as untracked content that is read, never restored.
#[test]
fn an_untracked_repository_without_commits_is_untracked_content() {
    let root = fixture_directory("unborn-nested");
    std::fs::create_dir_all(root.join("lib")).unwrap();
    initialize_repository(&root, &[("lib/a.py", b"print('a')\n")]);
    let scratch = root.join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    run_git(&scratch, &["init", "--initial-branch=main"]);
    std::fs::write(scratch.join("x.py"), b"print('x')\n").unwrap();

    let workspace = match discover(&root) {
        Ok(workspace) => workspace,
        Err(error) => panic!("an unborn nested repository failed discovery: {error}"),
    };

    assert_eq!(
        workspace.overlays,
        [WorkspaceOverlay {
            path: "scratch".to_owned(),
            kind: OverlayKind::Untracked,
        }]
    );
    assert_eq!(workspace.clean_source_alias("scratch/x.py"), None);
    assert!(workspace.clean_source_alias("lib/a.py").is_some());
    let clean = workspace
        .clean_trees()
        .into_iter()
        .map(|(prefix, _)| prefix)
        .collect::<Vec<_>>();
    assert_eq!(clean, ["lib"]);
    std::fs::remove_dir_all(root).unwrap();
}

/// A submodule checkout is a separate repository, never folded into the outer worktree: its
/// files are not base files, and its directory is an overlaid nested root, so neither it nor
/// its ancestors are clean.
#[test]
fn a_submodule_checkout_is_an_overlaid_nested_root() {
    let root = fixture_directory("undiscoverable-submodule");
    let submodule = root.join("submodule");
    let parent = root.join("parent");
    initialize_repository(&submodule, &[("nested.py", b"print('nested')\n")]);
    std::fs::create_dir_all(parent.join("lib")).unwrap();
    std::fs::create_dir_all(parent.join("vendor")).unwrap();
    initialize_repository(
        &parent,
        &[
            ("lib/a.py", b"print('a')\n"),
            ("vendor/other.py", b"print('other')\n"),
        ],
    );
    run_git(
        &parent,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "../submodule",
            "vendor/sub",
        ],
    );
    run_git(&parent, &["commit", "-am", "add submodule"]);

    let workspace = discover(&parent).unwrap();

    assert_eq!(
        workspace.nested_roots,
        [parent.canonical().unwrap().join("vendor/sub")]
    );

    assert_eq!(
        workspace.overlays,
        [WorkspaceOverlay {
            path: "vendor/sub".to_owned(),
            kind: OverlayKind::Untracked,
        }]
    );
    assert_eq!(workspace.clean_source_alias("vendor/sub/nested.py"), None);
    assert!(
        !workspace
            .base_files
            .iter()
            .any(|file| file.path.starts_with("vendor/sub/"))
    );
    assert!(workspace.clean_source_alias("vendor/other.py").is_some());
    let clean = workspace
        .clean_trees()
        .into_iter()
        .map(|(prefix, _)| prefix)
        .collect::<Vec<_>>();
    assert_eq!(clean, ["lib"]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn mandatory_ignore_prunes_nested_repositories_before_opening_them() {
    let root = fixture_directory("mandatory-nested-ignore");
    let submodule = root.join("submodule");
    let parent = root.join("parent");
    initialize_repository(&submodule, &[("source.py", b"print('nested')\n")]);
    std::fs::create_dir(submodule.join(".rerailignore")).unwrap();
    initialize_repository(&parent, &[("root.py", b"print('root')\n")]);
    run_git(
        &parent,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "../submodule",
            "node_modules",
        ],
    );
    run_git(&parent, &["commit", "-am", "add submodule"]);

    let workspace = discover(&parent).unwrap();
    assert!(workspace.nested_roots.is_empty());

    assert!(
        !workspace
            .index_files()
            .iter()
            .any(|(path, _)| path.starts_with("node_modules/"))
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn discovers_linked_worktree_submodule_and_ignores_a_stale_gitfile() {
    static NEXT_FIXTURE: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);

    let fixture = std::env::temp_dir().join(format!(
        "rerail-git-worktree-{}-{}",
        std::process::id(),
        NEXT_FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let parent = fixture.join("parent");
    let submodule = fixture.join("submodule");
    let linked = fixture.join("linked");
    initialize_repository(&submodule, &[("nested.rs", b"pub fn nested() {}\n")]);
    initialize_repository(&parent, &[("root.rs", b"pub fn root() {}\n")]);
    run_git(
        &parent,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "../submodule",
            "nested",
        ],
    );
    run_git(&parent, &["commit", "-am", "add submodule"]);
    run_git(
        &parent,
        &["worktree", "add", "--detach", linked.to_str().unwrap()],
    );
    run_git(
        &linked,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "update",
            "--init",
        ],
    );

    let linked_root = linked.canonical().unwrap();
    let workspace = discover(&linked).unwrap();
    assert_eq!(workspace.worktree_root, linked_root);
    assert!(
        !workspace
            .base_files
            .iter()
            .any(|file| file.path.starts_with("nested/"))
    );
    assert_eq!(workspace.nested_roots, [linked_root.join("nested")]);
    assert!(
        workspace
            .overlays
            .iter()
            .any(|overlay| overlay.path == "nested")
    );

    std::fs::copy(parent.join("nested/.git"), linked.join("nested/.git")).unwrap();
    let workspace = discover(&linked).unwrap();
    assert_eq!(workspace.worktree_root, linked.canonical().unwrap());
    assert!(
        !workspace
            .base_files
            .iter()
            .any(|file| file.path == "nested/nested.rs")
    );

    std::fs::remove_dir_all(fixture).unwrap();
}

#[cfg(unix)]
#[test]
fn dirty_source_reads_reject_symlink_escapes() {
    use std::os::unix::fs::symlink;
    let root =
        std::env::temp_dir().join(format!("rerail-anchored-read-{}", std::process::id()));
    let outside =
        std::env::temp_dir().join(format!("rerail-anchored-outside-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&outside);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("source.py"), b"print('escaped')\n").unwrap();
    symlink(&outside, root.join("escape")).unwrap();
    let workspace = GitWorkspace {
        snapshot: None,
        worktree_root: root.clone(),
        common_directory: root.clone(),
        hash_algorithm: GitHashAlgorithm::Sha1,
        root_tree: HashKey::git_sha1([0; 20]),
        trees: Vec::new(),
        base_files: Vec::new(),
        overlays: vec![WorkspaceOverlay {
            path: "escape/source.py".to_owned(),
            kind: OverlayKind::Untracked,
        }],
        nested_roots: Vec::new(),
    };
    assert!(workspace.read_source("escape/source.py", u64::MAX).is_err());
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(outside).unwrap();
}

#[cfg(unix)]
#[test]
fn dirty_source_reads_reject_fifos_without_blocking() {
    let root = fixture_directory("fifo");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("pipe.py");
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    let workspace = GitWorkspace {
        snapshot: None,
        worktree_root: root.clone(),
        common_directory: root.clone(),
        hash_algorithm: GitHashAlgorithm::Sha1,
        root_tree: HashKey::git_sha1([0; 20]),
        trees: Vec::new(),
        base_files: Vec::new(),
        overlays: vec![WorkspaceOverlay {
            path: "pipe.py".to_owned(),
            kind: OverlayKind::Untracked,
        }],
        nested_roots: Vec::new(),
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        sender
            .send(workspace.read_source("pipe.py", u64::MAX).is_err())
            .unwrap();
    });

    assert_eq!(
        receiver.recv_timeout(std::time::Duration::from_millis(250)),
        Ok(true)
    );
    reader.join().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rerailignore_uses_gitignore_negation() {
    let mut workspace = gix::ignore::Search::default();
    workspace
        .add_patterns_buffer(
            b"generated/\n!generated/keep.py\n!tmp/keep.py\n",
            ".rerailignore",
            None,
            gix::ignore::search::Ignore::default(),
        )
        .expect("the patterns parse");
    let mut mandatory = gix::ignore::Search::default();
    mandatory
        .add_patterns_buffer(
            BUILTIN_IGNORE,
            ".rerailignore.defaults",
            None,
            gix::ignore::search::Ignore::default(),
        )
        .expect("the patterns parse");
    let rules = IgnoreRules {
        mandatory,
        workspace,
        case: gix::ignore::glob::pattern::Case::Sensitive,
    };
    assert!(ignored(&rules, "generated/drop.py"));
    assert!(!ignored(&rules, "generated/keep.py"));
    assert!(ignored(&rules, "tmp/keep.py"));
}

/// A records folder named in another case is one where `core.ignoreCase` folds case, as on
/// Windows, so the walk skips it there as the built-in ignore and status's pathspec do.
#[test]
fn a_records_folder_is_matched_with_the_repository_case_folding() {
    use gix::ignore::glob::pattern::Case;
    for path in [".rerail/sessions", "sub/.rerail/sessions"] {
        assert!(records_folder(path.as_bytes(), Case::Sensitive), "{path}");
        assert!(records_folder(path.as_bytes(), Case::Fold), "{path}");
    }
    for path in [".Rerail/sessions", "sub/.RERAIL/Sessions"] {
        assert!(records_folder(path.as_bytes(), Case::Fold), "{path}");
        assert!(!records_folder(path.as_bytes(), Case::Sensitive), "{path}");
    }
    for path in [
        "sub.rerail/sessions",
        ".rerail/sessions/kept.py",
        "sessions",
    ] {
        assert!(!records_folder(path.as_bytes(), Case::Fold), "{path}");
    }
}

#[test]
fn clean_trees_comparisons_are_linear_for_many_trees_and_overlays() {
    let count = 2_000;
    let workspace = GitWorkspace {
        snapshot: None,
        worktree_root: PathBuf::new(),
        common_directory: PathBuf::new(),
        hash_algorithm: GitHashAlgorithm::Sha1,
        root_tree: HashKey::git_sha1([0; 20]),
        trees: (0..count)
            .map(|index| GitTree {
                id: HashKey::git_sha1([index as u8; 20]),
                prefix: format!("tree-{index}"),
                entries: Vec::new(),
            })
            .collect(),
        base_files: Vec::new(),
        // Overlays touch every even tree and as many directories without a tree. The dirty
        // set is a hash set of directories: each tree is one lookup, not a scan of overlays.
        overlays: (0..count)
            .map(|index| WorkspaceOverlay {
                path: if index % 2 == 0 {
                    format!("tree-{index}/file.py")
                } else {
                    format!("other-{index}/file.py")
                },
                kind: OverlayKind::Untracked,
            })
            .collect(),
        nested_roots: Vec::new(),
    };
    let clean = workspace
        .clean_trees()
        .into_iter()
        .map(|(prefix, _)| prefix)
        .collect::<Vec<_>>();
    let odd = (1..count)
        .step_by(2)
        .map(|index| format!("tree-{index}"))
        .collect::<Vec<_>>();
    assert_eq!(clean, odd);
}

#[test]
fn a_tree_whose_prefix_an_overlay_names_is_not_clean() {
    let tree = |prefix: &str, byte: u8| GitTree {
        id: HashKey::git_sha1([byte; 20]),
        prefix: prefix.to_owned(),
        entries: Vec::new(),
    };
    let workspace = GitWorkspace {
        snapshot: None,
        worktree_root: PathBuf::new(),
        common_directory: PathBuf::new(),
        hash_algorithm: GitHashAlgorithm::Sha1,
        root_tree: HashKey::git_sha1([0; 20]),
        trees: vec![
            tree("", 0),
            tree("vendor", 1),
            tree("vendor/lib", 2),
            tree("src", 3),
        ],
        base_files: Vec::new(),
        // A gitlink or an untracked nested repository overlays the directory itself.
        overlays: vec![WorkspaceOverlay {
            path: "vendor/lib".to_owned(),
            kind: OverlayKind::Untracked,
        }],
        nested_roots: Vec::new(),
    };
    let clean = workspace
        .clean_trees()
        .into_iter()
        .map(|(prefix, _)| prefix)
        .collect::<Vec<_>>();
    assert_eq!(clean, ["src"]);
}

fn tree_with_entries(byte: u8, prefix: &str, entries: Vec<GitTreeEntry>) -> GitTree {
    GitTree {
        id: HashKey::git_sha1([byte; 20]),
        prefix: prefix.to_owned(),
        entries,
    }
}

fn file_entries(byte: u8, count: u16) -> Vec<GitTreeEntry> {
    (0..count)
        .map(|index| {
            let mut target = [byte; 20];
            target[..2].copy_from_slice(&index.to_be_bytes());
            GitTreeEntry {
                name: format!("file-{index}.py"),
                mode: 0o100644,
                target: HashKey::git_sha1(target),
            }
        })
        .collect()
}

fn directory_entry(tree: &GitTree) -> GitTreeEntry {
    GitTreeEntry {
        name: tree.prefix.clone(),
        mode: 0o040000,
        target: tree.id,
    }
}

/// A workspace over `trees` in publication (post-) order, the root last.
fn tree_workspace(trees: Vec<GitTree>) -> GitWorkspace {
    GitWorkspace {
        snapshot: None,
        worktree_root: PathBuf::new(),
        common_directory: PathBuf::new(),
        hash_algorithm: GitHashAlgorithm::Sha1,
        root_tree: trees.last().unwrap().id,
        trees,
        base_files: Vec::new(),
        overlays: Vec::new(),
        nested_roots: Vec::new(),
    }
}

/// A store on a fresh temporary database; dropping the path removes the database.
fn temporary_store(label: &str) -> (TemporaryDatabase, ContentStore) {
    let path = TemporaryDatabase::new(&format!("rerail-gix-{label}"));
    let store = ContentStore::open(&path).unwrap();
    (path, store)
}

/// Another process's maintenance collects after every Git-tree write transaction.
fn collect_after_every_tree_transaction(path: &Path) {
    let collector = ContentStore::open(path).unwrap();
    crate::content_store::AFTER_GIT_TREE_TRANSACTION.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            collector
                .collect(0, crate::content_store::MAX_COLLECTION_BATCH)
                .unwrap();
        }));
    });
}

fn stop_collecting() {
    crate::content_store::AFTER_GIT_TREE_TRANSACTION.with(|hook| *hook.borrow_mut() = None);
}

fn published_workspace() -> WorkspaceRegistration<'static> {
    WorkspaceRegistration {
        root: "/published",
        repository: "repository",
        owner: "owner",
        generation: 1,
        lease_seconds: i64::MAX,
    }
}

/// The snapshot a publication returned still exists after the collections that ran inside the
/// publication: its workspace re-registers over it.
fn assert_published_snapshot_attaches(
    store: &ContentStore,
    published: StoreResult<(SnapshotId, WorkspaceId)>,
) {
    let (snapshot, workspace) = match published {
        Ok(published) => published,
        Err(error) => panic!("a concurrent collection failed snapshot publication: {error:?}"),
    };
    let attached =
        store.open_workspace("/published", "repository", "owner", snapshot, 1, i64::MAX);
    assert!(
        attached.is_ok(),
        "a concurrent collection deleted the published snapshot before its workspace \
         registered: {:?}",
        attached.err()
    );
    assert_eq!(attached.unwrap(), workspace);
}

/// A republication (which repairs anything collected mid-publication) whose workspace holds
/// the snapshot keeps the whole tree graph through a collection.
fn assert_snapshot_graph_is_live(workspace: &GitWorkspace, store: &ContentStore) {
    let grammar = HashKey::sha256(b"grammar");
    let registration = published_workspace();
    workspace
        .publish_snapshot(store, 1, grammar, 1, &registration)
        .unwrap();
    store
        .collect(0, crate::content_store::MAX_COLLECTION_BATCH)
        .unwrap();
    store.reset_git_tree_write_count();
    workspace
        .publish_snapshot(store, 1, grammar, 1, &registration)
        .unwrap();
    assert_eq!(
        store.git_tree_write_count(),
        0,
        "collection removed trees of a published snapshot"
    );
}

#[test]
fn publication_survives_a_collection_of_its_still_unreachable_trees() {
    let (path, store) = temporary_store("collect-fresh-trees");
    let child = tree_with_entries(81, "src", file_entries(81, 8));
    let root = tree_with_entries(82, "", vec![directory_entry(&child)]);
    let workspace = tree_workspace(vec![child, root]);
    collect_after_every_tree_transaction(&path);
    let published = workspace.publish_snapshot(
        &store,
        1,
        HashKey::sha256(b"grammar"),
        1,
        &published_workspace(),
    );
    stop_collecting();
    assert_published_snapshot_attaches(&store, published);
    assert_snapshot_graph_is_live(&workspace, &store);
}

#[test]
fn publication_survives_a_collection_of_its_already_present_root() {
    let (path, store) = temporary_store("collect-present-root");
    // The first child alone exceeds one publication transaction's entry budget.
    let large = tree_with_entries(83, "large", file_entries(83, 600));
    let small = tree_with_entries(84, "small", file_entries(84, 4));
    let root = tree_with_entries(
        85,
        "",
        vec![directory_entry(&large), directory_entry(&small)],
    );
    // A retired snapshot left the root complete but unreachable, and its children missing.
    store
        .put_git_trees_bulk(std::slice::from_ref(&root))
        .unwrap();
    let workspace = tree_workspace(vec![large, small, root]);
    collect_after_every_tree_transaction(&path);
    let published = workspace.publish_snapshot(
        &store,
        1,
        HashKey::sha256(b"grammar"),
        1,
        &published_workspace(),
    );
    stop_collecting();
    assert_published_snapshot_attaches(&store, published);
    assert_snapshot_graph_is_live(&workspace, &store);
}

/// What the tests publish of a workspace, as the store writes it (`ContentStore::publish_snapshot`, `put_source`).
trait Publish {
    fn publish_snapshot(
        &self,
        store: &ContentStore,
        scanner_version: i64,
        grammar_set: HashKey,
        tape_format_version: i64,
        workspace: &WorkspaceRegistration<'_>,
    ) -> StoreResult<(SnapshotId, WorkspaceId)>;
    fn store_source(&self, store: &ContentStore, relative_path: &str) -> Result<SourceId, DiscoveryError>;
}

impl Publish for GitWorkspace {
    /// Persist the reusable Git tree graph and register `workspace` over its snapshot, in the
    /// snapshot's own transaction. The store's own error is kept, so a caller can tell a busy
    /// store (a cache miss) from a failure.
    fn publish_snapshot(
        &self,
        store: &ContentStore,
        scanner_version: i64,
        grammar_set: HashKey,
        tape_format_version: i64,
        workspace: &WorkspaceRegistration<'_>,
    ) -> StoreResult<(SnapshotId, WorkspaceId)> {
        store.publish_snapshot(
            &self.trees,
            self.root_tree,
            scanner_version,
            grammar_set,
            tape_format_version,
            workspace,
        )
    }

    /// Store evaluated source bytes with a safe Git alias when one exists.
    fn store_source(
        &self,
        store: &ContentStore,
        relative_path: &str,
    ) -> Result<SourceId, DiscoveryError> {
        let bytes = self
            .read_source(relative_path, u64::MAX)
            .map_err(|error| crate::git_workspace::discovery_error("cannot read workspace source", error))?;
        let aliases = self
            .source_alias(relative_path, &bytes)
            .into_iter()
            .collect::<Vec<_>>();
        store
            .put_source(&bytes, &aliases)
            .map_err(|error| crate::git_workspace::discovery_error("cannot persist workspace source", error))
    }
}
