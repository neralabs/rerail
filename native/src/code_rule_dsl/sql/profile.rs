//! `RERAIL_SQL_PROFILE=<path>`: where the SQL engine's time goes, across every session of the
//! process. Each statement an operator emits carries a trailing comment
//! `/*op@<start>-<end>:<shape>*/` of its innermost operator (`start`/`end`: the operator's
//! statement range in its statement's emission, deterministic, so a stage lowered twice keeps one
//! digest). Every executed statement's time, rows and runs are summed by its text, and written as
//! JSON to the path every 30 seconds and with the workspace's engine report, with the clause names
//! of its namespaces. Unset, nothing is tagged or recorded.

use super::plan::{Op, Stop, Walk};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

static PATH: LazyLock<Option<String>> = LazyLock::new(|| {
    std::env::var("RERAIL_SQL_PROFILE")
        .ok()
        .filter(|path| !path.is_empty())
});

/// Per clause a caller names (`attribute`; empty: none named), per statement text: nanoseconds,
/// rows, runs.
type Totals = HashMap<String, HashMap<String, [u64; 3]>>;
static STATEMENTS: LazyLock<Mutex<(String, Totals)>> = LazyLock::new(Mutex::default);

/// The longest shape a tag carries.
const SHAPE_CHARS: usize = 120;

pub(super) fn enabled() -> bool {
    PATH.is_some()
}

/// Attributes what is recorded from now on to `clause` (the corpus census runs one clause at a
/// time, each in its own program, so its namespaces don't name it).
#[cfg(test)]
pub(in crate::code_rule_dsl) fn attribute(clause: &str) {
    STATEMENTS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .0 = clause.to_owned();
}

/// Records one execution of `sql` (or of a pseudo-statement such as `<inputs>`).
pub(super) fn record(sql: &str, nanos: u64, rows: usize) {
    let mut guard = STATEMENTS.lock().unwrap_or_else(|error| error.into_inner());
    let (clause, totals) = &mut *guard;
    if !totals.contains_key(clause.as_str()) {
        totals.insert(clause.clone(), HashMap::new());
    }
    let statements = totals
        .get_mut(clause.as_str())
        .expect("the clause's totals");
    match statements.get_mut(sql) {
        Some(entry) => {
            entry[0] += nanos;
            entry[1] += rows as u64;
            entry[2] += 1;
        }
        None => {
            statements.insert(sql.to_owned(), [nanos, rows as u64, 1]);
        }
    }
}

/// Appends `op`'s tag to each statement it emitted (`statements[start..]`) that runs (inserts;
/// not schema statements or loop markers) and no inner operator tagged.
pub(super) fn tag(statements: &mut [String], start: usize, op: &Op) {
    if !enabled() || statements.len() <= start {
        return;
    }
    let untagged = |statement: &String| {
        (statement.starts_with("INSERT") || statement.starts_with("WITH"))
            && !(statement.ends_with("*/") && statement.contains(" /*op@"))
    };
    if !statements[start..].iter().any(untagged) {
        return;
    }
    let mut shape = shape(op, 2);
    if let Some((cut, _)) = shape.char_indices().nth(SHAPE_CHARS) {
        shape.truncate(cut);
        shape.push('…');
    }
    let tag = format!(" /*op@{start}-{}:{}*/", statements.len(), shape.replace("*/", "* /"));
    for statement in &mut statements[start..] {
        if untagged(statement) {
            statement.push_str(&tag);
        }
    }
}

/// A walk's name: its direction and stop (`has.nb`, `inside.end`, `fieldHas.rule`…).
pub(super) fn walk_name(walk: Walk) -> String {
    let stop = |stop: Stop| match stop {
        Stop::Neighbor => "nb".to_owned(),
        Stop::End => "end".to_owned(),
        Stop::Rule(_) => "rule".to_owned(),
    };
    match walk {
        Walk::Has(s) => format!("has.{}", stop(s)),
        Walk::Inside(s) => format!("inside.{}", stop(s)),
        Walk::Precedes(s) => format!("precedes.{}", stop(s)),
        Walk::Follows(s) => format!("follows.{}", stop(s)),
        Walk::FieldHas(_, s) => format!("fieldHas.{}", stop(s)),
        Walk::FieldInside(_, s) => format!("fieldInside.{}", stop(s)),
        Walk::Slot(_) => "slot".to_owned(),
        Walk::Related(_) => "related".to_owned(),
    }
}

/// A compact rendering of `op`, `depth` levels deep.
pub(super) fn shape(op: &Op, depth: usize) -> String {
    let list = |items: &mut dyn Iterator<Item = &Op>| -> String {
        if depth == 0 {
            return "…".to_owned();
        }
        items
            .map(|item| shape(item, depth - 1))
            .collect::<Vec<_>>()
            .join(",")
    };
    let sub = |op: &Op| -> String {
        if depth == 0 {
            "…".to_owned()
        } else {
            shape(op, depth - 1)
        }
    };
    match op {
        Op::Kinds(_) => "kind".to_owned(),
        Op::Regex(_) => "regex".to_owned(),
        Op::Program(_) => "program".to_owned(),
        Op::Pattern { env, vars, .. } => format!(
            "pattern{}{}",
            if *env { "/env" } else { "" },
            if vars.is_empty() { "" } else { "$" }
        ),
        Op::All(items) => format!("all({})", list(&mut items.iter())),
        Op::Any(items) => format!("any({})", list(&mut items.iter())),
        Op::Not(inner) => format!("not({})", sub(inner)),
        Op::Never => "never".to_owned(),
        Op::Relation {
            walk: w,
            sub: inner,
            binds,
        } => format!(
            "{}{}[{}]",
            walk_name(*w),
            if *binds { "$" } else { "" },
            sub(inner)
        ),
        Op::NthChild { of_rule, .. } => {
            format!("nthChild{}", if of_rule.is_some() { "/of" } else { "" })
        }
        Op::Count {
            walk: w, sub: inner, ..
        } => format!("count:{}[{}]", walk_name(*w), sub(inner)),
        Op::Stage(stage) => format!("stage{stage}"),
        Op::Call { body, .. } => format!("call[{}]", sub(body)),
        Op::Constrained { rule, constraints } => format!(
            "constrained({};{})",
            sub(rule),
            list(&mut constraints.iter().map(|(_, op)| op))
        ),
        Op::Unknowable { op, .. } => format!("unknowable[{}]", sub(op)),
        Op::AllOpen(terms) => format!("all3({} terms)", terms.len()),
        Op::AnyOpen(branches) => {
            format!("any3({})", list(&mut branches.iter().map(|(op, _)| op)))
        }
        Op::Search {
            walk: w,
            sub: inner,
            binds,
            ..
        } => format!(
            "search:{}{}[{}]",
            walk_name(*w),
            if *binds { "$" } else { "" },
            sub(inner)
        ),
        Op::Fact { .. } => "fact".to_owned(),
        Op::Recur { unit, .. } => format!("recur{unit}"),
        Op::EachRoot { .. } => "eachRoot".to_owned(),
        Op::Excluded { .. } => "excluded".to_owned(),
        Op::NthCommit { .. } => "nthCommit".to_owned(),
        Op::Seq(items) => format!("seq({})", list(&mut items.iter())),
        Op::Gate(_) => "gate".to_owned(),
        Op::Every { walk: w, sub: inner } => format!("every:{}[{}]", walk_name(*w), sub(inner)),
        Op::Parse { body, .. } => format!("parse[{}]", sub(body)),
        Op::Host { rule, .. } => format!("host[{}]", sub(rule)),
        Op::Exports { .. } => "exports".to_owned(),
    }
}

/// The clause names of namespaces `c<index>_` (`set_names`).
static NAMES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Sets the clause names the dump writes, and, once per process, starts writing the profile
/// every 30 seconds, so a run stopped before its end still leaves one.
pub(in crate::code_rule_dsl) fn set_names(names: Vec<String>) {
    if !enabled() {
        return;
    }
    *NAMES.lock().unwrap_or_else(|error| error.into_inner()) = names;
    static DUMPER: std::sync::Once = std::sync::Once::new();
    DUMPER.call_once(|| {
        std::thread::spawn(|| {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(30));
                dump();
            }
        });
    });
}

/// Writes what was recorded, with the clause names, to the profile path: per statement, with
/// the clause it was `attribute`d to, where one was.
pub(in crate::code_rule_dsl) fn dump() {
    let Some(path) = PATH.as_deref() else {
        return;
    };
    let rows: Vec<serde_json::Value> = {
        let guard = STATEMENTS.lock().unwrap_or_else(|error| error.into_inner());
        guard
            .1
            .iter()
            .flat_map(|(clause, statements)| {
                statements.iter().map(move |(sql, [nanos, rows, runs])| {
                    let mut row = serde_json::json!({ "sql": sql, "nanos": nanos, "rows": rows, "runs": runs });
                    if !clause.is_empty() {
                        row["clause"] = clause.as_str().into();
                    }
                    row
                })
            })
            .collect()
    };
    let names = NAMES
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let document = serde_json::json!({ "clauses": names, "statements": rows });
    let partial = format!("{path}.partial");
    if let Err(error) = std::fs::write(&partial, document.to_string())
        .and_then(|()| std::fs::rename(&partial, path))
    {
        eprintln!("RERAIL_SQL_PROFILE: cannot write {path}: {error}");
    }
}
