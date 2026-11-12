//! Testing mode (decision 0119): the tests a `.rr` file carries, each run as the file's program over its fixtures.
//!
//! A test is `.test name [exactly] [uses a, b]` … `.end` (`syntax::Item::Test`); a fixture set, `.fixtures name` …
//! `.end` (`syntax::Item::Fixtures`), is what the tests that name it in `uses` read too. Lowering never reads either,
//! so a file's program is the same with its tests or without them.
//!
//! Here a test's program is the file's statements with the test's own (declarations, clauses, facts) spliced in after
//! them, read as `read_file` reads the file (`rr::lower_files`). A `.given r` drops every rule of `r` first, so `r`
//! holds the test's facts and nothing else: a unit test of the rules that read it. The run is `run-rr`'s
//! (`tools::run_program`) over a directory holding the test's fixture files (none, for a unit test). Its `.expect`s
//! are checked against the run's findings (finding keys: code, path, line, column) and the rows of the relations they
//! name.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::syntax::{self, Item, Statement, Tok};

/// One test of a file, its fixture sets merged in.
#[derive(Clone, Debug)]
pub struct Test {
    pub name: String,
    /// The line of its `.test`.
    pub line: usize,
    pub exactly: bool,
    /// Each fixture file's path under the run's root, and its bytes.
    pub fixtures: Vec<(String, Vec<u8>)>,
    /// The relations whose rules the test drops (`.given`): their rows are the test's facts.
    pub givens: Vec<(String, usize)>,
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

/// A value of a row assertion: as the run writes it, and whether it was written as a string.
pub type Value = Option<(String, bool)>;

#[derive(Clone, Debug, PartialEq)]
pub enum Expected {
    /// A finding: its code (`_`: any), and where (`at "path"[:line[:column]]`); what is left out matches anything.
    Finding { code: String, path: Option<String>, line: Option<i64>, column: Option<i64> },
    /// A row of a relation; `None` is `_`. `only`: the relation's rows are the ones its `only` lines name.
    Row { relation: String, values: Vec<Value>, only: bool },
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
            Expected::Row { relation, values, only } => {
                let only = if *only { "only " } else { "" };
                write!(f, "{no}{only}{relation}({})", values.iter().map(show).collect::<Vec<_>>().join(", "))
            }
            Expected::Count { name, n } => write!(f, "count {name} = {n}"),
        }
    }
}

fn show(v: &Value) -> String {
    match v {
        None => "_".to_owned(),
        Some((v, true)) => format!("{v:?}"),
        Some((v, false)) => v.clone(),
    }
}

fn bad<T>(path: &Path, line: usize, what: impl std::fmt::Display) -> Result<T, String> {
    Err(format!("{}:{line}: {what}", path.display()))
}

/// What a test or a fixture set holds, read from its statements.
#[derive(Default)]
struct Block {
    fixtures: Vec<(String, Vec<u8>)>,
    givens: Vec<(String, usize)>,
    rules: Vec<Statement>,
    expects: Vec<Expect>,
}

fn block(path: &Path, items: Vec<Statement>, test: bool) -> Result<Block, String> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let what = if test { "a test holds `.fixture`, `.given`, `.expect`, declarations and clauses" } else { "a fixture set holds `.fixture`, `.given`, declarations and clauses" };
    let mut b = Block::default();
    for (item, at) in items {
        match item {
            Item::Setting { key, text } if key == "fixture" => b.fixtures.push(fixture(path, dir, at, &text)?),
            Item::Setting { key, text } if key == "given" => {
                b.givens.extend(text.split(',').map(str::trim).filter(|r| !r.is_empty()).map(|r| (r.to_owned(), at)));
            }
            Item::Setting { key, text } if key == "expect" && test => b.expects.push(expect(path, at, &text)?),
            Item::Setting { key, .. } if key != "input" && key != "output" => return bad(path, at, format!(".{key}: {what}")),
            Item::Include(_) | Item::Policy(_) | Item::Violation { .. } => return bad(path, at, what),
            item => b.rules.push((item, at)),
        }
    }
    Ok(b)
}

/// The tests of the `.rr` file at `path`, in order, each with the fixture sets it uses merged in (theirs first).
pub fn tests(path: &Path) -> Result<Vec<Test>, String> {
    let (statements, _) = super::read(path).map_err(|e| e.to_string())?;
    let mut sets: BTreeMap<String, Block> = BTreeMap::new();
    for (item, line) in &statements {
        if let Item::Fixtures { name, items } = item {
            if sets.insert(name.clone(), block(path, items.clone(), false)?).is_some() {
                return bad(path, *line, format!("a second fixture set named {name}"));
            }
        }
    }
    let mut out: Vec<Test> = Vec::new();
    for (item, line) in statements {
        let Item::Test { name, exactly, uses, items } = item else { continue };
        if out.iter().any(|t| t.name == name) {
            return bad(path, line, format!("a second test named {name}"));
        }
        let own = block(path, items, true)?;
        let mut test = Test { name, line, exactly, fixtures: Vec::new(), givens: Vec::new(), rules: Vec::new(), expects: own.expects };
        for set in &uses {
            let Some(b) = sets.get(set) else { return bad(path, line, format!("no fixture set named {set}")) };
            test.fixtures.extend(b.fixtures.iter().cloned());
            test.givens.extend(b.givens.iter().cloned());
            test.rules.extend(b.rules.iter().cloned());
        }
        test.fixtures.extend(own.fixtures);
        test.givens.extend(own.givens);
        test.rules.extend(own.rules);
        let mut paths = BTreeSet::new();
        if let Some((name, _)) = test.fixtures.iter().find(|(n, _)| !paths.insert(n.clone())) {
            return bad(path, test.line, format!("two fixtures at {name:?}"));
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
    let spanned = syntax::tokens(text).map_err(|e| format!("{}:{line}: {e:?}", path.display()))?;
    let toks: Vec<&Tok> = spanned.iter().map(|(t, _, _)| t).collect();
    let name = match toks.as_slice() {
        [Tok::Str(name), ..] => name.clone(),
        _ => return bad(path, line, "`.fixture \"<path>\"` then its text"),
    };
    if name.starts_with('/') || name.split('/').any(|part| part == "..") {
        return bad(path, line, format!("a fixture's path is under the test's root: {name:?}"));
    }
    match toks.as_slice() {
        // The fence's body as written: the lexer drops a space inside a fence of two or more, which would take one
        // from the closing fence's indent.
        [_, Tok::Tick(_)] => {
            let (_, start, end) = spanned[1];
            let raw = &text[start..end];
            let n = raw.bytes().take_while(|&b| b == b'`').count();
            Ok((name, dedent(&raw[n..raw.len() - n]).into_bytes()))
        }
        [_, Tok::Id(from), Tok::Str(file)] if from == "from" => {
            std::fs::read(dir.join(file)).map(|bytes| (name, bytes)).map_err(|e| format!("{}:{line}: {file}: {e}", path.display()))
        }
        _ => bad(path, line, "`.fixture \"<path>\"` then a fenced text or `from \"<file>\"`"),
    }
}

/// A fence's body as a file, its first newline gone and ending in a newline. The closing fence, on a line of its own,
/// says the indent: each line loses that much leading whitespace, so a file whose lines all share a deeper indent
/// keeps it (as Java's text blocks and Swift's multi-line strings are read). A closing fence after text on its line
/// says none, and the lines lose the indent they share.
pub fn dedent(body: &str) -> String {
    let body = body.strip_prefix("\r\n").or_else(|| body.strip_prefix('\n')).unwrap_or(body);
    let mut lines: Vec<&str> = body.split('\n').collect();
    let indent = match lines.last() {
        Some(last) if lines.len() > 1 && last.trim().is_empty() => {
            let indent = last.len();
            lines.pop();
            indent
        }
        _ => lines.iter().filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0),
    };
    let mut out = String::new();
    for l in lines {
        let blanks = l.len() - l.trim_start().len();
        out.push_str(&l[blanks.min(indent)..]);
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
    let only = !negated && toks.len() > 1 && word(toks[0], "only") && matches!(toks[1], Tok::Id(_));
    if only {
        toks = &toks[1..];
    }
    let usage = "`.expect [no] <code or _> [at \"<path>\"[:<line>[:<column>]]]`, `.expect [no | only] <relation>(<value>, …)` or `.expect count <name> = <n>`";
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
        return Ok(Expect { line, negated, what: Expected::Row { relation: name, values, only } });
    }
    if only {
        return bad(path, line, usage);
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
    (code == "_" || *code == f.code)
        && path.as_ref().is_none_or(|p| *p == f.path)
        && line.is_none_or(|l| l == f.line)
        && column.is_none_or(|c| f.column.is_none_or(|fc| fc == c))
}

fn row_matches(values: &[Value], row: &[String]) -> bool {
    row.len() == values.len() && row.iter().zip(values).all(|(have, want)| want.as_ref().is_none_or(|(w, _)| w == have))
}

/// Runs `test` of the file at `path` in `work` (a directory of its own, made empty first): its fixture files under
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
    let id = format!("{} test {}", path.display(), test.name);
    // The file's program; each `.given` relation's rules dropped, wherever they are; the test's rules spliced in after
    // the file's own statements.
    let (mut modules, files) = super::program(path).map_err(|e| e.to_string())?;
    let given: BTreeSet<&str> = test.givens.iter().map(|(r, _)| r.as_str()).collect();
    for (relation, line) in &test.givens {
        if super::core_names().contains(relation) {
            return bad(path, *line, format!(".given {relation}: a core relation is the engine's; give a `.fixture` file instead"));
        }
        let declared = modules.iter().flat_map(|(_, s)| s).chain(&test.rules).any(|(item, _)| matches!(item, Item::Decl { name, .. } if name == relation));
        if !declared {
            return bad(path, *line, format!(".given {relation}: no rule declares it"));
        }
    }
    for (_, statements) in &mut modules {
        statements.retain(|(item, _)| !matches!(item, Item::Clause { heads, .. } if heads.iter().any(|h| given.contains(h.rel.as_str()))));
    }
    let own = path.canonicalize().map_err(|e| format!("{}: {e}", path.display()))?;
    let Some((_, statements)) = modules.iter_mut().find(|(p, _)| *p == own) else {
        return Err(format!("{}: not among its program's modules", path.display()));
    };
    statements.extend(test.rules.iter().cloned());
    let mut program = super::lower_files(path, modules).map_err(|e| format!("{id}: {e}"))?;
    // The relations the test's `.expect`s read are outputs of its run.
    let declared: BTreeSet<String> = program.decls.keys().cloned().collect();
    for e in &test.expects {
        let relation = match &e.what {
            Expected::Row { relation, .. } => relation,
            Expected::Count { name, .. } if declared.contains(name) => name,
            _ => continue,
        };
        if !declared.contains(relation) {
            return bad(path, e.line, format!("`.expect` names {relation}, which no rule declares"));
        }
        if !program.outputs.contains(relation) {
            program.outputs.push(relation.clone());
        }
    }
    let report = crate::code_rule_dsl::tools::run_program(path, program, &files, &root, &out, &serde_json::Value::Null).map_err(|e| format!("{id}: {e}"))?;
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
    let mut fail = |line: usize, what: String| outcome.failures.push(format!("line {line}: {what}"));
    for e in &test.expects {
        let failed = match &e.what {
            Expected::Finding { .. } => {
                let hit: Vec<Finding> = findings.iter().filter(|f| matches(&e.what, f)).cloned().collect();
                match (e.negated, hit.is_empty()) {
                    (false, true) => Some(if findings.is_empty() { "no findings".to_owned() } else { format!("reported {}", list(&findings)) }),
                    (true, false) => Some(format!("reported {}", list(&hit))),
                    _ => None,
                }
            }
            Expected::Row { relation, values, .. } => {
                let all = rows(relation)?;
                let hit = all.iter().any(|row| row_matches(values, row));
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
            fail(e.line, format!(".expect {e}: {why}"));
        }
    }
    // `only r(…)`: r's rows are the ones its `only` lines name; the first such line reports the others.
    let mut seen = BTreeSet::new();
    for e in &test.expects {
        let Expected::Row { relation, only: true, .. } = &e.what else { continue };
        if !seen.insert(relation.clone()) {
            continue;
        }
        let named: Vec<&Vec<Value>> = test.expects.iter().filter_map(|e| match &e.what {
            Expected::Row { relation: r, values, only: true } if r == relation => Some(values),
            _ => None,
        }).collect();
        let extra: Vec<Vec<String>> = rows(relation)?.into_iter().filter(|row| !named.iter().any(|v| row_matches(v, row))).collect();
        if !extra.is_empty() {
            fail(e.line, format!(".expect only {relation}: also {}{}", extra.len(), sample(&extra)));
        }
    }
    if test.exactly {
        let extra: Vec<Finding> = findings.iter().filter(|f| !test.expects.iter().any(|e| matches(&e.what, f))).cloned().collect();
        if !extra.is_empty() {
            fail(test.line, format!("exactly: also reported {}", list(&extra)));
        }
    }
    Ok(outcome)
}

fn list(findings: &[Finding]) -> String {
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

// ---------------------------------------------------------------- printing

/// The text of each `.test` and `.fixtures` block of `src`, as written, in order: what a printer of the file's
/// program (`flatten`) adds after it, so the file's tests go with it.
pub fn blocks(src: &str) -> Vec<String> {
    use super::tape::Kind;
    let mut parser = syntax::Parser::taped(src);
    let _ = parser.statements();
    let (nodes, _) = parser.into_tape();
    nodes
        .iter()
        .filter(|n| n.parent == Some(0) && !n.extra && (n.kind == Kind::TestBlock as u16 || n.kind == Kind::FixturesBlock as u16))
        .map(|n| src[n.start as usize..n.end as usize].to_owned())
        .collect()
}

/// A test as `.rr` text: its fixtures as fenced texts (each fence longer than any run of backticks it holds), then
/// its `.expect` lines (as written after `.expect`).
pub fn print_test(name: &str, exactly: bool, fixtures: &[(String, String)], expects: &[String]) -> String {
    let mut out = format!(".test {name}{}\n", if exactly { " exactly" } else { "" });
    for (path, text) in fixtures {
        let run = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
        let fence = "`".repeat((run + 1).max(3));
        out.push_str(&format!("    .fixture {} {fence}\n", syntax::quote(path)));
        for line in text.strip_suffix('\n').unwrap_or(text).split('\n') {
            if line.trim().is_empty() {
                out.push('\n');
            } else {
                out.push_str(&format!("        {line}\n"));
            }
        }
        out.push_str(&format!("        {fence}\n"));
    }
    for e in expects {
        out.push_str(&format!("    .expect {e}\n"));
    }
    out.push_str(".end\n");
    out
}

/// A test name from free text: lower-case words joined by `_`, starting with a letter.
pub fn test_name(text: &str) -> String {
    let mut name = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            name.push(c.to_ascii_lowercase());
        } else if !name.ends_with('_') && !name.is_empty() {
            name.push('_');
        }
    }
    let name = name.trim_end_matches('_').to_owned();
    if name.starts_with(|c: char| c.is_ascii_alphabetic()) { name } else { format!("t_{name}") }
}

/// A rule draft's cases (`{ caught: [{path, content}], passed: [...] }`, `rerail rules test`'s envelope) as tests: a
/// caught case is a test that its file has a finding (`.expect _ at "path"`), a passed case one that it has none.
pub fn tests_of_cases(cases: &serde_json::Value) -> Result<String, String> {
    let mut out = Vec::new();
    for (group, negated) in [("caught", false), ("passed", true)] {
        for (k, case) in cases[group].as_array().into_iter().flatten().enumerate() {
            let path = case["path"].as_str().ok_or(format!("{group}[{k}] has no path"))?;
            let content = case["content"].as_str().ok_or(format!("{group}[{k}] has no content"))?;
            let name = format!("{group}_{}_{}", k + 1, test_name(path));
            let expect = format!("{}_ at {}", if negated { "no " } else { "" }, syntax::quote(path));
            out.push(print_test(&name, false, &[(path.to_owned(), content.to_owned())], &[expect]));
        }
    }
    Ok(out.join("\n"))
}

// ---------------------------------------------------------------- the command

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

/// Each test of each `.rr` file under `paths` whose `<file>::<name>` contains `filter`: `[{file, test, line, status:
/// "pass" | "fail", failures, findings}]`.
pub fn report(paths: &[PathBuf], filter: Option<&str>) -> Result<serde_json::Value, String> {
    // The runs' directories, removed when the report is made.
    struct Work(PathBuf);
    impl Drop for Work {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let work = Work(std::env::temp_dir().join(format!("rerail-test-rr-{}-{nanos}", std::process::id())));
    // A store of testing mode's own (in the engine's tests, each test's: decision 0118).
    let _store = (cfg!(not(any(test, feature = "test-hooks"))) && std::env::var_os("RERAIL_TAPE_CACHE").is_none())
        .then(|| crate::code_rule_dsl::logic::plan::corpus::use_store(testing_store()));
    let mut rows = Vec::new();
    for (k, file) in rule_files(paths).into_iter().enumerate() {
        if !std::fs::read_to_string(&file).is_ok_and(|t| may_hold_tests(&t)) {
            continue;
        }
        for (j, test) in tests(&file)?.into_iter().enumerate() {
            let id = format!("{}::{}", file.display(), test.name);
            if filter.is_some_and(|f| !id.contains(f)) {
                continue;
            }
            let (failures, findings) = match run(&file, &test, &work.0.join(format!("{k}-{j}"))) {
                Ok(outcome) => (outcome.failures, outcome.findings),
                Err(e) => (vec![e], Vec::new()),
            };
            rows.push(serde_json::json!({
                "file": file, "test": test.name, "line": test.line,
                "status": if failures.is_empty() { "pass" } else { "fail" },
                "failures": failures, "findings": findings,
            }));
        }
    }
    Ok(serde_json::Value::Array(rows))
}

/// Testing mode's store, one per build of the engine, kept between runs: `$TMPDIR/rerail-test-rr-stores/<build>.sqlite`.
/// Not the project's store (`corpus::tape_cache_path`): a test's run would read what real runs left there, and wait
/// behind their writes (O-test-rr-busy). Another build's store unused for a day is removed.
fn testing_store() -> PathBuf {
    crate::code_rule_dsl::logic::plan::corpus::build_store("rerail-test-rr-stores", None)
}

/// `rerail-engine test-rr <file or directory>... [--filter <text>] [--json]`: each test of each file, one line each
/// (or the report as JSON); an error (the exit status) when any failed.
pub fn command(args: &[String]) -> Result<String, String> {
    let (mut paths, mut filter, mut json) = (Vec::new(), None, false);
    let mut args = args.iter();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--filter" => filter = Some(args.next().ok_or("--filter <text>")?.clone()),
            "--json" => json = true,
            _ => paths.push(PathBuf::from(a)),
        }
    }
    if paths.is_empty() {
        return Err("usage: rerail-engine test-rr <file or directory>... [--filter <text>] [--json]".into());
    }
    let rows = report(&paths, filter.as_deref())?;
    let rows = rows.as_array().expect("rows");
    let failed = rows.iter().filter(|r| r["status"] == "fail").count();
    let text = if json {
        serde_json::to_string_pretty(rows).map_err(|e| e.to_string())? + "\n"
    } else {
        let mut text = String::new();
        for r in rows {
            let id = format!("{}::{}", r["file"].as_str().unwrap_or_default(), r["test"].as_str().unwrap_or_default());
            text.push_str(&format!("{} {id}\n", if r["status"] == "pass" { "pass" } else { "FAIL" }));
            for f in r["failures"].as_array().into_iter().flatten() {
                text.push_str(&format!("    {}\n", f.as_str().unwrap_or_default()));
            }
        }
        text.push_str(&format!("{} passed, {failed} failed\n", rows.len() - failed));
        text
    };
    if failed > 0 {
        print!("{text}");
        Err(format!("{failed} tests failed"))
    } else {
        Ok(text)
    }
}
