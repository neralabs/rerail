mod blade;
mod budget;
// The engine's build identities, which it sets as it loads (`crate::install_build_ids`).
pub(crate) mod build_ids;
mod calls;
mod compile;
mod crypto;
mod cpython;
mod demand;
// Metal as the engine's device (`rerail-device-metal`): registered as the library loads (`install_devices`).
#[cfg(target_os = "macos")]
mod device_metal;
mod embed;
mod eval;
mod facts;
// Wired to the findings layer in a later slice; until then only tests call it.
#[allow(dead_code)]
mod findings;
// The relational pools' hook slots, compiled wherever the pools are (the hub's relational stage is in the library).
mod hooks;
mod identifier;
mod import_target;
mod json;
mod language;
mod lines;
mod literal;
mod logic;
/// A pool job's span under the span that started it (`crate::threads`), so a job's work is charged to its caller.
pub(crate) use logic::profile::{here as profile_here, span_at as profile_span_at};
/// The profile's SQL hazards, which the content store's connections and writer report (`content_store::hazards`, set as
/// the library loads; decision 0104).
pub(crate) mod sql_hazards {
    pub(crate) use super::logic::profile::{behind_the_run, on, sql_in_wave, wait_in_wave};
}
mod sql_executor;
mod matcher;
mod model;
mod module_graph;
mod module_path;
mod npm_ignore;
mod origin;
mod package_manifest;
// P's phase clocks; read by profiling once stages are named (05 Phase 11 row 2).
#[allow(dead_code)]
mod phase_clock;
mod php_literal;
mod php_view;
mod placement;
#[allow(dead_code)]
mod producers;
// Published from the scan in a later slice; until then only tests call it.
#[allow(dead_code)]
pub(crate) mod prefilter;
pub(in crate::code_rule_dsl) mod space;
// Read by the prefilter's lowering; the compiled-plan cache is wired with the plans row.
#[allow(dead_code)]
mod stats;
// Wall time by stage, with RERAIL_PROF set (O28).
pub(crate) mod prof;
// P's hooks are called from its matcher and relational code; W wires them with 03's stage names.
#[cfg(feature = "code-rule-profile")]
#[allow(dead_code)]
mod profile;
// The same SQL diagnostics serve standalone execution and the workspace profiler.
#[path = "profile/sql.rs"]
pub(in crate::code_rule_dsl) mod sql_profile;
mod query;
// Admission reads these once the batch budget is wired; until then only tests call it.
#[allow(dead_code)]
mod resolution;
mod search_index;
#[cfg(test)]
mod walks;
mod residency;
// P's paths for P's relational stage, under test (open item O18): P's workspace code reads it here.
use residency::admission::relational;
mod scope;
mod semver;
// The first slice's entry points (06 S6), called by hosts through `slice::napi::CodeRuleSlice`.
mod slice;
mod soundness;
mod suppress;
mod tape;
pub(crate) mod tree;

/// The language tag a stored tape image names (`tape::stored_tape_tag`): the store picks a
/// frame's dictionary by it.
pub(crate) fn stored_tape_tag(bytes: &[u8]) -> Option<u8> {
    tape::stored_tape_tag(bytes)
}

/// The language tag of the dialect a file at `path` is parsed as.
pub(crate) fn path_tape_tag(path: &str) -> Option<u8> {
    language::tape_tag(language::dialect_by_label(language::label_of_path(path)?)?)
}
pub(crate) mod text;
#[cfg(test)]
mod test_support;
pub mod tools;
mod transducer;
// Typing's rounds run over T's facts in tests only until Phase 10 wires them.
#[allow(dead_code)]
mod typing;
mod value;
mod violations;
mod vue;
// Wired to the logic planner and the push path in a later slice; until then only tests call it.
#[allow(dead_code)]
mod waves;
pub(crate) mod workspace;

#[cfg(test)]
pub(crate) mod fixtures;
#[cfg(test)]
pub(crate) use fixtures::{fixtures_dir, oracle};

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

    #[napi(catch_unwind)]
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
        let (id, rule_mode, scope, clauses, required, report_count_gt) =
            compile_document(&canonical_value, &capabilities)?;
        Ok(CompiledCodeRuleDsl {
            inner: Arc::new(CompiledCodeRule {
                id,
                report_count_gt,
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

#[napi]
pub struct CompiledCodeRuleDsl {
    inner: Arc<CompiledCodeRule>,
}

#[napi]
impl CompiledCodeRuleDsl {
    /// Audit the normal candidate compiler over fixed permutations. This executes no candidate SQL.
    #[napi]
    pub fn candidate_frontier_audit(&self) -> String {
        prefilter::candidate_frontier_audit(&self.inner).to_string()
    }

    #[napi]
    pub fn canonical_json(&self) -> String {
        self.inner.canonical.clone()
    }

    /// Whether a repository-relative path is inside this code rule's scope: the same
    /// matcher evaluation uses, so a host can hand the code rule only the files it will
    /// read and be sure the two never disagree.
    #[napi(catch_unwind)]
    pub fn in_scope(&self, path: String) -> bool {
        self.inner.scope.matches(&path)
    }

    /// Whether the code rule reads text: a clause holds a regex or file query, so files no
    /// code-rule language lists are read for it, as `text`.
    #[napi]
    pub fn reads_text(&self) -> bool {
        self.inner.reads_text()
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

    #[napi(catch_unwind)]
    pub fn evaluate(&self, snapshot_json: String, mode: String) -> Result<String> {
        let (snapshot, _) = parse_snapshot(&snapshot_json, self.inner.reads_server_graph())?;
        let evaluation = evaluate_code_rule(&self.inner, &snapshot, &mode)?;
        serde_json::to_string(&evaluation_json(evaluation, None, &[], &[])).map_err(json_error)
    }

    #[napi(catch_unwind)]
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
    #[napi(catch_unwind)]
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

    #[napi(catch_unwind)]
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

/// The engine's language packs, into the core's registry (`language::adapter::pack`). Called as the library
/// loads, before any run: the core asks a family's questions of the pack registered here.
pub(crate) fn install_languages() {
    use language::Family;
    use language::adapter::pack::{Pack, pack, register};
    use language::adapter::{blade::Blade, ecma::Ecma, json::Json, php::Php, python::Python, rr::Rr, rust::Rust, sql::Sql, vue::Vue};
    register(Family::Python, &Pack::<Python>::NEW);
    language::register_base(Family::Python, &Pack::<Python>::NEW);
    register(Family::Ecma, &Pack::<Ecma>::NEW);
    language::register_base(Family::Ecma, &Pack::<Ecma>::NEW);
    register(Family::Php, &Pack::<Php>::NEW);
    language::register_base(Family::Php, &Pack::<Php>::NEW);
    register(Family::Blade, &Pack::<Blade>::NEW);
    language::register_base(Family::Blade, &Pack::<Blade>::NEW);
    register(Family::Rust, &Pack::<Rust>::NEW);
    language::register_base(Family::Rust, &Pack::<Rust>::NEW);
    register(Family::Sql, &Pack::<Sql>::NEW);
    language::register_base(Family::Sql, &Pack::<Sql>::NEW);
    register(Family::Vue, &Pack::<Vue>::NEW);
    language::register_base(Family::Vue, &Pack::<Vue>::NEW);
    register(Family::Rr, &Pack::<Rr>::NEW);
    language::register_base(Family::Rr, &Pack::<Rr>::NEW);
    register(Family::Json, &Pack::<Json>::NEW);
    language::register_base(Family::Json, &Pack::<Json>::NEW);
    // The linters' suppressions, in the order `suppress` is parsed and checked in.
    {
        use suppress::{eslint, noqa, nosec, oxlint, biome, phpstan, psalm, phpcs, clippy, sqlfluff, kind};
        suppress::install(vec![
            kind::<eslint::Suppressions>(),
            kind::<noqa::Suppressions>(),
            kind::<nosec::Suppressions>(),
            kind::<oxlint::Suppressions>(),
            kind::<biome::Suppressions>(),
            kind::<phpstan::Suppressions>(),
            kind::<psalm::Suppressions>(),
            kind::<phpcs::Suppressions>(),
            kind::<clippy::Suppressions>(),
            kind::<sqlfluff::Suppressions>(),
        ]);
    }
    // The embedders, by `Via` (`embed::Embedder`), and the program scopes an embedding gives a meaning of its own.
    embed::install_embedders(vec![
        #[cfg(feature = "lang-blade")]
        (language::Via::CompiledView, &embed::CompiledView),
        #[cfg(feature = "lang-vue")]
        (language::Via::ScriptBlock, &embed::ScriptBlock),
        #[cfg(any(test, feature = "test-hooks"))]
        (language::Via::TestScript, &embed::TestScript),
    ]);
    scope::install_program_scopes(vec![
        #[cfg(feature = "lang-blade")]
        (language::Via::CompiledView, embed::compiled_view::scope),
    ]);
    // What reads W's values off a file's tape for facts' `value` relations.
    facts::core::install_value_reader(&matcher::Values);
    // The families' producers of a batch's relations: Python's f-string trees, CPython's syntax, string annotations.
    facts::vtab::syntax::install_producers(vec![
        &facts::vtab::syntax::fstring::Producer,
        &facts::vtab::syntax::cpython::Producer,
        &facts::vtab::syntax::annotation::Producer,
    ]);
    // The families' lazy relations over a batch (Python's `py.*`).
    facts::vtab::syntax::install_families(vec![&facts::vtab::syntax::py_families::Families]);
    // The families' tables over a bound `node` relation (Python's semantic producers).
    facts::vtab::install_node_tables(vec![facts::vtab::python::register]);
    // The semantic facts a program's native steps demand (Python's).
    facts::vtab::syntax::install_semantic_producer(&facts::vtab::syntax::python_demand::Semantic);
    // What each family reads of its own core.rr facts.
    {
        use crate::code_rule_dsl::facts::vtab::syntax::core_rr::{self, Family};
        use crate::code_rule_dsl::facts::vtab::syntax::{ecma_core, python_core, rr_core};
        core_rr::register_dialect_facts(Family::Python, &python_core::PythonFacts);
        core_rr::register_dialect_facts(Family::Ecma, &ecma_core::EcmaFacts);
        core_rr::register_dialect_facts(Family::Rr, &rr_core::RrFacts);
    }
    // A workspace's project context of ECMAScript's files (manifests, project configurations, server graphs).
    workspace::ingest::install_project(|| Box::new(workspace::ingest::ecma::EcmaProject::default()));
    // A snapshot's context of ECMAScript's files (its manifests).
    eval::install_snapshot_context(eval::ecma::derive);
    // The numbers `toPrecision` reads (ECMAScript's).
    matcher::install_number_text(&value::ecma::Numbers);
    // The canonical AST stage's producer (Python's CPython `ast`).
    residency::install_ast_producer(&residency::PythonAst);
    // What ECMAScript's import targets resolve among (`resolution`).
    resolution::install(import_target::held::RESOLUTIONS);
    // A repository's context as its index reads it (ECMAScript's `package.json` files).
    workspace::ingest::install_repository_context(workspace::ingest::ecma::repository_context);
    // The language terms (ECMAScript's `importTarget`).
    matcher::install_terms(vec![compile::import_target::TERM]);
    // Each format style's preparer (`formatArgs`): Python's, PHP's and Rust's.
    {
        use matcher::format_args::{Style, printf, python, rust, scanf};
        matcher::format_args::install_format_preparers(vec![
            (Style::Brace, &python::BRACE),
            (Style::Percent, &python::PERCENT),
            (Style::Printf, &printf::Printf),
            (Style::Scanf, &scanf::Scanf),
            (Style::Rust, &rust::Rust),
        ]);
    }
    // What compiles `resolves` over a flow (PHP's).
    compile::resolve::install_flow_resolves(compile::resolve::php::compile_php_resolves);
    // The containment relations' `nearest` solve (the file-local FIX's).
    facts::nearest::install_nearest(typing::file_local::nearest);
    // `core.rr`'s names, as the rule language reads them.
    facts::core::install_core_names(logic::rr::core_names);
    // What compiles a program's `ast` matchers into the transducer wave facts runs (decision 0106).
    facts::vtab::syntax::install_ast_compiler(&facts::vtab::syntax::ast_wave::Compiler);
    // The literal grammars `parse` applies, in the order messages list them (`literal::LiteralGrammar`).
    literal::install(vec![
        literal::percent::GRAMMAR,
        literal::identifier_words::GRAMMAR,
        literal::python_fstring::GRAMMAR,
        literal::brace::GRAMMAR,
        literal::annotation::GRAMMAR,
        literal::eslint_directive::GRAMMAR,
        literal::printf::GRAMMAR,
        literal::scanf::GRAMMAR,
        literal::phpdoc::GRAMMAR,
        literal::rust_format::GRAMMAR,
        #[cfg(test)]
        literal::CHARS,
    ]);
    // What a `package` file condition reads of a file without a manifest (ECMA's default Node range).
    language::set_no_manifest(&package_manifest::NoManifest);
    language::set_manifest_reader(package_manifest::read_wire);
    // The dialects their languages parse themselves (`tape::TreeParser`). A build without a language's
    // feature routes no file to it, so its parser is not linked.
    {
        let mut parsers: Vec<(language::GrammarDialect, &'static dyn tape::TreeParser)> = Vec::new();
        if cfg!(feature = "lang-blade") {
            parsers.push((language::GrammarDialect::Blade, &blade::tape::Forte));
        }
        if cfg!(feature = "lang-json") {
            parsers.push((language::GrammarDialect::Json, &transducer::packed::json_parser::tape::Bitlex));
        }
        tape::install_parsers(parsers);
    }
    // Every family the build has, registered (`pack` refuses one that is not).
    Family::ALL.into_iter().for_each(|family| _ = pack(family));
}

/// The engine's devices, into the logic's and the transducer's registries (`logic::device`,
/// `transducer::packed::device`). Called as the library loads, before any run.
pub(crate) fn install_devices() {
    #[cfg(target_os = "macos")]
    device_metal::register();
}
