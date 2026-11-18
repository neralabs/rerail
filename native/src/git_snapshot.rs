//! Read-only source views of Git trees. Paths identify objects; they never name an extracted checkout.
//! Like nera-data-layer's library Git importer, tree walks retain blob IDs and bytes are read on demand.

use std::cell::RefCell;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use super::HashKey;
use gix::bstr::ByteSlice;

#[derive(Debug)]
struct View {
    id: u64,
    root: PathBuf,
    repository: gix::ThreadSafeRepository,
    prefix: PathBuf,
    tree: gix::ObjectId,
    commit: Option<gix::ObjectId>,
    cache_root: PathBuf,
    scope: Option<String>,
    owner: OnceLock<crate::content_store::analysis_scope::AnalysisOwner>,
    inventory: OnceLock<Inventory>,
    ignore_blob: OnceLock<Result<Option<gix::ObjectId>, String>>,
    paths: Mutex<Option<Vec<String>>>,
    host_languages: OnceLock<SourceLanguages>,
}

#[derive(Debug)]
pub struct Inventory {
    pub files: Vec<super::BaseFile>,
    pub trees: Vec<crate::content_store::GitTree>,
    pub languages: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct SourceLanguages {
    pub suffixes: std::collections::BTreeMap<String, String>,
    pub names: std::collections::BTreeMap<String, String>,
}

type InventoryReader = fn(&Path, Option<&Path>) -> Result<Inventory, String>;
static READ_INVENTORY: OnceLock<InventoryReader> = OnceLock::new();

pub fn install_inventory_reader(reader: InventoryReader) {
    READ_INVENTORY.get_or_init(|| reader);
}

static VIEWS: Mutex<Vec<Arc<View>>> = Mutex::new(Vec::new());
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
thread_local! {
    // One object-store handle per worker, reused across reads. No cache of a snapshot's whole contents.
    static REPOSITORY: RefCell<Option<(u64, gix::Repository)>> = const { RefCell::new(None) };
}

fn error(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}
fn missing() -> io::Error {
    io::Error::from(io::ErrorKind::NotFound)
}

#[derive(Clone, Copy)]
enum Entry {
    File(gix::ObjectId),
    Dir(gix::ObjectId),
}

impl View {
    fn ignore_blob(&self) -> Result<Option<gix::ObjectId>, String> {
        self.ignore_blob.get_or_init(|| self.with_repo(|repo| {
            let tree = repo.find_tree(self.tree).map_err(|e| e.to_string())?;
            let entry = tree.lookup_entry_by_path(".rerailignore").map_err(|e| e.to_string())?;
            Ok(entry.filter(|entry| entry.mode().is_blob()).map(|entry| entry.id().detach()))
        })).clone()
    }

    fn inventory(&self, store: Option<&Path>) -> Result<&Inventory, String> {
        if let Some(value) = self.inventory.get() {
            return Ok(value);
        }
        let read = READ_INVENTORY.get().ok_or("Git tree inventory reader is unavailable")?;
        let value = read(&self.root, store)?;
        Ok(self.inventory.get_or_init(|| value))
    }

    fn with_repo<T>(&self, f: impl FnOnce(&gix::Repository) -> T) -> T {
        REPOSITORY.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.as_ref().is_none_or(|(id, _)| *id != self.id) {
                *slot = Some((self.id, self.repository.to_thread_local()));
            }
            f(&slot.as_ref().unwrap().1)
        })
    }

    fn entry(&self, path: &Path) -> io::Result<Entry> {
        let relative = path.strip_prefix(&self.root).map_err(error)?;
        let mut under = PathBuf::new();
        for component in relative.components() {
            match component {
                Component::Normal(name) => under.push(name),
                Component::CurDir => {}
                Component::ParentDir if under.pop() => {}
                _ => return Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            }
        }
        let full = self.prefix.join(under);
        if full.as_os_str().is_empty() {
            return Ok(Entry::Dir(self.tree));
        }
        let key = full
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        if let Some(files) = self.inventory.get().map(|i| &i.files)
            && let Ok(index) = files.binary_search_by(|file| file.path.as_str().cmp(&key))
        {
            let blob = files[index].blob;
            let width = if blob.is_git_sha256() { 32 } else { 20 };
            return Ok(Entry::File(gix::ObjectId::from_bytes_or_panic(
                &blob.key_bytes()[8..8 + width],
            )));
        }
        self.with_repo(|repo| {
            let tree = repo.find_tree(self.tree).map_err(error)?;
            let entry = tree
                .lookup_entry_by_path(full)
                .map_err(error)?
                .ok_or_else(missing)?;
            let id = entry.id().detach();
            if entry.mode().is_tree() {
                Ok(Entry::Dir(id))
            } else if entry.mode().is_blob() {
                Ok(Entry::File(id))
            } else {
                Err(missing())
            } // Never follow symlinks or enter submodules.
        })
    }
}

fn view(path: &Path) -> Option<Arc<View>> {
    if ACTIVE.load(Ordering::Acquire) == 0 {
        return None;
    }
    VIEWS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|v| path.starts_with(&v.root))
        .cloned()
}

pub fn contains(path: &Path) -> bool {
    view(path).is_some()
}

/// A scoped object-store view. Its stable root is an identity used by the engine's path-keyed stores.
/// No directory is created at that path, either when the view opens or while it is read.
#[derive(Debug)]
pub struct Snapshot {
    view: Arc<View>,
}

impl Snapshot {
    pub fn set_source_languages(&self, languages: SourceLanguages) {
        let _ = self.view.host_languages.set(languages);
    }

    pub fn host_language(&self, path: &str) -> Option<Option<String>> {
        self.view.host_languages.get()?;
        Some(self.view.inventory.get()?.languages.get(path).cloned())
    }

    pub fn restrict_paths(&self, paths: Vec<String>) {
        *self.view.paths.lock().unwrap_or_else(|err| err.into_inner()) = Some(paths);
    }

    pub fn begin_analysis(&self, store: &crate::content_store::ContentStore) -> crate::content_store::StoreResult<()> {
        if let Some(scope) = &self.view.scope && self.view.owner.get().is_none() {
            let owner = store.begin_analysis_scope(scope)?;
            let _ = self.view.owner.set(owner);
        }
        Ok(())
    }

    pub fn set_analysis_owner(&self, owner: crate::content_store::analysis_scope::AnalysisOwner) {
        let _ = self.view.owner.set(owner);
    }

    pub fn root(&self) -> &Path {
        &self.view.root
    }

    pub fn tree(&self) -> String {
        self.view.tree.to_string()
    }

    pub fn commit(&self) -> Option<String> {
        self.view.commit.map(|id| id.to_string())
    }

    pub fn workspace(self: &Arc<Self>, store: &Path) -> Result<super::GitWorkspace, String> {
        let inventory = self.view.inventory(Some(store))?;
        self.view.with_repo(|repo| {
            Ok(super::GitWorkspace {
                snapshot: Some(Arc::clone(self)),
                worktree_root: self.view.root.clone(),
                common_directory: repo.common_dir().to_path_buf(),
                hash_algorithm: super::hash_algorithm(repo.object_hash())
                    .map_err(|e| e.to_string())?,
                root_tree: super::hash_key(self.view.tree.as_ref()).map_err(|e| e.to_string())?,
                trees: inventory.trees.clone(),
                base_files: inventory.files.clone(),
                overlays: Vec::new(),
                nested_roots: Vec::new(),
            })
        })
    }

    pub fn read_source(&self, path: &str, limit: u64) -> io::Result<Vec<u8>> {
        self.with_blob(path, |size, read| {
            if size > limit {
                return Err(io::Error::from(io::ErrorKind::FileTooLarge));
            }
            read()
        })
    }

    pub fn owns_blob(&self, path: &str, length: u64) -> bool {
        self.with_blob(path, |size, _| Ok(size == length))
            .unwrap_or(false)
    }

    fn with_blob<T>(
        &self,
        path: &str,
        f: impl FnOnce(u64, &dyn Fn() -> io::Result<Vec<u8>>) -> io::Result<T>,
    ) -> io::Result<T> {
        super::check_source_path(path)?;
        let Entry::File(id) = self.view.entry(&self.view.root.join(path))? else {
            return Err(io::Error::from(io::ErrorKind::IsADirectory));
        };
        self.view.with_repo(|repo| {
            let header = repo.find_header(id).map_err(error)?;
            f(header.size(), &|| {
                Ok(repo.find_blob(id).map_err(error)?.data.clone())
            })
        })
    }

    pub fn new(source: &Path, revision: &str) -> Result<Self, String> {
        Self::open(source, revision, None)
    }

    pub fn scoped(source: &Path, revision: &str, name: &str) -> Result<Self, String> {
        let scope = crate::content_store::analysis_scope::named(name).ok_or("analysis scope name must not be empty")?;
        Self::open(source, revision, Some(scope))
    }

    fn open(source: &Path, revision: &str, scope: Option<String>) -> Result<Self, String> {
        let repo = gix::discover(source).map_err(|e| e.to_string())?;
        let (id, commit, tree) = {
            let object = repo
                .rev_parse_single(revision)
                .map_err(|e| format!("{revision}: {e}"))?
                .object()
                .and_then(|object| object.peel_tags_to_end())
                .map_err(|e| e.to_string())?;
            let id = object.id;
            match object.kind {
                gix::object::Kind::Commit => {
                    let tree = object.into_commit().tree_id().map_err(|e| e.to_string())?.detach();
                    (id, Some(id), tree)
                }
                gix::object::Kind::Tree => (id, None, id),
                _ => return Err("--revision requires a commit or tree".into()),
            }
        };
        let prefix = if let Some(work) = repo.workdir() {
            source
                .canonicalize()
                .map_err(|e| e.to_string())?
                .strip_prefix(work.canonicalize().map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
                .to_path_buf()
        } else {
            PathBuf::new()
        };
        if !prefix.as_os_str().is_empty() {
            repo.find_tree(tree).map_err(|e| e.to_string())?
                .lookup_entry_by_path(&prefix)
                .map_err(|e| e.to_string())?
                .ok_or("the revision lacks the run root")?
                .object()
                .map_err(|e| e.to_string())?
                .try_into_tree()
                .map_err(|e| e.to_string())?;
        }
        let git_dir = repo
            .git_dir()
            .canonicalize()
            .map_err(|e| e.to_string())?;
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let invocation = NEXT.fetch_add(1, Ordering::Relaxed);
        let mut root = git_dir.join("rerail-object-views");
        if let Some(scope) = &scope {
            use sha2::Digest as _;
            let hash: [u8; 32] = sha2::Sha256::digest(scope.as_bytes()).into();
            root.push(crate::content_store::observe::hex(&hash));
            root.push(invocation.to_string());
        }
        let root = root.join(id.to_string()).join(&prefix);
        let view = Arc::new(View {
            id: invocation,
            root,
            tree,
            commit,
            cache_root: git_dir.join("rerail-object-facts").join(&prefix),
            prefix,
            scope,
            owner: OnceLock::new(),
            inventory: OnceLock::new(),
            ignore_blob: OnceLock::new(),
            paths: Mutex::new(None),
            host_languages: OnceLock::new(),
            repository: repo.into_sync(),
        });
        let mut views = VIEWS.lock().unwrap_or_else(|e| e.into_inner());
        views.push(Arc::clone(&view));
        ACTIVE.store(views.len(), Ordering::Release);
        Ok(Self { view })
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        let mut views = VIEWS.lock().unwrap_or_else(|e| e.into_inner());
        views.retain(|v| v.id != self.view.id);
        ACTIVE.store(views.len(), Ordering::Release);
    }
}

/// The actual repository and repository-relative prefix behind a source root.
pub fn repository(
    root: &Path,
) -> Option<(
    gix::Repository,
    PathBuf,
    gix::ObjectId,
    Option<gix::ObjectId>,
)> {
    let v = view(root)?;
    let under = root.strip_prefix(&v.root).ok()?;
    Some((
        v.repository.to_thread_local(),
        v.prefix.join(under),
        v.tree,
        v.commit,
    ))
}

pub fn fact_root(path: &Path) -> Option<PathBuf> {
    let v = view(path)?;
    Some(v.cache_root.join(path.strip_prefix(&v.root).ok()?))
}

pub fn analysis_scope(path: &Path) -> Option<String> {
    view(path)?.scope.clone()
}

pub fn analysis_owner(path: &Path) -> Option<crate::content_store::analysis_scope::AnalysisOwner> {
    view(path)?.owner.get().cloned()
}

pub fn scan_files(root: &Path) -> Option<Result<Vec<(String, Option<String>)>, String>> {
    let v = view(root)?;
    let prefix = v.prefix.join(root.strip_prefix(&v.root).ok()?);
    Some(v.inventory(None).map(|inventory|
        inventory.files
            .iter()
            .filter_map(|file| {
                Path::new(&file.path)
                    .strip_prefix(&prefix)
                    .ok()
                    .map(|p| (p.to_string_lossy().into_owned(), inventory.languages.get(&file.path).cloned()))
            })
            .collect()
    ))
}

/// The source library's language result, once this view has run its inventory.
pub fn source_language(path: &Path) -> Option<Option<String>> {
    let v = view(path)?;
    let inventory = v.inventory.get()?;
    let full = v.prefix.join(path.strip_prefix(&v.root).ok()?);
    Some(inventory.languages.get(full.to_str()?).cloned())
}

pub struct InventoryFilter {
    prefix: PathBuf,
    paths: Option<Vec<String>>,
    rules: super::IgnoreRules,
}

impl InventoryFilter {
    pub fn child(&self, path: &str, name: &str, directory: bool) -> Option<String> {
        let path = if path.is_empty() { name.to_owned() } else { format!("{path}/{name}") };
        let relative = Path::new(&path);
        if !relative.starts_with(&self.prefix) {
            return (directory && self.prefix.starts_with(relative)).then_some(path);
        }
        if let Some(paths) = &self.paths {
            let under = relative.strip_prefix(&self.prefix).ok()?;
            if !paths.iter().any(|p| under.starts_with(p) || (directory && Path::new(p).starts_with(under))) {
                return None;
            }
        }
        if super::search_ignored_path(&self.rules.mandatory, &path, self.rules.case, directory)
            || (super::search_ignored_path(&self.rules.workspace, &path, self.rules.case, directory)
                && (!directory || !self.may_reinclude(&path)))
        {
            return None;
        }
        Some(path)
    }

    fn may_reinclude(&self, directory: &str) -> bool {
        use gix::ignore::glob::pattern::Mode;
        use gix::bstr::ByteSlice as _;
        use gix::ignore::glob::{wildmatch, wildmatch::Mode as Match};
        let below = format!("{directory}/");
        let mut later = std::collections::HashSet::new();
        self.rules.workspace.patterns.iter().rev().flat_map(|list| list.patterns.iter().rev()).any(|entry| {
            let pattern = &entry.pattern;
            if !later.insert((&pattern.text, pattern.mode - Mode::NEGATIVE)) || !pattern.is_negative() {
                return false;
            }
            if pattern.mode.contains(Mode::NO_SUB_DIR) && !pattern.mode.contains(Mode::ABSOLUTE) {
                return true;
            }
            // A descendant can match when the directory matches a prefix ending at a path separator.
            let matches = |prefix: &[u8]| wildmatch(prefix.as_bstr(), below.as_bytes().as_bstr(), Match::NO_MATCH_SLASH_LITERAL);
            pattern.text.iter().enumerate().any(|(at, &byte)| byte == b'/' && matches(&pattern.text[..=at]))
                || (pattern.text.ends_with(b"/**") && matches(&pattern.text))
        })
    }
}

pub fn inventory_filter(root: &Path) -> Result<InventoryFilter, String> {
    let v = view(root).ok_or("Git source view is closed")?;
    let bytes = match v.ignore_blob()? {
        Some(id) => v.with_repo(|repo| repo.find_blob(id).map(|blob| blob.data.clone()).map_err(|e| e.to_string()))?,
        None => Vec::new(),
    };
    let rules = super::ignore_patterns(Path::new("/"), gix::ignore::glob::pattern::Case::Sensitive, &bytes)
        .map_err(|err| err.to_string())?;
    let paths = v.paths.lock().unwrap_or_else(|err| err.into_inner()).clone();
    Ok(InventoryFilter { rules, paths, prefix: v.prefix.clone() })
}

pub fn inventory_selection(root: &Path) -> Option<String> {
    let v = view(root)?;
    let paths = v.paths.lock().unwrap_or_else(|err| err.into_inner());
    let ignores = match v.ignore_blob() {
        Ok(Some(id)) if v.with_repo(|repo| repo.find_header(id).is_ok()) => id.to_string(),
        Ok(Some(id)) => format!("unreadable:{id}"),
        Ok(None) => String::new(),
        Err(_) => format!("unreadable:{}", v.tree),
    };
    serde_json::to_string(&("ignore-v2", ignores, &v.prefix, &*paths)).ok()
}

pub fn source_languages(root: &Path) -> Option<SourceLanguages> {
    view(root)?.host_languages.get().cloned()
}

/// An immutable regular file's object identity. Directories, symlinks and gitlinks give no blob.
pub fn blob(path: &Path) -> Option<HashKey> {
    let v = view(path)?;
    let Entry::File(id) = v.entry(path).ok()? else {
        return None;
    };
    super::hash_key(id.as_ref()).ok()
}

pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let path = path.as_ref();
    let Some(v) = view(path) else {
        return std::fs::read(path);
    };
    let Entry::File(id) = v.entry(path)? else {
        return Err(io::Error::from(io::ErrorKind::IsADirectory));
    };
    v.with_repo(|repo| Ok(repo.find_blob(id).map_err(error)?.data.clone()))
}

/// Bounded read of a regular file in a registered Git source view.
pub fn read_bounded(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let v = view(path).ok_or_else(missing)?;
    let Entry::File(id) = v.entry(path)? else {
        return Err(io::Error::from(io::ErrorKind::IsADirectory));
    };
    v.with_repo(|repo| {
        if repo.find_header(id).map_err(error)?.size() > limit {
            return Err(io::Error::from(io::ErrorKind::FileTooLarge));
        }
        Ok(repo.find_blob(id).map_err(error)?.data.clone())
    })
}

pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    String::from_utf8(read(path)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    let path = path.as_ref();
    let Some(v) = view(path) else {
        return std::fs::canonicalize(path);
    };
    v.entry(path)?;
    // entry validates .. without allowing escape from this view.
    let mut out = v.root.clone();
    for part in path.strip_prefix(&v.root).map_err(error)?.components() {
        match part {
            Component::Normal(name) => out.push(name),
            Component::ParentDir => {
                out.pop();
            }
            _ => {}
        }
    }
    Ok(out)
}

pub fn is_file(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    view(path).map_or_else(
        || path.is_file(),
        |v| matches!(v.entry(path), Ok(Entry::File(_))),
    )
}

pub fn is_dir(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    view(path).map_or_else(
        || path.is_dir(),
        |v| matches!(v.entry(path), Ok(Entry::Dir(_))),
    )
}

pub fn listing(path: impl AsRef<Path>) -> io::Result<Vec<(String, bool)>> {
    let path = path.as_ref();
    let mut entries = if let Some(v) = view(path) {
        let Entry::Dir(id) = v.entry(path)? else {
            return Err(io::Error::from(io::ErrorKind::NotADirectory));
        };
        v.with_repo(|repo| {
            let tree = repo.find_tree(id).map_err(error)?;
            let mut entries = Vec::new();
            for entry in tree.iter() {
                let entry = entry.map_err(error)?;
                if entry.mode().is_tree() || entry.mode().is_blob() {
                    entries.push((
                        entry.filename().to_str().map_err(error)?.to_owned(),
                        entry.mode().is_tree(),
                    ));
                }
            }
            Ok::<_, io::Error>(entries)
        })?
    } else {
        std::fs::read_dir(path)?
            .map(|entry| {
                let entry = entry?;
                Ok((
                    entry.file_name().to_string_lossy().into_owned(),
                    entry.path().is_dir(),
                ))
            })
            .collect::<io::Result<Vec<_>>>()?
    };
    entries.sort();
    Ok(entries)
}

/// Filesystem stat indexes do not apply to immutable Git sources; their keys are blob IDs.
pub fn file_stat(path: impl AsRef<Path>) -> Option<crate::content_store::FileStat> {
    let path = path.as_ref();
    if contains(path) {
        None
    } else {
        std::fs::metadata(path)
            .ok()
            .map(|m| crate::content_store::FileStat::of(&m))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gix::objs::tree::{Entry as TreeEntry, EntryKind};

    #[test]
    fn bare_tree_sources_are_lazy_case_sensitive_and_never_materialized() {
        let dir = tempfile::tempdir().unwrap();
        let repo = gix::init_bare(dir.path().join("repo.git")).unwrap();
        let upper = repo.write_blob(b"UPPER\n").unwrap().detach();
        let lower = repo.write_blob(b"lower\n").unwrap().detach();
        // Deliberately absent: opening or listing this snapshot must not read file blobs.
        let absent = gix::ObjectId::from_hex(b"9999999999999999999999999999999999999999").unwrap();
        let tree = repo
            .write_object(&gix::objs::Tree {
                entries: vec![
                    TreeEntry {
                        mode: EntryKind::Blob.into(),
                        filename: "A.py".into(),
                        oid: upper,
                    },
                    TreeEntry {
                        mode: EntryKind::Blob.into(),
                        filename: "a.py".into(),
                        oid: lower,
                    },
                    TreeEntry {
                        mode: EntryKind::Link.into(),
                        filename: "link.py".into(),
                        oid: upper,
                    },
                    TreeEntry {
                        mode: EntryKind::Blob.into(),
                        filename: "missing.bin".into(),
                        oid: absent,
                    },
                    TreeEntry {
                        mode: EntryKind::Commit.into(),
                        filename: "submodule".into(),
                        oid: absent,
                    },
                ],
            })
            .unwrap()
            .detach();
        let snapshot = Snapshot::new(repo.git_dir(), &tree.to_string()).unwrap();
        let root = snapshot.root();
        assert!(
            !root.exists(),
            "the source view is never a directory on disk"
        );
        assert!(!repo.git_dir().join("rerail-object-views").exists());
        assert_eq!(read(root.join("A.py")).unwrap(), b"UPPER\n");
        assert_eq!(read(root.join("a.py")).unwrap(), b"lower\n");
        assert!(
            read(root.join("missing.bin")).is_err(),
            "the missing blob is read only when asked for"
        );
        assert!(!is_file(root.join("link.py")));
        assert!(!is_dir(root.join("submodule")));
        assert_eq!(
            listing(root).unwrap(),
            [
                ("A.py".into(), false),
                ("a.py".into(), false),
                ("missing.bin".into(), false)
            ]
        );
        assert_eq!(
            read(root.join("../config")).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        crate::content_store::observe::with(None, |observer| {
            let path = root.join("a.py");
            assert_eq!(
                observer.kind(&path),
                crate::content_store::observe::Kind::File
            );
            assert_eq!(observer.canonical(&path), Some(path.clone()));
            assert_eq!(observer.read(&path).unwrap().0, b"lower\n");
            assert_eq!(observer.digest(&path), Some(sha256(b"lower\n")));
            assert_eq!(observer.listing(root).unwrap().len(), 3);
        });
        assert!(
            !root.exists(),
            "even observer reads never extract a source file"
        );
    }

    fn sha256(bytes: &[u8]) -> [u8; 32] {
        use sha2::Digest;
        sha2::Sha256::digest(bytes).into()
    }

    #[test]
    fn identical_views_share_an_identity_and_have_independent_scopes() {
        let dir = tempfile::tempdir().unwrap();
        let repo = gix::init_bare(dir.path().join("repo.git")).unwrap();
        let blob = repo.write_blob(b"content").unwrap().detach();
        let tree = repo
            .write_object(&gix::objs::Tree {
                entries: vec![TreeEntry {
                    mode: EntryKind::Blob.into(),
                    filename: "file.py".into(),
                    oid: blob,
                }],
            })
            .unwrap()
            .detach();
        let one = Snapshot::new(repo.git_dir(), &tree.to_string()).unwrap();
        let two = Snapshot::new(repo.git_dir(), &tree.to_string()).unwrap();
        assert_eq!(one.root(), two.root());
        drop(one);
        assert_eq!(
            read_to_string(two.root().join("file.py")).unwrap(),
            "content"
        );
        let root = two.root().to_owned();
        drop(two);
        assert!(!contains(&root));
        assert!(!root.exists());
    }
}
