//! Plan inputs read in one batch from SQL-owned summaries (ast-index-spike S241).
//! These immutable values describe a decision; they are not a second frequency owner.
//! From Data-layer-rule-expressivity's `matcher/anchors/sql/statistics/mod.rs`: the summaries are
//! the content store's (`kind_freq`, `term_node_freq`, `term_kind_freq`), keyed by kind atom.
use crate::code_rule_dsl::matcher::{AnchorPlan, Condition, Required, Requirement};
use crate::code_rule_dsl::prefilter::LANG_STRIDE;
use rusqlite::{Connection, Result, limits::Limit, params_from_iter, types::Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd)]
struct Key(u32, Required);

#[derive(Clone, Copy)]
pub(in crate::code_rule_dsl) struct Estimate {
    pub rows: u64,
    pub uncovered: u64,
}

#[derive(Clone, Default)]
pub(in crate::code_rule_dsl) struct Statistics(Arc<BTreeMap<Key, Estimate>>);

fn patterns(plan: &AnchorPlan, seen: &mut HashSet<usize>, visit: &mut impl FnMut(&[Requirement])) {
    match &plan.condition {
        Condition::Pattern(required) => visit(required),
        Condition::All(plans) | Condition::Any(plans) => {
            for plan in plans {
                patterns(plan, seen, visit);
            }
        }
        Condition::Not(plan) | Condition::Relation { inner: plan, .. } => {
            patterns(plan, seen, visit)
        }
        Condition::Utility(plan) if seen.insert(Arc::as_ptr(plan) as usize) => {
            patterns(plan, seen, visit)
        }
        _ => {}
    }
}

impl Statistics {
    pub fn read<'a>(
        db: &Connection,
        plans: impl IntoIterator<Item = (u32, &'a AnchorPlan)>,
    ) -> Result<Self> {
        let mut requests = BTreeSet::new();
        let mut seen = BTreeMap::<u32, HashSet<usize>>::new();
        for (language, plan) in plans {
            patterns(plan, seen.entry(language).or_default(), &mut |required| {
                requests.extend(required.iter().map(|r| Key(language, r.value.clone())));
            });
        }
        let requests = requests.into_iter().collect::<Vec<_>>();
        let mut values = BTreeMap::new();
        let capacity = (db.limit(Limit::SQLITE_LIMIT_VARIABLE_NUMBER)? as usize / 3).max(1);
        for requests in requests.chunks(capacity) {
            let rows = std::iter::repeat_n("(?,?,?)", requests.len())
                .collect::<Vec<_>>()
                .join(",");
            // ERROR (65535) is the tape's code 0 (`prefilter::kind_code`).
            let sql = format!("WITH wanted(lang,code,term) AS (VALUES {rows})
                SELECT w.lang,w.code,w.term,coalesce(k.n,0),coalesce(v.n,0),
                       max(0,coalesce(k.n,0)-coalesce(c.n,0))
                FROM wanted w LEFT JOIN kind_freq k ON k.kind=w.lang*{LANG_STRIDE}+CASE WHEN w.code=65535 THEN 0 ELSE w.code END
                LEFT JOIN term_node_freq v ON v.lang=w.lang AND v.kind=k.kind AND v.term=w.term
                LEFT JOIN term_kind_freq c ON c.kind=k.kind");
            let parameters = requests.iter().flat_map(|Key(lang, required)| {
                let (kind, term) = match required {
                    Required::Kind(kind) => (*kind, Value::Null),
                    Required::Terminal(kind, term) => (*kind, Value::Text(term.clone())),
                };
                [
                    Value::Integer(i64::from(*lang)),
                    Value::Integer(i64::from(kind)),
                    term,
                ]
            });
            let mut query = db.prepare(&sql)?;
            for row in query.query_map(params_from_iter(parameters), |row| {
                let kind = row.get::<_, u16>(1)?;
                let term = row.get::<_, Option<String>>(2)?;
                let total = row.get::<_, i64>(3)? as u64;
                let uncovered = row.get::<_, i64>(5)? as u64;
                let (required, rows) = match term {
                    Some(term) => (
                        Required::Terminal(kind, term),
                        (row.get::<_, i64>(4)? as u64)
                            .saturating_add(uncovered)
                            .min(total),
                    ),
                    None => (Required::Kind(kind), total),
                };
                Ok((Key(row.get(0)?, required), Estimate { rows, uncovered }))
            })? {
                let (key, estimate) = row?;
                values.insert(key, estimate);
            }
        }
        Ok(Self(Arc::new(values)))
    }

    pub fn choose(&self, language: u32, required: &[Requirement]) -> (usize, Option<Estimate>) {
        required
            .iter()
            .enumerate()
            .map(|(ordinal, r)| {
                (
                    ordinal,
                    self.0.get(&Key(language, r.value.clone())).copied(),
                )
            })
            .min_by_key(|(ordinal, estimate)| (estimate.map_or(u64::MAX, |e| e.rows), *ordinal))
            .expect("nonempty pattern requirements")
    }

    pub fn decisions(&self, language: u32, plans: &[&AnchorPlan]) -> Vec<usize> {
        let mut decisions = Vec::new();
        let mut seen = HashSet::new();
        for plan in plans {
            patterns(plan, &mut seen, &mut |required| {
                if !required.is_empty() {
                    decisions.push(self.choose(language, required).0);
                }
            });
        }
        decisions
    }
}

#[derive(Clone, serde::Serialize)]
pub(in crate::code_rule_dsl) struct Decision {
    pub consumer: usize,
    pub pattern: usize,
    pub requirement: usize,
    pub kind: u16,
    pub literal: Option<String>,
    pub estimated_rows: Option<u64>,
    pub uncovered_rows: Option<u64>,
}

#[cfg(test)]
mod tests;
