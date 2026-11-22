//! The first slice's entry points (06 S6): `audit` and `apply`, the path from a scope's root tree
//! to its finding keys. Both push the root through the families (S3), run the wave over the
//! documents (S4) and the findings layer over its matches (S5); cold and warm are one path.
//!
//! A warm run evaluates only the documents whose core facts changed since the run before. The
//! others' match facts are kept in the store (`slice_match`, `slice_span`: kept facts of the
//! scope's space, `slice::kept`) and read where they lie as the wave's relations
//! (`Wave::kept_program`), so its families are of every document
//! and an edit's unchanged families cut off. The layer then runs over every document: its facts
//! are the documents' comments and lines, and its families cut off an unchanged finding set.
//!
//! Hosts call it through `napi::CodeRuleSlice` (S6's napi method, after S7's gate passed).

use crate::code_rule_dsl::prof;
use crate::code_rule_dsl::rule_api::CompiledCodeRuleDsl;
use crate::code_rule_dsl::facts::families::{Families, Ingested};
use crate::code_rule_dsl::facts::observe::tree_name;
use crate::code_rule_dsl::facts::vtab::syntax::Document;
use crate::code_rule_dsl::findings::contract::Findings;
use crate::code_rule_dsl::transducer::packed::ExecutionBackend;
use crate::code_rule_dsl::waves::findings::{Layer, Shape};
use crate::code_rule_dsl::waves::matches::{self, Clause, Wave};
use crate::code_rule_dsl::waves::owner::{Backing, Lease};
use crate::code_rule_dsl::waves::schedule::{self, Limits};
use crate::content_store::{ContentStore, DerivationInput, HashKey, StoreResult};
use std::collections::{BTreeMap, BTreeSet};

/// Waves a push may take (S3's tape; core and prefilter; kinds, with room for readers).
const PUSH_WAVES: u32 = 8;

/// The tables a store kept the last run per scope in before they were kept facts (`slice::kept`, t33 step 3).
const BEFORE: &str = "
DROP TABLE IF EXISTS slice_rules; DROP TABLE IF EXISTS slice_document;
DROP TABLE IF EXISTS slice_match; DROP TABLE IF EXISTS slice_span;
";

/// The rules the slice runs: the lowered Python `ast` clauses of a pack's policies, as one wave,
/// with the findings layer of their violation shapes; and the SQL owner kept between runs.
pub struct Rules {
    wave: Wave,
    layer: Layer,
    backing: Backing,
    /// The clauses' identity: kept rows are of these clauses, in this order.
    digest: [u8; 32],
}

impl Rules {
    /// The clauses of `policies` (each by its file stem) that lower (S1), with their shapes.
    pub fn lower(policies: &[(String, CompiledCodeRuleDsl)], backend: ExecutionBackend) -> StoreResult<Self> {
        Self::lower_with(policies, &crate::code_rule_dsl::waves::policies::nothing()?, backend)
    }

    /// `lower`, with a program in Rerail logic beside the policies (`policies::lower_with`).
    pub(in crate::code_rule_dsl) fn lower_with(
        policies: &[(String, CompiledCodeRuleDsl)],
        authored: &std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>,
        backend: ExecutionBackend,
    ) -> StoreResult<Self> {
        let lowered = crate::code_rule_dsl::waves::policies::lower_with(policies, authored)?;
        let layer = Layer::new(lowered.shapes)?;
        let wave = Wave::with_logic(&lowered.clauses, lowered.logic, backend)?;
        Ok(Self { wave, digest: layer.digest(), layer, backing: Backing::default() })
    }

    /// The lowered rules as `rir` (list step 18): the wave's programs and label tables, with
    /// `fused` its packs and groups, with `setplan` its set plan.
    pub fn dump(&self, fused: bool, setplan: bool) -> StoreResult<String> {
        self.wave.dump(fused, setplan)
    }

    /// How many clauses lowered.
    pub fn clauses(&self) -> usize {
        self.layer.shapes().len()
    }
}

/// `policies` lowered and dumped as `rir` (list step 18, `rerail engine dump --rule …`): the
/// wave of their clauses, or of the one named `clause` (its `policy/clause` name, or its clause
/// id), with `fused` and `setplan` as `Wave::dump`'s. No store: a dump reads only the rules.
pub fn dump(policies: &[(String, CompiledCodeRuleDsl)], clause: Option<&str>, fused: bool, setplan: bool) -> StoreResult<String> {
    let (mut clauses, _) = lowered(policies)?;
    if let Some(name) = clause {
        clauses.retain(|clause| clause.name == name || clause.name.rsplit('/').next() == Some(name));
        if clauses.is_empty() {
            return Err(crate::content_store::StoreError::Invalid(format!("no lowered clause `{name}`")));
        }
    }
    Wave::new(&clauses, ExecutionBackend::CpuInterpreter)?.dump(fused, setplan)
}

/// The Python `forbid` clauses of `policies` whose `ast` matcher lowers, with each one's shape,
/// as the policies' logic program has them (`waves::policies`, list step 21).
pub(in crate::code_rule_dsl) fn lowered(policies: &[(String, CompiledCodeRuleDsl)]) -> StoreResult<(Vec<Clause>, Vec<Shape>)> {
    let lowered = crate::code_rule_dsl::waves::policies::lower(policies)?;
    Ok((lowered.clauses, lowered.shapes))
}

/// What one run did and found.
pub struct Run {
    pub findings: Findings,
    /// Each document's path, by its key.
    pub paths: BTreeMap<u32, String>,
    /// The push: names rebound, derivations re-run and created.
    pub ingested: Ingested,
    /// The documents the wave evaluated, by path.
    pub evaluated: Vec<String>,
    /// The documents whose rows came from the content memo instead (`memo`), by path.
    pub reused: Vec<String>,
    /// The wave's families whose digest changed, and the layer's.
    pub matches: Vec<String>,
    pub changed: Vec<String>,
}

/// A cold audit of `scope` at `root`: every document of the root evaluated, whatever was kept.
pub fn audit(store: &ContentStore, scope: &str, root: HashKey, rules: &mut Rules) -> StoreResult<Run> {
    run(store, scope, root, rules, true)
}

/// `scope` after edits, as the root of the edited snapshot (S3: worktree edits reach the slice
/// through a directory snapshot): the documents whose core facts changed since the last run
/// evaluated, the others' matches kept. Cold where the scope has no run of these rules.
pub fn apply(store: &ContentStore, scope: &str, root: HashKey, rules: &mut Rules) -> StoreResult<Run> {
    run(store, scope, root, rules, false)
}

/// One full execution; what it left resident drops two minutes after the last one ends (PIAT
/// §20.10, decision 0032).
fn run(store: &ContentStore, scope: &str, root: HashKey, rules: &mut Rules, cold: bool) -> StoreResult<Run> {
    let run = execute(store, scope, root, rules, cold);
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    crate::code_rule_dsl::residency::slabs::executed();
    run
}

fn execute(store: &ContentStore, scope: &str, root: HashKey, rules: &mut Rules, cold: bool) -> StoreResult<Run> {
    let entered = rules.backing.enter(store);
    let lease = entered.pool().take()?;
    lease.execute_batch(BEFORE)?;
    let space = kept::space(scope);
    let last = kept::last(store, &space)?;
    // Cold: what was kept is of other rules, or not asked for; none of it is read, and all of it is written again.
    let cold = cold || last.rules != Some(rules.digest);
    // The paths that may have changed: every path of the root when cold, else the observer's.
    let old = if cold { None } else { store.binding(&tree_name(scope, ""))? };
    prof::peak("0.start");
    // The push writes through the overlay, its writer behind it on its own thread (decision 0024
    // §5): bindings, derivations and prefilter rows reach SQLite in order, and all of them before
    // the push ends, so the wave reads SQLite whole.
    let limits = crate::code_rule_dsl::facts::overlay::Limits {
        top: 1 << 20,
        resident: 64 << 20,
        behind: 8,
        spill: store.database_path().with_extension("overlay"),
        background: true,
    };
    let mut families = Families::default();
    // A push that did not end (a crash, its writes behind the overlay's writer) is finished first
    // (open-items O1): its invalid derivations re-run and propagate, then the marker clears.
    prof::time("slice.recover", || {
        crate::code_rule_dsl::facts::observe::recover(store, PUSH_WAVES, |frontier| families.recompute(store, frontier))
    })?;
    let ingested = prof::time("slice.push", || {
        crate::code_rule_dsl::facts::overlay::Overlay::scoped(store, limits, |overlay| {
            families.push(overlay, scope, root, PUSH_WAVES, |_| Ok(Vec::new()))
        })
    })?;
    let changes = store.tree_changes_since(old, root)?;
    let kept = if cold { Default::default() } else { last.documents.clone() };
    let paths = kept
        .keys()
        .cloned()
        .chain(changes.changes.into_iter().filter(|change| !change.tree).map(|change| change.path))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let documents = matches::documents(store, scope, &paths)?;
    let changed = documents.iter().filter(|d| kept.get(&d.path).is_none_or(|(_, digest)| *digest != d.digest)).cloned().collect::<Vec<_>>();
    // Of those, the ones whose blob the content memo knows under these rules take its rows.
    let mut evaluate = Vec::with_capacity(changed.len());
    let mut reused = Vec::new();
    let entries = if changed.is_empty() { None } else { memo::open(store)? };
    for document in changed {
        match prof::time("slice.memo_lookup", || memo::lookup(store, entries.as_ref(), scope, &rules.digest, &document))? {
            Some(rows) => reused.push((document, rows)),
            None => evaluate.push(document),
        }
    }
    // Rows to drop from the kept ones: of documents evaluated again, or gone.
    let current = documents.iter().map(|d| d.path.as_str()).collect::<BTreeSet<_>>();
    let mut fresh = evaluate.iter().chain(reused.iter().map(|(d, _)| d)).map(|d| i64::from(d.key)).collect::<Vec<_>>();
    fresh.extend(kept.iter().filter(|(path, _)| !current.contains(path.as_str())).map(|(_, (d, _))| *d));
    prof::time("slice.keep", || -> StoreResult<()> {
        kept::views(&lease, &space, &fresh, cold)?;
        for (document, rows) in &reused {
            memo::keep(&lease, document.key, rows, kept::REUSED_MATCH, kept::REUSED_SPAN)?;
        }
        Ok(())
    })?;

    let program = rules.wave.kept_program(store)?;
    let mut inputs: Vec<DerivationInput> = documents.iter().map(|d| DerivationInput { name: Some(d.core.clone()), digest: d.digest }).collect();
    // The demand statement's `py.*` families are facts the wave reads (decision 0029).
    if rules.wave.demand_reads_py() {
        inputs.extend(crate::code_rule_dsl::facts::families::derived_inputs(store, scope, &documents)?);
    }
    // A file-local program without recursion: one wave per stratum and chunk.
    let limits = Limits { max_waves: program.strata(), ..Limits::default() };
    let wave = &mut rules.wave;
    let evaluated = evaluate.iter().map(|d| d.path.clone()).collect();
    let memoised = evaluate.clone();
    let outcome = prof::time("slice.wave", || schedule::batches::run_over(&lease, store, &program, scope, evaluate, inputs, &limits, &mut |chunk| wave.demand(&lease, store, scope, chunk)))?;
    prof::time("slice.remember", || -> StoreResult<()> {
        wave.statistics(&lease, scope)?;
        memo::remember(&lease, store, scope, &rules.digest, &memoised)?;
        kept::remember(&lease, store, space, &rules.digest, &documents, &last, &fresh, cold)
    })?;

    crate::code_rule_dsl::prof::release();

    prof::peak("4.wave");
    let emitted = rules.layer.program(store)?;
    let (findings, layer) = prof::time("slice.layer", || rules.layer.findings(&lease, store, scope, &documents, &emitted))?;
    prof::peak("5.layer");
    Ok(Run {
        findings,
        paths: documents.iter().map(|d| (d.key, d.path.clone())).collect(),
        ingested,
        evaluated,
        reused: reused.into_iter().map(|(d, _)| d.path).collect(),
        matches: outcome.changed,
        changed: layer.changed,
    })
}

mod kept;
mod memo;
// The slice tests' fixtures: this crate's tests and the engine's napi test (`slice/napi_tests.rs`) read them.
#[cfg(any(test, feature = "test-hooks"))]
pub mod fixtures;
// `napi` (the host's entry, `napi::CodeRuleSlice`) is the engine's: it mounts it under this module (`code_rule_dsl/mod.rs`).
#[cfg(test)]
mod tests;
