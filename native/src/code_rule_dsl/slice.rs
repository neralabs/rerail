//! The first slice's entry points (06 S6): `audit` and `apply`, the path from a scope's root tree
//! to its finding keys. Both push the root through the families (S3), run the wave over the
//! documents (S4) and the findings layer over its matches (S5); cold and warm are one path.
//!
//! A warm run evaluates only the documents whose core facts changed since the run before. The
//! others' match facts are kept in the store (`slice_match`, `slice_span`, per scope) and read
//! back into the wave's relations (`Wave::kept_program`), so its families are of every document
//! and an edit's unchanged families cut off. The layer then runs over every document: its facts
//! are the documents' comments and lines, and its families cut off an unchanged finding set.
//!
//! Tests and benches only: nothing in production calls the slice until its differential gate
//! (S7) passes; a napi method follows that.

use crate::code_rule_dsl::CompiledCodeRuleDsl;
use crate::code_rule_dsl::facts::families::{Families, Ingested};
use crate::code_rule_dsl::facts::observe::tree_name;
use crate::code_rule_dsl::facts::vtab::syntax::Document;
use crate::code_rule_dsl::findings::contract::Findings;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::matcher::match_tree;
use crate::code_rule_dsl::model::{ClauseKind, Query};
use crate::code_rule_dsl::transducer::packed::{ExecutionBackend, LoweredMatcher};
use crate::code_rule_dsl::waves::findings::{Layer, Shape};
use crate::code_rule_dsl::waves::matches::{self, Clause, KEPT_MATCH, KEPT_SPAN, Wave};
use crate::code_rule_dsl::waves::owner::{Backing, Lease};
use crate::code_rule_dsl::waves::schedule::{self, Limits};
use crate::content_store::{ContentStore, DerivationInput, HashKey, StoreResult};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Waves a push may take (S3's tape; core and prefilter; kinds, with room for readers).
const PUSH_WAVES: u32 = 8;

/// The store's tables of the last run per scope: its rules, each document's core digest, and the
/// wave's rows, which a warm run keeps for the documents it does not evaluate.
const KEPT: &str = "
CREATE TABLE IF NOT EXISTS slice_rules(scope TEXT PRIMARY KEY, digest BLOB NOT NULL) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS slice_document(scope TEXT NOT NULL, path TEXT NOT NULL, d INTEGER NOT NULL,
  digest BLOB NOT NULL, PRIMARY KEY (scope, path)) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS slice_match(scope TEXT NOT NULL, c0 INTEGER NOT NULL, c1 INTEGER NOT NULL,
  c2 INTEGER NOT NULL, PRIMARY KEY (scope, c1, c0, c2)) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS slice_span(scope TEXT NOT NULL, c0 INTEGER NOT NULL, c1 INTEGER NOT NULL,
  c2 INTEGER NOT NULL, c3 INTEGER NOT NULL, PRIMARY KEY (scope, c0, c1, c2, c3)) WITHOUT ROWID;
";

/// The rules the slice runs: the lowered Python `ast` clauses of a pack's policies, as one wave,
/// with the findings layer of their violation shapes; and the SQL owner kept between runs.
pub(in crate::code_rule_dsl) struct Rules {
    wave: Wave,
    layer: Layer,
    backing: Backing,
    /// The clauses' identity: kept rows are of these clauses, in this order.
    digest: [u8; 32],
}

impl Rules {
    /// The clauses of `policies` (each by its file stem) that lower (S1), with their shapes.
    pub(in crate::code_rule_dsl) fn lower(policies: &[(String, CompiledCodeRuleDsl)], backend: ExecutionBackend) -> StoreResult<Self> {
        let (clauses, shapes) = lowered(policies);
        let layer = Layer::new(shapes)?;
        Ok(Self { wave: Wave::new(&clauses, backend)?, digest: layer.digest(), layer, backing: Backing::default() })
    }

    /// How many clauses lowered.
    pub(in crate::code_rule_dsl) fn clauses(&self) -> usize {
        self.layer.shapes().len()
    }
}

/// The Python `forbid` clauses of `policies` whose `ast` matcher lowers, with each one's shape.
pub(in crate::code_rule_dsl) fn lowered(policies: &[(String, CompiledCodeRuleDsl)]) -> (Vec<Clause>, Vec<Shape>) {
    let mut clauses = Vec::new();
    let mut shapes = Vec::new();
    for (policy, compiled) in policies {
        for (at, clause) in compiled.inner.clauses.iter().enumerate() {
            let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &clause.kind else { continue };
            for (index, grammar) in matchers.iter().enumerate().filter(|(_, matcher)| matcher.language == GrammarDialect::Python) {
                let Ok(tree) = match_tree(grammar) else { continue };
                let Ok(matcher) = LoweredMatcher::lower(&tree) else { continue };
                clauses.push(Clause { name: format!("{policy}/{}", clause.id), plan: Arc::clone(&grammar.anchors), matcher });
                shapes.push(Shape { rule: Arc::clone(&compiled.inner), clause: at, grammar: index });
            }
        }
    }
    (clauses, shapes)
}

/// What one run did and found.
pub(in crate::code_rule_dsl) struct Run {
    pub findings: Findings,
    /// Each document's path, by its key.
    pub paths: BTreeMap<u32, String>,
    /// The push: names rebound, derivations re-run and created.
    pub ingested: Ingested,
    /// The documents the wave evaluated, by path.
    pub evaluated: Vec<String>,
    /// The wave's families whose digest changed, and the layer's.
    pub matches: Vec<String>,
    pub changed: Vec<String>,
}

/// A cold audit of `scope` at `root`: every document of the root evaluated, whatever was kept.
pub(in crate::code_rule_dsl) fn audit(store: &ContentStore, scope: &str, root: HashKey, rules: &mut Rules) -> StoreResult<Run> {
    run(store, scope, root, rules, true)
}

/// `scope` after edits, as the root of the edited snapshot (S3: worktree edits reach the slice
/// through a directory snapshot): the documents whose core facts changed since the last run
/// evaluated, the others' matches kept. Cold where the scope has no run of these rules.
pub(in crate::code_rule_dsl) fn apply(store: &ContentStore, scope: &str, root: HashKey, rules: &mut Rules) -> StoreResult<Run> {
    run(store, scope, root, rules, false)
}

fn run(store: &ContentStore, scope: &str, root: HashKey, rules: &mut Rules, cold: bool) -> StoreResult<Run> {
    let entered = rules.backing.enter(store);
    let lease = entered.pool().take()?;
    lease.execute_batch(KEPT)?;
    let kept_rules: Option<Vec<u8>> = lease
        .query_row("SELECT digest FROM slice_rules WHERE scope = ?1", [scope], |row| row.get(0))
        .map(Some)
        .or_else(|error| if error == rusqlite::Error::QueryReturnedNoRows { Ok(None) } else { Err(error) })?;
    let cold = cold || kept_rules.as_deref() != Some(&rules.digest[..]);
    if cold {
        for table in ["slice_document", "slice_match", "slice_span"] {
            lease.execute(&format!("DELETE FROM {table} WHERE scope = ?1"), [scope])?;
        }
    }
    // The paths that may have changed: every path of the root when cold, else the observer's.
    let old = if cold { None } else { store.binding(&tree_name(scope, ""))? };
    let ingested = Families::default().push(store, scope, root, PUSH_WAVES, |_| Ok(Vec::new()))?;
    let changes = store.tree_changes_since(old, root)?;
    let kept = kept_documents(&lease, scope)?;
    let paths = kept
        .keys()
        .cloned()
        .chain(changes.changes.into_iter().filter(|change| !change.tree).map(|change| change.path))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let documents = matches::documents(store, scope, &paths)?;
    let evaluate = documents.iter().filter(|d| kept.get(&d.path).is_none_or(|(_, digest)| *digest != d.digest)).cloned().collect::<Vec<_>>();
    // Rows to drop from the kept ones: of documents evaluated again, or gone.
    let current = documents.iter().map(|d| d.path.as_str()).collect::<BTreeSet<_>>();
    let mut fresh = evaluate.iter().map(|d| i64::from(d.key)).collect::<Vec<_>>();
    fresh.extend(kept.iter().filter(|(path, _)| !current.contains(path.as_str())).map(|(_, (d, _))| *d));
    keep(&lease, scope, &fresh)?;

    let program = rules.wave.kept_program(store)?;
    let inputs = documents.iter().map(|d| DerivationInput { name: Some(d.core.clone()), digest: d.digest }).collect();
    let limits = Limits { max_waves: 1, ..Limits::default() };
    let wave = &mut rules.wave;
    let evaluated = evaluate.iter().map(|d| d.path.clone()).collect();
    let outcome = schedule::run_over(&lease, store, &program, scope, evaluate, inputs, &limits, &mut |chunk| wave.demand(store, scope, chunk))?;
    wave.statistics(&lease, scope)?;
    remember(&lease, scope, &rules.digest, &documents)?;

    let emitted = rules.layer.program(store)?;
    let (findings, layer) = rules.layer.findings(&lease, store, scope, &documents, &emitted)?;
    Ok(Run {
        findings,
        paths: documents.iter().map(|d| (d.key, d.path.clone())).collect(),
        ingested,
        evaluated,
        matches: outcome.changed,
        changed: layer.changed,
    })
}

/// Each kept document of `scope`: its key and core digest, by path.
fn kept_documents(lease: &Lease<'_>, scope: &str) -> StoreResult<BTreeMap<String, (i64, [u8; 32])>> {
    let rows = lease
        .prepare("SELECT path, d, digest FROM slice_document WHERE scope = ?1")?
        .query_map([scope], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, Vec<u8>>(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows.into_iter().filter_map(|(path, d, digest)| Some((path, (d, digest.try_into().ok()?)))).collect())
}

/// The wave's kept inputs: the scope's kept rows but those of the documents `fresh`.
fn keep(lease: &Lease<'_>, scope: &str, fresh: &[i64]) -> StoreResult<()> {
    lease.execute_batch(&format!(
        "DROP TABLE IF EXISTS {KEPT_MATCH}; CREATE TEMP TABLE {}(c0 INTEGER, c1 INTEGER, c2 INTEGER);
         DROP TABLE IF EXISTS {KEPT_SPAN}; CREATE TEMP TABLE {}(c0 INTEGER, c1 INTEGER, c2 INTEGER, c3 INTEGER);
         DROP TABLE IF EXISTS temp.slice_fresh; CREATE TEMP TABLE slice_fresh(d INTEGER PRIMARY KEY);",
        unqualified(KEPT_MATCH),
        unqualified(KEPT_SPAN),
    ))?;
    {
        let mut insert = lease.prepare("INSERT OR IGNORE INTO temp.slice_fresh(d) VALUES (?1)")?;
        for d in fresh {
            insert.execute([d])?;
        }
    }
    lease.execute(
        &format!("INSERT INTO {KEPT_MATCH} SELECT c0, c1, c2 FROM slice_match WHERE scope = ?1 AND c1 NOT IN (SELECT d FROM temp.slice_fresh)"),
        [scope],
    )?;
    lease.execute(
        &format!("INSERT INTO {KEPT_SPAN} SELECT c0, c1, c2, c3 FROM slice_span WHERE scope = ?1 AND c0 NOT IN (SELECT d FROM temp.slice_fresh)"),
        [scope],
    )?;
    Ok(())
}

/// The run's rules, documents and wave rows, kept for the next run of `scope`.
fn remember(lease: &Lease<'_>, scope: &str, digest: &[u8; 32], documents: &[Document]) -> StoreResult<()> {
    lease.execute("INSERT OR REPLACE INTO slice_rules(scope, digest) VALUES (?1, ?2)", rusqlite::params![scope, &digest[..]])?;
    for table in ["slice_document", "slice_match", "slice_span"] {
        lease.execute(&format!("DELETE FROM {table} WHERE scope = ?1"), [scope])?;
    }
    {
        let mut insert = lease.prepare("INSERT INTO slice_document(scope, path, d, digest) VALUES (?1, ?2, ?3, ?4)")?;
        for d in documents {
            insert.execute(rusqlite::params![scope, d.path, d.key, &d.digest[..]])?;
        }
    }
    lease.execute("INSERT INTO slice_match SELECT ?1, c0, c1, c2 FROM match", [scope])?;
    lease.execute("INSERT INTO slice_span SELECT ?1, c0, c1, c2, c3 FROM span", [scope])?;
    Ok(())
}

/// A temp table's name without its schema.
fn unqualified(table: &str) -> &str {
    table.strip_prefix("temp.").unwrap_or(table)
}

#[cfg(test)]
mod tests;
