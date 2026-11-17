//! The engine's text tools without Node (list step 18; P `spec-tooling.md` §2.2.1–2.2.2): the
//! `rerail-engine` binary (`src/bin/rerail-engine.rs`) and the napi wrappers both call these.

pub use super::logic::profile::watch::watch;
pub use super::sql_executor::CodeRuleSqlExecutor;
pub use super::logic::settings::{program_text, program_texts, settings};
pub use super::logic::settings::process_counters;
use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::transducer::packed::rir;

/// The package CLI's SQL tool, sharing the standalone command's request and execution path.
#[napi_derive::napi(catch_unwind)]
pub fn execute_sql_request(store_path: String, request_json: String) -> napi::Result<String> {
    sql(std::path::Path::new(&store_path), &request_json).map_err(napi::Error::from_reason)
}

/// A SQL extension request for the standalone tool. The same session serves the bindings,
/// lowered rule program and subsequent SQL queries; no AST matcher is lowered to SQL.
pub fn sql(store: &std::path::Path, request: &str) -> Result<String, String> {
    let profiling = super::logic::profile::on();
    let _run = profiling.then(super::logic::profile::one_run);
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
pub const CAPABILITIES: &str = include_str!("capabilities.json");

/// Read a public `.rr` file, including its core prelude and libraries. The CLI and migration
/// harness use the same reader as the engine; they never strip directives or run a lab interpreter.
pub fn read_rr(path: &std::path::Path) -> Result<String, String> {
    let program = super::logic::rr::parse_file(path).map_err(|e| e.to_string())?;
    super::logic::rr::print(&program).map_err(|e| e.to_string())
}

/// Convert whole AST policies for review. Unsupported clauses refuse the entire candidate.
/// Each rule (a rule document, or a draft `{ kind, rule, cases }`) converted to `.rr`. A draft's cases, or the cases
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
                .and_then(|combined| super::logic::plan::lower::domains(&super::logic::front::prune(combined)).map(|_| ()).map_err(|e| e.to_string())) =>
            {
                serde_json::json!({"path": path, "status": "types-failed", "error": error})
            }
            Ok((program, files)) => serde_json::json!({
                "path": path, "status": "reader-passed", "rules": program.rules.len(),
                "outputs": program.outputs,
                "files": files.iter().map(|file| serde_json::json!({
                    "path": file.path,
                    "sha256": file.sha256.map(|hash| hash.iter().map(|b| format!("{b:02x}")).collect::<String>())
                })).collect::<Vec<_>>()
            }),
            Err(error) => serde_json::json!({"path": path, "status": "reader-failed", "error": error.to_string()})
        }
    }).collect())
}

/// One strict `.rr` replay. The caller supplies a corpus and typed policy options; findings are
/// produced by the native relational engine, including the original message and parameters.
pub fn run_rr(
    path: &std::path::Path,
    root: &std::path::Path,
    out: &std::path::Path,
    options: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    use super::logic::rr;
    let (program, files) = rr::read_file(path).map_err(|e| e.to_string())?;
    if !program.outputs.iter().any(|name| super::logic::rr::is_diagnostic(name)) {
        return Err(format!("{}: no .violation declarations", path.display()));
    }
    run_program(program, &files, root, out, options)
}

/// The findings of the run whose outputs are in `dir`, as the CLI reports them: every finding relation's rows
/// (`rr::lower::findings`), each as its JSON object.
pub fn diagnostics(dir: &std::path::Path) -> Result<Vec<serde_json::Value>, String> {
    let outputs = crate::code_rule_dsl::logic::plan::output::Outputs::open(dir).map_err(|e| e.to_string())?;
    let mut all = Vec::new();
    for rel in outputs.names().into_iter().filter(|r| super::logic::rr::is_diagnostic(r)) {
        for row in outputs.rows(rel) {
            all.push(diagnostic(&row)?);
        }
    }
    Ok(all)
}

/// A finding's row as its JSON object: `policy`, `clause`, `code`, `path`, `line`, `at`, `tag` (`null` for none),
/// `message`, and `parameters` by name. Each value as its column holds it: a text a string, an integer or a node a
/// number (a node's value as the run held it), NULL `null`. Nothing is parsed.
fn diagnostic(row: &crate::code_rule_dsl::logic::plan::output::Row<'_>) -> Result<serde_json::Value, String> {
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
    program: super::logic::front::Program,
    files: &[super::logic::rr::LibraryFile],
    root: &std::path::Path,
    out: &std::path::Path,
    options: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    use super::{
        logic::{plan::corpus, rr},
        typing::file_local::Binding,
    };
    let metadata = rr::metadata(&program).map_err(|e| e.to_string())?;
    // A language of several dialects (`.language typescript` over `.ts`, `.tsx` and `.js` files) is a run per dialect
    // with files in scope: the first into `out`, each other into `out/<dialect>`, their findings together. A language
    // of one dialect, or none with files, runs as written. Each run holds the program read, its language the
    // dialect's (the `dialect` option, `corpus::added`): never printed and read again (decision 0249).
    let dialects = corpus::run_dialects(root, &program, &with_reuse(options)).map_err(|e| e.to_string())?;
    // One dialect with files may be another than the one named: a `.js`-scoped TypeScript pack reads JavaScript.
    let runs: Vec<(Option<&str>, std::path::PathBuf)> = if dialects.is_empty() {
        vec![(None, out.to_path_buf())]
    } else {
        dialects.iter().enumerate().map(|(i, label)| (Some(*label), if i == 0 { out.to_path_buf() } else { out.join(label) })).collect()
    };
    let (mut rows, mut files_read, mut nodes, mut phases) = (Vec::new(), 0, 0, Vec::new());
    // The device as `logic --metal` places it: `RERAIL_DEVICE=metal` (by price) or `always` (wherever it can).
    let binding = if std::env::var("RERAIL_DEVICE").is_ok_and(|v| v == "metal" || v == "always") { Binding::Metal } else { Binding::Cpu };
    for (dialect, dir) in &runs {
        let mut options = with_reuse(options);
        if let Some(dialect) = dialect {
            options["dialect"] = serde_json::json!(dialect);
        }
        let report = corpus::run_read(&program, root, dir, "program", binding, 1 << 18, &Default::default(), &options).map_err(|e| e.to_string())?;
        rows.extend(diagnostics(dir)?);
        files_read += report.files;
        nodes += report.nodes;
        phases.extend(report.phases);
    }
    Ok(
        serde_json::json!({"diagnostics": rows, "files": files_read, "nodes": nodes,
                         "metadata": metadata,
                         "programInputs": files.iter().map(|file| serde_json::json!({
                             "path": file.path,
                             "sha256": file.sha256.map(|hash| hash.iter().map(|b| format!("{b:02x}")).collect::<String>())
                         })).collect::<Vec<_>>(),
                         "phases": phases.iter().map(|(phase,user,system,rss,statements)| serde_json::json!({
                             "phase": phase, "userCpuSeconds": user, "systemCpuSeconds": system,
                             "peakRssMB": rss, "submittedStatements": statements
                         })).collect::<Vec<_>>()}),
    )
}

/// Several `.rr` policies over one repository (O105): each runs over the documents of the language it names
/// (`.language python`, `rust`, `rr`), into its own `out/<k>`, through the one content store, so a file's tape is
/// made once whichever policy reads it. The findings of all of them, in the order of `paths`, and each run's
/// report.
pub fn run_policies(
    paths: &[std::path::PathBuf],
    root: &std::path::Path,
    out: &std::path::Path,
    options: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let (mut diagnostics, mut runs) = (Vec::new(), Vec::new());
    // One run for the file observer: each root asked of the journal once for the batch.
    let _observed = super::logic::plan::corpus::ObservedBatch::begin();
    // Every policy's includes resolved by one run of phase 0 (`bootstrap.rr`) over all of them, each graph kept for
    // the policy's own read: not one run a policy. A path phase 0 cannot read is the policy's own error, below.
    let seeds: Vec<super::logic::plan::corpus::phase0::Seed> =
        paths.iter().filter_map(|p| dunce::canonicalize(p).ok()).map(super::logic::plan::corpus::phase0::Seed::File).collect();
    if seeds.len() > 1 && !super::logic::plan::corpus::phase0::active() {
        let _ = super::logic::plan::corpus::phase0::graphs(&seeds);
    }
    // What the batch's runs share is kept while it runs (phase 1's runs, by their inputs), and nothing past it.
    let _batch = super::logic::plan::corpus::phases::Batch::enter();
    // The policies of one language run as one pack (decision 0168): libraries read, lowered, hidden and evaluated once
    // (decisions 0051, 0089), each policy's findings split back by its id. The rest run alone.
    // `RERAIL_BATCH_PACK=off` runs each policy alone, as before, to compare (a measurement's switch).
    let packed = std::env::var("RERAIL_BATCH_PACK").map_or(true, |v| v != "off");
    let texts = if packed { program_texts(&paths.iter().map(|p| p.as_path()).collect::<Vec<_>>()).ok() } else { None };
    let groups = texts.as_deref().map(|texts| policy_groups(texts, root, options)).unwrap_or_default();
    let mut done: Vec<Option<(Vec<serde_json::Value>, serde_json::Value)>> = paths.iter().map(|_| None).collect();
    for group in groups.iter().filter(|g| g.members.len() > 1) {
        if let Some(each) = run_pack(paths, texts.as_deref().unwrap_or_default(), group, root, out, options)? {
            for (&k, found) in group.members.iter().zip(each) {
                done[k] = Some(found);
            }
        }
    }
    for (k, path) in paths.iter().enumerate() {
        let (found, run) = match done[k].take() {
            Some(found) => found,
            None => {
                let report = run_rr(path, root, &out.join(k.to_string()), options).map_err(|e| format!("{}: {e}", path.display()))?;
                let run = serde_json::json!({"path": path.display().to_string(), "files": report["files"], "metadata": report["metadata"], "together": 1});
                (report["diagnostics"].as_array().cloned().unwrap_or_default(), run)
            }
        };
        diagnostics.extend(found);
        runs.push(run);
    }
    Ok(serde_json::json!({"diagnostics": diagnostics, "runs": runs}))
}

/// Policies of a batch that run as one pack: one language of one dialect, each with its own policy id (its findings
/// are told apart by it) and findings declared. `members` index the batch's paths; `metadata` is each one's.
struct PolicyGroup {
    members: Vec<usize>,
    ids: Vec<String>,
    metadata: Vec<serde_json::Value>,
}

/// The batch's policies (`texts`, as `program_texts` reads them) grouped by the grammar each run reads (decision
/// 0168, `corpus::run_language`: its `.language`'s, else the tree's), each read for its header only (`rr::Text::header`:
/// its directives and declarations, no clause), which is all a group needs before its run.
/// Findings are told apart by their policy's `.policy` id, so a group holds each id once (one policy of none). A
/// policy whose header does not read, has an id another of its grammar has, declares no findings, or names a
/// language of several dialects runs as `run_rr` runs it (and fails there with its own error).
fn policy_groups(texts: &[String], root: &std::path::Path, options: &serde_json::Value) -> Vec<PolicyGroup> {
    use super::logic::{plan::corpus, rr};
    let mut groups: Vec<(&'static str, PolicyGroup)> = Vec::new();
    for (k, text) in texts.iter().enumerate() {
        let own = rr::text_header(text).ok();
        let joined = own.and_then(|own| {
            let named = own.settings.iter().find(|(k, _)| k == "language").map(|(_, v)| v.trim().trim_matches('"').to_owned());
            let one_dialect = named.as_deref().is_none_or(|name| {
                crate::code_rule_dsl::language::CodeRuleLanguage::ALL.into_iter().find(|l| l.name() == name).is_some_and(|l| l.dialects().len() < 2)
            });
            let language = corpus::run_language(root, &own, &with_reuse(options)).ok()?;
            let id = own.settings.iter().find(|(k, _)| k == "policy").and_then(|(_, t)| t.split('"').nth(1)).unwrap_or_default().to_owned();
            let findings = own.outputs.iter().any(|o| rr::is_diagnostic(o));
            let metadata = rr::metadata(&own).ok()?;
            (one_dialect && findings).then_some((language, id, metadata))
        });
        match joined {
            Some((language, id, metadata)) if !groups.iter().any(|(_, g)| g.ids.contains(&id)) => {
                let at = match groups.iter().position(|(l, _)| *l == language) {
                    Some(at) => at,
                    None => {
                        groups.push((language, PolicyGroup { members: Vec::new(), ids: Vec::new(), metadata: Vec::new() }));
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

/// A pack member's findings and its run's record (`run_pack`).
type KeyedRow = (Vec<serde_json::Value>, serde_json::Value);

/// A run of programs in one dialect (`None`: as each is written), and the programs.
type DialectRun = (Option<&'static str>, Vec<String>);

/// One group's policies run as one pack into `out/pack-<first>`, and each one's findings written to its own
/// `out/<k>` (its `rr_diagnostic`, in `outputs.rro`): per member, its findings and its run's record. `None` when the pack's run fails or a
/// finding names no member's id: the group's policies then run alone, each with its own error.
fn run_pack(
    paths: &[std::path::PathBuf],
    texts: &[String],
    group: &PolicyGroup,
    root: &std::path::Path,
    out: &std::path::Path,
    options: &serde_json::Value,
) -> Result<Option<Vec<KeyedRow>>, String> {
    use super::{logic::plan::corpus, typing::file_local::Binding};
    let texts: Vec<&str> = group.members.iter().map(|&k| texts[k].as_str()).collect();
    let binding = if std::env::var("RERAIL_DEVICE").is_ok_and(|v| v == "metal" || v == "always") { Binding::Metal } else { Binding::Cpu };
    let dir = out.join(format!("pack-{}", group.members[0]));
    crate::code_rule_dsl::logic::profile::decision("a batch's policies as one pack (decision 0168)", || {
        format!("{} policies: {}", texts.len(), group.ids.join(", "))
    });
    let Ok(report) = corpus::run_with_options(&texts, root, None, &dir, "program", binding, 1 << 18, &Default::default(), &with_reuse(options)) else {
        return Ok(None);
    };
    let mut each: Vec<Vec<serde_json::Value>> = group.members.iter().map(|_| Vec::new()).collect();
    let outputs = crate::code_rule_dsl::logic::plan::output::Outputs::open(&dir).map_err(|e| e.to_string())?;
    // Each member's rows of each finding relation, by the policy each row names (its first column).
    type Rows<'a> = Vec<crate::code_rule_dsl::logic::plan::output::Row<'a>>;
    let mut rows: Vec<std::collections::BTreeMap<String, Rows<'_>>> = group.members.iter().map(|_| std::collections::BTreeMap::new()).collect();
    for rel in outputs.names().into_iter().filter(|r| super::logic::rr::is_diagnostic(r)) {
        for row in outputs.rows(rel) {
            let Some(at) = row.get(0).text().and_then(|id| group.ids.iter().position(|x| x == id)) else { return Ok(None) };
            each[at].push(diagnostic(&row)?);
            rows[at].entry(rel.to_owned()).or_default().push(row);
        }
    }
    let mut done = Vec::with_capacity(group.members.len());
    for (((&k, found), rows), metadata) in group.members.iter().zip(each).zip(rows).zip(&group.metadata) {
        let own = out.join(k.to_string());
        std::fs::create_dir_all(&own).map_err(|e| format!("{}: {e}", own.display()))?;
        // Its own outputs, a base of its findings alone (decision 0239).
        let mut image = crate::code_rule_dsl::logic::plan::output::Builder::new(outputs.images().last().map_or(0, crate::code_rule_dsl::logic::plan::output::Image::flags), false);
        for (rel, rows) in &rows {
            let types = outputs.get(rel).map(|r| r.types().to_vec()).unwrap_or_default();
            image.rows_of(rel, &types, rows);
        }
        crate::code_rule_dsl::logic::plan::output::write(&own, &image.finish()).map_err(|e| format!("{}: {e}", own.display()))?;
        let run = serde_json::json!({"path": paths[k].display().to_string(), "files": report.files, "metadata": metadata, "together": group.members.len()});
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
            CodeRuleDslRuntime::compile(source.clone(), format.clone(), capabilities.to_string())
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
/// and `core.rr` resolved, as one program's text.
/// The file's text is kept as written (`program_text`), so its tests and fixture sets (decision 0130) go with it.
pub fn flatten(path: &std::path::Path) -> Result<String, String> {
    program_text(path)
}

/// `programs` (`.rr` texts) over the corpus T's `t_facts` names under `root` (list step 23): the outputs as T's
/// `rdl.py sql` writes them, into `out`, and a report of each phase's CPU and the peak RSS. Several programs run as
/// one (decision 0051): each file is parsed and batched once for all of them, and `violation` holds every one's
/// findings. T's facts reproduce one program's run only.
/// `settings` are `--env key=value`s ([`settings`]).
#[allow(clippy::too_many_arguments)]
/// The runs `logic` makes of `programs`: those whose language has one dialect, or one with files in scope, run as
/// written (the first run); a program of a language of several dialects with files (`run_dialects`) runs in each,
/// its language set to the dialect by the run (the `dialect` option, `corpus::added`), its text as written. The
/// corpus is listed once for all of them.
fn dialect_runs(programs: &[&str], root: &std::path::Path, options: &serde_json::Value) -> Result<Vec<DialectRun>, String> {
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
            let prog = super::logic::rr::parse_own(text).map_err(|e| e.to_string())?;
            corpus::run_dialects_in(&files, &prog, options).map_err(|e| e.to_string())?
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

#[allow(clippy::too_many_arguments, reason = "the `logic` command's options, one argument each")]
pub fn logic(programs: &[String], root: &std::path::Path, t_facts: Option<&std::path::Path>, out: &std::path::Path, modes: &str, metal: bool, chunk_nodes: usize, settings: &[String]) -> Result<String, String> {
    use super::typing::file_local::Binding;
    let binding = if metal { Binding::Metal } else { Binding::Cpu };
    if programs.len() > 1 && t_facts.is_some() {
        return Err("T facts reproduce one program's run, not several".into());
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
        let each = super::logic::plan::corpus::run_with_options(&texts, root, t_facts, &dir, modes, binding, chunk_nodes, &super::logic::plan::corpus::Typing::default(), &options)
            .map_err(|error| error.to_string())?;
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
    let report = report.ok_or("no programs")?;
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
    for (name, _) in &report.typing_runs {
        text += &format!("  typing program {name} run first, for the relations it gives\n");
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
//   rerail-engine logic <program.rr | dir of .rr> <corpus root> [<T facts.sqlite>] <out dir> [--modes <modes>] [--metal] [--chunk <nodes>] [--full]
//   rerail-engine gc <store.sqlite> [--history-mb <n>]
//   rerail-engine sql <store.sqlite> <request.json>
//   rerail-engine flatten <program.rr>
//   rerail-engine run-rr <policy.rr> <corpus root> <out dir> [<options.json>]
//   rerail-engine run-policies <corpus root> <out dir> <policy.rr>...
//   rerail-engine check-rr <rule>... | convert-rr <rule>...
//   rerail-engine test-rr <rule or directory>... [--filter <text>]
use std::path::Path;

const USAGE: &str = "usage:\n  rerail-engine repeat <script>\n  rerail-engine dump --rule <file>... [--clause <id>] [--fused] [--setplan] [--capabilities <file>]\n  rerail-engine asm <file.rir>\n  rerail-engine logic <program.rr | dir of .rr> <corpus root> [<T facts.sqlite>] <out dir> [--modes <modes> | --sql] [--metal] [--chunk <nodes>] [--env key=value]... [--full] [--facts <dir>] [--require-facts] [--base <rev>]\n  rerail-engine facts-stamp <raw dir> --out <facts dir> --root <repository> <policy.rr>...\n  rerail-engine images pack <exported dir> <bundle>\n  rerail-engine gc <store.sqlite> [--history-mb <n>]\n  rerail-engine capture-tapes <root> <out file> [--every <k>] [--max <files>] [--all-tokens]\n  rerail-engine upkeep [<store.sqlite>...] [--cpu <percent>] [--jobs <list>]\n  rerail-engine sql <store.sqlite> <request.json>\n  rerail-engine flatten <program.rr>\n  rerail-engine csv <out dir> [<to dir>]\n  rerail-engine run-rr <policy.rr> <corpus root> <out dir> [<options.json>]\n  rerail-engine run-policies <corpus root> <out dir> <policy.rr>...\n  rerail-engine check-rr <rule>...\n  rerail-engine convert-rr [--cases <cases.json>] <rule>...\n  rerail-engine test-rr <rule or directory>... [--filter <text>] [--json]\n  rerail-engine translate-json [--display] <rule.json>... --out <dir>\nevery command: [--memory-mb <n>] (overall memory budget, default 1024) [--metal-mb <n>] (Metal budget on top of it, default half the overall)";

/// One `rerail-engine` command (`args` after the program's name): its output, or the error to report. The binary
/// runs one per process; a long-lived instance (the browser's engine: `reactor`) runs one per call, and with
/// `RERAIL_SESSION` (a directory) keeps a corpus session across them (decision 0063): `cli_in`, with the session
/// it keeps.
pub fn cli(args: &[String]) -> Result<String, String> {
    // Decision 0272 (O167): the process ends with its command, before any idle time, so each run serves its waiting
    // join searches at its end, within a share of its CPU (decision 0243). A long-lived caller of `cli_in` leaves
    // them to the idle job.
    crate::code_rule_dsl::logic::plan::joins::drain_at_end(true);
    // Decision 0288: the command's clocks, for its time before its run and after it.
    crate::code_rule_dsl::logic::profile::command_begins();
    let out = cli_in(args, &mut Default::default());
    let (age, after) = crate::code_rule_dsl::logic::profile::command_ends();
    // On stderr: a command's output is its own (`run-rr`'s, `sql`'s: JSON), and a line after it made that unreadable.
    if let (Ok(_), Some(c)) = (&out, after) {
        eprint!(
            "  after the run {:.2} s wall: the command's thread {:.2} s own, {:.2} s off a core; the other threads {:.2} s; ready with no core {:.2} s\n  before the command: {age:.2} s since the process started\n",
            c.wall, c.own + c.helped, c.off_core(), c.others(), c.no_core()
        );
    }
    out
}

/// `cli`, in the session `kept` keeps across its caller's commands (`RERAIL_SESSION`): the caller owns it.
pub fn cli_in(args: &[String], kept: &mut crate::code_rule_dsl::logic::plan::corpus::session::Kept) -> Result<String, String> {
    // Done as the library loaded, except where nothing runs then (the browser's build).
    crate::install_store_functions();
    let budgets = budget_options(args)?;
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
        Some("asm") => match &args[1..] {
            [file] => crate::tools::assemble(&read(file)?),
            _ => Err(USAGE.into()),
        },
        Some("sql") => match &args[1..] {
            [store, request] => crate::tools::sql(Path::new(store), &read(request)?),
            _ => Err("usage: rerail-engine sql <store.sqlite> <request.json>".into()),
        },
        Some("logic") => logic_command(&args[1..]),
        Some("repeat") => match &args[1..] {
            [script] => repeat_command(&read(script)?),
            _ => Err("usage: rerail-engine repeat <script>".into()),
        },
        Some("facts-stamp") => facts_stamp_command(&args[1..]),
        Some("capture-tapes") => capture_tapes_command(&args[1..]),
        Some("kind-names") => kind_names_command(&args[1..]),
        Some("run-rr") if args.len() == 4 || args.len() == 5 => {
            let options = if args.len() == 5 { serde_json::from_str(&read(&args[4])?).map_err(|e| e.to_string())? } else { serde_json::Value::Null };
            serde_json::to_string(&crate::tools::run_rr(Path::new(&args[1]), Path::new(&args[2]), Path::new(&args[3]), &options)?).map_err(|e| e.to_string())
        }
        // Several policies over one repository, each over its own language's files (O105).
        Some("run-policies") if args.len() > 3 => {
            let paths = args[3..].iter().map(std::path::PathBuf::from).collect::<Vec<_>>();
            serde_json::to_string(&crate::tools::run_policies(&paths, Path::new(&args[1]), Path::new(&args[2]), &serde_json::Value::Null)?).map_err(|e| e.to_string())
        }
        Some("check-rr") if args.len() > 1 => {
            let paths = args[1..].iter().map(std::path::PathBuf::from).collect::<Vec<_>>();
            let report = crate::tools::check_rr(&paths);
            let failures = report.as_array().expect("rule reports").iter().filter(|row| row["status"] == "reader-failed").count();
            let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
            if failures > 0 {
                println!("{text}");
                Err(format!("{failures} rules failed the .rr reader"))
            } else { Ok(text) }
        }
        // Main's JSON rules as the engine reads them: each `<id>.json` to `<out>/<id>.rr` (decision 0175).
        Some("translate-json") => translate_json_command(&args[1..]),
        // Each test the files carry (decision 0130).
        Some("test-rr") => crate::code_rule_dsl::logic::rr::testing::command(&args[1..]),
        // `--cases <file>` gives the next rule its cases (decision 0130).
        Some("convert-rr") if args.len() > 1 => {
            let (mut rules, mut cases) = (Vec::new(), None);
            let mut rest = args[1..].iter();
            while let Some(a) = rest.next() {
                if a == "--cases" {
                    cases = Some(std::path::PathBuf::from(rest.next().ok_or("--cases <cases.json> <rule>")?));
                } else {
                    rules.push((std::path::PathBuf::from(a), cases.take()));
                }
            }
            serde_json::to_string_pretty(&crate::tools::convert_rr_with(&rules)).map_err(|e| e.to_string())
        }
        // The program `logic` runs for a file: `core.rr` and its includes resolved, lowered and printed as `.rr`.
        Some("flatten") => match &args[1..] {
            [file] => crate::tools::flatten(Path::new(file.as_str())),
            _ => Err(USAGE.into()),
        },
        // The rule images shipped with the engine (decision 0264): packed from those a run exported
        // (`RERAIL_EXPORT_IMAGES=<dir>`), into the bundle placed beside the engine (`rerail-library.images`).
        Some("images") => match &args[1..] {
            [pack, dir, out] if pack == "pack" => crate::code_rule_dsl::logic::rr::images::pack(Path::new(dir.as_str()), Path::new(out.as_str()))
                .map(|n| format!("{n} images packed into {out}"))
                .map_err(|e| e.to_string()),
            _ => Err("usage: rerail-engine images pack <exported dir> <bundle>".into()),
        },
        // A run's outputs (`out/outputs.rro`, decision 0239) as `.csv` files, one per relation, in `to` (`out` itself
        // when not given): for scripts that read them.
        Some("csv") => match &args[1..] {
            [out] | [out, _] => {
                let to = Path::new(args.get(2).unwrap_or(out).as_str());
                let n = crate::code_rule_dsl::logic::plan::output::to_csv(Path::new(out.as_str()), to).map_err(|e| e.to_string())?;
                Ok(format!("{n} relations written to {}\n", to.display()))
            }
            _ => Err(USAGE.into()),
        },
        #[cfg(feature = "tape-cache")]
        Some("gc") => gc_command(&args[1..]),
        #[cfg(feature = "tape-cache")]
        Some("upkeep") => upkeep_command(&args[1..]),
        #[cfg(not(all(feature = "dump-cli", feature = "tape-cache")))]
        Some(command @ ("dump" | "gc" | "upkeep")) => Err(format!("{command} is not in this build (its feature, `dump-cli` or `tape-cache`, is off)")),
        _ => Err(USAGE.into()),
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
    #[cfg(unix)]
    let cpu = || {
        let mut usage = unsafe { std::mem::zeroed::<libc::rusage>() };
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
        let time = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
        (time(usage.ru_utime), time(usage.ru_stime))
    };
    // Windows: the process's user and kernel CPU (`GetProcessTimes`).
    #[cfg(windows)]
    let cpu = || {
        let (user, kernel) = rerail_prof::windows_cpu(false);
        (user.as_secs_f64(), kernel.as_secs_f64())
    };
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
                let (u0, s0) = cpu();
                let result = cli_in(&args, &mut kept);
                let (u1, s1) = cpu();
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
/// `--metal-mb <n>` (the Metal budget, on top of it: half the overall budget by default) (decision 0126): the
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
/// written to `<dir>/<stem>.rr`. With `--display`, the display translation instead (the rules docs' generated view,
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
                let dest = out.join(format!("{stem}.rr"));
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
    let mut args = args.iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
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
    // A directory: every `.rr` in it, run as one program (each file parsed once for all of them); each file's
    // `#include`s are read in.
    let reading = crate::code_rule_dsl::logic::profile::span(|| "read the rule files".into());
    let programs = if Path::new(program.as_str()).is_dir() {
        let mut files: Vec<_> = std::fs::read_dir(program)
            .map_err(|e| format!("{program}: {e}"))?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            // and every JSON policy (`rerail.policy/v1`), translated as it is read (decision 0175)
            .filter(|path| path.extension().is_some_and(|x| x == "rr") || crate::code_rule_dsl::logic::json_rules::is_policy_path(path))
            .collect();
        files.sort();
        // A file another one `#include`s is a library of it (`…confusables.rr`), not a program of its own: alone it
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
        timeline_mark("the program texts read");
        texts
    } else {
        vec![crate::tools::program_text(Path::new(program.as_str()))?]
    };
    drop(reading);
    if programs.is_empty() {
        return Err(format!("{program} has no .rr programs"));
    }
    let run = || crate::tools::logic(&programs, Path::new(root), facts, Path::new(out), &modes, metal, chunk, &env);
    if full { crate::code_rule_dsl::logic::plan::corpus::session::full(run) } else { run() }
}

/// `facts-stamp <raw> --out <dir> --root <repository> <policy.rr>…` (data-layer/RUN-FACTS.md section 1, `rerail facts
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
        jobs.push(register(Box::new(crate::content_store::upkeep::Frames::new(Path::new(path)))));
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
    let mut text = String::new();
    let mut tags = 0;
    for &lang in GrammarDialect::all() {
        let Some(tag) = tape_tag(lang) else { continue };
        if from_tape_tag(tag) != Some(lang) {
            continue;
        }
        tags += 1;
        for id in 0..lang.kind_count() {
            let id = id as u16;
            if let Some(name) = lang.kind_name(id) {
                text.push_str(&format!("{tag}\t{id}\t{name}\t{}\n", u8::from(lang.kinds().kind_is_named(id))));
            }
        }
        for id in 1..=lang.kinds().field_count() {
            if let Some(name) = lang.kinds().field_name(id as u16) {
                text.push_str(&format!("{tag}\tfield {id}\t{name}\t\n"));
            }
        }
    }
    std::fs::write(out, text).map_err(|e| e.to_string())?;
    Ok(format!("{tags} languages to {out}"))
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
                if !matches!(name.as_ref(), ".git" | "node_modules" | ".venv" | "venv" | "target" | "dist" | "build" | "__pycache__" | "tmp" | "working-data") {
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
                    let rel = path.strip_prefix(root).unwrap_or(path).to_string_lossy().into_owned();
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
        let mut declares = crate::code_rule_dsl::logic::rr::parse(".decl env(key: symbol, value: symbol)\n").unwrap();
        added.add_to(&mut declares);
        let rows: Vec<Vec<Value>> = declares.facts["env"].iter().map(|r| r.to_vec()).collect();
        assert_eq!(rows, [vec![Value::Str("quote".into()), Value::Str("a\"b".into())], vec![Value::Str("python.version".into()), Value::Str("3.13".into())]]);
        let mut not = crate::code_rule_dsl::logic::rr::parse(".decl envy(x: symbol)\n").unwrap();
        added.add_to(&mut not);
        assert!(!not.facts.contains_key("env"));
    }

    #[test]
    fn the_command_lines_runs_keep_their_outputs_with_or_without_options() {
        use serde_json::json;
        // `run-policies` passes no options (`null`): its runs ask for the cutoff too.
        assert_eq!(super::with_reuse(&serde_json::Value::Null), json!({"reuse": true}));
        assert_eq!(super::with_reuse(&json!({"x": 1})), json!({"x": 1, "reuse": true}));
        assert_eq!(super::with_reuse(&json!({"reuse": false})), json!({"reuse": false}));
    }
}
