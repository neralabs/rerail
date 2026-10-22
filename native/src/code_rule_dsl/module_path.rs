//! A Python file's module path: where it sits among modules, as the host names it from the
//! package markers on disk. File conditions, evidence projections and qualified-name resolution
//! read it.

/// Where a Python file sits among modules, as Ruff names it (`checkers::ast::check_ast`).
#[derive(Clone, Debug, PartialEq)]
pub struct ModulePath {
    /// The names from the package root's parent as on disk: each directory's, then the file's
    /// stem (`json.d`, `__init__`). Ruff's `to_module_path` reads a directory by its stem.
    segments: Vec<Box<str>>,
    /// A module path proper (Ruff's `ModuleSource::Path`, inside a package); otherwise only the
    /// file's stem is known (`ModuleSource::File`), and relative imports do not resolve.
    package: bool,
}

/// Names one module path may hold.
const MAX_MODULE_NAMES: usize = 256;

/// `Path::file_stem`: the name without its last extension; a leading dot is no extension.
fn file_stem(name: &str) -> &str {
    match name.rfind('.') {
        Some(0) | None => name,
        Some(dot) => &name[..dot],
    }
}

impl ModulePath {
    /// A dotted module path such as `pkg.sub.mod` (`pkg.__init__` for a package's `__init__.py`).
    pub(crate) fn dotted(module: &str) -> Option<Self> {
        Self::names(&module.split('.').collect::<Vec<_>>())
    }

    /// A module path as its names: each directory's from the package root down, as on disk
    /// (`json.d`), then the file's stem (`my.mod`, `__init__`). Names are non-empty path
    /// components, neither `.` nor `..`.
    pub(crate) fn names(names: &[&str]) -> Option<Self> {
        let valid = |name: &&str| {
            !name.is_empty() && !name.contains(['/', '\\']) && !matches!(*name, "." | "..")
        };
        (!names.is_empty() && names.len() <= MAX_MODULE_NAMES && names.iter().all(valid)).then(
            || Self {
                segments: names.iter().map(|name| Box::from(*name)).collect(),
                package: true,
            },
        )
    }

    /// A file outside any package: Ruff names its module by the file's stem, dots and all.
    pub(crate) fn stem(path: &str) -> Option<Self> {
        let stem = file_stem(path.rsplit(['/', '\\']).next()?);
        (!stem.is_empty()).then(|| Self {
            segments: vec![stem.into()],
            package: false,
        })
    }

    /// The names joined by `.` (`json.d.__init__`, `good.my.mod`, `pkg.sub.mod`).
    pub(crate) fn text(&self) -> String {
        self.segments.join(".")
    }

    /// The containing package's names, without this module's final segment.
    pub(crate) fn package_text(&self) -> String {
        self.segments[..self.segments.len().saturating_sub(1)].join(".")
    }

    /// The last name, the file's stem (`__init__` for a package's `__init__.py`).
    pub(crate) fn last(&self) -> &str {
        self.segments.last().map_or("", |segment| segment.as_ref())
    }

    /// The first name: the top package's directory, in a package.
    pub(crate) fn first(&self) -> &str {
        self.segments.first().map_or("", |segment| segment.as_ref())
    }

    /// Whether the host sent this path (a file inside a package), not the stem fallback.
    pub(crate) fn in_package(&self) -> bool {
        self.package
    }

    /// The first `count` segments of Ruff's `to_module_path` (directories by their stem), or
    /// `None` when one keeps a dot: Ruff compares qualified names segment by segment, so such a
    /// segment never equals a dotted name.
    fn qualified(&self, count: usize) -> Option<Vec<&str>> {
        let file = self.segments.len() - 1;
        self.segments[..count]
            .iter()
            .enumerate()
            .map(|(index, name)| {
                if index == file {
                    name.as_ref()
                } else {
                    file_stem(name)
                }
            })
            .map(|segment| (!segment.contains('.')).then_some(segment))
            .collect()
    }

    /// The qualified name of a definition bound as `head`, followed by `tail`.
    pub(crate) fn definition(&self, head: &str, tail: &[String]) -> Option<String> {
        let mut name = self.qualified(self.segments.len())?.join(".");
        for part in std::iter::once(head).chain(tail.iter().map(String::as_str)) {
            name.push('.');
            name.push_str(part);
        }
        Some(name)
    }

    /// The dotted name other modules import this one by: a package's `__init__` is the package
    /// itself (`pkg`, not `pkg.__init__`). `None` where a segment keeps a dot (see `qualified`).
    pub(crate) fn import_name(&self) -> Option<String> {
        let mut names = self.qualified(self.segments.len())?;
        if names.len() > 1 && names.last() == Some(&"__init__") {
            names.pop();
        }
        Some(names.join("."))
    }

    /// The qualified name `from <level dots><rest> import ...` denotes, followed by `tail`.
    pub(crate) fn relative(&self, level: usize, rest: &str, tail: &[String]) -> Option<String> {
        if !self.package || level > self.segments.len() {
            return None;
        }
        let base = self.qualified(self.segments.len() - level)?;
        let mut parts = base
            .into_iter()
            .chain(rest.split('.'))
            .chain(tail.iter().map(String::as_str));
        let first = parts.next()?;
        Some(parts.fold(first.to_owned(), |mut name, part| {
            name.push('.');
            name.push_str(part);
            name
        }))
    }
}
