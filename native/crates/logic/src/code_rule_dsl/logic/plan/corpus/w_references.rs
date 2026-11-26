//! W's Python reference graph, for a program that reads it from W's Rust (`python/references_w.rerail`):
//!
//! ```text
//! w_reference_edge(src: node, dst: node, via: symbol)   w_reference_open(n: node)
//! ```
//!
//! The graph W (6970a3e539) built for `internal_dead_code` (its `logic::plan::corpus::references`, deleted by 0476
//! when the rules of `python/references.rerail` took its place), made again here so the two can be run side by side
//! on one program: each in-scope Python file summarized from the run's tape as its document is built
//! (`calls::summarize`; no parse of its own), the summaries linked by name once every file is built
//! (`closure::Graph::link_references`), under the program's `.references` setting. `src` and `dst` are
//! `function_definition` or `class_definition` nodes, in any files; `via` is what made the edge, one row per way.
//! `w_reference_open` holds the definitions whose file was not fully summarized or that refer to something no static
//! name denotes (`Graph::reference_open`).
//!
//! The `.references` setting is a JSON object with W's `siblingLinks` and `constructorFields` and the policy's `scope`
//! (`include`, `exclude`), each optional. Without it the graph has neither and is over every file.
//!
//! The two relations are derived facts written once, into a relation of the cross-file part (decision 0353: not a
//! copy). A run that reads them is not memoised: the graph needs every file's tape, which a memo hit does not build.

use super::{Error, Result};
use crate::code_rule_dsl::facts::core::Facts;
use crate::code_rule_dsl::facts::vtab::syntax;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::tape::TapeDoc;
use rerail_adapters::code_rule_dsl::calls::{self, closure::Graph, contracts, summary::Summary};
use rerail_adapters::code_rule_dsl::file_input::{FileInput, package_marker_directory, python_module_in};
use rerail_rules_base::code_rule_dsl::model::CodeRuleScope;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// The relations `python/references_w.rerail` declares, which the engine binds.
pub const RELATIONS: [&str; 2] = ["w_reference_edge", "w_reference_open"];

/// The edge labels `w_reference_edge`'s `via` takes.
const VIAS: [&str; 9] = ["param_type", "field", "bare", "unqualified", "self", "super", "late", "name", "sibling"];

/// A relation's columns and values, as the cross-file part binds it.
pub type Bound = (&'static str, Vec<String>, Vec<Option<rerail_blocks::vector::Vector<i64>>>);

/// What the graph is built with: the sibling links and constructor fields (W's `contracts::Key`), and the files it is
/// over (W's code rule scope; `None`: none).
struct Config {
    key: contracts::Key,
    scope: Option<CodeRuleScope>,
}

impl Config {
    /// The program's `.references` settings (module doc).
    fn of(settings: &[(String, String)]) -> Result<Self> {
        let mut settings = settings.iter().filter(|(key, _)| key == "references");
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
    rerail_rules_base::code_rule_dsl::compile::compile_scope(value).map_err(|e| Error::Plan(format!(".references scope: {}", e.reason)))
}

/// One summarized file: its path, summary, and its definitions' nodes by byte range (`syntax::definitions`).
type File = (Arc<str>, Arc<Summary>, HashMap<(u32, u32), i64>);

/// The run's Python files as the graph reads them, added as their documents are built (`add`, from the builder's
/// jobs, in any order), linked once they all are (`relations`), in path order.
pub(in crate::code_rule_dsl) struct Files {
    config: Config,
    /// The directories the corpus marks as packages (`__init__.py`), from which each file's module follows.
    packages: HashSet<String>,
    /// Whether the run's shape is core.rerail's (the program declares every relation core.rerail does): the shape its
    /// node ids are ranks in.
    core: bool,
    files: Mutex<Vec<File>>,
}

impl Files {
    /// For a program of `settings` that reads `read`, over the corpus's `paths`; `None` where it reads neither
    /// relation. `core`: whether the program is in core.rerail's shape.
    pub(in crate::code_rule_dsl) fn new(settings: &[(String, String)], read: &BTreeSet<String>, paths: &[String], core: bool) -> Result<Option<Self>> {
        if !RELATIONS.iter().any(|r| read.contains(*r)) {
            return Ok(None);
        }
        let packages = paths.iter().filter_map(|p| package_marker_directory(p)).map(str::to_owned).collect();
        Ok(Some(Files { config: Config::of(settings)?, packages, core, files: Mutex::new(Vec::new()) }))
    }

    /// Document `key` at `path`, with its source, facts and tape: summarized as W's stateless evaluation summarizes an
    /// in-scope Python file, its module named by the corpus's package markers. The summary reads the run's tape in
    /// place: no parse.
    pub(in crate::code_rule_dsl) fn add(&self, key: u32, path: &str, source: &Arc<crate::code_rule_dsl::tape::Text>, facts: &Arc<Facts>, tape: &TapeDoc) -> Result<()> {
        if !self.config.scope.as_ref().is_some_and(|scope| scope.matches(path)) {
            return Ok(());
        }
        let _span = crate::code_rule_dsl::logic::profile::span(|| "W's reference graph: a file summarized".into());
        let module = (!self.packages.is_empty()).then(|| python_module_in(path, |directory| self.packages.contains(directory))).flatten();
        let file = FileInput::new(path.to_owned(), "python".to_owned(), Arc::clone(source), module.as_ref(), None, "references").map_err(|e| Error::Plan(format!("{path}: {}", e.reason)))?;
        let root = ast_grep_core::AstGrep::doc(tape.clone());
        let (summary, _) = calls::summarize(root.root(), GrammarDialect::Python, path, file.module.as_deref(), facts.has_syntax_error(), file.source.len(), None);
        let nodes = syntax::definitions(key, facts, self.core).map_err(|e| Error::Plan(format!("{path}: {e}")))?;
        self.files.lock().expect("reference files").push((Arc::from(path), Arc::new(summary), nodes));
        Ok(())
    }

    /// The relations the program reads, as (name, columns, values): the summaries linked as W links them for the
    /// configured key, each edge and open definition at its node, `via` as an atom of the store.
    pub(in crate::code_rule_dsl) fn relations(&self, intern: &dyn Fn(&[&str]) -> Result<Vec<i64>>, read: &BTreeSet<String>) -> Result<Vec<Bound>> {
        let _span = crate::code_rule_dsl::logic::profile::span(|| "W's reference graph: the summaries linked".into());
        let key = self.config.key;
        // Taken once, by the cross-file part's binding: the builder's closures still borrow the set.
        let mut files = std::mem::take(&mut *self.files.lock().expect("reference files"));
        files.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        let graph = Graph::link_references(key, files.iter().map(|(path, summary, _)| (Arc::clone(path), Arc::new(summary.as_references(key)))).collect());
        let nodes: HashMap<&str, &HashMap<(u32, u32), i64>> = files.iter().map(|(path, _, nodes)| (path.as_ref(), nodes)).collect();
        let node = |path: &str, def: calls::summary::Def| {
            nodes.get(path).and_then(|n| n.get(&(def.start, def.end))).copied().ok_or_else(|| Error::Plan(format!("{path}: no definition node at {}..{}", def.start, def.end)))
        };
        let atoms = intern(&VIAS)?;
        let atom = |via: &str| VIAS.iter().position(|v| *v == via).map(|i| atoms[i]).ok_or_else(|| Error::Plan(format!("w_reference_edge: no label {via}")));
        let mut out = Vec::new();
        if read.contains(RELATIONS[0]) {
            let mut rows = graph
                .labelled_reference_rows()
                .into_iter()
                .map(|(edge, via)| Ok([node(&edge.path, edge.source)?, node(&edge.target_path, edge.target)?, atom(via)?]))
                .collect::<Result<Vec<_>>>()?;
            rows.sort_unstable();
            rows.dedup();
            out.push((RELATIONS[0], columns(3), (0..3).map(|c| Some(rows.iter().map(|r| r[c]).collect())).collect()));
        }
        if read.contains(RELATIONS[1]) {
            let mut rows = graph.reference_open().into_iter().map(|(path, def)| node(&path, def)).collect::<Result<Vec<_>>>()?;
            rows.sort_unstable();
            rows.dedup();
            out.push((RELATIONS[1], columns(1), vec![Some(rows.into_iter().collect())]));
        }
        Ok(out)
    }
}

/// A library input's columns, by position (`c0`, …): as the cross-file part names a relation core does not shape.
fn columns(width: usize) -> Vec<String> {
    (0..width).map(|c| format!("c{c}")).collect()
}
