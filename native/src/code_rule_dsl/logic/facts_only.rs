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
    "atom_segment",
    "atom_key",
    "atom_key_end",
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
    "push_pending",
    "rows",
    "git_alias",
    "git_trees",
    "git_tree_entries",
    "snapshots",
    "snapshot_files",
    "workspaces",
    "workspace_overlay",
    "node",
    "tape",
    "node_type",
    "frames",
    "frame_spaces",
    // The frames' own bookkeeping (decision 0117 step 5): each space's confirmations, and each run's use of it.
    "frame_confirmed",
    "frame_part_used",
    "frame_program_used",
    "frame_used",
    // The executor's measured jobs (decision 0100 §14), the cost model's data: kept as the measurements they are.
    "samples",
    // The SQL executor's planning observations (`costs::sql_plans`), its plan model's data: measurements, as `samples`.
    "sql_plan_observations",
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
    ("cross_rows", "3"),
    ("cross_blocks", "3"),
    ("cross_components", "3"),
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
    ("local_check_results", "8"),
    ("context_check_results", "8"),
];
/// The pinned size of `NOT_YET_TABLES`: lowered by each move, never raised. Raised once, 61 to 62, by the merge of
/// the cost classifier's per-machine `models` (2026-11-10), which a move to facts has yet to take; back to 61 once a
/// merge took one out (2026-11-12).
const NOT_YET_TABLES_PIN: usize = 61;

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
    // This thread's run's binding (CPU or Metal).
    ("code_rule_dsl/logic/plan/placement.rs", "DEVICE"),
    // Profiling: the placements a recording run made, and the writer's jobs by kind.
    ("code_rule_dsl/logic/plan/placement.rs", "RECORDED"),
    ("content_store/write_back.rs", "BY_KIND"),
    // The coverage check's census of the shipped rule files, read once by a test process.
    ("code_rule_dsl/logic/plan/coverage.rs", "CENSUS"),
    // The process's interned relation names (`front::Name`): one allocation per distinct name, not a result kept.
    ("code_rule_dsl/logic/front/name.rs", "TABLE"),
    ("code_rule_dsl/logic/costs.rs", "FITTED"),
    ("code_rule_dsl/logic/costs.rs", "WRITER"),
    ("code_rule_dsl/logic/costs.rs", "MODEL_STATE"),
    // The SQL executor's plan model, fitted from `sql_plan_observations`, and the observations queued for writing.
    ("code_rule_dsl/logic/costs/sql_plans.rs", "MODEL"),
    ("code_rule_dsl/logic/costs/sql_plans.rs", "QUEUE"),
    // The SQL lane's executor, and the work a thread's device wave runs when flushed (`lanes::wave`).
    ("code_rule_dsl/logic/lanes.rs", "SQL"),
    ("code_rule_dsl/logic/lanes.rs", "DEFERRED"),
    // The device's wave on this thread (decision 0113 §3): its open command buffer, the waves flushed, the strata
    // waiting for theirs; and the rule being compiled's arena-text slots. State of the run, not a result kept.
    ("code_rule_dsl/logic/native/setfix/device.rs", "WAVE"),
    ("code_rule_dsl/logic/native/setfix/device.rs", "FLUSHED"),
    ("code_rule_dsl/logic/native/setfix/device.rs", "SUSPENDED"),
    ("code_rule_dsl/logic/native/setfix/device.rs", "ARENA"),
    // Which connections hold tables or views a run left in SQL: state of the connections, not a result kept.
    ("code_rule_dsl/logic/plan.rs", "TEMP_LEFT"),
    // Each live connection's path and symbols, by its scope; dropped when it closes (`setfix::forget_scope`).
    ("code_rule_dsl/logic/native/setfix.rs", "SCOPE_PATHS"),
    ("code_rule_dsl/logic/native/setfix.rs", "SCOPE_SYMS"),
    ("code_rule_dsl/logic/lanes.rs", "ALL"),
    ("code_rule_dsl/logic/lanes.rs", "CPU"),
    ("code_rule_dsl/logic/lanes.rs", "BATCHES"),
    ("code_rule_dsl/logic/lanes.rs", "FORMS"),
    ("code_rule_dsl/logic/lanes.rs", "DONE"),
    ("code_rule_dsl/logic/lanes.rs", "QUEUED"),
    ("code_rule_dsl/logic/native.rs", "LATTICES"),
    ("code_rule_dsl/logic/native.rs", "RESULTS"),
    ("code_rule_dsl/logic/native/extension.rs", "RESULTS"),
    ("code_rule_dsl/logic/native/extension.rs", "TAPE"),
    ("code_rule_dsl/logic/native/extension/setfix.rs", "COMPONENTS"),
    ("code_rule_dsl/logic/native/extension/setfix.rs", "DERIVED"),
    ("code_rule_dsl/logic/native/extension/setfix.rs", "INPUTS"),
    ("code_rule_dsl/logic/native/extension/setfix.rs", "LOADS"),
    ("code_rule_dsl/logic/native/extension/setfix.rs", "SYMS"),
    ("code_rule_dsl/logic/native/setfix.rs", "ADOPTED"),
    ("code_rule_dsl/logic/native/setfix.rs", "CHANGING"),
    ("code_rule_dsl/logic/native/setfix.rs", "COMPONENTS"),
    ("code_rule_dsl/logic/native/setfix.rs", "DERIVATIONS"),
    ("code_rule_dsl/logic/native/setfix.rs", "DERIVED"),
    ("code_rule_dsl/logic/native/setfix.rs", "INPUTS"),
    ("code_rule_dsl/logic/native/setfix.rs", "LOADS"),
    ("code_rule_dsl/logic/native/setfix.rs", "STORED"),
    ("code_rule_dsl/logic/native/setfix.rs", "SYMS"),
    // Each live connection's path and symbols, dropped at its close (`forget_scope`): state of the connections.
    ("code_rule_dsl/logic/native/setfix.rs", "SCOPE_PATHS"),
    ("code_rule_dsl/logic/native/setfix.rs", "SCOPE_SYMS"),
    ("code_rule_dsl/logic/native/setfix/device.rs", "GROWTH"),
    ("code_rule_dsl/logic/native/setfix.rs", "PROBES"),
    // A live connection's file and symbol table, by its handle (dropped as it closes, `forget_scope`): state of the
    // connections, not a result kept.
    ("code_rule_dsl/logic/native/setfix.rs", "SCOPE_PATHS"),
    ("code_rule_dsl/logic/native/setfix.rs", "SCOPE_SYMS"),
    // The mixed index spike's per-thread counters (the shape grown wave by wave): a measurement, not a result kept.
    // Atoms reserved by components not yet retired, and this thread's reservation (decision 0114 §4): which
    // pending atoms no committed row may hold yet, not a result kept.
    ("content_store/atoms.rs", "RESERVED"),
    ("content_store/atoms.rs", "RESERVING"),
    ("code_rule_dsl/logic/native/setfix.rs", "GROWN"),
    ("code_rule_dsl/logic/native/setfix.rs", "VARIANTS"),
    // A test process's own tape cache path (`corpus::test_tape_cache`): where the store is, not a result kept.
    ("code_rule_dsl/logic/plan/corpus.rs", "PATH"),
    ("code_rule_dsl/logic/native/setfix/device.rs", "LAST"),
    ("code_rule_dsl/logic/native/setfix/device.rs", "RESIDENT"),
    ("code_rule_dsl/logic/native/setfix/level.rs", "PENDING"),
    ("code_rule_dsl/logic/native/setfix/table.rs", "BUILT"),
    ("code_rule_dsl/logic/plan.rs", "LOG"),
    ("code_rule_dsl/logic/plan.rs", "PATH"),
    ("code_rule_dsl/logic/plan.rs", "SETTING"),
    // core.rr's declarations (RER-131), read once from the engine's own library text: not a result kept.
    ("code_rule_dsl/logic/rr/lower/core.rs", "DECLS"),
    // This thread's zstd contexts by dictionary (RER-131): open things, not results.
    ("content_store/compression.rs", "DECODERS"),
    ("content_store/compression.rs", "DICT_ENCODERS"),
    // The language tags the store's dictionaries are chosen by (RER-131): set once as the library loads.
    ("content_store/compression.rs", "LANGUAGE_TAGS"),
    // The store handles open in this process, by path (RER-131): a registry of open things.
    ("content_store/connection.rs", "HANDLES"),
    ("code_rule_dsl/logic/plan/choices.rs", "DURABLE"),
    ("code_rule_dsl/logic/plan/corpus.rs", "TEST_TAPE_CACHE"),
    // A test's own store, the template's path and the threads said to ask off a test's (`test_store`): which files
    // the tests' stores are, not results.
    ("code_rule_dsl/logic/plan/corpus/test_store.rs", "OWN"),
    ("code_rule_dsl/logic/plan/corpus/test_store.rs", "PATH"),
    ("code_rule_dsl/logic/plan/corpus/test_store.rs", "SAID"),
    // Profiling totals, consumed and cleared by each report; no reusable results.
    ("code_rule_dsl/logic/plan/corpus/memo.rs", "SIZES"),
    ("code_rule_dsl/logic/plan/corpus/inputs.rs", "BASE"),
    ("code_rule_dsl/logic/plan/corpus/inputs.rs", "FACTS_DIR"),
    ("code_rule_dsl/logic/plan/corpus/phases.rs", "HANDED_DOWN"),
    // Phase 1's exports while the run that ran it is under way: the run's own facts, gone when it ends.
    ("code_rule_dsl/logic/plan/corpus/phases.rs", "PHASE_ONE"),
    ("code_rule_dsl/logic/plan/corpus/phases.rs", "RUNNING"),
    ("code_rule_dsl/logic/plan/sketch.rs", "FILE"),
    ("code_rule_dsl/logic/profile.rs", "DIR"),
    ("code_rule_dsl/logic/profile.rs", "ON"),
    ("code_rule_dsl/logic/profile.rs", "STACK"),
    ("code_rule_dsl/logic/profile.rs", "STATE"),
    ("code_rule_dsl/logic/profile.rs", "THREADS"),
    ("code_rule_dsl/logic/profile/memory.rs", "DOING"),
    ("code_rule_dsl/logic/profile/memory.rs", "SAMPLED"),
    ("code_rule_dsl/logic/profile/roofline.rs", "ROOFS"),
    ("code_rule_dsl/logic/rr.rs", "CORE"),
    ("code_rule_dsl/logic/rr.rs", "READS"),
    ("content_store/atoms.rs", "CANONICAL"),
    // Each database file's identity, read once a file: a registry, as CANONICAL is.
    ("content_store/atoms.rs", "IDENTITIES"),
    ("content_store/atoms.rs", "MEMORY"),
    ("content_store/atoms.rs", "REGISTRY"),
    ("content_store/compression.rs", "ENCODER"),
    ("content_store/connection.rs", "ALL_CONNECTIONS"),
    ("content_store/connection.rs", "ATTEMPT_WAITED"),
    ("content_store/connection.rs", "BUSY_ATTEMPT_HOOK"),
    ("content_store/connection.rs", "BUSY_DEADLINE_OVERRIDE"),
    ("content_store/connection.rs", "BUSY_TIMEOUT_OVERRIDE"),
    ("content_store/connection.rs", "BUSY_WAIT_HOOK"),
    ("content_store/connection.rs", "CONNECTION_FUNCTIONS"),
    ("content_store/connection.rs", "OPERATION_WAITED"),
    ("content_store/connection.rs", "THREAD_CONNECTIONS"),
    ("content_store/connection.rs", "WAIT_SPENT"),
    ("content_store/hazards.rs", "HAZARDS"),
    // The run each frame space last counted in this process, so a run's programs count it once.
    ("content_store/frames.rs", "COUNTED"),
    // The store's frames as this process holds them, ahead of SQLite (decision 0104): the facts themselves, read
    // once before a run's first wave, not a cache of anything else.
    ("content_store/frames.rs", "SNAPSHOTS"),
    // The device's automata and test tables for regex and text stages, held while the thread runs (a command buffer
    // may read them).
    ("code_rule_dsl/logic/native/setfix/device.rs", "AUTOMATA"),
    ("code_rule_dsl/logic/native/setfix/device.rs", "TESTS"),
    // A switch: testing mode's store (decision 0130).
    ("code_rule_dsl/logic/plan/corpus.rs", "STORE"),
    // Phase 1's runs within one batch of policies over one tree, dropped with it: nothing kept past the batch.
    ("code_rule_dsl/logic/plan/corpus/phases.rs", "BATCH"),
    // With them, the memo plans of the batch's runs so far, by root (O-store-single-program): dropped with the batch.
    ("code_rule_dsl/logic/plan/corpus/phases.rs", "BATCH_PLANS"),
    // A registry of an open thing: the holders of the rule files' shared tape-cache handle, and the handle.
    ("code_rule_dsl/logic/plan/corpus/phases.rs", "READING"),
    ("content_store/observe.rs", "OBSERVERS"),
    // The journal's open streams, one a root (decision 0105): open things.
    ("content_store/journal.rs", "LIVE"),
    // A job's own plans and observations (decisions 0113, 0116), taken into the run's as the job ends: the run's own
    // state on the job's thread, gone with it.
    ("code_rule_dsl/logic/plan.rs", "OWN_PLANS"),
    ("code_rule_dsl/logic/plan/keeps.rs", "OWN_OBSERVED"),
    // Profiling: where a plan is made and the keys planned in a profiled run, for the planning budget's account.
    ("code_rule_dsl/logic/plan.rs", "PLAN_SITE"),
    ("code_rule_dsl/logic/plan.rs", "PLANNED_NOW"),
    // Measurement aids under `RERAIL_PLAN_GREEDY_PROBE`: its switches, and the probe's counts and sums.
    ("code_rule_dsl/logic/plan.rs", "PROBE"),
    ("code_rule_dsl/logic/plan.rs", "SEEN"),
    ("code_rule_dsl/logic/plan.rs", "SUMS"),
    ("content_store/reclamation.rs", "EXECUTED_COLLECT_SQL"),
    ("content_store/schema.rs", "SCHEMA_OBJECTS"),
    ("content_store/snapshots.rs", "AFTER_GIT_TREE_TRANSACTION"),
    ("content_store/write_back.rs", "WRITERS"),
];
/// Maps (file, static) that are caches not yet facts, with the 0104 step that moves each.
const NOT_YET_MAPS: &[(&str, &str, &str)] = &[
    ("code_rule_dsl/logic/plan/corpus/inputs.rs", "CONSULTED", "1"),
    ("code_rule_dsl/logic/strings/git.rs", "STATE", "1"),
    ("code_rule_dsl/logic/rr.rs", "LIBRARIES", "2"),
    ("code_rule_dsl/logic/rr.rs", "PARSED", "2"),
    ("code_rule_dsl/logic/rr.rs", "PROGRAMS", "2"),
    ("code_rule_dsl/logic/plan/corpus/phase0.rs", "KEPT", "2"),
    ("code_rule_dsl/logic/plan/corpus.rs", "COMBINED", "2"),
    ("code_rule_dsl/logic/plan/corpus.rs", "PREPARED", "2"),
    ("code_rule_dsl/logic/plan/choices.rs", "KEPT", "2"),
    ("code_rule_dsl/logic/plan/corpus/guests.rs", "KEPT", "3"),
    ("code_rule_dsl/logic/plan/corpus/session.rs", "SESSION", "3"),
    ("code_rule_dsl/logic/plan/corpus/recursive_spike.rs", "CARRY", "4"),
    ("code_rule_dsl/logic/plan.rs", "PLANS", "6"),
    ("code_rule_dsl/logic/plan.rs", "SESSION_PLANS", "6"),
    ("code_rule_dsl/logic/plan.rs", "KEPT", "6"),
    ("code_rule_dsl/logic/derived.rs", "MEMO", "6"),
    ("code_rule_dsl/logic/derived.rs", "SESSION_MEMO", "6"),
    // The observed keeps, a handle a strand enters (decision 0113 §4).
    ("code_rule_dsl/logic/plan/keeps.rs", "OBSERVED_AT", "6"),
    ("code_rule_dsl/logic/native/setfix/table.rs", "PAID", "6"),
    ("code_rule_dsl/logic/strings.rs", "MATCHERS", "7"),
    // RER-131's compiled group patterns, where `GROUPS` was.
    ("code_rule_dsl/logic/strings.rs", "GROUP_REGEXES", "7"),
    ("code_rule_dsl/logic/strings/functions.rs", "CACHE", "7"),
    ("code_rule_dsl/text.rs", "LOWERED", "7"),
    ("code_rule_dsl/text.rs", "NAMES", "7"),
];
/// The pinned size of `NOT_YET_MAPS`: lowered by each move, never raised.
const NOT_YET_MAPS_PIN: usize = 24;

fn src() -> PathBuf {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join("src")
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

/// Each `static` behind a lock or a cell that holds more than a number or a flag, in the engine's run path:
/// (file, name).
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
    for f in fs {
        let text = std::fs::read_to_string(&f).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
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
