//! ECMAScript's project context of a workspace (`ProjectState`): the `package.json` entries requests sent, the
//! project configurations and ignore files beside them, and the server graphs of the manifests that name `next`,
//! from which the workspace gives an ECMA file the host sent no `package` its nearest manifest.

use super::*;
use std::any::Any;

/// A file the request's context changes moved to another manifest: its path, its input, and the manifest it reads
/// now (`ProjectState::refresh`).
type Stale = (String, FileInput, Option<crate::code_rule_dsl::language::FileManifest>);

/// The workspace's ECMAScript project context.
#[derive(Default)]
pub(crate) struct EcmaProject {
    /// The `package.json` entries requests sent, by directory: the manifests an ECMA file the
    /// host sent no `package` for reads (`refresh_manifest_context`).
    pub(crate) manifests: BTreeMap<String, Arc<Manifest>>,
    /// The `tsconfig.json`-style configuration texts requests sent, by path: the import aliases
    /// and `jsxImportSource` of those manifests (`refresh_manifest_context`).
    pub(crate) module_configs: BTreeMap<String, Arc<String>>,
    /// The `.npmignore` and `.gitignore` texts requests sent, by path: with those manifests'
    /// `files` and `main`, which of their files npm publishes (`refresh_manifest_context`).
    pub(crate) ignore_files: BTreeMap<String, Arc<String>>,
    /// The manifest directories whose server graph the workspace derives for files the host sent
    /// no `package` for: requests that change what the graph reads refresh them
    /// (`refresh_server_graphs`).
    pub(crate) server_graph_directories: BTreeSet<String>,
    /// What server graphs read of the files requests sent without a host `package`, by path:
    /// an ECMA source's summary, or `None` for another file, which only marks its folder present.
    /// Kept whatever the code rules' scopes, since a source outside them still moves others in or
    /// out of a graph (`refresh_server_graphs`).
    pub(crate) graph_entries: BTreeMap<String, Option<module_graph::Summary>>,
}

impl ProjectState for EcmaProject {
    fn unhosted(&self, files: &[WireFile]) -> HashSet<String> {
        unhosted_manifests(files)
    }
    fn refresh(
        &self,
        state: &WorkspaceState,
        inputs: &mut Vec<FileInput>,
        unhosted: &HashSet<String>,
        removed: &[String],
    ) -> (Vec<Stale>, Box<dyn Any + Send>) {
        let (stale, changes) = refresh_manifest_context(self, state, inputs, unhosted, removed);
        (stale, Box::new(changes))
    }
    fn hold(&mut self, path: &str, source: &Arc<String>) {
        if module_graph::is_module_config(path) {
            self.module_configs.insert(path.to_owned(), Arc::clone(source));
        }
        if ignore_file_directory(path).is_some() {
            self.ignore_files.insert(path.to_owned(), Arc::clone(source));
        }
    }
    fn release(&mut self, path: &str) {
        self.module_configs.remove(path);
        self.ignore_files.remove(path);
    }
    fn apply(&mut self, changes: Box<dyn Any + Send>) {
        if let Ok(changes) = changes.downcast::<ContextChanges>() {
            changes.apply(self);
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A request's `package.json` changes, by directory: the manifest after it, or `None` where it
/// removes the one there.
type ManifestChanges = BTreeMap<String, Option<Arc<Manifest>>>;

/// A request's changes of what server graphs read (`WorkspaceState::graph_entries`), by path:
/// the entry after it, or `None` where it removes the path.
type GraphEntryChanges = BTreeMap<String, Option<Option<module_graph::Summary>>>;

/// A request's changes of the workspace's package context besides its files: its manifests,
/// what server graphs read, and the directories joining (`true`) or leaving (`false`)
/// `server_graph_directories`. `apply_request` applies them once nothing after can fail, so a
/// request that fails leaves them as they were.
#[derive(Default)]
struct ContextChanges {
    manifests: ManifestChanges,
    graph_entries: GraphEntryChanges,
    server_graph_directories: BTreeMap<String, bool>,
}

impl ContextChanges {
    fn apply(self, project: &mut EcmaProject) {
        for (directory, change) in self.manifests {
            match change {
                Some(manifest) => project.manifests.insert(directory, manifest),
                None => project.manifests.remove(&directory),
            };
        }
        for (path, entry) in self.graph_entries {
            match entry {
                Some(summary) => project.graph_entries.insert(path, summary),
                None => project.graph_entries.remove(&path),
            };
        }
        for (directory, joined) in self.server_graph_directories {
            if joined {
                project.server_graph_directories.insert(directory);
            } else {
                project.server_graph_directories.remove(&directory);
            }
        }
    }
}

/// Expand a `package.json` edit to the ECMA files whose nearest manifest it changes, and give an
/// ECMA file the host sent no `package` for the workspace's nearest one. As package markers are
/// the packages of JSON/buffer overlays, the workspace's `package.json` entries (whatever their
/// language) are their manifests, read as on disk (`Manifest::parse`) with the `jsxImportSource`
/// of the `tsconfig.json`-style entries beside them and the `.npmignore` and `.gitignore` entries
/// in their folder (a change of one is a change of the manifest). A resident file follows them
/// while its `package` equals their derivation before the request, so a host's own fact stays
/// authoritative. The changes are the workspace's once the request applies (`ContextChanges`).
fn refresh_manifest_context(
    project: &EcmaProject,
    state: &WorkspaceState,
    inputs: &mut Vec<FileInput>,
    unhosted: &HashSet<String>,
    removed: &[String],
) -> (Vec<Stale>, ContextChanges) {
    let mut changes = ManifestChanges::new();
    for path in removed {
        if let Some(directory) = manifest_directory(path)
            && project.manifests.contains_key(directory)
        {
            changes.insert(directory.to_owned(), None);
        }
    }
    let config = |directory: &str| {
        module_graph::Config::read(directory, &|path| {
            module_config_text(project, inputs, removed, path)
        })
    };
    let ignore_files = |directory: &str| {
        let text = |name: &str| {
            let path = if directory.is_empty() {
                name.to_owned()
            } else {
                format!("{directory}/{name}")
            };
            request_text(&project.ignore_files, inputs, removed, &path)
        };
        (text(NPMIGNORE), text(GITIGNORE))
    };
    for input in inputs.iter() {
        if let Some(directory) = manifest_directory(&input.path) {
            let (npmignore, gitignore) = ignore_files(directory);
            let manifest = Manifest::parse(directory.to_owned(), input.source.as_bytes())
                .with_ignore_files(npmignore, gitignore)
                .with_jsx_import_source(config(directory).jsx_import_source());
            if project
                .manifests
                .get(directory)
                .is_some_and(|current| **current == manifest)
            {
                changes.remove(directory);
            } else {
                changes.insert(directory.to_owned(), Some(Arc::new(manifest)));
            }
        }
    }
    let ignore_directories = inputs
        .iter()
        .map(|input| input.path.as_str())
        .chain(removed.iter().map(String::as_str))
        .filter_map(ignore_file_directory)
        .collect::<BTreeSet<_>>();
    for directory in ignore_directories {
        if changes.contains_key(directory) {
            continue;
        }
        let Some(current) = project.manifests.get(directory) else {
            continue;
        };
        let (npmignore, gitignore) = ignore_files(directory);
        let updated = (**current).clone().with_ignore_files(npmignore, gitignore);
        if updated != **current {
            changes.insert(directory.to_owned(), Some(Arc::new(updated)));
        }
    }
    if inputs
        .iter()
        .map(|input| input.path.as_str())
        .chain(removed.iter().map(String::as_str))
        .any(module_graph::is_module_config)
    {
        for (directory, current) in &project.manifests {
            if changes.contains_key(directory) {
                continue;
            }
            let config = config(directory);
            if current.jsx_import_source() != config.jsx_import_source() {
                let updated = (**current)
                    .clone()
                    .with_jsx_import_source(config.jsx_import_source());
                changes.insert(directory.clone(), Some(Arc::new(updated)));
            }
        }
    }
    let mut graph_entries = GraphEntryChanges::new();
    let mut server_graph_directories = BTreeMap::new();
    if state
        .code_rules
        .iter()
        .any(|code_rule| code_rule.reads_server_graph())
    {
        for path in removed {
            graph_entries.insert(path.clone(), None);
        }
        for input in inputs.iter() {
            if unhosted.contains(&input.path) {
                let summary = module_graph::summarize_bounded(&input.path, &input.source);
                graph_entries.insert(input.path.clone(), Some(summary));
            } else if !package_manifests(&input.language) {
                graph_entries.insert(input.path.clone(), Some(None));
            }
        }
        if !unhosted.is_empty() || !project.server_graph_directories.is_empty() || !changes.is_empty()
        {
            server_graph_directories = refresh_server_graphs(
                project,
                state,
                inputs,
                unhosted,
                removed,
                &graph_entries,
                &mut changes,
            );
        }
    }
    if changes.is_empty() && unhosted.is_empty() {
        let context = ContextChanges {
            manifests: changes,
            graph_entries,
            server_graph_directories,
        };
        return (Vec::new(), context);
    }
    let manifests = &project.manifests;
    let before = |directory: &str| manifests.get(directory);
    let after = |directory: &str| match changes.get(directory) {
        Some(change) => change.as_ref(),
        None => manifests.get(directory),
    };
    let requested = inputs
        .iter()
        .map(|input| input.path.clone())
        .collect::<HashSet<_>>();
    let mut paths = BTreeSet::new();
    for directory in changes.keys() {
        let start = if directory.is_empty() {
            String::new()
        } else {
            format!("{directory}/")
        };
        let resident = state.files.range(start.clone()..).map(|(path, _)| path);
        let evicted = state.evicted_inputs.range(start..).map(|(path, _)| path);
        for path in resident
            .take_while(|path| path_is_below(path, directory))
            .chain(evicted.take_while(|path| path_is_below(path, directory)))
        {
            if !requested.contains(path) && !removed.contains(path) {
                paths.insert(path.clone());
            }
        }
    }
    let mut stale = Vec::new();
    for path in paths {
        let input = match state.files.get(&path) {
            Some(file) => &file.input,
            None => &state.evicted_inputs[&path],
        };
        if !package_manifests(&input.language) || input.package != manifest_in(&path, before) {
            continue;
        }
        let package = manifest_in(&path, after);
        if package != input.package {
            stale.push((path, input.clone(), package));
        }
    }
    for input in inputs.iter_mut() {
        if unhosted.contains(&input.path) {
            input.package = manifest_in(&input.path, after);
        }
    }
    let context = ContextChanges {
        manifests: changes,
        graph_entries,
        server_graph_directories,
    };
    (stale, context)
}

/// Gives each workspace manifest naming `next` the server graph of the files the workspace holds
/// below it, the request applied (`module_graph::in_file_set`), for files it derives `package`
/// for: a directory some file of the request without a host `package` lies below joins
/// `server_graph_directories`, and a joined directory's graph is derived again whenever the
/// request changes what it reads (its manifest, a source below it, a project configuration
/// anywhere, since one may be another's `extends` base), removals included. A manifest whose
/// graph the request changes is a change of the manifest, so the files that read it are
/// evaluated again. `graph_entries` are the request's changes of what the graphs read; returns
/// the directories joining (`true`) or leaving `server_graph_directories`.
fn refresh_server_graphs(
    project: &EcmaProject,
    state: &WorkspaceState,
    inputs: &[FileInput],
    unhosted: &HashSet<String>,
    removed: &[String],
    graph_entries: &GraphEntryChanges,
    changes: &mut ManifestChanges,
) -> BTreeMap<String, bool> {
    let mut directories = BTreeMap::new();
    let after = project
        .manifests
        .iter()
        .filter(|(directory, _)| !changes.contains_key(*directory))
        .map(|(directory, manifest)| (directory.clone(), Some(Arc::clone(manifest))))
        .chain(
            changes
                .iter()
                .map(|(directory, change)| (directory.clone(), change.clone())),
        )
        .collect::<BTreeMap<_, _>>();
    let manifest_directories = after
        .iter()
        .filter(|(_, manifest)| manifest.is_some())
        .map(|(directory, _)| directory.clone())
        .collect::<HashSet<_>>();
    let requested = inputs
        .iter()
        .map(|input| input.path.as_str())
        .collect::<HashSet<_>>();
    let touched_paths = requested
        .iter()
        .copied()
        .chain(removed.iter().map(String::as_str))
        .collect::<Vec<_>>();
    let configs_changed = touched_paths
        .iter()
        .any(|path| module_graph::is_module_config(path));
    for (directory, manifest) in after {
        let Some(manifest) = manifest.filter(|manifest| manifest.has_server_graph()) else {
            if project.server_graph_directories.contains(&directory) {
                directories.insert(directory, false);
            }
            continue;
        };
        // The workspace derives the graph for files it gives `package`: ones of this request,
        // or, when the manifest changes, ones it already holds (a manifest that gains `next`
        // after its sources arrived).
        let derived = project.server_graph_directories.contains(&directory);
        let needed = derived
            || unhosted.iter().any(|path| path_is_below(path, &directory))
            || (changes.contains_key(&directory) && derives_package_below(project, state, &directory));
        // A nested manifest created or removed moves the boundary of this package's files, and any
        // file below its root `app/` may make that folder present or absent (it hides `src/app`).
        let root_app = if directory.is_empty() {
            "app".to_owned()
        } else {
            format!("{directory}/app")
        };
        // A graph not derived yet is derived; a derived one, an absent one (`None`: past a
        // bound) included, stays as it is until the request changes what it reads.
        let touched = !derived
            || changes.contains_key(&directory)
            || configs_changed
            || changes
                .keys()
                .any(|nested| nested != &directory && path_is_below(nested, &directory))
            || touched_paths.iter().any(|path| {
                path_is_below(path, &directory)
                    && (module_graph::is_source(path) || path_is_below(path, &root_app))
            });
        if !needed || !touched {
            continue;
        }
        directories.insert(directory.clone(), true);
        let start = if directory.is_empty() {
            String::new()
        } else {
            format!("{directory}/")
        };
        let config = module_graph::Config::read(&directory, &|path| {
            module_config_text(project, inputs, removed, path)
        });
        // The workspace's entries, the request's changes applied.
        let entries = project
            .graph_entries
            .range(start.clone()..)
            .take_while(|(path, _)| path_is_below(path, &directory))
            .filter(|(path, _)| !graph_entries.contains_key(*path))
            .map(|(path, summary)| (path.as_str(), summary.clone()))
            .chain(
                graph_entries
                    .range(start..)
                    .take_while(|(path, _)| path_is_below(path, &directory))
                    .filter_map(|(path, entry)| Some((path.as_str(), entry.as_ref()?.clone()))),
            );
        let members = module_graph::from_summaries(
            &directory,
            entries,
            &|folder| manifest_directories.contains(folder),
            &config,
        );
        let updated = Arc::new((*manifest).clone().with_server_graph(members));
        if project
            .manifests
            .get(&directory)
            .is_some_and(|current| **current == *updated)
        {
            changes.remove(&directory);
        } else {
            changes.insert(directory, Some(updated));
        }
    }
    directories
}

/// Whether the workspace holds a file below `directory` whose `package` it derived from its own
/// manifests (no host `package`), rather than one a host sent.
fn derives_package_below(project: &EcmaProject, state: &WorkspaceState, directory: &str) -> bool {
    let start = if directory.is_empty() {
        String::new()
    } else {
        format!("{directory}/")
    };
    let derived = |path: &String, input: &FileInput| {
        package_manifests(&input.language)
            && input.package == manifest_in(path, |folder| project.manifests.get(folder))
    };
    state
        .files
        .range(start.clone()..)
        .take_while(|(path, _)| path_is_below(path, directory))
        .any(|(path, file)| derived(path, &file.input))
        || state
            .evicted_inputs
            .range(start..)
            .take_while(|(path, _)| path_is_below(path, directory))
            .any(|(path, input)| derived(path, input))
}

/// The text of the project configuration at `path` once the request applies (`request_text`
/// over `WorkspaceState::module_configs`).
fn module_config_text(
    project: &EcmaProject,
    inputs: &[FileInput],
    removed: &[String],
    path: &str,
) -> Option<String> {
    request_text(&project.module_configs, inputs, removed, path)
}

/// The text of the file at `path` once the request applies: the request's own, else the
/// workspace's (`stored`) unless the request removes it.
fn request_text(
    stored: &BTreeMap<String, Arc<String>>,
    inputs: &[FileInput],
    removed: &[String],
    path: &str,
) -> Option<String> {
    if let Some(input) = inputs.iter().find(|input| input.path == path) {
        return Some(input.source.as_str().to_owned());
    }
    if removed.iter().any(|removed| removed == path) {
        return None;
    }
    stored.get(path).map(|text| text.as_str().to_owned())
}

/// The ECMA files of a request the host sent no `package` for (`refresh_manifest_context`).
fn unhosted_manifests(files: &[WireFile]) -> HashSet<String> {
    files
        .iter()
        .filter(|wire| wire.package.is_none() && package_manifests(&wire.language))
        .map(|wire| wire.path.clone())
        .collect()
}

