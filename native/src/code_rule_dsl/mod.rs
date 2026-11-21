// The facades re-export their dependency's module by glob, and the mounted sources import the same items under
// the same names: the glob's name and the private one are one item.
#![allow(hidden_glob_reexports)]
// Every module is boot's (rerail-boot). The engine adds napi (the items below, `slice::napi`, the CLI's
// entry points in `tools`) and the tests of them (`napi_tests`, `slice::napi_tests`); every other test is a test
// crate's (native/crates/tests_*).
pub use rerail_boot::code_rule_dsl::*;
// The first slice's entry points (06 S6), called by hosts through `slice::napi::CodeRuleSlice`.
mod slice {
    pub use rerail_boot::code_rule_dsl::slice::*;
    // The host's entry, which only the engine (napi) compiles.
    pub mod napi;
    #[cfg(test)]
    mod napi_tests;
}
#[cfg(test)]
#[allow(dead_code, unused_imports)]
mod test_support;
/// The CLI's tools (boot's), with the engine's napi wrapper and the entry points that boot the program first.
pub mod tools {
    pub use rerail_boot::code_rule_dsl::tools::*;
    mod napi;
    pub use napi::{cli, cli_in, execute_sql_request};
}
// The napi items' own tests: the package facts and server graph a host reads through them.
#[cfg(test)]
mod napi_tests;
// The engine's own items: napi.
use json::{invalid, json_error};
use napi::Result;
use napi_derive::napi;
use serde_json::Value;

#[napi]
pub struct CodeRuleDslRuntime;

#[napi]
impl CodeRuleDslRuntime {
    /// Canonical front-end facts for one complete demand batch, including invalid syntax.
    /// This only parses source; it never imports or executes a source module.
    #[napi(catch_unwind)]
    pub fn python_ast_batch(documents_json: String) -> Result<String> {
        let documents: Vec<value::python_ast::Document> =
            serde_json::from_str(&documents_json).map_err(json_error)?;
        let response =
            value::python_ast::batch_with_metrics(&documents).map_err(|error| invalid(error.0))?;
        serde_json::to_string(&response).map_err(json_error)
    }

    /// The tests the `.rerail` files under `paths_json` (a JSON array of files and directories) carry, each run (decision
    /// 0130, `rerail-engine test-rr`): `[{file, test, line, status, failures, findings}]` as JSON. `rerail rules test`
    /// calls it for a `.rerail` file. The runs read the engine's own `.rerail` programs (`typing/programs`) and the rule
    /// library (`logic::rr::library`) from a checkout (`source_tree`), which the package does not ship: without them,
    /// an error that says so.
    #[napi(catch_unwind)]
    pub fn test_rr(paths_json: String, filter: Option<String>) -> Result<String> {
        let paths: Vec<std::path::PathBuf> =
            serde_json::from_str(&paths_json).map_err(json::json_error)?;
        let programs =
            rerail_store::source_tree::native().join("src/code_rule_dsl/typing/programs");
        if !programs.join("sources.rerail").is_file() || !logic::rr::library().join("core.rerail").is_file()
        {
            return Err(json::invalid(
                "a .rerail file's tests read the engine's .rerail programs and rule library from a Rerail checkout \
                 (native/src/code_rule_dsl/typing/programs, docs/rule-syntax/lib), which Rerail's package does not \
                 ship: run them from a Rerail checkout, or set RERAIL_SOURCE_ROOT to one"
                    .to_owned(),
            ));
        }
        let report =
            logic::rr::testing::report(&paths, filter.as_deref()).map_err(json::invalid)?;
        serde_json::to_string(&report).map_err(json::json_error)
    }

    #[napi(catch_unwind)]
    pub fn compile(
        source: String,
        format: String,
        capabilities_json: String,
    ) -> Result<CompiledCodeRuleDsl> {
        rule_api::compile(&source, &format, &capabilities_json)
    }
}

/// The module a file's `module` carries, from the package markers on disk, for the file at
/// `absolute` whose dialect's label is `label`: a dotted string, a list of names when one keeps
/// a dot, or null outside a package and for a family without modules (`Adapter::module_path`).
/// Hosts and the native repository ingestion paths share this one derivation.
#[napi(catch_unwind)]
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
    /// `true` where the manifest's `private` is `true`; none otherwise.
    pub private: Option<bool>,
    /// Whether npm publishes the file, as eslint-plugin-n's `getNpmignore` reads the manifest's
    /// `files` and `main` and its folder's `.npmignore` or `.gitignore`: the file's own fact,
    /// which `package_facts` sends and `nearest_package` does not.
    pub published: Option<bool>,
}

impl PackageFacts {
    fn of(manifest: &package_manifest::Manifest) -> Self {
        Self {
            folder: manifest.folder().to_owned(),
            package_type: manifest.package_type().as_str().to_owned(),
            bin: manifest.bin().map(str::to_owned).collect(),
            node_version: manifest.node_version().map(str::to_owned),
            dependencies: manifest.dependencies().map(str::to_owned).collect(),
            jsx_import_source: manifest.jsx_import_source().map(str::to_owned),
            private: manifest.is_private().then_some(true),
            published: None,
        }
    }
}

/// The `package` a file carries, for the file at `absolute` whose dialect's label is `label`: its
/// nearest `package.json` at or above it, never above `root` (the workspace's), with whether npm
/// publishes the file, or null without one and for a family that reads none
/// (`Adapter::PACKAGE_MANIFESTS`). Hosts and the native repository ingestion paths share this one
/// derivation; a host reading many files reads them through one `PackageFactsReader`.
#[napi(catch_unwind)]
pub fn package_facts(label: String, absolute: String, root: String) -> Option<PackageFacts> {
    PackageFactsReader::new(root).facts(label, absolute)
}

/// The `package` facts of the files below one root, each manifest and its ignore files read once
/// (`package_manifest::Manifests`): one walk's reader, as files on disk do not change during it.
#[napi]
pub struct PackageFactsReader {
    manifests: parking_lot::Mutex<package_manifest::Manifests>,
}

#[napi]
impl PackageFactsReader {
    #[napi(constructor)]
    pub fn new(root: String) -> Self {
        Self {
            manifests: parking_lot::Mutex::new(package_manifest::Manifests::new(root)),
        }
    }

    /// `package_facts` for the file at `absolute` below the reader's root.
    #[napi(catch_unwind)]
    pub fn facts(&self, label: String, absolute: String) -> Option<PackageFacts> {
        if !language::adapter::package_manifests(&label) {
            return None;
        }
        let mut manifests = self.manifests.lock();
        let file = std::path::Path::new(&absolute);
        let manifest = manifests.of_file(file)?;
        let path = manifests.relative(file);
        Some(PackageFacts {
            published: manifest.publishes(&path),
            ..PackageFacts::of(&manifest)
        })
    }
}

/// The nearest `package.json` at or above `absolute` (a file or a folder's `package.json`),
/// never above `root`, whatever reads it: the package an edit of any file belongs to, so a host
/// can tell which applied sources it may change. Null without one.
#[napi(catch_unwind)]
// Only JavaScript calls it.
#[cfg_attr(test, allow(dead_code))]
pub fn nearest_package(absolute: String, root: String) -> Option<PackageFacts> {
    let manifest =
        package_manifest::Manifests::new(root).of_file(std::path::Path::new(&absolute))?;
    Some(PackageFacts::of(&manifest))
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
        crate::panic_boundary::contain_panic(std::any::type_name::<Self>(), || Ok(self.members()))
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
#[napi(catch_unwind)]
pub fn module_config_files(root: String, folder: String) -> Vec<String> {
    module_graph::config_on_disk(std::path::Path::new(&root), &folder)
        .files()
        .to_vec()
}

pub use rule_api::CompiledCodeRuleDsl;

/// The engine's composition: every provider registered once, in one order (`rerail-boot`).
pub use rerail_boot::code_rule_dsl::boot;
