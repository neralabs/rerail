//! Statistics-based lowering on the set path (03 §6, architecture point 7; 05 Phase 3 rows 6 and
//! 8): each plan's best anchors gate the files its candidates can be in, so a batch reads postings
//! only where some plan can match, and each plan's set keeps only its gated files.
//!
//! A `Pattern`'s requirements are a conjunction: a candidate's file holds every one, so it holds
//! the one the statistics estimate rarest (`Statistics::choose`). When that is a terminal, the
//! pattern's files are the term's (any kind: a superset) and every file with text the term
//! postings do not cover (`term_coverage.missing`), as the set lowering's `Uncovered`. A kind gates
//! nothing (its files are most of a batch, and its postings are read where needed anyway).
//! `All` intersects its parts' gates, `Any` unions them, relations and utilities gate as their
//! inner plan (the related node is in the same file), and `Always`, `Text` and `Not` gate nothing.
//! A gate is a necessary condition, so a gated set is still a superset of the plan's matches.
//!
//! Each plan's gate is a compiled plan (`stats::plans`): kept under the plan's structural identity
//! with the statistics its choices read, and reused while each stays in its log2 bucket. Each
//! batch's observed selectivity (a plan's candidates over the batch's nodes) is folded into
//! `plan_feedback`.
use crate::code_rule_dsl::matcher::{AnchorPlan, Condition, Direction, Required};
use crate::code_rule_dsl::stats::inputs::Statistics;
use crate::code_rule_dsl::stats::plans::{Compiled, Plans, PlanId};
use rusqlite::Connection;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::{Arc, Mutex};

/// The files a plan's candidates can be in.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum Gate {
    /// Every file.
    All,
    /// Files with a posting of this term, or with uncovered text.
    Term(String),
    And(Vec<Gate>),
    Or(Vec<Gate>),
}

impl Gate {
    fn and(gates: Vec<Gate>) -> Gate {
        let gates = gates.into_iter().filter(|gate| *gate != Gate::All).collect::<Vec<_>>();
        match gates.len() {
            0 => Gate::All,
            1 => gates.into_iter().next().expect("one gate"),
            _ => Gate::And(gates),
        }
    }

    fn or(gates: Vec<Gate>) -> Gate {
        if gates.is_empty() || gates.contains(&Gate::All) {
            return Gate::All;
        }
        if gates.len() == 1 { gates.into_iter().next().expect("one gate") } else { Gate::Or(gates) }
    }

    /// The terms the gate reads.
    fn terms<'a>(&'a self, out: &mut BTreeSet<&'a str>) {
        match self {
            Gate::All => {}
            Gate::Term(term) => {
                out.insert(term);
            }
            Gate::And(gates) | Gate::Or(gates) => gates.iter().for_each(|gate| gate.terms(out)),
        }
    }

    /// The gate's files among the batch's, from each term's files and the uncovered files; None:
    /// every file.
    fn files(&self, term_files: &BTreeMap<&str, BTreeSet<i64>>, uncovered: &BTreeSet<i64>) -> Option<BTreeSet<i64>> {
        match self {
            Gate::All => None,
            Gate::Term(term) => Some(term_files.get(term.as_str()).into_iter().flatten().chain(uncovered).copied().collect()),
            Gate::And(gates) => gates.iter().filter_map(|gate| gate.files(term_files, uncovered)).reduce(|a, b| a.intersection(&b).copied().collect()),
            Gate::Or(gates) => {
                let mut all = BTreeSet::new();
                for gate in gates {
                    all.extend(gate.files(term_files, uncovered)?);
                }
                Some(all)
            }
        }
    }
}

/// `plan`'s gate under `statistics`.
fn gate(plan: &AnchorPlan, lang: u32, statistics: &Statistics, seen: &mut HashSet<usize>) -> Gate {
    match &plan.condition {
        // A root gate (main's `Condition::RootGate`) is every root of its scope away from the file's root: no gate.
        Condition::Always | Condition::Text { .. } | Condition::Not(_) | Condition::RootGate { .. } => Gate::All,
        Condition::Pattern(required) if required.is_empty() => Gate::All,
        Condition::Pattern(required) => match &required[statistics.choose(lang, required).0].value {
            Required::Terminal(_, term) => Gate::Term(term.clone()),
            Required::Kind(_) => Gate::All,
        },
        Condition::All(plans) => Gate::and(plans.iter().map(|plan| gate(plan, lang, statistics, seen)).collect()),
        Condition::Any(plans) => Gate::or(plans.iter().map(|plan| gate(plan, lang, statistics, seen)).collect()),
        Condition::Relation { inner, .. } => gate(inner, lang, statistics, seen),
        // A utility met again on the same path gates nothing: sound, and finite.
        Condition::Utility(inner) if seen.insert(Arc::as_ptr(inner) as usize) => gate(inner, lang, statistics, seen),
        Condition::Utility(_) => Gate::All,
    }
}

/// `plan`'s structural identity: equal for plans that select the same candidates.
fn identity(plan: &AnchorPlan, lang: u32) -> PlanId {
    fn walk(plan: &AnchorPlan, hash: &mut Sha256, seen: &mut HashSet<usize>) {
        let kinds = plan.root_kinds.as_ref().map(|kinds| kinds.iter().copied().collect::<Vec<_>>());
        hash.update(format!("{kinds:?}{}", u8::from(plan.exact)).as_bytes());
        match &plan.condition {
            Condition::Always => hash.update(b"always"),
            Condition::Pattern(required) => {
                hash.update(b"pattern");
                for requirement in required {
                    let value = match &requirement.value {
                        Required::Kind(kind) => format!("k{kind}"),
                        Required::Terminal(kind, term) => format!("t{kind}:{}:{term}", term.len()),
                    };
                    hash.update(format!("{value}{:?};", requirement.parents).as_bytes());
                }
            }
            Condition::Text { regex, .. } => hash.update(format!("text{}:{}", regex.as_str().len(), regex.as_str()).as_bytes()),
            Condition::All(plans) | Condition::Any(plans) => {
                hash.update(if matches!(plan.condition, Condition::All(_)) { b"all".as_slice() } else { b"any" });
                hash.update((plans.len() as u64).to_le_bytes());
                plans.iter().for_each(|plan| walk(plan, hash, seen));
            }
            Condition::Not(inner) => {
                hash.update(b"not");
                walk(inner, hash, seen);
            }
            Condition::Relation { inner, direction, recursive } => {
                let direction = match direction {
                    Direction::Has => "has",
                    Direction::Inside => "inside",
                    Direction::Precedes => "precedes",
                    Direction::Follows => "follows",
                };
                hash.update(format!("relation{direction}{recursive}").as_bytes());
                walk(inner, hash, seen);
            }
            Condition::Utility(inner) if seen.insert(Arc::as_ptr(inner) as usize) => {
                hash.update(b"utility");
                walk(inner, hash, seen);
            }
            Condition::Utility(_) => hash.update(b"again"),
            Condition::RootGate { kind, gated } => {
                hash.update(format!("rootgate{kind}").as_bytes());
                walk(gated, hash, seen);
            }
        }
    }
    let mut hash = Sha256::new();
    hash.update(b"rerail.prefilter-gate/v1\0");
    hash.update(lang.to_le_bytes());
    walk(plan, &mut hash, &mut HashSet::new());
    let structure: [u8; 32] = hash.finalize().into();
    // `compiled_plans` is keyed by `CompiledPlanId`s (list step 20): domain-separated from every
    // other identity, so a gate's key never equals a wave program's.
    *crate::code_rule_dsl::stats::identity::CompiledPlanId::builder().text("rerail.prefilter-gate").bytes(&structure).finish().as_bytes()
}

/// The process's compiled gates, in memory before the store (`stats::plans`' bounded cache).
static PLANS: std::sync::LazyLock<Mutex<Plans>> = std::sync::LazyLock::new(Default::default);

/// What a batch's gates selected.
pub(super) struct Gated {
    /// Each plan's files; None: every file.
    pub plans: Vec<Option<BTreeSet<i64>>>,
    /// The files any plan can match in; None: every file.
    pub any: Option<BTreeSet<i64>>,
    pub ids: Vec<PlanId>,
}

/// The gates of `plans` over the batch's `files`, compiled or reused (one statement reads every
/// statistic; one reads every stored gate's validity).
pub(super) fn select(db: &Connection, lang: u32, plans: &[&AnchorPlan], files: &[i64]) -> rusqlite::Result<Gated> {
    let statistics = Statistics::read(db, plans.iter().map(|plan| (lang, *plan)))?;
    let current = statistics.reads();
    let ids = plans.iter().map(|plan| identity(plan, lang)).collect::<Vec<_>>();
    let mut compiled = PLANS.lock().expect("gate plans mutex poisoned");
    let valid = compiled.valid(db, &ids, &current)?;
    let mut gates = Vec::with_capacity(plans.len());
    for (plan, id) in plans.iter().zip(&ids) {
        let held = valid.get(id).and_then(|held| serde_json::from_slice::<Gate>(&held.program).ok());
        let gate = match held {
            Some(gate) => {
                crate::code_rule_dsl::prof::count("gates.reused", 1);
                gate
            }
            None => {
                let gate = gate(plan, lang, &statistics, &mut HashSet::new());
                let program = serde_json::to_vec(&gate).expect("a gate serialises");
                compiled.put(db, *id, Compiled { decisions: String::new(), program }, &current)?;
                crate::code_rule_dsl::prof::count("gates.compiled", 1);
                gate
            }
        };
        gates.push(gate);
    }
    drop(compiled);
    // Each gate term's files, and the files with uncovered text, among the batch's.
    let batch = files.iter().copied().collect::<BTreeSet<_>>();
    let mut terms = BTreeSet::new();
    gates.iter().for_each(|gate| gate.terms(&mut terms));
    let mut term_files = BTreeMap::new();
    let mut uncovered = BTreeSet::new();
    if !terms.is_empty() {
        let mut query = db.prepare_cached(
            "SELECT DISTINCT p.tape_id FROM atoms a CROSS JOIN term_postings p ON p.lang=?1 AND p.term=a.atom WHERE a.value=?2",
        )?;
        for term in &terms {
            let found = query
                .query_map(rusqlite::params![lang, term], |row| row.get::<_, i64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
                .into_iter()
                .filter(|file| batch.contains(file))
                .collect::<BTreeSet<_>>();
            term_files.insert(*term, found);
        }
        let mut missing = db.prepare_cached("SELECT 1 FROM term_coverage WHERE tape_id=?1 AND length(missing)>0 LIMIT 1")?;
        for &file in &batch {
            if missing.exists([file])? {
                uncovered.insert(file);
            }
        }
    }
    let per_plan = gates.iter().map(|gate| gate.files(&term_files, &uncovered)).collect::<Vec<_>>();
    let any = per_plan.iter().try_fold(BTreeSet::new(), |mut all, files| {
        all.extend(files.as_ref()?);
        Some(all)
    });
    Ok(Gated { plans: per_plan, any, ids })
}

/// Folds each plan's observed selectivity (its candidates over the batch's `nodes`) and cost into
/// `plan_feedback`.
pub(super) fn observe(db: &Connection, gated: &Gated, candidates: &[usize], nodes: u32, backend: &str, ns_per_plan: f64) -> rusqlite::Result<()> {
    if nodes == 0 {
        return Ok(());
    }
    for (id, &found) in gated.ids.iter().zip(candidates) {
        let roots = found.max(1) as f64;
        crate::code_rule_dsl::stats::plans::observe(db, *id, backend, found as f64 / f64::from(nodes), ns_per_plan / roots)?;
    }
    Ok(())
}
