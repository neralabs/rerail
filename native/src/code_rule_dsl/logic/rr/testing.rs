//! Testing mode (decision 0119): the tests a `.rr` file carries, each run as the file's policy over its fixtures.
//!
//! A test is `.test name [exactly]` … `.end` (`syntax::Item::Test`). Lowering never reads one, so a file's program is
//! the same with its tests or without them. Here a test's program is the file's statements with the test's own
//! (declarations, clauses, facts) spliced in after them, read as `read_file` reads the file (`rr::lower_files`); its
//! run is `run-rr`'s (`tools::run_program`) over a directory holding its fixtures; its `.expect`s are checked against
//! the run's findings (finding keys: code, path, line, column) and the rows of the relations they name.

use std::path::{Path, PathBuf};

use super::syntax::{self, Item, Statement, Tok};

/// One test of a file.
#[derive(Clone, Debug)]
pub struct Test {
    pub name: String,
    /// The line of its `.test`.
    pub line: usize,
    pub exactly: bool,
    /// Each fixture's path under the run's root, and its bytes.
    pub fixtures: Vec<(String, Vec<u8>)>,
    /// The test's own declarations, clauses and facts.
    pub rules: Vec<Statement>,
    pub expects: Vec<Expect>,
}

/// One `.expect`, and its line.
#[derive(Clone, Debug, PartialEq)]
pub struct Expect {
    pub line: usize,
    pub negated: bool,
    pub what: Expected,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expected {
    /// A finding: its code, and where (`at "path"[:line[:column]]`); what is left out matches anything.
    Finding { code: String, path: Option<String>, line: Option<i64>, column: Option<i64> },
    /// A row of a relation; `None` is `_`. Each value as the run writes it, and whether it was written as a string.
    Row { relation: String, values: Vec<Option<(String, bool)>> },
    /// `count <code or relation> = n`: the findings of that code, or the relation's rows.
    Count { name: String, n: usize },
}

impl std::fmt::Display for Expect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let no = if self.negated { "no " } else { "" };
        match &self.what {
            Expected::Finding { code, path, line, column } => {
                write!(f, "{no}{code}")?;
                if let Some(path) = path {
                    write!(f, " at {path:?}")?;
                }
                if let Some(line) = line {
                    write!(f, ":{line}")?;
                }
                if let Some(column) = column {
                    write!(f, ":{column}")?;
                }
                Ok(())
            }
            Expected::Row { relation, values } => {
                let values = values
                    .iter()
                    .map(|v| match v {
                        None => "_".to_owned(),
                        Some((v, true)) => format!("{v:?}"),
                        Some((v, false)) => v.clone(),
                    })
                    .collect::<Vec<_>>();
                write!(f, "{no}{relation}({})", values.join(", "))
            }
            Expected::Count { name, n } => write!(f, "count {name} = {n}"),
        }
    }
}

fn bad<T>(path: &Path, line: usize, what: impl std::fmt::Display) -> Result<T, String> {
    Err(format!("{}:{line}: {what}", path.display()))
}

/// The tests of the `.rr` file at `path`, in order.
pub fn tests(path: &Path) -> Result<Vec<Test>, String> {
    let (statements, _) = super::read(path).map_err(|e| e.to_string())?;
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut out: Vec<Test> = Vec::new();
    for (item, line) in statements {
        let Item::Test { name, exactly, items } = item else { continue };
        if out.iter().any(|t| t.name == name) {
            return bad(path, line, format!("a second test named {name}"));
        }
        let mut test = Test { name, line, exactly, fixtures: Vec::new(), rules: Vec::new(), expects: Vec::new() };
        for (item, at) in items {
            match item {
                Item::Setting { key, text } if key == "fixture" => test.fixtures.push(fixture(path, dir, at, &text)?),
                Item::Setting { key, text } if key == "expect" => test.expects.push(expect(path, at, &text)?),
                Item::Setting { key, .. } if key != "input" && key != "output" => {
                    return bad(path, at, format!(".{key} in a test: a test holds `.fixture`, `.expect`, declarations and clauses"));
                }
                Item::Include(_) | Item::Policy(_) | Item::Violation { .. } => {
                    return bad(path, at, "a test holds `.fixture`, `.expect`, declarations and clauses");
                }
                item => test.rules.push((item, at)),
            }
        }
        if let Some(e) = test.expects.iter().find(|e| test.exactly && e.negated) {
            return bad(path, e.line, "`exactly` lists the findings a test has: no `.expect no` beside it");
        }
        out.push(test);
    }
    Ok(out)
}

/// `.fixture "path" <fenced text>` or `.fixture "path" from "file"` (beside the `.rr` file).
fn fixture(path: &Path, dir: &Path, line: usize, text: &str) -> Result<(String, Vec<u8>), String> {
    let toks = syntax::tokens(text).map_err(|e| format!("{}:{line}: {e:?}", path.display()))?;
    let toks: Vec<&Tok> = toks.iter().map(|(t, _, _)| t).collect();
    let name = match toks.as_slice() {
        [Tok::Str(name), ..] => name.clone(),
        _ => return bad(path, line, "`.fixture \"<path>\"` then its text"),
    };
    if name.starts_with('/') || name.split('/').any(|part| part == "..") {
        return bad(path, line, format!("a fixture's path is under the test's root: {name:?}"));
    }
    match toks.as_slice() {
        [_, Tok::Tick(body)] => Ok((name, dedent(body).into_bytes())),
        [_, Tok::Id(from), Tok::Str(file)] if from == "from" => {
            std::fs::read(dir.join(file)).map(|bytes| (name, bytes)).map_err(|e| format!("{}:{line}: {file}: {e}", path.display()))
        }
        _ => bad(path, line, "`.fixture \"<path>\"` then a fenced text or `from \"<file>\"`"),
    }
}

/// A fence's body as a file: its first newline gone, then the indent its lines share; a last line of only blanks (the
/// closing fence's indent) gone; ending in a newline.
pub fn dedent(body: &str) -> String {
    let body = body.strip_prefix("\r\n").or_else(|| body.strip_prefix('\n')).unwrap_or(body);
    let mut lines: Vec<&str> = body.split('\n').collect();
    if lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    let indent = lines.iter().filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    let mut out = String::new();
    for l in lines {
        out.push_str(l.get(indent..).unwrap_or(l.trim_start()));
        out.push('\n');
    }
    out
}

/// One `.expect`'s text.
fn expect(path: &Path, line: usize, text: &str) -> Result<Expect, String> {
    let toks = syntax::tokens(text).map_err(|e| format!("{}:{line}: {e:?}", path.display()))?;
    let mut toks: &[&Tok] = &toks.iter().map(|(t, _, _)| t).collect::<Vec<_>>();
    let word = |t: &Tok, w: &str| matches!(t, Tok::Id(s) if s == w);
    let negated = toks.first().is_some_and(|t| word(t, "no"));
    if negated {
        toks = &toks[1..];
    }
    let usage = "`.expect [no] <code> [at \"<path>\"[:<line>[:<column>]]]`, `.expect [no] <relation>(<value>, …)` or `.expect count <name> = <n>`";
    if let [count, Tok::Id(name) | Tok::Str(name), Tok::P("="), Tok::Num(n)] = toks
        && word(count, "count")
        && !negated
    {
        return Ok(Expect { line, negated, what: Expected::Count { name: name.clone(), n: *n as usize } });
    }
    let (name, rest) = match toks {
        [Tok::Id(name) | Tok::Str(name), rest @ ..] => (name.clone(), rest),
        _ => return bad(path, line, usage),
    };
    if let [Tok::P("("), inner @ .., Tok::P(")")] = rest {
        let mut values = Vec::new();
        for (k, t) in inner.iter().enumerate() {
            match (k % 2, t) {
                (0, Tok::Str(s)) => values.push(Some((s.clone(), true))),
                (0, Tok::Num(n)) => values.push(Some((n.to_string(), false))),
                (0, Tok::Float(x)) => values.push(Some((x.clone(), false))),
                (0, Tok::Id(w)) if w == "_" => values.push(None),
                (1, Tok::P(",")) => {}
                _ => return bad(path, line, usage),
            }
        }
        return Ok(Expect { line, negated, what: Expected::Row { relation: name, values } });
    }
    let (mut at, mut row, mut column) = (None, None, None);
    match rest {
        [] => {}
        [a, Tok::Str(p), more @ ..] if word(a, "at") => {
            at = Some(p.clone());
            match more {
                [] => {}
                [Tok::P(":"), Tok::Num(l)] => row = Some(*l),
                [Tok::P(":"), Tok::Num(l), Tok::P(":"), Tok::Num(c)] => (row, column) = (Some(*l), Some(*c)),
                _ => return bad(path, line, usage),
            }
        }
        _ => return bad(path, line, usage),
    }
    Ok(Expect { line, negated, what: Expected::Finding { code: name, path: at, line: row, column } })
}

/// A test's outcome: the `.expect`s that failed, each with what the run had instead.
#[derive(Debug, Default)]
pub struct Outcome {
    pub failures: Vec<String>,
    /// The run's findings, as keys: `code at "path":line:column`.
    pub findings: Vec<String>,
}

impl Outcome {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

/// A finding's key, from a row of `rr_diagnostic.csv`.
#[derive(Clone, Debug, PartialEq)]
struct Finding {
    code: String,
    path: String,
    line: i64,
    column: Option<i64>,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at {:?}:{}", self.code, self.path, self.line)?;
        if let Some(column) = self.column {
            write!(f, ":{column}")?;
        }
        Ok(())
    }
}

fn matches(e: &Expected, f: &Finding) -> bool {
    let Expected::Finding { code, path, line, column } = e else { return false };
    *code == f.code
        && path.as_ref().is_none_or(|p| *p == f.path)
        && line.is_none_or(|l| l == f.line)
        && column.is_none_or(|c| f.column.is_none_or(|fc| fc == c))
}

/// Runs `test` of the file at `path` in `work` (a directory of its own, made empty first): its fixtures under
/// `work/root`, the run's outputs under `work/out`.
pub fn run(path: &Path, test: &Test, work: &Path) -> Result<Outcome, String> {
    let root = work.join("root");
    let out = work.join("out");
    let _ = std::fs::remove_dir_all(work);
    std::fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    for (name, bytes) in &test.fixtures {
        let file = root.join(name);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&file, bytes).map_err(|e| format!("{}: {e}", file.display()))?;
    }
    // The file's program, the test's rules spliced in after the file's own statements.
    let (mut modules, files) = super::program(path).map_err(|e| e.to_string())?;
    let own = path.canonicalize().map_err(|e| format!("{}: {e}", path.display()))?;
    let Some((_, statements)) = modules.iter_mut().find(|(p, _)| *p == own) else {
        return Err(format!("{}: not among its program's modules", path.display()));
    };
    statements.extend(test.rules.iter().cloned());
    let mut program = super::lower_files(path, modules).map_err(|e| format!("{} test {}: {e}", path.display(), test.name))?;
    // The relations the test's `.expect`s read are outputs of its run.
    let named = |e: &Expect| match &e.what {
        Expected::Row { relation, .. } => Some(relation.clone()),
        Expected::Count { name, .. } if program.decls.contains_key(name) => Some(name.clone()),
        _ => None,
    };
    for relation in test.expects.iter().filter_map(named).collect::<Vec<_>>() {
        if !program.decls.contains_key(&relation) {
            return Err(format!("{} test {}: `.expect` names {relation}, which no rule declares", path.display(), test.name));
        }
        if !program.outputs.contains(&relation) {
            program.outputs.push(relation);
        }
    }
    let declared: std::collections::BTreeSet<String> = program.decls.keys().cloned().collect();
    let report = crate::code_rule_dsl::tools::run_program(path, program, &files, &root, &out, &serde_json::Value::Null)?;
    let findings: Vec<Finding> = report["diagnostics"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|row| Finding {
            code: row["code"].as_str().unwrap_or_default().to_owned(),
            path: row["path"].as_str().unwrap_or_default().to_owned(),
            line: row["line"].as_i64().unwrap_or(0),
            column: row["column"].as_i64(),
        })
        .collect();
    let rows = |relation: &str| -> Result<Vec<Vec<String>>, String> {
        match std::fs::read_to_string(out.join(format!("{relation}.csv"))) {
            Ok(text) => Ok(text.lines().map(|l| l.split('\t').map(str::to_owned).collect()).collect()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(format!("{relation}.csv: {e}")),
        }
    };
    let mut outcome = Outcome { failures: Vec::new(), findings: findings.iter().map(ToString::to_string).collect() };
    let reported = || if findings.is_empty() { "no findings".to_owned() } else { format!("reported {}", outcome_list(&findings)) };
    for e in &test.expects {
        let failed = match &e.what {
            Expected::Finding { .. } => {
                let hit: Vec<&Finding> = findings.iter().filter(|f| matches(&e.what, f)).collect();
                match (e.negated, hit.is_empty()) {
                    (false, true) => Some(reported()),
                    (true, false) => Some(format!("reported {}", outcome_list(&hit.into_iter().cloned().collect::<Vec<_>>()))),
                    _ => None,
                }
            }
            Expected::Row { relation, values } => {
                let all = rows(relation)?;
                let hit = all.iter().any(|row| row.len() == values.len() && row.iter().zip(values).all(|(have, want)| want.as_ref().is_none_or(|(w, _)| w == have)));
                match (e.negated, hit) {
                    (false, false) => Some(format!("{relation} has {} rows{}", all.len(), sample(&all))),
                    (true, true) => Some(format!("{relation} has such a row{}", sample(&all))),
                    _ => None,
                }
            }
            Expected::Count { name, n } => {
                let have = if declared.contains(name) { rows(name)?.len() } else { findings.iter().filter(|f| f.code == *name).count() };
                (have != *n).then(|| format!("counted {have}"))
            }
        };
        if let Some(why) = failed {
            outcome.failures.push(format!("line {}: .expect {e}: {why}", e.line));
        }
    }
    if test.exactly {
        let extra: Vec<Finding> = findings.iter().filter(|f| !test.expects.iter().any(|e| matches(&e.what, f))).cloned().collect();
        if !extra.is_empty() {
            outcome.failures.push(format!("line {}: exactly: also reported {}", test.line, outcome_list(&extra)));
        }
    }
    Ok(outcome)
}

fn outcome_list(findings: &[Finding]) -> String {
    let all: Vec<String> = findings.iter().map(ToString::to_string).collect();
    if all.len() > 5 { format!("{} and {} more", all[..5].join(", "), all.len() - 5) } else { all.join(", ") }
}

fn sample(rows: &[Vec<String>]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let shown: Vec<String> = rows.iter().take(3).map(|r| format!("({})", r.join(", "))).collect();
    format!(": {}{}", shown.join(", "), if rows.len() > 3 { ", …" } else { "" })
}

/// Every `.rr` file under each of `paths` (a file, or a directory read recursively), sorted.
pub fn rule_files(paths: &[PathBuf]) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|e| e == "rr") {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    for p in paths {
        if p.is_dir() { walk(p, &mut out) } else { out.push(p.clone()) }
    }
    out.sort();
    out
}

/// Whether a file's text may hold a test, without reading its tape: most rule files hold none.
pub fn may_hold_tests(text: &str) -> bool {
    text.lines().any(|l| l.trim_start().starts_with(".test "))
}

/// `rerail-engine test-rr <file or directory>... [--filter <text>]`: each test of each file, one line each; an error
/// (the exit status) when any failed.
pub fn command(args: &[String]) -> Result<String, String> {
    let (mut paths, mut filter) = (Vec::new(), None);
    let mut args = args.iter();
    while let Some(a) = args.next() {
        if a == "--filter" {
            filter = Some(args.next().ok_or("--filter <text>")?.clone());
        } else {
            paths.push(PathBuf::from(a));
        }
    }
    if paths.is_empty() {
        return Err("usage: rerail-engine test-rr <file or directory>... [--filter <text>]".into());
    }
    // The runs' directories, removed when the command ends.
    struct Work(PathBuf);
    impl Drop for Work {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let work = Work(std::env::temp_dir().join(format!("rerail-test-rr-{}-{nanos}", std::process::id())));
    let (mut text, mut passed, mut failed) = (String::new(), 0, 0);
    for (k, file) in rule_files(&paths).into_iter().enumerate() {
        if !std::fs::read_to_string(&file).is_ok_and(|t| may_hold_tests(&t)) {
            continue;
        }
        for (j, test) in tests(&file)?.into_iter().enumerate() {
            let id = format!("{}::{}", file.display(), test.name);
            if filter.as_ref().is_some_and(|f| !id.contains(f.as_str())) {
                continue;
            }
            match run(&file, &test, &work.0.join(format!("{k}-{j}"))) {
                Ok(outcome) if outcome.passed() => {
                    passed += 1;
                    text.push_str(&format!("pass {id}\n"));
                }
                Ok(outcome) => {
                    failed += 1;
                    text.push_str(&format!("FAIL {id}\n"));
                    for f in outcome.failures {
                        text.push_str(&format!("    {f}\n"));
                    }
                }
                Err(e) => {
                    failed += 1;
                    text.push_str(&format!("FAIL {id}\n    {e}\n"));
                }
            }
        }
    }
    text.push_str(&format!("{passed} passed, {failed} failed\n"));
    if failed > 0 {
        print!("{text}");
        Err(format!("{failed} tests failed"))
    } else {
        Ok(text)
    }
}
