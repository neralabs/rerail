//! ECMAScript's context of a snapshot's files (`SnapshotContext`): the `package` it gives each ECMA file the host
//! sent none for, as a workspace holding the same files gives it (`workspace::ingest::ecma`).

use super::SnapshotContext;
use crate::code_rule_dsl::model::Snapshot;
use crate::code_rule_dsl::module_graph;
use crate::code_rule_dsl::package_manifest::{Manifest, ignore_file_directory, manifest_directory, manifest_in};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

/// Derives the snapshot's ECMAScript context (`eval::DeriveContext`).
pub(crate) fn derive(snapshot: &mut Snapshot, files: Vec<usize>, server_graph: bool) -> Box<dyn SnapshotContext> {
    Box::new(DerivedPackages::new(snapshot, files, server_graph))
}

impl SnapshotContext for DerivedPackages {
    fn edited(&mut self, snapshot: &mut Snapshot, index: usize) {
        self.edited_again(snapshot, index);
    }
}

#[cfg(test)]
thread_local! {
    /// Package manifests `DerivedPackages` derived on this thread.
    pub(crate) static DERIVED_MANIFESTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The `package` a snapshot gives each of its ECMA files without a host one: the nearest
/// `package.json` entry it holds, read as on disk (`Manifest::parse`, `manifest_in`), with the
/// `jsxImportSource` of the package's `tsconfig.json` or `jsconfig.json` entry, the `.npmignore`
/// and `.gitignore` entries in its folder, and with its package's server graph over the
/// snapshot's files where `server_graph` asks for it (`module_graph::in_file_set`). It keeps what
/// it read, so an editor's edit derives again only the packages the edit changes (`edited`), as a
/// fresh snapshot would derive them.
pub(crate) struct DerivedPackages {
    /// The indices of the files whose `package` the snapshot derives.
    files: Vec<usize>,
    /// Whether a derived manifest naming `next` carries its package's server graph.
    server_graph: bool,
    /// Each `package.json` entry as parsed, with its package's configuration, by directory.
    packages: HashMap<String, (Manifest, module_graph::Config)>,
    /// Each package's manifest as its files carry it, by directory.
    manifests: HashMap<String, Arc<Manifest>>,
    /// What server graphs read of the snapshot's files below a package naming `next`, by path.
    summaries: HashMap<String, Option<module_graph::Summary>>,
}

impl DerivedPackages {
    /// Derives the `package` of the snapshot's files at `files`; none where it has none to derive.
    pub(crate) fn new(snapshot: &mut Snapshot, files: Vec<usize>, server_graph: bool) -> Self {
        let mut derived = Self {
            files,
            server_graph,
            packages: HashMap::new(),
            manifests: HashMap::new(),
            summaries: HashMap::new(),
        };
        if derived.files.is_empty() {
            return derived;
        }
        let texts = config_texts(snapshot);
        let read = |path: &str| texts.get(path).map(|text| (*text).to_owned());
        for file in &snapshot.files {
            if let Some(directory) = manifest_directory(&file.path) {
                let manifest = Manifest::parse(directory.to_owned(), file.source.as_bytes());
                let config = module_graph::Config::read(directory, &read);
                derived
                    .packages
                    .insert(directory.to_owned(), (manifest, config));
            }
        }
        let directories = derived.packages.keys().cloned().collect();
        derived.derive(snapshot, directories);
        derived
    }

    /// Derives again what an edit of the file at `index` changes: its package's manifest, where
    /// it is a `package.json` entry or an ignore file beside one; the packages whose
    /// configuration a project configuration changes (its own, or one it `extends`); and, where a
    /// server graph reads the source, its package's graph, only where the edit changes the
    /// source's summary.
    fn edited_again(&mut self, snapshot: &mut Snapshot, index: usize) {
        if self.files.is_empty() {
            return;
        }
        let file = &snapshot.files[index];
        let mut changed = BTreeSet::new();
        if let Some(directory) = manifest_directory(&file.path) {
            let parsed = Manifest::parse(directory.to_owned(), file.source.as_bytes());
            if let Some((manifest, _)) = self.packages.get_mut(directory) {
                *manifest = parsed;
            }
            changed.insert(directory.to_owned());
        }
        if let Some(directory) = ignore_file_directory(&file.path)
            && self.packages.contains_key(directory)
        {
            changed.insert(directory.to_owned());
        }
        if module_graph::is_module_config(&file.path) {
            let texts = config_texts(snapshot);
            let read = |path: &str| texts.get(path).map(|text| (*text).to_owned());
            for (directory, (_, config)) in &mut self.packages {
                let read_again = module_graph::Config::read(directory, &read);
                if read_again != *config {
                    *config = read_again;
                    changed.insert(directory.clone());
                }
            }
        }
        // Only sources below a package naming `next` have a summary, and only the nearest
        // package's graph reads one (`module_graph::from_summaries`).
        if let Some(summary) = self.summaries.get_mut(&file.path) {
            let again = module_graph::summarize_bounded(&file.path, &file.source);
            if again != *summary {
                *summary = again;
                changed.extend(
                    manifest_in(&file.path, |directory| self.manifests.get(directory))
                        .as_ref()
                        .and_then(crate::code_rule_dsl::package_manifest::of_file)
                        .filter(|manifest| manifest.has_server_graph())
                        .map(|manifest| manifest.folder().to_owned()),
                );
            }
        }
        if !changed.is_empty() {
            self.derive(snapshot, changed);
        }
    }

    /// Derives the manifests of the packages in `directories` again, then gives each derived file
    /// its package's.
    fn derive(&mut self, snapshot: &mut Snapshot, directories: BTreeSet<String>) {
        let Self {
            files,
            server_graph,
            packages,
            manifests,
            summaries,
        } = self;
        let ignore_files = ignore_texts(snapshot);
        for directory in directories {
            #[cfg(test)]
            DERIVED_MANIFESTS.set(DERIVED_MANIFESTS.get() + 1);
            let (parsed, config) = &packages[&directory];
            let (npmignore, gitignore) = ignore_files
                .get(directory.as_str())
                .copied()
                .unwrap_or_default();
            let manifest = parsed
                .clone()
                .with_ignore_files(npmignore.map(str::to_owned), gitignore.map(str::to_owned))
                .with_jsx_import_source(config.jsx_import_source());
            let manifest = if *server_graph && manifest.has_server_graph() {
                let members = module_graph::in_file_set(
                    &directory,
                    snapshot
                        .files
                        .iter()
                        .map(|file| (file.path.as_str(), file.source.as_str())),
                    summaries,
                    &|directory| packages.contains_key(directory),
                    config,
                );
                manifest.with_server_graph(members)
            } else {
                manifest
            };
            manifests.insert(directory, Arc::new(manifest));
        }
        for &index in files.iter() {
            let file = &mut snapshot.files[index];
            file.package = manifest_in(&file.path, |directory| manifests.get(directory));
        }
    }
}

/// The snapshot's project configuration entries' texts, by path.
fn config_texts(snapshot: &Snapshot) -> HashMap<&str, &str> {
    snapshot
        .files
        .iter()
        .filter(|file| module_graph::is_module_config(&file.path))
        .map(|file| (file.path.as_str(), file.source.as_str()))
        .collect()
}

/// The texts of the snapshot's `.npmignore` and `.gitignore` entries, by the folder they sit in.
fn ignore_texts(snapshot: &Snapshot) -> HashMap<&str, (Option<&str>, Option<&str>)> {
    let mut texts: HashMap<&str, (Option<&str>, Option<&str>)> = HashMap::new();
    for file in &snapshot.files {
        if let Some(directory) = ignore_file_directory(&file.path) {
            let entry = texts.entry(directory).or_default();
            if file.path.ends_with(crate::code_rule_dsl::package_manifest::NPMIGNORE) {
                entry.0 = Some(file.source.as_str());
            } else {
                entry.1 = Some(file.source.as_str());
            }
        }
    }
    texts
}

