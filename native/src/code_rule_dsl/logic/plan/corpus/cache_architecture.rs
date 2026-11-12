//! Decision 0103: a cache is two rules, and nothing else may be one. Cached facts are just facts: stored, held in
//! memory and written back the same way as every other fact. This test holds the stores' tables and the freshness
//! tests that were made before the decision. A table or a call that is not on its list fails, so a new cache cannot
//! be made outside the form. An entry is removed when its cache moves to the two rules; none may be added.
//!
//! It also holds the form to rules only: a cache's two rules are ordinary rules, evaluated, kept and maintained as
//! every rule is. Only the lint of `.cache` (`front::caches`) knows a relation is a cache. No store, table, hook,
//! lookup or rewrite of a cache's own.

use std::path::{Path, PathBuf};

fn src() -> PathBuf {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join("src")
}

/// The non-test Rust files under `dir`.
fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.is_dir() {
            if name != "tests" {
                sources(&path, out);
            }
        } else if name.ends_with(".rs") && !name.ends_with("tests.rs") && !name.starts_with("test_") && name != "cache_architecture.rs" {
            out.push(path);
        }
    }
}

fn relative(path: &Path) -> String {
    path.strip_prefix(src()).unwrap().to_string_lossy().into_owned()
}

/// The stores' tables that are not caches: sources, names and the store's bookkeeping, collection, and a run's
/// scratch (in memory or temporary, dropped when the run ends).
const NOT_CACHES: &[&str] = &[
    "source_objects", "source_aliases", "strings", "atom_segment", "atom_key", "atom_key_end", "hashes", "paths", "store_state", "store_identity", "object_pins",
    "push_pending", "snapshots", "snapshot_files", "workspaces", "workspace_overlay", "git_trees", "git_tree_entries", "git_alias",
    "corpus_roots", "corpus_manifests", "corpus_manifest_plans", "corpus_live", "corpus_plans", "corpus_plan_relations",
    "corpus_decided", "corpus_swept", "corpus_state",
    "x", "rows", "facts", "x.node_type", "s.stat_col", "s.stat_docrange", "s.stat_edge", "s.stat_kind", "s.stat_meta", "s.stat_val",
];

/// The relations' own storage between runs: every relation's rows as frames (notes/t33, K1). A cache's rows are
/// kept here as any relation's are, never in a table of their own.
const RELATIONS: &[&str] = &["frames", "frame_spaces"];

/// Caches made before decision 0103, each to move to the two rules (§7 lists each one's). This list may only shrink.
const NOT_YET_MOVED: &[&str] = &[
    // Tapes, documents and their indexes.
    "tape_objects", "file_documents", "prefilter_sets", "tape_kind_counts", "term_postings", "term_freq", "term_kind_freq",
    "term_node_freq", "term_coverage", "edge_freq", "text_matcher", "text_pending", "text_term", "vocabulary_literal",
    "vocabulary_pending", "vocabulary_predicate", "vocabulary_term", "tree_pages", "kind_size", "search_index_sets",
    // The per-file memo and its plans.
    "facts_results", "facts_plan_sets", "facts_completions", "compiled_plans", "compiled_plan_reads", "plan_feedback",
    // The whole run's record.
    "run_record",
    // The cross part's memo, statistics, checks, completed trees.
    "cross_blocks", "cross_components", "cross_rows", "stat_blob", "fact_stats", "fact_value_stats", "corpus_stats",
    "local_check_results", "context_check_results", "candidate_catalogs", "candidate_checks", "completed_directory_reuse",
    "completed_workspace_revisions", "tree_result_segments", "completed_snapshot_trees", "accel_models",
    // The derivation graph: what it records is the keys of §7.
    "name_bindings", "derivations", "derivation_inputs",
    // Phase 0's observations (what each read is now).
    "observed", "observer_meta",
];

#[test]
fn every_stored_table_is_a_relations_the_stores_own_or_a_cache_not_yet_moved() {
    let mut files = Vec::new();
    for dir in ["content_store", "code_rule_dsl/logic/plan"] {
        sources(&src().join(dir), &mut files);
    }
    let create = regex::Regex::new(r"CREATE TABLE (?:IF NOT EXISTS )?([a-z_][a-z_0-9.]*)").unwrap();
    let mut unknown = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        for c in create.captures_iter(&text) {
            let name = c[1].to_owned();
            if ![NOT_CACHES, RELATIONS, NOT_YET_MOVED].iter().any(|list| list.contains(&name.as_str())) {
                unknown.push(format!("{}: {name}", relative(file)));
            }
            seen.insert(name);
        }
    }
    assert!(unknown.is_empty(), "a stored table outside decision 0103 (a cache is two rules in the program's .rr): {unknown:?}");
    // A cache moved: its entry goes, so the list only shrinks.
    let gone: Vec<&&str> = NOT_YET_MOVED.iter().filter(|t| !seen.contains(**t)).collect();
    assert!(gone.is_empty(), "moved to the two rules, so off NOT_YET_MOVED: {gone:?}");
}

/// The tests of freshness written outside the form, and the most call sites each may still have.
const FRESHNESS_CALLS: &[(&str, &str, usize)] = &[
    ("code_rule_dsl/logic/plan/corpus.rs", "inputs::still(", 1),
    ("code_rule_dsl/logic/plan/corpus.rs", ".still(", 2),
    ("code_rule_dsl/logic/plan/corpus/reuse.rs", "unchanged(programs", 1),
    ("code_rule_dsl/logic/plan/corpus/combined.rs", "file.unchanged(", 1),
    ("code_rule_dsl/facts.rs", "derivation_valid(", 1),
    ("code_rule_dsl/logic/plan/corpus/constants.rs", ".still(", 2),
];

#[test]
fn no_cache_tests_its_own_freshness_but_those_not_yet_moved() {
    let mut files = Vec::new();
    sources(&src(), &mut files);
    let needles: Vec<&str> = FRESHNESS_CALLS.iter().map(|(_, n, _)| *n).collect();
    let mut found: Vec<(String, &str)> = Vec::new();
    for file in &files {
        let rel = relative(file);
        let text = std::fs::read_to_string(file).unwrap();
        for needle in &needles {
            let calls = text.lines().filter(|l| l.contains(needle) && !l.trim_start().starts_with("//") && !l.contains("fn ")).count();
            found.extend(std::iter::repeat_n((rel.clone(), *needle), calls));
        }
    }
    let extra: Vec<String> = found.iter().filter(|(file, needle)| !FRESHNESS_CALLS.iter().any(|(f, n, _)| f == file && n == needle)).map(|(f, n)| format!("{f}: {n}")).collect();
    assert!(extra.is_empty(), "a cache tests its own freshness (decision 0103: write its key rule instead): {extra:?}");
    for (file, needle, most) in FRESHNESS_CALLS {
        let calls = found.iter().filter(|(f, n)| f == file && n == needle).count();
        assert!(calls <= *most, "{file}: {calls} calls of {needle}, at most {most} while it is not yet moved");
    }
}

/// The only files that may know a relation is a cache: the lint of `.cache` and the program front that calls it.
/// Nothing that evaluates, keeps or maintains a relation asks whether it is a cache.
const KNOWS_CACHES: &[&str] = &["code_rule_dsl/logic/front/caches.rs", "code_rule_dsl/logic/front.rs"];

/// A cache's own store, hook or lookup: names no engine code may have (decision 0103: cached facts are just facts).
const CACHE_MACHINERY: &[&str] = &["CacheStore", "cache_hits", "queue_cache_rows", "keep_cache_rows", "CachePut", "hold_kept"];

#[test]
fn a_cache_is_rules_only_with_no_store_hook_or_lookup_of_its_own() {
    let mut files = Vec::new();
    sources(&src(), &mut files);
    let word = |name: &str| regex::Regex::new(&format!(r"\b{name}\b")).unwrap();
    let machinery: Vec<(String, regex::Regex)> = CACHE_MACHINERY.iter().map(|n| ((*n).to_owned(), word(n))).collect();
    let knows = regex::Regex::new(r"\bfront::caches\b|\bsuper::caches::").unwrap();
    let mut found = Vec::new();
    for file in &files {
        let rel = relative(file);
        let text = std::fs::read_to_string(file).unwrap();
        for (name, re) in &machinery {
            if re.is_match(&text) {
                found.push(format!("{rel}: {name}"));
            }
        }
        // A re-export (a crate's facade over another's module) names the module but knows nothing of caches.
        let knowing = text.lines().any(|l| knows.is_match(l) && !l.trim_start().starts_with("pub use ") && !l.trim_start().starts_with("use "));
        if knowing && !KNOWS_CACHES.contains(&rel.as_str()) {
            found.push(format!("{rel}: knows a relation is a cache"));
        }
    }
    assert!(found.is_empty(), "a cache's own store, hook or lookup (decision 0103: cached facts are just facts, kept as every fact is): {found:?}");
}
