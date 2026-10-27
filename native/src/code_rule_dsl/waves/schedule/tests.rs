use super::super::owner::Backing;
use super::*;
use crate::code_rule_dsl::facts::vtab::syntax::tests::documents;
use crate::code_rule_dsl::transducer::packed::ExecutionBackend;
use crate::content_store::{Binding, ContentStore};
use std::path::PathBuf;

fn temp(what: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("rerail-waves-{what}-{}-{nonce}.sqlite", std::process::id()))
}
/// A store whose `#core:` names bind the fixture's first `n` documents.
fn store(what: &str, n: usize) -> (ContentStore, Vec<Document>) {
    let store = ContentStore::open(&temp(what)).unwrap();
    let docs: Vec<Document> = documents().into_iter().take(n).collect();
    let bindings: Vec<Binding> = docs.iter().map(|d| Binding { name: d.core.clone(), digest: d.digest }).collect();
    store.bind_names(&bindings).unwrap();
    (store, docs)
}
fn step(output: &str, stratum: u32, reads: &[&str], sql: &str) -> Step {
    Step { output: output.into(), stratum, reads: reads.iter().map(|r| (*r).to_owned()).collect(), sql: sql.into() }
}
/// Ancestry over `child`, semi-naive, and the nodes with no ancestor in a second stratum.
fn ancestry() -> Program {
    Program {
        name: "ancestry".into(),
        relations: vec![("anc".into(), 2), ("root".into(), 1)],
        steps: vec![
            step("anc", 0, &["child"], "INSERT OR IGNORE INTO new_anc SELECT p, c FROM child"),
            step("anc", 0, &["anc", "child"], "INSERT OR IGNORE INTO new_anc SELECT a.c0, k.c FROM delta_anc a JOIN child k ON k.p = a.c1"),
            step("root", 1, &["anc"], "INSERT OR IGNORE INTO new_root SELECT DISTINCT c0 FROM anc WHERE c0 NOT IN (SELECT c1 FROM anc)"),
        ],
        file_local: true,
    }
}
fn none(_: &[Document]) -> StoreResult<Option<PackedDemand>> {
    Ok(None)
}
fn count(lease: &Lease<'_>, sql: &str) -> i64 {
    lease.query_row(sql, [], |r| r.get(0)).unwrap()
}
const CLOSURE: &str = "WITH RECURSIVE r(a, d) AS (SELECT p, c FROM child UNION SELECT r.a, k.c FROM r JOIN child k ON k.p = r.d) SELECT count(*) FROM r";

/// SQL selects each wave's frontier until no relation changes: the closure equals a recursive CTE's
/// over the same vtab, the second stratum runs on the complete closure, and the counts reach the
/// statistics.
#[test]
fn the_frontier_loop_reaches_the_closure_and_runs_strata_in_order() {
    let (store, docs) = store("closure", 2);
    let backing = Backing::default();
    let scope = backing.enter(&store);
    let lease = scope.pool().take().unwrap();
    let out = run(&lease, &store, &ancestry(), "/w", docs, &Limits::default(), &mut none).unwrap();
    let anc = count(&lease, "SELECT count(*) FROM anc");
    assert_eq!(anc, count(&lease, CLOSURE));
    // Each document's module is the one root of its tree.
    assert_eq!(count(&lease, "SELECT count(*) FROM root"), 2);
    assert_eq!(out.rows["rows.anc"], anc as u64);
    // The deepest chain bounds the waves: one per level, one that finds nothing, one for `root`.
    let depth = count(&lease, "SELECT max(n) FROM (SELECT c1, count(*) n FROM anc GROUP BY c1)");
    assert_eq!(i64::from(out.waves), depth + 2);
    assert_eq!(out.changed.len(), 2);
}

/// File-local chunks give the relations, families and digests of one chunk; at most one chunk is
/// built ahead of the one running.
#[test]
fn chunks_give_the_relations_of_one_run() {
    let mut digests = Vec::new();
    for budget in [usize::MAX, 1] {
        let (store, docs) = store(&format!("chunks-{budget}"), 3);
        let backing = Backing::default();
        let scope = backing.enter(&store);
        let lease = scope.pool().take().unwrap();
        let limits = Limits { chunk_bytes: budget, ..Limits::default() };
        let out = run(&lease, &store, &ancestry(), "/w", docs, &limits, &mut none).unwrap();
        assert_eq!(out.chunks, if budget == 1 { 3 } else { 1 });
        let names = [family_name("/w", "ancestry", "anc"), family_name("/w", "ancestry", "root")];
        digests.push((count(&lease, "SELECT count(*) FROM anc"), names.map(|n| store.binding(&n).unwrap())));
    }
    assert_eq!(digests[0], digests[1]);
}

/// A run with unchanged inputs rebinds no family, so nothing reading them runs again; another
/// program over the same documents is a different producer, with the same digests.
#[test]
fn an_unchanged_rerun_changes_no_family() {
    let (store, docs) = store("rerun", 2);
    let backing = Backing::default();
    for (round, changed) in [(0, 2), (1, 0)] {
        let scope = backing.enter(&store);
        let lease = scope.pool().take().unwrap();
        let out = run(&lease, &store, &ancestry(), "/w", docs.clone(), &Limits::default(), &mut none).unwrap();
        assert_eq!(out.changed.len(), changed, "round {round}");
        // The recycled owner starts each run from empty relations.
        assert_eq!(count(&lease, "SELECT count(*) FROM anc"), count(&lease, CLOSURE));
    }
}

/// A document whose core facts were rebound since it was read fails the chunk, not later.
#[test]
fn a_stale_document_fails_its_chunk() {
    let (store, docs) = store("stale", 2);
    store.bind_names(&[Binding { name: docs[1].core.clone(), digest: [7; 32] }]).unwrap();
    let backing = Backing::default();
    let scope = backing.enter(&store);
    let lease = scope.pool().take().unwrap();
    let err = run(&lease, &store, &ancestry(), "/w", docs, &Limits::default(), &mut none).unwrap_err();
    assert!(err.to_string().contains("stale core facts"), "{err}");
}

/// A step reads its chunk's packed batch through `demand(batch)`; one batch per chunk, removed
/// after it.
#[test]
fn steps_read_the_chunks_demand_batch() {
    let (store, docs) = store("demand", 2);
    let backing = Backing::default();
    let scope = backing.enter(&store);
    let lease = scope.pool().take().unwrap();
    let program = Program {
        name: "status".into(),
        relations: vec![("seen".into(), 1)],
        steps: vec![step("seen", 0, &["rerail_batch_status"], "INSERT OR IGNORE INTO new_seen SELECT complete FROM rerail_batch_status WHERE batch = :batch")],
        file_local: true,
    };
    let mut batches = 0;
    let mut demand = |_: &[Document]| {
        batches += 1;
        Ok(Some(PackedDemand::new(ExecutionBackend::CpuJit, Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()).unwrap()))
    };
    let limits = Limits { chunk_bytes: 1, ..Limits::default() };
    run(&lease, &store, &program, "/w", docs.clone(), &limits, &mut demand).unwrap();
    assert_eq!(batches, 2);
    assert_eq!(count(&lease, "SELECT count(*) FROM seen WHERE c0 = 1"), 1);
    drop(lease);
    let lease = scope.pool().take().unwrap();
    let err = run(&lease, &store, &program, "/w", docs, &limits, &mut none).unwrap_err();
    assert!(err.to_string().contains("without a batch"), "{err}");
}
