//! The engine's text tools without Node (list step 18; P `spec-tooling.md` §2.2.1–2.2.2): the
//! `rerail-engine` binary (`src/bin/rerail-engine.rs`) and the napi wrappers both call these. Boot's: the engine
//! adds the napi wrapper and boots the program before a command (`code_rule_dsl/tools/napi.rs`).

use rerail_store::Canonical as _;
pub use super::logic::profile::watch::watch;
pub use super::sql_executor::CodeRuleSqlExecutor;
pub use super::logic::settings::{program_text, program_texts, settings};
pub use super::logic::settings::process_counters;
use crate::code_rule_dsl::transducer::packed::rir;
use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;


/// A SQL extension request for the standalone tool. The same session serves the bindings,
/// lowered rule program and subsequent SQL queries; no AST matcher is lowered to SQL.
pub fn sql(store: &std::path::Path, request: &str) -> Result<String, String> {
    // One run (decision 0442): the profile's records are its own.
    let _run = super::logic::run::scope(super::logic::native::width::Settings::from_env);
    let profiling = super::logic::profile::on();
    if profiling { super::logic::profile::reset(); }
    let span = super::logic::profile::span(||"SQL extension request".into());
    let result = sql_request(store,request);
    drop(span);
    if profiling
        && let Some(directory) = super::logic::profile::directory() {
            super::logic::profile::report::write(&directory,"SQL extension request").map_err(|e|e.to_string())?;
        }
    result
}

fn sql_request(store: &std::path::Path, request: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Query {
        sql: String,
        #[serde(default)]
        parameters: Vec<serde_json::Value>,
        max_rows: Option<u32>,
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Stored {
        scope: String,
        paths: Option<Vec<String>>,
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        snapshot: Option<serde_json::Value>,
        stored: Option<Stored>,
        #[serde(default)]
        relations: Vec<serde_json::Value>,
        program: Option<String>,
        dialect: Option<String>,
        max_waves: Option<u32>,
        limits: Option<serde_json::Value>,
        #[serde(default)]
        indexes: Vec<Index>,
        #[serde(default)]
        queries: Vec<Query>,
        #[serde(default)]
        query_plans: Vec<serde_json::Value>,
        #[serde(default)]
        changes: Vec<Vec<serde_json::Value>>,
        #[serde(default)]
        snapshots: Vec<serde_json::Value>,
        #[serde(default)]
        file_changes: Vec<serde_json::Value>,
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Index {
        relation: String,
        columns: Vec<u32>,
    }
    let request: Request = serde_json::from_str(request).map_err(|e| e.to_string())?;
    if request.snapshot.is_some() && request.stored.is_some() {
        return Err("a SQL request selects either stored documents or a snapshot".into());
    }
    let mut session = CodeRuleSqlExecutor::new(store.to_string_lossy().into_owned())
        .map_err(|e| e.to_string())?;
    let result = (|| -> napi::Result<serde_json::Value> {
        if let Some(limits) = request.limits {
            session.set_limits(limits.to_string())?;
        }
        if let Some(snapshot) = request.snapshot {
            session.bind_snapshot(snapshot.to_string())?;
        }
        if let Some(stored) = request.stored {
            session.bind_stored(stored.scope, stored.paths)?;
        }
        for relation in request.relations {
            session.bind_relation(relation.to_string())?;
        }
        let execution = request
            .program
            .map(|p| {
                session.run(
                    p,
                    request.dialect.unwrap_or_else(|| "python".into()),
                    request.max_waves,
                )
            })
            .transpose()?;
        for index in request.indexes {
            session.create_index(index.relation, index.columns)?;
        }
        let mut updates = request
            .changes
            .into_iter()
            .map(|batch| {
                session
                    .apply_changes(
                        serde_json::Value::Array(batch).to_string(),
                        request.max_waves,
                    )
                    .and_then(|s| {
                        serde_json::from_str::<serde_json::Value>(&s)
                            .map_err(|e| napi::Error::from_reason(e.to_string()))
                    })
            })
            .collect::<napi::Result<Vec<_>>>()?;
        for snapshot in request.snapshots {
            updates.push(
                session
                    .apply_snapshot(snapshot.to_string(), request.max_waves)
                    .and_then(|s| {
                        serde_json::from_str::<serde_json::Value>(&s)
                            .map_err(|e| napi::Error::from_reason(e.to_string()))
                    })?,
            );
        }
        for batch in request.file_changes {
            updates.push(
                session
                    .apply_file_changes(batch.to_string(), request.max_waves)
                    .and_then(|s| {
                        serde_json::from_str::<serde_json::Value>(&s)
                            .map_err(|e| napi::Error::from_reason(e.to_string()))
                    })?,
            );
        }
        let queries = request
            .queries
            .into_iter()
            .map(|query| {
                let result = session.query(
                    query.sql,
                    Some(
                        serde_json::to_string(&query.parameters)
                            .map_err(|e| napi::Error::from_reason(e.to_string()))?,
                    ),
                    query.max_rows,
                )?;
                serde_json::from_str::<serde_json::Value>(&result)
                    .map_err(|e| napi::Error::from_reason(e.to_string()))
            })
            .collect::<napi::Result<Vec<_>>>()?;
        let execution = execution
            .map(|s| serde_json::from_str::<serde_json::Value>(&s))
            .transpose()
            .map_err(|e| napi::Error::from_reason(e.to_string()))?;
        let query_plans=request.query_plans.into_iter().map(|plan| {
            let result=session.query_plan(plan.to_string())?;
            serde_json::from_str::<serde_json::Value>(&result)
                .map_err(|e|napi::Error::from_reason(e.to_string()))
        }).collect::<napi::Result<Vec<_>>>()?;
        let mut result=serde_json::json!({"execution": execution, "updates":updates, "queries": queries});
        if !query_plans.is_empty() {result["query_plans"]=serde_json::json!(query_plans);}
        Ok(result)
    })()
    .map_err(|e| e.to_string())?;
    serde_json::to_string_pretty(&result)
        .map(|s| s + "\n")
        .map_err(|e| e.to_string())
}

/// The capabilities rules compile against, as bundled with the engine.
pub const CAPABILITIES: &str = super::rule_api::CAPABILITIES;

/// Read a public `.rerail` file, including its core prelude and libraries. The CLI and migration
/// harness use the same reader as the engine; they never strip directives or run a lab interpreter.
pub fn read_rr(path: &std::path::Path) -> Result<String, String> {
    let program = super::logic::rr::parse_file_version(path).map_err(|e| e.to_string())?;
    super::logic::rr::print_version(&program).map_err(|e| e.to_string())
}

/// Convert whole AST policies for review. Unsupported clauses refuse the entire candidate.
/// Each rule (a rule document, or a draft `{ kind, rule, cases }`) converted to `.rerail`. A draft's cases, or the cases
/// file `--cases` names for the rule after it, become the converted file's tests (decision 0130): `tests` in the
/// report, and after the program in `program`. They are printed even when the rule needs a hand-written port.
pub fn convert_rr(paths: &[std::path::PathBuf]) -> serde_json::Value {
    convert_rr_with(&paths.iter().map(|p| (p.clone(), None)).collect::<Vec<_>>())
}

/// `convert_rr`, each rule with the cases file given for it.
pub fn convert_rr_with(rules: &[(std::path::PathBuf, Option<std::path::PathBuf>)]) -> serde_json::Value {
    let one = |path: &std::path::Path, cases: Option<&std::path::Path>| -> Result<serde_json::Value, String> {
        let source = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let document: serde_json::Value = serde_json::from_str(&source).map_err(|e| e.to_string())?;
        let (rule, mut cases_value) = if document.get("rule").is_some() && document.get("cases").is_some() {
            (document["rule"].to_string(), Some(document["cases"].clone()))
        } else {
            (source, None)
        };
        if let Some(file) = cases {
            let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
            cases_value = Some(serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?);
        }
        let mut report = super::logic::convert::policy(&rule)?;
        if let Some(cases) = cases_value {
            let tests = super::logic::rr::testing::tests_of_cases(&cases)?;
            if let Some(program) = report["program"].as_str().filter(|p| !p.is_empty()) {
                report["program"] = serde_json::json!(format!("{}\n\n{tests}", program.trim_end()));
            }
            report["tests"] = serde_json::json!(tests);
        }
        Ok(report)
    };
    serde_json::Value::Array(rules.iter().map(|(path, cases)| {
        match one(path, cases.as_deref()) {
            Ok(mut report) => { report["path"] = serde_json::json!(path); report },
            Err(error) => serde_json::json!({"path": path, "status": "conversion-failed", "error": error})
        }
    }).collect())
}

/// Check every requested rule and report every refusal. A reader success is not execution parity.
pub fn check_rr(paths: &[std::path::PathBuf]) -> serde_json::Value {
    serde_json::Value::Array(paths.iter().map(|path| {
        match super::logic::rr::read_file(path) {
            // A program the reader passes still has to type: each variable one domain (a kind, a name, …), as a run
            // checks when it plans (`plan::lower::domains`). A run would stop there, so the check does too.
            Ok((program, _)) if let Err(error) = super::logic::plan::corpus::combine_read_program(&program)
                .map_err(|e| e.to_string())
                // Pruned first, as a run is: a library's relation the outputs do not read is not planned.
                .and_then(super::logic::plan::corpus::preparation::pruned)
                .and_then(|pruned| super::logic::plan::lower::domains_of(&pruned).map(|_| ()).map_err(|e| e.to_string())) =>
            {
                serde_json::json!({"path": path, "status": "types-failed", "error": error})
            }
            Ok((program, files)) => serde_json::json!({
                "path": path, "status": "reader-passed", "rules": program.rules().count(),
                "outputs": program.output_names(),
                "files": files.iter().map(|file| serde_json::json!({
                    "path": file.path,
                    "sha256": file.sha256.map(|hash| hash.iter().map(|b| format!("{b:02x}")).collect::<String>())
                })).collect::<Vec<_>>()
            }),
            Err(error) => serde_json::json!({"path": path, "status": "reader-failed", "error": error.to_string()})
        }
    }).collect())
}

/// One strict `.rerail` replay. The caller supplies a corpus and typed policy options; findings are
/// produced by the native relational engine, including the original message and parameters.
pub fn run_rr(
    path: &std::path::Path,
    root: &std::path::Path,
    out: &std::path::Path,
    options: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let mut options = options.clone();
    let mut run = PolicyInvocation::from_options(root, &mut options)?;
    let (program, files) = read_policy(path)?;
    let mut report = run.evaluate(|root| run_program_checked(program.clone(), &files, root, out, &options))?;
    run.report(&mut report, true);
    Ok(report)
}

pub fn read_policy(path: &Path) -> Result<(std::sync::Arc<super::logic::program_image::Version>, Vec<super::logic::rr::LibraryFile>), String> {
    read_policy_after(path, None)
}

pub fn read_policy_after(path: &Path, inspected: Option<&[super::logic::rr::LibraryFile]>) -> Result<(std::sync::Arc<super::logic::program_image::Version>, Vec<super::logic::rr::LibraryFile>), String> {
    let read = inspect_policy_after(path, inspected)?;
    Ok((read.program, read.files))
}

pub fn inspect_policy_after(path: &Path, inspected: Option<&[super::logic::rr::LibraryFile]>) -> Result<super::logic::rr::ReadProgram, String> {
    let read = super::logic::rr::inspect_file_after(path, inspected).map_err(|e| e.to_string())?;
    if !read.program.output_names().iter().any(|name| super::logic::rr::is_diagnostic(name)) {
        return Err(format!("{}: no .violation declarations", path.display()));
    }
    Ok(read)
}

pub fn policy_inputs(files: &[super::logic::rr::LibraryFile]) -> serde_json::Value {
    serde_json::json!(files.iter().map(|file| serde_json::json!({
        "path": file.path,
        "sha256": file.sha256.map(|hash| crate::content_store::observe::hex(&hash)),
    })).collect::<Vec<_>>())
}

enum PolicySource {
    Directory(std::path::PathBuf),
    Revision { expression: String, snapshot: super::logic::strings::git::Snapshot },
}

struct PolicyInvocation {
    source: PolicySource,
    analysis: Option<crate::content_store::analysis_scope::AnalysisRun>,
    warning: Option<String>,
}

impl PolicyInvocation {
    fn from_options(root: &Path, options: &mut serde_json::Value) -> Result<Self, String> {
        let revision = options.get("revision").map(|v| v.as_str().ok_or("revision must be a commit or tree string")).transpose()?;
        if revision.is_none() && options.get("analysisScope").is_some() {
            return Err("--analysis-scope requires --revision".into());
        }
        let invocation = Self::begin(root, revision, options["analysisScope"].as_str())?;
        if let Some(fields) = options.as_object_mut() {
            fields.remove("revision");
            fields.remove("analysisScope");
        }
        Ok(invocation)
    }

    fn begin(root: &Path, revision: Option<&str>, scope: Option<&str>) -> Result<Self, String> {
        use crate::content_store::{ContentStore, analysis_scope::{self, AnalysisRun}};
        let (source, scope) = match revision {
            Some(expression) => {
                let scope = scope.ok_or("--revision requires --analysis-scope <name>")?;
                let snapshot = super::logic::strings::git::Snapshot::scoped(root, expression, scope)?;
                (PolicySource::Revision { expression: expression.to_owned(), snapshot },
                 analysis_scope::named(scope).expect("snapshot validated its scope"))
            }
            None => {
                let scope = crate::git_workspace::snapshot::analysis_scope(root)
                    .unwrap_or_else(|| analysis_scope::directory(root));
                (PolicySource::Directory(root.to_owned()), scope)
            }
        };
        let mut invocation = Self { source, analysis: None, warning: None };
        if revision.is_none() && super::logic::plan::corpus::has_analysis_owner(root) {
            return Ok(invocation);
        }
        if let Some(path) = super::logic::plan::corpus::tape_cache_path() {
            let source = match &invocation.source {
                PolicySource::Directory(root) => analysis_scope::RunSource::directory(root),
                PolicySource::Revision { snapshot, .. } => snapshot.run_source(),
            };
            match ContentStore::open(&path).and_then(|store| AnalysisRun::begin_batch(store, &scope, source)) {
                Ok(run) => invocation.analysis = Some(run),
                Err(error) => invocation.warning = Some(error.to_string()),
            }
        }
        Ok(invocation)
    }

    fn evaluate<T>(&mut self, run: impl Fn(&Path) -> Result<T, ProgramRunError>) -> Result<T, String> {
        let root = match &self.source {
            PolicySource::Directory(root) => root.as_path(),
            PolicySource::Revision { snapshot, .. } => snapshot.root(),
        };
        if self.warning.is_none() {
            let result = match self.analysis.as_ref() {
                Some(analysis) => super::logic::plan::corpus::with_tape_cache_path(
                    analysis.store_path(), Some((root.to_owned(), analysis.owner.clone())), || run(root)),
                None => run(root),
            };
            match result {
                Ok(report) => return Ok(report),
                Err(ProgramRunError::Cache(error)) if self.analysis.is_some() => {
                    self.warning = Some(error.to_string());
                }
                Err(error) => return Err(error.to_string()),
            }
        }
        super::logic::plan::corpus::with_tape_cache_path(Path::new(":memory:"), None, || run(root))
            .map_err(|error| error.to_string())
    }

    fn finish(&mut self, succeeded: bool) -> Option<String> {
        if let Some(mut run) = self.analysis.take() {
            if let Err(error) = run.finish(succeeded && self.warning.is_none()) {
                self.warning.get_or_insert_with(|| error.to_string());
            }
            if let Some(warning) = run.publication_warning() {
                self.warning.get_or_insert(warning);
            }
        }
        self.warning.take()
    }

    fn source_failures(&self) -> Vec<crate::git_workspace::snapshot::SourceFailure> {
        match &self.source {
            PolicySource::Directory(_) => Vec::new(),
            PolicySource::Revision { snapshot, .. } => snapshot.source_failures(),
        }
    }

    fn report(&mut self, report: &mut serde_json::Value, policies_succeeded: bool) {
        if let PolicySource::Revision { expression, snapshot } = &self.source {
            report["source"] = serde_json::json!({"revision": expression, "tree": snapshot.tree(), "commit": snapshot.commit()});
        }
        let failures = self.source_failures();
        let complete = policies_succeeded && failures.is_empty();
        report["status"] = if complete { "completed" } else { "failed" }.into();
        if !failures.is_empty() {
            report["sourceFailures"] = serde_json::json!(failures);
        }
        if let Some(warning) = self.finish(complete) {
            report["cacheWarnings"] = serde_json::json!([warning]);
        }
    }
}

/// The findings of the run whose outputs are in `dir`, as the CLI reports them: every finding relation's rows
/// (`rr::lower::findings`), each as its JSON object.
pub fn diagnostics(dir: &std::path::Path) -> Result<Vec<serde_json::Value>, String> {
    let outputs = crate::code_rule_dsl::logic::plan::output::Outputs::open(dir).map_err(|e| e.to_string())?;
    diagnostic_rows(&outputs)
}

fn diagnostic_rows(outputs: &crate::code_rule_dsl::logic::plan::output::Outputs) -> Result<Vec<serde_json::Value>, String> {
    let spans: std::collections::HashMap<_, _> = outputs.rows("rr_scan_span").iter().filter_map(|row| {
        Some(((row.get(0).text()?, row.raw(1)), serde_json::json!({
            "start": { "line": row.get(2).int()?, "column": row.get(3).int()? },
            "end": { "line": row.get(4).int()?, "column": row.get(5).int()? },
            "text": row.get(6).text()?,
        })))
    }).collect();
    let mut all = Vec::new();
    for rel in outputs.names().into_iter().filter(|r| super::logic::rr::is_diagnostic(r)) {
        for row in outputs.rows(rel) {
            let mut finding = diagnostic(rel, &row)?;
            if matches!(row.get(5), crate::code_rule_dsl::logic::plan::output::Value::Node(_))
                && let Some(span) = row.get(3).text().and_then(|path| spans.get(&(path, row.raw(5))))
            {
                finding["span"] = span.clone();
            }
            all.push(finding);
        }
    }
    Ok(all)
}

/// A finding's row as its JSON object: `policy`, `clause`, `code`, `path`, `line`, `at`, `tag` (`null` for none),
/// `message`, and `parameters` by name. Each value as its column holds it: a text a string, an integer or a node a
/// number (a node's value as the run held it), NULL `null`. Nothing is parsed.
fn diagnostic(rel: &str, row: &crate::code_rule_dsl::logic::plan::output::Row<'_>) -> Result<serde_json::Value, String> {
    use crate::code_rule_dsl::logic::plan::output::Value;
    let value = |c: usize| -> serde_json::Value {
        match row.get(c) {
            Value::Text(t) => serde_json::json!(t),
            Value::Int(n) => serde_json::json!(n),
            Value::Node(_) => serde_json::json!(row.raw(c)),
            Value::Null => serde_json::Value::Null,
        }
    };
    let head = super::logic::rr::DIAGNOSTIC_HEAD;
    if row.len() < head.len() {
        return Err(format!("a finding's row of {} values, not at least {}", row.len(), head.len()));
    }
    let mut finding = serde_json::Map::new();
    for (c, name) in head.iter().enumerate().take(head.len() - 1) {
        finding.insert((*name).to_owned(), value(c));
    }
    let tagged = row.get(head.len() - 1).int() == Some(1);
    let mut c = head.len();
    finding.insert("tag".to_owned(), if tagged { c += 1; value(c - 1) } else { serde_json::Value::Null });
    if rel.starts_with(super::logic::rr::lower::findings::COMMIT_PREFIX) {
        if c + 4 > row.len() { return Err("incomplete Git commit location".to_owned()); }
        let repo = row.get(c).text().ok_or("missing Git repository identity")?;
        let commit = row.get(c+1).text().ok_or("missing Git commit ID")?;
        let field = match row.get(c+2).int() {
            Some(0) => None,
            Some(1) => Some(row.get(c+3).text().ok_or("missing Git commit field")?),
            _ => return Err("invalid Git commit field indicator".to_owned()),
        };
        finding.insert("location".into(), serde_json::json!({"kind": "git-commit", "repository": repo, "commit": commit, "field": field}));
        for column in ["path", "line", "at"] { finding.insert(column.to_owned(), serde_json::Value::Null); }
        c += 4;
    }
    let mut parameters = serde_json::Map::new();
    while c + 1 < row.len() {
        let name = row.get(c).text().ok_or("a finding's parameter name: not a text")?.to_owned();
        parameters.insert(name, value(c + 1));
        c += 2;
    }
    finding.insert("parameters".to_owned(), serde_json::Value::Object(parameters));
    Ok(serde_json::Value::Object(finding))
}

/// `run_rr` of a program already read: a file's, or a test's (decision 0130, `rr::testing`).
pub fn run_program(
    program: std::sync::Arc<super::logic::program_image::Version>,
    files: &[super::logic::rr::LibraryFile],
    root: &std::path::Path,
    out: &std::path::Path,
    options: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    run_program_checked(program, files, root, out, options).map_err(|error| error.to_string())
}

#[derive(Debug)]
pub enum ProgramRunError {
    Evaluation(String),
    Cache(crate::content_store::StoreError),
}

impl std::fmt::Display for ProgramRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Evaluation(error) => f.write_str(error),
            Self::Cache(error) => error.fmt(f),
        }
    }
}

impl From<String> for ProgramRunError {
    fn from(reason: String) -> Self {
        Self::Evaluation(reason)
    }
}

impl From<super::logic::plan::Error> for ProgramRunError {
    fn from(error: super::logic::plan::Error) -> Self {
        match error {
            super::logic::plan::Error::Cache(error) => Self::Cache(error),
            other => Self::Evaluation(other.to_string()),
        }
    }
}

pub fn run_program_checked(
    mut program: std::sync::Arc<super::logic::program_image::Version>,
    files: &[super::logic::rr::LibraryFile],
    root: &std::path::Path,
    out: &std::path::Path,
    options: &serde_json::Value,
) -> Result<serde_json::Value, ProgramRunError> {
    use super::{
        logic::{plan::corpus, rr},
        typing::file_local::Binding,
    };
    let metadata = rr::metadata(&*program).map_err(|e| e.to_string())?;
    // `scanSpans`: each finding at a node also gets its node's span and text (`rr_scan_span`), for a scan's report.
    if options["scanSpans"] == true {
        let mut rules = String::from(".decl rr_report_location(n: node, l: number, c: number, el: number, ec: number)\n.input rr_report_location\n.decl rr_scan_span(p: symbol, n: node, l: number, c: number, el: number, ec: number, matched: symbol)\n.output rr_scan_span\n");
        let mut wanted = false;
        for rel in program.declared_names() {
            let Some(cols) = program.columns_of(&rel) else { continue };
            if rr::is_diagnostic(&rel) && cols.get(5).is_some_and(|col| col.1 == "node") {
                let tail = ", _".repeat(cols.len() - 6);
                rules.push_str(&format!("rr_scan_span(p, n, l, c, el, ec, matched) :- {rel}(_, _, _, p, _, n{tail}), rr_report_location(n, l, c, el, ec), text(n, matched).\n"));
                wanted = true;
            }
        }
        if wanted {
            // The spans' rules, read as a program of their own, added to the program read: an edit of its version
            // (decision 0276 stage 6: no tree), their declarations, inputs and outputs with them.
            let spans = rr::parse_version(&rules).map_err(|e| e.to_string())?;
            let mut b = crate::code_rule_dsl::logic::program_image::Builder::on(&program);
            for rel in spans.declared_names() {
                if !program.declares(&rel) {
                    let cols = spans.columns_of(&rel).unwrap_or_default();
                    b.fields().declare(&rel, &cols.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect::<Vec<_>>());
                }
            }
            for rel in spans.input_names().into_iter().filter(|r| !program.has_input(r)) {
                b.fields().add_input(&rel);
            }
            for rel in spans.output_names().into_iter().filter(|r| !program.has_output(r)) {
                b.fields().add_output(&rel);
            }
            for r in spans.rules() {
                b.copy(&spans, r);
            }
            let (edited, _) = b.finish()?;
            program = edited;
        }
    }
    // A language of several dialects (`.language typescript` over `.ts`, `.tsx` and `.js` files) is a run per dialect
    // with files in scope: the first into `out`, each other into `out/<dialect>`, their findings together. A language
    // of one dialect, or none with files, runs as written. Each run holds the program read, its language the
    // dialect's (the `dialect` option, `corpus::added`): never printed and read again (decision 0249).
    let dialects = corpus::run_dialects(root, &program, &with_reuse(options))?;
    // One dialect with files may be another than the one named: a `.js`-scoped TypeScript pack reads JavaScript.
    let runs: Vec<(Option<&str>, std::path::PathBuf)> = if dialects.is_empty() {
        vec![(None, out.to_path_buf())]
    } else {
        dialects.iter().enumerate().map(|(i, label)| (Some(*label), if i == 0 { out.to_path_buf() } else { out.join(label) })).collect()
    };
    let (mut rows, mut files_read, mut nodes, mut phases) = (Vec::new(), 0, 0, Vec::new());
    // The device as `logic --metal` places it: `RERAIL_DEVICE=metal` (by price) or `always` (wherever it can).
    let binding = if std::env::var("RERAIL_DEVICE").is_ok_and(|v| v == "metal" || v == "always") { Binding::Metal } else { Binding::Cpu };
    let handed = program;
    for (dialect, dir) in &runs {
        let mut options = with_reuse(options);
        if let Some(dialect) = dialect {
            options["dialect"] = serde_json::json!(dialect);
        }
        let report = if out.as_os_str().is_empty() {
            let (report, bytes) = corpus::run_policy_image(&handed, root, binding, &options)?;
            let outputs = super::logic::plan::output::Outputs::of(&bytes)?;
            rows.extend(diagnostic_rows(&outputs)?);
            report
        } else {
            let report = corpus::run_read(&handed, root, dir, "program", binding, 1 << 18, &Default::default(), &options)?;
            rows.extend(diagnostics(dir)?);
            report
        };
        files_read += report.files;
        nodes += report.nodes;
        phases.extend(report.phases);
    }
    if crate::git_workspace::snapshot::contains(root) {
        for row in &mut rows {
            if let Some(path) = row["path"].as_str() {
                row["pathIdentity"] = serde_json::json!({"encoding": "utf8", "value": path});
            }
        }
    }
    Ok(
        serde_json::json!({"diagnostics": rows, "files": files_read, "nodes": nodes,
                         "metadata": metadata,
                         "programInputs": policy_inputs(files),
                         "phases": phases.iter().map(|(phase,user,system,rss,statements)| serde_json::json!({
                             "phase": phase, "userCpuSeconds": user, "systemCpuSeconds": system,
                             "peakRssMB": rss, "submittedStatements": statements
                         })).collect::<Vec<_>>()}),
    )
}

/// Several `.rerail` policies over one repository (O105): each runs over the documents of the language it names
/// (`.language python`, `rust`, `rr`), into its own `out/<k>`, through the one content store, so a file's tape is
/// made once whichever policy reads it. The findings of all of them, in the order of `paths`, and each run's
/// report.
pub fn run_policies(
    paths: &[std::path::PathBuf],
    root: &std::path::Path,
    out: &std::path::Path,
    options: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let mut options = options.clone();
    let mut invocation = PolicyInvocation::from_options(root, &mut options)?;
    let options = &options;
    let (mut diagnostics, mut runs) = (Vec::new(), Vec::new());
    // One run for the file observer: each root asked of the journal once for the batch.
    let _observed = super::logic::plan::corpus::ObservedBatch::begin();
    invocation.evaluate(|_| { prepare_policy_graphs(paths); Ok(()) })?;
    // What the batch's runs share is kept while it runs (phase 1's runs, by their inputs), and nothing past it.
    let _batch = super::logic::plan::corpus::phases::Batch::enter();
    // The image store, opened once for the batch's runs.
    let _images = super::logic::rr::HoldImages::enter();
    // The policies of one language run as one pack (decision 0168): libraries read, lowered, hidden and evaluated once
    // (decisions 0051, 0089), each policy's findings split back by its id. The rest run alone.
    // `RERAIL_BATCH_PACK=off` runs each policy alone. With `strict`, a failed group returns its error.
    // Grouping by default grammar inventories the source before the rules are
    // available. Git policy runs determine their source demand individually.
    let packed = matches!(invocation.source, PolicySource::Directory(_)) && paths.len() > 1 && !crate::git_workspace::snapshot::contains(root)
        && std::env::var("RERAIL_BATCH_PACK").map_or(true, |v| v != "off");
    let (texts, groups) = if packed {
        invocation.evaluate(|root| {
            let paths: Vec<_> = paths.iter().map(|p| p.as_path()).collect();
            let texts = program_texts(&paths).unwrap_or_default();
            let groups = policy_groups(&texts, root, options);
            Ok((texts, groups))
        })?
    } else {
        (Vec::new(), Vec::new())
    };
    let mut done: Vec<Option<(Vec<serde_json::Value>, serde_json::Value)>> = paths.iter().map(|_| None).collect();
    // `RERAIL_BATCH_PACK=strict`: a failed pack's error is each member's, and none runs alone.
    let mut failed: Vec<Option<String>> = paths.iter().map(|_| None).collect();
    for group in groups.iter().filter(|g| g.members.len() > 1) {
        // A failed pack can still contain policies that succeed on their own.
        match invocation.evaluate(|root| run_pack(paths, &texts, group, root, out, options)) {
            Ok(Some(each)) => {
                for (&k, found) in group.members.iter().zip(each) {
                    done[k] = Some(found);
                }
            }
            Err(e) if std::env::var("RERAIL_BATCH_PACK").as_deref() == Ok("strict") => {
                for &k in &group.members {
                    failed[k] = Some(e.to_string());
                }
            }
            _ => {}
        }
    }
    let mut succeeded = true;
    for (k, path) in paths.iter().enumerate() {
        let result = match done[k].take() {
            Some(run) => Ok(run),
            None if failed[k].is_some() => Err(failed[k].take().unwrap_or_default()),
            None => {
                let dir = out.join(k.to_string());
                let evaluated = read_policy(path).and_then(|(program, files)| {
                    invocation.evaluate(|root| run_program_checked(program.clone(), &files, root, &dir, options))
                });
                evaluated.map(|report| single_policy_report(path, &report))
            }
        };
        match result {
            Ok((found, run)) => {
                diagnostics.extend(found);
                runs.push(run);
            }
            Err(error) => {
                succeeded = false;
                runs.push(serde_json::json!({"path": path.display().to_string(), "error": error}));
            }
        }
    }
    let mut report = serde_json::json!({"diagnostics": diagnostics, "runs": runs});
    invocation.report(&mut report, succeeded);
    Ok(report)
}

/// Resolve the batch's includes together. A graph that fails is reported when its policy is read.
fn prepare_policy_graphs(paths: &[std::path::PathBuf]) {
    use super::logic::plan::corpus::phase0;
    let seeds: Vec<_> = paths.iter().filter_map(|p| p.canonical().ok()).map(phase0::Seed::File).collect();
    if seeds.len() > 1 && !phase0::active() {
        let _ = phase0::graphs(&seeds);
    }
}

fn single_policy_report(path: &Path, report: &serde_json::Value) -> (Vec<serde_json::Value>, serde_json::Value) {
    let run = serde_json::json!({"path": path.display().to_string(), "files": report["files"], "metadata": report["metadata"], "together": 1});
    (report["diagnostics"].as_array().cloned().unwrap_or_default(), run)
}

/// Policies of a batch that run as one pack: one language, the same dialects with files in scope, the same guest
/// requests (`rr::GuestAsks`), each with its own policy id (its findings are told apart by it) and findings declared.
/// `members` index the batch's paths; `metadata` is each one's; `dialects` the runs a policy alone makes
/// (`corpus::run_dialects`: none for a language of one dialect).
struct PolicyGroup {
    members: Vec<usize>,
    ids: Vec<String>,
    metadata: Vec<serde_json::Value>,
    dialects: Vec<&'static str>,
    asks: super::logic::rr::GuestAsks,
}

/// The batch's policies (`texts`, as `program_texts` reads them) grouped by the grammar each run reads (decision
/// 0168, `corpus::run_language`: its `.language`'s, else the tree's), the dialects its language's runs read
/// (`corpus::run_dialects_in`, PR #508's 1999e204e) and the guests it asks for (`rr::guest_asks`, 370b90f9c), each read
/// for its header only (`rr::Text::header`: its directives and declarations, no clause) and its clauses' guest reads,
/// which is all a group needs before its run. A pack makes one set of guests for all its programs, and a rule that
/// reaches a guest reads all of them: programs that ask for different guests would each read the others'. A policy
/// whose own clauses ask for guests runs in a group of its own. The corpus is listed once for all of them.
/// Findings are told apart by their policy's `.policy` id, so a group holds each id once (one policy of none). A
/// policy whose header does not read, has an id another of its grammar has, or declares no findings runs as `run_rr`
/// runs it (and fails there with its own error).
fn policy_groups(texts: &[String], root: &std::path::Path, options: &serde_json::Value) -> Vec<PolicyGroup> {
    use super::logic::{plan::corpus, rr};
    let listing: std::cell::OnceCell<corpus::CorpusFiles> = std::cell::OnceCell::new();
    let files = || -> super::logic::plan::Result<corpus::CorpusFiles> {
        if let Some(f) = listing.get() {
            return Ok(f.clone());
        }
        let f = corpus::corpus_files(root)?;
        let _ = listing.set(f.clone());
        Ok(f)
    };
    let mut groups: Vec<(&'static str, PolicyGroup)> = Vec::new();
    for (k, text) in texts.iter().enumerate() {
        let own = rr::text_header(text).ok();
        let joined = own.and_then(|own| {
            let settings = ProgramFields::settings(&*own);
            let language = corpus::run_language(root, &*own, &with_reuse(options)).ok()?;
            // A language of several dialects (`.language typescript` over `.ts`, `.tsx` and `.js` files) is a run per
            // dialect with files in the policy's scope, as `run_program` makes them: policies of the same dialects
            // make the same runs, each one pack.
            let dialects = corpus::run_dialects_in(&files, &*own, &with_reuse(options)).ok()?;
            let id = settings.iter().find(|(k, _)| k == "policy").and_then(|(_, t)| t.split('"').nth(1)).unwrap_or_default().to_owned();
            let findings = own.output_names().iter().any(|o| rr::is_diagnostic(o));
            let metadata = rr::metadata(&*own).ok()?;
            let asks = rr::guest_asks(text).ok()?;
            findings.then_some((language, dialects, id, metadata, asks))
        });
        match joined {
            Some((language, dialects, id, metadata, asks)) if !groups.iter().any(|(_, g)| g.ids.contains(&id)) => {
                let at = match groups.iter().position(|(l, g)| *l == language && g.dialects == dialects && g.asks == asks && !asks.own) {
                    Some(at) => at,
                    None => {
                        groups.push((language, PolicyGroup { members: Vec::new(), ids: Vec::new(), metadata: Vec::new(), dialects, asks }));
                        groups.len() - 1
                    }
                };
                let group = &mut groups[at].1;
                group.members.push(k);
                group.ids.push(id);
                group.metadata.push(metadata);
            }
            _ => {}
        }
    }
    groups.into_iter().map(|(_, g)| g).collect()
}

/// A pack's member's run (`run_pack`): its findings, and its run's record.
type MemberRun = (Vec<serde_json::Value>, serde_json::Value);

/// One group's policies run as one pack into `out/pack-<first>` (a dialect after the first into its own directory
/// below it, as `run_program` places a policy's), and each one's findings written to its own `out/<k>` (its
/// `rr_diagnostic`, in `outputs.rro`): per member, its findings and its run's record. `None` when a pack's run fails
/// or a finding names no member's id: the group's policies then run alone, each with its own error (with
/// `RERAIL_BATCH_PACK=strict`, the pack's error instead).
fn run_pack(
    paths: &[std::path::PathBuf],
    texts: &[String],
    group: &PolicyGroup,
    root: &std::path::Path,
    out: &std::path::Path,
    options: &serde_json::Value,
) -> Result<Option<Vec<MemberRun>>, ProgramRunError> {
    use super::{logic::plan::corpus, typing::file_local::Binding};
    use crate::code_rule_dsl::logic::plan::output;
    let texts: Vec<&str> = group.members.iter().map(|&k| texts[k].as_str()).collect();
    let binding = if std::env::var("RERAIL_DEVICE").is_ok_and(|v| v == "metal" || v == "always") { Binding::Metal } else { Binding::Cpu };
    let _sources = super::logic::profile::on().then(|| super::logic::rr::profile_join_sources(
        &group.members.iter().map(|&k| paths[k].as_path()).collect::<Vec<_>>(),
    ));
    let dir = out.join(format!("pack-{}", group.members[0]));
    crate::code_rule_dsl::logic::profile::decision("a batch's policies as one pack (decision 0168)", || {
        format!("{} policies: {}", texts.len(), group.ids.join(", "))
    });
    // `RERAIL_BATCH_PACK=strict`: a failed pack returns its error, not `None`.
    let fallback = |reason: String| {
        if std::env::var("RERAIL_BATCH_PACK").as_deref() == Ok("strict") {
            Err(ProgramRunError::Evaluation(format!("combined run of {} policies ({}) failed: {reason}", texts.len(), group.ids.join(", "))))
        } else {
            Ok(None)
        }
    };
    // Each dialect's run and its directory, its language the dialect's (the `dialect` option, as `run_program` runs
    // a policy alone): one run as written where the language has one dialect.
    let runs: Vec<(Option<&str>, std::path::PathBuf)> = if group.dialects.is_empty() {
        vec![(None, dir.clone())]
    } else {
        group.dialects.iter().enumerate().map(|(i, label)| (Some(*label), if i == 0 { dir.clone() } else { dir.join(label) })).collect()
    };
    let mut files = 0;
    let mut outputs = Vec::with_capacity(runs.len());
    for (dialect, dir) in &runs {
        let mut options = with_reuse(options);
        if let Some(dialect) = dialect {
            options["dialect"] = serde_json::json!(dialect);
        }
        let report = match corpus::run_with_options(&texts, root, None, dir, "program", binding, 1 << 18, &Default::default(), &options) {
            Ok(report) => report,
            // A store that fails is the invocation's to recover from (`PolicyInvocation::evaluate`): it runs again in memory.
            Err(error) => match ProgramRunError::from(error) {
                ProgramRunError::Evaluation(reason) => return fallback(reason),
                cache => return Err(cache),
            },
        };
        files += report.files;
        outputs.push(output::Outputs::open(dir).map_err(|e| e.to_string())?);
    }
    let mut each: Vec<Vec<serde_json::Value>> = group.members.iter().map(|_| Vec::new()).collect();
    // Each member's rows of each finding relation, by the policy each row names (its first column), every dialect's.
    type Rows<'a> = Vec<output::Row<'a>>;
    let mut rows: Vec<std::collections::BTreeMap<String, Rows<'_>>> = group.members.iter().map(|_| std::collections::BTreeMap::new()).collect();
    for run in &outputs {
        for rel in run.names().into_iter().filter(|r| super::logic::rr::is_diagnostic(r)) {
            for row in run.rows(rel) {
                let Some(at) = row.get(0).text().and_then(|id| group.ids.iter().position(|x| x == id)) else {
                    return fallback(format!("{rel} produced a finding with an unknown policy id {:?}", row.get(0).text()));
                };
                each[at].push(diagnostic(rel, &row)?);
                rows[at].entry(rel.to_owned()).or_default().push(row);
            }
        }
    }
    let flags = outputs[0].images().last().map_or(0, output::Image::flags);
    let mut done = Vec::with_capacity(group.members.len());
    for (((&k, found), rows), metadata) in group.members.iter().zip(each).zip(rows).zip(&group.metadata) {
        let own = out.join(k.to_string());
        std::fs::create_dir_all(&own).map_err(|e| format!("{}: {e}", own.display()))?;
        // Its own outputs, a base of its findings alone (decision 0239).
        let mut image = output::Builder::new(flags, false);
        for (rel, rows) in &rows {
            let types = outputs.iter().find_map(|o| o.get(rel).map(|r| r.types().to_vec())).unwrap_or_default();
            image.rows_of(rel, &types, rows);
        }
        output::write(&own, &image.finish()).map_err(|e| format!("{}: {e}", own.display()))?;
        let run = serde_json::json!({"path": paths[k].display().to_string(), "files": files, "metadata": metadata, "together": group.members.len()});
        done.push((found, run));
    }
    Ok(Some(done))
}

/// `policies` ((name, source, format `json`/`yaml`)) lowered with no store and dumped as `rir`:
/// every clause, or the one named `clause`; `fused` adds packs and groups, `setplan` the set plan.
pub fn dump(
    policies: &[(String, String, String)],
    capabilities: &str,
    clause: Option<&str>,
    fused: bool,
    setplan: bool,
) -> Result<String, String> {
    let compiled = policies
        .iter()
        .map(|(name, source, format)| {
            crate::code_rule_dsl::rule_api::compile(source, format, capabilities)
                .map(|rule| (name.clone(), rule))
                .map_err(|error| format!("{name}: {}", error.reason))
        })
        .collect::<Result<Vec<_>, _>>()?;
    super::slice::dump(&compiled, clause, fused, setplan).map_err(|error| error.to_string())
}

/// Every `program` block of `text` assembled through the verifier and dumped again by its own
/// grammar, or the first refusal.
pub fn assemble(text: &str) -> Result<String, String> {
    let programs = rir::assemble_all(text)?;
    Ok(programs
        .iter()
        .map(|(program, grammar)| rir::disassemble(program, grammar.as_ref()))
        .collect())
}






/// The command line's runs keep their outputs for phase 0's cutoff (`corpus::reuse`), unless the caller said.
fn with_reuse(options: &serde_json::Value) -> serde_json::Value {
    // No options (`null`, as `run-policies` passes) are options too: the cutoff was off for every policy it ran.
    let mut options = if options.is_null() { serde_json::json!({}) } else { options.clone() };
    if let Some(map) = options.as_object_mut() {
        map.entry("reuse").or_insert(serde_json::Value::Bool(true));
    }
    options
}

/// [`program_text`] under the name the wasm build's callers use: a rule as written in the library, its includes
/// and `core.rerail` resolved, as one program's text.
/// The file's text is kept as written (`program_text`), so its tests and fixture sets (decision 0130) go with it.
pub fn flatten(path: &std::path::Path) -> Result<String, String> {
    program_text(path)
}

/// The program `run-rr` runs for a rule file: read, its includes resolved and lowered (`rr::read_file`), printed as
/// `.rerail`. Its size is the lowering's (rules, helper relations), before the run adds its libraries and plans it.
/// (02439e9145, which an earlier merge into the hub lost, restored at the merge of main 6ec384ad9.)
pub fn flatten_lowered(path: &std::path::Path) -> Result<String, String> {
    let (program, _) = super::logic::rr::read_file(path).map_err(|e| e.to_string())?;
    super::logic::rr::print_version(&program).map_err(|e| e.to_string())
}

/// A run `logic` makes (`dialect_runs`): its dialect (`None`: as written), and its programs.
type DialectRun = (Option<&'static str>, Vec<String>);

/// The runs `logic` makes of `programs`: those whose language has one dialect, or one with files in scope, run as
/// written (the first run); a program of a language of several dialects with files (`run_dialects`) runs in each,
/// its language set to the dialect by the run (the `dialect` option, `corpus::added`), its text as written. The
/// corpus is listed once for all of them.
fn dialect_runs(programs: &[&str], root: &std::path::Path, options: &serde_json::Value) -> Result<Vec<DialectRun>, ProgramRunError> {
    use super::language::CodeRuleLanguage;
    use super::logic::plan::corpus;
    let listing: std::cell::OnceCell<corpus::CorpusFiles> = std::cell::OnceCell::new();
    let files = || -> super::logic::plan::Result<corpus::CorpusFiles> {
        if let Some(f) = listing.get() {
            return Ok(f.clone());
        }
        let f = corpus::corpus_files(root)?;
        let _ = listing.set(f.clone());
        Ok(f)
    };
    let language_line = |text: &str| text.lines().position(|l| l.trim_start().starts_with(".language "));
    let mut base: Vec<String> = Vec::new();
    let mut by: Vec<(&'static str, Vec<String>)> = Vec::new();
    for text in programs {
        let line = language_line(text);
        let named = line.and_then(|i| text.lines().nth(i)).map(|l| l.trim_start()[".language ".len()..].trim().trim_matches('"').to_owned());
        let several = named.as_deref().is_some_and(|n| CodeRuleLanguage::ALL.into_iter().any(|l| l.name() == n && l.dialects().len() > 1));
        let labels = if several {
            let prog = super::logic::rr::parse_own_version(text).map_err(|e| e.to_string())?;
            if crate::git_workspace::snapshot::contains(root) {
                corpus::run_dialects(root, &prog, options)
            } else {
                corpus::run_dialects_in(&files, &*prog, options)
            }?
        } else {
            Vec::new()
        };
        if labels.is_empty() || labels == [named.as_deref().unwrap_or("")] {
            base.push((*text).to_owned());
            continue;
        }
        for label in labels {
            let each = (*text).to_owned();
            if Some(label) == named.as_deref() {
                base.push(each);
            } else if let Some((_, texts)) = by.iter_mut().find(|(l, _)| *l == label) {
                texts.push(each);
            } else {
                by.push((label, vec![each]));
            }
        }
    }
    let mut runs: Vec<(Option<&'static str>, Vec<String>)> = Vec::new();
    if !base.is_empty() {
        runs.push((None, base));
    }
    runs.extend(by.into_iter().map(|(l, t)| (Some(l), t)));
    if runs.is_empty() {
        runs.push((None, Vec::new()));
    }
    Ok(runs)
}

/// How `logic` runs its programs: the SQL executor's modes, on Metal or the CPU, the chunk's nodes, and the
/// `--env key=value` settings ([`settings`]).
#[derive(Clone, Copy)]
pub struct LogicRun<'a> {
    pub modes: &'a str,
    pub metal: bool,
    pub chunk_nodes: usize,
    pub settings: &'a [String],
}

/// `programs` (`.rerail` texts) over the corpus T's `t_facts` names under `root` (list step 23): the outputs as T's
/// `rdl.py sql` writes them, into `out`, and a report of each phase's CPU and the peak RSS. Several programs run as
/// one (decision 0051): each file is parsed and batched once for all of them, and `violation` holds every one's
/// findings. T's facts reproduce one program's run only.
/// `run`'s settings are `--env key=value`s ([`settings`]).
pub fn logic(programs: &[String], root: &std::path::Path, t_facts: Option<&std::path::Path>, out: &std::path::Path, run: &LogicRun<'_>) -> Result<String, String> {
    logic_checked(programs, root, t_facts, out, run).map_err(|error| error.to_string())
}

fn logic_checked(programs: &[String], root: &std::path::Path, t_facts: Option<&std::path::Path>, out: &std::path::Path, run: &LogicRun<'_>) -> Result<String, ProgramRunError> {
    let LogicRun { modes, metal, chunk_nodes, settings } = *run;
    use super::typing::file_local::Binding;
    let binding = if metal { Binding::Metal } else { Binding::Cpu };
    if programs.len() > 1 && t_facts.is_some() {
        return Err(ProgramRunError::Evaluation("T facts reproduce one program's run, not several".to_owned()));
    }
    let options = serde_json::json!({ "env": settings, "reuse": true });
    let programs: Vec<&str> = programs.iter().map(String::as_str).collect();
    // A program whose language has several dialects with files in its scope (`.language typescript` over `.ts`,
    // `.tsx` and `.js` files, which main's TypeScript reads) runs once per dialect, lowered for each, as
    // `run_program` runs one. The first run writes `out`; each other dialect's writes `out/<dialect>`, and its
    // findings are added to `out`'s.
    let dialects = crate::code_rule_dsl::logic::profile::span(|| "the dialects in the corpus".into());
    let runs = if t_facts.is_some() { vec![(None, programs.iter().map(|p| (*p).to_owned()).collect())] } else { dialect_runs(&programs, root, &options)? };
    drop(dialects);
    let mut report = None;
    for (i, (label, texts)) in runs.iter().enumerate() {
        let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
        let dir = if i == 0 { out.to_path_buf() } else { out.join(label.unwrap_or("other")) };
        let mut options = options.clone();
        if let Some(label) = label {
            options["dialect"] = serde_json::json!(label);
        }
        let each = super::logic::plan::corpus::run_with_options(&texts, root, t_facts, &dir, modes, binding, chunk_nodes, &super::logic::plan::corpus::Typing::default(), &options)?;
        match &mut report {
            None => report = Some(each),
            Some(first) => {
                let r: &mut super::logic::plan::corpus::Report = first;
                r.files += each.files;
                r.nodes += each.nodes;
                r.chunks += each.chunks;
                // Its findings joined to the first run's, over `out`'s outputs (decision 0239).
                let (theirs, ours) = (crate::code_rule_dsl::logic::plan::output::Outputs::open(&dir).map_err(|e| e.to_string())?, crate::code_rule_dsl::logic::plan::output::Outputs::open(out).map_err(|e| e.to_string())?);
                if let Some(found) = theirs.get("violation").filter(|v| !v.is_empty()) {
                    let mut image = crate::code_rule_dsl::logic::plan::output::Builder::new(ours.images().last().map_or(0, crate::code_rule_dsl::logic::plan::output::Image::flags), !ours.images().is_empty());
                    let mine = ours.get("violation");
                    let views: Vec<&crate::code_rule_dsl::logic::plan::output::View<'_>> = mine.iter().map(|r| r.view()).chain([found.view()]).collect();
                    image.union("violation", &views);
                    let bytes = image.finish();
                    drop((mine, found));
                    crate::code_rule_dsl::logic::plan::output::write(out, &bytes).map_err(|e| e.to_string())?;
                }
            }
        }
    }
    let report = report.ok_or_else(|| "no programs".to_owned())?;
    let execution = if modes.is_empty() { "sql" } else { modes };
    let mut text = format!(
        "{} programs; {} files, {} nodes in {} chunks, {} components native ({execution}, nearest on {binding:?})\n",
        programs.len(),
        report.files,
        report.nodes,
        report.chunks,
        report.native
    );
    if report.prepared != "prepared" {
        text += if report.prepared == "store" { "  program prepared: read from the store\n" } else { "  program prepared: kept in memory\n" };
    }
    if report.memo.0 > 0 {
        text += &format!("  {} of {} files from the per-file memo\n", report.memo.0, report.files);
    }
    for (phase, user, sys, rss, statements) in &report.phases {
        text += &format!(
            "  {phase:<6} {user:.2} s user + {sys:.2} s sys; peak RSS {rss:.0} MB; {statements} SQLite statements\n"
        );
    }
    // Decision 0288: the run's thread per phase, at the phase's bounds: what of its wall it ran, helped the pool or
    // spent off a core, and how parallel the process was meanwhile.
    for (phase, c) in &report.phase_clock {
        text += &format!(
            "  {phase:<6} {:.2} s wall: the run's thread {:.2} s own, {:.2} s helping, {:.2} s off a core; the other threads {:.2} s; parallel {:.2}; ready with no core {:.2} s; {} involuntary switches\n",
            c.wall, c.own, c.helped, c.off_core(), c.others(), c.process_cpu / c.wall.max(1e-9), c.no_core(), c.involuntary
        );
    }
    if !report.phase_clock.is_empty() {
        let run: f64 = report.phase_clock.iter().map(|(_, c)| c.wall).sum();
        let age = crate::code_rule_dsl::logic::profile::process_age();
        if age > 0.0 {
            text += &format!("  the phases above {run:.2} s of the process's {age:.2} s so far\n");
        }
    }
    // Retired instructions and cycles of the whole process: steadier than CPU time on a loaded machine, so a change
    // is compared by them (macOS; nothing elsewhere).
    timeline_mark("report");
    if let Some((instructions, cycles)) = process_counters() {
        text += &format!("  process {:.2} G instructions, {:.2} G cycles\n", instructions as f64 / 1e9, cycles as f64 / 1e9);
        for (phase, n) in &report.phase_instructions {
            text += &format!("    {phase:<6} {:.2} G instructions\n", *n as f64 / 1e9);
        }
    }
    for run in &report.phase_runs {
        text += &format!("  phase {} run first, for the exports it gives\n", run.name);
    }
    for (rel, rows) in &report.rows {
        text += &format!("  {rel}: {rows} rows\n");
    }
    if t_facts.is_some() {
        text += "  (planner statistics and node ids read from the T facts file)\n";
    }
    Ok(text)
}


// ---------------------------------------------------------------- the command line
//
//   rerail-engine dump --rule <file>... [--clause <id>] [--fused] [--setplan] [--capabilities <json>]
//   rerail-engine asm <file.rir>
//   rerail-engine logic <program.rerail | dir of .rerail> <corpus root> [<T facts.sqlite>] <out dir> [--modes <modes>] [--metal] [--chunk <nodes>] [--full]
//   rerail-engine gc <store.sqlite> [--history-mb <n>]
//   rerail-engine sql <store.sqlite> <request.json>
//   rerail-engine flatten <program.rerail>
//   rerail-engine run-rr <policy.rerail> <corpus root> <out dir> [<options.json>]
//   rerail-engine run-policies [--tool-calls <calls.json>] <corpus root> <out dir> <policy.rerail>...
//   rerail-engine check-rr <rule>... | convert-rr <rule>...
//   rerail-engine test-rr <rule or directory>... [--filter <text>]
use std::path::Path;

const USAGE: &str = "usage:\n  rerail-engine repeat <script>\n  rerail-engine dump --rule <file>... [--clause <id>] [--fused] [--setplan] [--capabilities <file>]\n  rerail-engine asm <file.rir>\n  rerail-engine logic <program.rerail | dir of .rerail> <corpus root> [<T facts.sqlite>] <out dir> [--modes <modes> | --sql] [--metal] [--chunk <nodes>] [--env key=value]... [--full] [--facts <dir>] [--require-facts] [--base <rev>] [--revision <commit-or-tree> --analysis-scope <name>]\n  rerail-engine facts-stamp <raw dir> --out <facts dir> --root <repository> <policy.rerail>...\n  rerail-engine images pack <exported dir> <bundle>\n  rerail-engine gc <store.sqlite> [--history-mb <n>]\n  rerail-engine delete-scope <store.sqlite> <name>\n  rerail-engine capture-tapes <root> <out file> [--every <k>] [--max <files>] [--all-tokens]\n  rerail-engine code-counts <root>... [--per-repo N] --out <counts.tsv>\n  rerail-engine upkeep [<store.sqlite>...] [--cpu <percent>] [--jobs <list>]\n  rerail-engine kernels [list | compile [--all] [--idle] | ship <dir> [--from <store dir>] [--air] | prune [--mb <n>]]\n  rerail-engine sql <store.sqlite> <request.json>\n  rerail-engine flatten [--lowered] <program.rerail>\n  rerail-engine csv <out dir> [<to dir>]\n  rerail-engine run-rr <policy.rerail> <corpus root> <out dir> [<options.json>] [--revision <commit-or-tree> --analysis-scope <name>]\n  rerail-engine run-policies [--tool-calls <calls.json>] <corpus root> <out dir> <policy.rerail>... [--revision <commit-or-tree> --analysis-scope <name>]\n  rerail-engine check-rr <rule>...\n  rerail-engine convert-rr [--cases <cases.json>] <rule>...\n  rerail-engine test-rr <rule or directory>... [--filter <text>] [--json]\n  rerail-engine translate-json [--display] <rule.json>... --out <dir>\nevery command: [--memory-mb <n>] (overall memory budget, default 1024) [--metal-mb <n>] (Metal budget on top of it, default half the overall)";

/// One `rerail-engine` command (`args` after the program's name): its output, or the error to report. The binary
/// runs one per process; a long-lived instance (the browser's engine: `reactor`) runs one per call, and with
/// `RERAIL_SESSION` (a directory) keeps a corpus session across them (decision 0063): `cli_in`, with the session
/// it keeps.
///
/// Returns the command's result, and where it cut a relation at the width cap (decision 0462, `RERAIL_WIDEST=cut`) the
/// message that its results are incomplete: the caller fails the process with it. So too where the engine read state
/// outside any run (decision 0442, `rerail_context::misses`): a refused read, which took a value of its own.
pub fn cli(args: &[String], machine: &std::sync::Arc<crate::threads::Machine>) -> (Result<String, String>, Option<String>) {
    // Decision 0272 (O167): the process ends with its command, before any idle time, so each run serves its waiting
    // join searches at its end, within a share of its CPU (decision 0243). A long-lived caller of `cli_in` leaves
    // them to the idle job.
    crate::code_rule_dsl::logic::plan::joins::drain_at_end(true);
    // Decision 0288: the command's clocks, for its time before its run and after it.
    crate::code_rule_dsl::logic::profile::command_begins();
    // Decision 0395: a run never waits on a Metal compile of a kernel it has another form of; the kernels it went
    // without are compiled by maintenance (below, upkeep, `kernels compile`).
    // The process's command, its own caller: what it shares by design is its own, on the machine its program made
    // (decision 0442).
    let shared = crate::code_rule_dsl::logic::profile::Shared::on(machine);
    #[cfg(target_os = "macos")]
    if args.first().is_none_or(|c| c != "kernels") {
        rerail_accel::gpu::kernels::set_mode(shared.machine(), rerail_accel::gpu::kernels::Mode::Defer);
    }
    let (out, incomplete) = cli_in(args, &mut Default::default(), &shared);
    // The kernels this run went without (its machine's), compiled after it by the engine in a process of its own, at
    // the background priority (decision 0395; `RERAIL_METAL_MAINTENANCE=off`: not).
    #[cfg(target_os = "macos")]
    if let Ok(engine) = std::env::current_exe() {
        let _on = crate::code_rule_dsl::logic::profile::within(std::sync::Arc::clone(&shared));
        rerail_accel::gpu::kernels::spawn_maintenance(&engine);
    }
    let (age, after) = crate::code_rule_dsl::logic::profile::command_ends();
    // On stderr: a command's output is its own (`run-rr`'s, `sql`'s: JSON), and a line after it made that unreadable.
    if let (Ok(_), Some(c)) = (&out, after) {
        eprint!(
            "  after the run {:.2} s wall: the command's thread {:.2} s own, {:.2} s off a core; the other threads {:.2} s; ready with no core {:.2} s\n  before the command: {age:.2} s since the process started\n",
            c.wall, c.own + c.helped, c.off_core(), c.others(), c.no_core()
        );
    }
    let misses = crate::code_rule_dsl::logic::profile::misses();
    let refused = (!misses.is_empty()).then(|| {
        format!("{} read(s) outside any run, refused (decision 0442; each with its backtrace above): {}", misses.len(), misses.join(", "))
    });
    let incomplete = match (incomplete, refused) {
        (Some(a), Some(b)) => Some(format!("{a}\n{b}")),
        (a, b) => a.or(b),
    };
    (out, incomplete)
}

/// `cli`, in the session `kept` keeps across its caller's commands (`RERAIL_SESSION`), sharing by design what
/// `shared` holds (decision 0442): the caller owns both.
///
/// The command is one run (decisions 0462, 0441): its limits are read as it starts, its runs share them and one
/// record, and nothing of them outlives it. Beside the result: the message that it cut a relation, if it did.
pub fn cli_in(args: &[String], kept: &mut crate::code_rule_dsl::logic::plan::corpus::session::Kept, shared: &std::sync::Arc<crate::code_rule_dsl::logic::run::Shared>) -> (Result<String, String>, Option<String>) {
    let own = crate::code_rule_dsl::logic::run::scope_in(Some(shared), crate::code_rule_dsl::logic::native::width::Settings::from_env);
    let out = command(args, kept);
    let incomplete = own.finish().and_then(|r| r.width.incomplete());
    (out, incomplete)
}

/// A command of `cli_in`, within its limits.
fn command(args: &[String], kept: &mut crate::code_rule_dsl::logic::plan::corpus::session::Kept) -> Result<String, String> {
    // The program is booted before its first command: as the library loads, or by the engine's `cli`/`cli_in`
    // (`code_rule_dsl/tools/napi.rs`) where nothing runs then (the browser's build, the CLI binary).
    let mut budgets = budget_options(args)?;
    for flag in ["--revision", "--analysis-scope"] {
        if budgets.iter().filter(|a| *a == flag).count() > 1 {
            return Err(format!("{flag} can be supplied only once"));
        }
    }
    let mut revision = None;
    if let Some(index) = budgets.iter().position(|a| a == "--revision").filter(|_| budgets.first().is_none_or(|a| a != "logic")) {
        let root_index = match budgets.first().map(String::as_str) {
            Some("run-rr") => 2,
            Some("run-policies") => 1,
            _ => return Err("--revision is supported by logic, run-rr and run-policies".into()),
        };
        if index <= root_index { return Err("place --revision after the corpus root".into()); }
        revision = Some(budgets.get(index + 1).filter(|s| !s.starts_with("--")).ok_or("--revision requires a commit or tree")?.clone());
        budgets.drain(index..=index + 1);
    }
    let mut analysis_scope = None;
    if let Some(index) = budgets.iter().position(|a| a == "--analysis-scope").filter(|_| budgets.first().is_none_or(|a| a != "logic")) {
        if !matches!(budgets.first().map(String::as_str), Some("run-rr" | "run-policies")) {
            return Err("--analysis-scope is supported by logic, run-rr and run-policies".into());
        }
        analysis_scope = Some(budgets.get(index + 1).filter(|s| !s.starts_with("--")).ok_or("--analysis-scope requires a name")?.clone());
        budgets.drain(index..=index + 1);
    }
    let with_revision = |mut options: serde_json::Value| {
        if revision.is_some() || analysis_scope.is_some() {
            if !options.is_object() {
                options = serde_json::json!({});
            }
            if let Some(revision) = &revision {
                options["revision"] = serde_json::json!(revision);
            }
            if let Some(scope) = &analysis_scope {
                options["analysisScope"] = serde_json::json!(scope);
            }
        }
        options
    };
    let args = budgets.as_slice();
    // However the command ends, the cost samples it made are written (decision 0100 §14).
    struct Flush;
    impl Drop for Flush {
        fn drop(&mut self) {
            crate::code_rule_dsl::logic::costs::flush();
            #[cfg(feature = "tape-reads")]
            crate::code_rule_dsl::tape::reads::report();
        }
    }
    let _flush = Flush;
    let _session = kept.bind_from_env()?;
    match args.first().map(String::as_str) {
        #[cfg(feature = "dump-cli")]
        Some("dump") => dump_command(&args[1..]),
        Some("asm") => asm_command(&args[1..]),
        Some("sql") => sql_command(&args[1..]),
        Some("logic") => logic_command(&args[1..]),
        Some("repeat") => repeat_script_command(&args[1..]),
        Some("facts-stamp") => facts_stamp_command(&args[1..]),
        Some("capture-tapes") => capture_tapes_command(&args[1..]),
        Some("code-counts") => code_counts_command(&args[1..]),
        Some("kind-names") => kind_names_command(&args[1..]),
        Some("run-rr") => run_rr_command(&args[1..], &with_revision),
        // Several policies over one repository, each over its own language's files (O105), and the tool calls it is
        // given.
        Some("run-policies") => run_policies_command(&args[1..], &with_revision),
        Some("check-rr") => check_rr_command(&args[1..]),
        // Main's JSON rules as the engine reads them: each `<id>.json` to `<out>/<id>.rerail` (decision 0175).
        Some("translate-json") => translate_json_command(&args[1..]),
        // Each test the files carry (decision 0130).
        Some("test-rr") => crate::code_rule_dsl::logic::rr::testing::command(&args[1..]),
        Some("convert-rr") => convert_rr_command(&args[1..]),
        Some("flatten") => flatten_command(&args[1..]),
        Some("images") => images_command(&args[1..]),
        Some("csv") => csv_command(&args[1..]),
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        Some("kernels") => kernels_command(&args[1..]),
        #[cfg(feature = "tape-cache")]
        Some("gc") => gc_command(&args[1..]),
        Some("delete-scope") => match &args[1..] {
            [store, name] => {
                let scope = crate::content_store::analysis_scope::named(name).ok_or("analysis scope name must not be empty")?;
                let path = Path::new(store);
                if path.exists() {
                    crate::content_store::ContentStore::open(path).and_then(|store| store.delete_analysis_scope(&scope)).map_err(|err| err.to_string())?;
                }
                Ok(String::new())
            }
            _ => Err("usage: rerail-engine delete-scope <store.sqlite> <name>".into()),
        },
        #[cfg(feature = "tape-cache")]
        Some("upkeep") => upkeep_command(&args[1..]),
        #[cfg(not(all(feature = "dump-cli", feature = "tape-cache")))]
        Some(command @ ("dump" | "gc" | "upkeep")) => Err(format!("{command} is not in this build (its feature, `dump-cli` or `tape-cache`, is off)")),
        _ => Err(USAGE.into()),
    }
}

/// `asm <file.rir>`.
fn asm_command(args: &[String]) -> Result<String, String> {
    match args {
        [file] => crate::tools::assemble(&read(file)?),
        _ => Err(USAGE.into()),
    }
}

/// `sql <store.sqlite> <request.json>`.
fn sql_command(args: &[String]) -> Result<String, String> {
    match args {
        [store, request] => crate::tools::sql(Path::new(store), &read(request)?),
        _ => Err("usage: rerail-engine sql <store.sqlite> <request.json>".into()),
    }
}

/// `repeat <script>` (`repeat_command`).
fn repeat_script_command(args: &[String]) -> Result<String, String> {
    match args {
        [script] => repeat_command(&read(script)?),
        _ => Err("usage: rerail-engine repeat <script>".into()),
    }
}

/// `run-rr <policy.rerail> <corpus root> <out dir> [<options.json>]`.
/// `revision` adds the command's `--revision` and `--analysis-scope` to the options.
fn run_rr_command(args: &[String], revision: &dyn Fn(serde_json::Value) -> serde_json::Value) -> Result<String, String> {
    let options = match args {
        [_, _, _] => serde_json::Value::Null,
        [_, _, _, file] => serde_json::from_str(&read(file)?).map_err(|e| e.to_string())?,
        _ => return Err(USAGE.into()),
    };
    policy_command_report(crate::tools::run_rr(Path::new(&args[0]), Path::new(&args[1]), Path::new(&args[2]), &revision(options))?)
}

/// `run-policies [--tool-calls <calls.json>] <corpus root> <out dir> <policy.rerail>...`: several policies over one
/// repository, each over its own language's files (O105). `--tool-calls`: the tool calls the run is given (`[{"id",
/// "tool", "input"}]`, the `toolCalls` option: `corpus::added`), which `.scope tool_calls` policies read.
/// `revision` adds the command's `--revision` and `--analysis-scope` to the options.
fn run_policies_command(args: &[String], revision: &dyn Fn(serde_json::Value) -> serde_json::Value) -> Result<String, String> {
    let usage = "usage: rerail-engine run-policies [--tool-calls <calls.json>] <corpus root> <out dir> <policy.rerail>...";
    let (mut positional, mut options) = (Vec::new(), serde_json::Value::Null);
    let mut rest = args.iter();
    while let Some(a) = rest.next() {
        match a.as_str() {
            "--tool-calls" => options = tool_calls_option(rest.next().ok_or(usage)?)?,
            _ => positional.push(a),
        }
    }
    let [root, out, first, more @ ..] = positional.as_slice() else { return Err(usage.into()) };
    let paths: Vec<std::path::PathBuf> = std::iter::once(first).chain(more).map(|p| std::path::PathBuf::from(p.as_str())).collect();
    policy_command_report(crate::tools::run_policies(&paths, Path::new(root.as_str()), Path::new(out.as_str()), &revision(options))?)
}

/// The run's options of the tool calls in `file`, once they are of their shape (`added::tool_call_rows`).
fn tool_calls_option(file: &str) -> Result<serde_json::Value, String> {
    let calls: serde_json::Value = serde_json::from_str(&read(file)?).map_err(|e| format!("{file}: {e}"))?;
    crate::code_rule_dsl::logic::plan::corpus::added::tool_call_rows(&calls).map_err(|e| format!("{file}: {e}"))?;
    Ok(serde_json::json!({ "toolCalls": calls }))
}

/// `check-rr <rule>...`: the rules as the `.rerail` reader reads them; an error when any fails.
fn check_rr_command(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Err(USAGE.into());
    }
    let report = crate::tools::check_rr(&args.iter().map(std::path::PathBuf::from).collect::<Vec<_>>());
    let failures = report.as_array().expect("rule reports").iter().filter(|row| row["status"] == "reader-failed").count();
    let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    if failures > 0 {
        println!("{text}");
        return Err(format!("{failures} rules failed the .rerail reader"));
    }
    Ok(text)
}

/// `convert-rr [--cases <cases.json>] <rule>...`: `--cases <file>` gives the next rule its cases (decision 0130).
fn convert_rr_command(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Err(USAGE.into());
    }
    let (mut rules, mut cases) = (Vec::new(), None);
    let mut rest = args.iter();
    while let Some(a) = rest.next() {
        match a.as_str() {
            "--cases" => cases = Some(std::path::PathBuf::from(rest.next().ok_or("--cases <cases.json> <rule>")?)),
            _ => rules.push((std::path::PathBuf::from(a), cases.take())),
        }
    }
    serde_json::to_string_pretty(&crate::tools::convert_rr_with(&rules)).map_err(|e| e.to_string())
}

/// `flatten [--lowered] <program.rerail>`: the program `logic` runs for a file, `core.rerail` and its includes
/// resolved, as `.rerail` text; with `--lowered`, the program `run-rr` runs, lowered and printed as `.rerail`.
fn flatten_command(args: &[String]) -> Result<String, String> {
    match args {
        [file] => crate::tools::flatten(Path::new(file.as_str())),
        [flag, file] if flag == "--lowered" => crate::tools::flatten_lowered(Path::new(file.as_str())),
        _ => Err(USAGE.into()),
    }
}

/// `images pack <exported dir> <bundle>`: the rule images shipped with the engine (decision 0264), packed from those
/// a run exported (`RERAIL_EXPORT_IMAGES=<dir>`), into the bundle placed beside the engine (`rerail-library.images`).
fn images_command(args: &[String]) -> Result<String, String> {
    match args {
        [pack, dir, out] if pack == "pack" => crate::code_rule_dsl::logic::rr::images::pack(Path::new(dir.as_str()), Path::new(out.as_str()))
            .map(|n| format!("{n} images packed into {out}"))
            .map_err(|e| e.to_string()),
        _ => Err("usage: rerail-engine images pack <exported dir> <bundle>".into()),
    }
}

/// `csv <out dir> [<to dir>]`: a run's outputs (`out/outputs.rro`, decision 0239) as `.csv` files, one per relation,
/// in `to` (`out` itself when not given), for scripts that read them.
fn csv_command(args: &[String]) -> Result<String, String> {
    match args {
        [out] | [out, _] => {
            let to = Path::new(args.get(1).unwrap_or(out).as_str());
            let n = crate::code_rule_dsl::logic::plan::output::to_csv(Path::new(out.as_str()), to).map_err(|e| e.to_string())?;
            Ok(format!("{n} relations written to {}\n", to.display()))
        }
        _ => Err(USAGE.into()),
    }
}

/// A partial report is still the command's JSON output. The executable maps the error to exit 1.
fn policy_command_report(report: serde_json::Value) -> Result<String, String> {
    let text = serde_json::to_string(&report).map_err(|error| error.to_string())?;
    if report["status"] == "failed" {
        println!("{text}");
        Err("analysis failed; see the policy and source errors in the report".into())
    } else {
        Ok(text)
    }
}

/// `repeat`: several commands in one process, so a session (decision 0063, `RERAIL_SESSION`) can be measured from the
/// command line (t31 step 0.2). One command per line of `script`:
///
/// ```text
/// env RERAIL_SESSION=/tmp/s        // set (or, with no `=`, remove) a variable before the next command
/// logic rules/ corpus/ out/cold    // any engine command, as its arguments; a session starts at the first one
/// sh echo "x = 1" >> corpus/a.py   // an edit between runs, through /bin/sh
/// # a comment, and blank lines, are skipped
/// ```
///
/// Each engine command prints one line to stderr: its CPU (user + sys) and the process's footprint, now and at most.
#[cfg(not(target_family = "wasm"))]
fn repeat_command(script: &str) -> Result<String, String> {
    use crate::code_rule_dsl::logic::plan::corpus::usage;
    let mut out = String::new();
    // The session the script's commands keep (`RERAIL_SESSION`), from its first command to its last.
    let mut kept = crate::code_rule_dsl::logic::plan::corpus::session::Kept::default();
    for (n, line) in script.lines().enumerate().map(|(n, l)| (n + 1, l.trim())) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
        match word {
            "env" => match rest.split_once('=') {
                Some((k, v)) => {
                    unsafe { std::env::set_var(k.trim(), v.trim()) };
                    // The profiler reads its directory once: each run's report goes where the script says.
                    if k.trim() == "RERAIL_PROFILE" {
                        crate::code_rule_dsl::logic::profile::set(true);
                        crate::code_rule_dsl::logic::profile::set_directory(Path::new(v.trim()));
                    }
                }
                None => unsafe { std::env::remove_var(rest.trim()) },
            },
            "sh" => {
                let status = std::process::Command::new("/bin/sh").arg("-c").arg(rest).status().map_err(|e| format!("line {n}: {e}"))?;
                if !status.success() {
                    return Err(format!("line {n}: `{rest}` exited with {status}"));
                }
            }
            _ => {
                let args: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
                let (u0, s0, _) = usage();
                let result = command(&args, &mut kept);
                let (u1, s1, _) = usage();
                eprintln!(
                    "repeat line {n}: {:.2} s CPU ({:.2} user + {:.2} sys); footprint {} MB, at most {} MB; {}",
                    (u1 - u0) + (s1 - s0),
                    u1 - u0,
                    s1 - s0,
                    crate::code_rule_dsl::prof::footprint_mb(),
                    crate::code_rule_dsl::prof::max_footprint_mb(),
                    if result.is_ok() { "ok" } else { "failed" }
                );
                out.push_str(&result.map_err(|e| format!("line {n}: {e}"))?);
            }
        }
    }
    Ok(out)
}

#[cfg(target_family = "wasm")]
fn repeat_command(_: &str) -> Result<String, String> {
    Err("repeat is not in the browser build".into())
}

/// The memory budgets, set by any command's `--memory-mb <n>` (the overall budget, 1 GB by default) and
/// `--metal-mb <n>` (the Metal budget, on top of it: the overall budget by default) (decisions 0126, 0549): the
/// arguments without them.
fn budget_options(args: &[String]) -> Result<Vec<String>, String> {
    let mut rest = Vec::with_capacity(args.len());
    let mut args = args.iter();
    while let Some(argument) = args.next() {
        let set: fn(usize) -> Result<(), String> = match argument.as_str() {
            "--memory-mb" => rerail_accel::budget::set_memory_mb,
            "--metal-mb" => rerail_accel::budget::set_metal_mb,
            _ => {
                rest.push(argument.clone());
                continue;
            }
        };
        let mb = args.next().and_then(|v| v.parse::<usize>().ok()).ok_or_else(|| format!("{argument} takes a size in MB"))?;
        set(mb)?;
    }
    Ok(rest)
}

/// `logic`: a program over a corpus, its outputs as T's (list step 23).
/// `translate-json [--display] <rule.json>... --out <dir>`: each JSON policy translated as the reader translates it,
/// written to `<dir>/<stem>.rerail`. With `--display`, the display translation instead (the rules docs' generated view,
/// before the engine's dialect), and `<dir>/counts.json`: each stem's JSON terms left as `ast(..)` atoms
/// (`untranslated`) and rules no positive atom grounds (`ungrounded`). A rule the translator cannot take is reported
/// and the rest go on.
fn translate_json_command(args: &[String]) -> Result<String, String> {
    use crate::code_rule_dsl::logic::json_rules;
    let usage = "usage: rerail-engine translate-json [--display] <rule.json>... --out <dir>";
    let (mut files, mut out, mut display) = (Vec::new(), None, false);
    let mut rest = args.iter();
    while let Some(a) = rest.next() {
        if a == "--display" {
            display = true;
        } else if a == "--out" {
            out = Some(std::path::PathBuf::from(rest.next().ok_or(usage)?));
        } else {
            files.push(std::path::PathBuf::from(a));
        }
    }
    let out = out.ok_or(usage)?;
    if files.is_empty() {
        return Err(usage.into());
    }
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let translator = json_rules::Translator::new();
    let mut failed = 0;
    let mut counts = serde_json::Map::new();
    for f in &files {
        let stem = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let made = if display {
            std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display())).and_then(|json| translator.display_text(&json, &stem)).map(|d| {
                counts.insert(stem.clone(), serde_json::json!({"untranslated": d.untranslated, "ungrounded": d.ungrounded}));
                d.text
            })
        } else {
            translator.translate_file(f)
        };
        match made {
            Ok(text) => {
                let dest = out.join(format!("{stem}.rerail"));
                std::fs::write(&dest, text).map_err(|e| format!("{}: {e}", dest.display()))?;
            }
            Err(e) => {
                failed += 1;
                eprintln!("FAIL {e}");
            }
        }
    }
    if display {
        let dest = out.join("counts.json");
        std::fs::write(&dest, serde_json::Value::Object(counts).to_string()).map_err(|e| format!("{}: {e}", dest.display()))?;
    }
    Ok(format!("{} translated, {failed} failed", files.len() - failed))
}

use crate::code_rule_dsl::logic::profile::timeline_mark;

fn logic_command(args: &[String]) -> Result<String, String> {
    timeline_mark("cli start");
    let (mut positional, mut modes, mut metal, mut chunk) = (Vec::new(), None, false, 1_usize << 18);
    let (mut sql, mut env, mut full) = (false, Vec::new(), false);
    let mut revision = None;
    let mut analysis_scope = None;
    let mut args = args.iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--revision" => {
                if revision.is_some() { return Err("--revision can be supplied only once".into()); }
                revision = Some(args.next().filter(|s| !s.starts_with("--")).ok_or("--revision requires a commit or tree")?.clone());
            }
            "--analysis-scope" => {
                if analysis_scope.is_some() { return Err("--analysis-scope can be supplied only once".into()); }
                analysis_scope = Some(args.next().filter(|s| !s.starts_with("--")).ok_or("--analysis-scope requires a name")?.clone());
            }
            "--sql" => sql = true,
            "--metal" => metal = true,
            // A full run in a session: compiled and evaluated afresh, as without one (decision 0063).
            "--full" => full = true,
            "--modes" => modes = Some(args.next().ok_or("--modes requires a value")?.clone()),
            "--env" => env.push(args.next().ok_or("--env requires key=value")?.clone()),
            "--chunk" => chunk = args.next().ok_or("--chunk requires a value")?.parse().map_err(|_| "--chunk takes a node count")?,
            // Where `artifact=` inputs are read (data-layer/RUN-FACTS.md section 2); `<root>/.rerail/facts` without it.
            "--facts" => crate::code_rule_dsl::logic::plan::corpus::inputs::set_facts_dir(args.next().ok_or("--facts requires a directory")?.into())?,
            // In CI: a missing, stale or malformed artifact fails the run (RUN-FACTS.md section 5).
            "--require-facts" => crate::code_rule_dsl::logic::plan::corpus::inputs::require_facts(),
            // The revision `at="base"` inputs are read at: a commit id or a reference name.
            "--base" => crate::code_rule_dsl::logic::plan::corpus::inputs::set_base(args.next().ok_or("--base requires a revision")?.clone())?,
            other => positional.push(other.to_owned()),
        }
    }
    let modes = match (sql, modes) {
        (true, Some(_)) => return Err("--sql cannot be combined with --modes".into()),
        (true, None) => String::new(),
        (false, modes) => modes.unwrap_or_else(|| "program".into()),
    };
    // `<program> <corpus> <out>` on the typing programs' own ingestion, or `<program> <corpus> <T facts> <out>` to
    // reproduce T (its statistics and node ids).
    let (program, root, facts, out) = match positional.as_slice() {
        [program, root, out] => (program, root, None, out),
        [program, root, facts, out] => (program, root, Some(Path::new(facts.as_str())), out),
        _ => return Err(USAGE.into()),
    };
    // A directory: every `.rerail` in it, run as one program (each file parsed once for all of them); each file's
    // `#include`s are read in.
    let reading = crate::code_rule_dsl::logic::profile::span(|| "read the rule files".into());
    let _sources;
    let programs = if Path::new(program.as_str()).is_dir() {
        let mut files: Vec<_> = std::fs::read_dir(program)
            .map_err(|e| format!("{program}: {e}"))?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            // and every JSON policy (`rerail.policy/v1`), translated as it is read (decision 0175)
            .filter(|path| path.extension().is_some_and(|x| x == "rerail") || crate::code_rule_dsl::logic::json_rules::is_policy_path(path))
            .collect();
        files.sort();
        // A file another one `#include`s is a library of it (`…confusables.rerail`), not a program of its own: alone it
        // has neither the declarations it reads nor the rules that fill its inputs.
        let included: std::collections::BTreeSet<std::ffi::OsString> = files
            .iter()
            .filter_map(|path| std::fs::read_to_string(path).ok())
            .flat_map(|text| {
                text.lines()
                    .filter_map(|l| l.trim().strip_prefix("#include"))
                    .filter_map(|rest| rest.trim().strip_prefix('"')?.strip_suffix('"').map(|p| Path::new(p).file_name().map(ToOwned::to_owned)))
                    .flatten()
                    .collect::<Vec<_>>()
            })
            .collect();
        files.retain(|path| !path.file_name().is_some_and(|n| included.contains(n)));
        timeline_mark("the program files listed");
        let texts = crate::tools::program_texts(&files.iter().map(|p| p.as_ref()).collect::<Vec<&std::path::Path>>())?;
        _sources = super::logic::rr::profile_join_sources(&files.iter().map(|p| p.as_path()).collect::<Vec<_>>());
        timeline_mark("the program texts read");
        texts
    } else {
        let path = Path::new(program.as_str());
        let text = crate::tools::program_text(path)?;
        _sources = super::logic::rr::profile_join_sources(&[path]);
        vec![text]
    };
    drop(reading);
    if programs.is_empty() {
        return Err(format!("{program} has no .rerail programs"));
    }
    if revision.is_none() && analysis_scope.is_some() {
        return Err("--analysis-scope requires --revision".into());
    }
    let eval = |root: &Path| {
        let run = || logic_checked(&programs, root, facts, Path::new(out), &crate::tools::LogicRun { modes: &modes, metal, chunk_nodes: chunk, settings: &env });
        if full { crate::code_rule_dsl::logic::plan::corpus::session::full(run) } else { run() }
    };
    let mut invocation = PolicyInvocation::begin(Path::new(root), revision.as_deref(), analysis_scope.as_deref())?;
    let report = invocation.evaluate(eval)?;
    let failures = invocation.source_failures();
    for failure in &failures {
        eprintln!("rerail: {}: {}", failure.path, failure.message);
    }
    if let Some(warning) = invocation.finish(failures.is_empty()) {
        eprintln!("rerail: cache was not retained. {warning}");
    }
    if !failures.is_empty() {
        print!("{report}");
        return Err(format!("incomplete Git source coverage ({} source failures)", failures.len()));
    }
    Ok(report)
}

/// `facts-stamp <raw> --out <dir> --root <repository> <policy.rerail>…` (data-layer/RUN-FACTS.md section 1, `rerail facts
/// stamp`): wraps each producer's output with the digest of the inputs the policies declare for it.
fn facts_stamp_command(args: &[String]) -> Result<String, String> {
    use crate::code_rule_dsl::logic::plan::corpus::inputs;
    let (mut raw, mut out, mut root, mut policies) = (None, None, None, Vec::new());
    let mut args = args.iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--out" => out = Some(args.next().ok_or("--out requires a directory")?.clone()),
            "--root" => root = Some(args.next().ok_or("--root requires a directory")?.clone()),
            other if raw.is_none() => raw = Some(other.to_owned()),
            other => policies.push(other.to_owned()),
        }
    }
    let (Some(raw), Some(out)) = (raw, out) else { return Err(USAGE.into()) };
    let root = root.unwrap_or_else(|| ".".into());
    // The declarations, from each policy with its `#include`s read in: `.artifact` lines only.
    let mut settings = Vec::new();
    for policy in &policies {
        let text = crate::tools::program_text(Path::new(policy))?;
        settings.extend(text.lines().filter_map(|l| l.trim().strip_prefix(".artifact ")).map(|rest| ("artifact".to_owned(), rest.to_owned())));
    }
    let declared = inputs::artifacts(&settings).map_err(|e| e.to_string())?;
    inputs::stamp(Path::new(&raw), Path::new(&out), Path::new(&root), &declared)
}

/// `gc`: collects a store until nothing is left to collect (decision 0053): converts a store made before 0053 to
/// incremental vacuum, decides which corpus runs to keep (`--history-mb`: the high-water mark, default 512; the low
/// water is three quarters of it), and runs passes until two in a row find nothing.
#[cfg(feature = "tape-cache")]
fn gc_command(args: &[String]) -> Result<String, String> {
    use crate::content_store::{ContentStore, CorpusPolicy};
    let (mut path, mut policy) = (None, CorpusPolicy::default());
    let mut args = args.iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--history-mb" => {
                let mb: u64 = args.next().ok_or("--history-mb requires a value")?.parse().map_err(|_| "--history-mb takes a number")?;
                (policy.high_water, policy.low_water) = (mb << 20, (mb << 20) / 4 * 3);
            }
            other if path.is_none() => path = Some(other.to_owned()),
            _ => return Err(USAGE.into()),
        }
    }
    let path = path.ok_or(USAGE)?;
    if !Path::new(&path).exists() {
        return Err(format!("{path}: no such store"));
    }
    let e = |e: crate::content_store::StoreError| e.to_string();
    let store = ContentStore::open(Path::new(&path)).map_err(e)?;
    let before = store.capacity().map_err(e)?.bytes;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let mut out = String::new();
    if store.convert_to_incremental_vacuum().map_err(e)? {
        out += "converted to incremental vacuum (a store made before decision 0053)\n";
    }
    let kept = store.keep_corpus_runs(&policy, now).map_err(e)?;
    out += &format!(
        "corpus runs: {} manifests kept, {} evicted; history {:.1} MB (high water {} MB), protected {:.1} MB\n",
        kept.manifests,
        kept.evicted,
        kept.history_bytes as f64 / 1048576.0,
        policy.high_water >> 20,
        kept.protected_bytes as f64 / 1048576.0
    );
    let (mut idle, mut passes, mut tapes, mut sources, mut plan_rows) = (0, 0, 0, 0, 0);
    while idle < 2 {
        let pass = store.collect_corpus_pass(&policy, now).map_err(e)?;
        passes += 1;
        idle = if pass.did_work() { 0 } else { idle + 1 };
        (tapes, sources, plan_rows) = (tapes + pass.collection.tapes, sources + pass.collection.sources, plan_rows + pass.plan_rows);
    }
    let after = store.capacity().map_err(e)?.bytes;
    out += &format!(
        "{passes} passes: {tapes} tapes, {sources} sources and {plan_rows} memo rows of dead plans deleted; {:.1} MB -> {:.1} MB\n",
        before as f64 / 1048576.0,
        after as f64 / 1048576.0
    );
    Ok(out)
}

/// Every fixed kernel of the engine (a kernel whose source is the same for every rule set), to compile ahead of its
/// first use (decision 0395's tiers, on every backend): the GPU lane's (`rerail_accel::gpu::kernels_ahead`), the GPU
/// runtime's (`rerail_metal::kernels_ahead`), the device's (`MatcherDevice::kernels_ahead`) and, in a build with the
/// tests' hooks, the tape format's tests' (`rerail_tape_format::f16::kernels`). Each entry asks for its kernel
/// as its use does, so its use finds it: in this process, in the store, or shipped.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
pub fn kernels_ahead() -> Vec<rerail_gpu::kernels::Ahead> {
    let mut v = rerail_accel::gpu::kernels_ahead();
    v.extend(rerail_metal::kernels_ahead());
    if let Some(device) = crate::code_rule_dsl::transducer::packed::device::matcher() {
        v.extend(device.kernels_ahead().into_iter().map(|(name, make)| rerail_accel::gpu::ahead(name, make)));
    }
    #[cfg(feature = "test-hooks")]
    v.extend(rerail_tape_format::f16::kernels().into_iter().map(|(name, text)| {
        rerail_gpu::kernels::Ahead::new(name.clone(), move || rerail_gpu::kernels::program(&name, &text).map(drop))
    }));
    v
}

/// `rerail-engine kernels` (decision 0395): the compiled kernels, on every backend. The device layer's store
/// (`rerail_device::store`: CUDA's cubins, OpenCL's binaries, the CPU's shared objects) and, on macOS, Metal's
/// archive store (`rerail_accel::gpu::kernels`).
///
/// ```text
/// kernels [list]                   the stores, the shipped kernels, the device, and (Metal) the kernels runs went
///                                  without
/// kernels compile [--all] [--idle] [--threads <n>] [--only <text>] [--within <s>]
///                                  compiles every fixed kernel now, here (`kernels_ahead`; --only: those whose names
///                                  hold <text>), <n> at a time (default half the cores), none starting past <s>
///                                  seconds and none waited for past <s> + 60 (the next goes on from the store), then
///                                  (Metal) the last run's (--all: every run's) wanted kernels
///                                  (--idle: at the background priority, as after a run)
/// kernels ship <dir> [--from <s>] [--air]
///                                  the fixed kernels as the engine's bundle: this device's forms under
///                                  <dir>/<backend>/ by the store's keys (compiled first where the store lacks them);
///                                  Metal: the archives a run compiled (store <s>, else this machine's); --air: AIR
///                                  libraries too (Xcode's Metal Toolchain)
/// kernels prune [--mb <n>]         each store within n MB (default RERAIL_GPU_KERNELS_MB and RERAIL_METAL_KERNELS_MB,
///                                  2048)
/// ```
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
fn kernels_command(args: &[String]) -> Result<String, String> {
    use rerail_gpu::store;
    let mut out = String::new();
    let threads = match args.iter().position(|a| a == "--threads") {
        Some(i) => args.get(i + 1).and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 0).ok_or("--threads takes a number")?,
        None => rerail_gpu::kernels::ahead_threads(),
    };
    // `--within <s>`: no compile starts after `s` seconds (the next `kernels compile` goes on from the store).
    let within = match args.iter().position(|a| a == "--within") {
        Some(i) => Some(std::time::Duration::from_secs_f64(args.get(i + 1).and_then(|v| v.parse::<f64>().ok()).ok_or("--within takes seconds")?)),
        None => None,
    };
    let until = within.map(|s| std::time::Instant::now() + s);
    // `--only <text>`: the fixed kernels whose names hold it.
    let only = match args.iter().position(|a| a == "--only") {
        Some(i) => Some(args.get(i + 1).ok_or("--only takes a name")?.as_str()),
        None => None,
    };
    // The process's device (its forms are kept under its backend's directory, in the store and in a bundle).
    let device = || rerail_gpu::shared::get().map(|s| s.device.name());
    // Compiles the fixed kernels, reporting each: what it loaded (shipped or from the store), compiled, and took.
    let compile = |out: &mut String| -> Vec<rerail_gpu::kernels::Made> {
        let started = std::time::Instant::now();
        let list = kernels_ahead().into_iter().filter(|a| only.is_none_or(|o| a.name.contains(o))).collect();
        // The compiles still running a minute past `--within` are left (the process ends without them).
        let bound = rerail_gpu::kernels::Bound { start_by: until, end_by: until.map(|u| u + std::time::Duration::from_secs(60)) };
        let made = rerail_gpu::kernels::compile_ahead(list, threads, bound);
        for m in &made {
            let how = match &m.result {
                _ if m.skipped => "  not started (--within)".to_owned(),
                _ if m.unfinished => "  still compiling, left (--within)".to_owned(),
                Err(e) => format!("  failed: {}", e.lines().next().unwrap_or("")),
                Ok(()) if m.compiled > 0 => format!("  compiled {}", m.compiled),
                Ok(()) if m.loaded > 0 => format!("  loaded {}", m.loaded),
                Ok(()) => String::new(),
            };
            *out += &format!("{:<40.40} {:>8.2} s{how}\n", m.name, m.seconds);
        }
        let (loaded, compiled): (u64, u64) = made.iter().fold((0, 0), |(l, c), m| (l + m.loaded, c + m.compiled));
        *out += &format!(
            "{} fixed kernels in {:.2} s ({threads} at a time): {compiled} compiled, {loaded} loaded, {} failed, {} not started, {} left compiling\n",
            made.len(),
            started.elapsed().as_secs_f64(),
            made.iter().filter(|m| m.result.is_err()).count(),
            made.iter().filter(|m| m.skipped).count(),
            made.iter().filter(|m| m.unfinished).count()
        );
        made
    };
    match args.first().map(String::as_str) {
        None | Some("list") => {
            out += &format!("store    {}\n", store::dir().map_or("off".to_owned(), |d| d.display().to_string()));
            out += &format!("shipped  {}\n", store::shipped_dir().map_or("none".to_owned(), |d| d.display().to_string()));
            out += &format!("device   {} ({})\n", device().unwrap_or_else(|| "none".to_owned()), rerail_gpu::device::backend());
            out += &format!("fixed    {} kernels\n", kernels_ahead().len());
            if let Some(s) = rerail_gpu::shared::get() {
                out += &format!("addresses {}\n", s.device.allocations_have_addresses());
                if let rerail_gpu::device::Device::OpenCl(d) = &s.device {
                    let (ext, synced) = d.describe();
                    out += &format!("opencl   synced {synced}; extensions:\n");
                    for x in ext.split_ascii_whitespace() {
                        out += &format!("  {x}\n");
                    }
                }
            }
            #[cfg(target_os = "macos")]
            {
                use rerail_accel::gpu::kernels as k;
                let s = k::size();
                out += &format!("metal store    {}\n", k::store_dir().map_or("off".to_owned(), |d| d.display().to_string()));
                out += &format!("metal shipped  {}\n", k::shipped_dir().map_or("none".to_owned(), |d| d.display().to_string()));
                if let Some(device) = rerail_accel::gpu::shared::get() {
                    out += &format!("metal device   {}\n", k::device_tag(&device.device));
                }
                out += &format!("archives {} ({:.1} MB), sources {} ({:.1} MB), wanted lists {}\n", s.archives, s.archive_bytes as f64 / 1e6, s.sources, s.source_bytes as f64 / 1e6, s.wanted_lists);
                for (which, label) in [(k::Which::Last, "the last run"), (k::Which::All, "every run")] {
                    let pending = k::pending(which);
                    out += &format!("not compiled, wanted by {label}: {}\n", pending.len());
                    if which == k::Which::All {
                        for w in pending.iter().take(40) {
                            out += &format!("  {} {}\n", &w.pipeline[..16], w.name);
                        }
                    }
                }
            }
            Ok(out)
        }
        Some("compile") => {
            let rest = &args[1..];
            // A machine with no device (a Linux host without a GPU) has no form of a fixed kernel to make: there is
            // nothing to compile, and nextest's setup script (`kernels compile`) goes on to the tests.
            #[cfg(target_os = "macos")]
            let metal = rerail_accel::gpu::shared::get().is_some();
            #[cfg(not(target_os = "macos"))]
            let metal = false;
            if device().is_none() && !metal {
                return Ok(format!("no device ({}): no kernels to compile\n", rerail_gpu::device::backend()));
            }
            if rest.iter().any(|a| a == "--idle") {
                rerail_accel::upkeep::lower_this_thread();
            }
            let made = compile(&mut out);
            #[cfg(target_os = "macos")]
            {
                use rerail_accel::gpu::kernels as k;
                let which = if rest.iter().any(|a| a == "--all") { k::Which::All } else { k::Which::Last };
                let started = std::time::Instant::now();
                // Past `--within`, none (the next run's step compiles them).
                let done = if until.is_some_and(|u| started >= u) { Vec::new() } else { k::compile_wanted(which, usize::MAX) };
                for c in &done {
                    out += &format!("{:<40.40} {} {:>8.2} s{}\n", c.name, &c.pipeline[..16], c.seconds, c.skipped.as_ref().map_or(String::new(), |s| format!("  skipped: {s}")));
                }
                let n = k::counts();
                out += &format!(
                    "{} wanted kernels in {:.2} s: front end {:.2} s, back end {:.2} s\n",
                    done.iter().filter(|c| c.skipped.is_none()).count(),
                    started.elapsed().as_secs_f64(),
                    n.front_end_s,
                    n.back_end_s
                );
                k::prune(k::max_bytes());
            }
            store::prune(store::max_bytes());
            match made.iter().find(|m| m.result.is_err()) {
                Some(m) => Err(format!("{out}{}: {}", m.name, m.result.as_ref().err().map_or("", String::as_str))),
                None => Ok(out),
            }
        }
        Some("ship") => {
            let dir = Path::new(args.get(1).ok_or("kernels ship <dir> [--from <store dir>] [--air]")?);
            let started = std::time::Instant::now();
            // The device layer's forms: each fixed kernel's entries as its compile looked them up (in the store,
            // or shipped already), by their own names under the backend's directory. Metal's device layer keeps
            // none (its archives are below).
            let made = compile(&mut out);
            if let Some(m) = made.iter().find(|m| m.result.is_err()) {
                return Err(format!("{out}{}: {}", m.name, m.result.as_ref().err().map_or("", String::as_str)));
            }
            let mut shipped = 0;
            for entry in made.iter().flat_map(|m| &m.entries) {
                let (Some(name), Some(backend)) = (entry.file_name(), entry.parent().and_then(Path::file_name)) else { continue };
                if !entry.is_file() {
                    continue;
                }
                let to = dir.join(backend).join(name);
                if to == *entry {
                    continue;
                }
                std::fs::create_dir_all(dir.join(backend)).map_err(|e| format!("{}: {e}", dir.display()))?;
                std::fs::copy(entry, &to).map_err(|e| format!("{} -> {}: {e}", entry.display(), to.display()))?;
                shipped += 1;
            }
            out += &format!("{shipped} device-layer kernels shipped into {}\n", dir.display());
            #[cfg(target_os = "macos")]
            {
                use rerail_accel::gpu::kernels as k;
                let from = match args.iter().position(|a| a == "--from") {
                    Some(i) => std::path::PathBuf::from(args.get(i + 1).ok_or("--from requires a directory")?),
                    None => k::store_dir().ok_or("no store (RERAIL_METAL_KERNELS=off): name one with --from")?,
                };
                for line in k::ship(dir, &from, args.iter().any(|a| a == "--air"))? {
                    out += &line;
                    out += "\n";
                }
            }
            out += &format!("shipped into {} in {:.2} s\n", dir.display(), started.elapsed().as_secs_f64());
            Ok(out)
        }
        Some("prune") => {
            let mb = match args.iter().position(|a| a == "--mb") {
                Some(i) => Some(args.get(i + 1).and_then(|v| v.parse::<u64>().ok()).ok_or("--mb takes a number")? << 20),
                None => None,
            };
            out += &format!("{} device-layer kernels removed\n", store::prune(mb.unwrap_or_else(store::max_bytes)));
            #[cfg(target_os = "macos")]
            {
                use rerail_accel::gpu::kernels as k;
                out += &format!("{} archives removed\n", k::prune(mb.unwrap_or_else(k::max_bytes)));
            }
            Ok(out)
        }
        Some(other) => Err(format!("kernels {other}: not a kernels command (list, compile, ship, prune)")),
    }
}

/// `upkeep [STORE...] [--cpu PCT] [--jobs LIST]`: runs every upkeep job that has work to its end now, idle machine
/// or not (decision 0190): each store's collection, search-index backlog and SQLite file, and the cost
/// statistics' store and model. `--cpu` is the share of one core the jobs average (the setting's default, 25);
/// `--jobs` is `RERAIL_UPKEEP_JOBS`'s list. Prints each job's steps, wall and CPU.
#[cfg(feature = "tape-cache")]
fn upkeep_command(args: &[String]) -> Result<String, String> {
    use crate::content_store::ContentStore;
    use crate::content_store::upkeep::{Collect, SqliteFile, Tables};
    use rerail_accel::upkeep::{configure, register, run_now, settings};
    let mut s = settings();
    let mut stores = Vec::new();
    let mut args = args.iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--cpu" => {
                let p: f64 = args.next().ok_or("--cpu requires a value")?.parse().map_err(|_| "--cpu takes a percentage")?;
                s.cpu = (p / 100.0).max(0.01);
            }
            "--jobs" => {
                let list = args.next().ok_or("--jobs requires a value")?;
                let (out, only): (Vec<&str>, Vec<&str>) = list.split(',').map(str::trim).filter(|j| !j.is_empty()).partition(|j| j.starts_with('-'));
                s.without = out.iter().map(|j| j[1..].to_owned()).collect();
                s.only = (!only.is_empty()).then(|| only.iter().map(|j| j.to_string()).collect());
            }
            other => stores.push(other.to_owned()),
        }
    }
    configure(s);
    let e = |e: crate::content_store::StoreError| e.to_string();
    let mut held = Vec::new();
    let mut jobs = Vec::new();
    for path in &stores {
        if !Path::new(path).exists() {
            return Err(format!("{path}: no such store"));
        }
        let store = std::sync::Arc::new(ContentStore::open(Path::new(path)).map_err(e)?);
        jobs.push(register(Box::new(Collect::new(&store))));
        jobs.push(register(Box::new(crate::content_store::upkeep::MemoSweep::new(Path::new(path)))));
        jobs.push(register(Box::new(crate::code_rule_dsl::workspace::storage::SearchIndex::new(&store))));
        jobs.push(register(Box::new(SqliteFile::new(Path::new(path), Tables::All))));
        held.push(store);
    }
    jobs.extend(crate::code_rule_dsl::logic::costs::upkeep_jobs());
    let started = std::time::Instant::now();
    let report = run_now();
    let mut out = String::new();
    for r in &report {
        out += &format!(
            "{:<48} {:>5} steps {:>9.1} ms wall {:>9.1} ms CPU{}\n",
            r.name,
            r.steps,
            r.wall * 1e3,
            r.cpu * 1e3,
            r.last_error.as_ref().map_or(String::new(), |e| format!("  error: {e}"))
        );
    }
    out += &format!("{:.2} s\n", started.elapsed().as_secs_f64());
    let forms = crate::code_rule_dsl::logic::costs::kernel_forms(20);
    if !forms.is_empty() {
        out += "\nforms this machine's runs went without (decision 0202), most batches first:\n";
        for (kind, name, batches, compiled, seconds, failed, missing) in forms {
            out += &format!(
                "{kind:<16} {name:<40.40} {batches:>9} batches  {compiled:>5} compiles {seconds:>8.2} s  {failed} failed{}\n",
                missing.map_or(String::new(), |m| format!("  missing: {m}"))
            );
        }
    }
    drop(jobs);
    Ok(out)
}

#[cfg(feature = "dump-cli")]
fn dump_command(args: &[String]) -> Result<String, String> {
    let (mut rules, mut clause, mut capabilities) = (Vec::new(), None, None);
    let (mut fused, mut setplan) = (false, false);
    let mut args = args.iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--fused" => fused = true,
            "--setplan" => setplan = true,
            "--rule" | "--clause" | "--capabilities" => {
                let value = args.next().ok_or_else(|| format!("{argument} requires a value"))?;
                match argument.as_str() {
                    "--rule" => rules.push(value.clone()),
                    "--clause" => clause = Some(value.clone()),
                    _ => capabilities = Some(read(value)?),
                }
            }
            other => return Err(format!("unknown dump option `{other}`\n{USAGE}")),
        }
    }
    if rules.is_empty() {
        return Err(format!("dump needs at least one --rule <file>\n{USAGE}"));
    }
    let policies = rules.iter().map(|file| policy(file)).collect::<Result<Vec<_>, _>>()?;
    let capabilities = capabilities.unwrap_or_else(|| crate::tools::CAPABILITIES.to_string());
    crate::tools::dump(&policies, &capabilities, clause.as_deref(), fused, setplan)
}

/// A rule file as (name, source, format): named by its stem, its format from the extension.
#[cfg(feature = "dump-cli")]
fn policy(file: &str) -> Result<(String, String, String), String> {
    let path = Path::new(file);
    let format = match path.extension().and_then(|extension| extension.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("json") => "json",
        Some("yaml" | "yml") => "yaml",
        _ => return Err(format!("{file}: a rule file is .json, .yaml or .yml")),
    };
    let name = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or(file).to_string();
    Ok((name, read(file)?, format.to_string()))
}

fn read(file: &str) -> Result<String, String> {
    std::fs::read_to_string(file).map_err(|error| format!("{file}: {error}"))
}

/// `capture-tapes <root> <out> [--every <k>] [--max <n>]`: every k-th file under `root` with a tape (sorted
/// paths; `.git`, `node_modules`, `.venv`, `target`, `dist` left out) parsed as the engine parses it, and its
/// source and primary tape image written to `out`, for the tape-format lab (`native/lab/tape16`, decision 0156).
///
/// The file: `T16CAP01`, then per file: path (u16 length, bytes), tape tag (u8), source (u32 length, bytes),
/// image (u32 length, bytes), all little-endian. A file the parser refuses or stops on is left out.
/// `kind-names <out>`: every tape language's kinds as `tag<TAB>id<TAB>name<TAB>named`, then its fields as
/// `tag<TAB>field id<TAB>name`, for the tape lab
/// (`native/lab/tape16`), whose captures carry kind ids only.
fn kind_names_command(args: &[String]) -> Result<String, String> {
    use crate::code_rule_dsl::language::{from_tape_tag, tape_tag};
    use crate::code_rule_dsl::language::GrammarDialect;
    let [out] = args else { return Err("usage: rerail-engine kind-names <out file>".to_owned()) };
    // A name escaped, so each kind and field is one line of four cells: C's and Go's tokens hold a newline and a NUL.
    let escaped = |name: &str| {
        let mut s = String::with_capacity(name.len());
        for c in name.chars() {
            match c {
                '\\' => s.push_str("\\\\"),
                '\n' => s.push_str("\\n"),
                '\r' => s.push_str("\\r"),
                '\t' => s.push_str("\\t"),
                '\0' => s.push_str("\\0"),
                c => s.push(c),
            }
        }
        s
    };
    let mut text = String::new();
    let mut tags = 0;
    for &lang in GrammarDialect::all() {
        let Some(tag) = tape_tag(lang) else { continue };
        if from_tape_tag(tag) != Some(lang) {
            continue;
        }
        tags += 1;
        // Each kind's public symbol: the id a node of its name and kind has (`ts_node_symbol`). A grammar's symbols of
        // one name and kind share one; an own parser's kinds are their own.
        let grammar = lang.kinds().grammar();
        for id in 0..lang.kind_count() {
            let id = id as u16;
            if let Some(name) = lang.kind_name(id) {
                let named = lang.kinds().kind_is_named(id);
                let public = grammar.map_or(id, |g| g.id_for_node_kind(name, named));
                text.push_str(&format!("{tag}\t{id}\t{}\t{}\t{public}\n", escaped(name), u8::from(named)));
            }
        }
        for id in 1..=lang.kinds().field_count() {
            if let Some(name) = lang.kinds().field_name(id as u16) {
                text.push_str(&format!("{tag}\tfield {id}\t{}\t\n", escaped(name)));
            }
        }
    }
    std::fs::write(out, text).map_err(|e| e.to_string())?;
    Ok(format!("{tags} languages to {out}"))
}

/// Every file under `root` a tape language reads, in path order (`.git`, `node_modules`, `.venv`, `target`, `dist`
/// and the like left out): `capture-tapes`' and `code-counts`' files.
/// The directories a walk of sources skips: dependencies, builds, caches.
const SKIPPED_DIRS: [&str; 10] = [".git", "node_modules", ".venv", "venv", "target", "dist", "build", "__pycache__", "tmp", "working-data"];

/// A bare repository's (a mirror's) HEAD tree: each blob the engine parses, as `(blob, dialect)`, at most `per` of
/// each dialect, spread evenly over the tree's order. Read from the object store through gix: nothing is checked
/// out (decision 0497).
fn bare_files(repo: &gix::Repository, per: usize) -> Vec<(gix::ObjectId, crate::code_rule_dsl::language::GrammarDialect)> {
    use crate::code_rule_dsl::language::{dialect_by_label, label_of_path, tape_tag};
    let Ok(tree) = repo.head_commit().map_err(|e| e.to_string()).and_then(|c| c.tree().map_err(|e| e.to_string())) else { return Vec::new() };
    let mut rec = gix::traverse::tree::Recorder::default();
    if tree.traverse().breadthfirst(&mut rec).is_err() {
        return Vec::new();
    }
    let mut by: std::collections::BTreeMap<String, Vec<(gix::ObjectId, crate::code_rule_dsl::language::GrammarDialect)>> = Default::default();
    for e in rec.records.into_iter().filter(|e| e.mode.is_blob()) {
        let path = e.filepath.to_string();
        if path.split('/').any(|part| SKIPPED_DIRS.contains(&part)) {
            continue;
        }
        let Some(d) = label_of_path(&path).and_then(dialect_by_label).filter(|d| tape_tag(*d).is_some()) else { continue };
        by.entry(format!("{d:?}")).or_default().push((e.oid, d));
    }
    by.into_values()
        .flat_map(|all| {
            let step = all.len().div_ceil(per.max(1)).max(1);
            all.into_iter().step_by(step)
        })
        .collect()
}

fn tape_files(root: &Path) -> Vec<(std::path::PathBuf, crate::code_rule_dsl::language::GrammarDialect)> {
    use crate::code_rule_dsl::language::{dialect_by_label, label_of_path, tape_tag};
    fn walk(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                if !SKIPPED_DIRS.contains(&name.as_ref()) {
                    walk(&entry.path(), files);
                }
            } else if kind.is_file() {
                files.push(entry.path());
            }
        }
    }
    let mut all = Vec::new();
    walk(root, &mut all);
    all.into_iter()
        .filter_map(|p| {
            let d = label_of_path(&p.to_string_lossy()).and_then(dialect_by_label).filter(|d| tape_tag(*d).is_some())?;
            Some((p, d))
        })
        .collect()
}

/// `code-counts <root>... --out <counts.tsv>`: how often each code of each language's format 16 table is a node's,
/// over every file under the roots the engine parses (as `capture-tapes` walks them): `tag<TAB>number<TAB>count`, the
/// number the shipped table's, and a `nodes` line per language. The lab's `tape16 table --counts` numbers a
/// language's codes by it (most frequent first); what a table holds is the grammar's alone (decision 0626).
fn code_counts_command(args: &[String]) -> Result<String, String> {
    use crate::code_rule_dsl::language::tape_tag;
    use crate::code_rule_dsl::tape::{Grammar, TapeImage};
    use rerail_tape_format::tape::Tape;
    use std::collections::HashMap;
    let usage = || "usage: rerail-engine code-counts <root>... [--per-repo N] --out <counts.tsv>".to_owned();
    let at = args.iter().position(|a| a == "--out").ok_or_else(usage)?;
    let out = args.get(at + 1).ok_or_else(usage)?;
    let per = match args.iter().position(|a| a == "--per-repo") {
        Some(i) => args.get(i + 1).and_then(|n| n.parse().ok()).ok_or_else(usage)?,
        None => usize::MAX,
    };
    let roots: Vec<&String> = args.iter().take_while(|a| !a.starts_with("--")).collect();
    if roots.is_empty() {
        return Err(usage());
    }
    // A root is a directory of sources, a bare repository (a mirror: `HEAD` and `objects`, no work tree), or a
    // directory of bare repositories. A repository's files are its HEAD tree's blobs, read from its object store.
    enum Src {
        File(std::path::PathBuf),
        Blob(gix::ObjectId),
    }
    // What is counted, one after another: a directory's files, or a repository. A repository is open only while its
    // files are counted, so its packs' maps and caches go with it.
    enum Unit {
        Dir(std::path::PathBuf),
        Repo(std::path::PathBuf),
    }
    let bare = |p: &Path| p.join("HEAD").is_file() && p.join("objects").is_dir();
    let mut units = Vec::new();
    for root in &roots {
        let root = Path::new(root.as_str());
        let mut gits: Vec<std::path::PathBuf> = if bare(root) {
            vec![root.to_path_buf()]
        } else {
            std::fs::read_dir(root).map_or(Vec::new(), |d| d.filter_map(Result::ok).map(|e| e.path()).filter(|p| bare(p)).collect())
        };
        gits.sort();
        if gits.is_empty() {
            units.push(Unit::Dir(root.to_path_buf()));
        }
        units.extend(gits.into_iter().map(Unit::Repo));
    }
    let counts: std::sync::Mutex<HashMap<(u8, u32), u64>> = std::sync::Mutex::new(HashMap::new());
    let nodes: std::sync::Mutex<HashMap<u8, (u64, u64)>> = std::sync::Mutex::new(HashMap::new());
    let refused = std::sync::atomic::AtomicUsize::new(0);
    let mut listed = 0;
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
    for unit in units {
        let (repo, files): (Option<gix::ThreadSafeRepository>, Vec<(Src, crate::code_rule_dsl::language::GrammarDialect)>) = match unit {
            Unit::Dir(d) => (None, tape_files(&d).into_iter().map(|(p, d)| (Src::File(p), d)).collect()),
            Unit::Repo(g) => {
                let Ok(repo) = gix::open_opts(&g, gix::open::Options::isolated()) else { continue };
                let files = bare_files(&repo, per).into_iter().map(|(oid, d)| (Src::Blob(oid), d)).collect();
                (Some(repo.into_sync()), files)
            }
        };
        listed += files.len();
        let next = std::sync::atomic::AtomicUsize::new(0);
        rerail_accel::exec::scoped(|scope| {
            for _ in 0..threads {
                scope.spawn(|| {
                    let (mut mine, mut seen): (HashMap<(u8, u32), u64>, HashMap<u8, (u64, u64)>) = (HashMap::new(), HashMap::new());
                    let local = repo.as_ref().map(gix::ThreadSafeRepository::to_thread_local);
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some((src, lang)) = files.get(i) else { break };
                        let bytes = match src {
                            Src::File(path) => std::fs::read(path).ok(),
                            Src::Blob(oid) => local.as_ref().and_then(|repo| repo.find_object(*oid).ok().map(|o| o.detach().data)),
                        };
                        let source = bytes.and_then(|s| String::from_utf8(s).ok()).filter(|s| s.len() <= 16 << 20);
                        let image = source.as_ref().and_then(|source| Grammar::new(*lang).ok().and_then(|g| TapeImage::parse(&g, source, &|| false).and_then(Result::ok)));
                        let Some(file) = image else {
                            refused.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            continue;
                        };
                        let tag = tape_tag(*lang).expect("filtered");
                        let n = rerail_tape_format::with_tape!(file.primary.tape(), |t| {
                            for k in 0..t.len() {
                                *mine.entry((tag, t.code(k))).or_default() += 1;
                            }
                            t.len() as u64
                        });
                        let e = seen.entry(tag).or_default();
                        (e.0, e.1) = (e.0 + 1, e.1 + n);
                    }
                    let mut c = counts.lock().unwrap();
                    for (k, v) in mine {
                        *c.entry(k).or_default() += v;
                    }
                    let mut s = nodes.lock().unwrap();
                    for (k, (f, n)) in seen {
                        let e = s.entry(k).or_default();
                        (e.0, e.1) = (e.0 + f, e.1 + n);
                    }
                });
            }
        });
    }
    let mut rows: Vec<_> = counts.into_inner().unwrap().into_iter().collect();
    rows.sort_unstable();
    let mut text = String::from("# code-counts: tag, the shipped table's number, nodes of that code; then per tag its files and nodes, and the table's revision\n");
    for ((tag, number), n) in rows {
        text.push_str(&format!("{tag}\t{number}\t{n}\n"));
    }
    let mut langs: Vec<_> = nodes.into_inner().unwrap().into_iter().collect();
    langs.sort_unstable();
    for (tag, (f, n)) in &langs {
        text.push_str(&format!("nodes\t{tag}\t{f}\t{n}\n"));
        // The table revision the numbers are of: a lab numbering by them refuses another's.
        let revision = rerail_tape_format::f16::lang(*tag).map_or(0, |l| l.revision());
        text.push_str(&format!("revision\t{tag}\t{revision}\n"));
    }
    std::fs::write(out, text).map_err(|e| e.to_string())?;
    Ok(format!("{} files counted ({} refused) in {} languages\n", listed - refused.load(std::sync::atomic::Ordering::Relaxed), refused.into_inner(), langs.len()))
}

fn capture_tapes_command(args: &[String]) -> Result<String, String> {
    use crate::code_rule_dsl::language::{dialect_by_label, label_of_path, tape_tag};
    use crate::code_rule_dsl::tape::{Grammar, TapeImage};
    use std::io::Write;
    let usage = || "usage: rerail-engine capture-tapes <root> <out file> [--every <k>] [--max <files>] [--all-tokens]".to_owned();
    let (root, out) = match args {
        [root, out, ..] => (Path::new(root), Path::new(out)),
        _ => return Err(usage()),
    };
    let mut every = 1usize;
    let mut max = usize::MAX;
    let all_tokens = args[2..].iter().any(|a| a == "--all-tokens");
    let mut rest = args[2..].iter().filter(|a| *a != "--all-tokens");
    while let Some(flag) = rest.next() {
        let value = rest.next().and_then(|v| v.parse::<usize>().ok()).ok_or_else(usage)?;
        match flag.as_str() {
            "--every" => every = value.max(1),
            "--max" => max = value,
            _ => return Err(usage()),
        }
    }
    fn walk(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                if !SKIPPED_DIRS.contains(&name.as_ref()) {
                    walk(&entry.path(), files);
                }
            } else if kind.is_file() {
                files.push(entry.path());
            }
        }
    }
    let mut all = Vec::new();
    walk(root, &mut all);
    let dialect = |path: &Path| {
        label_of_path(&path.to_string_lossy()).and_then(dialect_by_label).filter(|d| tape_tag(*d).is_some())
    };
    let files: Vec<_> = all.into_iter().filter(|p| dialect(p).is_some()).step_by(every).take(max).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let writer = std::sync::Mutex::new(std::io::BufWriter::new(std::fs::File::create(out).map_err(|e| e.to_string())?));
    writer.lock().unwrap().write_all(b"T16CAP01").map_err(|e| e.to_string())?;
    let (written, refused) = (std::sync::atomic::AtomicUsize::new(0), std::sync::atomic::AtomicUsize::new(0));
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
    rerail_accel::exec::scoped(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(path) = files.get(i) else { break };
                    let lang = dialect(path).expect("filtered");
                    let Ok(source) = std::fs::read(path) else { continue };
                    let Ok(source) = String::from_utf8(source) else {
                        refused.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        continue;
                    };
                    if source.len() > 16 << 20 {
                        refused.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        continue;
                    }
                    let parse = |grammar: std::sync::Arc<Grammar>| TapeImage::parse(&grammar, &source, &|| false);
                    let image = Grammar::new(lang)
                        .ok()
                        .and_then(|grammar| if all_tokens { crate::code_rule_dsl::tape::with_all_tokens(|| parse(grammar)) } else { parse(grammar) })
                        .and_then(Result::ok)
                        .and_then(|file| file.primary.to_bytes().ok());
                    let Some(image) = image else {
                        refused.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        continue;
                    };
                    // A document's path, with `/` on every OS.
                    let rel = path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
                    let mut record = Vec::with_capacity(rel.len() + source.len() + image.len() + 16);
                    record.extend_from_slice(&(rel.len() as u16).to_le_bytes());
                    record.extend_from_slice(rel.as_bytes());
                    record.push(tape_tag(lang).expect("filtered"));
                    record.extend_from_slice(&(source.len() as u32).to_le_bytes());
                    record.extend_from_slice(source.as_bytes());
                    record.extend_from_slice(&(image.len() as u32).to_le_bytes());
                    record.extend_from_slice(&image);
                    if writer.lock().unwrap().write_all(&record).is_ok() {
                        written.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            });
        }
    });
    writer.into_inner().unwrap().flush().map_err(|e| e.to_string())?;
    Ok(format!(
        "{} files captured, {} refused, of {} with a tape\n",
        written.into_inner(),
        refused.into_inner(),
        files.len()
    ))
}


#[cfg(test)]
mod settings_tests {
    #[test]
    fn settings_become_env_rows_for_a_program_that_declares_env() {
        let root = std::env::temp_dir().join(format!("rerail-settings-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("rerail.env"), "# comment\npython.version = 3.12\nquote=a\"b\n").unwrap();
        let settings = super::settings(&root, &["python.version=3.13".into()]).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(settings, vec![("quote".into(), "a\"b".into()), ("python.version".into(), "3.13".into())]);
        // Rows given to a program that declares `env`, and to no other (decision 0249).
        use crate::code_rule_dsl::logic::front::Value;
        let added = crate::code_rule_dsl::logic::plan::corpus::added::Added::settings(&settings);
        use crate::code_rule_dsl::logic::program_image::{Builder, fields::ProgramFields};
        let mut declares = Builder::base();
        declares.fields().declare("env", &[("key", "symbol"), ("value", "symbol")]);
        added.add_to(&mut declares.fields());
        let rows: Vec<Vec<Value>> = declares.fact_rows("env").iter().map(|r| r.to_vec()).collect();
        assert_eq!(rows, [vec![Value::Str("quote".into()), Value::Str("a\"b".into())], vec![Value::Str("python.version".into()), Value::Str("3.13".into())]]);
        let mut not = Builder::base();
        not.fields().declare("envy", &[("x", "symbol")]);
        added.add_to(&mut not.fields());
        assert!(!not.has_fact_rows("env"));
    }

    #[test]
    fn the_command_lines_runs_keep_their_outputs_with_or_without_options() {
        use serde_json::json;
        // `run-policies` passes no options (`null`): its runs ask for the cutoff too.
        assert_eq!(super::with_reuse(&serde_json::Value::Null), json!({"reuse": true}));
        assert_eq!(super::with_reuse(&json!({"x": 1})), json!({"x": 1, "reuse": true}));
        assert_eq!(super::with_reuse(&json!({"reuse": false})), json!({"reuse": false}));
    }

    /// `run-policies --tool-calls <calls.json>` gives the run its tool calls (`docs/rule-syntax/tool-calls.md`): a
    /// `.scope tool_calls` policy's finding is at the call. A file not of their shape, or no policy, is an error, and
    /// the form with no calls runs as before.
    #[test]
    fn run_policies_takes_its_tool_calls_from_the_command_line() {
        let dir = tempfile::tempdir().unwrap();
        let at = |name: &str| dir.path().join(name).display().to_string();
        std::fs::create_dir_all(at("root")).unwrap();
        std::fs::write(at("p.rerail"), ".policy \"t:cli-calls\" revision 1\n.scope tool_calls\n.violation Push(at: symbol) code \"no-push\" \"a push\"\nPush(c) :- tool_input(c, \"command\", s), match(\"(?s:.*)(?:git push)(?s:.*)\", s).\n").unwrap();
        std::fs::write(at("calls.json"), r#"[{"id": "e1", "tool": "Bash", "input": {"command": "git push"}}]"#).unwrap();
        std::fs::write(at("bad.json"), "{}").unwrap();
        let shared = crate::code_rule_dsl::logic::run::Shared::new();
        let run = |args: &[String]| super::cli_in(&[vec!["run-policies".to_owned()], args.to_vec()].concat(), &mut Default::default(), &shared).0;
        let at_of = |out: String| -> Vec<serde_json::Value> {
            let report: serde_json::Value = serde_json::from_str(&out).unwrap();
            report["diagnostics"].as_array().unwrap().iter().map(|d| d["at"].clone()).collect()
        };
        assert_eq!(at_of(run(&["--tool-calls".into(), at("calls.json"), at("root"), at("out1"), at("p.rerail")]).unwrap()), [serde_json::json!("e1")]);
        assert_eq!(at_of(run(&[at("root"), at("out2"), at("p.rerail")]).unwrap()), Vec::<serde_json::Value>::new());
        let shape = run(&["--tool-calls".into(), at("bad.json"), at("root"), at("out3"), at("p.rerail")]).unwrap_err();
        assert!(shape.contains("toolCalls: a JSON array"), "{shape}");
        for args in [vec![at("root"), at("out4")], vec![at("root"), at("out4"), at("p.rerail"), "--tool-calls".into()]] {
            let usage = run(&args).unwrap_err();
            assert!(usage.starts_with("usage: rerail-engine run-policies [--tool-calls <calls.json>]"), "{args:?}: {usage}");
        }
    }
}

/// Every pack and the data layer's policies are split with no undeclared crossing (decision 0652): each run as
/// `run-policies` runs them, as one pack per language (`RERAIL_BATCH_PACK=strict`: a failed pack is its error), over
/// a file of each language. Their `.scope` lines are dropped, so the file is in every policy's scope. Any run error
/// fails the test, every one shown.
#[cfg(test)]
mod pack_split_tests {
    /// Every policy of `dir`.
    fn pack_runs(dir: &str) {
        pack_shard_runs(dir, None)
    }

    /// Every policy of `dir`, or with `shard` (k, n) those whose place in the sorted list is k modulo n: python-main's
    /// 537 policies, 142 of which ask for guests of their own and run alone, take 84 s of CPU at once,
    /// JavaScript's 121 about 15 s (decision 0652).
    fn pack_shard_runs(dir: &str, shard: Option<(usize, usize)>) {
        pack_runs_as(dir, shard, "strict")
    }

    /// Every policy of `dir` run alone (`RERAIL_BATCH_PACK=off`), as a run of one policy is: a policy batched with
    /// others is split as part of their pack, which can hold a relation one file's that the policy alone does not.
    /// bounded-composed-graph-paths was refused alone over the corpus, its batch passing (decision 0665).
    fn pack_alone_runs(dir: &str) {
        pack_runs_as(dir, None, "off")
    }

    fn pack_runs_as(dir: &str, shard: Option<(usize, usize)>, batch: &str) {
        // SAFETY: nextest runs each test in a process of its own.
        unsafe { std::env::set_var("RERAIL_BATCH_PACK", batch) };
        let from = crate::source_tree::native().join("../docs/rule-syntax").join(dir);
        let tmp = tempfile::tempdir().unwrap();
        let rules = tmp.path().join("rules").join(dir);
        let mut policies = Vec::new();
        let mut walk = vec![from.clone()];
        while let Some(d) = walk.pop() {
            for e in std::fs::read_dir(&d).unwrap() {
                let path = e.unwrap().path();
                if path.is_dir() {
                    walk.push(path);
                    continue;
                }
                if path.extension().is_none_or(|x| x != "rerail") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                let to = rules.join(path.strip_prefix(&from).unwrap());
                std::fs::create_dir_all(to.parent().unwrap()).unwrap();
                // The scope's paths dropped (`include`, `exclude`, `dirs`), with the lines a list of them goes on to
                // (an indented line after one ending in a comma): every policy runs over the corpus. What a policy
                // reads (`.scope tool_calls`) stays.
                let paths = |l: &str| l.strip_prefix(".scope").is_some_and(|r| ["include", "exclude", "dirs"].iter().any(|w| r.trim_start().starts_with(w)));
                let code = |l: &str| l.split("//").next().unwrap_or("").trim_end().to_owned();
                let mut dropping = false;
                let kept: String = text.lines().filter(|l| {
                    let drop = paths(l) || (dropping && l.starts_with(char::is_whitespace));
                    dropping = drop && code(l).ends_with(',');
                    !drop
                }).map(|l| format!("{l}\n")).collect();
                std::fs::write(&to, kept).unwrap();
                if text.lines().any(|l| l.starts_with(".policy")) {
                    policies.push(to);
                }
            }
        }
        policies.sort();
        if let Some((k, n)) = shard {
            policies = policies.into_iter().enumerate().filter(|(i, _)| i % n == k).map(|(_, p)| p).collect();
        }
        let root = tmp.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        for (name, text) in [
            ("a.py", "import os\n\ndef f(x):\n    return os.path.join(x, 'a')\n"),
            ("a.js", "export function f(x) { return x + 1; }\n"),
            ("a.ts", "export function f(x: number): number { return x + 1; }\n"),
            ("a.tsx", "export function A() { return <div>a</div>; }\n"),
            ("a.php", "<?php\nfunction f($x) { return $x + 1; }\n"),
            ("a.blade.php", "<div>{{ $x }}</div>\n"),
            ("a.rs", "fn f(x: i32) -> i32 { x + 1 }\n"),
            ("a.sql", "select a from t where b = 1;\n"),
            ("a.vue", "<template><div>{{ x }}</div></template>\n<script>export default {}</script>\n"),
        ] {
            std::fs::write(root.join(name), text).unwrap();
        }
        let report = super::run_policies(&policies, &root, &tmp.path().join("out"), &serde_json::Value::Null).unwrap();
        let mut errors: Vec<String> = report["runs"].as_array().unwrap().iter().filter_map(|r| r["error"].as_str()).map(str::to_owned).collect();
        errors.sort();
        errors.dedup();
        assert!(errors.is_empty(), "{dir}: {} policies, {} distinct errors:\n{}", policies.len(), errors.len(), errors.join("\n\n"));
    }

    /// `policy`'s findings over `files` (path, text), each as (code, path, line, message), sorted.
    fn findings_of(policy: &str, files: &[(&str, &str)]) -> Vec<(String, String, i64, String)> {
        findings_of_all(&[policy], files)
    }

    /// `findings_of` for several policies run together, as a pack's are.
    fn findings_of_all(policies: &[&str], files: &[(&str, &str)]) -> Vec<(String, String, i64, String)> {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        for (path, text) in files {
            let to = root.join(path);
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            std::fs::write(to, text).unwrap();
        }
        let policies: Vec<std::path::PathBuf> = policies.iter().map(|p| crate::source_tree::native().join("../docs/rule-syntax").join(p)).collect();
        let report = super::run_policies(&policies, &root, &tmp.path().join("out"), &serde_json::Value::Null).unwrap();
        let errors: Vec<&str> = report["runs"].as_array().unwrap().iter().filter_map(|r| r["error"].as_str()).collect();
        assert!(errors.is_empty(), "{errors:?}");
        let mut found: Vec<(String, String, i64, String)> = report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| (d["code"].as_str().unwrap_or("").to_owned(), d["path"].as_str().unwrap_or("").to_owned(), d["line"].as_i64().unwrap_or(0), d["message"].as_str().unwrap_or("").to_owned()))
            .collect();
        found.sort();
        found
    }

    /// plan-helpers-allowlist finds what its linter finds (scripts/lint_plan_helpers_allowlist.py, decision 0692): a
    /// name's helper is the latest write in its own file (the linter's map is one per file: `a.py` and `b.py` write `h`
    /// at the same visit index, each a helper of its own, and neither's `fetch` takes the other's), and a private or
    /// eager-sounding name is one by its spelling alone.
    #[test]
    fn plan_helpers_allowlist_finds_what_its_linter_finds() {
        let found = findings_of(
            "data-layer/plan-helpers-allowlist.rerail",
            &[
                ("impl/a.py", "h = grammar_row_lazy\nfetch = h\n"),
                ("impl/b.py", "h = execute_via_plan\nfetch = h\n"),
                ("impl/c.py", "from impl.common.l2070_server_plan.plan_helpers import _x_via_plan\nfrom impl.common.l2070_server_plan import plan_helpers as ph\nph._y_via_plan()\n"),
                ("impl/d.py", "from impl.common.l2070_server_plan.plan_helpers import grammar_row_lazy as execute_rows\n"),
            ],
        );
        let want = |path: &str, line: i64, message: &str| ("PHA001".to_owned(), path.to_owned(), line, message.to_owned());
        assert_eq!(
            found,
            [
                want("impl/a.py", 2, "lazy terminal grammar_row_lazy assigned to eager alias"),
                want("impl/b.py", 1, "old or private execute_via_plan alias is forbidden"),
                want("impl/b.py", 2, "old or private execute_via_plan alias is forbidden"),
                want("impl/c.py", 1, "old or private _x_via_plan import is forbidden"),
                want("impl/c.py", 3, "old or private _y_via_plan() call is forbidden"),
                want("impl/d.py", 1, "lazy terminal grammar_row_lazy imported under eager alias"),
            ]
        );
    }

    /// pure-lazy-runtime-purity closes over the calls as its linter does (scripts/lint_pure_lazy_runtime_purity.py,
    /// `_pure_lazy_closure`, `_runtime_owner_closure`, decision 0693): from a `@pure_lazy` def through an imported
    /// helper in another file, through `self.` to a base class's method in a third, stopping at a `@legacy_*` def, and
    /// a def nothing reaches is not analysed. PLP-007: a public `*_lazy` def `data_layer/api.py` exports, named in its
    /// `__all__` and imported, directly or through a module that re-exports it, is reported at its def. The third and
    /// fourth files name a trigger (`scan_paths` reads a file a
    /// seed reaches only through a base class when its text names one). Each analysed def's findings are its own file's, and the split derives
    /// the analysis per file: only the walk over the records' ids crosses files.
    #[test]
    fn pure_lazy_closes_over_the_calls_and_analyses_each_file_on_its_own() {
        let tmp = tempfile::tempdir().unwrap();
        // SAFETY: nextest runs each test in a process of its own.
        unsafe { std::env::set_var("RERAIL_SPLIT_DUMP", tmp.path().join("split.txt")) };
        let found = findings_of(
            "data-layer/pure-lazy-runtime-purity.rerail",
            &[
                ("impl/a.py", "from impl.b import helper\nfrom impl.c import Base\n\n\n@pure_lazy\ndef entry(x):\n    return helper(x)\n\n\nclass Child(Base):\n    @pure_lazy\n    def run(self, x):\n        return self.step(x)\n"),
                ("impl/b.py", "def helper(txn):\n    return txn._conn\n\n\ndef unreached(txn):\n    return txn._conn\n"),
                ("impl/c.py", "from impl.d import old\n\n\nclass Base:\n    def step(self, txn):\n        y = txn._conn\n        return old(txn)\n# pure_lazy: scanned\n"),
                ("impl/d.py", "@legacy_sync\ndef old(txn):\n    return txn._conn\n# pure_lazy: scanned\n"),
                ("data_layer/api.py", "from impl.e import fetch_lazy\n\n__all__ = [\"fetch_lazy\", \"go_lazy\"]\nfrom impl.f import go_lazy\n"),
                ("impl/e.py", "def fetch_lazy(x):\n    return x\n"),
                ("impl/f.py", "from impl.g import go_lazy\n"),
                ("impl/g.py", "def go_lazy():\n    return 1\n"),
            ],
        );
        // The plugin's grandfathered list is not in the tree (PLP-ALLOWLIST).
        let keys: Vec<(&str, &str, i64)> = found.iter().filter(|f| f.0 != "PLP-ALLOWLIST").map(|(c, p, l, _)| (c.as_str(), p.as_str(), *l)).collect();
        assert_eq!(keys, [("PLP-001", "impl/c.py", 6), ("PLP-003", "impl/b.py", 2), ("PLP-007", "impl/e.py", 1), ("PLP-007", "impl/g.py", 1)], "{found:#?}");
        let causes: String = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.to_string_lossy().ends_with(".causes.txt"))
            .map(|p| std::fs::read_to_string(p).unwrap())
            .collect();
        assert!(!causes.is_empty(), "no split dumped");
        for r in ["plp_an", "plp_in", "plp_tnt_at", "plp_opq_before", "plp_bfs", "plp_find", "plp_kept"] {
            let named: Vec<&str> = causes.lines().filter(|l| l.split('\t').next() == Some(r)).collect();
            assert!(named.is_empty(), "{r} is cross-file: {named:?}");
        }
    }

    /// cached-pure-lazy-allowlist's cross-file part reads no position (decision 0695): the verifier keeps the last
    /// item and the last policy entry of a name (its dicts), which is chosen in each file, by `earlier`, and between
    /// files by path (the verifier's order across files is `rglob`'s, which the filesystem decides). A function
    /// defined twice in one file is compared at its second definition, an entry listed twice at its second.
    #[test]
    fn cached_pure_lazy_compares_the_last_of_a_name_and_reads_no_position_across_files() {
        // SAFETY: nextest runs each test in a process of its own.
        let found = findings_of(
            "data-layer/cached-pure-lazy-allowlist.rerail",
            &[
                ("data_layer/a.py", "from l2055_lazy_result import cached_pure_lazy\n\n\n@cached_pure_lazy(cache_args=(\"x\",))\ndef f(x):\n    return x.a\n\n\n@cached_pure_lazy(cache_args=(\"x\",))\ndef f(x):\n    return x.b\n"),
                ("data_layer/b.py", "from l2055_lazy_result import cached_pure_lazy\n\n\n@cached_pure_lazy(cache_args=(\"y\",))\ndef g(y):\n    return y.c\n"),
                ("l2055_lazy_result/cached_pure_lazy_policy.py", "CachedPureLazyPolicyEntry(function=\"data_layer.a.f\", cache_args=(\"x\",), cache_types=(), cache_members=((\"x\", \"a\"),), owner_scope=\"optimizer_run\", rationale=\"r\", body_hash=\"h\", signature_hash=\"h\")\nCachedPureLazyPolicyEntry(function=\"data_layer.a.f\", cache_args=(\"x\",), cache_types=(), cache_members=((\"x\", \"b\"),), owner_scope=\"optimizer_run\", rationale=\"\", body_hash=\"h\", signature_hash=\"h\")\n"),
            ],
        );
        let mut messages: Vec<&str> = found.iter().map(|(_, _, _, m)| m.as_str()).collect();
        messages.sort();
        // The second entry's empty rationale is the one compared, and the second definition's member `x.b` matches it.
        assert_eq!(
            messages,
            [
                "data_layer.a.f: a cache argument cannot escape through return",
                "data_layer.a.f: cache argument 'x' must be keyword-only",
                "data_layer.a.f: cache argument 'x' requires an exact annotation",
                "data_layer.a.f: cache type metadata must name each cache argument exactly once",
                "data_layer.a.f: function body hash changed",
                "data_layer.a.f: function signature hash changed",
                "data_layer.a.f: review rationale is required",
                "duplicate decorator entry: data_layer.a.f",
                "duplicate policy entry: data_layer.a.f",
                "unsigned cached pure-lazy function: data_layer.b.g",
            ]
        );
    }

    /// eager-resolve's cross-file part reads no position (decision 0695): a local made lazy by an annotation is lazy
    /// after its statement only (`er_lazy_val`, by `earlier`, in its file), and the facade modules' symbols are
    /// exempt by the file's path and the symbol's name alone (`_should_ignore_pattern_violation`), not by every
    /// file's symbols.
    #[test]
    fn eager_resolve_finds_what_its_linter_finds_and_reads_no_position_across_files() {
        // SAFETY: nextest runs each test in a process of its own.
        let found = findings_of(
            "data-layer/eager-resolve.rerail",
            &[
                ("impl/x.py", "def f(x: LazyResult):\n    y = x\n    return y.resolve()\n\n\ndef g(z):\n    w = z\n    return w.resolve()\n"),
                ("l2055_lazy_result/lazy_result.py", "class LazyResult:\n    def peek(self):\n        return self.resolve()\n\n\nclass Other:\n    def peek(self):\n        return self.resolve()\n"),
            ],
        );
        // The signed lists are not in the tree (EAGER-ALLOWLIST). LazyResult.peek is a facade symbol in its facade file.
        let keys: Vec<(&str, &str, i64)> = found.iter().filter(|f| f.0 != "EAGER-ALLOWLIST").map(|(c, p, l, _)| (c.as_str(), p.as_str(), *l)).collect();
        assert_eq!(
            keys,
            [
                (".resolve()", "impl/x.py", 3),
                (".resolve()", "impl/x.py", 8),
                (".resolve()", "l2055_lazy_result/lazy_result.py", 8),
                ("transitive:*.resolve()", "impl/x.py", 1),
                ("transitive:*.resolve()", "impl/x.py", 6),
                ("transitive:*.resolve()", "l2055_lazy_result/lazy_result.py", 7),
            ],
            "{found:#?}"
        );
    }

    /// lazy-architecture's delegates are a set of names (decision 0695): a `_lazy` function that defers to a function
    /// of another file which does plan I/O is LA-009 at its first such call, as `facts_by_symbol` finds it across files;
    /// the set is every file's names of such functions, read per file, and the cross-file part reads no position.
    #[test]
    fn lazy_architecture_finds_a_delegate_in_another_file_and_reads_no_position_across_files() {
        // SAFETY: nextest runs each test in a process of its own.
        let found = findings_of(
            "data-layer/lazy-architecture.rerail",
            &[
                ("impl/a.py", "from l2055_lazy_result import defer_lazy_result\nfrom impl.b import eager, quiet\n\n\ndef go_lazy():\n    x = defer_lazy_result(sync_resolve=quiet)\n    y = defer_lazy_result(sync_resolve=eager)\n    return defer_lazy_result(async_resolve=eager)\n"),
                ("impl/b.py", "from l2060 import grammar_row_lazy\n\n\ndef eager():\n    return grammar_row_lazy(1)\n\n\ndef quiet():\n    return 1\n"),
                ("impl/c.py", "from impl.b import eager\nfrom l2055_lazy_result import defer_lazy_result\n\n\ndef other_lazy():\n    return defer_lazy_result(sync_resolve=eager)\n"),
                (".ratchet-baselines/lint_lazy_architecture.json", "{\"counts\": {\"LA-009\": 1, \"LA-013\": 9}}"),
            ],
        );
        let keys: Vec<(&str, &str, i64)> = found.iter().map(|(c, p, l, _)| (c.as_str(), p.as_str(), *l)).collect();
        // LA-009 at the first call deferring to `eager` (not `quiet`), once in each file; LA-013 at each file's first
        // call. The budget counts every file's sites: LA-009's two are over its ceiling of 1, in no one file.
        assert_eq!(
            keys,
            [
                ("LA-009", ".ratchet-baselines/lint_lazy_architecture.json", 1),
                ("LA-009", "impl/a.py", 7),
                ("LA-009", "impl/c.py", 6),
                ("LA-013", "impl/a.py", 6),
                ("LA-013", "impl/c.py", 6),
            ],
            "{found:#?}"
        );
    }

    /// A pack's resolution through re-exports is staged when a policy beside it negates a conjunction with a constant
    /// test (decision 0695): D412's `!(doc_sphinx(s, c), k = 1)` made a negation context of `k` alone, with no
    /// location, a join of documents that hid behind the exports' cone and kept the whole cone, `resolve_export` with
    /// it, out of the stage. Run together, neither policy's cross-file part reads a position.
    #[test]
    fn a_packs_resolution_is_staged_beside_a_negated_conjunction() {
        // SAFETY: nextest runs each test in a process of its own.
        let found = findings_of_all(
            &["packs/python/ruff-sim910-dict-get-with-none-default.rerail", "packs/python/ruff-d412-blank-lines-between-header-and-content.rerail"],
            &[
                ("pkg/__init__.py", "from pkg.m import f\n\n__all__ = [\"f\"]\n"),
                ("pkg/m.py", "def f(k):\n    \"\"\"Do it.\n\n    Args:\n\n        k: the key.\n    \"\"\"\n    d = {}\n    return d.get(k, None)\n\n\ndef g():\n    \"\"\"Do it.\n\n    Notes:\n\n    .. note:: x\n    \"\"\"\n"),
            ],
        );
        let keys: Vec<(&str, &str, i64)> = found.iter().map(|(c, p, l, _)| (c.as_str(), p.as_str(), *l)).collect();
        // `f`'s Args header is followed by a blank line; `g`'s Notes header by one before an RST directive, which D412
        // lets be (the negated conjunction, `k = 1`).
        assert_eq!(keys, [("D412", "pkg/m.py", 4), ("SIM910", "pkg/m.py", 9)], "{found:#?}");
    }

    /// The Django practices name one write, query or caller of several, across files, by an order that edits elsewhere
    /// do not move (decision 0695): its def's qualified name, then its place in the def. The cross-file part reads no
    /// position.
    #[test]
    fn the_django_practices_name_one_of_several_by_a_key_edits_elsewhere_do_not_move() {
        // SAFETY: nextest runs each test in a process of its own.
        let found = findings_of_all(
            &[
                "packs/django/django-practice-get-handler-writes.rerail",
                "packs/django/django-practice-sync-orm-reached-from-async.rerail",
                "packs/django/django-practice-select-for-update-outside-atomic.rerail",
            ],
            &[
                ("shop/__init__.py", ""),
                ("shop/models.py", "from django.db import models\n\n\nclass Book(models.Model):\n    pass\n"),
                ("shop/services.py", "from shop.models import Book\n\n\ndef record():\n    Book.objects.filter(a=1).update(b=2)\n    Book.objects.create(a=1)\n\n\ndef fetch():\n    x = 1\n    return Book.objects.get(pk=1)\n\n\ndef lock(pk):\n    return Book.objects.select_for_update().get(pk=pk)\n"),
                ("shop/views.py", "from django.views.decorators.http import require_GET\nfrom shop.services import record, fetch, lock\n\n\n@require_GET\ndef detail(request):\n    record()\n\n\nasync def page(request):\n    return fetch()\n\n\ndef zed(request):\n    x = 1\n    lock(1)\n"),
                ("shop/other.py", "from shop.services import lock\n\n\ndef alpha(request):\n    a = 1\n    b = 2\n    c = 3\n    d = 4\n    e = 5\n    f = 6\n    g = 7\n    h = 8\n    i = 9\n    j = 10\n    k = 11\n    l = 12\n    lock(2)\n"),
            ],
        );
        let got: Vec<(&str, &str, i64, &str)> =
            found.iter().map(|(c, p, l, m)| (c.as_str(), p.as_str(), *l, m.split(". ").next().unwrap_or(""))).collect();
        // record's first write, fetch's query; lock's bare callers are views.py's zed (line 16) and other.py's alpha (line
        // 17): alpha's def comes first by name, where the least line was zed's.
        assert_eq!(
            got,
            [
                ("GetHandlerWritesVia", "shop/views.py", 6, "`detail` answers a safe method (GET/HEAD) but writes to the database in `record()`, which it calls: `update()` at shop/services.py:5"),
                ("SelectForUpdateOutsideAtomic", "shop/services.py", 15, "`select_for_update()` in `lock` runs outside transaction.atomic(): `alpha` calls it outside a transaction at shop/other.py:17"),
                ("SyncOrmReached", "shop/views.py", 11, "`fetch()` runs a blocking ORM query (`get()` at shop/services.py:11) inside `async def`, where Django raises SynchronousOnlyOperation"),
            ],
            "{found:#?}"
        );
    }

    /// versioned-merged-identity finds what its checker finds (scripts/check_versioned_merged_identity.py, decision
    /// 0692): an unreviewed caller (VMI002), a reviewed one whose projection is assigned only after the call, through
    /// an imported alias (VMI003), one backed through `b = a` (nothing), each reviewed caller absent (VMI001, at the
    /// steps directory), and a caller two files share (`b.py:go`, under both roots) reported at the last path the
    /// plugin recorded for it, the graph query's, each at its own line. Each line is its constructor's file's, given by
    /// that file: the cross-file part joins callers' names to the path alone, and reads no position (decision 0695).
    #[test]
    fn versioned_merged_identity_finds_what_its_checker_finds() {
        // SAFETY: nextest runs each test in a process of its own.
        let steps = "impl/common/l2070_server_plan/steps";
        let graph = "impl/common/l9700_sql_graph_query";
        let found = findings_of(
            "data-layer/versioned-merged-identity.rerail",
            &[
                (&format!("{steps}/a.py"), "from m import SqlVersionedMergedCte\n\nclass Foo:\n    def compile(self):\n        return SqlVersionedMergedCte(read_projection=p)\n"),
                (&format!("{steps}/b.py"), "from m import SqlVersionedMergedCte\n\ndef go():\n    return SqlVersionedMergedCte()\n"),
                (&format!("{graph}/b.py"), "from m import SqlVersionedMergedCte\n\n\ndef go():\n    x = 1\n    return SqlVersionedMergedCte()\n"),
                (&format!("{steps}/versioned_query_steps.py"), "from m import SqlVersionedMergedCte as Cte\n\n\nclass VersionedSelectManyRawStep:\n    def compile(self):\n        out = Cte(read_projection=a)\n        a = catalog_versioned_merged_projection(M, cols)\n        return out\n"),
                (&format!("{steps}/fk_cascade_step.py"), "from m import SqlVersionedMergedCte\n\n\nclass RestrictCheckStep:\n    def compile(self):\n        a = catalog_versioned_merged_projection(M, cols)\n        b = a\n        return SqlVersionedMergedCte(read_projection=b)\n"),
            ],
        );
        let keys: Vec<(&str, &str, i64)> = found.iter().map(|(c, p, l, _)| (c.as_str(), p.as_str(), *l)).collect();
        let mut want: Vec<(&str, String, i64)> = vec![
            ("VMI002", format!("{steps}/a.py"), 5),
            ("VMI002", format!("{graph}/b.py"), 4),
            ("VMI002", format!("{graph}/b.py"), 6),
            ("VMI003", format!("{steps}/versioned_query_steps.py"), 6),
        ];
        want.extend(std::iter::repeat_n(("VMI001", steps.to_owned(), 1), 10));
        want.sort();
        let want: Vec<(&str, &str, i64)> = want.iter().map(|(c, p, l)| (*c, p.as_str(), *l)).collect();
        assert_eq!(keys, want, "{found:#?}");
        assert!(found.iter().filter(|(c, p, ..)| c == "VMI002" && p.starts_with(graph)).all(|(.., m)| m.ends_with("Caller: b.py:go.")), "{found:#?}");
    }

    /// The policies reading pyflakes-format's `str.format` family (F521, F522, F524, UP030, UP032) as one pack, with a
    /// policy of another scope (PYI021, stubs only: each rule then tests its documents' scope, `s_rr_scope_not`), through
    /// the whole preparation (their counts below a bound made ranks, decision 0361 step 7): no relation is cross-file
    /// through the family, a view of the format string's values each call holds in its own node (decision 0672). Read
    /// off the split's causes (`RERAIL_SPLIT_DUMP`).
    #[test]
    fn a_format_strings_ranked_fields_are_per_file() {
        let tmp = tempfile::tempdir().unwrap();
        let pack = crate::source_tree::native().join("../docs/rule-syntax/packs/python");
        let policies: Vec<std::path::PathBuf> = ["ruff-f521-string-dot-format-invalid-format", "ruff-f522-string-dot-format-extra-named-arguments", "ruff-f524-string-dot-format-missing-arguments", "ruff-up030-format-literals", "ruff-up032-f-string", "ruff-pyi021-docstring-in-stub"]
            .iter()
            .map(|p| pack.join(format!("{p}.rerail")))
            .collect();
        let root = tmp.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.py"), "x = \"{} {}\".format(a, b)\n").unwrap();
        let dump = tmp.path().join("split.txt");
        // SAFETY: nextest runs each test in a process of its own.
        unsafe {
            std::env::set_var("RERAIL_BATCH_PACK", "strict");
            std::env::set_var("RERAIL_SPLIT_DUMP", &dump);
        }
        let report = super::run_policies(&policies, &root, &tmp.path().join("out"), &serde_json::Value::Null).unwrap();
        let errors: Vec<&str> = report["runs"].as_array().unwrap().iter().filter_map(|r| r["error"].as_str()).collect();
        assert!(errors.is_empty(), "{errors:?}");
        // Every split the run made (the batch's, and each split again on the way: `split-N`).
        let causes: String = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.to_string_lossy().ends_with(".causes.txt"))
            .map(|p| {
                let name = p.file_name().unwrap().to_string_lossy().into_owned();
                std::fs::read_to_string(&p).unwrap().lines().map(|l| format!("{name}: {l}\n")).collect::<String>()
            })
            .collect();
        assert!(!causes.is_empty(), "no split dumped");
        let through: Vec<&str> = causes.lines().filter(|l| l.contains("brace_")).collect();
        assert!(through.is_empty(), "cross-file through the format family:\n{}\nall causes:\n{causes}", through.join("\n"));
    }

    macro_rules! packs {
        ($($name:ident: $dir:expr),* $(,)?) => {
            $(#[test] fn $name() { pack_runs($dir) })*
        };
    }

    packs! {
        the_data_layer_is_split_with_its_crossings_declared: "data-layer",
        the_blade_pack_is_split_with_its_crossings_declared: "packs/blade",
        the_complexity_pack_is_split_with_its_crossings_declared: "packs/complexity",
        the_django_pack_is_split_with_its_crossings_declared: "packs/django",
        the_next_pack_is_split_with_its_crossings_declared: "packs/next",
        the_node_pack_is_split_with_its_crossings_declared: "packs/node",
        the_php_pack_is_split_with_its_crossings_declared: "packs/php",
        the_react_pack_is_split_with_its_crossings_declared: "packs/react",
        the_rerail_pack_is_split_with_its_crossings_declared: "packs/rerail",
        the_rr_pack_is_split_with_its_crossings_declared: "packs/rr",
        the_rust_pack_is_split_with_its_crossings_declared: "packs/rust",
        the_sql_pack_is_split_with_its_crossings_declared: "packs/sql",
        the_typescript_pack_is_split_with_its_crossings_declared: "packs/typescript",
        the_vue_pack_is_split_with_its_crossings_declared: "packs/vue",
    }

    #[test]
    fn the_data_layer_is_split_with_its_crossings_declared_policy_by_policy() {
        pack_alone_runs("data-layer")
    }

    macro_rules! shards {
        ($dir:expr, $n:expr, $($name:ident: $k:expr),* $(,)?) => {
            $(#[test] fn $name() { pack_shard_runs($dir, Some(($k, $n))) })*
        };
    }

    shards! {
        "packs/python-main", 12,
        the_python_main_pack_is_split_with_its_crossings_declared_0: 0,
        the_python_main_pack_is_split_with_its_crossings_declared_1: 1,
        the_python_main_pack_is_split_with_its_crossings_declared_2: 2,
        the_python_main_pack_is_split_with_its_crossings_declared_3: 3,
        the_python_main_pack_is_split_with_its_crossings_declared_4: 4,
        the_python_main_pack_is_split_with_its_crossings_declared_5: 5,
        the_python_main_pack_is_split_with_its_crossings_declared_6: 6,
        the_python_main_pack_is_split_with_its_crossings_declared_7: 7,
        the_python_main_pack_is_split_with_its_crossings_declared_8: 8,
        the_python_main_pack_is_split_with_its_crossings_declared_9: 9,
        the_python_main_pack_is_split_with_its_crossings_declared_10: 10,
        the_python_main_pack_is_split_with_its_crossings_declared_11: 11,
    }

    shards! {
        "packs/python", 2,
        the_python_pack_is_split_with_its_crossings_declared_0: 0,
        the_python_pack_is_split_with_its_crossings_declared_1: 1,
    }

    shards! {
        "packs/javascript", 3,
        the_javascript_pack_is_split_with_its_crossings_declared_0: 0,
        the_javascript_pack_is_split_with_its_crossings_declared_1: 1,
        the_javascript_pack_is_split_with_its_crossings_declared_2: 2,
    }
}

#[cfg(test)]
mod git_revision_tests {
    use super::*;

    #[test]
    fn a_nested_working_tree_batch_leaves_publication_to_its_caller() {
        use super::super::logic::plan::corpus;
        use crate::content_store::{ContentStore, analysis_scope::AnalysisRun};
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("work");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("a.py"), "x = 1\n").unwrap();
        let rule = fixture.path().join("policy.rerail");
        std::fs::write(&rule, ".language python\n.violation Seen(at: node) \"identifier\"\nSeen(n) :- kind(n, \"identifier\").\n").unwrap();
        let path = fixture.path().join("store.sqlite");
        let mut caller = AnalysisRun::begin(ContentStore::open(&path).unwrap(), "named:outer", "outer").unwrap();
        let report = corpus::with_tape_cache_path(&path, Some((root.clone(), caller.owner.clone())), || {
            run_policies(&[rule], &root, &fixture.path().join("out"), &serde_json::Value::Null).unwrap()
        });
        assert_eq!(report["diagnostics"].as_array().unwrap().len(), 1);
        let db = rusqlite::Connection::open(&path).unwrap();
        let count = |pattern: &str| db.query_row("SELECT count(*) FROM analysis_scope_manifest WHERE scope GLOB ?1", [pattern], |r| r.get::<_, i64>(0)).unwrap();
        // A run that could not publish says why in its cache warnings.
        assert!(count("pending:*") > 0, "no pending rows: {}", report["cacheWarnings"]);
        assert_eq!(count("cwd:*"), 0);
        caller.finish(false).unwrap();
        assert_eq!(count("*"), 0);
    }

    #[test]
    fn revision_cli_reports_results_when_the_store_cannot_open() {
        use super::super::logic::plan::corpus;
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1")
                .arg("-C").arg(&root).args(args).output().unwrap();
            assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        };
        git(&["init", "-q"]);
        git(&["-c", "user.name=Example", "-c", "user.email=example@example.com", "commit", "--allow-empty", "-qm", "selected commit"]);
        let commit = git(&["rev-parse", "HEAD"]);
        let policy = fixture.path().join("rule.rerail");
        std::fs::write(&policy, r#"
#include "git.rerail"
.policy "offline-cache" revision 1
git_object_request(r, c) :- git_source_commit(r, c).
.violation Seen(at: span, text: symbol) "{text}"
Seen(@place("commit", 1), subject) :- git_source_commit(r, c), git_subject(r, c, subject).
"#).unwrap();
        let store = fixture.path().join("corrupt.sqlite");
        let corrupt = b"this is not a SQLite file";
        std::fs::write(&store, corrupt).unwrap();
        let options = serde_json::json!({"revision": "HEAD", "analysisScope": "review"});
        corpus::with_tape_cache_path(&store, None, || {
            for report in [
                run_rr(&policy, &root, &fixture.path().join("one"), &options).unwrap(),
                run_policies(std::slice::from_ref(&policy), &root, &fixture.path().join("batch"), &options).unwrap(),
            ] {
                assert_eq!(report["source"]["commit"], commit);
                assert_eq!(report["diagnostics"].as_array().unwrap().len(), 1, "{report}");
                assert_eq!(report["diagnostics"][0]["message"], "selected commit");
                assert!(report["cacheWarnings"][0].as_str().unwrap().contains("file is not a database"));
            }
            let out = fixture.path().join("logic");
            logic_command(&[
                policy.display().to_string(), root.display().to_string(), out.display().to_string(),
                "--revision".to_owned(), "HEAD".to_owned(), "--analysis-scope".to_owned(), "review".to_owned(),
            ]).unwrap();
            assert_eq!(diagnostics(&out).unwrap()[0]["message"], "selected commit");
        });
        assert_eq!(std::fs::read(&store).unwrap(), corrupt);

        let durable = fixture.path().join("durable.sqlite");
        corpus::with_tape_cache_path(&durable, None, || {
            let mut invocation = PolicyInvocation::begin(&root, Some("HEAD"), Some("review")).unwrap();
            let (program, files) = read_policy(&policy).unwrap();
            let attempts = std::cell::Cell::new(0);
            let mut report = invocation.evaluate(|source| {
                attempts.set(attempts.get() + 1);
                let run = || run_program_checked(program.clone(), &files, source, &fixture.path().join("retry"), &serde_json::Value::Null);
                if attempts.get() == 1 {
                    git(&["-c", "user.name=Example", "-c", "user.email=example@example.com", "commit", "--allow-empty", "-qm", "new commit"]);
                    std::fs::write(&policy, "replaced while the cache fails").unwrap();
                    corpus::with_tape_cache_path(&store, None, run)
                } else {
                    assert!(corpus::tape_cache_path().is_none());
                    run()
                }
            }).unwrap();
            invocation.report(&mut report, true);
            assert_eq!(attempts.get(), 2);
            assert_eq!(report["source"]["commit"], commit);
            assert_eq!(report["diagnostics"][0]["message"], "selected commit");
            assert!(report["cacheWarnings"][0].as_str().unwrap().contains("file is not a database"));
        });
    }

    #[test]
    fn revision_readers_use_git_objects_for_settings_inputs_and_world_queries() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("repo");
        std::fs::create_dir_all(root.join("allow")).unwrap();
        std::fs::create_dir_all(root.join("data")).unwrap();
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1")
                .arg("-C").arg(&root).args(args).output().unwrap();
            assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        };
        git(&["init", "-q"]);
        std::fs::write(root.join("a.py"), "committed = 1\n").unwrap();
        std::fs::write(root.join("rerail.env"), "mark = committed\n").unwrap();
        std::fs::write(root.join("data/value.json"), r#"{"title":"from-git"}"#).unwrap();
        std::fs::write(root.join("allow/one.txt"), "allowed\n").unwrap();
        git(&["add", "a.py", "rerail.env", "data/value.json", "allow/one.txt"]);
        git(&["-c", "user.name=Example", "-c", "user.email=example@example.com", "commit", "-qm", "source"]);
        let commit = git(&["rev-parse", "HEAD"]);
        std::fs::write(root.join("rerail.env"), "mark = local\n").unwrap();
        std::fs::write(root.join("data/value.json"), r#"{"title":"local"}"#).unwrap();
        std::fs::remove_file(root.join("a.py")).unwrap();
        std::fs::remove_file(root.join("allow/one.txt")).unwrap();
        std::fs::write(root.join("allow/local.txt"), "local\n").unwrap();
        let policy = fixture.path().join("rule.rerail");
        std::fs::write(&policy, r#"
.policy "git-readers" revision 1
.language python
.decl title(value: symbol)
.input title(IO=json, filename="data/value.json", path="/title")
.decl allowed(path: symbol, line: number, text: symbol)
.input allowed(IO=lines, dir="allow")
.violation Seen(at: span, key: symbol, value: symbol) "{key}:{value}"
Seen(@place("a.py", 1), "config", value) :- env("mark", value).
Seen(@place("a.py", 1), "json", value) :- title(value).
Seen(@place("a.py", 1), "dirinput", value) :- allowed("allow/one.txt", 1, value).
Seen(@place("a.py", 1), "line", value) :- file_line("allow/one.txt", 1, value).
Seen(@place("a.py", 1), "listing", value) :- dir_entry("allow", value, 0).
Seen(@place("a.py", 1), "source", value) :- doc(d, "a.py", _), src_line(d, 1, sp), text(sp, value).
"#).unwrap();
        let options = serde_json::json!({"revision": commit, "analysisScope": "review"});
        let report = run_rr(&policy, &root, &fixture.path().join("out"), &options).unwrap();
        let mut messages: Vec<_> = report["diagnostics"].as_array().unwrap().iter()
            .map(|row| row["message"].as_str().unwrap()).collect();
        messages.sort();
        assert_eq!(messages, ["config:committed", "dirinput:allowed", "json:from-git", "line:allowed", "listing:one.txt", "source:committed = 1"]);
        let repo = gix::discover(&root).unwrap();
        assert!(!repo.common_dir().join("rerail-object-views").exists());
        assert!(!root.join("a.py").exists());
    }

    #[test]
    fn revision_cli_applies_current_policy_to_committed_files_and_metadata() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1")
                .arg("-C").arg(&root).args(args).output().unwrap();
            assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        };
        git(&["init", "-q"]);
        std::fs::write(root.join("a.py"), "committed = 1\n").unwrap();
        let policy = root.join("rule.rerail");
        std::fs::write(&policy, r#"
.policy "historical-policy" revision 1
.language python
.violation Old(at: span) "historical policy ran"
Old(@place("a.py", 1)).
"#).unwrap();
        git(&["add", "a.py", "rule.rerail"]);
        git(&["-c", "user.name=Example", "-c", "user.email=example@example.com", "commit", "-qm", "snapshot message"]);
        let commit = git(&["rev-parse", "HEAD"]);
        let tree = git(&["rev-parse", "HEAD^{tree}"]);
        git(&["-c", "user.name=Other", "-c", "user.email=other@example.com", "commit", "--allow-empty", "-qm", "today"]);
        std::fs::write(root.join("a.py"), "dirty = 2\n").unwrap();
        std::fs::write(root.join("untracked.py"), "untracked = 3\n").unwrap();
        std::fs::write(&policy, r#"
#include "git.rerail"
.policy "git-snapshot" revision 1
.language python
git_revision_request(r, "HEAD") :- git_root_repo(r).
git_object_request(r, h) :- git_revision(r, "HEAD", h).
git_object_request(r, p) :- git_parent(r, _, _, p).
.violation Seen(at: span, text: symbol, message: symbol, email: symbol, head: symbol) "{text}"
Seen(@place(p, 1), text, message, email, head) :- doc(d, p, _), src_line(d, 1, sp), text(sp, text),
    git_source_commit(r, c), git_message(r, c, message), git_author(r, c, _, email, _, _),
    git_revision(r, "HEAD", h), git_parent(r, h, 0, c), git_subject(r, h, head).
"#).unwrap();
        let args = vec!["run-rr".to_owned(), policy.display().to_string(), root.display().to_string(),
            fixture.path().join("commit-out").display().to_string(), "--analysis-scope".to_owned(), "review".to_owned(), "--revision".to_owned(), commit];
        let result: serde_json::Value = serde_json::from_str(&cli_in(&args, &mut Default::default(), &crate::code_rule_dsl::logic::run::Shared::new()).0.unwrap()).unwrap();
        let diagnostics = result["diagnostics"].as_array().unwrap();
        assert_eq!(diagnostics.len(), 1, "{result}");
        assert_eq!(diagnostics[0]["code"], "Seen");
        assert_eq!(diagnostics[0]["path"], "a.py");
        assert_eq!(diagnostics[0]["parameters"]["text"], "committed = 1");
        assert_eq!(diagnostics[0]["parameters"]["message"], "snapshot message\n");
        assert_eq!(diagnostics[0]["parameters"]["email"], "example@example.com");
        assert_eq!(diagnostics[0]["parameters"]["head"], "today");
        assert_eq!(result["source"]["tree"], tree);
        std::fs::write(&policy, r#"
.policy "git-tree" revision 1
.language python
.violation Seen(at: span, text: symbol, tree: symbol) "{text}"
Seen(@place(p, 1), text, tree) :- doc(d, p, _), src_line(d, 1, sp), text(sp, text), git_rev("tree", tree).
"#).unwrap();
        let result = run_rr(&policy, &root, &fixture.path().join("tree-out"), &serde_json::json!({"revision": tree, "analysisScope": "review"})).unwrap();
        let diagnostics = result["diagnostics"].as_array().unwrap();
        assert_eq!(diagnostics.len(), 1, "{result}");
        assert_eq!(diagnostics[0]["parameters"]["text"], "committed = 1");
        assert_eq!(diagnostics[0]["parameters"]["tree"], tree);
        assert_eq!(std::fs::read_to_string(root.join("a.py")).unwrap(), "dirty = 2\n");
        assert!(root.join("untracked.py").exists());
        let mut missing = args.clone();
        missing.pop();
        assert!(cli_in(&missing, &mut Default::default(), &crate::code_rule_dsl::logic::run::Shared::new()).0.unwrap_err().contains("--revision requires"));
    }

    #[test]
    fn revision_cli_reads_commit_metadata_without_its_tree() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path();
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1")
                .current_dir(root).args(args).output().unwrap();
            assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        };
        git(&["init", "-q"]);
        let object = fixture.path().join("commit-without-tree");
        std::fs::write(&object, format!("tree {}\nauthor Test <test@example.com> 100 +0000\ncommitter Test <test@example.com> 100 +0000\n\nmessage without a tree\n", "1".repeat(40))).unwrap();
        let commit = git(&["hash-object", "-t", "commit", "-w", object.to_str().unwrap()]);
        let policy = root.join("metadata.rerail");
        std::fs::write(&policy, r#"
#include "git.rerail"
.language typescript
.decl subject(s: symbol)
git_object_request(r, c) :- git_source_commit(r, c).
subject(s) :- git_source_commit(r, c), git_subject(r, c, s).
.output subject
"#).unwrap();
        let output = fixture.path().join("metadata-out");
        cli_in(&["logic".into(), policy.display().to_string(), root.display().to_string(), output.display().to_string(),
            "--revision".into(), commit, "--analysis-scope".into(), "review".into()], &mut Default::default(), &crate::code_rule_dsl::logic::run::Shared::new()).0.unwrap();
        let facts = super::super::logic::plan::output::Outputs::open(&output).unwrap();
        assert_eq!(facts.rows("subject")[0].get(0).text(), Some("message without a tree"));
    }
}
