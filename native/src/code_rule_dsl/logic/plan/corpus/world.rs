//! The file system a program reads and the documents it loads (t21 E1, E2): the generic half of typing against
//! libraries. No rule here knows a language; a language's `env.rr` and `modules.rr` say what to read.
//!
//! A program may read, as relations:
//! - `dir_entry(dir, name, is_dir)`: the entries of a directory;
//! - `file_line(path, i, text)`: the lines of a file, from 1;
//!
//! and ask for documents with `load(path, module, grammar, role)`: the file at `path`, parsed in `grammar`, as
//! module `module` (its resolution's answer, which names the document as a file of the corpus is named by its path),
//! in `role` (`library`). A loaded document's path is the file's canonical path, and `loaded(path, module, grammar,
//! role)` says which documents were loaded, by that path.
//!
//! Reading `dir_entry` or `file_line` asks for it: `rr::lower` derives `rr_dir_req(dir)` and `rr_file_req(path)` from
//! the literals that bind the key, as `parsed` asks for a guest (decision 0055). The runner answers in rounds
//! ([`discover`]): the program, cut to its requests, runs over the corpus and the documents loaded so far; the
//! directories and files asked for are read and the loads taken; and it runs again, until a round asks for nothing
//! new or [`ROUNDS`] is reached. The run proper then reads the answers as facts, over the corpus and the loaded
//! documents, and writes no rows for the loaded ones (they are not the corpus's findings).
//!
//! Only paths under the corpus root, or under a directory a setting names (`env(k, v)` with v a directory, relative
//! to the root or absolute), are read. A path elsewhere, or missing, has no entries or lines.

use super::{Error, Result, front};
use crate::code_rule_dsl::logic::rr::{WORLD, quote};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Rounds of requests: an import chain deeper than this is not followed. A guard, as `guests::DEPTH` is.
pub(super) const ROUNDS: usize = 24;
/// Documents loaded at most, unless `env("load.budget", n)` says otherwise.
const BUDGET: usize = 4000;
/// The request relations, and the relation of loads.
const LOAD: &str = "load";
const LOADED: &str = "loaded";

/// The option a run of [`discover`] gives the runs it makes: `"discover"` for a round, `"final"` for the run proper.
pub(super) const OPTION: &str = "world";

/// Whether the program reads the file system or loads documents.
pub(super) fn wanted(prog: &front::Program) -> bool {
    prog.rules.iter().any(|r| r.head.rel == LOAD || WORLD.iter().any(|(_, d)| *d == r.head.rel))
}

/// The requests a round's program answers: its outputs (those it derives).
pub(super) fn requests(prog: &front::Program) -> Vec<String> {
    WORLD.iter().map(|(_, d)| *d).chain([LOAD]).filter(|r| prog.rules.iter().any(|x| x.head.rel == *r)).map(str::to_owned).collect()
}

/// A loaded document: its path as the rule gave it, and the module it is.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Loaded {
    pub(super) path: String,
    pub(super) module: String,
    pub(super) grammar: String,
    pub(super) role: String,
}

#[derive(Default)]
pub(super) struct World {
    roots: Vec<PathBuf>,
    root: PathBuf,
    dirs: BTreeMap<String, Vec<(String, bool)>>,
    files: BTreeMap<String, Vec<String>>,
    loads: BTreeMap<String, Loaded>,
    refused: BTreeSet<String>,
    budget: usize,
    /// Loads past the budget, or in another grammar than the run's, not taken.
    pub(super) dropped: usize,
}

impl World {
    /// The roots a request may read: the corpus root, and each setting's value that is a directory.
    pub(super) fn new(root: &Path, prog: &front::Program) -> World {
        let canonical = |p: &Path| std::fs::canonicalize(p).ok();
        let mut roots: Vec<PathBuf> = canonical(root).into_iter().collect();
        let mut budget = BUDGET;
        for row in prog.facts.get("env").into_iter().flatten() {
            if let [front::Value::Str(k), front::Value::Str(v)] = row.as_slice() {
                if k == "load.budget" {
                    budget = v.parse().unwrap_or(budget);
                }
                if let Some(dir) = canonical(&root.join(v)).filter(|p| p.is_dir()) {
                    roots.push(dir);
                }
            }
        }
        World { roots, root: root.to_path_buf(), budget, ..World::default() }
    }

    /// The file `asked` names, when it lies under a root: relative to the corpus root, or absolute.
    fn allowed(&self, asked: &str) -> Option<PathBuf> {
        let path = std::fs::canonicalize(self.root.join(asked)).ok()?;
        self.roots.iter().any(|r| path.starts_with(r)).then_some(path)
    }

    /// Answers a round's requests (its `<rel>.csv` in `dir`); whether anything is new.
    pub(super) fn answer(&mut self, dir: &Path, grammar: &str, corpus: &BTreeSet<String>) -> Result<bool> {
        let lines = |rel: &str| -> Result<Vec<String>> {
            let p = dir.join(format!("{rel}.csv"));
            if !p.exists() {
                return Ok(Vec::new());
            }
            Ok(std::fs::read_to_string(&p).map_err(|e| Error::Plan(format!("{}: {e}", p.display())))?.lines().map(str::to_owned).collect())
        };
        let mut grew = false;
        for asked in lines("rr_dir_req")? {
            if self.dirs.contains_key(&asked) || self.refused.contains(&asked) {
                continue;
            }
            grew = true;
            let Some(path) = self.allowed(&asked) else {
                self.refused.insert(asked);
                continue;
            };
            let mut entries: Vec<(String, bool)> = std::fs::read_dir(&path)
                .map(|it| it.filter_map(|e| e.ok()).map(|e| (e.file_name().to_string_lossy().into_owned(), e.path().is_dir())).collect())
                .unwrap_or_default();
            entries.sort();
            self.dirs.insert(asked, entries);
        }
        for asked in lines("rr_file_req")? {
            if self.files.contains_key(&asked) || self.refused.contains(&asked) {
                continue;
            }
            grew = true;
            let text = self.allowed(&asked).and_then(|p| std::fs::read_to_string(p).ok());
            let Some(text) = text else {
                self.refused.insert(asked);
                continue;
            };
            let text = text.replace("\r\n", "\n").replace('\r', "\n");
            self.files.insert(asked, text.split_terminator('\n').map(str::to_owned).collect());
        }
        for row in lines(LOAD)? {
            let c: Vec<&str> = row.splitn(4, '\t').collect();
            let [asked, module, g, role] = c.as_slice() else { continue };
            if corpus.contains(*asked) || self.refused.contains(*asked) {
                continue;
            }
            // A loaded document is named by its canonical path: the same file is one document, and its key and memo
            // entries are the same in every workspace.
            let Some(path) = self.allowed(asked).filter(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned()) else {
                self.dropped += 1;
                self.refused.insert((*asked).to_owned());
                continue;
            };
            if self.loads.contains_key(&path) {
                continue;
            }
            if *g != grammar || self.loads.len() >= self.budget {
                self.dropped += 1;
                self.refused.insert((*asked).to_owned());
                continue;
            }
            grew = true;
            self.loads.insert(path.clone(), Loaded { path, module: (*module).to_owned(), grammar: (*g).to_owned(), role: (*role).to_owned() });
        }
        Ok(grew)
    }

    /// The answers as `.rr` facts, for the relations the program declares.
    pub(super) fn facts(&self, prog: &front::Program) -> String {
        let mut out = String::new();
        if prog.decls.contains_key("dir_entry") {
            for (dir, entries) in &self.dirs {
                for (name, is_dir) in entries {
                    out += &format!("dir_entry({}, {}, {}).\n", quote(dir), quote(name), i32::from(*is_dir));
                }
            }
        }
        if prog.decls.contains_key("file_line") {
            for (path, lines) in &self.files {
                for (i, line) in lines.iter().enumerate() {
                    out += &format!("file_line({}, {}, {}).\n", quote(path), i + 1, quote(line));
                }
            }
        }
        if prog.decls.contains_key(LOADED) {
            for l in self.loads.values() {
                out += &format!("{LOADED}({}, {}, {}, {}).\n", quote(&l.path), quote(&l.module), quote(&l.grammar), quote(&l.role));
            }
        }
        out
    }

    /// The options of a run over the answers so far: `round` is `"discover"` or `"final"`.
    pub(super) fn options(&self, base: &serde_json::Value, prog: &front::Program, round: &str) -> serde_json::Value {
        let mut o = base.as_object().cloned().unwrap_or_default();
        o.insert(OPTION.into(), round.into());
        o.insert("worldFacts".into(), self.facts(prog).into());
        o.insert("libraryFiles".into(), self.loads.values().map(|l| serde_json::json!([l.path, l.module])).collect::<Vec<_>>().into());
        serde_json::Value::Object(o)
    }

    pub(super) fn summary(&self, rounds: usize) -> String {
        let entries: usize = self.dirs.values().map(Vec::len).sum();
        let lines: usize = self.files.values().map(Vec::len).sum();
        format!(
            "{rounds} rounds of requests: {} directories ({entries} entries), {} files ({lines} lines), {} documents loaded; {} paths refused or missing, {} loads not taken",
            self.dirs.len(),
            self.files.len(),
            self.loads.len(),
            self.refused.len(),
            self.dropped
        )
    }
}

/// A document's path as the content store keys it: a loaded document's canonical path without its root.
pub(super) fn store_path(path: &str) -> &str {
    path.strip_prefix('/').unwrap_or(path)
}

/// The library documents an option names: path and module.
pub(super) fn library_files(options: &serde_json::Value) -> Vec<(String, String)> {
    options.get("libraryFiles").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|v| Some((v.get(0)?.as_str()?.to_owned(), v.get(1)?.as_str()?.to_owned()))).collect()
}
