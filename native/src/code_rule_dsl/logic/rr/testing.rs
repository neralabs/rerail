//! Testing mode (decision 0130): the tests a `.rerail` file carries, each run as the file's program over its fixtures.
//!
//! A test is `.test name [exactly] [uses a, b]` … `.end` (`from_tape::S::Test`); a fixture set, `.fixtures name` …
//! `.end` (`from_tape::S::Fixtures`), is what the tests that name it in `uses` read too. Lowering never reads either,
//! so a file's program is the same with its tests or without them.
//!
//! Here a test's program is the file's statements with the test's own (declarations, clauses, facts) spliced in after
//! them, read as `read_file` reads the file (`rr::lower_files`). A `.given r` drops every rule of `r` first, so `r`
//! holds the test's facts and nothing else: a unit test of the rules that read it. The run is `run-rr`'s
//! (`tools::run_program`) over a directory holding the test's fixture files (none, for a unit test), and given the
//! test's tool calls (`.tool_call "Bash" `{"command": "…"}``, the `toolCalls` option: `docs/rule-syntax/tool-calls.md`),
//! whose ids are "1", "2", … in order. Its `.expect`s are checked against the run's findings (finding keys: code,
//! path, line, column; a tool call's has path "" and line 0) and the rows of the relations they name.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::from_tape::{S, Stmt};
use super::syntax::{self, Tok};

/// One test of a file, its fixture sets merged in.
#[derive(Clone, Debug)]
pub struct Test {
    pub name: String,
    /// The line of its `.test`.
    pub line: usize,
    pub exactly: bool,
    /// Each fixture file's path under the run's root, and its bytes.
    pub fixtures: Vec<(String, Vec<u8>)>,
    /// The tool calls the run is given, in order: each `{"tool", "input"}` (`.tool_call`); its id is its place, from 1.
    pub calls: Vec<serde_json::Value>,
    /// The relations whose rules the test drops (`.given`): their rows are the test's facts.
    pub givens: Vec<(String, usize)>,
    /// The test's own declarations, clauses and facts.
    pub rules: Vec<Stmt>,
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
#[derive(Clone, Default)]
struct Block {
    fixtures: Vec<(String, Vec<u8>)>,
    calls: Vec<serde_json::Value>,
    givens: Vec<(String, usize)>,
    rules: Vec<Stmt>,
    expects: Vec<Expect>,
}

fn block(path: &Path, items: Vec<Stmt>, test: bool) -> Result<Block, String> {
    let mut b = Block::default();
    for st in items {
        let at = st.line();
        match st.read().map_err(|e| format!("{}:{at}: {e}", path.display()))? {
            // An `.input` or `.output` of the block's own is one of its statements.
            S::Setting { key, text } => {
                if b.setting(path, at, key, text, test)? {
                    b.rules.push(st.clone());
                }
            }
            S::Include(_) | S::Policy | S::Violation(_) => return bad(path, at, holds(test)),
            _ => b.rules.push(st.clone()),
        }
    }
    Ok(b)
}

/// What a test, or a fixture set, may hold.
fn holds(test: bool) -> &'static str {
    if test { "a test holds `.fixture`, `.tool_call`, `.given`, `.expect`, declarations and clauses" } else { "a fixture set holds `.fixture`, `.tool_call`, `.given`, declarations and clauses" }
}

impl Block {
    /// A directive of the block at line `at`: a fixture, a tool call, the relations given, an `.expect` (a test's
    /// only), or an `.input` or `.output` of its own, which is one of the block's statements (true).
    fn setting(&mut self, path: &Path, at: usize, key: &str, text: &str, test: bool) -> Result<bool, String> {
        let dir = path.parent().unwrap_or(Path::new("."));
        match key {
            "fixture" => self.fixtures.push(fixture(path, dir, at, text)?),
            "tool_call" => self.calls.push(tool_call(path, at, text)?),
            "given" => self.givens.extend(text.split(',').map(str::trim).filter(|r| !r.is_empty()).map(|r| (r.to_owned(), at))),
            "expect" if test => self.expects.push(expect(path, at, text)?),
            "input" | "output" => return Ok(true),
            _ => return bad(path, at, format!(".{key}: {}", holds(test))),
        }
        Ok(false)
    }
}

/// The tests of the `.rerail` file at `path`, in order, each with the fixture sets it uses merged in (theirs first).
pub fn tests(path: &Path) -> Result<Vec<Test>, String> {
    let (statements, _) = super::read(path).map_err(|e| e.to_string())?;
    let sets = fixture_sets(path, &statements)?;
    let mut out: Vec<Test> = Vec::new();
    for st in statements.iter().filter(|s| s.what() == Some(S::Test)) {
        let line = st.line();
        let (name, exactly, uses, items) = block_of(path, st)?;
        if out.iter().any(|t| t.name == name) {
            return bad(path, line, format!("a second test named {name}"));
        }
        out.push(test(path, line, (name, exactly), &uses, items, &sets)?);
    }
    Ok(out)
}

/// The file's fixture sets (`.fixtures name` … `.end`), by name.
fn fixture_sets(path: &Path, statements: &[Stmt]) -> Result<BTreeMap<String, Block>, String> {
    let mut sets: BTreeMap<String, Block> = BTreeMap::new();
    for st in statements.iter().filter(|s| s.what() == Some(S::Fixtures)) {
        let (name, _, _, items) = block_of(path, st)?;
        if sets.insert(name.clone(), block(path, items, false)?).is_some() {
            return bad(path, st.line(), format!("a second fixture set named {name}"));
        }
    }
    Ok(sets)
}

/// A block's name, `exactly`, `uses` and statements: nodes of the file's tape, as its other statements are.
fn block_of(path: &Path, st: &Stmt) -> Result<(String, bool, Vec<String>, Vec<Stmt>), String> {
    let (t, n) = st.tape();
    t.block(n)
        .map(|(name, exactly, uses, body)| (name, exactly, uses, body.into_iter().map(|c| Stmt::at(st.file(), c)).collect()))
        .map_err(|e| format!("{}:{}: {e}", path.display(), st.line()))
}

/// The test `name` (`exactly` or not) at `line`: its own block (`items`) after the fixture sets it uses.
fn test(path: &Path, line: usize, (name, exactly): (String, bool), uses: &[String], items: Vec<Stmt>, sets: &BTreeMap<String, Block>) -> Result<Test, String> {
    let own = block(path, items, true)?;
    let mut test = Test { name, line, exactly, fixtures: Vec::new(), calls: Vec::new(), givens: Vec::new(), rules: Vec::new(), expects: Vec::new() };
    for set in uses {
        let Some(b) = sets.get(set) else { return bad(path, line, format!("no fixture set named {set}")) };
        test.take(b.clone());
    }
    test.take(own);
    checked(path, test)
}

/// `test`, or why it cannot run: two fixtures at one path, or `exactly` beside an `.expect no`.
fn checked(path: &Path, test: Test) -> Result<Test, String> {
    let mut paths = BTreeSet::new();
    if let Some((name, _)) = test.fixtures.iter().find(|(n, _)| !paths.insert(n.clone())) {
        return bad(path, test.line, format!("two fixtures at {name:?}"));
    }
    if let Some(e) = test.expects.iter().find(|e| test.exactly && e.negated) {
        return bad(path, e.line, "`exactly` lists the findings a test has: no `.expect no` beside it");
    }
    Ok(test)
}

impl Test {
    /// A block's fixtures, tool calls, givens, rules and expectations, after those it has.
    fn take(&mut self, b: Block) {
        self.fixtures.extend(b.fixtures);
        self.calls.extend(b.calls);
        self.givens.extend(b.givens);
        self.rules.extend(b.rules);
        self.expects.extend(b.expects);
    }
}

/// `.fixture "path" <fenced text>` or `.fixture "path" from "file"` (beside the `.rerail` file).
fn fixture(path: &Path, dir: &Path, line: usize, text: &str) -> Result<(String, Vec<u8>), String> {
    let spanned = syntax::tokens(text).map_err(|e| format!("{}:{line}: {e:?}", path.display()))?;
    let toks: Vec<&Tok> = spanned.iter().map(|(t, _, _)| t).collect();
    let name = match toks.as_slice() {
        [Tok::Str(name), ..] => name.to_string(),
        _ => return bad(path, line, "`.fixture \"<path>\"` then its text"),
    };
    if !under_root(&name) {
        return bad(path, line, format!("a fixture's path is under the test's root: {name:?}"));
    }
    let usage = "`.fixture \"<path>\"` then a fenced text or `from \"<file>\"`";
    match toks.as_slice() {
        [_, Tok::Tick(_)] => fence(text, spanned[1].1, spanned[1].2).map_or_else(|| bad(path, line, usage), |body| Ok((name, dedent(body).into_bytes()))),
        [_, Tok::Id(from), Tok::Str(file)] if *from == "from" => {
            std::fs::read(dir.join(file.as_ref())).map(|bytes| (name, bytes)).map_err(|e| format!("{}:{line}: {file}: {e}", path.display()))
        }
        _ => bad(path, line, usage),
    }
}

/// Whether a fixture's path stays under the test's root: relative, and no `..` in it.
fn under_root(name: &str) -> bool {
    !name.starts_with('/') && !name.split('/').any(|part| part == "..")
}

/// The body of the fence at `start..end` of `text`, as written: the lexer drops a space inside a fence of two or more,
/// which would take one from the closing fence's indent. None for two backticks and nothing else, which the lexer
/// reads as an empty fence.
fn fence(text: &str, start: usize, end: usize) -> Option<&str> {
    let raw = &text[start..end];
    let n = raw.bytes().take_while(|&b| b == b'`').count();
    raw.get(n..raw.len().checked_sub(n)?)
}

/// `.tool_call "<tool>" <fenced JSON object>`: a tool call the test's run is given, the tool's name and its arguments
/// (`{"command": "…"}`), as `{"tool", "input"}`.
fn tool_call(path: &Path, line: usize, text: &str) -> Result<serde_json::Value, String> {
    let usage = "`.tool_call \"<tool>\"` then its arguments, a JSON object in a fence: .tool_call \"Bash\" `{\"command\": \"ls\"}`";
    let spanned = syntax::tokens(text).map_err(|e| format!("{}:{line}: {e:?}", path.display()))?;
    let (Some((Tok::Str(tool), _, _)), Some((Tok::Tick(_), start, end)), None) = (spanned.first(), spanned.get(1), spanned.get(2)) else {
        return bad(path, line, usage);
    };
    match fence(text, *start, *end).map(serde_json::from_str::<serde_json::Value>) {
        Some(Ok(input)) if input.is_object() => Ok(serde_json::json!({ "tool": tool.to_string(), "input": input })),
        Some(Err(e)) => bad(path, line, format!("{usage}: {e}")),
        _ => bad(path, line, usage),
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
    let word = |t: &Tok, w: &str| matches!(t, Tok::Id(s) if *s == w);
    // A name: a word or a string.
    let named = |t: &Tok| match t {
        Tok::Id(s) => Some((*s).to_owned()),
        Tok::Str(s) => Some(s.to_string()),
        _ => None,
    };
    let negated = toks.first().is_some_and(|t| word(t, "no"));
    if negated {
        toks = &toks[1..];
    }
    let only = !negated && toks.len() > 1 && word(toks[0], "only") && matches!(toks[1], Tok::Id(_));
    if only {
        toks = &toks[1..];
    }
    let usage = "`.expect [no] <code or _> [at \"<path>\"[:<line>[:<column>]]]`, `.expect [no | only] <relation>(<value>, …)` or `.expect count <name> = <n>`";
    if let [count, name, Tok::P("="), Tok::Num(n)] = toks
        && word(count, "count")
        && !negated
        && let Some(name) = named(name)
    {
        return Ok(Expect { line, negated, what: Expected::Count { name, n: *n as usize } });
    }
    let (name, rest) = match toks {
        [first, rest @ ..] if let Some(name) = named(first) => (name, rest),
        _ => return bad(path, line, usage),
    };
    if let [Tok::P("("), inner @ .., Tok::P(")")] = rest {
        let mut values = Vec::new();
        for (k, t) in inner.iter().enumerate() {
            match (k % 2, t) {
                (0, Tok::Str(s)) => values.push(Some((s.to_string(), true))),
                (0, Tok::Num(n)) => values.push(Some((n.to_string(), false))),
                (0, Tok::Float(x)) => values.push(Some(((*x).to_owned(), false))),
                (0, Tok::Id(w)) if *w == "_" => values.push(None),
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
            at = Some(p.to_string());
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

#[cfg(test)]
impl Outcome {
    #[cfg(test)]
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

/// A finding's key, from a row of `rr_diagnostic`; `call`: the tool call's id, for a finding at one (path "", line 0).
#[derive(Clone, Debug, PartialEq)]
struct Finding {
    code: String,
    path: String,
    line: i64,
    column: Option<i64>,
    call: Option<String>,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(call) = &self.call {
            return write!(f, "{} at call {call:?}", self.code);
        }
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

/// Whether `row` is one `values` names: a text by its text, any other value as the run shows it.
fn row_matches(values: &[Value], row: &crate::code_rule_dsl::logic::plan::output::Row<'_>) -> bool {
    use crate::code_rule_dsl::logic::plan::output::Value as V;
    row.len() == values.len()
        && row.values().zip(values).all(|(have, want)| {
            want.as_ref().is_none_or(|(w, _)| match have {
                V::Text(t) => t == w,
                v => v.to_string() == *w,
            })
        })
}

/// Runs `test` of the file at `path` in `work` (a directory of its own, made empty first): its fixture files under
/// `work/root`, the run's outputs under `work/out`.
pub fn run(path: &Path, test: &Test, work: &Path) -> Result<Outcome, String> {
    let (root, out) = (work.join("root"), work.join("out"));
    write_fixtures(work, &root, &test.fixtures)?;
    let id = format!("{} test {}", path.display(), test.name);
    let (program, files, declared) = test_program(path, test, &id)?;
    let report = crate::code_rule_dsl::tools::run_program(program, &files, &root, &out, &call_options(&test.calls)).map_err(|e| format!("{id}: {e}"))?;
    let findings = findings_of(&report);
    // The run's outputs, read where they are.
    let outputs = Outputs::open(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut outcome = Outcome { failures: Vec::new(), findings: findings.iter().map(ToString::to_string).collect() };
    expectations(test, &findings, &declared, &outputs, &mut outcome.failures);
    Ok(outcome)
}

/// `work` made empty, and the test's fixture files written under `root`.
fn write_fixtures(work: &Path, root: &Path, fixtures: &[(String, Vec<u8>)]) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(work);
    std::fs::create_dir_all(root).map_err(|e| format!("{}: {e}", root.display()))?;
    for (name, bytes) in fixtures {
        let file = root.join(name);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&file, bytes).map_err(|e| format!("{}: {e}", file.display()))?;
    }
    Ok(())
}

/// The test's program, its library files and the relations it declares: the file's program, each `.given` relation's
/// rules dropped, wherever they are, and the test's rules spliced in after the file's own statements. The relations
/// the test's `.expect`s read are outputs of its run.
fn test_program(path: &Path, test: &Test, id: &str) -> Result<(std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>, Vec<super::LibraryFile>, BTreeSet<String>), String> {
    let (mut modules, files) = super::program(path).map_err(|e| e.to_string())?;
    check_givens(path, test, &modules)?;
    drop_given_rules(test, &mut modules);
    let own = dunce::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let Some((_, statements)) = modules.iter_mut().find(|(p, _)| *p == own) else {
        return Err(format!("{}: not among its program's modules", path.display()));
    };
    statements.extend(test.rules.iter().cloned());
    let read = super::lower_files(path, modules).map_err(|e| format!("{id}: {e}"))?;
    let declared: BTreeSet<String> = read.declared_names().into_iter().collect();
    let mut program = crate::code_rule_dsl::logic::program_image::Builder::on(&read);
    expect_outputs(path, test, &declared, &mut program)?;
    let program = program.finish().map(|(v, _)| v).map_err(|e| format!("{id}: {e}"))?;
    Ok((program, files, declared))
}

/// Each `.given` relation is one a rule declares, and no core relation (the engine's: a `.fixture` file gives it).
fn check_givens(path: &Path, test: &Test, modules: &super::Files) -> Result<(), String> {
    for (relation, line) in &test.givens {
        if super::core_names().contains(relation) {
            return bad(path, *line, format!(".given {relation}: a core relation is the engine's; give a `.fixture` file instead"));
        }
        let declared = modules.iter().flat_map(|(_, s)| s).chain(&test.rules).any(|s| s.what() == Some(S::Decl(relation)));
        if !declared {
            return bad(path, *line, format!(".given {relation}: no rule declares it"));
        }
    }
    Ok(())
}

/// Every rule of a `.given` relation dropped, wherever it is: the relation holds the test's facts alone.
fn drop_given_rules(test: &Test, modules: &mut super::Files) {
    let given: BTreeSet<&str> = test.givens.iter().map(|(r, _)| r.as_str()).collect();
    for (_, statements) in modules {
        statements.retain(|s| !s.heads().iter().any(|h| given.contains(h)));
    }
}

/// The relations the test's `.expect`s read, outputs of `program`; one no rule declares is an error.
fn expect_outputs(path: &Path, test: &Test, declared: &BTreeSet<String>, program: &mut crate::code_rule_dsl::logic::program_image::Builder<'_>) -> Result<(), String> {
    for e in &test.expects {
        let relation = match &e.what {
            Expected::Row { relation, .. } => relation,
            Expected::Count { name, .. } if declared.contains(name) => name,
            _ => continue,
        };
        if !declared.contains(relation) {
            return bad(path, e.line, format!("`.expect` names {relation}, which no rule declares"));
        }
        program.fields().add_output(relation);
    }
    Ok(())
}

/// The test's tool calls as the run is given them (`corpus::added`), each with its id: its place, from 1.
fn call_options(calls: &[serde_json::Value]) -> serde_json::Value {
    if calls.is_empty() {
        return serde_json::Value::Null;
    }
    let calls: Vec<serde_json::Value> = calls.iter().enumerate().map(|(k, call)| {
        let mut call = call.clone();
        call["id"] = serde_json::json!((k + 1).to_string());
        call
    }).collect();
    serde_json::json!({ "toolCalls": calls })
}

/// The run's findings, as keys; a finding at a tool call (no path) keeps the call's id.
fn findings_of(report: &serde_json::Value) -> Vec<Finding> {
    report["diagnostics"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|row| {
            let path = row["path"].as_str().unwrap_or_default().to_owned();
            Finding {
                code: row["code"].as_str().unwrap_or_default().to_owned(),
                call: row["at"].as_str().filter(|_| path.is_empty()).map(str::to_owned),
                path,
                line: row["line"].as_i64().unwrap_or(0),
                column: row["column"].as_i64(),
            }
        })
        .collect()
}

/// A run's outputs, as `.expect`s read them.
type Outputs = crate::code_rule_dsl::logic::plan::output::Outputs;

/// The test's `.expect`s, its `only` rows and its `exactly` checked against the run: each that fails, into `failures`.
fn expectations(test: &Test, findings: &[Finding], declared: &BTreeSet<String>, outputs: &Outputs, failures: &mut Vec<String>) {
    for e in &test.expects {
        if let Some(why) = expect_failure(e, findings, declared, outputs) {
            failures.push(format!("line {}: .expect {e}: {why}", e.line));
        }
    }
    only_failures(test, outputs, failures);
    if test.exactly {
        let extra: Vec<Finding> = findings.iter().filter(|f| !test.expects.iter().any(|e| matches(&e.what, f))).cloned().collect();
        if !extra.is_empty() {
            failures.push(format!("line {}: exactly: also reported {}", test.line, list(&extra)));
        }
    }
}

/// Why `.expect` `e` fails on the run, if it does.
fn expect_failure(e: &Expect, findings: &[Finding], declared: &BTreeSet<String>, outputs: &Outputs) -> Option<String> {
    match &e.what {
        Expected::Finding { .. } => finding_failure(e, findings),
        Expected::Row { relation, values, .. } => {
            let all = outputs.rows(relation);
            let hit = all.iter().any(|row| row_matches(values, row));
            match (e.negated, hit) {
                (false, false) => Some(format!("{relation} has {} rows{}", all.len(), sample(&all))),
                (true, true) => Some(format!("{relation} has such a row{}", sample(&all))),
                _ => None,
            }
        }
        Expected::Count { name, n } => {
            let have = if declared.contains(name) { outputs.rows(name).len() } else { findings.iter().filter(|f| f.code == *name).count() };
            (have != *n).then(|| format!("counted {have}"))
        }
    }
}

/// Why a finding's `.expect` fails: no finding it names, or (`no`) one it names.
fn finding_failure(e: &Expect, findings: &[Finding]) -> Option<String> {
    let hit: Vec<Finding> = findings.iter().filter(|f| matches(&e.what, f)).cloned().collect();
    match (e.negated, hit.is_empty()) {
        (false, true) => Some(if findings.is_empty() { "no findings".to_owned() } else { format!("reported {}", list(findings)) }),
        (true, false) => Some(format!("reported {}", list(&hit))),
        _ => None,
    }
}

/// `only r(…)`: r's rows are the ones its `only` lines name; the first such line reports the others.
fn only_failures(test: &Test, outputs: &Outputs, failures: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    for e in &test.expects {
        let Expected::Row { relation, only: true, .. } = &e.what else { continue };
        if !seen.insert(relation.clone()) {
            continue;
        }
        let named = only_named(test, relation);
        let extra: Vec<_> = outputs.rows(relation).into_iter().filter(|row| !named.iter().any(|v| row_matches(v, row))).collect();
        if !extra.is_empty() {
            failures.push(format!("line {}: .expect only {relation}: also {}{}", e.line, extra.len(), sample(&extra)));
        }
    }
}

/// The rows `relation`'s `only` lines name.
fn only_named<'t>(test: &'t Test, relation: &str) -> Vec<&'t Vec<Value>> {
    test.expects.iter().filter_map(|e| match &e.what {
        Expected::Row { relation: r, values, only: true } if r == relation => Some(values),
        _ => None,
    }).collect()
}

fn list(findings: &[Finding]) -> String {
    let all: Vec<String> = findings.iter().map(ToString::to_string).collect();
    if all.len() > 5 { format!("{} and {} more", all[..5].join(", "), all.len() - 5) } else { all.join(", ") }
}

fn sample(rows: &[crate::code_rule_dsl::logic::plan::output::Row<'_>]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    // The first rows in the order their values show (a relation's rows hold no order of their own).
    let mut shown: Vec<String> = rows.iter().map(|r| format!("({})", r.values().map(|v| v.to_string()).collect::<Vec<_>>().join(", "))).collect();
    shown.sort_unstable();
    shown.truncate(3);
    format!(": {}{}", shown.join(", "), if rows.len() > 3 { ", …" } else { "" })
}

// ---------------------------------------------------------------- printing

#[cfg(test)]
/// The text of each `.test` and `.fixtures` block of `src`, as written, in order: what a printer of the file's
/// program (`flatten`) adds after it, so the file's tests go with it.
#[cfg(test)]
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

/// A test as `.rerail` text: its fixtures as fenced texts (each fence longer than any run of backticks it holds), then
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

/// Every `.rerail` file under each of `paths` (a file, or a directory read recursively), sorted.
pub fn rule_files(paths: &[PathBuf]) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|e| e == "rerail") {
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

/// Each test of each `.rerail` file under `paths` whose `<file>::<name>` contains `filter`: `[{file, test, line, status:
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

/// Testing mode's store, one per build of the engine, kept between runs: `$TMPDIR/rerail-test-rr-stores/<build>.sqlite`,
/// or the next of its slots that no other process owns (`corpus::owner::slot`: `<build>-2.sqlite`, …), so test-rr
/// processes at once each have one. Not the project's store (`corpus::tape_cache_path`): a test's run would read what
/// real runs left there, and wait behind their writes (O-test-rr-busy). Another build's store unused for a day is
/// removed.
fn testing_store() -> PathBuf {
    use crate::code_rule_dsl::logic::plan::corpus;
    corpus::owner::slot(&corpus::build_store("rerail-test-rr-stores", None))
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
