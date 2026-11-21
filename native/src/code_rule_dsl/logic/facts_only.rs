//! Everything kept is facts (decision 0104): the checks that keep a cache from being built beside them.
//!
//! A cache is a rule that makes a key fact from what decides validity, and a rule that makes the result from the
//! key by a native step. The engine keeps both like any fact, so an unchanged key is no delta and the step does not
//! run again. Nothing else may keep a result for a later run, a later chunk or a later process.
//!
//! These tests read the engine's source. Each store table and each process-wide or thread-local map is listed under
//! what it is:
//!
//! - **fact store**: where facts and documents live (atoms, tapes, sources, the writer's queue);
//! - **scratch**: a table a query makes and drops inside one statement or one test lab;
//! - **state**: what is not kept for later use (registries of open things, queues, profiling, switches);
//! - **not yet facts**: a cache still to move. Each names the step of 0104 that moves it.
//!
//! A new table or map in none of the lists fails. So does a listed one that is gone: the list is cut, never left
//! stale. The not-yet-facts lists may only shrink: their sizes are pinned, and a move lowers the pin.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Tables where facts and documents live: the store's atoms, sources and tapes, the writer's queue, repository
/// objects, and a run's relations held in SQLite.
const FACT_STORE: &[&str] = &[
    "atoms",
    "atom_blocks",
    "strings",
    "hashes",
    "paths",
    "source_objects",
    "source_aliases",
    "tape_objects",
    "file_documents",
    "object_pins",
    "store_state",
    // The store's identity (RER-131): its atoms and connections belong to this store only.
    "store_identity",
    // The segment files objects (sources, tapes) are in, by number (decision 0295), and each guest's key, an identity
    // interned to its number as `strings` and `hashes` are (decision 0391; frames retired, 0439).
    "object_segments",
    "guest_keys",
    "push_pending",
    "rows",
    "git_alias",
    "git_trees",
    "git_tree_entries",
    "snapshots",
    "snapshot_files",
    "workspaces",
    // A store session per durable owner (decision 0450 §2): which data the store keeps.
    "store_sessions",
    // Page sets put and not yet bound (decision 0450): which a collection keeps.
    "tree_pages_pending",
    // The archive of aged sources and tapes (decision 0450 step 4): their bytes, compressed.
    "object_archive",
    "workspace_overlay",
    "node",
    "tape",
    "node_type",
    // A space's kept facts (decisions 0401, 0456): which files its chain has.
    "facts_chain",
    // The per-file memo's entries (decision 0240 §4): where each one's bytes are in the pack beside the store, and the
    // packs. They replace the memo's frames (`local`, `local_out`): the same entries, no new cache.
    "memo_entries",
    "memo_packs",
    // The observer's own journal (decision 0433): the paths its listings named changed, by generation. What it
    // observed, as `samples` are measurements.
    "observed_changes",
    // The executor's measured jobs (decision 0100 §14), the cost model's data: kept as the measurements they are.
    "samples",
    // The SQL executor's planning observations (`costs::sql_plans`), its plan model's data: measurements, as `samples`.
    "sql_plan_observations",
    // The kernels' forms per machine (decision 0202): each compiled or interpreted, its compile time, what it missed;
    // measurements of the machine, as `samples`, ranked for the upkeep job that compiles them.
    "kernel_forms",
    // Each program's measured steps per unit of size, and its overruns (decision 0334): what a task's budgets are
    // predicted from (`evaluation_costs`); measurements, as `samples`.
    "program_steps",
    // The join steps' estimates and what they made (decision 0354, `costs::join_steps`): the learned correction's
    // data, measurements, as `samples`.
    "join_steps",
    "{p}_out in code_rule_dsl/logic/native.rs",
    "{p}_out in code_rule_dsl/logic/native/extension.rs",
    "{} in code_rule_dsl/logic/plan.rs",
    "{0} in code_rule_dsl/logic/plan.rs",
    "{r}__m in code_rule_dsl/logic/plan.rs",
    "delta_{r} in code_rule_dsl/logic/plan.rs",
    "lfx_{rel}_{kind} in code_rule_dsl/logic/plan.rs",
];
/// Tables made and dropped inside one statement, one lab or one older pipeline's query.
const SCRATCH: &[&str] = &[
    // The device parity tests' temporary inputs and heads (`setfix/device/parity.rs`, a `#[cfg(test)]` module the
    // file filter cannot leave out by name).
    "ia", "ib", "ic", "iz", "io", "ie", "dn", "sn", "sm", "sp", "h0",
    "x",
    "t",
    "input",
    "value",
    "events",
    "owners",
    "meta",
    "rdl_int",
    "rerail_capture",
    "rerail_terminal_match",
    "capture_tapes",
    "fact_batch",
    "fact_dv",
    "stat_meta",
    "stat_col",
    "stat_kind",
    "stat_edge",
    "stat_val",
    "stat_docrange",
    "{table} in code_rule_dsl/logic/native/setfix/lab.rs",
    "{} in code_rule_dsl/slice.rs",
    // A test's in-memory input (`plan/acyclic/queries.rs`).
    "facts",
];
/// Caches between runs not yet facts: (table, the 0104 step that moves it).
const NOT_YET_TABLES: &[(&str, &str)] = &[
    // The cost model fitted from `samples`, per machine (planning's placement).
    ("models", "6"),
    ("run_record", "1"),
    ("observed", "1"),
    ("observer_meta", "1"),
    ("derivations", "1"),
    ("derivation_inputs", "1"),
    ("name_bindings", "1"),
    ("facts_results", "3"),
    ("facts_plan_sets", "3"),
    ("facts_completions", "3"),
    ("corpus_plans", "3"),
    ("corpus_plan_relations", "3"),
    ("corpus_manifests", "3"),
    ("corpus_manifest_plans", "3"),
    ("corpus_roots", "3"),
    ("corpus_live", "3"),
    ("corpus_state", "3"),
    ("corpus_decided", "3"),
    ("corpus_swept", "3"),
    ("corpus_stats", "3"),
    ("slice_rules", "3"),
    ("slice_document", "3"),
    ("slice_match", "3"),
    ("slice_span", "3"),
    ("slice_fresh", "3"),
    ("compiled_plans", "6"),
    ("compiled_plan_reads", "6"),
    ("plan_feedback", "6"),
    ("accel_models", "6"),
    ("stat_blob", "6"),
    ("fact_stats", "6"),
    ("fact_value_stats", "6"),
    ("kind_size", "6"),
    ("edge_freq", "6"),
    ("tape_kind_counts", "6"),
    ("term_freq", "6"),
    ("term_kind_freq", "6"),
    ("term_node_freq", "6"),
    ("term_coverage", "6"),
    ("text_matcher", "7"),
    ("text_term", "7"),
    ("text_pending", "7"),
    ("prefilter_sets", "7"),
    ("term_postings", "7"),
    ("vocabulary_predicate", "7"),
    ("vocabulary_term", "7"),
    ("vocabulary_literal", "7"),
    ("vocabulary_pending", "7"),
    ("search_index_sets", "7"),
    ("tree_pages", "8"),
    ("tree_result_segments", "8"),
    ("completed_snapshot_trees", "8"),
    ("completed_workspace_revisions", "8"),
    ("completed_directory_reuse", "8"),
    ("candidate_catalogs", "8"),
    ("candidate_checks", "8"),
    ("context_check_results", "8"),
];
/// The pinned size of `NOT_YET_TABLES`: lowered by each move, never raised. Raised once, 61 to 62, by the merge of
/// the cost classifier's per-machine `models` (2026-11-10), which a move to facts has yet to take; back to 61 once a
/// merge took one out (2026-11-12); 58 when the cross-file memo's three tables went (decision 0450 §3, 2026-11-19); 57 when `local_check_results`, which nothing wrote, went (0450 step 2).
const NOT_YET_TABLES_PIN: usize = 57;

/// Maps (file, static) that keep nothing for later use: registries of open things, queues, switches, profiling, and
/// the engine's own facts in memory within a run.
const STATE: &[(&str, &str)] = &[
    // What a crate above registers at load, as the crate split made them (a function or a device, set once).
    ("code_rule_dsl/logic/device.rs", "SOLVER"),
    ("code_rule_dsl/logic/plan/corpus.rs", "CROSS_FILE"),
    ("code_rule_dsl/logic/strings.rs", "FUNCTORS"),
    ("code_rule_dsl/logic/strings/functions.rs", "PY_REPR"),
    ("code_rule_dsl/logic/strings/functions.rs", "TO_PRECISION"),
    // The rule being compiled: the step that first bound each slot, for a refusal's message.
    ("code_rule_dsl/logic/native/setfix/device.rs", "BINDER"),
    // What registers at load: the world's files with the answerers installed, and the record's kinds' applies.
    ("code_rule_dsl/logic/rr/lower.rs", "ALL"),
    ("content_store/record.rs", "KINDS"),
    // A test's log.
    // This thread's run's binding (CPU or Metal).
    ("code_rule_dsl/logic/plan/placement.rs", "Device"),
    // Profiling: the placements a recording run made, and the writer's jobs by kind.
    ("code_rule_dsl/logic/plan/placement.rs", "Recording"),
    // The coverage check's census of the shipped rule files, read once by a test process.
    ("code_rule_dsl/logic/plan/coverage.rs", "CENSUS"),
    // The process's interned relation names (`front::Name`): one allocation per distinct name, not a result kept.
    // The relations' numbers (decision 0209 step 1, `table::rel_id`): an interner as `TABLE`, one number per name.
    // Decision 0203: the batch predictor a crate above registers (`joins::set_pricer`), the wider searches queued for
    // the idle `join-search` upkeep job, and that job's one registration.
    // Decision 0203's feedback as measured: each line's notes, exact pair statistics, proposals and trials, written
    // back with the kept plans (`joins::to_store`) and read from them; measurements, as the cost model's `FITTED`.
    // The gate of the RERAIL_* switches (`corpus::switches`): its readers, writer and waiters, no value kept.
    ("code_rule_dsl/logic/plan/corpus/switches.rs", "STATE"),
    // The SQL executor's plan model, fitted from `sql_plan_observations`, and the observations queued for writing.
    // The join steps submitted at their lines' ends, queued for the cost statistics' writer (decision 0354).
    // The SQL lane's executor, and the work a thread's device wave runs when flushed (`lanes::wave`).
    ("code_rule_dsl/logic/lanes.rs", "DEFERRED"),
    // The device's waves on this thread (decision 0113 §3; the open one is the job's, `lanes::OPEN`): the waves
    // flushed, the strata waiting for theirs; and the rule being compiled's arena-text slots. State of the run, not a
    // result kept.
    ("code_rule_dsl/logic/native/setfix/device.rs", "FLUSHED"),
    ("code_rule_dsl/logic/native/setfix/device.rs", "SUSPENDED"),
    ("code_rule_dsl/logic/native/setfix/device.rs", "ARENA"),
    // The job's open device submission its strata share (`lanes::open`): an open thing, as `WAVE`.
    ("code_rule_dsl/logic/lanes.rs", "OPEN"),
    ("code_rule_dsl/logic/lanes.rs", "DONE"),
    ("code_rule_dsl/logic/lanes.rs", "QUEUED"),
    // Each live connection's path and symbols, dropped at its close (`forget_scope`): state of the connections.
    // Atoms reserved by components not yet retired, and this thread's reservation (decision 0114 §4): which
    // pending atoms no committed row may hold yet, not a result kept.
    ("content_store/atoms.rs", "RESERVING"),
    // The mixed index spike's per-thread counters (the shape grown wave by wave): a measurement, not a result kept.
    ("code_rule_dsl/logic/native/setfix.rs", "GROWN"),
    ("code_rule_dsl/logic/native/setfix.rs", "VARIANTS"),
    // A test process's own tape cache path (`corpus::test_tape_cache`): where the store is, not a result kept.
    ("code_rule_dsl/logic/plan/corpus.rs", "PATH"),
    ("code_rule_dsl/logic/native/setfix/device.rs", "LAST"),
    // core.rerail's declarations (RER-131), read once from the engine's own library text: not a result kept.
    ("code_rule_dsl/logic/rr/lower/core.rs", "DECLS"),
    // This thread's zstd decompression context (RER-131): an open thing, not a result.
    ("content_store/compression.rs", "DECODER"),
    // The store handles open in this process, by path (RER-131): a registry of open things.
    ("code_rule_dsl/logic/plan/choices.rs", "DurableSide"),
    ("code_rule_dsl/logic/plan/corpus.rs", "TEST_TAPE_CACHE"),
    // A test's own store, the template's path and the threads said to ask off a test's (`test_store`): which files
    // the tests' stores are, not results.
    // The caller's options for its runs' inputs (`--facts`, `--require-facts`, `--base`; decision 0442).
    ("code_rule_dsl/logic/plan/corpus/inputs.rs", "Options"),
    ("code_rule_dsl/logic/plan/corpus/test_store.rs", "Handed"),
    ("code_rule_dsl/logic/plan/corpus/test_store.rs", "OWN"),
    ("code_rule_dsl/logic/plan/corpus/test_store.rs", "PATH"),
    ("code_rule_dsl/logic/plan/corpus/test_store.rs", "SAID"),
    // The stores this process owns or was refused, with their held locks, and the slot it took of each base store
    // (`corpus::owner`): which files are this process's, not results.
    ("code_rule_dsl/logic/plan/corpus/owner.rs", "CLAIMS"),
    ("code_rule_dsl/logic/plan/corpus/owner.rs", "SLOTS"),
    // Profiling totals, consumed and cleared by each report; no reusable results.
    ("code_rule_dsl/logic/plan/corpus/phases.rs", "HandedDown"),
    // Phase 1's exports while the run that ran it is under way: the run's own facts, gone when it ends.
    ("code_rule_dsl/logic/plan/corpus/phases.rs", "HeldPhaseOne"),
    ("code_rule_dsl/logic/plan/corpus/phases.rs", "Phases"),
    ("code_rule_dsl/logic/profile.rs", "Dir"),
    // Ended threads' CPU accounts and their CPU outside spans, summed for the report; no reusable results.
    ("code_rule_dsl/logic/profile.rs", "EXITED"),
    ("code_rule_dsl/logic/profile.rs", "EXITED_GAPS"),
    // Each thread's shard of its run's profile records, merged into the run's as they are read: records, no
    // reusable results.
    ("code_rule_dsl/logic/profile.rs", "SHARD"),
    ("code_rule_dsl/logic/profile.rs", "STACK"),
    ("code_rule_dsl/logic/profile.rs", "THREADS"),
    // The accounts and gaps of threads that have ended, read by the report: profiling, as `THREADS`.
    ("code_rule_dsl/logic/profile.rs", "EXITED"),
    ("code_rule_dsl/logic/profile.rs", "EXITED_GAPS"),
    ("code_rule_dsl/logic/profile/memory.rs", "DOING"),
    ("code_rule_dsl/logic/profile/memory.rs", "SAMPLED"),
    // The world's rows (`rr::lower::World`), made once the languages are installed: a registry of what is
    // installed, as `FUNCTORS`.
    ("code_rule_dsl/logic/rr/lower.rs", "ALL"),
    ("code_rule_dsl/logic/rr.rs", "CORE"),
    ("code_rule_dsl/logic/rr.rs", "READS"),
    // `core_names` less 0174's on-request relations: a set of the engine's own, as `CORE`, the same for every run.
    ("code_rule_dsl/logic/rr.rs", "SHAPE"),
    // The library images shipped beside the executable (0264), read once: an input, as `CORE`'s core.rerail, no result.
    ("code_rule_dsl/logic/rr/images.rs", "SHIPPED"),
    // The rule files' entries a thread read before its run (0275), taken by the run's manifest: a queue.
    ("code_rule_dsl/logic/plan/corpus/combined.rs", "READ_BEFORE"),
    // rerail's own files the process names to its observers (0284): a registry.
    // Profiling (0288): the command's clocks, the process's start, the run thread's phases, each step's decisions.
    ("code_rule_dsl/logic/profile.rs", "COMMAND"),
    ("code_rule_dsl/logic/profile.rs", "DECIDING"),
    ("code_rule_dsl/logic/profile/life.rs", "ENTERED"),
    // Each database file's identity, read once a file: a registry, as CANONICAL is.
    ("content_store/compression.rs", "ENCODER"),
    ("content_store/connection.rs", "ATTEMPT_WAITED"),
    ("content_store/connection.rs", "BUSY_ATTEMPT_HOOK"),
    ("content_store/connection.rs", "BUSY_DEADLINE_OVERRIDE"),
    ("content_store/connection.rs", "BUSY_TIMEOUT_OVERRIDE"),
    ("content_store/connection.rs", "BUSY_WAIT_HOOK"),
    ("content_store/connection.rs", "CONNECTION_FUNCTIONS"),
    ("content_store/connection.rs", "OPERATION_WAITED"),
    ("content_store/connection.rs", "THREAD_CONNECTIONS"),
    ("content_store/connection.rs", "WAIT_SPENT"),
    // A switch: testing mode's store (decision 0130).
    ("code_rule_dsl/logic/plan/corpus.rs", "UseStore"),
    // Phase 1's runs within one batch of policies over one tree, dropped with it: nothing kept past the batch.
    ("code_rule_dsl/logic/plan/corpus/phases.rs", "BatchUnderWay"),
    // A registry of an open thing: the holders of the rule files' shared tape-cache handle, and the handle.
    ("code_rule_dsl/logic/plan/corpus/phases.rs", "ReadingUnderWay"),
    // The journal's open streams, one a root (decision 0105): open things.
    // A job's own plans and observations (decisions 0113, 0116), taken into the run's as the job ends: the run's own
    // state on the job's thread, gone with it.
    ("code_rule_dsl/logic/plan.rs", "OWN_PLANS"),
    ("code_rule_dsl/logic/plan.rs", "OWN_PLANNED"),
    ("code_rule_dsl/logic/plan/keeps.rs", "OWN_OBSERVED"),
    ("content_store/reclamation.rs", "EXECUTED_COLLECT_SQL"),
    ("content_store/schema.rs", "SCHEMA_OBJECTS"),
    ("content_store/snapshots.rs", "AFTER_GIT_TREE_TRANSACTION"),
    // The session this thread's runs are runs of (decision 0204): the caller's handle, set and taken back by it.
    ("code_rule_dsl/logic/plan/corpus/session.rs", "InSession"),
    // The record kinds' applies, registered at load.
    ("content_store/record.rs", "KINDS"),
];
/// Maps (file, static) that are caches not yet facts, with the 0104 step that moves each.
const NOT_YET_MAPS: &[(&str, &str, &str)] = &[
    ("code_rule_dsl/logic/plan/corpus.rs", "PREPARED", "2"),
    // The session a thread's runs use (decision 0204: the caller's `Handle`, bound for its runs; was `SESSION`, the
    // process's): what it keeps between runs is still a cache to move.
    ("code_rule_dsl/logic/plan/corpus/session.rs", "InSession", "3"),
    ("code_rule_dsl/logic/plan.rs", "KeptPlans", "6"),
    ("code_rule_dsl/logic/plan.rs", "SESSION_PLANS", "6"),
    ("code_rule_dsl/logic/derived.rs", "Memoized", "6"),
    ("code_rule_dsl/logic/derived.rs", "SESSION_MEMO", "6"),
    // The observed keeps, a handle a strand enters (decision 0113 §4).
    ("code_rule_dsl/logic/plan/keeps.rs", "ObservedAt", "6"),
    // RER-131's compiled group patterns, where `GROUPS` was.
    ("code_rule_dsl/logic/strings/functions.rs", "CACHE", "7"),
    ("code_rule_dsl/text.rs", "NAMES", "7"),
];
/// The pinned size of `NOT_YET_MAPS`: lowered by each move, never raised.
const NOT_YET_MAPS_PIN: usize = 9;

fn src() -> PathBuf {
    std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("../..").join("src")
}

/// The crate's source files, tests left out: a test may make what it likes.
fn files(under: &Path, out: &mut Vec<PathBuf>) {
    let Ok(dir) = std::fs::read_dir(under) else { return };
    for e in dir.flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if p.is_dir() {
            if name != "tests" {
                files(&p, out);
            }
        } else if name.ends_with(".rs") && name != "tests.rs" && !name.ends_with("_tests.rs") {
            out.push(p);
        }
    }
}

fn rel(p: &Path) -> String {
    p.strip_prefix(src()).unwrap().to_string_lossy().into_owned()
}

/// The source a run's kept state can live in: the content store (every table of the store a run opens) and the
/// logic engine. The older pipelines' scratch databases (`waves`, `residency`, `matcher`, `prefilter`,
/// `sql_executor`) keep nothing between runs; their tables in the store are the store's, and are checked here.
const KEPT_IN: &[&str] = &["content_store", "code_rule_dsl/logic", "code_rule_dsl/stats", "code_rule_dsl/slice.rs", "code_rule_dsl/slice"];

/// Each table a `CREATE TABLE` in `KEPT_IN` names, with the files that make it.
fn tables() -> BTreeMap<String, BTreeSet<String>> {
    let mut fs = Vec::new();
    for d in KEPT_IN {
        let p = src().join(d);
        if p.is_file() {
            fs.push(p);
        } else {
            files(&p, &mut fs);
        }
    }
    let re = regex::Regex::new(r"CREATE\s+(?:TEMP\s+|TEMPORARY\s+)?TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?(?:[a-z_]+\.)?([A-Za-z_{][A-Za-z_0-9{}:]*)").unwrap();
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for f in fs {
        let text = std::fs::read_to_string(&f).unwrap_or_default();
        for c in re.captures_iter(&text) {
            // A name a `format!` makes is listed with its file: `{name} in content_store/kept_strata.rs`.
            let name = if c[1].contains('{') { format!("{} in {}", &c[1], rel(&f)) } else { c[1].to_owned() };
            out.entry(name).or_default().insert(rel(&f));
        }
    }
    out
}

/// Each `static` behind a lock or a cell that holds more than a number or a flag, in the engine's run path, and each
/// value of the run's context that does (a type bound by `bind`, `bind_here`, or read by `bound`, `with_bound`;
/// decision 0442): (file, name).
fn maps() -> BTreeSet<(String, String)> {
    let mut fs = Vec::new();
    for d in ["code_rule_dsl/logic", "content_store"] {
        files(&src().join(d), &mut fs);
    }
    fs.push(src().join("code_rule_dsl/text.rs"));
    let head = regex::Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?static\s+([A-Z_][A-Z_0-9]*)\s*:").unwrap();
    let held = regex::Regex::new(r"(Mutex|RwLock|RefCell|OnceLock|LazyLock|Cell)\s*<").unwrap();
    // A plain number, flag or instant holds no result.
    let plain = regex::Regex::new(r"(Mutex|RwLock|RefCell|OnceLock|LazyLock|Cell)\s*<\s*(\(\s*\)|bool|u8|u16|u32|u64|usize|i32|i64|f64|std::time::Instant|Instant|std::time::Duration|Option<\s*(usize|u64|i64|bool|f64|std::time::Instant)\s*>|\(\s*(Option<usize>|usize|u64|f64)\s*,\s*(usize|u64|f64)\s*\))\s*>").unwrap();
    let mut out = BTreeSet::new();
    // The run's context: the types bound, each declared with its field in the module that binds and reads it.
    let binds = regex::Regex::new(r"\bbind(?:_here)?\(\s*([A-Z][A-Za-z0-9]*)[({]|\bbound::<([A-Z][A-Za-z0-9]*)>|\bwith_bound\(\|[a-z_]+: &([A-Z][A-Za-z0-9]*)\|").unwrap();
    let tuple = regex::Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?struct\s+([A-Z][A-Za-z0-9]*)\((.*)\);").unwrap();
    // A record (named fields) holds more than a number or a flag.
    let record = regex::Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?struct\s+([A-Z][A-Za-z0-9]*)\s*\{").unwrap();
    let flat = regex::Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(bool|u8|u16|u32|u64|usize|i32|i64|f64|&'static str|Option<\s*(usize|u64|i64|bool|f64)\s*>)\s*$").unwrap();
    let texts: Vec<(std::path::PathBuf, String)> = fs.iter().map(|f| (f.clone(), std::fs::read_to_string(f).unwrap_or_default())).collect();
    for (f, text) in &texts {
        let bound: BTreeSet<&str> = binds.captures_iter(text).filter_map(|c| c.iter().skip(1).flatten().next().map(|m| m.as_str())).collect();
        for l in text.lines() {
            if let Some(c) = tuple.captures(l)
                && bound.contains(&c[1].as_ref())
                && !flat.is_match(&c[2])
            {
                out.insert((rel(f), c[1].to_owned()));
            }
            if let Some(c) = record.captures(l)
                && bound.contains(&c[1].as_ref())
            {
                out.insert((rel(f), c[1].to_owned()));
            }
        }
    }
    for f in fs {
        let text = std::fs::read_to_string(&f).unwrap_or_default();
        let mut lines: Vec<&str> = text.lines().collect();
        // A file's inline tests (`#[cfg(test)] mod … {`, to its closing brace) are tests, as a test file is: blanked.
        let mut i = 0;
        while i + 1 < lines.len() {
            if lines[i].trim() == "#[cfg(test)]" && lines[i + 1].trim_start().starts_with("mod ") && lines[i + 1].trim_end().ends_with('{') {
                let mut depth = 0i64;
                let mut j = i + 1;
                while j < lines.len() {
                    depth += lines[j].matches('{').count() as i64 - lines[j].matches('}').count() as i64;
                    lines[j] = "";
                    j += 1;
                    if depth <= 0 {
                        break;
                    }
                }
                i = j;
            } else {
                i += 1;
            }
        }
        for (i, l) in lines.iter().enumerate() {
            let Some(c) = head.captures(l) else { continue };
            // The declaration up to its `=`, over a few lines.
            let decl: String = lines[i..(i + 4).min(lines.len())].join(" ");
            let decl = decl.split(" = ").next().unwrap_or("");
            if held.is_match(decl) && !plain.is_match(decl) {
                out.insert((rel(&f), c[1].to_owned()));
            }
        }
    }
    out
}

#[test]
fn every_table_is_facts_or_listed() {
    let found = tables();
    let listed: BTreeSet<&str> = FACT_STORE.iter().chain(SCRATCH).copied().chain(NOT_YET_TABLES.iter().map(|t| t.0)).collect();
    let new: Vec<String> = found.iter().filter(|(t, _)| !listed.contains(t.as_str())).map(|(t, f)| format!("{t}\t{}", f.iter().cloned().collect::<Vec<_>>().join(" "))).collect();
    assert!(new.is_empty(), "tables in no list of decision 0104 (a cache is facts: keep it as rows of a relation, not a table of its own):\n{}", new.join("\n"));
    let gone: Vec<&&str> = listed.iter().filter(|t| !found.contains_key(**t)).collect();
    assert!(gone.is_empty(), "listed tables no longer made (cut them from the list, and lower the pin if moved): {gone:?}");
    let mut seen = BTreeSet::new();
    for t in FACT_STORE.iter().chain(SCRATCH).chain(NOT_YET_TABLES.iter().map(|t| &t.0)) {
        assert!(seen.insert(*t), "{t} is in two lists");
    }
    assert!(NOT_YET_TABLES.len() <= NOT_YET_TABLES_PIN, "a cache table added to the list (it may only shrink): {} > {NOT_YET_TABLES_PIN}", NOT_YET_TABLES.len());
    assert_eq!(NOT_YET_TABLES.len(), NOT_YET_TABLES_PIN, "a cache table moved: lower NOT_YET_TABLES_PIN to {}", NOT_YET_TABLES.len());
}

#[test]
fn every_kept_map_is_state_or_listed() {
    let found = maps();
    let listed: BTreeSet<(String, String)> =
        STATE.iter().map(|(f, s)| (f.to_string(), s.to_string())).chain(NOT_YET_MAPS.iter().map(|(f, s, _)| (f.to_string(), s.to_string()))).collect();
    let new: Vec<String> = found.difference(&listed).map(|(f, s)| format!("{f}\t{s}")).collect();
    assert!(new.is_empty(), "maps in no list of decision 0104 (a result kept for later use is facts, not a map of its own):\n{}", new.join("\n"));
    let gone: Vec<String> = listed.difference(&found).map(|(f, s)| format!("{f}\t{s}")).collect();
    assert!(gone.is_empty(), "listed maps no longer there (cut them from the list, and lower the pin if moved):\n{}", gone.join("\n"));
    assert!(NOT_YET_MAPS.len() <= NOT_YET_MAPS_PIN, "a cache map added to the list (it may only shrink): {} > {NOT_YET_MAPS_PIN}", NOT_YET_MAPS.len());
    assert_eq!(NOT_YET_MAPS.len(), NOT_YET_MAPS_PIN, "a cache map moved: lower NOT_YET_MAPS_PIN to {}", NOT_YET_MAPS.len());
}
