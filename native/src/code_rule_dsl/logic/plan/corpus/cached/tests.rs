use super::{Entry, Rows, fresh};
use std::collections::BTreeMap;

fn rows(rel: &str, lines: &[&str]) -> Rows {
    BTreeMap::from([(rel.to_owned(), lines.iter().map(|l| (*l).to_owned()).collect())])
}

fn store() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cache.sqlite");
    (dir, path)
}

#[test]
fn an_entry_is_fresh_while_what_it_read_is_as_it_was() {
    let (_dir, path) = store();
    let mut e = Entry::new("constants", "py_version");
    e.read("file", "/ws/.venv/pyvenv.cfg", "aa");
    e.read("refused", "/ws/.python-version", "none");
    e.read("file", "/ws/unreadable", "");
    e.read("build", "rerail-engine", "b1");
    e.keep(&path, &rows("py_version", &["3\t12"])).unwrap();
    let now = |file: &'static str| move |kind: &str, name: &str| Some(match kind { "file" if name == "/ws/unreadable" => "", "file" => file, "refused" => "none", _ => "b1" }.to_owned());
    let hit = fresh(&path, "constants", "py_version", &mut now("aa")).unwrap().expect("fresh");
    assert_eq!(hit.rows["py_version"], vec!["3\t12".to_owned()]);
    // The reads come back as they were made: a missing path's "none", an unreadable thing's empty digest.
    assert!(hit.reads.contains(&("refused".to_owned(), "/ws/.python-version".to_owned(), "none".to_owned())));
    assert!(hit.reads.contains(&("file".to_owned(), "/ws/unreadable".to_owned(), String::new())));
    // A read whose digest moved, and one that can no longer be observed, each make it stale.
    assert!(fresh(&path, "constants", "py_version", &mut now("bb")).unwrap().is_none());
    assert!(fresh(&path, "constants", "py_version", &mut |kind, _| (kind != "build").then(|| "aa".to_owned())).unwrap().is_none());
    // Another key, or another cache, has no entry.
    assert!(fresh(&path, "constants", "other", &mut now("aa")).unwrap().is_none());
    assert!(fresh(&path, "phases", "py_version", &mut now("aa")).unwrap().is_none());
}

#[test]
fn a_key_keeps_an_entry_for_each_state_of_its_inputs() {
    let (_dir, path) = store();
    for (digest, row) in [("aa", "3\t11"), ("bb", "3\t12")] {
        let mut e = Entry::new("constants", "py_version");
        e.read("file", "/ws/.venv/pyvenv.cfg", digest);
        e.keep(&path, &rows("py_version", &[row])).unwrap();
    }
    // A branch and its revert both find their entry.
    for (digest, row) in [("aa", "3\t11"), ("bb", "3\t12")] {
        let hit = fresh(&path, "constants", "py_version", &mut |_, _| Some(digest.to_owned())).unwrap().expect("fresh");
        assert_eq!(hit.rows["py_version"], vec![row.to_owned()]);
    }
    assert!(fresh(&path, "constants", "py_version", &mut |_, _| Some("cc".to_owned())).unwrap().is_none());
}

/// Decision 0102 §6: every cache follows the one pattern, and the caches not yet moved may only shrink.
mod architecture {
    use std::path::{Path, PathBuf};

    fn src() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
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
            } else if name.ends_with(".rs") && !name.ends_with("tests.rs") && !name.starts_with("test_") {
                out.push(path);
            }
        }
    }

    fn relative(path: &Path) -> String {
        path.strip_prefix(src()).unwrap().to_string_lossy().into_owned()
    }

    /// The tables of the stores a run keeps between runs (the content store, the phase store) and of the code
    /// that writes them, as each is: the pattern's, the store's own (not a cache), or a cache not yet moved.
    const PATTERN: &[&str] = &super::super::TABLES;
    const NOT_CACHES: &[&str] = &[
        // Sources, names, atoms and the store's bookkeeping.
        "source_objects", "source_aliases", "strings", "atom_segment", "hashes", "paths", "store_state", "object_pins",
        "push_pending", "snapshots", "snapshot_files", "workspaces", "workspace_overlay", "git_trees", "git_tree_entries",
        // Collection (0053): moves to reachability over `cache_entries` (0102 step 7).
        "corpus_roots", "corpus_manifests", "corpus_manifest_plans", "corpus_live", "corpus_plans", "corpus_plan_relations",
        "corpus_decided", "corpus_swept", "corpus_state",
        // Phase 0's observer: what is there now, the facts freshness reads (0102 §2).
        "observed", "observer_meta", "git_alias",
        // A run's scratch, in memory or temporary.
        "x", "rows", "x.node_type", "s.stat_col", "s.stat_docrange", "s.stat_edge", "s.stat_kind", "s.stat_meta", "s.stat_val",
    ];
    /// Caches not yet moved to the pattern (O-caches-as-facts). This list may only shrink.
    const NOT_YET_MOVED: &[&str] = &[
        // Tapes, documents and their indexes (0102 step 5).
        "tape_objects", "file_documents", "prefilter_sets", "tape_kind_counts", "term_postings", "term_freq", "term_kind_freq",
        "term_node_freq", "term_coverage", "edge_freq", "text_matcher", "text_pending", "text_term", "vocabulary_literal",
        "vocabulary_pending", "vocabulary_predicate", "vocabulary_term", "tree_pages", "kind_size", "search_index_sets",
        // The per-file memo and its plans (0102 step 4).
        "facts_results", "facts_plan_sets", "facts_completions", "compiled_plans", "compiled_plan_reads", "plan_feedback",
        // Phases and the whole-run cutoff (0102 step 3).
        "phase_granules", "phase_cones", "phase_record", "export_versions", "store_epoch", "run_record",
        // Kept strata, cross parts, statistics, checks (0102 step 6).
        "kept_strata", "kept_strata_programs", "cross_blocks", "cross_components", "cross_rows", "stat_blob", "fact_stats",
        "fact_value_stats", "corpus_stats", "local_check_results", "context_check_results", "candidate_catalogs",
        "candidate_checks", "completed_directory_reuse", "completed_workspace_revisions", "tree_result_segments",
        "completed_snapshot_trees", "accel_models",
        // The derivation graph (03 §14): its bindings and inputs fold into `cache_reads` and `observed`.
        "name_bindings", "derivations", "derivation_inputs",
    ];

    #[test]
    fn every_stored_table_is_the_patterns_the_stores_own_or_a_cache_not_yet_moved() {
        let mut files = Vec::new();
        for dir in ["content_store", "code_rule_dsl/logic/plan"] {
            sources(&src().join(dir), &mut files);
        }
        let create = regex::Regex::new(r"CREATE TABLE (?:IF NOT EXISTS )?([a-z_][a-z_0-9.]*)").unwrap();
        let mut unknown = Vec::new();
        for file in &files {
            let text = std::fs::read_to_string(file).unwrap();
            for c in create.captures_iter(&text) {
                let name = &c[1];
                if ![PATTERN, NOT_CACHES, NOT_YET_MOVED].iter().any(|list| list.contains(&name)) {
                    unknown.push(format!("{}: {name}", relative(file)));
                }
            }
        }
        assert!(unknown.is_empty(), "a stored table outside decision 0102's pattern (keep it as cache_entries/cache_reads/cache_rows, or list it): {unknown:?}");
    }

    /// The tests of freshness written outside the pattern, and the one call site each may still have.
    const BESPOKE_CALLS: &[(&str, &str)] = &[
        ("code_rule_dsl/logic/plan/corpus.rs", "inputs::still("),
        ("code_rule_dsl/logic/plan/corpus.rs", ".still("),
        ("code_rule_dsl/logic/plan/corpus/phases.rs", "unchanged_phase("),
        ("code_rule_dsl/logic/plan/corpus/reuse.rs", "!unchanged(programs"),
        ("code_rule_dsl/logic/plan/corpus/combined.rs", "file.unchanged("),
        ("code_rule_dsl/facts.rs", "derivation_valid("),
    ];

    #[test]
    fn no_cache_tests_its_own_freshness_but_those_not_yet_moved() {
        let mut files = Vec::new();
        sources(&src(), &mut files);
        let mut found = Vec::new();
        for file in &files {
            let rel = relative(file);
            let text = std::fs::read_to_string(file).unwrap();
            for needle in ["inputs::still(", ".still(", "unchanged_phase(", "unchanged(programs", "file.unchanged(", "derivation_valid("] {
                let calls = text.lines().filter(|l| l.contains(needle) && !l.trim_start().starts_with("//") && !l.contains("fn ")).count();
                for _ in 0..calls {
                    found.push((rel.clone(), needle));
                }
            }
        }
        let extra: Vec<String> = found
            .iter()
            .filter(|(file, needle)| !BESPOKE_CALLS.iter().any(|(f, n)| f == file && n.contains(*needle)))
            .map(|(file, needle)| format!("{file}: {needle}"))
            .collect();
        assert!(extra.is_empty(), "a cache tests its own freshness (decision 0102 §6: read `cache_fresh` through `corpus::cached`): {extra:?}");
        for (file, needle) in BESPOKE_CALLS {
            let calls = found.iter().filter(|(f, n)| f == file && needle.contains(*n)).count();
            assert!(calls <= 1, "{file}: {calls} calls of {needle}, one allowed while it is not yet moved");
        }
    }

    #[test]
    fn freshness_is_the_engines_not_rust_comparing_digests() {
        let text = std::fs::read_to_string(src().join("code_rule_dsl/logic/plan/corpus/cached.rs")).unwrap();
        let compared: Vec<&str> = text.lines().filter(|l| l.contains("digest") && (l.contains(" == ") || l.contains(" != "))).collect();
        assert!(compared.is_empty(), "cached.rs compares digests itself: {compared:?}");
        assert!(text.contains("run_facts(&prog)") && super::super::RULES.contains("cache_fresh(c, k, i) :- cache_entry(c, k, i), !cache_stale(c, k, i)."));
    }
}
