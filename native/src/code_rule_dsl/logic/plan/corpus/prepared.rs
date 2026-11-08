//! A program as a run prepares it, kept in the store across processes (decision 0073): pruned, simplified, demanded,
//! lowered, its joins pushed, located and split. It is a fact of the program's text, kept the way a file's rows are
//! (decision 0052): in the facts memo, under the text's stored source, an entry per plan, read by no path. The plan
//! names everything else preparing reads: the engine build, the grammar version, the dialect and what the run added
//! to the program (`prepared_key`).
//!
//! A prepared program holds atoms of the store, which lowering interned. They are the store's own, never given out
//! again (decision 0049), so an entry read from the store is valid under its atoms; `interned` lists every intern
//! call, and the run asks them again before it uses the entry, as it does of one kept in memory (decision 0060). Its
//! text tests are kept as their specs and their matchers made again on reading.

use super::{Prepared, lower};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::logic::front::Program;
use crate::code_rule_dsl::logic::strings::Vocabulary;
use crate::content_store::{ContentStore, FactsMemoKey, FactsPlanSet, FactsResult, SourceId, StoreResult};
use sha2::{Digest as _, Sha256};
use std::sync::Arc;

/// The entry's schema: bumped when what it holds changes meaning (2: `match_all`'s regexes and the f-string markers).
const SCHEMA: u32 = 2;

/// A prepared program as stored: `Prepared` with its text tests as specs, its `match_all` regexes as their sources and
/// its lowered program without the dialect, which the run names.
#[derive(serde::Serialize, serde::Deserialize)]
struct Stored {
    prog: Program,
    tests: Vec<String>,
    specs: Vec<(String, Vec<String>)>,
    matches: Vec<String>,
    markers: Vec<String>,
    lowered: Program,
    domains: lower::Domains,
    ops: [i64; 3],
    sets: Vec<(String, Vec<i64>)>,
    demanded: Vec<String>,
    pushed: Vec<String>,
    located: std::result::Result<Vec<String>, Vec<String>>,
    merged: (usize, usize),
    split: crate::code_rule_dsl::typing::file_local::Programs,
    interned: Vec<(Vec<String>, Vec<u32>)>,
}

/// The path a lookup names. An entry reads no path (`path_dependent: false`), so the store keeps it under none; the
/// plan names the dialect.
const PATH: &str = "rerail/prepared";

/// The plan an entry is made under: the engine build, the grammar version and `key` (the program's texts, the
/// dialect and the run's additions).
fn plan(key: &[u8; 32]) -> FactsPlanSet {
    let mut hash = Sha256::new();
    hash.update(b"rerail.prepared/v1\0");
    hash.update(env!("RERAIL_ENGINE_BUILD_ID").as_bytes());
    hash.update(crate::code_rule_dsl::workspace::grammar_version().to_le_bytes());
    hash.update(key);
    FactsPlanSet::new(vec![hash.finalize().into()])
}

/// The program `key` names, prepared for `dialect`, as an earlier process stored it under `source`; `None`
/// when none did, or when its entry no longer reads (another schema).
pub(super) fn lookup(store: &ContentStore, source: SourceId, dialect: GrammarDialect, key: &[u8; 32]) -> StoreResult<Option<Prepared>> {
    super::memo::off();
    let set = plan(key);
    let memo = store.facts_memo(FactsMemoKey { source, fact_schema: SCHEMA, path: PATH })?;
    let Some(result) = memo.results.get(&set.plans[0]) else { return Ok(None) };
    let Ok(stored) = serde_json::from_slice::<Stored>(&result.entry) else { return Ok(None) };
    let Ok(mut vocabulary) = Vocabulary::from_specs(&stored.tests, stored.specs) else { return Ok(None) };
    let Ok(matches) = stored.matches.iter().map(|re| regex::Regex::new(re).map(Arc::new)).collect::<Result<Vec<_>, _>>() else { return Ok(None) };
    (vocabulary.matches, vocabulary.markers) = (matches, stored.markers);
    Ok(Some(Prepared {
        prog: stored.prog,
        vocabulary: Arc::new(vocabulary),
        lowered: lower::Lowered { program: stored.lowered, domains: stored.domains, ops: stored.ops, dialect },
        sets: stored.sets,
        demanded: stored.demanded,
        pushed: stored.pushed,
        located: stored.located,
        merged: stored.merged,
        split: stored.split,
        interned: stored.interned,
    }))
}

/// Stores `prepared` as the program `key` names under `source`.
pub(super) fn remember(store: &ContentStore, source: SourceId, key: &[u8; 32], prepared: &Prepared) -> StoreResult<()> {
    super::memo::off();
    let stored = Stored {
        prog: prepared.prog.clone(),
        tests: prepared.vocabulary.tests.iter().map(|(rel, _)| rel.clone()).collect(),
        specs: prepared.vocabulary.specs.clone(),
        matches: prepared.vocabulary.matches.iter().map(|re| re.as_str().to_owned()).collect(),
        markers: prepared.vocabulary.markers.clone(),
        lowered: prepared.lowered.program.clone(),
        domains: prepared.lowered.domains.clone(),
        ops: prepared.lowered.ops,
        sets: prepared.sets.clone(),
        demanded: prepared.demanded.clone(),
        pushed: prepared.pushed.clone(),
        located: prepared.located.clone(),
        merged: prepared.merged,
        split: prepared.split.clone(),
        interned: prepared.interned.clone(),
    };
    let entry = serde_json::to_vec(&stored).map_err(|e| crate::content_store::StoreError::Invalid(e.to_string()))?;
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, entry.len() as u64);
    let set = plan(key);
    store.put_facts_memo(FactsMemoKey { source, fact_schema: SCHEMA, path: PATH }, &set, &[FactsResult { plan: set.plans[0], path_dependent: false, entry }], &[])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    /// A program one process prepared is read from the store by the next (decision 0073). With this process's
    /// prepared programs dropped, as a new process starts without them, the second run reads it from the store and
    /// gives the first run's rows. A third run finds it in memory.
    #[test]
    fn a_prepared_program_is_read_from_the_store() {
        // A text no other run prepared, so the first run prepares it: the marker is a constant of the rule, since
        // printing the rule drops its comments.
        let marker = format!("{:?}", std::time::SystemTime::now());
        let rule = format!(
            ".decl calls(c: node, x: symbol)\ncalls(c, x) :- kind(c, \"call\"), text(c, x), x != \"g()\", x != {marker:?}.\n.output calls\n"
        );
        let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("prepared");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.py"), "f(1)\ng()\nh(f(2))\n").unwrap();
        // The rule with the core declarations it reads (`kind`, `text`), as `rerail-engine` reads a rule file.
        let path = dir.join("calls.rr");
        std::fs::write(&path, &rule).unwrap();
        let program = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&path).unwrap()).unwrap();
        std::fs::remove_file(&path).unwrap();
        let run = || {
            let out = dir.join("out");
            let report = super::super::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
            let mut rows: Vec<String> = std::fs::read_to_string(out.join("calls.csv")).unwrap_or_default().lines().map(str::to_owned).collect();
            rows.sort();
            (report.prepared, rows)
        };
        let (from, first) = run();
        assert_eq!(from, "prepared");
        assert_eq!(first.len(), 3, "{first:?}");
        super::super::prepared_cache().lock().unwrap().clear();
        let (from, second) = run();
        assert_eq!(from, "store", "the program is read from the store");
        assert_eq!(second, first);
        let (from, third) = run();
        assert_eq!(from, "memory");
        assert_eq!(third, first);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
