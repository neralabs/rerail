//! Publish compatible scalar consumers in one fenced SQL support phase.
use super::*;
use reducer::combined::publication;
impl Retained {
    pub fn scalar_phase_analysis(&self) -> crate::code_rule_dsl::logic::analysis::Decision {
        use crate::code_rule_dsl::logic::analysis::Decision;
        let reason = if self.lattice.as_ref().is_some_and(|l| l.tree.is_some()) {
            Some("Registered lattice ordering keeps its existing transition boundary")
        } else if self.rules.len() > 32 || self.rules.iter().any(|r| r.width + self.width >= 2000) {
            Some("The combined support phase exceeds its checked SQL size bound")
        } else if self.rules.iter().all(|r| r.reducers.is_empty()) {
            Some("No scalar consumer support to combine")
        } else {
            None
        };
        if let Some(reason) = reason {
            return Decision::new(false, reason);
        }
        if let Some(rule) = self.rules.iter().find(|r| !r.evaluation_order.proven) {
            return rule.evaluation_order.clone();
        }
        Decision::new(
            true,
            "Independent summaries and fenced context evaluation share one SQL support phase; builtin lattice supports fold affected keys before the unchanged outer merge and widening gate",
        )
    }
    pub fn scalar_phase_eligible(&self) -> bool {
        self.lattice.as_ref().is_none_or(|l| l.tree.is_none())
            && self.rules.len() <= 32
            && self.rules.iter().any(|r| !r.reducers.is_empty())
            && self
                .rules
                .iter()
                .all(|r| r.width + self.width < 2000 && r.evaluation_order.proven)
    }
}
pub fn apply(
    state: &Retained,
    w: &mut Work<'_>,
    changes: &BTreeMap<String, Delta>,
) -> Result<(Delta, u64, u64)> {
    let _context = w.profile_scope(|| json!({"logicalAnalysis":state.explain()}));
    let mut summaries = reducer::staged::Batch::default();
    let mut phase = phase::Phase::default();
    let mut extra = BTreeMap::new();
    for rule in state.rules.iter() {
        let mut queries = vec![];
        for reducer in &rule.reducers {
            if let Some(keys) = summaries.append(reducer, w, changes, &mut phase)? {
                queries.push(reducer.affected_contexts(rule, &keys));
            }
        }
        if !queries.is_empty() {
            extra.insert(rule.step, queries);
        }
    }
    if w.profiler.is_some() {
        phase
            .analyses
            .push(json!({"operation":"scalar consumer support","analysis":state.explain()}));
    }
    let mut pending = vec![];
    let (keys, next, difference) =
        fused::append_with(state, w, changes, &mut phase, &extra, &mut pending, &summaries.sources)?;
    let width = state.width;
    let key_width = state.lattice.as_ref().map_or(width, |l| l.keys);
    let (plus_rows, minus_rows) = if let Some(difference) = difference {
        difference
    } else {
        let old = phase.relation(
            width,
            &format!(
                "SELECT r.* FROM {keys} k CROSS JOIN {} r WHERE {}",
                quote(&state.output),
                equal("r", "k", key_width)
            ),
        );
        let plus_rows = phase.relation(
            width,
            &format!(
                "SELECT n.* FROM {next} n WHERE NOT EXISTS(SELECT 1 FROM {old} o WHERE {})",
                equal("n", "o", width)
            ),
        );
        let minus_rows = phase.relation(
            width,
            &format!(
                "SELECT o.* FROM {old} o WHERE NOT EXISTS(SELECT 1 FROM {next} n WHERE {})",
                equal("n", "o", width)
            ),
        );
        (plus_rows, minus_rows)
    };
    let plus = w.output_table(width)?;
    let minus = w.output_table(width)?;
    let store = w.versions.unwrap();
    phase.load_empty(store, &plus, &plus_rows)?;
    phase.load_empty(store, &minus, &minus_rows)?;
    let counts = phase.execute(w)?;
    publication::publish(w, &pending, &counts)?;
    let groups = summaries.finish(w, &counts)?;
    let contexts = state
        .rules
        .iter()
        .map(|r| {
            if let Some(c) = counts.get(&r.contexts) {
                c.keys as u64
            } else {
                pending
                    .iter()
                    .find(|p| p.table == r.contributions)
                    .map_or(0, |p| counts[&p.keys].rows as u64)
            }
        })
        .sum();
    let (inserted, deleted) = (counts[&plus].rows as u64, counts[&minus].rows as u64);
    if store.available(&state.output).is_none() {
        if deleted > 0 {
            execute(
                w.db,
                &format!(
                    "DELETE FROM {} WHERE ({}) IN (SELECT {} FROM {})",
                    quote(&state.output),
                    names("", width),
                    names("", width),
                    quote(&minus)
                ),
            )?;
        }
        if inserted > 0 {
            execute(
                w.db,
                &format!(
                    "INSERT INTO {} SELECT * FROM {}",
                    quote(&state.output),
                    quote(&plus)
                ),
            )?;
        }
    }
    Ok((
        Delta {
            plus,
            minus,
            inserted,
            deleted,
        },
        contexts,
        groups,
    ))
}
