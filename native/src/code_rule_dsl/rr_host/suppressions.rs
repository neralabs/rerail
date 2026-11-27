//! The findings layer's suppression comments for a `.rerail` run (docs/rule-syntax/data-layer/FINDINGS-LAYER.md §1,
//! stage 3). Lowering keeps a `.violation`'s `suppress TOOL "rule", … [over {c}]` lines as the settings
//! `suppress V TOOL "rule", … [over {c}]` (`lower::finding_settings`); here they drop the findings the linter's
//! comments silence. Each line is the JSON rule's `suppress` for that linter (`rr::SUPPRESS_TOOLS`), and the product's
//! own matchers read it (`violations::finding_suppressed`, the family adapter's `suppresses`), on the file's tree as
//! they read it: the finding is anchored over its `at`'s bytes, and a linter that reads the node it visited (Bandit's
//! lines) reads `over`'s, else `at`'s. An `at` the run's projection cannot name exactly (a `@place`, a guest's node, a
//! match span) stands for its line (`spans`). A file's findings are read in order of their anchors, under one file
//! scope, as a JSON rule's witnesses are: each linter's comments are read once, and PHPStan's spend as they are met.

use super::findings::parsed_as;
use super::spans::FileSpans;
use crate::code_rule_dsl::logic::plan::output::{Node, Row, Value};
use crate::code_rule_dsl::logic::rr::{self, SUPPRESS_TOOLS};
use crate::code_rule_dsl::matcher::FileScopeSlot;
use crate::code_rule_dsl::suppress::{self, Suppress};
use crate::code_rule_dsl::tape::TapeDoc;
use crate::code_rule_dsl::violations::finding_suppressed;
use ast_grep_core::AstGrep;
use rerail_rr_syntax::syntax::{Tok, tokens};
use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::path::Path;

/// The `suppress` lines of a run's policies: each `.violation`'s, by its policy and name, and each policy's
/// `.language`.
#[derive(Default)]
pub struct Suppressions {
    lines: HashMap<(String, String), Vec<Line>>,
    languages: HashMap<String, Option<String>>,
}

/// One `suppress` line: the linter's comments as a JSON rule asks for them, and the column whose node the linter
/// reads over.
struct Line {
    suppress: Suppress,
    over: Option<String>,
}

impl Suppressions {
    /// Those a program's settings hold (its `.policy`'s `.violation`s'): `[key, text]` pairs.
    pub fn add<'s>(&mut self, settings: impl IntoIterator<Item = (&'s str, &'s str)> + Clone) -> Result<(), String> {
        let policy = settings.clone().into_iter().find(|(key, _)| *key == "policy").and_then(|(_, text)| text.split('"').nth(1));
        let Some(policy) = policy else { return Ok(()) };
        let language = settings.clone().into_iter().find(|(key, _)| *key == "language").map(|(_, text)| text.trim().trim_matches('"').to_owned());
        for (_, text) in settings.into_iter().filter(|(key, _)| *key == "suppress") {
            let (violation, line) = line(text).map_err(|e| format!("{policy}: suppress {text}: {e}"))?;
            self.lines.entry((policy.to_owned(), violation)).or_default().push(line);
            self.languages.insert(policy.to_owned(), language.clone());
        }
        Ok(())
    }

    /// Whether no `.violation` has a `suppress` line.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Which of a run's findings (rows of its `rr_diagnostic` relations) their `suppress` lines drop, under `root`.
    pub fn suppressed(&self, root: &Path, rows: &[Row<'_>]) -> Vec<bool> {
        let mut dropped = vec![false; rows.len()];
        if self.is_empty() {
            return dropped;
        }
        // The findings to read, by the file and the dialect their policy reads it in.
        let mut files: BTreeMap<(&str, Option<&'static str>), Vec<(usize, Finding<'_, '_>)>> = BTreeMap::new();
        for (k, row) in rows.iter().enumerate() {
            if let Some(finding) = self.finding(row) {
                let language = self.languages.get(finding.policy).cloned().flatten();
                files.entry((finding.path, parsed_as(language.as_deref(), finding.path))).or_default().push((k, finding));
            }
        }
        for ((path, label), findings) in files {
            let Some(file) = FileSpans::read_with_tree(root, path, label) else { continue };
            let Some(tree) = file.tree() else { continue };
            for (k, suppressed) in read(path, &file, tree, findings) {
                dropped[k] = suppressed;
            }
        }
        dropped
    }

    /// A finding a `suppress` line asks about: one in a file, of a `.violation` that has them.
    fn finding<'r, 'a>(&'r self, row: &Row<'a>) -> Option<Finding<'r, 'a>> {
        let (policy, violation) = (row.get(0).text()?, row.get(1).text()?);
        let lines = self.lines.get(&(policy.to_owned(), violation.to_owned()))?;
        let path = row.get(3).text().filter(|path| !path.is_empty())?;
        let node = |value: Value<'_>| match value {
            Value::Node(Node::Of { path: Some(of), value, .. }) if of == path => Some(value),
            _ => None,
        };
        // The parameters after the head (and the tag), each name beside its value (`tools::diagnostic`).
        let head = rr::DIAGNOSTIC_HEAD.len() + usize::from(row.get(rr::DIAGNOSTIC_HEAD.len() - 1).int() == Some(1));
        let parameter = |name: &str| (head..row.len().saturating_sub(1)).step_by(2).find(|&c| row.get(c).text() == Some(name)).map(|c| row.get(c + 1));
        Some(Finding {
            policy,
            code: row.get(2).text().unwrap_or(violation),
            path,
            line: row.get(4).int().unwrap_or(0),
            at: node(row.get(5)),
            lines: lines.iter().map(|line| (&line.suppress, line.over.as_deref().and_then(&parameter).and_then(&node))).collect(),
        })
    }
}

/// A finding a `suppress` line asks about: its policy, code, file, line and `at` (a node of the file's), and each line
/// with the node its `over` names.
struct Finding<'r, 'a> {
    policy: &'a str,
    code: &'a str,
    path: &'a str,
    line: i64,
    at: Option<i64>,
    lines: Vec<(&'r Suppress, Option<i64>)>,
}

/// Whether each of a file's findings is suppressed, by its index: read in order of their anchors, under one file scope.
fn read(path: &str, file: &FileSpans, tree: &TapeDoc, findings: Vec<(usize, Finding<'_, '_>)>) -> Vec<(usize, bool)> {
    let mut anchored: Vec<(Range<usize>, usize, Finding<'_, '_>)> = findings
        .into_iter()
        .filter_map(|(k, finding)| {
            let span = file.span(finding.at, finding.line)?;
            Some((span.start_byte..span.end_byte, k, finding))
        })
        .collect();
    anchored.sort_by_key(|(bytes, k, _)| (bytes.start, bytes.end, *k));
    let _scope = FileScopeSlot::install(Vec::new(), None, false).path(path);
    let grep = AstGrep::doc(tree.clone());
    let root = grep.root();
    anchored
        .into_iter()
        .map(|(bytes, k, finding)| {
            let node = node_over(root.clone(), &bytes);
            let suppressed = finding.lines.iter().any(|(suppress, over)| {
                let witness = over.and_then(|over| file.node_span(over)).map_or((bytes.start, bytes.end), |span| (span.start_byte, span.end_byte));
                finding_suppressed(finding.code, suppress, &node, bytes.clone(), witness)
            });
            (k, suppressed)
        })
        .collect()
}

/// The innermost node of the tree under `node` that holds `bytes`.
fn node_over<'t>(mut node: ast_grep_core::Node<'t, TapeDoc>, bytes: &Range<usize>) -> ast_grep_core::Node<'t, TapeDoc> {
    loop {
        let holder = node.children().find(|child| {
            let range = child.range();
            range.start <= bytes.start && bytes.end <= range.end && range.start < range.end
        });
        match holder {
            Some(child) => node = child,
            None => return node,
        }
    }
}

/// A `suppress` setting's text, `V TOOL "rule", … [over {c}]`: the violation's name, and the line as the JSON rule's
/// `suppress` for the tool's kind (`suppress::parse`).
fn line(text: &str) -> Result<(String, Line), String> {
    let tokens = tokens(text).map_err(|rerail_rr_syntax::Error::Syntax(error)| error)?;
    let words: Vec<&Tok<'_>> = tokens.iter().map(|(tok, _, _)| tok).collect();
    let [Tok::Id(violation), Tok::Id(tool), rest @ ..] = words.as_slice() else {
        return Err("no violation and linter".to_owned());
    };
    let (_, kind, list) = SUPPRESS_TOOLS.iter().find(|(name, _, _)| name == tool).ok_or_else(|| format!("no linter {tool}"))?;
    let names: Vec<&str> = rest.iter().map_while(|tok| match tok {
        Tok::Str(name) => Some(Some(&**name)),
        Tok::P(",") => Some(None),
        _ => None,
    }).flatten().collect();
    let over = match rest.iter().position(|tok| matches!(tok, Tok::Id("over"))).map(|at| &rest[at + 1..]) {
        Some([Tok::P("{"), Tok::Id(column), Tok::P("}"), ..]) => Some((*column).to_owned()),
        Some(_) => return Err("over names no column".to_owned()),
        None => None,
    };
    let spec = serde_json::json!({ *kind: { *list: names } });
    let suppress = suppress::parse(&spec, "suppress").map_err(|e| e.reason)?;
    Ok(((*violation).to_owned(), Line { suppress, over }))
}
