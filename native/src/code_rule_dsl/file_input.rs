//! A request's file as the engine reads it (`FileInput`), and the module a Python file is: Ruff's package walk, over
//! the files on disk or a file set's markers. The language packs read both (a pack names a file's module), so they
//! are below the model that holds compiled rules (`model` re-exports them).

use crate::code_rule_dsl::json::invalid;
use crate::code_rule_dsl::language::PackageType;
use crate::code_rule_dsl::language::adapter::pack as language_pack;
use crate::code_rule_dsl::module_path::ModulePath;
use napi::Result;
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone)]
pub struct FileInput {
    pub path: String,
    pub language: String,
    pub source: Arc<String>,
    /// A Python file's module path: the host's (`module`), else the file's stem.
    pub module: Option<Arc<ModulePath>>,
    /// An ECMA file's nearest `package.json` (`package`); `None` without one.
    pub package: Option<crate::code_rule_dsl::language::FileManifest>,
}

/// A request file as facts read it.
impl crate::code_rule_dsl::facts::core::SourceFile for FileInput {
    fn language(&self) -> &str {
        &self.language
    }
    fn source(&self) -> &Arc<String> {
        &self.source
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl FileInput {
    /// `module` is the module path a host derived for a Python file (Ruff's package walk): a
    /// dotted string, or a list of names when one keeps a dot (`["json.d", "__init__"]`). Without
    /// it a Python file is named by its stem, as Ruff names a file outside any package; a file set
    /// that holds package markers derives it first (`python_module_in`). `package` is the ECMA
    /// file's nearest `package.json` (`Manifest::from_wire`, `Manifests`); without it the file
    /// has none.
    pub fn new(
        path: String,
        language: String,
        source: Arc<String>,
        module: Option<&Value>,
        package: Option<crate::code_rule_dsl::language::FileManifest>,
        at: &str,
    ) -> Result<Self> {
        let module = match module {
            Some(module) => Some(module_path(module).ok_or_else(|| {
                invalid(format!(
                    "{at}/module must be a dotted module path (non-empty names joined by `.`) or a list of names"
                ))
            })?),
            None => crate::code_rule_dsl::language::dialect_by_label(&language).and_then(|dialect| {
                let pack = language_pack::pack(crate::code_rule_dsl::language::Family::of(dialect));
                pack.scope_unpackaged_module(&path)
            }),
        };
        Ok(Self {
            path,
            language,
            source,
            module: module.map(Arc::new),
            package,
        })
    }

    /// The `type` of the file's nearest `package.json`, which decides a `.js` file's module system.
    pub fn package_type(&self) -> Option<PackageType> {
        self.package
            .as_deref()
            .map(|package| package.package_type())
    }
}

fn module_path(value: &Value) -> Option<ModulePath> {
    match value {
        Value::String(module) => ModulePath::dotted(module),
        Value::Array(names) => ModulePath::names(
            &names
                .iter()
                .map(Value::as_str)
                .collect::<Option<Vec<_>>>()?,
        ),
        _ => None,
    }
}

/// Ruff's module path for a Python file (`detect_package_root`), in the shape hosts send as
/// `module`: the package root is the highest directory reached from the file's own by climbing
/// while each holds an `__init__.py`. The module is the names from that root's parent down, the
/// file's by its stem (`pkg/__init__.py` is `pkg.__init__`): dotted, or a list when a name keeps a
/// dot (`["json.d", "__init__"]`), since N999 reads such names whole. `None` outside a package,
/// where the engine names the module by the file's stem. Every ingestion path derives it here.
pub fn python_module_path(
    file: &std::path::Path,
    is_package: impl Fn(&std::path::Path) -> bool,
) -> Option<Value> {
    let mut root = None;
    let mut directory = file.parent()?;
    while is_package(directory) {
        root = Some(directory);
        match directory.parent() {
            Some(parent) if parent != directory => directory = parent,
            _ => break,
        }
    }
    let relative = file.strip_prefix(root?.parent()?).ok()?;
    let mut names = relative
        .components()
        .map(|component| component.as_os_str().to_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()?;
    let name = names.pop()?;
    let stem = match name.rfind('.') {
        Some(dot) if dot > 0 => &name[..dot],
        _ => name.as_str(),
    };
    names.push(stem.to_owned());
    if names.iter().any(String::is_empty) {
        return None;
    }
    Some(if names.iter().any(|name| name.contains('.')) {
        Value::Array(names.into_iter().map(Value::String).collect())
    } else {
        Value::String(names.join("."))
    })
}

/// `python_module_path` over the files on disk.
pub fn python_module_path_on_disk(file: &std::path::Path) -> Option<Value> {
    python_module_path(file, |directory| {
        let marker = directory.join("__init__.py");
        if crate::git_workspace::snapshot::contains(file) {
            crate::git_workspace::snapshot::is_file(marker)
        } else {
            marker.exists()
        }
    })
}

/// The directory a file set's path marks as a Python package (`pkg` for `pkg/__init__.py`, `""`
/// for a root `__init__.py`), if any.
pub fn package_marker_directory(path: &str) -> Option<&str> {
    path.strip_suffix("/__init__.py")
        .or_else(|| (path == "__init__.py").then_some(""))
}

/// `python_module_path` of a relative `path` in a file set (a workspace, a repository snapshot)
/// whose package markers `is_package` reads: the module a host walking the same files on disk
/// would send.
pub fn python_module_in(path: &str, is_package: impl Fn(&str) -> bool) -> Option<Value> {
    python_module_path(std::path::Path::new(path), |directory| {
        is_package(directory.to_string_lossy().as_ref())
    })
}
