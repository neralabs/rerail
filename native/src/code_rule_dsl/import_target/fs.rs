//! The files an import target's resolution reads: the ones a workspace or snapshot holds, and,
//! below the workspace's root on disk, the ones there. Paths are absolute: the root's own path
//! (`View::base`), or `/` for a file set without a root on disk.

use super::path;
use parking_lot::Mutex;
use super::json::Json as Parsed;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A file a resolution reads the text of is read only up to this size; a larger one is absent.
const MAX_TEXT_BYTES: u64 = 1024 * 1024;
/// A held `.json` file larger than this keeps no text: one a resolution reads is a manifest or a
/// project configuration, which `keeps_text` names whatever its size.
const MAX_HELD_JSON_BYTES: usize = 64 * 1024;
/// The absolute path a file set without a root on disk is read at: a folder below `/`, so a
/// description file at its root is found the way enhanced-resolve finds one in any folder.
const VIRTUAL_BASE: &str = "/workspace";
/// A view's caches are dropped whole beyond this many entries.
const MAX_CACHED: usize = 200_000;

/// Whether the text of a held file at `path` (workspace-relative) is kept: what a resolution
/// reads besides the files' existence (manifests, project configurations and their bases, the
/// ignore files and pnpm's workspace file).
fn keeps_text(path: &str, bytes: usize) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(
        name,
        "package.json" | ".npmignore" | ".gitignore" | "pnpm-workspace.yaml"
    ) || crate::code_rule_dsl::module_graph::is_module_config(path)
        || (name.ends_with(".json") && bytes <= MAX_HELD_JSON_BYTES)
}

/// The files a workspace or snapshot holds, by workspace-relative path, each with its text where
/// a resolution reads it (`keeps_text`).
#[derive(Clone, Default)]
pub(crate) struct HeldFiles {
    files: BTreeMap<String, Option<Arc<String>>>,
}

impl HeldFiles {
    /// Holds the file at `path` with `source`; whether that changes what a resolution reads.
    pub(crate) fn insert(&mut self, path: &str, source: &Arc<String>) -> bool {
        let text = keeps_text(path, source.len()).then(|| Arc::clone(source));
        match self.files.get(path) {
            Some(held) if held.as_deref() == text.as_deref() => false,
            _ => {
                self.files.insert(path.to_owned(), text);
                true
            }
        }
    }

    /// Stops holding the file at `path`; whether it was held.
    pub(crate) fn remove(&mut self, path: &str) -> bool {
        self.files.remove(path).is_some()
    }

    /// Whether the file at `path` is held, and its text where kept.
    pub(crate) fn get(&self, path: &str) -> Option<Option<&Arc<String>>> {
        self.files.get(path).map(Option::as_ref)
    }

    /// Whether some held file lies below `directory` (workspace-relative, `""` the root).
    fn holds_below(&self, directory: &str) -> bool {
        if directory.is_empty() {
            return !self.files.is_empty();
        }
        let prefix = format!("{directory}/");
        self.files
            .range(prefix.clone()..)
            .next()
            .is_some_and(|(path, _)| path.starts_with(&prefix))
    }

    /// The names directly in `directory` (workspace-relative) that held files give it.
    fn children(&self, directory: &str) -> BTreeSet<String> {
        let prefix = if directory.is_empty() {
            String::new()
        } else {
            format!("{directory}/")
        };
        self.files
            .range(prefix.clone()..)
            .take_while(|(path, _)| path.starts_with(&prefix))
            .filter_map(|(path, _)| path[prefix.len()..].split('/').next())
            .map(str::to_owned)
            .collect()
    }
}

/// What a path is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    File,
    Directory,
}

/// A parsed JSON file: its value, or the parse error enhanced-resolve reports.
pub(crate) type Json = Result<Arc<Parsed>, String>;

#[derive(Default)]
struct Cache {
    kinds: HashMap<String, Option<Kind>>,
    texts: HashMap<String, Option<Arc<String>>>,
    json: HashMap<String, Option<Json>>,
    listings: HashMap<String, Arc<Vec<String>>>,
    real: HashMap<String, String>,
}

impl Cache {
    fn bound(&mut self) {
        let size = self.kinds.len()
            + self.texts.len()
            + self.json.len()
            + self.listings.len()
            + self.real.len();
        if size > MAX_CACHED {
            *self = Self::default();
        }
    }
}

/// The files one evaluation's resolutions read: the held ones over the ones on disk below
/// `root`, read once per view. The view is the workspace's at one apply (or one snapshot's), so
/// it reads disk as it was then.
pub(crate) struct View {
    held: Arc<HeldFiles>,
    /// The root on disk the held paths are relative to, if any; nothing above it is read.
    root: Option<PathBuf>,
    /// `root` with its symlinks resolved: a path on disk is read only where its own resolved path
    /// lies below it.
    canonical_root: Option<PathBuf>,
    /// The absolute path of the root: `root` as given, or `VIRTUAL_BASE` without one.
    base: String,
    cache: Mutex<Cache>,
    /// Whether each folder (workspace-relative) resolves inside the root (`directory_inside`).
    inside: Mutex<HashMap<String, bool>>,
    /// What the view's resolutions read once (`import_target::Memo`).
    pub(super) memo: super::Memo,
}

impl View {
    pub(crate) fn new(held: Arc<HeldFiles>, root: Option<&Path>) -> Self {
        let root = root
            .map(Path::to_path_buf)
            .filter(|root| root.to_str().is_some_and(|root| path::normalize(root) != "/"));
        let base = root
            .as_ref()
            .and_then(|root| root.to_str())
            .filter(|root| root.starts_with('/'))
            .map(|root| path::normalize(root).trim_end_matches('/').to_owned())
            .unwrap_or_else(|| VIRTUAL_BASE.to_owned());
        let root = root.filter(|_| base != VIRTUAL_BASE);
        Self {
            canonical_root: root.as_ref().and_then(|root| root.canonicalize().ok()),
            held,
            root,
            base,
            cache: Mutex::new(Cache::default()),
            inside: Mutex::new(HashMap::new()),
            memo: super::Memo::default(),
        }
    }

    /// The absolute path of the root.
    pub(crate) fn base(&self) -> &str {
        &self.base
    }

    /// The absolute path of the workspace-relative `relative`.
    pub(crate) fn absolute(&self, relative: &str) -> String {
        path::join(&self.base, relative)
    }

    /// The workspace-relative path of `absolute`, if it lies at or below the root.
    pub(crate) fn relative<'p>(&self, absolute: &'p str) -> Option<&'p str> {
        if absolute == self.base {
            return Some("");
        }
        absolute
            .strip_prefix(self.base.as_str())
            .and_then(|rest| rest.strip_prefix('/'))
    }

    /// The path on disk of `relative`, where it lies inside the root's resolved folder: its
    /// folder's resolved path does (`directory_inside`), and so does its own where it is a
    /// symlink.
    fn on_disk(&self, relative: &str) -> Option<PathBuf> {
        let root = self.root.as_ref()?;
        let canonical_root = self.canonical_root.as_ref()?;
        if relative.is_empty() {
            return Some(root.clone());
        }
        let parent = relative.rfind('/').map_or("", |at| &relative[..at]);
        if !self.directory_inside(parent) {
            return None;
        }
        let path = root.join(relative);
        let metadata = std::fs::symlink_metadata(&path).ok()?;
        if metadata.file_type().is_symlink() {
            let canonical = path.canonicalize().ok()?;
            if !canonical.starts_with(canonical_root) {
                return None;
            }
        }
        Some(path)
    }

    /// Whether the folder at `relative` resolves to a path inside the root's resolved folder,
    /// read once per folder.
    fn directory_inside(&self, relative: &str) -> bool {
        if relative.is_empty() {
            return true;
        }
        if let Some(inside) = self.inside.lock().get(relative) {
            return *inside;
        }
        let inside = self
            .root
            .as_ref()
            .zip(self.canonical_root.as_ref())
            .is_some_and(|(root, canonical_root)| {
                root.join(relative)
                    .canonicalize()
                    .is_ok_and(|canonical| canonical.starts_with(canonical_root))
            });
        let mut cache = self.inside.lock();
        if cache.len() > MAX_CACHED {
            cache.clear();
        }
        cache.insert(relative.to_owned(), inside);
        inside
    }

    /// What is at `absolute`: a held file, a folder holding one, or what disk has there.
    pub(crate) fn kind(&self, absolute: &str) -> Option<Kind> {
        let relative = self.relative(absolute)?;
        if self.held.get(relative).is_some() {
            return Some(Kind::File);
        }
        if self.held.holds_below(relative) || (relative.is_empty() && self.root.is_none()) {
            return Some(Kind::Directory);
        }
        if let Some(kind) = self.cache.lock().kinds.get(absolute) {
            return *kind;
        }
        let kind = self.on_disk(relative).and_then(|path| {
            let metadata = std::fs::metadata(path).ok()?;
            if metadata.is_file() {
                Some(Kind::File)
            } else if metadata.is_dir() {
                Some(Kind::Directory)
            } else {
                None
            }
        });
        let mut cache = self.cache.lock();
        cache.bound();
        cache.kinds.insert(absolute.to_owned(), kind);
        kind
    }

    pub(crate) fn is_file(&self, absolute: &str) -> bool {
        self.kind(absolute) == Some(Kind::File)
    }

    pub(crate) fn is_directory(&self, absolute: &str) -> bool {
        self.kind(absolute) == Some(Kind::Directory)
    }

    /// The text of the file at `absolute`: a held one's, else disk's (at most 1 MiB, UTF-8).
    pub(crate) fn text(&self, absolute: &str) -> Option<Arc<String>> {
        let relative = self.relative(absolute)?;
        if let Some(Some(text)) = self.held.get(relative) {
            return Some(Arc::clone(text));
        }
        if let Some(text) = self.cache.lock().texts.get(absolute) {
            return text.clone();
        }
        let text = self
            .on_disk(relative)
            .filter(|path| {
                std::fs::metadata(path)
                    .is_ok_and(|metadata| metadata.is_file() && metadata.len() <= MAX_TEXT_BYTES)
            })
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .map(Arc::new);
        let mut cache = self.cache.lock();
        cache.bound();
        cache.texts.insert(absolute.to_owned(), text.clone());
        text
    }

    /// The JSON file at `absolute`, read as `JSON.parse` reads it (a leading BOM aside): `None`
    /// where there is no file, an error where its text is no JSON.
    pub(crate) fn json(&self, absolute: &str) -> Option<Json> {
        if let Some(json) = self.cache.lock().json.get(absolute) {
            return json.clone();
        }
        let json = self
            .text(absolute)
            .map(|text| Parsed::parse(&text).map(Arc::new));
        let mut cache = self.cache.lock();
        cache.bound();
        cache.json.insert(absolute.to_owned(), json.clone());
        json
    }

    /// The names in the folder at `absolute`: held files' and disk's.
    pub(crate) fn names(&self, absolute: &str) -> Arc<Vec<String>> {
        let Some(relative) = self.relative(absolute) else {
            return Arc::default();
        };
        if let Some(names) = self.cache.lock().listings.get(absolute) {
            return Arc::clone(names);
        }
        let mut names = self.held.children(relative);
        if let Some(entries) = self
            .on_disk(relative)
            .and_then(|path| std::fs::read_dir(path).ok())
        {
            names.extend(
                entries
                    .flatten()
                    .filter_map(|entry| entry.file_name().into_string().ok()),
            );
        }
        let names = Arc::new(names.into_iter().collect::<Vec<_>>());
        let mut cache = self.cache.lock();
        cache.bound();
        cache
            .listings
            .insert(absolute.to_owned(), Arc::clone(&names));
        names
    }

    /// `absolute` with its symlinks resolved on disk (enhanced-resolve's `symlinks: true`): a held
    /// file is its own path; one that leads outside the root keeps the path it names.
    pub(crate) fn real_path(&self, absolute: &str) -> String {
        let Some(relative) = self.relative(absolute) else {
            return absolute.to_owned();
        };
        if self.held.get(relative).is_some() {
            return absolute.to_owned();
        }
        let mut cache = self.cache.lock();
        if let Some(real) = cache.real.get(absolute) {
            return real.clone();
        }
        let real = self
            .root
            .as_ref()
            .zip(self.canonical_root.as_ref())
            .and_then(|(root, canonical_root)| {
                let canonical = root.join(relative).canonicalize().ok()?;
                let inside = canonical.strip_prefix(canonical_root).ok()?;
                let inside = inside.to_str()?;
                Some(self.absolute(inside))
            })
            .unwrap_or_else(|| absolute.to_owned());
        cache.bound();
        cache.real.insert(absolute.to_owned(), real.clone());
        real
    }
}
