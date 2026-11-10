//! One retained bag summary for compatible count/sum/min/max lookups.
use super::*;
use crate::code_rule_dsl::logic::{analysis::scalars, sql::properties};
pub(super) mod phase;
pub(in crate::code_rule_dsl::sql_executor::reductions) mod publication;

#[derive(Clone)]
pub(super) struct Shared {
    pub owner: bool,
    pub joined: bool,
    pub layout: scalars::Layout,
    pub members: Vec<usize>,
}

pub(in crate::code_rule_dsl::sql_executor::reductions) fn plan(
    reducers: &mut [Reducer],
    outer: &BTreeMap<(String, String), usize>,
) -> scalars::Analysis {
    let inputs = reducers
        .iter()
        .map(|r| {
            let alias = &r.projection.body.from[0].alias;
            let columns = r
                .columns
                .iter()
                .enumerate()
                .map(|(i, c)| ((alias.clone(), c.clone()), i))
                .collect();
            let mut predicates: Vec<_> = r
                .projection
                .body
                .predicates
                .iter()
                .map(|e| properties::expression(e, &columns))
                .collect();
            predicates.sort();
            let value = properties::expression(&r.projection.body.columns[r.keys()].0, &columns);
            scalars::Scalar {
                kind: match r.kind {
                    Kind::Count => scalars::Kind::Count,
                    Kind::Sum => scalars::Kind::Sum,
                    Kind::Min => scalars::Kind::Min,
                    Kind::Max => scalars::Kind::Max,
                    Kind::Join => scalars::Kind::OrderedJoin,
                    Kind::Matches(_) => scalars::Kind::Exists,
                },
                source: r.relation.clone(),
                keys: r.projection.body.columns[..r.keys()]
                    .iter()
                    .map(|(e, _)| properties::expression(e, &columns))
                    .collect(),
                predicates,
                lookup: r
                    .outer_keys
                    .iter()
                    .map(|e| properties::expression(e, outer))
                    .collect(),
                non_null: value.non_null(),
                value,
                dependent: r.dynamic()
                    || r.outer_keys
                        .iter()
                        .any(|e| !matches!(e, E::Qualified { .. })),
            }
        })
        .collect();
    let analysis = scalars::analyze(inputs);
    for family in &analysis.families {
        if !family.lookup.proven {
            continue;
        }
        let owner = family.members[0];
        let projection = family
            .members
            .iter()
            .find(|i| !matches!(reducers[**i].kind, Kind::Count))
            .copied()
            .unwrap_or(owner);
        let projection = reducers[projection].projection.clone();
        let groups = reducers[owner].groups.clone();
        let values = reducers[owner].values.clone();
        for &i in &family.members {
            reducers[i].combined = Some(Shared {
                owner: i == owner,
                joined: family.lookup.proven,
                layout: family.layout.clone(),
                members: family.members.clone(),
            });
            reducers[i].groups = groups.clone();
            reducers[i].values = values.clone();
            reducers[i].projection = projection.clone();
        }
    }
    analysis
}

pub(in crate::code_rule_dsl::sql_executor::reductions) fn storage(r: &Reducer) -> (String, String, Query, bool) {
    (r.groups.clone(), r.values.clone(), r.projection.clone(), r.combined.is_some())
}
pub(in crate::code_rule_dsl::sql_executor::reductions) fn cursor_names(r: &mut Reducer, groups: &str, values: &str) {
    r.groups=groups.into(); r.values=values.into();
}
pub(in crate::code_rule_dsl::sql_executor::reductions) fn share(
    r: &mut Reducer, groups: &str, values: &str, projection: &Query,
    layout: &scalars::Layout, members: &[usize], owner: bool, joined: bool, consumers: usize,
) {
    r.combined = Some(Shared { owner, joined, layout: layout.clone(), members: members.to_vec() });
    r.groups = groups.into(); r.values = values.into(); r.projection = projection.clone();
    r.pool_consumers = consumers;
}

pub(super) fn apply(
    r: &Reducer,
    shared: &Shared,
    w: &mut Work<'_>,
    changes: &BTreeMap<String, Delta>,
) -> Result<Option<(String, u64)>> {
    if !shared.owner {
        return Ok(None);
    }
    let Some(delta) = changes
        .get(&r.relation)
        .filter(|d| d.inserted + d.deleted > 0)
    else {
        return Ok(None);
    };
    if r.pool_consumers > 1 {
        if let Some(result) = super::super::pool::recalled(w, &r.groups)? { return Ok(result); }
    }
    let result = apply_once(r, shared, w, delta)?;
    if r.pool_consumers > 1 { super::super::pool::remember(w, &r.groups, &result, r.keys())?; }
    Ok(result)
}
fn apply_once(r: &Reducer, shared: &Shared, w: &mut Work<'_>, delta: &Delta) -> Result<Option<(String,u64)>> {
    if w.versions.is_some() {
        return phase::apply(r, shared, w, delta);
    }
    let _context = w.profile_scope(|| json!({"stage":"combined scalar summaries","relation":r.relation,"scalarSummary":{"members":shared.members,"layout":shared.layout,"reason":"Compatible non-null bag summaries; exact 128-bit sums with deferred public range checks"}}));
    let k = r.keys();
    let cols = names("", k);
    let val = format!("c{k}");
    let signed = temporary(w, r.width(), "weight INTEGER NOT NULL", None)?;
    for (table, sign, count) in [
        (&delta.plus, 1, delta.inserted),
        (&delta.minus, -1, delta.deleted),
    ] {
        if count == 0 {
            continue;
        }
        let mut q = r.projection.clone();
        q.body.from[0].relation = source(table, &r.columns);
        execute(
            w.db,
            &format!(
                "INSERT INTO {signed} SELECT p.*,{sign} FROM ({}) p",
                q.render()
            ),
        )?;
    }
    let keys = w.table(k)?;
    let touched = execute(
        w.db,
        &format!("INSERT INTO {keys} SELECT DISTINCT {cols} FROM {signed}"),
    )? as u64;
    if touched == 0 {
        return Ok(None);
    }
    if shared.layout.min || shared.layout.max {
        r.values_delta(w, &signed)?;
    }
    let sums = if shared.layout.sum {
        let sums = temporary(
            w,
            k,
            "value INTEGER NOT NULL,high INTEGER NOT NULL",
            Some(k),
        )?;
        execute(
            w.db,
            &format!(
                "INSERT INTO {sums} SELECT {cols},rr_sum_low(v,weight,high),rr_sum_high(v,weight,high) FROM (SELECT {cols},{val} AS v,weight,NULL AS high FROM {signed} UNION ALL SELECT {},g.value,1,g.high FROM {keys} k CROSS JOIN {} g WHERE {}) GROUP BY {cols}",
                names("g", k),
                r.groups,
                equal("g", "k", k)
            ),
        )?;
        Some(sums)
    } else {
        None
    };
    let sum = sums.as_ref().map_or("0,0".into(), |s| {
        format!(
            "(SELECT value FROM {s} s WHERE {}),(SELECT high FROM {s} s WHERE {})",
            equal("s", "d", k),
            equal("s", "d", k)
        )
    });
    let extreme = |enabled, direction| {
        if enabled {
            format!(
                "(SELECT v.{val} FROM {} v WHERE {} ORDER BY v.{val} {direction} LIMIT 1)",
                r.values,
                equal("v", "d", k)
            )
        } else {
            "0".into()
        }
    };
    let totals = temporary(
        w,
        k,
        "n INTEGER NOT NULL CHECK(n>=0),value INTEGER,high INTEGER,minimum INTEGER,maximum INTEGER,old_n INTEGER,old_value INTEGER,old_high INTEGER,old_minimum INTEGER,old_maximum INTEGER",
        Some(k),
    )?;
    execute(
        w.db,
        &format!(
            "INSERT INTO {totals} SELECT {},coalesce(g.n,0)+d.n,{sum},{},{},g.n,g.value,g.high,g.minimum,g.maximum FROM (SELECT {cols},sum(weight) n FROM {signed} GROUP BY {cols}) d LEFT JOIN {} g ON {}",
            names("d", k),
            extreme(shared.layout.min, "ASC"),
            extreme(shared.layout.max, "DESC"),
            r.groups,
            equal("g", "d", k)
        ),
    )?;
    let mut differs = vec![];
    if shared.layout.count {
        differs.push("coalesce(old_n,0) IS NOT n");
    }
    if shared.layout.sum {
        differs.push("coalesce(old_value,0) IS NOT value OR coalesce(old_high,0) IS NOT high");
    }
    if shared.layout.min {
        differs.push("old_minimum IS NOT minimum");
    }
    if shared.layout.max {
        differs.push("old_maximum IS NOT maximum");
    }
    let changed = w.table(k)?;
    execute(
        w.db,
        &format!(
            "INSERT INTO {changed} SELECT {cols} FROM {totals} WHERE {}",
            differs.join(" OR ")
        ),
    )?;
    execute(
        w.db,
        &format!(
            "DELETE FROM {} WHERE ({cols}) IN (SELECT {cols} FROM {totals} WHERE n=0)",
            r.groups
        ),
    )?;
    execute(
        w.db,
        &format!(
            "INSERT OR REPLACE INTO {} SELECT {cols},n,value,high,minimum,maximum FROM {totals} WHERE n>0",
            r.groups
        ),
    )?;
    Ok(Some((changed, touched)))
}
