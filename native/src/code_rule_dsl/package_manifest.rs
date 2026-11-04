//! What the engine reads of an ECMA file's nearest `package.json` (a file's `package`): its
//! `type`, `bin` targets, configured Node range, dependency names, `private`, and which of its
//! files npm publishes (its `files`, `main` and the folder's `.npmignore` or `.gitignore`).
//! Hosts and the native repository index read manifests on disk (`Manifests`); workspaces and
//! stateless snapshots read the `package.json`, `.npmignore` and `.gitignore` entries of their
//! own file set (`manifest_in`). Both go through `Manifest::parse` and
//! `Manifest::with_ignore_files`, so they cannot disagree.

use crate::code_rule_dsl::json::invalid;
use crate::code_rule_dsl::language::{JsxRuntime, PackageType};
use crate::code_rule_dsl::npm_ignore::NpmPublication;
use crate::code_rule_dsl::semver::Range;
use napi::Result;
use parking_lot::Mutex;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

/// Serde's reading of a request file's `package` (`Manifest::from_wire`).
pub(crate) fn deserialize_wire<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Arc<Manifest>>, D::Error> {
    let value = <Value as serde::Deserialize>::deserialize(deserializer)?;
    Manifest::from_wire(&value, "files")
        .map(|manifest| Some(Arc::new(manifest)))
        .map_err(|error| serde::de::Error::custom(error.reason))
}

/// A `package.json` larger than this is not one a person wrote; it is not read (the pack
/// matcher's `MAX_MANIFEST_BYTES`).
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

/// eslint-plugin-n's configured Node range where the manifest gives none (`fallbackRange`).
pub(crate) static DEFAULT_NODE_RANGE: LazyLock<Range> =
    LazyLock::new(|| Range::parse(">=16.0.0").expect("the default Node range parses"));

/// The dependency kinds `dependsOn` and the JSX runtime read.
const DEPENDENCY_KEYS: [&str; 4] = [
    "dependencies",
    "devDependencies",
    "peerDependencies",
    "optionalDependencies",
];

/// The packages that make a manifest's JSX React's, and the ones that make it another library's.
const REACT_PACKAGES: [&str; 4] = ["react", "react-dom", "react-native", "next"];
const OTHER_JSX_PACKAGES: [&str; 7] = [
    "vue",
    "@stencil/core",
    "preact",
    "solid-js",
    "@builder.io/qwik",
    "inferno",
    "mithril",
];

/// The ignore files whose rules decide which files of the package in their folder npm publishes
/// (`NpmPublication`).
pub(crate) const NPMIGNORE: &str = ".npmignore";
pub(crate) const GITIGNORE: &str = ".gitignore";

/// The folder of the package whose publication an ignore file at `path` in a file set decides
/// (`pkg` for `pkg/.npmignore`, `""` for a root `.gitignore`), if it is one.
pub(crate) fn ignore_file_directory(path: &str) -> Option<&str> {
    [NPMIGNORE, GITIGNORE].into_iter().find_map(|name| {
        if path == name {
            return Some("");
        }
        path.strip_suffix(name)?.strip_suffix('/')
    })
}

/// An ECMA file's nearest `package.json`, as `file.package` and the module system read it.
#[derive(Clone, Debug)]
pub(crate) struct Manifest {
    /// The manifest's directory, relative to the workspace root (`""` at the root).
    folder: String,
    package_type: PackageType,
    /// `bin` targets resolved against `folder`, relative to it (`""` is the folder itself).
    bin: BTreeSet<String>,
    /// The configured Node range: `engines.node`, else the `devEngines.runtime` entry named
    /// `node`; `None` where neither gives a valid range.
    node_version: Option<String>,
    /// The names in `dependencies`, `devDependencies`, `peerDependencies` and
    /// `optionalDependencies`.
    dependencies: BTreeSet<String>,
    /// The package's `compilerOptions.jsxImportSource` (`module_graph::Config`): the JSX runtime
    /// its project configuration names.
    jsx_import_source: Option<String>,
    /// The file's or files' place in Next's App Router server module graph (`module_graph`).
    server_graph: ServerGraph,
    /// Whether the manifest's `private` is `true`.
    private: bool,
    /// Which files below the manifest npm publishes.
    publication: Publication,
    /// SHA-256 of `to_wire`, the server graph and the publication: what equality and reuse keys
    /// compare.
    digest: [u8; 32],
    /// `node_version` parsed.
    node_range: Option<Range>,
}

/// Which files below a manifest npm publishes, as eslint-plugin-n's `getNpmignore` reads them.
#[derive(Debug, Clone)]
pub(crate) enum Publication {
    /// Not known: a host sent the file no `published` fact, so neither value holds.
    Unread,
    /// Read from the package: its manifest's `files` and `main`, and its folder's ignore files.
    Package(Arc<PackagePublication>),
    /// One file's fact, a host's: whether npm publishes it.
    Member(bool),
}

/// What a package's publication reads, and its matcher.
pub(crate) struct PackagePublication {
    /// The `files` field's non-empty strings, where it is an array.
    files: Option<Vec<String>>,
    /// `main`, where a string.
    main: Option<String>,
    /// The texts of the folder's `.npmignore` and `.gitignore`, where present.
    npmignore: Option<String>,
    gitignore: Option<String>,
    matcher: NpmPublication,
}

impl PackagePublication {
    fn new(
        folder: &str,
        files: Option<Vec<String>>,
        main: Option<String>,
        npmignore: Option<String>,
        gitignore: Option<String>,
    ) -> Self {
        let matcher = NpmPublication::new(
            folder,
            files.as_deref(),
            main.as_deref(),
            npmignore.as_deref(),
            gitignore.as_deref(),
        );
        Self {
            files,
            main,
            npmignore,
            gitignore,
            matcher,
        }
    }
}

impl std::fmt::Debug for PackagePublication {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PackagePublication")
            .field("files", &self.files)
            .field("main", &self.main)
            .field("npmignore", &self.npmignore)
            .field("gitignore", &self.gitignore)
            .finish_non_exhaustive()
    }
}

/// Whether a configured Node range is a subset of a condition's range, by the two ranges' texts
/// (`Manifest::node_range_is_subset_of`): every manifest with the same range, and every copy of
/// one, asks the same of each gated rule. Cleared when it holds `NODE_SUBSETS_MAX` answers.
static NODE_SUBSETS: LazyLock<Mutex<HashMap<(String, String), bool>>> =
    LazyLock::new(Mutex::default);
const NODE_SUBSETS_MAX: usize = 1 << 16;

/// Whether a configured Node range (`None`: eslint-plugin-n's default, as for a file without a
/// manifest) is a subset of `range`, node-semver's `subset`, through `NODE_SUBSETS`.
pub(crate) fn node_range_is_subset_of(configured: Option<&Range>, range: &Range) -> bool {
    let configured = configured.unwrap_or(&DEFAULT_NODE_RANGE);
    let key = (configured.text().to_owned(), range.text().to_owned());
    if let Some(&subset) = NODE_SUBSETS.lock().get(&key) {
        return subset;
    }
    let subset = configured.is_subset_of(range);
    let mut subsets = NODE_SUBSETS.lock();
    if subsets.len() >= NODE_SUBSETS_MAX {
        subsets.clear();
    }
    subsets.insert(key, subset);
    subset
}

/// Which files below a manifest are in their Next app's server module graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ServerGraph {
    /// Not read: the manifest names no `next`, no code rule reads the graph, or the package has
    /// none (a source or folder on disk cannot be read, or it is past a bound), so neither
    /// membership holds for its files.
    Unread,
    /// Derived from the package's files: the members, relative to the manifest's folder, and the
    /// manifest a file below it carries (`Manifest::for_file`) inside the graph and outside it.
    Members {
        members: Arc<BTreeSet<String>>,
        inside: Arc<Manifest>,
        outside: Arc<Manifest>,
    },
    /// One file's fact, a host's or `Manifest::for_file`'s: whether it is a member.
    Member(bool),
}

impl PartialEq for Manifest {
    fn eq(&self, other: &Self) -> bool {
        self.digest == other.digest
    }
}

impl Eq for Manifest {}

impl Manifest {
    /// `node_version` comes with its parse, which validated it.
    fn new(
        folder: String,
        package_type: PackageType,
        bin: BTreeSet<String>,
        node_version: Option<(String, Range)>,
        dependencies: BTreeSet<String>,
    ) -> Self {
        let (node_version, node_range) = node_version.unzip();
        let mut manifest = Self {
            folder,
            package_type,
            bin,
            node_version,
            dependencies,
            jsx_import_source: None,
            server_graph: ServerGraph::Unread,
            private: false,
            publication: Publication::Unread,
            digest: [0; 32],
            node_range,
        };
        manifest.digest = manifest.compute_digest();
        manifest
    }

    fn compute_digest(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(self.to_wire().to_string().as_bytes());
        if let ServerGraph::Members { members, .. } = &self.server_graph {
            hasher.update(b"\0serverGraph");
            for member in members.iter() {
                hasher.update(b"\0");
                hasher.update(member.as_bytes());
            }
        }
        if let Publication::Package(package) = &self.publication {
            hasher.update(b"\0publication");
            let read = serde_json::json!([
                package.files,
                package.main,
                package.npmignore,
                package.gitignore
            ]);
            hasher.update(read.to_string().as_bytes());
        }
        hasher.finalize().into()
    }

    /// The digest, and the server graph's per-file manifests, again after a change.
    fn rederived(mut self) -> Self {
        if let ServerGraph::Members { members, .. } = &self.server_graph {
            let members = (**members).clone();
            return self.with_server_graph(Some(members));
        }
        self.digest = self.compute_digest();
        self
    }

    /// This manifest with the server graph its package's files give (`module_graph`); without one
    /// (`None`) it carries none (`ServerGraph::Unread`).
    pub(crate) fn with_server_graph(mut self, members: Option<BTreeSet<String>>) -> Self {
        self.server_graph = ServerGraph::Unread;
        if let Some(members) = members {
            let member = |member| {
                let mut manifest = self.clone();
                manifest.server_graph = ServerGraph::Member(member);
                manifest.digest = manifest.compute_digest();
                Arc::new(manifest)
            };
            let (inside, outside) = (member(true), member(false));
            self.server_graph = ServerGraph::Members {
                members: Arc::new(members),
                inside,
                outside,
            };
        }
        self.digest = self.compute_digest();
        self
    }

    /// The manifest the file at `path` (workspace-relative) below `manifest` carries: with a
    /// derived graph, one holding only the file's own membership (as a host sends it), so a
    /// change of the graph changes only the files whose membership it changes; else `manifest`.
    pub(crate) fn for_file(manifest: &Arc<Self>, path: &str) -> Arc<Self> {
        match &manifest.server_graph {
            ServerGraph::Members {
                members,
                inside,
                outside,
            } => {
                let member = manifest
                    .relative_path(path)
                    .is_some_and(|relative| members.contains(relative));
                Arc::clone(if member { inside } else { outside })
            }
            _ => Arc::clone(manifest),
        }
    }

    /// This manifest with the `jsxImportSource` its package's project configuration gives.
    pub(crate) fn with_jsx_import_source(mut self, source: Option<&str>) -> Self {
        self.jsx_import_source = source.map(str::to_owned);
        self.rederived()
    }

    /// This manifest with the texts of its folder's `.npmignore` and `.gitignore` (`None` where
    /// absent), which decide with its `files` and `main` which of its files npm publishes. A
    /// host's manifest, which carries each file's own fact, is returned as it is.
    pub(crate) fn with_ignore_files(
        mut self,
        npmignore: Option<String>,
        gitignore: Option<String>,
    ) -> Self {
        let Publication::Package(package) = &self.publication else {
            return self;
        };
        if package.npmignore == npmignore && package.gitignore == gitignore {
            return self;
        }
        self.publication = Publication::Package(Arc::new(PackagePublication::new(
            &self.folder,
            package.files.clone(),
            package.main.clone(),
            npmignore,
            gitignore,
        )));
        self.rederived()
    }

    /// Whether the manifest's `private` is `true`.
    pub(crate) fn is_private(&self) -> bool {
        self.private
    }

    /// Whether npm publishes the file at `path` (workspace-relative), as eslint-plugin-n's
    /// `getNpmignore` reads it (`NpmPublication`); `None` where a host sent no fact, or for a
    /// path outside the manifest's folder.
    pub(crate) fn publishes(&self, path: &str) -> Option<bool> {
        match &self.publication {
            Publication::Unread => None,
            Publication::Member(published) => Some(*published),
            Publication::Package(package) => self
                .relative_path(path)
                .map(|relative| !package.matcher.leaves_out(relative)),
        }
    }

    /// Whether the package's files have a server graph: it names a `next` dependency.
    pub(crate) fn has_server_graph(&self) -> bool {
        self.dependencies
            .contains(crate::code_rule_dsl::module_graph::NEXT)
    }

    /// Whether the file at `path` (workspace-relative) is in its Next app's server graph; `None`
    /// where the manifest carries none (it names no `next`, nothing read the graph, or the
    /// package has none).
    pub(crate) fn in_server_graph(&self, path: &str) -> Option<bool> {
        match &self.server_graph {
            ServerGraph::Unread => None,
            ServerGraph::Member(member) => Some(*member),
            ServerGraph::Members { members, .. } => Some(
                self.relative_path(path)
                    .is_some_and(|relative| members.contains(relative)),
            ),
        }
    }

    /// A `package.json` that cannot be read (not a regular file, over 1 MiB, unreadable, or not
    /// JSON): a manifest with no valid `type` and nothing else, whose publication only its
    /// folder's ignore files decide.
    fn unreadable(folder: String) -> Self {
        let mut manifest = Self::new(
            folder,
            PackageType::Unset,
            BTreeSet::new(),
            None,
            BTreeSet::new(),
        );
        manifest.publication = Publication::Package(Arc::new(PackagePublication::new(
            &manifest.folder,
            None,
            None,
            None,
            None,
        )));
        manifest.digest = manifest.compute_digest();
        manifest
    }

    /// The manifest in `folder` whose text is `bytes`. eslint-plugin-n's readings: `bin` as a
    /// string or the string values of an object, each resolved against the folder (one that
    /// leaves it never names a file below it); `engines.node` when it is a valid range, else the
    /// first `devEngines.runtime` entry named `node` with a string `version`, when that is valid;
    /// `files` (its non-empty strings, where an array) and `main` (where a string), which with
    /// the folder's ignore files (`with_ignore_files`, none until then) decide which files npm
    /// publishes; `private` when it is `true`.
    pub(crate) fn parse(folder: String, bytes: &[u8]) -> Self {
        let manifest = (bytes.len() <= MAX_MANIFEST_BYTES)
            .then(|| serde_json::from_slice::<Value>(bytes).ok())
            .flatten();
        let Some(Value::Object(manifest)) = manifest else {
            return Self::unreadable(folder);
        };
        let package_type = match manifest.get("type").and_then(Value::as_str) {
            Some("module") => PackageType::Module,
            Some("commonjs") => PackageType::CommonJs,
            _ => PackageType::Unset,
        };
        let targets = match manifest.get("bin") {
            Some(Value::String(target)) if !target.is_empty() => vec![target.as_str()],
            Some(Value::Object(targets)) => targets.values().filter_map(Value::as_str).collect(),
            Some(Value::Array(targets)) => targets.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        let dependencies = DEPENDENCY_KEYS
            .iter()
            .filter_map(|key| manifest.get(*key).and_then(Value::as_object))
            .flat_map(Map::keys)
            .cloned()
            .collect();
        let bin = targets
            .into_iter()
            .filter_map(|target| resolve_target(&folder, target))
            .collect();
        let files = manifest
            .get("files")
            .and_then(Value::as_array)
            .map(|files| {
                files
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|file| !file.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            });
        let main = manifest
            .get("main")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let mut parsed = Self::new(
            folder,
            package_type,
            bin,
            configured_node_version(&manifest),
            dependencies,
        );
        parsed.private = manifest.get("private") == Some(&Value::Bool(true));
        parsed.publication = Publication::Package(Arc::new(PackagePublication::new(
            &parsed.folder,
            files,
            main,
            None,
            None,
        )));
        parsed.digest = parsed.compute_digest();
        parsed
    }

    /// The host's `package` fact: `{ folder, type, bin?, nodeVersion?, dependencies?,
    /// jsxImportSource?, private?, serverGraph?, published? }`, as `to_wire` writes it;
    /// `nodeVersion` must be a valid npm range, as the on-disk reader keeps only one
    /// (`configured_node_version`); `serverGraph` and `published` are the file's own facts.
    pub(crate) fn from_wire(value: &Value, at: &str) -> Result<Self> {
        let malformed = || {
            invalid(format!(
                "{at}/package must be {{ \"folder\": string, \"type\": \"module\" | \"commonjs\" | \"unset\", \"bin\"?: [string], \"nodeVersion\"?: string (a valid npm range), \"dependencies\"?: [string], \"jsxImportSource\"?: string, \"private\"?: boolean, \"serverGraph\"?: boolean, \"published\"?: boolean }}"
            ))
        };
        let object = value.as_object().ok_or_else(malformed)?;
        if object.keys().any(|key| {
            !matches!(
                key.as_str(),
                "folder"
                    | "type"
                    | "bin"
                    | "nodeVersion"
                    | "dependencies"
                    | "jsxImportSource"
                    | "private"
                    | "serverGraph"
                    | "published"
            )
        }) {
            return Err(malformed());
        }
        let names = |key: &str| -> Result<BTreeSet<String>> {
            match object.get(key) {
                None => Ok(BTreeSet::new()),
                Some(Value::Array(names)) => names
                    .iter()
                    .map(|name| name.as_str().map(str::to_owned).ok_or_else(malformed))
                    .collect(),
                Some(_) => Err(malformed()),
            }
        };
        let node_version = match object.get("nodeVersion") {
            None => None,
            Some(Value::String(version)) => Some((
                version.clone(),
                Range::parse(version).ok_or_else(malformed)?,
            )),
            Some(_) => return Err(malformed()),
        };
        let jsx_import_source = match object.get("jsxImportSource") {
            None => None,
            Some(Value::String(source)) => Some(source.clone()),
            Some(_) => return Err(malformed()),
        };
        let server_graph = match object.get("serverGraph") {
            None => ServerGraph::Unread,
            Some(Value::Bool(member)) => ServerGraph::Member(*member),
            Some(_) => return Err(malformed()),
        };
        let private = match object.get("private") {
            None => false,
            Some(Value::Bool(private)) => *private,
            Some(_) => return Err(malformed()),
        };
        let publication = match object.get("published") {
            None => Publication::Unread,
            Some(Value::Bool(published)) => Publication::Member(*published),
            Some(_) => return Err(malformed()),
        };
        let mut manifest = Self::new(
            object
                .get("folder")
                .and_then(Value::as_str)
                .ok_or_else(malformed)?
                .to_owned(),
            object
                .get("type")
                .and_then(Value::as_str)
                .and_then(PackageType::parse)
                .ok_or_else(malformed)?,
            names("bin")?,
            node_version,
            names("dependencies")?,
        );
        manifest.jsx_import_source = jsx_import_source;
        manifest.server_graph = server_graph;
        manifest.private = private;
        manifest.publication = publication;
        manifest.digest = manifest.compute_digest();
        Ok(manifest)
    }

    /// The `package` fact hosts send for this manifest.
    pub(crate) fn to_wire(&self) -> Value {
        let mut wire = Map::new();
        wire.insert("folder".to_owned(), Value::from(self.folder.as_str()));
        wire.insert("type".to_owned(), Value::from(self.package_type.as_str()));
        if !self.bin.is_empty() {
            wire.insert("bin".to_owned(), Value::from_iter(self.bin.iter().cloned()));
        }
        if let Some(version) = &self.node_version {
            wire.insert("nodeVersion".to_owned(), Value::from(version.as_str()));
        }
        if !self.dependencies.is_empty() {
            wire.insert(
                "dependencies".to_owned(),
                Value::from_iter(self.dependencies.iter().cloned()),
            );
        }
        if let Some(source) = &self.jsx_import_source {
            wire.insert("jsxImportSource".to_owned(), Value::from(source.as_str()));
        }
        if self.private {
            wire.insert("private".to_owned(), Value::from(true));
        }
        if let ServerGraph::Member(member) = self.server_graph {
            wire.insert("serverGraph".to_owned(), Value::from(member));
        }
        if let Publication::Member(published) = self.publication {
            wire.insert("published".to_owned(), Value::from(published));
        }
        Value::Object(wire)
    }

    pub(crate) fn folder(&self) -> &str {
        &self.folder
    }

    pub(crate) fn package_type(&self) -> PackageType {
        self.package_type
    }

    pub(crate) fn bin(&self) -> impl Iterator<Item = &str> {
        self.bin.iter().map(String::as_str)
    }

    pub(crate) fn node_version(&self) -> Option<&str> {
        self.node_version.as_deref()
    }

    pub(crate) fn dependencies(&self) -> impl Iterator<Item = &str> {
        self.dependencies.iter().map(String::as_str)
    }

    /// The digest reuse keys carry for this manifest.
    pub(crate) fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    /// `path` (workspace-relative) relative to the manifest's folder; `None` outside it.
    pub(crate) fn relative_path<'p>(&self, path: &'p str) -> Option<&'p str> {
        if self.folder.is_empty() {
            return Some(path);
        }
        path.strip_prefix(self.folder.as_str())?.strip_prefix('/')
    }

    /// Whether the file at `path` is one of the manifest's `bin` targets, as eslint-plugin-n's
    /// `isBinFile` resolves them: the path itself, without `.js`, or without `.js` and a last
    /// `/index`.
    pub(crate) fn is_bin(&self, path: &str) -> bool {
        let Some(relative) = self.relative_path(path) else {
            return false;
        };
        // Rooted at the folder, as `path.resolve` gives absolute paths: `/index.js` there drops
        // to `""`, the folder itself.
        let rooted = format!("/{relative}");
        let without_js = rooted.strip_suffix(".js").unwrap_or(&rooted);
        let without_index = without_js.strip_suffix("/index").unwrap_or(without_js);
        self.bin.iter().any(|target| {
            [rooted.as_str(), without_js, without_index]
                .iter()
                .any(|candidate| match candidate.strip_prefix('/') {
                    Some(rest) => !target.is_empty() && rest == target,
                    None => target.is_empty(),
                })
        })
    }

    /// Whether the manifest lists any of `names` as a dependency of any kind.
    pub(crate) fn depends_on(&self, names: &[String]) -> bool {
        names.iter().any(|name| self.dependencies.contains(name))
    }

    /// Whether eslint-plugin-n's configured Node range for files below the manifest (its own, else
    /// the default) is a subset of `range`, worked out once per pair of range texts (`NODE_SUBSETS`).
    pub(crate) fn node_range_is_subset_of(&self, range: &Range) -> bool {
        node_range_is_subset_of(self.node_range.as_ref(), range)
    }

    /// The JSX runtime the package names: its `jsxImportSource` where that names React (or a
    /// React wrapper) or another JSX library, else its dependencies, React's packages and no other
    /// JSX library's or the reverse; `Unknown` for both or neither.
    pub(crate) fn jsx_runtime(&self) -> JsxRuntime {
        if let Some(runtime) = self
            .jsx_import_source
            .as_deref()
            .and_then(crate::code_rule_dsl::language::jsx_import_source_runtime)
        {
            return runtime;
        }
        let lists = |names: &[&str]| names.iter().any(|name| self.dependencies.contains(*name));
        match (lists(&REACT_PACKAGES), lists(&OTHER_JSX_PACKAGES)) {
            (true, false) => JsxRuntime::React,
            (false, true) => JsxRuntime::Other,
            _ => JsxRuntime::Unknown,
        }
    }

    pub(crate) fn jsx_import_source(&self) -> Option<&str> {
        self.jsx_import_source.as_deref()
    }
}

/// eslint-plugin-n's `getEnginesNode`, then `getDevEnginesNode`: the first source that is
/// present decides, and one whose range does not parse falls through to the next.
fn configured_node_version(manifest: &Map<String, Value>) -> Option<(String, Range)> {
    if let Some(Value::String(node)) = manifest
        .get("engines")
        .and_then(Value::as_object)
        .and_then(|engines| engines.get("node"))
        && let Some(range) = Range::parse(node)
    {
        return Some((node.clone(), range));
    }
    let runtime = manifest
        .get("devEngines")
        .and_then(Value::as_object)?
        .get("runtime")?;
    let entries = match runtime {
        Value::Array(entries) => entries.as_slice(),
        entry => std::slice::from_ref(entry),
    };
    let version = entries.iter().find_map(|entry| {
        (entry.get("name").and_then(Value::as_str) == Some("node"))
            .then(|| entry.get("version").and_then(Value::as_str))
            .flatten()
    })?;
    Range::parse(version).map(|range| (version.to_owned(), range))
}

/// A `bin` target (or `main`) of the manifest in `folder` relative to that folder, as Node's
/// `path.resolve` resolves it against the folder; `None` for an absolute target, or one that ends
/// outside the folder and so names no file below it.
pub(crate) fn resolve_target(folder: &str, target: &str) -> Option<String> {
    if target.starts_with('/') {
        return None;
    }
    let mut names = folder
        .split('/')
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    let depth = names.len();
    for name in target.split('/') {
        match name {
            "" | "." => {}
            ".." => {
                names.pop()?;
            }
            name => names.push(name),
        }
    }
    let inside = names.len() >= depth
        && folder
            .split('/')
            .filter(|name| !name.is_empty())
            .eq(names[..depth].iter().copied());
    inside.then(|| names[depth..].join("/"))
}

/// The directory a file set's path holds a manifest for (`pkg` for `pkg/package.json`, `""` for
/// a root one), if any.
pub(crate) fn manifest_directory(path: &str) -> Option<&str> {
    path.strip_suffix("/package.json")
        .or_else(|| (path == "package.json").then_some(""))
}

/// The nearest manifest at or above `path` in a file set (a workspace, a repository snapshot)
/// whose manifests `manifest` returns by directory, as the file carries it (`Manifest::for_file`):
/// the one a host reading the same files on disk would send.
pub(crate) fn manifest_in<'m>(
    path: &str,
    manifest: impl Fn(&str) -> Option<&'m Arc<Manifest>>,
) -> Option<Arc<Manifest>> {
    let mut rest = path;
    loop {
        let directory = rest.rfind('/').map_or("", |at| &rest[..at]);
        if let Some(found) = manifest(directory) {
            return Some(Manifest::for_file(found, path));
        }
        if directory.is_empty() {
            return None;
        }
        rest = directory;
    }
}

/// The manifest in `directory` on disk, `None` without one. It is read as the pack matcher reads
/// one: a regular file (never a symlink) of at most 1 MiB; any other is `unreadable`.
fn manifest_on_disk(directory: &Path, folder: String) -> Option<Manifest> {
    let path = directory.join("package.json");
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(_) => return Some(Manifest::unreadable(folder)),
    };
    let manifest = if !metadata.is_file() || metadata.len() > MAX_MANIFEST_BYTES as u64 {
        Manifest::unreadable(folder)
    } else {
        match std::fs::read(&path) {
            Ok(bytes) => Manifest::parse(folder, &bytes),
            Err(_) => Manifest::unreadable(folder),
        }
    };
    Some(manifest.with_ignore_files(
        ignore_file_on_disk(directory, NPMIGNORE),
        ignore_file_on_disk(directory, GITIGNORE),
    ))
}

/// The text of the ignore file `name` in `directory` on disk, read as a manifest is: a regular
/// file (never a symlink, which eslint-plugin-n would follow) of at most 1 MiB, its invalid UTF-8
/// replaced; `None` for any other, or where it cannot be read.
fn ignore_file_on_disk(directory: &Path, name: &str) -> Option<String> {
    let path = directory.join(name);
    let metadata = std::fs::symlink_metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_MANIFEST_BYTES as u64 {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// The nearest manifest of files below a root, looked up once per directory: the `package.json`
/// at or above a file's directory, never above the root (the workspace's). Every ingestion path
/// that reads files on disk derives `package` here. With `server_graph`, a manifest naming
/// `next` carries its package's server graph, read on disk (`module_graph::on_disk`).
pub(crate) struct Manifests {
    root: PathBuf,
    /// `root` with its symlinks resolved, `None` where it cannot be: a directory is read only
    /// where it resolves below it, so a symlinked folder pointing outside the root holds no
    /// manifest.
    canonical_root: Option<PathBuf>,
    server_graph: bool,
    directories: HashMap<PathBuf, Option<Arc<Manifest>>>,
}

impl Manifests {
    pub(crate) fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            canonical_root: std::fs::canonicalize(&root).ok(),
            root,
            server_graph: false,
            directories: HashMap::new(),
        }
    }

    /// Manifests that carry their package's server graph where one is read.
    pub(crate) fn reading_server_graphs(mut self, server_graph: bool) -> Self {
        self.server_graph = server_graph;
        self
    }

    /// `file`'s nearest manifest, as the file carries it (`Manifest::for_file`); `None` without
    /// one, or for a file outside the root.
    pub(crate) fn of_file(&mut self, file: &Path) -> Option<Arc<Manifest>> {
        let manifest = self.of_directory(file.parent()?)?;
        Some(Manifest::for_file(&manifest, &self.folder(file)))
    }

    /// `path` relative to the root, `/`-separated: a file's workspace-relative path.
    pub(crate) fn relative(&self, path: &Path) -> String {
        self.folder(path)
    }

    /// The nearest manifest at or above `directory`; `None` without one, or for a directory
    /// outside the root.
    pub(crate) fn of_directory(&mut self, directory: &Path) -> Option<Arc<Manifest>> {
        if !directory.starts_with(&self.root) {
            return None;
        }
        let mut unknown = Vec::new();
        let mut found = None;
        for ancestor in directory.ancestors() {
            if let Some(known) = self.directories.get(ancestor) {
                found = known.clone();
                break;
            }
            unknown.push(ancestor);
            let contained = self.canonical_root.as_ref().is_some_and(|root| {
                std::fs::canonicalize(ancestor).is_ok_and(|directory| directory.starts_with(root))
            });
            if let Some(manifest) = contained
                .then(|| manifest_on_disk(ancestor, self.folder(ancestor)))
                .flatten()
            {
                let config = crate::code_rule_dsl::module_graph::config_on_disk(
                    &self.root,
                    manifest.folder(),
                );
                let manifest = manifest.with_jsx_import_source(config.jsx_import_source());
                let manifest = if self.server_graph && manifest.has_server_graph() {
                    let members = crate::code_rule_dsl::module_graph::on_disk(
                        &self.root,
                        manifest.folder(),
                        &config,
                        None,
                    );
                    manifest.with_server_graph(members)
                } else {
                    manifest
                };
                found = Some(Arc::new(manifest));
                break;
            }
            if ancestor == self.root {
                break;
            }
        }
        for ancestor in unknown {
            self.directories
                .insert(ancestor.to_path_buf(), found.clone());
        }
        found
    }

    /// `directory` relative to the root, `/`-separated.
    fn folder(&self, directory: &Path) -> String {
        directory
            .strip_prefix(&self.root)
            .map(|relative| {
                relative
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::Manifest;
    use crate::code_rule_dsl::semver::{Range, tally};
    use serde_json::json;

    /// A manifest's configured Node range is parsed once, on disk and from a host alike.
    #[test]
    fn a_manifest_parses_its_node_range_once() {
        let parses = tally::PARSES.get();
        let manifest = Manifest::parse(String::new(), br#"{ "engines": { "node": ">=20" } }"#);
        assert_eq!(tally::PARSES.get() - parses, 1);
        assert_eq!(manifest.node_version(), Some(">=20"));
        let parses = tally::PARSES.get();
        let wire = json!({ "folder": "", "type": "unset", "nodeVersion": ">=20" });
        let manifest = Manifest::from_wire(&wire, "files/0").unwrap();
        assert_eq!(tally::PARSES.get() - parses, 1);
        assert_eq!(manifest.node_version(), Some(">=20"));
    }

    /// Whether a long `||` range is a subset of a condition's is worked out once per condition
    /// range, whichever copy of the manifest a file carries, with node-semver's answer.
    #[test]
    fn a_manifest_works_out_each_node_range_subset_once() {
        let alternatives = (0..1_000)
            .map(|minor| format!("1.{minor}.0"))
            .collect::<Vec<_>>()
            .join(" || ");
        let manifest = Manifest::parse(
            String::new(),
            json!({ "engines": { "node": alternatives } })
                .to_string()
                .as_bytes(),
        );
        let copy = manifest.clone().with_jsx_import_source(Some("preact"));
        for (condition, expected) in [(">=1.0.0 <2.0.0", true), ("^1.2.0", false)] {
            let condition = Range::parse(condition).unwrap();
            let subsets = tally::SIMPLE_SUBSETS.get();
            assert_eq!(manifest.node_range_is_subset_of(&condition), expected);
            let worked = tally::SIMPLE_SUBSETS.get() - subsets;
            assert!((1..=1_000).contains(&worked), "{worked} simpleSubset calls");
            let subsets = tally::SIMPLE_SUBSETS.get();
            for _ in 0..100 {
                assert_eq!(manifest.node_range_is_subset_of(&condition), expected);
                assert_eq!(copy.node_range_is_subset_of(&condition), expected);
            }
            assert_eq!(tally::SIMPLE_SUBSETS.get() - subsets, 0);
        }
    }
}
