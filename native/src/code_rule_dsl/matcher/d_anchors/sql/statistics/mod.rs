//! Plan inputs read in one batch from SQL-owned summaries (ast-index-spike S241).
//! These immutable values describe a decision; they are not a second frequency owner.
use super::super::{AnchorPlan, Condition, Required, Requirement};
use rusqlite::{Connection, Result, limits::Limit, params_from_iter, types::Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd)]
struct Key(u32, Required);

#[derive(Clone, Copy)]
pub(super) struct Estimate {
    pub rows: u64,
    pub uncovered: u64,
}

#[derive(Clone, Default)]
pub(super) struct Statistics(Arc<BTreeMap<Key, Estimate>>);

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
        #[cfg(feature = "code-rule-profile")]
        let _timer = crate::code_rule_dsl::profile::PhaseTimer::start(
            crate::code_rule_dsl::profile::Phase::Planning,
        );
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
            let sql = format!("WITH wanted(lang,code,term) AS (VALUES {rows})
                SELECT w.lang,w.code,w.term,coalesce(k.n,0),coalesce(v.n,0),
                       max(0,coalesce(k.n,0)-coalesce(c.n,0))
                FROM wanted w LEFT JOIN kfreq k ON k.lang=w.lang AND k.code=CASE WHEN w.code=65535 THEN 0 ELSE w.code END
                LEFT JOIN value_frequency v ON v.lang=w.lang AND v.kind=k.code AND v.term=w.term
                LEFT JOIN value_kind_frequency c ON c.lang=w.lang AND c.kind=k.code");
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
pub(super) struct Decision {
    pub consumer: usize,
    pub pattern: usize,
    pub requirement: usize,
    pub kind: u16,
    pub literal: Option<String>,
    pub estimated_rows: Option<u64>,
    pub uncovered_rows: Option<u64>,
}

/// Legacy stores backfill counts once. The execution path never recounts postings.
pub(super) fn upgrade(db: &Connection) -> Result<()> {
    use rusqlite::functions::FunctionFlags;
    db.create_scalar_function(
        c"__rerail_posting_count",
        1,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        |ctx| {
            let bytes = ctx.get_raw(0).as_blob()?;
            let mut count = 0_i64;
            let (mut value, mut shift, mut position) = (0_u32, 0_u32, 0_u32);
            for &byte in bytes {
                if shift > 28 || (shift == 28 && byte > 15) {
                    return Err(rusqlite::Error::UserFunctionError(
                        "invalid legacy posting".into(),
                    ));
                }
                value |= u32::from(byte & 127) << shift;
                if byte & 128 == 0 {
                    position = position.checked_add(value).ok_or_else(|| {
                        rusqlite::Error::UserFunctionError("legacy posting overflow".into())
                    })?;
                    count += 1;
                    value = 0;
                    shift = 0;
                } else {
                    shift += 7;
                }
            }
            if shift != 0 {
                return Err(rusqlite::Error::UserFunctionError(
                    "truncated legacy posting".into(),
                ));
            }
            Ok(count)
        },
    )?;
    let result = (|| {
        let tx = db.unchecked_transaction()?;
        tx.execute_batch(
            "ALTER TABLE vidx ADD COLUMN n INTEGER NOT NULL DEFAULT 0;
            UPDATE vidx SET n=__rerail_posting_count(postings)",
        )?;
        let (_, schema) = include_str!("../schema.sql")
            .split_once("-- SQL-owned planner statistics.")
            .expect("statistics schema boundary");
        tx.execute_batch(schema)?;
        tx.execute_batch("INSERT INTO value_frequency SELECT lang,term,kind,sum(n) FROM vidx GROUP BY lang,term,kind;
            INSERT INTO value_kind_frequency SELECT lang,kind,sum(n) FROM vidx GROUP BY lang,kind")?;
        tx.pragma_update(None, "user_version", 3_i64)?;
        tx.commit()
    })();
    let removed = db.remove_function(c"__rerail_posting_count", 1);
    result.and(removed)
}

#[cfg(test)]
mod tests;
