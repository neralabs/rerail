//! The data-layer bench: a data-layer policy on the final shape's path (slice → wave → findings
//! layer) over a directory of the data-layer repository, with what of it lowers, its CPU, wall
//! time and memory, and (`RERAIL_DL_W=1`) its keys against W's engine on the same files.
//!
//!   RERAIL_DL_POLICY=<policy.json>[:<policy.json>…] RERAIL_DL_ROOT=<dir>
//!   [RERAIL_DL_LOGIC=<program.rr>] [RERAIL_DL_BACKEND=cpu|metal] [RERAIL_DL_W=1] [RERAIL_PROF=1]
//!   cargo test --profile profiling --lib -- slice::tests::data_layer::data_layer_bench --ignored --nocapture
//!
//! A clause the path cannot run is reported with its reason, and left out of the policy the slice
//! lowers: the round trip (`logic::front::yaml`) refuses a whole policy for one such clause.
//! `RERAIL_DL_LOGIC` is a program in Rerail logic beside the policies (`policies::lower_with`): its
//! own `.policy` of anchor clauses, and rules giving their violations.

use super::*;
use crate::code_rule_dsl::matcher::gated_match_tree;
use crate::code_rule_dsl::transducer::packed::LoweredMatcher;

/// Peak resident set of the process, in MB (`ru_maxrss` is bytes on macOS, KB elsewhere).
fn peak_rss_mb() -> u64 {
    let mut usage = unsafe { std::mem::zeroed::<libc::rusage>() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    let raw = usage.ru_maxrss as u64;
    if cfg!(target_os = "macos") { raw >> 20 } else { raw >> 10 }
}

/// Why a clause does not run on the slice's path, or `None` when it lowers.
fn refusal(clause: &serde_json::Value) -> Option<String> {
    let Some(forbid) = clause.get("forbid") else {
        let kind = ["relational", "define", "budget", "require_each", "llm"].into_iter().find(|k| clause.get(*k).is_some());
        return match kind {
            Some("define") => None,
            Some(kind) => Some(format!("{kind} clause")),
            None => Some("unknown clause kind".to_owned()),
        };
    };
    if forbid.get("ast").is_none() {
        return Some(format!("forbid {:?}", forbid.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()).unwrap_or_default()));
    }
    None
}

/// The authored program of `RERAIL_DL_LOGIC`, empty without one; each `define`'s options at
/// `options` where given.
fn authored(options: &serde_json::Map<String, serde_json::Value>) -> crate::code_rule_dsl::logic::front::Program {
    let Ok(path) = std::env::var("RERAIL_DL_LOGIC") else { return Default::default() };
    let mut program = crate::code_rule_dsl::logic::rr::parse(&std::fs::read_to_string(&path).unwrap()).unwrap_or_else(|e| panic!("{path}: {e}"));
    for document in &mut program.policies {
        with_options(document, options);
    }
    program
}

/// `document` with each `define`'s option defaults set to `options`' values.
fn with_options(document: &mut serde_json::Value, options: &serde_json::Map<String, serde_json::Value>) {
    for clause in document["clauses"].as_array_mut().into_iter().flatten() {
        if let Some(defined) = clause.pointer_mut("/define/options").and_then(|o| o.as_object_mut()) {
            for (name, value) in options {
                if let Some(option) = defined.get_mut(name) {
                    option["default"] = value.clone();
                }
            }
        }
    }
}

/// Each policy split into what the slice runs (compiled) and the clauses it refused, with reasons.
fn split(path: &Path) -> Result<((String, CompiledCodeRuleDsl), Vec<(String, String)>), String> {
    let mut document: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut refused = Vec::new();
    document["clauses"].as_array_mut().unwrap().retain(|clause| match refusal(clause) {
        Some(reason) => {
            refused.push((clause["id"].as_str().unwrap_or("?").to_owned(), reason));
            false
        }
        None => {
            // `RERAIL_DL_ONLY` / `RERAIL_DL_SKIP`: comma-separated clause ids (a `define` is always kept).
            let id = clause["id"].as_str().unwrap_or_default();
            let listed = |name: &str| std::env::var(name).ok().map(|ids| ids.split(',').any(|x| x == id));
            clause.get("define").is_some() || (listed("RERAIL_DL_ONLY") != Some(false) && listed("RERAIL_DL_SKIP") != Some(true))
        }
    });
    let compiled = CodeRuleDslRuntime::compile(document.to_string(), "json".to_owned(), CAPABILITIES.to_owned()).map_err(|e| e.reason)?;
    // Matchers that do not lower are left out of the wave by `policies::lower`; name them here.
    for clause in &compiled.inner.clauses {
        let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &clause.kind else { continue };
        for grammar in matchers {
            let why = match gated_match_tree(grammar).map(|(tree, _)| tree) {
                Err(error) => Some(format!("match_tree: {error:?}")),
                Ok(tree) => LoweredMatcher::lower(&tree).err().map(|error| format!("lower: {error:?}")),
            };
            if let Some(why) = why {
                refused.push((clause.id.clone(), why.chars().take(200).collect()));
            }
        }
    }
    Ok(((path.file_stem().unwrap().to_string_lossy().into_owned(), compiled), refused))
}

#[test]
#[ignore = "the data-layer bench; RERAIL_DL_POLICY and RERAIL_DL_ROOT, --ignored --nocapture"]
fn data_layer_bench() {
    let root = PathBuf::from(std::env::var("RERAIL_DL_ROOT").expect("RERAIL_DL_ROOT"));
    let mut policies = Vec::new();
    for path in std::env::var("RERAIL_DL_POLICY").expect("RERAIL_DL_POLICY").split(':') {
        // A policy left with nothing to run (every clause refused) is named and skipped.
        let (policy, refused) = match split(Path::new(path)) {
            Ok(split) => split,
            Err(why) => {
                eprintln!("DL {path}: skipped: {why}");
                continue;
            }
        };
        let total = policy.1.inner.clauses.len() + refused.iter().filter(|(_, why)| !why.starts_with("match_tree") && !why.starts_with("lower")).count();
        eprintln!("DL {}: {total} clauses, {} not run on the slice's path", policy.0, refused.len());
        for (clause, why) in &refused {
            eprintln!("DL   refused {clause}: {why}");
        }
        policies.push(policy);
    }
    let backend = match std::env::var("RERAIL_DL_BACKEND").as_deref() {
        #[cfg(target_os = "macos")]
        Ok("metal") => ExecutionBackend::MetalBackground,
        _ => ExecutionBackend::CpuBackground,
    };
    let mut sources = Vec::new();
    files(&root, "py", &mut sources);
    sources.sort();
    let bytes = sources.iter().map(|p| std::fs::metadata(p).map_or(0, |m| m.len())).sum::<u64>();

    let footprint = crate::code_rule_dsl::prof::footprint_mb;
    let (cpu0, wall0) = (cpu(), std::time::Instant::now());
    let program = authored(&Default::default());
    // A policy the wave does not lower on its own is named and left out.
    policies.retain(|policy| match Rules::lower_with(std::slice::from_ref(policy), &program, backend) {
        Ok(_) => true,
        Err(error) => {
            eprintln!("DL {}: skipped: does not lower: {error:?}", policy.0);
            false
        }
    });
    let mut rules = Rules::lower_with(&policies, &program, backend).unwrap();
    let lowering = (cpu() - cpu0, wall0.elapsed());
    eprintln!("DL lowered {} clauses in cpu {:?}, wall {:?}; footprint {} MB", rules.clauses(), lowering.0, lowering.1, footprint());

    let store = ContentStore::open(&temp("dl-store").with_extension("sqlite")).unwrap();
    let (cpu0, wall0) = (cpu(), std::time::Instant::now());
    let tree = directory_root(&store, &root).unwrap();
    let run = audit(&store, WS, tree, &mut rules).unwrap();
    let (spent, wall) = (cpu() - cpu0, wall0.elapsed());
    let ours = keys(&run);
    let mut by_code = BTreeMap::<(String, String), usize>::new();
    for key in &ours {
        *by_code.entry((key.0.clone(), key.1.clone())).or_default() += 1;
    }
    let unknown = coverage(&run).values().filter(|c| matches!(c, Coverage::Unknown(_))).count();
    eprintln!(
        "DL cold audit: {} files ({} .py, {} KB), {} keys, {unknown} (rule, file)s Unknown; cpu {spent:?}, wall {wall:?}; footprint {} MB, peak RSS {} MB",
        run.paths.len(),
        sources.len(),
        bytes >> 10,
        ours.len(),
        footprint(),
        peak_rss_mb()
    );
    for ((rule, code), n) in &by_code {
        eprintln!("DL   {rule} {code}: {n}");
    }
    // `RERAIL_DL_KEYS=<dir>`: every key, as `rule code path line` lines (`slice.tsv`, and `w.tsv` with RERAIL_DL_W).
    let write_keys = |name: &str, keys: &BTreeSet<(String, String, String, usize)>| {
        if let Some(dir) = std::env::var_os("RERAIL_DL_KEYS") {
            let text: String = keys.iter().map(|(rule, code, path, line)| format!("{rule}\t{code}\t{path}\t{line}\n")).collect();
            std::fs::write(PathBuf::from(dir).join(name), text).unwrap();
        }
    };
    write_keys("slice.tsv", &ours);
    crate::code_rule_dsl::prof::report("dl cold");

    let (cpu0, wall0) = (cpu(), std::time::Instant::now());
    let tree = directory_root(&store, &root).unwrap();
    let warm = apply(&store, WS, tree, &mut rules).unwrap();
    eprintln!("DL warm, nothing changed: {} keys, {} evaluated; cpu {:?}, wall {:?}", warm.findings.keys.len(), warm.evaluated.len(), cpu() - cpu0, wall0.elapsed());

    if std::env::var_os("RERAIL_DL_W").is_some() {
        let (clauses, _) = lowered(&policies).unwrap();
        let names = clauses.iter().map(|c| c.name.clone()).collect::<BTreeSet<_>>();
        let texts = sources
            .iter()
            .filter_map(|p| Some((p.strip_prefix(&root).unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(p).ok()?)))
            .collect::<Vec<_>>();
        let (cpu0, wall0) = (cpu(), std::time::Instant::now());
        let (ws, failed) = ws_keys(&policies, &names, &texts);
        eprintln!("DL W: {} keys, {} failures; cpu {:?}, wall {:?}", ws.len(), failed.len(), cpu() - cpu0, wall0.elapsed());
        write_keys("w.tsv", &ws);
        let difference = Difference::of(&run, &ws, &failed);
        for ((rule, code), (only_ours, only_ws)) in &difference.clauses {
            eprintln!("DL   differs {rule} {code}: {only_ours} only ours, {only_ws} only W");
        }
        for key in ours.symmetric_difference(&ws).take(30) {
            eprintln!("DL     {} {key:?}", if ours.contains(key) { "only ours" } else { "only W" });
        }
        for line in difference.unexplained.iter().take(10) {
            eprintln!("DL   {line}");
        }
    }
}

/// The data-layer fixtures on the final shape's path: each recorded pytest event of one check
/// (`RERAIL_DL_RECORDING`, `RERAIL_DL_CHECK`, e.g. `gate/data-layer/fixtures/host/data-layer-port/
/// batch-06-recording.json` and `serverplan_quality`) is a root of its files, audited cold with the
/// policy compiled at the event's options; its keys (code, path, line) are compared with the
/// recorded findings. Prints per entry point how many events agree.
#[test]
#[ignore = "the data-layer fixtures; RERAIL_DL_POLICY, RERAIL_DL_RECORDING, RERAIL_DL_CHECK, --ignored --nocapture"]
fn data_layer_fixtures() {
    let policy = PathBuf::from(std::env::var("RERAIL_DL_POLICY").expect("RERAIL_DL_POLICY"));
    let recording: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(std::env::var("RERAIL_DL_RECORDING").expect("RERAIL_DL_RECORDING")).unwrap()).unwrap();
    let check = std::env::var("RERAIL_DL_CHECK").expect("RERAIL_DL_CHECK");
    let only = std::env::var("RERAIL_DL_ENTRY").ok();
    let events = recording["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["check"] == check.as_str())
        .filter(|e| only.as_ref().is_none_or(|only| e["oracle_config"]["options"]["entryPoint"].as_str() == Some(only)))
        .collect::<Vec<_>>();
    let document: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&policy).unwrap()).unwrap();
    let mut by_options = BTreeMap::<String, Vec<&serde_json::Value>>::new();
    for event in &events {
        by_options.entry(event["oracle_config"]["options"].to_string()).or_default().push(event);
    }
    let backend = ExecutionBackend::CpuBackground;
    let (mut agree, mut total, mut spent) = (0, 0, std::time::Duration::ZERO);
    let verbose = std::env::var_os("RERAIL_DL_VERBOSE").is_some();
    for (options, events) in &by_options {
        let options: serde_json::Map<String, serde_json::Value> = serde_json::from_str(options).unwrap();
        let mut document = document.clone();
        with_options(&mut document, &options);
        let file = temp("dl-policy").with_extension("json");
        std::fs::write(&file, document.to_string()).unwrap();
        let (compiled, refused) = split(&file).unwrap_or_else(|why| panic!("{}: {why}", file.display()));
        let entry = options.get("entryPoint").and_then(|e| e.as_str()).unwrap_or("?").to_owned();
        let policies = vec![(policy.file_stem().unwrap().to_string_lossy().into_owned(), compiled.1)];
        let program = authored(&options);
        let mut rules = Rules::lower_with(&policies, &program, backend).unwrap();
        // W runs every clause of the policy (relational ones included, as test builds compile them).
        let full = CodeRuleDslRuntime::compile(document.to_string(), "json".to_owned(), CAPABILITIES.to_owned()).unwrap_or_else(|e| panic!("{}", e.reason));
        let mut good = 0;
        let mut w_agree = 0;
        for event in events {
            let dir = temp("dl-event");
            std::fs::create_dir_all(&dir).unwrap();
            for f in event["files"].as_array().unwrap() {
                let path = dir.join(f["path"].as_str().unwrap());
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, f["source"].as_str().unwrap()).unwrap();
            }
            let expected = event["findings"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| (f["code"].as_str().unwrap().to_owned(), f["path"].as_str().unwrap().to_owned(), f["line"].as_u64().unwrap() as usize))
                .collect::<BTreeSet<_>>();
            let store = ContentStore::open(&temp("dl-event-store").with_extension("sqlite")).unwrap();
            let started = cpu();
            let tree = directory_root(&store, &dir).unwrap();
            let run = audit(&store, WS, tree, &mut rules);
            spent += cpu() - started;
            let found = match &run {
                Ok(run) => keys(run).into_iter().map(|(_, code, path, line)| (code, path, line + 1)).collect::<BTreeSet<_>>(),
                Err(error) => {
                    eprintln!("DLF {entry} error: {error:?}");
                    BTreeSet::new()
                }
            };
            total += 1;
            if std::env::var_os("RERAIL_DL_W").is_some() {
                let names = full.inner.clauses.iter().map(|c| format!("{}/{}", policies[0].0, c.id)).collect::<BTreeSet<_>>();
                let texts = event["files"].as_array().unwrap().iter().map(|f| (f["path"].as_str().unwrap().to_owned(), f["source"].as_str().unwrap().to_owned())).collect::<Vec<_>>();
                let (ws, _) = ws_keys(&[(policies[0].0.clone(), CompiledCodeRuleDsl { inner: std::sync::Arc::clone(&full.inner) })], &names, &texts);
                let ws = ws.into_iter().map(|(_, code, path, line)| (code, path, line + 1)).collect::<BTreeSet<_>>();
                w_agree += usize::from(ws == expected);
                if ws != expected && verbose {
                    eprintln!("DLF W {entry} {}: only W {:?}, only recorded {:?}", event["test"].as_str().unwrap_or("?"), ws.difference(&expected).collect::<Vec<_>>(), expected.difference(&ws).collect::<Vec<_>>());
                }
            }
            if found == expected {
                good += 1;
            } else if verbose {
                eprintln!("DLF {entry} {}: only ours {:?}, only recorded {:?}", event["test"].as_str().unwrap_or("?"), found.difference(&expected).collect::<Vec<_>>(), expected.difference(&found).collect::<Vec<_>>());
            }
        }
        agree += good;
        eprintln!("DLF {entry}: {good}/{} events agree (W {w_agree}); {} clauses not run", events.len(), refused.len());
    }
    eprintln!("DLF total: {agree}/{total} events agree; audits cpu {spent:?}");
}
