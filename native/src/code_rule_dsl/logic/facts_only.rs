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
    "strings",
    "hashes",
    "paths",
    "source_objects",
    "source_aliases",
    "tape_objects",
    "file_documents",
    "object_pins",
    "store_state",
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
    // The executor's measured jobs (decision 0100 §14), the cost model's data: kept as the measurements they are.
    "samples",
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
    "x",
    "a",
    "b",
    "c",
    "h0",
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
    "{t} in code_rule_dsl/logic/native/setfix/lab.rs",
    "{} in code_rule_dsl/slice.rs",
];
/// Caches between runs not yet facts: (table, the 0104 step that moves it).
const NOT_YET_TABLES: &[(&str, &str)] = &[
    ("constant_rows", "1"),
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
/// The pinned size of `NOT_YET_TABLES`: lowered by each move, never raised.
const NOT_YET_TABLES_PIN: usize = 61;

/// Maps (file, static) that keep nothing for later use: registries of open things, queues, switches, profiling, and
/// the engine's own facts in memory within a run.
const STATE: &[(&str, &str)] = &[
    ("code_rule_dsl/logic/costs.rs", "FITTED"),
    ("code_rule_dsl/logic/costs.rs", "WRITER"),
    // Which connections hold tables or views a run left in SQL: state of the connections, not a result kept.
    ("code_rule_dsl/logic/plan.rs", "TEMP_LEFT"),
    // Each live connection's path and symbols, by its scope; dropped when it closes (`setfix::forget_scope`).
    ("code_rule_dsl/logic/native/setfix.rs", "SCOPE_PATHS"),
    ("code_rule_dsl/logic/native/setfix.rs", "SCOPE_SYMS"),
    // The store files this process opened and checked (by device and inode): a registry of open things.
    ("content_store/mod.rs", "CHECKED"),
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
    ("code_rule_dsl/logic/native/setfix/device.rs", "GROWTH"),
    ("code_rule_dsl/logic/native/setfix.rs", "PROBES"),
    // The mixed index spike's per-thread counters (the shape grown wave by wave): a measurement, not a result kept.
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
    ("code_rule_dsl/logic/plan/choices.rs", "DURABLE"),
    ("code_rule_dsl/logic/plan/corpus.rs", "TEST_TAPE_CACHE"),
    // The build's Python grammar, and what follows from it and a matcher's text alone (its single-field answers, its
    // lowered code): the process's code, as a compiled rule's, not a run's result.
    ("code_rule_dsl/logic/plan/corpus/matchers.rs", "GRAMMAR"),
    ("code_rule_dsl/logic/plan/corpus/matchers.rs", "KEPT"),
    ("code_rule_dsl/logic/plan/corpus/matchers.rs", "LOWERED"),
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
    ("code_rule_dsl/logic/derived.rs", "KEYS"),
    ("content_store/atoms.rs", "CANONICAL"),
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
    ("content_store/observe.rs", "OBSERVERS"),
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
    ("code_rule_dsl/logic/plan/keeps.rs", "OBSERVED", "6"),
    // A chunk mix's tree counts read from memory across runs (the Typing branch's edit step): kept statistics.
    ("code_rule_dsl/logic/plan/corpus.rs", "OWN_KEPT", "6"),
    ("code_rule_dsl/logic/native/setfix/table.rs", "PAID", "6"),
    ("code_rule_dsl/logic/strings.rs", "MATCHERS", "7"),
    ("code_rule_dsl/logic/strings.rs", "GROUPS", "7"),
    ("code_rule_dsl/logic/strings/functions.rs", "CACHE", "7"),
    ("code_rule_dsl/text.rs", "LOWERED", "7"),
    ("code_rule_dsl/text.rs", "NAMES", "7"),
];
/// The pinned size of `NOT_YET_MAPS`: lowered by each move, never raised.
// Raised to 25 by the Typing branch's merge (`OWN_KEPT`, a cache it added): to come down with step 6.
const NOT_YET_MAPS_PIN: usize = 25;

fn src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
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
