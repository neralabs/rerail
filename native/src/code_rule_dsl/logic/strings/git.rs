//! `@git_commit(sha)` and `@git_has(sha, path)`: questions about Git objects, answered in process from the
//! object store of the repository that holds the run's root, through gix. Never a `git` subprocess.
//!
//! A full or abbreviated object id names immutable content, so an answer never goes stale: it is kept for the
//! process, and no invalidation is needed. Only the names that move (`head`, `base`, the index) are phase 0's
//! facts, checked by its observer. Paths are relative to the run's root, as every policy path is.
//!
//! With no repository, or an object id that is malformed or ambiguous, a call has no value (NULL), so the row is
//! not derived: never a confident 0.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::types::Value;

struct Repo {
    repo: gix::Repository,
    /// The run root's path inside the work tree.
    prefix: PathBuf,
}

#[derive(Default)]
struct State {
    root: Option<PathBuf>,
    repo: Option<Option<Repo>>,
    answers: HashMap<(&'static str, String, String), Option<bool>>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// The run's root: later calls answer from the repository that holds it, opened on the first call.
pub(in crate::code_rule_dsl) fn set_root(root: &Path) {
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let state = state.get_or_insert_with(State::default);
    if state.root.as_deref() != Some(root) {
        *state = State { root: Some(root.to_owned()), ..State::default() };
    }
}

fn open(root: &Path) -> Option<Repo> {
    let repo = gix::discover(root).ok()?;
    let workdir = repo.workdir()?.canonicalize().ok()?;
    let prefix = root.canonicalize().ok()?.strip_prefix(&workdir).ok()?.to_path_buf();
    Some(Repo { repo, prefix })
}

/// The object a full or abbreviated (at least 4 digits) hex id names, if exactly one does.
fn resolve(repo: &gix::Repository, sha: &str) -> Option<gix::ObjectId> {
    if let Ok(id) = gix::ObjectId::from_hex(sha.as_bytes()) {
        return repo.has_object(id).then_some(id);
    }
    let prefix = gix::hash::Prefix::from_hex(sha).ok()?;
    match repo.objects.lookup_prefix(prefix, None) {
        Ok(Some(Ok(id))) => Some(id),
        _ => None,
    }
}

fn commit(repo: &Repo, sha: &str) -> Option<bool> {
    let Some(id) = resolve(&repo.repo, sha) else { return Some(false) };
    Some(repo.repo.find_object(id).ok()?.kind == gix::object::Kind::Commit)
}

fn has(repo: &Repo, sha: &str, path: &str) -> Option<bool> {
    let Some(id) = resolve(&repo.repo, sha) else { return Some(false) };
    let object = repo.repo.find_object(id).ok()?;
    if object.kind != gix::object::Kind::Commit {
        return Some(false);
    }
    let tree = object.peel_to_tree().ok()?;
    Some(tree.lookup_entry_by_path(repo.prefix.join(path)).ok()?.is_some())
}

fn answer(name: &'static str, sha: &str, path: &str) -> Value {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let state = guard.get_or_insert_with(State::default);
    let key = (name, sha.to_owned(), path.to_owned());
    if let Some(known) = state.answers.get(&key) {
        return known.map_or(Value::Null, |b| Value::Integer(b.into()));
    }
    let root = state.root.clone();
    let repo = state.repo.get_or_insert_with(|| root.as_deref().and_then(open));
    let found = repo.as_ref().and_then(|repo| if name == "git_commit" { commit(repo, sha) } else { has(repo, sha, path) });
    state.answers.insert(key, found);
    found.map_or(Value::Null, |b| Value::Integer(b.into()))
}

/// `@git_commit(sha)`: 1 when `sha` names a commit of the repository, else 0.
pub(super) fn git_commit(sha: &str) -> Value {
    answer("git_commit", sha, "")
}

/// `@git_has(sha, path)`: 1 when the commit `sha` names has `path` (a file or a directory) in its tree, else 0.
pub(super) fn git_has(sha: &str, path: &str) -> Value {
    answer("git_has", sha, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commits_and_paths_are_answered_from_the_object_store() {
        let dir = std::env::temp_dir().join(format!("rerail-git-fn-{}", std::process::id()));
        let root = dir.join("sub");
        std::fs::create_dir_all(root.join("docs")).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git").arg("-C").arg(&dir).args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}");
            String::from_utf8(out.stdout).unwrap().trim().to_owned()
        };
        git(&["init", "-q"]);
        std::fs::write(root.join("docs/a.md"), "a\n").unwrap();
        git(&["add", "sub/docs/a.md"]);
        git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "one"]);
        let head = git(&["rev-parse", "HEAD"]);
        let blob = git(&["rev-parse", "HEAD:sub/docs/a.md"]);
        set_root(&root);
        assert_eq!(git_commit(&head), Value::Integer(1));
        assert_eq!(git_commit(&head[..7]), Value::Integer(1), "an abbreviated id");
        assert_eq!(git_commit(&blob), Value::Integer(0), "a blob is not a commit");
        assert_eq!(git_commit("0000000000000000000000000000000000000000"), Value::Integer(0));
        assert_eq!(git_commit("not hex"), Value::Integer(0));
        assert_eq!(git_has(&head, "docs/a.md"), Value::Integer(1), "paths are the root's");
        assert_eq!(git_has(&head, "docs"), Value::Integer(1));
        assert_eq!(git_has(&head, "docs/b.md"), Value::Integer(0));
        set_root(&std::env::temp_dir().join("rerail-no-repo-here"));
        assert_eq!(git_commit(&head), Value::Null, "no repository: no value");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
