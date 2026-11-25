//! SPIKE (throw-away, branch spike-w-refs): W's reference graph in Rust, in place of `python/references.rerail`'s
//! strata, to measure whether the strata's execution shape is what makes the head slower than W.
//!
//! `RERAIL_SPIKE_W_REFS=1` and a program that reads `reference_edge` or `reference_open` as inputs (a library whose
//! `core.rerail` declares them `.input`, a program that does not include `python/references.rerail`): each Python file
//! is summarized by W's `calls::summarize` as its facts are built (the run's tape, read in place), and the summaries
//! are linked by W's `Graph::link_references` before the cross-file part, whose base relations they become. This is
//! W's deleted `corpus::references` (6970a3e539), with no `.references` setting (no sibling links or constructor
//! fields, every Python file in scope), as the benchmark program has none.

use super::{Error, Result};
use crate::code_rule_dsl::facts::core::Facts;
use crate::code_rule_dsl::facts::vtab::syntax;
use crate::code_rule_dsl::tape::TapeDoc;
use rerail_adapters::code_rule_dsl::calls::{self, closure::Graph, summary::Summary};
use rerail_adapters::code_rule_dsl::file_input::{FileInput, package_marker_directory, python_module_in};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

/// The relations, and their columns.
pub(crate) const RELATIONS: [&str; 2] = ["reference_edge", "reference_open"];

/// The edge labels `reference_edge`'s `via` takes.
const VIAS: [&str; 9] = ["param_type", "field", "bare", "unqualified", "self", "super", "late", "name", "sibling"];

/// Whether the spike is on.
pub(crate) fn on() -> bool {
    std::env::var_os("RERAIL_SPIKE_W_REFS").is_some()
}

/// One summarized file: its path, summary, and its definitions' nodes by byte range.
type File = (Arc<str>, Arc<Summary>, HashMap<(u32, u32), i64>);

pub(crate) struct Files {
    packages: HashSet<String>,
    core: bool,
    files: parking_lot::Mutex<Vec<File>>,
}

impl Files {
    /// For a program reading `read`, over the run's `paths`; `None` where the spike is off or the program reads
    /// neither relation.
    pub(crate) fn new<'a>(read: &BTreeSet<String>, paths: impl Iterator<Item = &'a str>, core: bool) -> Option<Self> {
        if !on() || !RELATIONS.iter().any(|r| read.contains(*r)) {
            return None;
        }
        let packages = paths.filter_map(package_marker_directory).map(str::to_owned).collect();
        Some(Files { packages, core, files: parking_lot::Mutex::new(Vec::new()) })
    }

    /// Document `key` at `path`, summarized as W's stateless evaluation summarizes a Python file.
    pub(crate) fn add(&self, key: u32, path: &str, source: &Arc<crate::code_rule_dsl::tape::Text>, facts: &Arc<Facts>, tape: &TapeDoc) -> Result<()> {
        if !(path.ends_with(".py") || path.ends_with(".pyi")) {
            return Ok(());
        }
        let _span = crate::code_rule_dsl::logic::profile::span(|| "W's reference graph".into());
        let module = (!self.packages.is_empty()).then(|| python_module_in(path, |directory| self.packages.contains(directory))).flatten();
        let file = FileInput::new(path.to_owned(), "python".to_owned(), Arc::clone(source), module.as_ref(), None, "references").map_err(|e| Error::Plan(format!("{path}: {}", e.reason)))?;
        let has_errors = facts.has_syntax_error();
        let root = ast_grep_core::AstGrep::doc(tape.clone());
        let (summary, _) = calls::summarize(root.root(), crate::code_rule_dsl::language::GrammarDialect::Python, path, file.module.as_deref(), has_errors, file.source.len(), None);
        let nodes = syntax::definitions(key, facts, self.core).map_err(|e| Error::Plan(format!("{path}: {e}")))?;
        self.files.lock().push((Arc::from(path), Arc::new(summary), nodes));
        Ok(())
    }

    /// The relations, as (name, columns, values): the summaries linked as W links them, each edge and open definition
    /// at its node, `via` as an atom.
    #[allow(clippy::type_complexity)]
    pub(crate) fn relations(&self, store: &crate::content_store::ContentStore) -> Result<Vec<(&'static str, Vec<String>, Vec<Option<rerail_blocks::vector::Vector<i64>>>)>> {
        let _span = crate::code_rule_dsl::logic::profile::span(|| "W's reference graph linked".into());
        let mut files = std::mem::take(&mut *self.files.lock());
        files.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        let graph = Graph::link_references(0, files.iter().map(|(path, summary, _)| (Arc::clone(path), Arc::new(summary.as_references(0)))).collect());
        let nodes: HashMap<&str, &HashMap<(u32, u32), i64>> = files.iter().map(|(path, _, nodes)| (path.as_ref(), nodes)).collect();
        let node = |path: &str, def: calls::summary::Def| {
            nodes.get(path).and_then(|n| n.get(&(def.start, def.end))).copied().ok_or_else(|| Error::Plan(format!("{path}: no definition node at {}..{}", def.start, def.end)))
        };
        let atoms = store.intern_atoms(&VIAS).map_err(|e| Error::Plan(e.to_string()))?;
        let atom = |via: &str| VIAS.iter().position(|v| *v == via).map(|i| i64::from(atoms[i])).ok_or_else(|| Error::Plan(format!("reference_edge: no label {via}")));
        let mut edges = graph
            .labelled_reference_rows()
            .into_iter()
            .map(|(edge, via)| Ok([node(&edge.path, edge.source)?, node(&edge.target_path, edge.target)?, atom(via)?]))
            .collect::<Result<Vec<_>>>()?;
        edges.sort_unstable();
        edges.dedup();
        let mut open = graph.reference_open().into_iter().map(|(path, def)| node(&path, def)).collect::<Result<Vec<_>>>()?;
        open.sort_unstable();
        open.dedup();
        let names = |c: &[&str]| c.iter().map(|c| (*c).to_owned()).collect::<Vec<_>>();
        Ok(vec![
            ("reference_edge", names(&["src", "dst", "via"]), (0..3).map(|c| Some(edges.iter().map(|r| r[c]).collect())).collect()),
            ("reference_open", names(&["n"]), vec![Some(open.into_iter().collect())]),
        ])
    }
}
