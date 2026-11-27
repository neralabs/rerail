//! A finding's fix, as `runRr` hands it (decision 0720): the rows of each fix edit's relation (`lower::fixes`), read
//! from the run's outputs, and the first fix of a finding (by its `fix` line, then by its rule) whose every edit has
//! one row, each edit's text filled from the file and the edits composed into one replacement, as a JSON rule's
//! violation carries its fix: `{span, text}`.
//!
//! An edit's text is padded as the applier pads it (core.rerail, "edits"): where it would start or end with an
//! identifier character beside one outside the edit, a space goes between. It is not reindented. A fix none of whose
//! rules gives a whole set of edits, or whose edits overlap or reach another file, is not offered.

use super::spans::FileSpans;
use crate::code_rule_dsl::logic::plan::output::{self, Node, Outputs, Row};
use crate::code_rule_dsl::logic::rr::lower::fixes::{HEAD, Part, is_fix_edit, template_parts};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;

/// A value of a fix edit's row, as an edit reads it: a text, a number, or a node or position of a file of the run (its
/// path and value).
#[derive(Clone, Debug, PartialEq)]
enum Cell {
    Text(String),
    Int(i64),
    Node(String, i64),
    Other,
}

/// One edit of one fix of a finding: which fix, rule and edit it is, the rule's count of edits, its text's pieces, its
/// target and its placeholders' values by name.
#[derive(Debug, PartialEq)]
pub struct Edit {
    fix: i64,
    rule: i64,
    edit: i64,
    edits: i64,
    parts: Vec<Part>,
    target: Cell,
    holes: Vec<(String, Cell)>,
}

/// The fix edits of a run, by the finding each fixes: its policy, violation and `at` (as a finding's JSON holds it),
/// with the violation's other columns by name, which tell findings at one place apart.
#[derive(Default)]
pub struct Edits {
    by_at: HashMap<(String, String, String), Vec<(Vec<(String, Value)>, Edit)>>,
}

/// A row's value as a finding's JSON holds it (`tools::diagnostic`): a text, a number, a node's value, or null.
fn json_of(row: &Row<'_>, c: usize) -> Value {
    match row.get(c) {
        output::Value::Text(t) => json!(t),
        output::Value::Int(n) => json!(n),
        output::Value::Node(_) => json!(row.raw(c)),
        output::Value::Null => Value::Null,
    }
}

/// A row's value as an edit reads it.
fn cell_of(row: &Row<'_>, c: usize) -> Cell {
    match row.get(c) {
        output::Value::Text(t) => Cell::Text(t.to_owned()),
        output::Value::Int(n) => Cell::Int(n),
        output::Value::Node(Node::Of { path: Some(path), value, .. }) => Cell::Node(path.to_owned(), value),
        _ => Cell::Other,
    }
}

/// The comma-joined names in column `c` of `row`.
fn names(row: &Row<'_>, c: usize) -> Vec<String> {
    row.get(c).text().unwrap_or_default().split(',').filter(|name| !name.is_empty()).map(str::to_owned).collect()
}

impl Edits {
    /// The fix edits in the run's output directories `dirs`.
    pub fn read(dirs: &[PathBuf]) -> Result<Edits, String> {
        let mut edits = Edits::default();
        for dir in dirs {
            let outputs = Outputs::open(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            for rel in outputs.names().into_iter().filter(|rel| is_fix_edit(rel)) {
                outputs.rows(rel).iter().for_each(|row| edits.add(row));
            }
        }
        Ok(edits)
    }

    /// One row of a fix edit's relation (`HEAD`, the violation's columns, the target, the placeholders); a row of
    /// another shape is no edit.
    fn add(&mut self, row: &Row<'_>) {
        let (columns, holes) = (names(row, 7), names(row, 8));
        let at = HEAD.len();
        let (Some(policy), Some(violation), Some(template)) = (row.get(0).text(), row.get(1).text(), row.get(6).text()) else { return };
        let Some(parts) = template_parts(template).filter(|_| row.len() == at + columns.len() + 1 + holes.len() && !columns.is_empty()) else { return };
        let int = |c: usize| row.get(c).int().unwrap_or(-1);
        let edit = Edit {
            fix: int(2), rule: int(3), edit: int(4), edits: int(5), parts,
            target: cell_of(row, at + columns.len()),
            holes: holes.iter().enumerate().map(|(k, hole)| (hole.clone(), cell_of(row, at + columns.len() + 1 + k))).collect(),
        };
        let others = columns.iter().enumerate().skip(1).map(|(k, name)| (name.clone(), json_of(row, at + k))).collect();
        let key = (policy.to_owned(), violation.to_owned(), json_of(row, at).to_string());
        self.by_at.entry(key).or_default().push((others, edit));
    }

    /// The fixes that may fix finding `d` (`tools::diagnostic`'s JSON), in order: each rule's edits, one each, where
    /// every edit of the rule has one row for the finding.
    pub fn of(&self, d: &Value) -> Vec<Vec<&Edit>> {
        let key = (d["policy"].as_str().unwrap_or_default().to_owned(), d["clause"].as_str().unwrap_or_default().to_owned(), d["at"].to_string());
        let agrees = |others: &[(String, Value)]| others.iter().all(|(name, value)| d["parameters"].get(name).is_none_or(|given| given.is_null() || given == value));
        let mut rules: Vec<((i64, i64), Vec<&Edit>)> = Vec::new();
        for (_, edit) in self.by_at.get(&key).into_iter().flatten().filter(|(others, _)| agrees(others)) {
            match rules.iter_mut().find(|(rule, _)| *rule == (edit.fix, edit.rule)) {
                Some((_, edits)) => edits.push(edit),
                None => rules.push(((edit.fix, edit.rule), vec![edit])),
            }
        }
        rules.sort_by_key(|(rule, _)| *rule);
        rules.into_iter().filter_map(|(_, edits)| one_each(edits)).collect()
    }
}

/// `edits`, one per edit of their rule in order, where each has exactly one (rows that are the same are one).
fn one_each(edits: Vec<&Edit>) -> Option<Vec<&Edit>> {
    let mut unique: Vec<&Edit> = Vec::with_capacity(edits.len());
    for edit in edits {
        if !unique.contains(&edit) {
            unique.push(edit);
        }
    }
    unique.sort_by_key(|edit| edit.edit);
    let whole = unique.iter().enumerate().all(|(k, edit)| edit.edit == k as i64 && edit.edits == unique.len() as i64);
    whole.then_some(unique)
}

/// The fix `edits` make in `file`, the finding's at `path`, as one replacement `{span, text}`: none where an edit's
/// target or a placeholder's node is elsewhere, or two edits overlap.
pub fn compose(file: &FileSpans, path: &str, edits: &[&Edit]) -> Option<Value> {
    let mut replaced = edits.iter().map(|edit| filled(file, path, edit)).collect::<Option<Vec<(usize, usize, String)>>>()?;
    replaced.sort_by_key(|(start, end, _)| (*start, *end));
    let (start, end) = (replaced.first()?.0, replaced.iter().map(|(_, end, _)| *end).max()?);
    let mut text = String::new();
    let mut at = start;
    for (from, to, with) in replaced {
        text.push_str(file.source().get(at..from)?);
        text.push_str(&with);
        at = to;
    }
    text.push_str(file.source().get(at..end)?);
    Some(json!({"span": file.bytes(start, end), "text": text}))
}

/// An edit's bytes in `file` and its text, filled and padded; none where its target or a placeholder's node is not
/// one of `file`'s (at `path`) that the run's tree holds.
fn filled(file: &FileSpans, path: &str, edit: &Edit) -> Option<(usize, usize, String)> {
    let [start, end] = node_range(file, path, &edit.target)?;
    let mut text = String::new();
    for part in &edit.parts {
        match part {
            Part::Text(t) => text.push_str(t),
            Part::Hole(name) => text.push_str(&hole_text(file, path, edit.holes.iter().find(|(hole, _)| hole == name).map(|(_, cell)| cell)?)?),
        }
    }
    Some((start, end, padded(file.source(), start, end, text)))
}

/// The bytes of node `cell` in `file`, at `path`.
fn node_range(file: &FileSpans, path: &str, cell: &Cell) -> Option<[usize; 2]> {
    let Cell::Node(at, value) = cell else { return None };
    let [start, end] = file.range(*value).filter(|_| at == path)?;
    Some([start as usize, end as usize])
}

/// A placeholder's text: a node's source, a symbol, a number.
fn hole_text(file: &FileSpans, path: &str, cell: &Cell) -> Option<String> {
    match cell {
        Cell::Text(t) => Some(t.clone()),
        Cell::Int(n) => Some(n.to_string()),
        Cell::Node(..) => {
            let [start, end] = node_range(file, path, cell)?;
            file.source().get(start..end).map(str::to_owned)
        }
        Cell::Other => None,
    }
}

/// `text` in place of `source[start..end]`, with a space where it would join an identifier outside the edit.
fn padded(source: &str, start: usize, end: usize, text: String) -> String {
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '$');
    let before = word(source[..start].chars().next_back()) && word(text.chars().next());
    let after = word(source[end..].chars().next()) && word(text.chars().next_back());
    format!("{}{text}{}", if before { " " } else { "" }, if after { " " } else { "" })
}

#[cfg(test)]
mod tests {
    use super::padded;

    #[test]
    fn a_text_that_would_join_a_word_outside_it_is_padded() {
        assert_eq!(padded("return{a: 1}", 6, 12, "dict(a=1)".to_owned()), " dict(a=1)");
        assert_eq!(padded("d.keys()in x", 0, 8, "d".to_owned()), "d ");
        assert_eq!(padded("if (a) b();", 7, 11, "{b();}".to_owned()), "{b();}");
    }
}
