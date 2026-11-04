//! The data-layer bench: a data-layer policy on the final shape's path (slice → wave → findings
//! layer) over a directory of the data-layer repository, with what of it lowers, its CPU, wall
//! time and memory, and (`RERAIL_DL_W=1`) its keys against W's engine on the same files.
//!
//!   RERAIL_DL_POLICY=<policy.json>[:<policy.json>…] RERAIL_DL_ROOT=<dir>
//!   [RERAIL_DL_BACKEND=cpu|metal] [RERAIL_DL_W=1] [RERAIL_PROF=1]
//!   cargo test --profile profiling --lib -- slice::tests::data_layer::data_layer_bench --ignored --nocapture
//!
//! A clause the path cannot run is reported with its reason, and left out of the policy the slice
//! lowers: the round trip (`logic::front::yaml`) refuses a whole policy for one such clause.

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

/// Each policy split into what the slice runs (compiled) and the clauses it refused, with reasons.
fn split(path: &Path) -> ((String, CompiledCodeRuleDsl), Vec<(String, String)>) {
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
    let compiled = CodeRuleDslRuntime::compile(document.to_string(), "json".to_owned(), CAPABILITIES.to_owned()).unwrap_or_else(|e| panic!("{}", e.reason));
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
    ((path.file_stem().unwrap().to_string_lossy().into_owned(), compiled), refused)
}

#[test]
#[ignore = "the data-layer bench; RERAIL_DL_POLICY and RERAIL_DL_ROOT, --ignored --nocapture"]
fn data_layer_bench() {
    let root = PathBuf::from(std::env::var("RERAIL_DL_ROOT").expect("RERAIL_DL_ROOT"));
    let mut policies = Vec::new();
    for path in std::env::var("RERAIL_DL_POLICY").expect("RERAIL_DL_POLICY").split(':') {
        let (policy, refused) = split(Path::new(path));
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
    let mut rules = Rules::lower(&policies, backend).unwrap();
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
