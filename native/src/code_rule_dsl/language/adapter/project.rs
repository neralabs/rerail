//! A workspace's project context, as a language family keeps it besides its files (ECMAScript's `package.json`
//! files, project configurations and server graphs): the traits a family implements (`RepositoryContext`,
//! `ProjectState`) and registers at load, what they read of the workspace's files (`ProjectFiles`, which the
//! workspace implements) and a request's files as sent (`WireFile`). Below the workspace, so a family's context
//! names no workspace type (`workspace::ingest` re-exports them).

use crate::code_rule_dsl::file_input::FileInput;
use napi::Result;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

/// What a project context reads of the workspace's files: whether a code rule reads server graphs, the files below
/// a directory and one file's input, each file resident or evicted.
pub trait ProjectFiles {
    /// Whether one of the workspace's code rules reads a server graph.
    fn reads_server_graph(&self) -> bool;
    /// The files below `directory` (every file where it is empty), with their inputs: the resident files in path
    /// order, then the evicted ones.
    fn below<'a>(&'a self, directory: &'a str) -> Box<dyn Iterator<Item = (&'a String, &'a FileInput)> + 'a>;
    /// The input of the file at `path`, resident or evicted.
    fn input(&self, path: &str) -> Option<&FileInput>;
}

/// A repository's project context as its index reads it on disk (ECMAScript's `package.json` files below the
/// workspace's root): the context of each file of a family that reads one, and what a clean tree's files inherit.
pub trait RepositoryContext: Send {
    /// The context of the repository file at `file` (absolute).
    fn of_file(
        &mut self,
        file: &std::path::Path,
    ) -> Option<crate::code_rule_dsl::language::FileManifest>;
    /// The context a file directly in `directory` (absolute) inherits.
    fn of_directory(
        &mut self,
        directory: &std::path::Path,
    ) -> Option<crate::code_rule_dsl::language::FileManifest>;
}

/// What makes a repository's context: below `root`, with server graphs where `server_graph`.
pub type MakeRepositoryContext = fn(std::path::PathBuf, bool) -> Box<dyn RepositoryContext>;

static REPOSITORY_CONTEXT: std::sync::OnceLock<MakeRepositoryContext> = std::sync::OnceLock::new();

/// Registers what makes a repository's context (once, at load).
pub fn install_repository_context(make: MakeRepositoryContext) {
    let _ = REPOSITORY_CONTEXT.set(make);
}

/// No repository context: a family set that reads none.
struct NoRepositoryContext;

impl RepositoryContext for NoRepositoryContext {
    fn of_file(
        &mut self,
        _: &std::path::Path,
    ) -> Option<crate::code_rule_dsl::language::FileManifest> {
        None
    }
    fn of_directory(
        &mut self,
        _: &std::path::Path,
    ) -> Option<crate::code_rule_dsl::language::FileManifest> {
        None
    }
}

/// The repository's context below `root` (`RepositoryContext`).
pub fn repository_context(
    root: std::path::PathBuf,
    server_graph: bool,
) -> Box<dyn RepositoryContext> {
    match REPOSITORY_CONTEXT.get() {
        Some(make) => make(root, server_graph),
        None => Box::new(NoRepositoryContext),
    }
}

/// A file a request's context changes moved to another context: its path, its input, and the context it reads now.
pub type StaleContext = (
    String,
    FileInput,
    Option<crate::code_rule_dsl::language::FileManifest>,
);

/// A workspace's project context of a language family's files besides the files themselves (ECMAScript's
/// manifests, project configurations and server graphs), from which the workspace derives the context of a file
/// the host sent none for. Registered at load (`install_project`); without one there is none.
pub trait ProjectState: Send + Sync {
    /// The paths of the request's `files` whose context the host left to the workspace.
    fn unhosted(&self, files: &[WireFile]) -> HashSet<String>;
    /// Whether the file at `path` is one the context reads (a manifest, an ignore file).
    fn is_context_file(&self, path: &str) -> bool;
    /// The request's changes of the context (`inputs` after it, `removed` the paths it removes), applied by
    /// `apply` once nothing after can fail; gives `unhosted` inputs their context, and the files outside the
    /// request whose context it changes, with the context they read now.
    fn refresh(
        &self,
        state: &dyn ProjectFiles,
        inputs: &mut Vec<FileInput>,
        unhosted: &HashSet<String>,
        removed: &[String],
    ) -> (Vec<StaleContext>, Box<dyn std::any::Any + Send>);
    /// Holds the text of a file a request sent (a project configuration, an ignore file), as its context reads it.
    fn hold(&mut self, path: &str, source: &Arc<String>);
    /// Stops holding the text of a removed path.
    fn release(&mut self, path: &str);
    /// Applies the changes `refresh` gave.
    fn apply(&mut self, changes: Box<dyn std::any::Any + Send>);
    /// The context as its own type (tests read it).
    fn as_any(&self) -> &dyn std::any::Any;
}

/// No project context: a workspace of no family that derives one.
struct NoProject;

impl ProjectState for NoProject {
    fn unhosted(&self, _: &[WireFile]) -> HashSet<String> {
        HashSet::new()
    }
    fn is_context_file(&self, _: &str) -> bool {
        false
    }
    fn refresh(
        &self,
        _: &dyn ProjectFiles,
        _: &mut Vec<FileInput>,
        _: &HashSet<String>,
        _: &[String],
    ) -> (Vec<StaleContext>, Box<dyn std::any::Any + Send>) {
        (Vec::new(), Box::new(()))
    }
    fn hold(&mut self, _: &str, _: &Arc<String>) {}
    fn release(&mut self, _: &str) {}
    fn apply(&mut self, _: Box<dyn std::any::Any + Send>) {}
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

static PROJECT: std::sync::OnceLock<fn() -> Box<dyn ProjectState>> = std::sync::OnceLock::new();

/// Registers what makes a workspace's project context (once, at load).
pub fn install_project(make: fn() -> Box<dyn ProjectState>) {
    let _ = PROJECT.set(make);
}

impl Default for Box<dyn ProjectState> {
    fn default() -> Self {
        match PROJECT.get() {
            Some(make) => make(),
            None => Box::new(NoProject),
        }
    }
}

#[derive(Deserialize)]
pub struct WireFile {
    pub path: String,
    pub language: String,
    pub source: String,
    /// A Python file's module path, dotted or as a list of names (see `FileInput::new`).
    #[serde(default)]
    pub module: Option<Value>,
    /// An ECMA file's nearest `package.json` (see `FileInput::new`); without it the workspace's
    /// `package.json` entries decide (`refresh_manifest_context`).
    #[serde(
        default,
        deserialize_with = "crate::code_rule_dsl::language::deserialize_manifest"
    )]
    pub package: Option<crate::code_rule_dsl::language::FileManifest>,
}
impl WireFile {
    pub fn into_input(self, index: usize) -> Result<FileInput> {
        FileInput::new(
            self.path,
            self.language,
            Arc::new(self.source),
            self.module.as_ref(),
            self.package,
            &format!("files/{index}"),
        )
    }
}

pub fn path_is_below(path: &str, directory: &str) -> bool {
    directory.is_empty()
        || path
            .strip_prefix(directory)
            .is_some_and(|suffix| suffix.starts_with('/'))
}
