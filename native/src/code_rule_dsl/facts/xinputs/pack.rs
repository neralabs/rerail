//! The X6 pack's thirteen per-file inputs (`symbols-merged.json`, clause `pack`, `relational.inputs`)
//! evaluated by this crate's matcher over one tape: each input becomes a `budget` clause whose
//! query is the input's `evidence`, compiled by the production compiler with the pack's `shared`
//! define (its utils), and run by the stateless evaluator (`eval::stateless_rows_to`) with the
//! tape installed as the evaluation's tape (`TapeCacheSlot::seed`), so nothing is parsed again.
//!
//! The rows are a function of the content alone: the file is evaluated as `x.py` (cgnative's
//! reading), the rule's scope admits it, and an input's `file` predicate (`py_all`,
//! `py_all_names`: `^data_layer/[^/]+\.py$`) is taken out of its query and kept as `Input::file`,
//! which the reader applies to the real path.
use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::eval;
use crate::code_rule_dsl::model::{ClauseKind, CompiledCodeRule, FileInput, Metrics, Snapshot};
use crate::code_rule_dsl::violations::anchored_span;
use regex::Regex;
use rusqlite::types::{Value, ValueRef};
use serde_json::{Value as Json, json};
use std::sync::Arc;

/// The pack as copied (facts-engine `spikes/x6-rules/linked/symbols-merged.json`).
const PACK: &str = include_str!("symbols-merged.json");

/// The reading every content is evaluated under: no path but a Python stem.
pub(crate) const READING: &str = "x.py";

/// Limits that never cut a corpus file short (the exports had none): a file the evaluator still
/// marks incomplete fails its content.
const CAPABILITIES: &str = r#"{"ast":{"engine":"ast-grep-core","engineVersion":"0.45","languages":{"python":"tree-sitter-python@0.23"}},"format":"rerail.policy-capabilities/v1","limits":{"maxAstPatternBytes":65536,"maxFiles":1,"maxRegexPatternBytes":65536,"maxSourceBytes":64000000,"maxVisitedNodes":4000000000,"maxVisitsPerNode":64,"maxWitnesses":100000000}}"#;

/// The inputs, in the pack's order.
pub(crate) const INPUTS: [&str; 13] = [
    "py_functions",
    "py_classes",
    "py_decorators",
    "py_parameters",
    "py_parameter_annotations",
    "py_return_annotations",
    "py_imports",
    "py_bases",
    "py_all",
    "py_all_names",
    "dc__raw_module_names",
    "dc__raw_dynamic_imports",
    "ef__raw_execution_calls",
];

/// One compiled input: its clause, its projected columns (in projection order) and the path
/// predicate taken out of its query.
pub(crate) struct Input {
    pub name: &'static str,
    clause: usize,
    pub columns: Vec<String>,
    pub file: Option<Regex>,
}

/// One input's rows: the anchor span (`__start`, `__end`) and the projected values.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Table {
    pub rows: Vec<(i64, i64, Vec<Value>)>,
}

impl Table {
    /// Column `name`'s position among the projected values.
    pub fn col(input: &Input, name: &str) -> usize {
        input.columns.iter().position(|c| c == name).unwrap_or_else(|| panic!("{}: no column {name}", input.name))
    }
}

pub(crate) struct Pack {
    rule: Arc<CompiledCodeRule>,
    pub inputs: Vec<Input>,
    /// The `references` links (`siblingLinks`, `constructorFields`) the pack's reference graph
    /// reads, as canonical JSON (`cgnative::links_key`).
    pub links: String,
    /// The pack's scope (`scope.include`, `scope.exclude`), which `in_scope` reads.
    pub scope: [Vec<String>; 2],
}

impl Pack {
    pub fn compile() -> Result<Pack, String> {
        let doc: Json = serde_json::from_str(PACK).map_err(|e| format!("pack: {e}"))?;
        let clauses = doc["clauses"].as_array().ok_or("pack: clauses")?;
        let shared = clauses.iter().find(|c| c.get("define").is_some()).ok_or("pack: no define clause")?;
        let pack = clauses.iter().find(|c| c["id"] == "pack").ok_or("pack: no clause `pack`")?;
        let relational = &pack["relational"];
        let mut out = vec![shared.clone()];
        let mut files = Vec::new();
        for name in INPUTS {
            let mut input = relational["inputs"].get(name).cloned().ok_or_else(|| format!("pack: no input {name}"))?;
            let ast = &mut input["evidence"]["query"]["ast"];
            let file = match ast.as_object_mut().and_then(|a| a.remove("file")) {
                None => None,
                Some(f) => {
                    let rx = f["path"]["regex"].as_str().ok_or_else(|| format!("pack: {name}: file is not a path regex"))?;
                    Some(Regex::new(rx).map_err(|e| format!("pack: {name}: {e}"))?)
                }
            };
            files.push(file);
            out.push(json!({
                "id": name,
                "budget": {"query": input, "count": {"lte": u32::MAX}},
                "violation": {"anchor": "witness.span", "code": name, "message": name},
            }));
        }
        let policy = json!({
            "format": "rerail.policy/v1", "id": "xinputs", "revision": 1, "mode": "log",
            "scope": {"include": ["**/*"], "exclude": []},
            "clauses": out,
        });
        let compiled = CodeRuleDslRuntime::compile(policy.to_string(), "json".into(), CAPABILITIES.into()).map_err(|e| format!("pack: {}", e.reason))?;
        let rule = Arc::clone(&compiled.inner);
        let mut inputs = Vec::new();
        for (name, file) in INPUTS.into_iter().zip(files) {
            let clause = rule.clauses.iter().position(|c| c.id == name).ok_or_else(|| format!("pack: {name} not compiled"))?;
            let ClauseKind::Budget { query, .. } = &rule.clauses[clause].kind else { return Err(format!("pack: {name} is not a budget")) };
            inputs.push(Input { name, clause, columns: query.project.iter().map(|p| p.field.clone()).collect(), file });
        }
        let refs = &relational["references"];
        let links = json!({"constructorFields": refs["constructorFields"], "siblingLinks": refs["siblingLinks"]}).to_string();
        let globs = |key: &str| -> Result<Vec<String>, String> {
            let list = doc["scope"][key].as_array().ok_or_else(|| format!("pack: scope.{key}"))?;
            list.iter().map(|g| g.as_str().map(str::to_owned).ok_or_else(|| format!("pack: scope.{key} glob"))).collect()
        };
        let scope = [globs("include")?, globs("exclude")?];
        Ok(Pack { rule, inputs, links, scope })
    }

    /// Every input's rows over `file` (read as `READING`), whose tape the caller installed
    /// (`TapeCacheSlot::seed`).
    pub fn evaluate(&self, file: &FileInput) -> Result<Vec<Table>, String> {
        let snapshot = Snapshot { revision: 0, files: vec![file.clone()] };
        let mut out = Vec::with_capacity(self.inputs.len());
        for input in &self.inputs {
            let clause = &self.rule.clauses[input.clause];
            let ClauseKind::Budget { query, .. } = &clause.kind else { unreachable!() };
            let mut metrics = Metrics::default();
            let rows = eval::stateless_rows_to(&self.rule, clause, (query, eval::evidence::Side::Counted), &snapshot, &mut metrics, None).map_err(|e| format!("{}: {}", input.name, e.reason))?;
            if !rows.incomplete.is_empty() {
                return Err(format!("{}: incomplete", input.name));
            }
            let mut table = Table::default();
            for w in &rows.witnesses {
                let span = anchored_span(&query.anchor, w).0;
                let row = w.captures.projection().ok_or_else(|| format!("{}: a witness without its projection", input.name))?;
                if !row.published() {
                    return Err(format!("{}: unpublished projection", input.name));
                }
                let values = (0..input.columns.len())
                    .map(|i| match row.sql(i) {
                        ValueRef::Null => Value::Null,
                        ValueRef::Integer(v) => Value::Integer(v),
                        ValueRef::Real(v) => Value::Real(v),
                        ValueRef::Text(t) => Value::Text(String::from_utf8_lossy(t).into_owned()),
                        ValueRef::Blob(b) => Value::Blob(b.to_vec()),
                    })
                    .collect();
                table.rows.push((span.start_byte as i64, span.end_byte as i64, values));
            }
            out.push(table);
        }
        Ok(out)
    }
}
