//! Where the preprocessor's files come from: a header name resolved to a path, and a path read.
//!
//! In the engine, resolution is rules (`c/includes.rr`) and reading is the engine's; `Search` is
//! the search order a compiler driver has (quote, `-iquote`, `-I`, `-isystem`, `-idirafter`), over
//! the file system or a map of texts, for tests and the `clang -E` oracle.

/// The preprocessor's files.
pub trait Files {
    /// The file a header name `name` (its quotes or brackets taken off) names: `angle` for `<…>`;
    /// `includer` is the including file's path; `next_after` is, for `#include_next`, the search
    /// entry the includer was found by. Its path, and the search entry it was found by (`None`:
    /// the includer's directory, or an absolute name).
    fn find(&mut self, name: &[u8], angle: bool, includer: &str, next_after: Option<Option<u32>>) -> Option<(String, Option<u32>)>;

    /// The bytes of `path`.
    fn read(&mut self, path: &str) -> Option<Vec<u8>>;
}

/// A search path entry's kind, in a driver's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DirKind {
    /// `-iquote`: for `"…"` only.
    Quote,
    /// `-I`.
    Angle,
    /// `-isystem` (and the compiler's own).
    System,
    /// A framework directory (`-F`, and the compiler's own): `<A/b.h>` is `A.framework/Headers/b.h`
    /// there.
    Framework,
    /// `-idirafter`.
    After,
}

/// The search order over `read`'s texts.
pub struct Search<R: FnMut(&str) -> Option<Vec<u8>>> {
    /// The entries, in their kinds' order and then as given.
    pub dirs: Vec<(String, DirKind)>,
    pub read: R,
}

/// The directory of `path` (`""`: the current one).
fn dir_of(path: &str) -> &str {
    path.rfind('/').map_or("", |k| &path[..k])
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() { name.to_string() } else { format!("{dir}/{name}") }
}

impl<R: FnMut(&str) -> Option<Vec<u8>>> Search<R> {
    pub fn new(mut dirs: Vec<(String, DirKind)>, read: R) -> Self {
        dirs.sort_by_key(|d| d.1);
        Search { dirs, read }
    }

    fn exists(&mut self, path: &str) -> bool {
        (self.read)(path).is_some()
    }
}

impl<R: FnMut(&str) -> Option<Vec<u8>>> Files for Search<R> {
    fn find(&mut self, name: &[u8], angle: bool, includer: &str, next_after: Option<Option<u32>>) -> Option<(String, Option<u32>)> {
        let name = std::str::from_utf8(name).ok()?;
        if name.starts_with('/') {
            return self.exists(name).then(|| (name.to_string(), None));
        }
        // `#include_next` searches from the entry after the includer's; an includer found by its
        // own directory (or the main file) searches from the first.
        let from = match next_after {
            Some(Some(k)) => k as usize + 1,
            _ => 0,
        };
        if !angle && next_after.is_none() {
            let p = join(dir_of(includer), name);
            if self.exists(&p) {
                return Some((p, None));
            }
        }
        for k in from..self.dirs.len() {
            if angle && self.dirs[k].1 == DirKind::Quote {
                continue;
            }
            let p = if self.dirs[k].1 == DirKind::Framework {
                let Some((fw, rest)) = name.split_once('/') else { continue };
                format!("{}/{fw}.framework/Headers/{rest}", self.dirs[k].0)
            } else {
                join(&self.dirs[k].0, name)
            };
            if self.exists(&p) {
                return Some((p, Some(k as u32)));
            }
        }
        None
    }

    fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        (self.read)(path)
    }
}
