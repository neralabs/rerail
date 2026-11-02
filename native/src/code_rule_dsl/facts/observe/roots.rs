//! The roots observers push (06 S3): a directory snapshot, hashed into Git trees here, and a Git
//! checkout's `HEAD` tree, read by W's `git_workspace`. Each stores its trees and the sources of
//! the files a family reads (paths with a tape dialect), each source under its Git blob id, so a
//! path name's digest finds its source (`families::blob_source`).

use crate::code_rule_dsl::language::label_of_path;
use crate::content_store::{ContentStore, GitTree, GitTreeEntry, HashKey, StoreError, StoreResult};
use crate::git_workspace;
use std::path::Path;

const BLOB: u16 = 0o100644;
const EXECUTABLE: u16 = 0o100755;
const TREE: u16 = 0o040000;

fn git_hash(kind: gix::objs::Kind, bytes: &[u8]) -> StoreResult<[u8; 20]> {
    let id = gix::objs::compute_hash(gix::hash::Kind::Sha1, kind, bytes)
        .map_err(|error| StoreError::Invalid(format!("cannot hash a Git object: {error}")))?;
    Ok(id.as_bytes().try_into().expect("a SHA-1 is twenty bytes"))
}

/// Stores `bytes` under the Git blob id `blob` unless a source has that alias already.
fn store_source(store: &ContentStore, blob: HashKey, bytes: &[u8]) -> StoreResult<()> {
    if store.source_by_alias(blob)?.is_none() {
        store.put_source(bytes, &[blob])?;
    }
    Ok(())
}

/// Snapshots the directory `root` as Git trees (SHA-1, Git's own entry order and encoding, so a
/// clean checkout's snapshot is its `HEAD` tree), stores them and the sources of its files with a
/// language, and returns the root tree's id. `.git` and symbolic links are left out, and so are
/// empty directories, as Git leaves them out.
pub(crate) fn directory_root(store: &ContentStore, root: &Path) -> StoreResult<HashKey> {
    buffered_root(store, root, &std::collections::BTreeMap::new())
}

/// `directory_root` with editor buffers over it (the buffers observer, O2): each buffer's bytes
/// stand for the file at its path (relative, `/`-separated), on disk or not yet, in place of what
/// is on disk.
pub(crate) fn buffered_root(store: &ContentStore, root: &Path, buffers: &std::collections::BTreeMap<String, Vec<u8>>) -> StoreResult<HashKey> {
    let mut trees = Vec::new();
    let id = directory_tree(store, Some(root), "", buffers, &mut trees)?.unwrap_or(git_hash(gix::objs::Kind::Tree, b"")?);
    if trees.is_empty() {
        trees.push(GitTree { id: HashKey::git_sha1(id), prefix: String::new(), entries: Vec::new() });
    }
    store.put_git_trees_bulk(&trees)?;
    Ok(HashKey::git_sha1(id))
}

/// The tree of `dir` at `prefix` (`None`: a directory only buffers hold), with `buffers` over
/// it, pushed onto `trees` after its subtrees; `None` when empty.
fn directory_tree(
    store: &ContentStore,
    dir: Option<&Path>,
    prefix: &str,
    buffers: &std::collections::BTreeMap<String, Vec<u8>>,
    trees: &mut Vec<GitTree>,
) -> StoreResult<Option<[u8; 20]>> {
    let mut children: Vec<(String, bool, Option<std::path::PathBuf>)> = Vec::new();
    if let Some(dir) = dir {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let kind = entry.file_type()?;
            if name == ".git" || kind.is_symlink() {
                continue;
            }
            children.push((name, kind.is_dir(), Some(entry.path())));
        }
    }
    // Buffers below this tree that name an entry the disk does not have: a file, or a directory
    // only buffers hold.
    let below = if prefix.is_empty() { String::new() } else { format!("{prefix}/") };
    for path in buffers.keys().filter_map(|path| path.strip_prefix(below.as_str())) {
        let (name, is_dir) = match path.split_once('/') {
            Some((name, _)) => (name, true),
            None => (path, false),
        };
        if !name.is_empty() && !children.iter().any(|(held, _, _)| held == name) {
            children.push((name.to_owned(), is_dir, None));
        }
    }
    // Git orders a tree's entries by name, a directory's as if it ended in '/'.
    children.sort_by(|(a, a_dir, _), (b, b_dir, _)| {
        let key = |name: &str, dir: bool| [name.as_bytes(), if dir { b"/" } else { b"" }].concat();
        key(a, *a_dir).cmp(&key(b, *b_dir))
    });
    let mut entries = Vec::with_capacity(children.len());
    let mut encoded = Vec::new();
    for (name, is_dir, path) in children {
        let child = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        let (mode, id) = if is_dir {
            match directory_tree(store, path.as_deref(), &child, buffers, trees)? {
                Some(id) => (TREE, id),
                None => continue,
            }
        } else {
            let bytes = match (buffers.get(&child), &path) {
                (Some(buffer), _) => buffer.clone(),
                (None, Some(path)) => std::fs::read(path)?,
                (None, None) => continue,
            };
            let id = git_hash(gix::objs::Kind::Blob, &bytes)?;
            if label_of_path(&child).is_some() {
                store_source(store, HashKey::git_sha1(id), &bytes)?;
            }
            let executable = match &path {
                Some(path) => executable(path)?,
                None => false,
            };
            (if executable { EXECUTABLE } else { BLOB }, id)
        };
        encoded.extend_from_slice(format!("{mode:o} {name}\0").as_bytes());
        encoded.extend_from_slice(&id);
        entries.push(GitTreeEntry { name, mode, target: HashKey::git_sha1(id) });
    }
    if entries.is_empty() {
        return Ok(None);
    }
    let id = git_hash(gix::objs::Kind::Tree, &encoded)?;
    trees.push(GitTree { id: HashKey::git_sha1(id), prefix: prefix.to_owned(), entries });
    Ok(Some(id))
}

#[cfg(unix)]
fn executable(path: &Path) -> StoreResult<bool> {
    use std::os::unix::fs::PermissionsExt as _;
    Ok(std::fs::metadata(path)?.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable(_: &Path) -> StoreResult<bool> {
    Ok(false)
}

/// The `HEAD` tree of the Git checkout containing `root` (W's discovery), stored with the
/// committed sources of its files with a language, as Git holds them; worktree edits are not
/// part of `HEAD` (a directory snapshot observes them).
pub(crate) fn git_root(store: &ContentStore, root: &Path) -> StoreResult<HashKey> {
    let workspace = git_workspace::discover(root).map_err(|error| StoreError::Invalid(format!("{error:?}")))?;
    store.put_git_trees_bulk(&workspace.trees)?;
    let repository = gix::open(&workspace.worktree_root)
        .map_err(|error| StoreError::Invalid(format!("cannot open the Git repository: {error}")))?;
    for file in workspace.base_files.iter().filter(|file| label_of_path(&file.path).is_some()) {
        if store.source_by_alias(file.blob)?.is_some() {
            continue;
        }
        let id = gix::ObjectId::from_bytes_or_panic(&file.blob.git_sha1_bytes().ok_or_else(|| {
            StoreError::Invalid("only SHA-1 repositories are observed".to_owned())
        })?);
        let blob = repository
            .find_object(id)
            .map_err(|error| StoreError::Invalid(format!("cannot read a committed blob: {error}")))?;
        store.put_source(&blob.data, &[file.blob])?;
    }
    Ok(workspace.root_tree)
}
