//! A run's findings as `runRr` hands them: each of `run_policies`' diagnostics, with its span (`spans`) and its
//! parameters as texts.

use super::spans::FileSpans;
use crate::code_rule_dsl::language::{CodeRuleLanguage, label, label_named, label_of_path};
use crate::code_rule_dsl::logic::plan::output::{self, Node, Outputs};
use crate::code_rule_dsl::logic::rr;
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The findings of the run whose report is `report` and whose outputs are under `out`, over the corpus at `root`.
pub fn of(root: &Path, out: &Path, report: &Value) -> Result<Vec<Value>, String> {
    let mut located = Located { root, nodes: file_nodes(out)?, languages: languages(report), files: HashMap::new() };
    Ok(report["diagnostics"].as_array().into_iter().flatten().map(|diagnostic| located.finding(diagnostic)).collect())
}

/// What a finding's span is read from: the nodes of the run's files its findings name, each policy's language, and
/// each file read so far.
struct Located<'r> {
    root: &'r Path,
    nodes: HashSet<(String, i64)>,
    languages: HashMap<String, Option<String>>,
    files: HashMap<(String, Option<&'static str>), Option<FileSpans>>,
}

impl Located<'_> {
    fn finding(&mut self, d: &Value) -> Value {
        json!({
            "policy": d["policy"], "clause": d["clause"], "code": d["code"], "message": d["message"], "path": d["path"],
            "line": d["line"], "span": self.span(d), "tag": d["tag"], "parameters": parameters(&d["parameters"]),
        })
    }

    /// The finding's span; `null` for a finding in no file (a tool call's), or in a file that no longer reads.
    fn span(&mut self, d: &Value) -> Value {
        let Some(path) = d["path"].as_str().filter(|path| !path.is_empty()) else { return Value::Null };
        let language = d["policy"].as_str().and_then(|policy| self.languages.get(policy)).cloned().flatten();
        let label = parsed_as(language.as_deref(), path);
        let node = d["at"].as_i64().filter(|&at| self.nodes.contains(&(path.to_owned(), at)));
        let root = self.root;
        let file = self.files.entry((path.to_owned(), label)).or_insert_with(|| FileSpans::read(root, path, label));
        let span = file.as_ref().and_then(|file| file.span(node, d["line"].as_i64().unwrap_or(0)));
        span.map_or(Value::Null, |span| json!(span))
    }
}

/// The dialect a policy of `language` parses the file at `path` in: the file's own, where the language has it (or
/// the policy names none); `None` where the run reads the file as text.
fn parsed_as(language: Option<&str>, path: &str) -> Option<&'static str> {
    let own = label_of_path(path)?;
    match language.map(dialect_labels) {
        Some(labels) if !labels.contains(&own) => None,
        _ => Some(own),
    }
}

/// The labels of the dialects a language named `name` reads (TypeScript's: `.ts`, `.tsx` and `.js`).
fn dialect_labels(name: &str) -> Vec<&'static str> {
    match CodeRuleLanguage::ALL.into_iter().find(|language| language.name() == name) {
        Some(language) => language.dialects().iter().filter_map(|dialect| label(*dialect)).collect(),
        None => label_named(name).into_iter().collect(),
    }
}

/// Each policy's `.language`, by its `.policy` id, from the runs' records (`rr::metadata`'s settings).
fn languages(report: &Value) -> HashMap<String, Option<String>> {
    let settings = report["runs"].as_array().into_iter().flatten().filter_map(|run| run["metadata"]["settings"].as_array());
    settings.filter_map(|settings| Some((setting(settings, "policy")?.split('"').nth(1)?.to_owned(), setting(settings, "language").map(|l| l.trim().trim_matches('"').to_owned())))).collect()
}

/// The first setting `key` of `[key, text]` pairs.
fn setting<'v>(settings: &'v [Value], key: &str) -> Option<&'v str> {
    settings.iter().find(|pair| pair[0] == key).and_then(|pair| pair[1].as_str())
}

/// The `at` of every finding of the outputs under `out` that is a node of a file of the run (not a guest's, not a
/// place), with its file's path: those whose span is read off the file's tree.
fn file_nodes(out: &Path) -> Result<HashSet<(String, i64)>, String> {
    let at = rr::DIAGNOSTIC_HEAD.iter().position(|column| *column == "at").ok_or("a finding has no `at` column")?;
    let mut nodes = HashSet::new();
    for dir in output_dirs(out) {
        let outputs = Outputs::open(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for rel in outputs.names().into_iter().filter(|rel| rr::is_diagnostic(rel)) {
            nodes.extend(outputs.rows(rel).iter().filter_map(|row| file_node(row.get(at))));
        }
    }
    Ok(nodes)
}

/// A node of a file of the run: its file's path and its value.
fn file_node(value: output::Value<'_>) -> Option<(String, i64)> {
    match value {
        output::Value::Node(Node::Of { path: Some(path), value, .. }) => Some((path.to_owned(), value)),
        _ => None,
    }
}

/// `dir` and every directory under it that holds a run's outputs.
fn output_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        if dir.join(output::FILE).is_file() {
            found.push(dir.clone());
        }
        let entries = std::fs::read_dir(&dir).into_iter().flatten().flatten();
        pending.extend(entries.map(|entry| entry.path()).filter(|path| path.is_dir()));
    }
    found
}

/// A finding's parameters, each value as a text (a number as written; none for a value that is none).
fn parameters(parameters: &Value) -> Map<String, Value> {
    let texts = parameters.as_object().into_iter().flatten().filter_map(|(name, value)| match value {
        Value::String(text) => Some((name.clone(), json!(text))),
        Value::Null => None,
        other => Some((name.clone(), json!(other.to_string()))),
    });
    texts.collect()
}
