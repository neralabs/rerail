mod blade;
mod budget;
mod calls;
mod compile;
mod demand;
mod embed;
mod eval;
mod facts;
mod identifier;
mod json;
mod language;
mod lines;
mod literal;
mod logic;
mod matcher;
mod model;
mod module_graph;
mod module_path;
mod origin;
mod package_manifest;
mod php_literal;
mod php_view;
mod placement;
// Published from the scan in a later slice; until then only tests call it.
#[allow(dead_code)]
pub(crate) mod prefilter;
// Read by the prefilter's lowering; the compiled-plan cache is wired with the plans row.
#[allow(dead_code)]
mod stats;
#[cfg(feature = "code-rule-profile")]
mod profile;
mod query;
mod scope;
mod semver;
mod soundness;
mod suppress;
mod tape;
#[cfg(test)]
mod test_support;
mod transducer;
mod value;
mod violations;
mod vue;
// Wired to the logic planner and the push path in a later slice; until then only tests call it.
#[allow(dead_code)]
mod waves;
mod workspace;

/// The conformance fixtures directory `native/build.rs` found: `RERAIL_ORACLES_DIR`, or the
/// repository's `fixtures/`.
#[cfg(test)]
macro_rules! fixtures_dir {
    () => {
        env!(
            "RERAIL_ORACLES_DIR",
            "Rerail fixtures not found: check out the repository's fixtures/ directory, or set RERAIL_ORACLES_DIR"
        )
    };
}

#[cfg(test)]
pub(crate) use fixtures_dir;

#[cfg(test)]
macro_rules! oracle {
    ($path:literal) => {
        include_str!(concat!(crate::code_rule_dsl::fixtures_dir!(), "/", $path))
    };
}

#[cfg(test)]
pub(crate) use oracle;

use compile::{compile_document, parse_capabilities, parse_document};
use eval::{DerivedPackages, evaluate_code_rule, evaluation_json, parse_snapshot, violation_ids};
use json::{canonicalize_value, hex_digest, invalid, json_error, required_str, required_u64};
use model::{CompiledCodeRule, Snapshot};
use napi::Result;
use napi_derive::napi;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::Arc;

#[napi]
pub struct CodeRuleDslRuntime;

#[napi]
impl CodeRuleDslRuntime {
    #[napi]
    pub fn compile(
        source: String,
        format: String,
        capabilities_json: String,
    ) -> Result<CompiledCodeRuleDsl> {
        let capabilities = parse_capabilities(&capabilities_json)?;
        let document = parse_document(&source, &format)?;
        let canonical_value = canonicalize_value(document);
        let canonical = serde_json::to_string(&canonical_value).map_err(json_error)?;
        let digest = format!("sha256:{}", hex_digest(canonical.as_bytes()));
        let (id, rule_mode, scope, clauses, required) =
            compile_document(&canonical_value, &capabilities)?;
        Ok(CompiledCodeRuleDsl {
            inner: Arc::new(CompiledCodeRule {
                id,
                rule_mode,
                canonical,
                digest,
                required_capabilities: serde_json::to_string(&required).map_err(json_error)?,
                clauses,
                scope,
                capabilities,
            }),
        })
    }
}

/// The module a file's `module` carries, from the package markers on disk, for the file at
/// `absolute` whose dialect's label is `label`: a dotted string, a list of names when one keeps
/// a dot, or null outside a package and for a family without modules (`Adapter::module_path`).
/// Hosts and the native repository ingestion paths share this one derivation.
#[napi]
pub fn module_path(label: String, absolute: String) -> Option<napi::Either<String, Vec<String>>> {
    match language::adapter::module_path(&label, std::path::Path::new(&absolute))? {
        Value::String(module) => Some(napi::Either::A(module)),
        Value::Array(names) => Some(napi::Either::B(
            names
                .into_iter()
                .filter_map(|name| name.as_str().map(str::to_owned))
                .collect(),
        )),
        _ => None,
    }
}

/// Whether files of the dialect labelled `label` carry a `package` (`Adapter::PACKAGE_MANIFESTS`):
/// hosts resend such a file when a `package.json` edit changes its nearest manifest.
#[napi]
pub fn reads_package_manifests(label: String) -> bool {
    language::adapter::package_manifests(&label)
}

/// A file's `package` fact: what the engine reads of its nearest `package.json`.
#[napi(object)]
pub struct PackageFacts {
    /// The manifest's directory, relative to the workspace root (`""` at the root).
    pub folder: String,
    /// `"module"`, `"commonjs"`, or `"unset"` for a manifest without a valid `type`.
    #[napi(js_name = "type")]
    pub package_type: String,
    /// The `bin` targets, relative to `folder`.
    pub bin: Vec<String>,
    /// The configured Node range (`engines.node`, else `devEngines.runtime`); none without one.
    pub node_version: Option<String>,
    /// Every dependency name, of any kind.
    pub dependencies: Vec<String>,
    /// The package's `tsconfig.json` (else `jsconfig.json`) `compilerOptions.jsxImportSource`.
    pub jsx_import_source: Option<String>,
}

/// The `package` a file carries, for the file at `absolute` whose dialect's label is `label`: its
/// nearest `package.json` at or above it, never above `root` (the workspace's), or null without
/// one and for a family that reads none (`Adapter::PACKAGE_MANIFESTS`). Hosts and the native
/// repository ingestion paths share this one derivation.
#[napi]
pub fn package_facts(label: String, absolute: String, root: String) -> Option<PackageFacts> {
    if !language::adapter::package_manifests(&label) {
        return None;
    }
    nearest_package(absolute, root)
}

/// The nearest `package.json` at or above `absolute` (a file or a folder's `package.json`),
/// never above `root`, whatever reads it: the package an edit of any file belongs to, so a host
/// can tell which applied sources it may change. Null without one.
#[napi]
pub fn nearest_package(absolute: String, root: String) -> Option<PackageFacts> {
    let manifest = package_manifest::manifest_of_file(
        std::path::Path::new(&absolute),
        std::path::Path::new(&root),
    )?;
    Some(PackageFacts {
        folder: manifest.folder().to_owned(),
        package_type: manifest.package_type().as_str().to_owned(),
        bin: manifest.bin().map(str::to_owned).collect(),
        node_version: manifest.node_version().map(str::to_owned),
        dependencies: manifest.dependencies().map(str::to_owned).collect(),
        jsx_import_source: manifest.jsx_import_source().map(str::to_owned),
    })
}

/// The server graph of the package whose folder (relative to `root`) is `folder`, read on disk
/// off the JS thread: its files in Next's App Router server module graph, relative to `folder`
/// (`module_graph::on_disk`); null where the package has none (a source or folder cannot be read,
/// or it is past the walk's file or byte bounds), so the host sends its files no membership. A
/// host asks for it once per walk for a package whose `package` names `next` when a code rule
/// reads `file.package.serverGraph`, and sends each file of the package its own membership
/// (`package.serverGraph`). With `overlay_path`, that source reads as `overlay_source` (deleted
/// without one): an edit or a sample disk does not hold. The task is not cancellable; the walk's
/// bounds limit it.
#[napi]
// Only JavaScript calls it; tests drive `ServerGraphTask::members`.
#[cfg_attr(test, allow(dead_code))]
pub fn server_graph_async(
    root: String,
    folder: String,
    overlay_path: Option<String>,
    overlay_source: Option<String>,
) -> napi::bindgen_prelude::AsyncTask<ServerGraphTask> {
    napi::bindgen_prelude::AsyncTask::new(ServerGraphTask::new(
        root,
        folder,
        overlay_path,
        overlay_source,
    ))
}

/// `server_graph_async`'s work.
pub struct ServerGraphTask {
    root: String,
    folder: String,
    overlay_path: Option<String>,
    overlay_source: Option<String>,
}

impl ServerGraphTask {
    pub(crate) fn new(
        root: String,
        folder: String,
        overlay_path: Option<String>,
        overlay_source: Option<String>,
    ) -> Self {
        Self {
            root,
            folder,
            overlay_path,
            overlay_source,
        }
    }

    /// The members, as the binding resolves them; `None` without a graph.
    pub(crate) fn members(&self) -> Option<Vec<String>> {
        let root = std::path::Path::new(&self.root);
        let overlay = self
            .overlay_path
            .as_deref()
            .map(|path| (path, self.overlay_source.as_deref()));
        module_graph::on_disk(
            root,
            &self.folder,
            &module_graph::config_on_disk(root, &self.folder),
            overlay,
        )
        .map(|members| members.into_iter().collect())
    }
}

impl napi::Task for ServerGraphTask {
    type Output = Option<Vec<String>>;
    type JsValue = Option<Vec<String>>;

    fn compute(&mut self) -> Result<Option<Vec<String>>> {
        Ok(self.members())
    }

    fn resolve(
        &mut self,
        _env: napi::Env,
        output: Option<Vec<String>>,
    ) -> Result<Option<Vec<String>>> {
        Ok(output)
    }
}

/// The project configuration files the package whose folder (relative to `root`) is `folder`
/// reads on disk (`module_graph::Config`): its `tsconfig.json` (else `jsconfig.json`), then the
/// `extends` bases followed, relative to `root`. Conformance forwards them with a corpus, as it
/// forwards its `package.json` files.
#[napi]
pub fn module_config_files(root: String, folder: String) -> Vec<String> {
    module_graph::config_on_disk(std::path::Path::new(&root), &folder)
        .files()
        .to_vec()
}

#[napi]
pub struct CompiledCodeRuleDsl {
    inner: Arc<CompiledCodeRule>,
}

#[napi]
impl CompiledCodeRuleDsl {
    #[napi]
    pub fn canonical_json(&self) -> String {
        self.inner.canonical.clone()
    }

    /// Whether a repository-relative path is inside this code rule's scope: the same
    /// matcher evaluation uses, so a host can hand the code rule only the files it will
    /// read and be sure the two never disagree.
    #[napi]
    pub fn in_scope(&self, path: String) -> bool {
        self.inner.scope.matches(&path)
    }

    #[napi]
    pub fn digest(&self) -> String {
        self.inner.digest.clone()
    }

    #[napi]
    pub fn required_capabilities_json(&self) -> String {
        self.inner.required_capabilities.clone()
    }

    /// Whether the code rule reads `file.package.serverGraph`: a host then sends each file of a
    /// package naming `next` its membership (`serverGraph`).
    #[napi]
    pub fn reads_server_graph(&self) -> bool {
        self.inner.reads_server_graph()
    }

    #[napi]
    pub fn evaluate(&self, snapshot_json: String, mode: String) -> Result<String> {
        let (snapshot, _) = parse_snapshot(&snapshot_json, self.inner.reads_server_graph())?;
        let evaluation = evaluate_code_rule(&self.inner, &snapshot, &mode)?;
        serde_json::to_string(&evaluation_json(evaluation, None, &[], &[])).map_err(json_error)
    }

    #[napi]
    pub fn open_editor(&self, snapshot_json: String) -> Result<CodeRuleDslEditor> {
        let (snapshot, derived) = parse_snapshot(&snapshot_json, self.inner.reads_server_graph())?;
        Ok(CodeRuleDslEditor {
            code_rule: Arc::clone(&self.inner),
            snapshot,
            derived,
            previous_ids: BTreeSet::new(),
        })
    }
}

#[napi]
pub struct CodeRuleDslEditor {
    code_rule: Arc<CompiledCodeRule>,
    snapshot: Snapshot,
    /// The `package` the snapshot's `package.json` entries give the files the host sent none for:
    /// an edit gives them theirs again where it changes what they read (`DerivedPackages::edited`).
    derived: DerivedPackages,
    previous_ids: BTreeSet<String>,
}

#[napi]
impl CodeRuleDslEditor {
    #[napi]
    pub fn evaluate(&mut self, mode: String) -> Result<String> {
        let evaluation = evaluate_code_rule(&self.code_rule, &self.snapshot, &mode)?;
        let current = violation_ids(&evaluation.violations);
        let added = current
            .difference(&self.previous_ids)
            .cloned()
            .collect::<Vec<_>>();
        let retracted = self
            .previous_ids
            .difference(&current)
            .cloned()
            .collect::<Vec<_>>();
        self.previous_ids = current;
        serde_json::to_string(&evaluation_json(
            evaluation,
            Some(self.snapshot.revision),
            &added,
            &retracted,
        ))
        .map_err(json_error)
    }

    #[napi]
    pub fn apply_edit(&mut self, edit_json: String, mode: String) -> Result<String> {
        let edit: Value = serde_json::from_str(&edit_json).map_err(json_error)?;
        let revision = required_u64(&edit, "revision", "edit")? as u32;
        let expected = self.snapshot.revision + 1;
        if revision != expected {
            return Err(invalid(format!(
                "out-of-order edit revision {revision}; expected {expected}"
            )));
        }
        let path = required_str(&edit, "path", "edit")?;
        let start = required_u64(&edit, "startByte", "edit")? as usize;
        let end = required_u64(&edit, "oldEndByte", "edit")? as usize;
        let new_text = required_str(&edit, "newText", "edit")?;
        let index = self
            .snapshot
            .files
            .iter()
            .position(|file| file.path == path)
            .ok_or_else(|| invalid(format!("edit path `{path}` is not in the snapshot")))?;
        let file = &mut self.snapshot.files[index];
        if start > end
            || end > file.source.len()
            || !file.source.is_char_boundary(start)
            || !file.source.is_char_boundary(end)
        {
            return Err(invalid("edit byte range is invalid"));
        }
        Arc::make_mut(&mut file.source).replace_range(start..end, new_text);
        self.derived.edited(&mut self.snapshot, index);
        self.snapshot.revision = revision;
        self.evaluate(mode)
    }
}
