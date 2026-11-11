//! `@git_commit(sha)` and `@git_has(sha, path)`: questions about Git objects, answered in process from the
//! object store of the repository that holds the run's root, through gix. Never a `git` subprocess.
//!
//! A full or abbreviated object id names immutable content, so an answer never goes stale: it is kept for the
//! process, and no invalidation is needed. Only the names that move are read again each run, and recorded for the
//! whole-run cutoff (`consulted`, `still`): the index (`@git_tracked`), `HEAD` and the base reference (`git_rev`,
//! `revisions`). Paths are relative to the run's root, as every policy path is.
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
    /// The index as this run read it, with its file's digest: read at most once per run.
    index: Option<Option<(gix::index::File, String)>>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// The root of a run, for the run's length: calls answer from the repository that holds it. A run inside another
/// (phase 0's `bootstrap.rr` run, a phase) sets its own and puts the outer run's back when it ends.
pub struct Scope(Option<PathBuf>);

pub(in crate::code_rule_dsl) fn scope(root: &Path) -> Scope {
    let previous = STATE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|s| s.root.clone());
    set_root(root);
    Scope(previous)
}

impl Drop for Scope {
    fn drop(&mut self) {
        if let Some(previous) = self.0.take() {
            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
            let state = state.get_or_insert_with(State::default);
            if state.root.as_deref() != Some(previous.as_path()) {
                *state = State { root: Some(previous), ..State::default() };
            }
        }
    }
}

/// The run's root: later calls answer from the repository that holds it, opened on the first call.
pub(in crate::code_rule_dsl) fn set_root(root: &Path) {
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let state = state.get_or_insert_with(State::default);
    if state.root.as_deref() != Some(root) {
        *state = State { root: Some(root.to_owned()), ..State::default() };
    }
    // The index moves between runs; object answers do not.
    state.index = None;
    state.answers.retain(|(name, _, _), _| *name != "git_tracked");
}

fn sha256_file(path: &Path) -> String {
    use sha2::Digest as _;
    std::fs::read(path).map(|b| sha2::Sha256::digest(&b).iter().map(|x| format!("{x:02x}")).collect()).unwrap_or_default()
}

fn tracked(state: &mut State, path: &str) -> Option<bool> {
    let root = state.root.clone();
    let repo = state.repo.get_or_insert_with(|| root.as_deref().and_then(open)).as_ref()?;
    let full = repo.prefix.join(path);
    let full = full.to_str()?.replace('\\', "/");
    let index = state.index.get_or_insert_with(|| {
        let file = repo.repo.open_index().ok()?;
        let digest = sha256_file(&repo.repo.index_path());
        Some((file, digest))
    });
    let (index, _) = index.as_ref()?;
    use gix::bstr::ByteSlice;
    Some(index.entry_by_path(full.as_bytes().as_bstr()).is_some())
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
    let found = if name == "git_tracked" {
        tracked(state, path)
    } else {
        let root = state.root.clone();
        let repo = state.repo.get_or_insert_with(|| root.as_deref().and_then(open));
        repo.as_ref().and_then(|repo| if name == "git_commit" { commit(repo, sha) } else { has(repo, sha, path) })
    };
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

/// `@git_tracked(path)`: 1 when `path` is a file in the index (`git ls-files`: staged additions count, a file only on
/// disk does not), else 0.
pub(super) fn git_tracked(path: &str) -> Value {
    answer("git_tracked", "", path)
}

/// What the run over `root` read that moves: `("git_index", root, digest)` when `@git_tracked` read the index. For
/// the whole-run cutoff's record (`Report::consulted`). The state is the process's, one root at a time: a run over
/// another root since gives nothing, so it is never recorded against this one.
pub(in crate::code_rule_dsl) fn consulted(root: &Path) -> Vec<(String, String, String)> {
    let guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(state) = guard.as_ref() else { return Vec::new() };
    match (&state.root, &state.index) {
        (Some(r), Some(index)) if r == root => vec![("git_index".to_owned(), r.to_string_lossy().into_owned(), index.as_ref().map(|i| i.1.clone()).unwrap_or_default())],
        _ => Vec::new(),
    }
}

/// The commit `rev` names: a revision spec (`HEAD~1`, `origin/main`, an id).
fn commit_of(repo: &gix::Repository, rev: &str) -> Result<gix::ObjectId, String> {
    let id = repo.rev_parse_single(rev).map_err(|e| format!("{rev}: {e}"))?;
    let object = id.object().map_err(|e| format!("{rev}: {e}"))?;
    Ok(object.peel_to_commit().map_err(|e| format!("{rev}: {e}"))?.id)
}

/// `git_rev`'s rows for the repository holding `root`: `("head", HEAD)`, and with a base reference, `("base", the
/// merge base of HEAD and it)`, the revision a change is compared against. Also the root's path in the work tree.
pub(in crate::code_rule_dsl) fn revisions(root: &Path, base: Option<&str>) -> Result<(Vec<(String, String)>, PathBuf), String> {
    let repo = open(root).ok_or_else(|| format!("no Git work tree holds {}", root.display()))?;
    let head = commit_of(&repo.repo, "HEAD")?;
    let mut rows = vec![("head".to_owned(), head.to_string())];
    if let Some(base) = base {
        let other = commit_of(&repo.repo, base)?;
        let merge = repo.repo.merge_base(head, other).map_err(|e| format!("the merge base of HEAD and {base}: {e}"))?;
        // gix 0.90 answers no common ancestor as none, where 0.84 gave an error.
        let merge = merge.ok_or_else(|| format!("HEAD and {base} have no merge base"))?;
        rows.push(("base".to_owned(), merge.detach().to_string()));
    }
    Ok((rows, repo.prefix))
}

/// Whether what `consulted` recorded (and `git_rev`'s reads, `("git_rev", "REV", id)`) reads the same now.
pub(in crate::code_rule_dsl) fn still(what: &str, path: &str, digest: &str) -> bool {
    match what {
        "git_index" => open(Path::new(path)).map(|r| sha256_file(&r.repo.index_path())).unwrap_or_default() == digest,
        "git_rev" => {
            let Some((root, rev)) = path.split_once('\t') else { return false };
            let Ok((rows, _)) = revisions(Path::new(root), (!rev.is_empty()).then_some(rev)) else { return digest.is_empty() };
            rows.iter().map(|(n, id)| format!("{n}={id}")).collect::<Vec<_>>().join(",") == digest
        }
        _ => false,
    }
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
        // The index: a committed file, a staged addition, a file only on disk.
        assert_eq!(git_tracked("docs/a.md"), Value::Integer(1));
        std::fs::write(root.join("docs/b.md"), "b\n").unwrap();
        std::fs::write(root.join("docs/c.md"), "c\n").unwrap();
        assert_eq!(git_tracked("docs/b.md"), Value::Integer(0), "only on disk");
        git(&["add", "sub/docs/b.md"]);
        set_root(&root);
        assert_eq!(git_tracked("docs/b.md"), Value::Integer(1), "staged, read again in the next run");
        assert_eq!(git_tracked("docs/c.md"), Value::Integer(0));
        let recorded = consulted(&root);
        assert!(consulted(Path::new("/elsewhere")).is_empty());
        assert!(still(&recorded[0].0, &recorded[0].1, &recorded[0].2), "the index as it was");
        git(&["add", "sub/docs/c.md"]);
        assert!(!still(&recorded[0].0, &recorded[0].1, &recorded[0].2), "the index moved");
        // git_rev: HEAD, and the merge base of HEAD and a branch that moved on.
        git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "two"]);
        let two = git(&["rev-parse", "HEAD"]);
        git(&["checkout", "-q", "-b", "side", &head]);
        std::fs::write(root.join("docs/d.md"), "d\n").unwrap();
        git(&["add", "sub/docs/d.md"]);
        git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "three"]);
        let three = git(&["rev-parse", "HEAD"]);
        let (rows, prefix) = revisions(&root, Some(&two)).unwrap();
        assert_eq!(rows, [("head".to_owned(), three), ("base".to_owned(), head.clone())]);
        assert_eq!(prefix, Path::new("sub"));
        // A run inside a run (phase 0's) answers from its own root, and the outer run's comes back after it.
        {
            let _outer = scope(&root);
            {
                let _inner = scope(&std::env::temp_dir().join("rerail-no-repo-here"));
                assert_eq!(git_commit(&head), Value::Null, "the inner run's root: no repository");
            }
            assert_eq!(git_commit(&head), Value::Integer(1), "the outer run's root again");
        }
        set_root(&std::env::temp_dir().join("rerail-no-repo-here"));
        assert_eq!(git_commit(&head), Value::Null, "no repository: no value");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
