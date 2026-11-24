//! A Git policy's last include graph and each compiled version use the ordinary combined-program store.
//! The graph is a hint. Verify its paths in the current view before using its program.
use super::{
    Error, LibraryFile, ReadProgram, builtin_program_resolved, library_root,
    lower_files_dropping,
};
use crate::code_rule_dsl::logic::{
    plan::corpus::{combined, phase0, phases},
    program_text,
};
use crate::code_rule_dsl::prof;
use crate::content_store::ContentStore;
use crate::git_workspace::snapshot;
use sha2::{Digest as _, Sha256};
use std::path::{Path, PathBuf};

/// A policy's program, as the engine holds every program: its version (decision 0281).
type Program = std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>;

fn stored_path(root: &Path, path: &Path) -> Option<PathBuf> {
    let path = match path.strip_prefix(root) {
        Ok(relative) => relative.to_owned(),
        Err(_) => std::path::absolute(path).ok()?,
    };
    // The combined-program fact codec stores paths as strings.
    path.to_str()?;
    Some(path)
}

fn stored_files(root: &Path, files: &[LibraryFile]) -> Option<Vec<LibraryFile>> {
    files
        .iter()
        .map(|file| {
            Some(LibraryFile {
                path: stored_path(root, &file.path)?,
                sha256: file.sha256,
            })
        })
        .collect()
}

fn graph_key(path: &Path, library: &Path, digest: &[u8; 32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"rerail/git-policy-graph/v1\0");
    hash.update(combined::rule_file_key(path, library, digest));
    hash.finalize().into()
}

/// Stable between processes. A consuming catalog can also retain the inspected graph in another store.
#[derive(Clone)]
pub struct GitPolicyInputs {
    graph: [u8; 32],
    resolutions: Vec<(PathBuf, PathBuf)>,
}

impl GitPolicyInputs {
    fn version_key(&self, files: &[LibraryFile]) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"rerail/git-policy-version/v1\0");
        hash.update(self.graph);
        for (path, target) in &self.resolutions {
            hash.update(path.as_os_str().as_encoded_bytes());
            hash.update([0]);
            hash.update(target.as_os_str().as_encoded_bytes());
            hash.update([0]);
        }
        hash.update([0]);
        for file in files {
            hash.update(file.path.as_os_str().as_encoded_bytes());
            hash.update([0, u8::from(file.sha256.is_some())]);
            if let Some(digest) = file.sha256 {
                hash.update(digest);
            }
        }
        hash.finalize().into()
    }

    fn compiled_version(&self, store: &ContentStore, files: &[LibraryFile]) -> Option<Program> {
        let key = self.version_key(files);
        store.settle(&combined::queued(&key));
        let read = combined::read_record(store, &key).ok()??;
        if read.files != files {
            return None;
        }
        program_text::load_image(store, &read.program)
            .ok()?
            .map(|(program, _)| program)
    }

    fn keep_version(&self, store: &ContentStore, files: &[LibraryFile], program: &Program) {
        let key = self.version_key(files);
        store.settle(&combined::queued(&key));
        if !combined::retains_read(store, &key, files).unwrap_or(false) {
            combined::record_version(store, "a Git policy version", &key, files, program, &[]);
        }
        combined::read_before_the_run(key);
        combined::read_before_the_run(self.graph);
    }

    fn keep(&self, store: &ContentStore, files: &[LibraryFile], program: &Program) {
        self.keep_version(store, files, program);
        store.settle(&combined::queued(&self.graph));
        let present = combined::read_record(store, &self.graph)
            .ok()
            .flatten()
            .is_some_and(|read| {
                read.files == files
                    && read.resolutions == self.resolutions
                    && program_text::available(store, &read.program).unwrap_or(false)
            });
        if !present {
            combined::record_version_resolved(
                store,
                "a Git policy include graph",
                &self.graph,
                files,
                program,
                &[],
                &self.resolutions,
            );
        }
    }

    pub fn retain(&self, root: &Path, files: &[LibraryFile], program: &Program) {
        let Some(files) = stored_files(root, files) else {
            return;
        };
        let Some(store) = phases::reading_store() else {
            return;
        };
        let Ok(_lease) = store.hold_collection() else {
            return;
        };
        self.keep(&store, &files, program);
    }
}

fn verified_digest(path: &Path) -> Result<[u8; 32], Error> {
    snapshot::read_bounded(path, 1024 * 1024)
        .map(|bytes| Sha256::digest(bytes).into())
        .map_err(|error| Error::Syntax(format!("{}: {error}", path.display())))
}

/// Absolute paths name the selected host library. Relative paths name objects in this Git view.
fn reopen(root: &Path, store: &ContentStore, graph: [u8; 32]) -> Option<ReadProgram> {
    store.settle(&combined::queued(&graph));
    let read = combined::read_record(store, &graph).ok()??;
    for (path, target) in &read.resolutions {
        if snapshot::canonicalize(root.join(path)).ok()? != root.join(target) {
            return None;
        }
    }
    let inputs = GitPolicyInputs {
        graph,
        resolutions: read.resolutions,
    };
    let mut current = read.files.clone();
    let mut files = Vec::with_capacity(current.len());
    for file in &mut current {
        let path = root.join(&file.path);
        let digest = if file.path.is_relative() {
            Some(verified_digest(&path).ok()?)
        } else {
            LibraryFile::now(&path).sha256
        };
        // A candidate that appeared or disappeared can change which include the program selects.
        if file.sha256.is_some() != digest.is_some() {
            return None;
        }
        file.sha256 = digest;
        files.push(LibraryFile {
            path,
            sha256: digest,
        });
    }
    let program = if read.files == current {
        let (program, _) = program_text::load_image(store, &read.program).ok()??;
        inputs.keep_version(store, &current, &program);
        program
    } else {
        // The same include graph can have several retained versions. Their bytes were each checked above.
        let program = inputs.compiled_version(store, &current)?;
        inputs.keep(store, &current, &program);
        program
    };
    Some(ReadProgram {
        program,
        files,
        git: Some(inputs),
    })
}

pub(super) fn read(root: &Path, path: &Path) -> Result<ReadProgram, Error> {
    let store = if crate::code_rule_dsl::logic::plan::corpus::keeps_store() && !phase0::active() {
        phases::reading_store()
    } else {
        None
    };
    let held = store
        .as_deref()
        .and_then(|store| Some((store, store.hold_collection().ok()?)));
    let store = held.as_ref().map(|(store, _lease)| *store);
    let graph = graph_key(
        path.strip_prefix(root)
            .expect("a path inside the selected Git view"),
        &library_root(),
        &verified_digest(path)?,
    );
    if let Some(store) = store
        && let Some(read) = reopen(root, store, graph)
    {
        prof::count("rr.git_policy_graph_hits", 1);
        prof::count("rr.git_policy_hits", 1);
        return Ok(read);
    }
    let mut resolutions = Vec::new();
    let (modules, files) = builtin_program_resolved(path, &mut |from, to| {
        resolutions.push((from.to_owned(), to.to_owned()))
    })?;
    let stable = stored_files(root, &files);
    let resolutions: Option<Vec<_>> = resolutions
        .iter()
        .map(|(f, t)| Some((stored_path(root, f)?, stored_path(root, t)?)))
        .collect();
    let git = resolutions.map(|resolutions| GitPolicyInputs { graph, resolutions });
    let kept = match (store, &stable, &git) {
        (Some(store), Some(files), Some(inputs)) => inputs.compiled_version(store, files),
        _ => None,
    };
    let program = match kept {
        Some(program) => {
            prof::count("rr.git_policy_hits", 1);
            program
        }
        None => {
            prof::count("rr.git_policy_compiles", 1);
            lower_files_dropping(path, modules, true)?
        }
    };
    if let (Some(store), Some(files), Some(inputs)) = (store, &stable, &git) {
        inputs.keep(store, files, &program);
    }
    Ok(ReadProgram {
        program,
        files,
        git,
    })
}
