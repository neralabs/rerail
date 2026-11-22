//! The module graph tests' fixtures: a file set's server graph, and a package on disk. The engine's napi tests read
//! them too.

use super::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A React hook module.
pub const HOOK: &str =
    "import { useState } from 'react';\nexport function useThing() { return useState(0); }\n";

/// A manifest that names Next.
pub const NEXT_MANIFEST: &str = r#"{ "dependencies": { "next": "15" } }"#;

/// The server graph of a file set (path, text) whose `package.json` entries are its manifests and
/// whose configuration entries the package's `Config` reads; `None` without one.
pub fn graph(folder: &str, files: &[(&str, &str)]) -> Option<Vec<String>> {
    let manifests = files
        .iter()
        .filter_map(|(path, _)| {
            path.strip_suffix("/package.json")
                .or_else(|| (*path == "package.json").then_some(""))
        })
        .collect::<HashSet<_>>();
    let read = |path: &str| {
        files
            .iter()
            .find(|(entry, _)| *entry == path)
            .map(|(_, text)| (*text).to_owned())
    };
    in_file_set(
        folder,
        files.iter().copied(),
        &mut HashMap::new(),
        &|directory| manifests.contains(directory),
        &Config::read(folder, &read),
    )
    .map(|members| members.into_iter().collect())
}

/// A fresh directory below the system's temporary folder.
pub fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rerail-module-graph-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

/// Writes `bytes` to `path` below `root`, creating its folders.
pub fn write_file(root: &Path, path: &str, bytes: impl AsRef<[u8]>) {
    let file = root.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, bytes).unwrap();
}

/// The members `on_disk` gives the package in `folder` below `root`; `None` without a graph.
pub fn disk_graph(root: &Path, folder: &str) -> Option<Vec<String>> {
    on_disk(root, folder, &config_on_disk(root, folder), None)
        .map(|members| members.into_iter().collect())
}
