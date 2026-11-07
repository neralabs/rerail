//! The dependency universe (03 §13 "Dependency universe"; I18; list step 29).
//!
//! Installed packages, stubs and typeshed hold orders of magnitude more files than a repository. They produce
//! **interface summaries only**: no findings and no tapes.
//!
//! - **Identity:** a module file is named by (package, version, path, digest). The digest is the file's own, as
//!   the package's `RECORD` lists it (`path,sha256=<urlsafe base64>,size`). The version is the `.dist-info`
//!   directory's, checked against the lockfile's where there is one (`lock_versions`: `uv.lock` and
//!   `poetry.lock`). Nothing is ever executed.
//! - **Cache:** summaries are content-addressed by that identity (`Identity::key`), in one machine-wide
//!   directory (`Cache`). Two environments with the same file share its summary.
//! - **Demand:** summaries are read for the modules the repository imports and for what those re-export. This
//!   is the type program's magic-set relation:
//!
//!   ```text
//!   demand module(M2) :- module(M), reexport(M, _, M2).
//!   ```
//!
//!   evaluated here as a worklist (`Universe::demand`), with a budget of modules per package.
//! - **States:** a module is `Ready` (its summary), `Pending` (it is installed but has no summary yet: one is
//!   requested) or `Top` (nothing can be known: not installed, or past its package's budget). "Pending" is not
//!   ⊤ (03 §13): a check through a pending module waits for its summary instead of reporting unknown.
//!
//! A summary is a `rerail.types/v1` file (`typing::declare`), with `from M import x` lines for its re-exports.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::declare::{self, Declarations};
use super::terms::Terms;
use crate::content_store::atoms::Sym;

/// A module file's identity: what its summary is cached under.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct Identity {
    pub(crate) package: String,
    pub(crate) version: String,
    /// The file, relative to the site directory (`RECORD`'s path).
    pub(crate) path: String,
    /// `RECORD`'s digest of the file (`sha256=…`).
    pub(crate) digest: String,
}

impl Identity {
    /// The content address: a SHA-256 of the identity, in hex.
    pub(crate) fn key(&self) -> String {
        let mut h = Sha256::new();
        for part in [&self.package, &self.version, &self.path, &self.digest] {
            h.update(part.as_bytes());
            h.update([0]);
        }
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// `RECORD`'s rows: (path, digest), for rows with a digest (the `RECORD` row itself has none).
pub(crate) fn read_record(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let mut cols = l.rsplitn(3, ',');
            let (_size, digest, path) = (cols.next()?, cols.next()?, cols.next()?);
            (!digest.is_empty()).then(|| (path.to_owned(), digest.to_owned()))
        })
        .collect()
}

/// A `.dist-info` directory's (package, version): `name-version.dist-info`, the name normalized (PEP 503).
pub(crate) fn dist_info_name(dir: &str) -> Option<(String, String)> {
    let stem = dir.strip_suffix(".dist-info")?;
    let (name, version) = stem.rsplit_once('-')?;
    Some((normalize(name), version.to_owned()))
}

fn normalize(name: &str) -> String {
    name.to_lowercase().replace(['_', '.'], "-")
}

/// A lockfile's pinned versions (`uv.lock`, `poetry.lock`: `[[package]]` tables with `name` and `version`).
pub(crate) fn lock_versions(text: &str) -> BTreeMap<String, String> {
    let (mut out, mut name) = (BTreeMap::new(), None::<String>);
    let value = |l: &str, key: &str| -> Option<String> {
        let rest = l.strip_prefix(key)?.trim_start().strip_prefix('=')?.trim();
        Some(rest.trim_matches('"').to_owned())
    };
    for l in text.lines().map(str::trim) {
        if l == "[[package]]" {
            name = None;
        } else if let Some(n) = value(l, "name") {
            name = Some(normalize(&n));
        } else if let (Some(v), Some(n)) = (value(l, "version"), name.take()) {
            out.insert(n, v);
        }
    }
    out
}

/// What is installed in one site directory: each module's file and identity.
#[derive(Debug, Default)]
pub(crate) struct Installed {
    pub(crate) root: PathBuf,
    /// Module name (`a.b`) → identity. A stub (`.pyi`) is preferred to its source.
    pub(crate) modules: BTreeMap<String, Identity>,
    /// Lockfile versions that differ from the installed ones: (package, locked, installed).
    pub(crate) mismatches: Vec<(String, String, String)>,
}

impl Installed {
    /// Reads every `*.dist-info/RECORD` under `site` (no code runs), checked against `lock` if given.
    pub(crate) fn scan(site: &Path, lock: Option<&BTreeMap<String, String>>) -> std::io::Result<Self> {
        let mut out = Installed { root: site.to_owned(), ..Default::default() };
        let mut entries: Vec<_> = std::fs::read_dir(site)?.filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        entries.sort();
        for dir in entries {
            let Some((package, version)) = dist_info_name(&dir) else { continue };
            if let Some(locked) = lock.and_then(|l| l.get(&package))
                && *locked != version
            {
                out.mismatches.push((package.clone(), locked.clone(), version.clone()));
            }
            let Ok(record) = std::fs::read_to_string(site.join(&dir).join("RECORD")) else { continue };
            for (path, digest) in read_record(&record) {
                let Some(module) = module_of(&path) else { continue };
                let id = Identity { package: package.clone(), version: version.clone(), path: path.clone(), digest };
                let stub = path.ends_with(".pyi");
                match out.modules.get(&module) {
                    Some(have) if have.path.ends_with(".pyi") && !stub => {}
                    _ => {
                        out.modules.insert(module, id);
                    }
                }
            }
        }
        Ok(out)
    }
}

/// A file's module name: `a/b.py` → `a.b`, `a/__init__.pyi` → `a`.
fn module_of(path: &str) -> Option<String> {
    let stem = path.strip_suffix(".pyi").or_else(|| path.strip_suffix(".py"))?;
    let stem = stem.strip_suffix("/__init__").unwrap_or(stem);
    (!stem.contains(".dist-info") && !stem.starts_with("..")).then(|| stem.replace('/', "."))
}

/// Summaries by content address, machine-wide (`~/.cache/rerail/summaries` by default).
pub(crate) struct Cache {
    pub(crate) dir: PathBuf,
}

impl Cache {
    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(&key[..2]).join(format!("{key}.pyi"))
    }
    pub(crate) fn get(&self, id: &Identity) -> Option<String> {
        std::fs::read_to_string(self.path(&id.key())).ok()
    }
    pub(crate) fn put(&self, id: &Identity, summary: &str) -> std::io::Result<()> {
        let p = self.path(&id.key());
        std::fs::create_dir_all(p.parent().expect("a cache path has a parent"))?;
        // Written whole, then renamed: a reader never sees half a summary.
        let tmp = p.with_extension(format!("tmp{}", std::process::id()));
        std::fs::write(&tmp, summary)?;
        std::fs::rename(tmp, p)
    }
}

/// A module's state in the universe.
#[derive(Debug)]
pub(crate) enum State {
    Ready(Declarations),
    /// Installed, no summary yet: requested (`Universe::requested`). Not ⊤.
    Pending,
    /// Nothing can be known: not installed, unreadable, or past its package's budget.
    Top,
}

/// The re-exports a summary names: `from M2 import x`. A relative import is resolved against `module`'s package:
/// the module itself when it is one (`__init__`), else its parent.
pub(crate) fn reexports(module: &str, package: bool, summary: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for l in summary.lines().map(str::trim) {
        let Some(rest) = l.strip_prefix("from ") else { continue };
        let Some((from, names)) = rest.split_once(" import ") else { continue };
        let dots = from.len() - from.trim_start_matches('.').len();
        let target = if dots == 0 {
            from.to_owned()
        } else {
            let parts: Vec<&str> = module.split('.').collect();
            let keep = (parts.len() + usize::from(package)).saturating_sub(dots);
            let base = parts[..keep].join(".");
            let tail = &from[dots..];
            match (base.is_empty(), tail.is_empty()) {
                (true, _) => tail.to_owned(),
                (false, true) => base,
                (false, false) => format!("{base}.{tail}"),
            }
        };
        for n in names.split(',') {
            let n = n.trim().split(" as ").next().unwrap_or("").trim().trim_matches(['(', ')']);
            if !n.is_empty() {
                out.push((n.to_owned(), target.clone()));
            }
        }
    }
    out
}

/// The universe as one run demands it.
#[derive(Debug, Default)]
pub(crate) struct Universe {
    pub(crate) modules: BTreeMap<String, State>,
    /// Pending modules whose summaries were requested, by identity.
    pub(crate) requested: BTreeSet<Identity>,
    /// `reexport(M, x, M2)` rows met.
    pub(crate) reexports: BTreeSet<(String, String, String)>,
}

impl Universe {
    /// `demand module(M2) :- module(M), reexport(M, _, M2).` from the repository's imports `roots`: each
    /// demanded module's summary is read from `cache`, its re-exports demanded in turn, at most `budget`
    /// modules per package.
    pub(crate) fn demand(roots: &[&str], installed: &Installed, cache: &Cache, budget: usize, terms: &mut Terms, intern: &mut dyn FnMut(&str) -> Sym) -> Self {
        let mut out = Universe::default();
        let mut per_package: BTreeMap<String, usize> = BTreeMap::new();
        let mut queue: VecDeque<String> = roots.iter().map(|r| (*r).to_owned()).collect();
        while let Some(m) = queue.pop_front() {
            if out.modules.contains_key(&m) {
                continue;
            }
            let Some(id) = installed.modules.get(&m) else {
                out.modules.insert(m, State::Top);
                continue;
            };
            let used = per_package.entry(id.package.clone()).or_default();
            if *used >= budget {
                out.modules.insert(m, State::Top);
                continue;
            }
            *used += 1;
            let Some(text) = cache.get(id) else {
                out.requested.insert(id.clone());
                out.modules.insert(m, State::Pending);
                continue;
            };
            let package = id.path.ends_with("/__init__.py") || id.path.ends_with("/__init__.pyi");
            for (name, target) in reexports(&m, package, &text) {
                out.reexports.insert((m.clone(), name, target.clone()));
                queue.push_back(target);
            }
            let state = match declare::parse(&text, terms, intern) {
                Ok(d) => State::Ready(d),
                Err(_) => State::Top,
            };
            out.modules.insert(m, state);
        }
        out
    }
}

/// A Python source's absolute imports, from its import statements (syntax only): `import a.b, c as d` gives
/// (`a.b`, []) and (`c`, []); `from a.b import x, y` gives (`a.b`, [x, y]). Relative imports (the corpus's own) are
/// kept only if `relative`.
pub(crate) fn imports(text: &str, relative: bool) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    for l in text.lines() {
        let l = l.trim();
        if let Some(rest) = l.strip_prefix("import ") {
            for m in rest.split(',') {
                let m = m.trim().split(" as ").next().unwrap_or("").trim();
                if !m.is_empty() {
                    out.push((m.to_owned(), Vec::new()));
                }
            }
        } else if let Some(rest) = l.strip_prefix("from ")
            && let Some((m, names)) = rest.split_once(" import ")
            && (relative || !m.starts_with('.'))
        {
            let names = names.trim().trim_matches(['(', ')']).split(',').map(|n| n.trim().split(" as ").next().unwrap_or("").trim().to_owned()).filter(|n| !n.is_empty() && n != "*");
            out.push((m.trim().to_owned(), names.collect()));
        }
    }
    out
}

/// The modules a corpus's imports demand: each imported module, and `m.x` for each `from m import x` (a
/// submodule, when one is installed).
pub(crate) fn roots(imports: &[(String, Vec<String>)]) -> Vec<String> {
    let mut out = BTreeSet::new();
    for (m, names) in imports {
        out.insert(m.clone());
        for n in names {
            out.insert(format!("{m}.{n}"));
        }
    }
    out.into_iter().collect()
}

/// A module's summary: its functions' return types (the member names the program derived; none is `Any`) and the
/// `from` imports it re-exports, as a `rerail.types/v1` file.
pub(crate) fn summary(module: &str, functions: &BTreeMap<String, BTreeSet<String>>, reexported: &[(String, Vec<String>)]) -> String {
    let mut out = format!("{}\n# module: {module}\n", declare::HEADER);
    for (m, names) in reexported {
        if !names.is_empty() {
            out.push_str(&format!("from {m} import {}\n", names.join(", ")));
        }
    }
    for (name, ret) in functions {
        let ret = if ret.is_empty() { "Any".to_owned() } else { ret.iter().cloned().collect::<Vec<_>>().join(" | ") };
        out.push_str(&format!("def {name}(*args, **kwargs) -> {ret}: ...\n"));
    }
    out
}

#[cfg(test)]
mod tests;
