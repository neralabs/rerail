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
use crate::code_rule_dsl::calls::{self, closure::{Graph, ReferenceFile}, contracts, summary::Summary};
use crate::code_rule_dsl::facts::core::Facts;
use crate::code_rule_dsl::facts::vtab::syntax;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
use crate::code_rule_dsl::model::{CodeRuleScope, FileInput, package_marker_directory, python_module_in};
use crate::code_rule_dsl::module_path::ModulePath;
use crate::code_rule_dsl::tape::TapeDoc;
use crate::content_store::ContentStore;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::path::Path;
use super::reference_facts;

/// The relations, and their columns (`sql::select::input_columns`).
pub(in crate::code_rule_dsl) use super::REFERENCE_RELATIONS as RELATIONS;

/// The edge labels `reference_edge`'s `via` takes.
const VIAS: [&str; 9] = ["param_type", "field", "bare", "unqualified", "self", "super", "late", "name", "sibling"];

/// What the graph is built with: the sibling links and constructor fields (W's `contracts::Key`), and the files it is
/// over (W's code rule scope; `None`: none).
struct Config {
    key: contracts::Key,
    bindings: String,
    scope: Option<CodeRuleScope>,
}

impl Config {
    /// `prog`'s `.references` setting (module doc).
    fn of(prog: &dyn ProgramFields) -> Result<Self> {
        let all = prog.settings();
        let mut settings = all.iter().filter(|(key, _)| key == "references");
        let Some((_, text)) = settings.next() else {
            return Ok(Config { key: 0, bindings: "null".to_owned(), scope: Some(scope(None)?) });
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
        let bindings = object.get("siblingLinks").unwrap_or(&Value::Null).to_string();
        Ok(Config { key, bindings, scope: if empty { None } else { Some(scope(object.get("scope"))?) } })
    }
}

fn scope(value: Option<&Value>) -> Result<CodeRuleScope> {
    crate::code_rule_dsl::compile::compile_scope(value).map_err(|e| Error::Plan(format!(".references scope: {}", e.reason)))
}

enum LocalSummary {
    Direct(Summary),
    Requested(reference_facts::Source),
}

/// One file's summary and its definitions' nodes by byte range (`syntax::definitions`).
type File = (Arc<str>, LocalSummary, HashMap<(u32, u32), i64>);

fn summarize(tape: TapeDoc, path: &str, module: Option<&ModulePath>) -> Summary {
    let has_errors = tape.syntax_error_nodes().next().is_some();
    let bytes = tape.source.len();
    let root = ast_grep_core::AstGrep::doc(tape);
    crate::code_rule_dsl::prof::count("reference.tape_summaries", 1);
    calls::summarize(root.root(), GrammarDialect::Python, path, module, has_errors, bytes, None).0
}

pub(in crate::code_rule_dsl) fn summarize_tape(tape: TapeDoc, module: &str, stub: bool) -> Result<String> {
    let module: Option<ModulePath> = serde_json::from_str(module).map_err(|err| Error::Plan(err.to_string()))?;
    let path = if stub { "input.pyi" } else { "input.py" };
    let summary = summarize(tape, path, module.as_ref());
    if crate::code_rule_dsl::budget::matcher_budget_exhausted() {
        return Err(Error::Plan("reference analysis was interrupted".into()));
    }
    let bytes = summary.encode();
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn decode_summary(hex: &str) -> Option<Summary> {
    if hex.len() % 2 != 0 { return None; }
    let bytes: Vec<u8> = hex.as_bytes().chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect::<Option<_>>()?;
    Summary::decode(&bytes, crate::code_rule_dsl::language::Family::Python)
}

/// The corpus's files as the graph reads them, gathered while the chunks are built (`add`, from the chunk builder's
/// jobs, in any order), linked once they all are (`relations`), in path order.
pub(in crate::code_rule_dsl) struct Files {
    config: Config,
    /// The directories the corpus marks as packages (`__init__.py`), from which each file's module follows.
    packages: HashSet<String>,
    /// Whether the run's shape is core.rerail's (the program declares every relation core.rerail does, `Projection::of`):
    /// the shape its node ids are ranks in.
    core: bool,
    files: Mutex<Vec<File>>,
    retained: bool,
}

impl Files {
    /// For `prog`, over the corpus's `paths`; `None` where `prog` reads neither relation.
    pub(in crate::code_rule_dsl) fn new(prog: &dyn ProgramFields, read: &BTreeSet<String>, paths: &[String], markers: Option<&HashMap<String, bool>>, retained: bool) -> Result<Option<Self>> {
        if !RELATIONS.iter().any(|r| read.contains(*r)) {
            return Ok(None);
        }
        let packages = match markers {
            Some(markers) => markers.iter().filter(|(_, present)| **present).map(|(path, _)| path.clone()).collect(),
            None => paths.iter().filter_map(|p| package_marker_directory(p)).map(str::to_owned).collect(),
        };
        let names = crate::code_rule_dsl::facts::core::core_names();
        let core = !names.is_empty() && names.iter().all(|r| prog.declares(r.as_str()));
        Ok(Some(Files { config: Config::of(prog)?, packages, core, files: Mutex::new(Vec::new()), retained }))
    }

    /// Document `key` at `path`, with its content digest, tape and core facts: summarized as W's stateless evaluation
    /// summarizes an in-scope Python file, its module named by the corpus's package markers (`eval::parse_snapshot`).
    /// The summary reads the run's tape in place, as W's workspace reads a restored one (`calls_state`): no parse
    /// (Q51); a retained graph (`reference_facts`) asks for the summary by the file's digest instead.
    fn add_file(&self, key: u32, path: &str, digest: [u8; 32], tape: Option<&TapeDoc>, facts: &Arc<Facts>) -> Result<()> {
        if !self.config.scope.as_ref().is_some_and(|scope| scope.matches(path)) {
            return Ok(());
        }
        let module = (!self.packages.is_empty()).then(|| python_module_in(path, |directory| self.packages.contains(directory))).flatten();
        let file = FileInput::new(path.to_owned(), "python".to_owned(), Arc::clone(&facts.source), module.as_ref(), None, "references").map_err(|e| Error::Plan(format!("{path}: {}", e.reason)))?;
        let summary = if self.retained {
            LocalSummary::Requested(reference_facts::Source {
                path: path.to_owned(), digest: crate::content_store::observe::hex(&digest),
                module: serde_json::to_string(&file.module.as_deref()).map_err(|err| Error::Plan(err.to_string()))?,
                stub: crate::code_rule_dsl::language::is_stub_file(path), extractor: calls::summary::EXTRACTOR_VERSION,
            })
        } else {
            LocalSummary::Direct(summarize(tape.expect("direct reference analysis requests tapes").clone(), path, file.module.as_deref()))
        };
        let nodes = syntax::definitions(key, facts, self.core).map_err(|e| Error::Plan(format!("{path}: {e}")))?;
        self.files.lock().expect("reference files").push((Arc::from(path), summary, nodes));
        Ok(())
    }

    /// The relations the program reads, as (name, columns, values): the summaries linked as W links them for the
    /// configured key (`CallsView::build_modes`), each edge and open definition at its node, `via` as an atom.
    #[allow(clippy::type_complexity)]
    fn relations_of(self, root: &Path, store: &ContentStore, read: &BTreeSet<String>) -> Result<Vec<(&'static str, Vec<String>, Vec<Option<Vec<i64>>>)>> {
        let key = self.config.key;
        let mut files = self.files.into_inner().expect("reference files");
        files.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        let graph = if self.retained {
            let sources: Vec<_> = files.iter().filter_map(|(_, summary, _)| match summary {
                LocalSummary::Requested(source) => Some(source.clone()),
                LocalSummary::Direct(_) => None,
            }).collect();
            let prepared = reference_facts::run(root, &sources, &reference_facts::Preparation {
                settings: &self.config.bindings,
                siblings: &|bytes| {
                    let summary = decode_summary(bytes).ok_or_else(|| Error::Plan("invalid reference summary".into()))?;
                    serde_json::to_string(&summary.sibling_targets(key)).map_err(|error| Error::Plan(error.to_string()))
                },
                prepare: &|path, bytes, targets| {
                    let summary = decode_summary(bytes)
                        .ok_or_else(|| Error::Plan(format!("{path}: invalid reference summary")))?;
                    let siblings = serde_json::from_str(targets).map_err(|error| Error::Plan(format!("{path}: invalid sibling targets: {error}")))?;
                    Ok(ReferenceFile::prepare(path, &summary, siblings))
                },
            })?;
            let prepared = files.iter().map(|(path, _, _)| {
                let file = prepared.get(path.as_ref()).and_then(|value| ReferenceFile::decode(value))
                    .ok_or_else(|| Error::Plan(format!("{path}: missing or invalid reference file facts")))?;
                Ok((Arc::clone(path), file))
            }).collect::<Result<_>>()?;
            Graph::link_references_with(key, prepared, |graph, names| {
                let links = reference_facts::link(root, names.iter().map(|name| name.text()), &reference_facts::LinkProvider {
                    resolve: &|name| graph.resolve_reference_link(name),
                    is_current: &|answer| graph.reference_link_is_current(answer),
                })?;
                names.iter().map(|name| {
                    links.get(name.text()).and_then(|answer| graph.restore_reference_link(answer))
                        .ok_or_else(|| Error::Cache(crate::content_store::StoreError::Invalid(
                            format!("invalid reference link result for {}", name.text()),
                        )))
                }).collect()
            })?
        } else {
            let summaries = files.iter().map(|(path, summary, _)| {
                let LocalSummary::Direct(summary) = summary else {
                    return Err(Error::Plan(format!("{path}: missing direct reference summary")));
                };
                Ok((Arc::clone(path), Arc::new(summary.as_references(key))))
            }).collect::<Result<_>>()?;
            Graph::link_references(key, summaries)
        };
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
pub(in crate::code_rule_dsl) fn make(label: &str, prog: &dyn ProgramFields, read: &BTreeSet<String>, paths: &[String], markers: Option<&HashMap<String, bool>>, retained: bool) -> Result<Option<Box<dyn super::CrossFileRelations>>> {
    if label != "python" {
        return Ok(None);
    }
    Ok(Files::new(prog, read, paths, markers, retained)?.map(|files| Box::new(files) as Box<dyn super::CrossFileRelations>))
}

impl super::CrossFileRelations for Files {
    fn needs_tapes(&self) -> bool {
        !self.retained
    }

    fn add(&self, key: u32, path: &str, digest: [u8; 32], tape: Option<&TapeDoc>, facts: &Arc<Facts>) -> Result<()> {
        self.add_file(key, path, digest, tape, facts)
    }
    fn relations(self: Box<Self>, root: &Path, store: &ContentStore, read: &BTreeSet<String>) -> Result<Vec<(&'static str, Vec<String>, Vec<Option<Vec<i64>>>)>> {
        (*self).relations_of(root, store, read)
    }
}
