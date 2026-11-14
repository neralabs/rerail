//! The reference graph's base relations, bound in the cross-file part for a program that reads them:
//!
//! ```text
//! reference_edge(src: node, dst: node, via: symbol)   reference_open(n: node)
//! ```
//!
//! W's Python reference graph (`calls { graph: references }` and a relational clause's `references`), as W's
//! stateless evaluation builds it (`eval::stateless_calls`): each in-scope file summarized with its module from the
//! corpus's package markers (`calls::summarize`), the summaries linked by name under the configured sibling links and
//! constructor fields (`closure::Graph::link_references`). `src` and `dst` are `function_definition` or
//! `class_definition` nodes, in any files; `via` is what made the edge (`closure::reference_via`, or `sibling`), one
//! row per way. `reference_open` holds the definitions whose file was not fully summarized or that refer to something
//! no static name denotes (`Graph::open_defs`), as W's `calls` terms read unknown.
//!
//! The program's `.references` setting (the converter's, from the policy) configures the graph: a JSON object with
//! W's `siblingLinks` and `constructorFields` and the policy's `scope` (`include`, `exclude`), each optional. Without
//! it the graph has neither and is over every file.

use super::{Error, Result};
use crate::code_rule_dsl::calls::{self, closure::Graph, contracts, summary::Summary};
use crate::code_rule_dsl::facts::core::Facts;
use crate::code_rule_dsl::facts::vtab::syntax;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::logic::front::Program;
use crate::code_rule_dsl::model::{CodeRuleScope, FileInput, package_marker_directory, python_module_in};
use crate::content_store::ContentStore;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// The relations, and their columns (`sql::select::input_columns`).
pub(in crate::code_rule_dsl) use super::REFERENCE_RELATIONS as RELATIONS;

/// The edge labels `reference_edge`'s `via` takes.
const VIAS: [&str; 9] = ["param_type", "field", "bare", "unqualified", "self", "super", "late", "name", "sibling"];

/// What the graph is built with: the sibling links and constructor fields (W's `contracts::Key`), and the files it is
/// over (W's code rule scope; `None`: none).
struct Config {
    key: contracts::Key,
    scope: Option<CodeRuleScope>,
}

impl Config {
    /// `prog`'s `.references` setting (module doc).
    fn of(prog: &Program) -> Result<Self> {
        let mut settings = prog.settings.iter().filter(|(key, _)| key == "references");
        let Some((_, text)) = settings.next() else {
            return Ok(Config { key: 0, scope: Some(scope(None)?) });
        };
        if settings.next().is_some() {
            return Err(Error::Plan("more than one .references setting".into()));
        }
        let bad = |m: String| Error::Plan(format!(".references: {m}"));
        let value: Value = serde_json::from_str(text.trim()).map_err(|e| bad(e.to_string()))?;
        let object = value.as_object().ok_or_else(|| bad("not a JSON object".into()))?;
        if let Some(key) = object.keys().find(|k| !["siblingLinks", "constructorFields", "scope"].contains(&k.as_str())) {
            return Err(bad(format!("unknown key {key}")));
        }
        let key = contracts::compile(object.get("siblingLinks"), object.get("constructorFields"), "/references", usize::MAX).map_err(|e| bad(e.reason.clone()))?;
        // A scope including nothing (an event's empty scope) holds no file; W refuses to compile one.
        let empty = object.get("scope").and_then(|s| s.get("include")).and_then(Value::as_array).is_some_and(Vec::is_empty);
        Ok(Config { key, scope: if empty { None } else { Some(scope(object.get("scope"))?) } })
    }
}

fn scope(value: Option<&Value>) -> Result<CodeRuleScope> {
    crate::code_rule_dsl::compile::compile_scope(value).map_err(|e| Error::Plan(format!(".references scope: {}", e.reason)))
}

/// One summarized file: its path, summary, and its definitions' nodes by byte range (`syntax::definitions`).
type File = (Arc<str>, Arc<Summary>, HashMap<(u32, u32), i64>);

/// The corpus's files as the graph reads them, gathered while the chunks are built (`add`, from the chunk builder's
/// jobs, in any order), linked once they all are (`relations`), in path order.
pub(in crate::code_rule_dsl) struct Files {
    config: Config,
    /// The directories the corpus marks as packages (`__init__.py`), from which each file's module follows.
    packages: HashSet<String>,
    files: Mutex<Vec<File>>,
}

impl Files {
    /// For `prog`, over the corpus's `paths`; `None` where `prog` reads neither relation.
    pub(in crate::code_rule_dsl) fn new(prog: &Program, read: &BTreeSet<String>, paths: &[String]) -> Result<Option<Self>> {
        if !RELATIONS.iter().any(|r| read.contains(*r)) {
            return Ok(None);
        }
        let packages = paths.iter().filter_map(|p| package_marker_directory(p)).map(str::to_owned).collect();
        Ok(Some(Files { config: Config::of(prog)?, packages, files: Mutex::new(Vec::new()) }))
    }

    /// Document `key` at `path`, with its source and core facts: summarized as W's stateless evaluation summarizes an
    /// in-scope Python file, its module named by the corpus's package markers (`eval::parse_snapshot`).
    fn add_file(&self, key: u32, path: &str, source: &Arc<String>, facts: &Arc<Facts>) -> Result<()> {
        if !self.config.scope.as_ref().is_some_and(|scope| scope.matches(path)) {
            return Ok(());
        }
        let module = (!self.packages.is_empty()).then(|| python_module_in(path, |directory| self.packages.contains(directory))).flatten();
        let file = FileInput::new(path.to_owned(), "python".to_owned(), Arc::clone(source), module.as_ref(), None, "references").map_err(|e| Error::Plan(format!("{path}: {}", e.reason)))?;
        let root = crate::code_rule_dsl::language::adapter::ast_grep(GrammarDialect::Python, &file.source);
        let has_errors = root.root().get_doc().tree.root_node().has_error();
        let (summary, _) = calls::summarize(root.root(), GrammarDialect::Python, path, file.module.as_deref(), has_errors, file.source.len(), None);
        let nodes = syntax::definitions(key, facts).map_err(|e| Error::Plan(format!("{path}: {e}")))?;
        self.files.lock().expect("reference files").push((Arc::from(path), Arc::new(summary), nodes));
        Ok(())
    }

    /// The relations the program reads, as (name, columns, values): the summaries linked as W links them for the
    /// configured key (`CallsView::build_modes`), each edge and open definition at its node, `via` as an atom.
    #[allow(clippy::type_complexity)]
    fn relations_of(self, store: &ContentStore, read: &BTreeSet<String>) -> Result<Vec<(&'static str, Vec<String>, Vec<Option<Vec<i64>>>)>> {
        let key = self.config.key;
        let mut files = self.files.into_inner().expect("reference files");
        files.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        let graph = Graph::link_references(key, files.iter().map(|(path, summary, _)| (Arc::clone(path), Arc::new(summary.as_references(key)))).collect());
        let nodes: HashMap<&str, &HashMap<(u32, u32), i64>> = files.iter().map(|(path, _, nodes)| (path.as_ref(), nodes)).collect();
        let node = |path: &str, def: calls::summary::Def| {
            nodes.get(path).and_then(|n| n.get(&(def.start, def.end))).copied().ok_or_else(|| Error::Plan(format!("{path}: no definition node at {}..{}", def.start, def.end)))
        };
        let atoms = store.intern_atoms(&VIAS).map_err(|e| Error::Plan(e.to_string()))?;
        let atom = |via: &str| VIAS.iter().position(|v| *v == via).map(|i| i64::from(atoms[i])).ok_or_else(|| Error::Plan(format!("reference_edge: no label {via}")));
        let mut out = Vec::new();
        if read.contains("reference_edge") {
            let mut rows = graph
                .labelled_reference_rows()
                .into_iter()
                .map(|(edge, via)| Ok([node(&edge.path, edge.source)?, node(&edge.target_path, edge.target)?, atom(via)?]))
                .collect::<Result<Vec<_>>>()?;
            rows.sort_unstable();
            rows.dedup();
            out.push(("reference_edge", columns(&["src", "dst", "via"]), (0..3).map(|c| Some(rows.iter().map(|r| r[c]).collect())).collect()));
        }
        if read.contains("reference_open") {
            let mut rows = graph.reference_open().into_iter().map(|(path, def)| node(&path, def)).collect::<Result<Vec<_>>>()?;
            rows.sort_unstable();
            rows.dedup();
            out.push(("reference_open", columns(&["n"]), vec![Some(rows)]));
        }
        Ok(out)
    }
}

fn columns(names: &[&str]) -> Vec<String> {
    names.iter().map(|c| (*c).to_owned()).collect()
}

/// Python's reference graph, made where a program of Python files reads it (`corpus::install_cross_file`).
pub(in crate::code_rule_dsl) fn make(label: &str, prog: &Program, read: &BTreeSet<String>, paths: &[String]) -> Result<Option<Box<dyn super::CrossFileRelations>>> {
    if label != "python" {
        return Ok(None);
    }
    Ok(Files::new(prog, read, paths)?.map(|files| Box::new(files) as Box<dyn super::CrossFileRelations>))
}

impl super::CrossFileRelations for Files {
    fn add(&self, key: u32, path: &str, source: &Arc<String>, facts: &Arc<Facts>) -> Result<()> {
        self.add_file(key, path, source, facts)
    }
    fn relations(self: Box<Self>, store: &ContentStore, read: &BTreeSet<String>) -> Result<Vec<(&'static str, Vec<String>, Vec<Option<Vec<i64>>>)>> {
        (*self).relations_of(store, read)
    }
}
