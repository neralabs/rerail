//! Update changed positive contexts and reducer groups, then fold signed head support.
use super::*;

pub(super) fn apply(
    state: &Retained,
    w: &mut Work<'_>,
    changes: &BTreeMap<String, Delta>,
) -> Result<(Delta, u64, u64)> {
    let weights = temporary(w, state.width, "weight INTEGER NOT NULL", None)?;
    let (mut contexts, mut groups) = (0u64, 0u64);
    for rule in &state.rules {
        let dirty = w.table(rule.width)?;
        let removed = w.table(rule.width)?;
        for atom in &rule.atoms {
            let Some(delta) = changes.get(&atom.relation).filter(|d| d.deleted > 0) else {
                continue;
            };
            let same = (0..atom.columns.len())
                .map(|i| format!("c.c{}=d.c{i}", atom.offset + i))
                .collect::<Vec<_>>()
                .join(" AND ");
            execute(
                w.db,
                &format!(
                    "INSERT OR IGNORE INTO {removed} SELECT c.* FROM {} d CROSS JOIN {} c WHERE {same}",
                    delta.minus, rule.contexts
                ),
            )?;
        }
        execute(
            w.db,
            &format!("INSERT INTO {dirty} SELECT * FROM {removed}"),
        )?;
        let cols = names("", rule.width);
        execute(
            w.db,
            &format!(
                "DELETE FROM {} WHERE ({cols}) IN (SELECT {cols} FROM {removed})",
                rule.contexts
            ),
        )?;
        for atom in &rule.atoms {
            let Some(delta) = changes.get(&atom.relation).filter(|d| d.inserted > 0) else {
                continue;
            };
            let mut q = rule.context_query.clone();
            q.body.from[atom.at].relation = source(&delta.plus, &atom.columns);
            maintenance::anchor(&mut q, atom.at);
            let added = w.table(rule.width)?;
            execute(
                w.db,
                &format!("INSERT OR IGNORE INTO {added} {}", q.render()),
            )?;
            execute(
                w.db,
                &format!("INSERT OR IGNORE INTO {dirty} SELECT * FROM {added}"),
            )?;
            execute(
                w.db,
                &format!(
                    "INSERT OR IGNORE INTO {} SELECT * FROM {added}",
                    rule.contexts
                ),
            )?;
        }
        for reducer in &rule.reducers {
            if let Some((keys, touched)) = reducer.apply(w, changes)? {
                groups += touched;
                reducer.mark_contexts(w, rule, &keys, &dirty)?;
            }
        }
        contexts +=
            w.db.query_row(&format!("SELECT count(*) FROM {dirty}"), [], |r| {
                r.get::<_, i64>(0)
            })
            .map_err(error)? as u64;
        let heads = (0..state.width)
            .map(|i| format!("c.h{i}"))
            .collect::<Vec<_>>()
            .join(",");
        execute(
            w.db,
            &format!(
                "INSERT INTO {weights} SELECT {heads},-1 FROM {dirty} d CROSS JOIN {} c WHERE {}",
                rule.contributions,
                equal("c", "d", rule.width)
            ),
        )?;
        execute(
            w.db,
            &format!(
                "DELETE FROM {} WHERE ({cols}) IN (SELECT {cols} FROM {dirty})",
                rule.contributions
            ),
        )?;
        let active = w.table(rule.width)?;
        execute(
            w.db,
            &format!(
                "INSERT INTO {active} SELECT d.* FROM {dirty} d WHERE EXISTS(SELECT 1 FROM {} c WHERE {})",
                rule.contexts,
                equal("c", "d", rule.width)
            ),
        )?;
        for reducer in &rule.reducers {
            reducer.refresh_dependencies(w, rule, &dirty, &active)?;
        }
        execute(
            w.db,
            &format!(
                "INSERT INTO {} {}",
                rule.contributions,
                rule.evaluation(&active).render()
            ),
        )?;
        execute(
            w.db,
            &format!(
                "INSERT INTO {weights} SELECT {heads},1 FROM {active} d CROSS JOIN {} c WHERE {}",
                rule.contributions,
                equal("c", "d", rule.width)
            ),
        )?;
    }
    let width = state.width;
    let cols = names("", width);
    let totals = temporary(
        w,
        width,
        "n INTEGER NOT NULL CHECK(n>=0),old_n INTEGER NOT NULL",
        Some(width),
    )?;
    execute(
        w.db,
        &format!(
            "INSERT INTO {totals} SELECT {},coalesce(h.n,0)+d.weight,coalesce(h.n,0) FROM (SELECT {cols},sum(weight) AS weight FROM {weights} GROUP BY {cols} HAVING sum(weight)<>0) d LEFT JOIN {} h ON {}",
            names("d", width),
            state.heads,
            equal("h", "d", width)
        ),
    )?;
    let plus = w.table(width)?;
    let minus = w.table(width)?;
    // Retain raw candidate multiplicities for lattice outputs. Fold only affected keys.
    if state.lattice.is_some() {
        execute(
            w.db,
            &format!(
                "DELETE FROM {} WHERE ({cols}) IN (SELECT {cols} FROM {totals} WHERE n=0)",
                state.heads
            ),
        )?;
        execute(
            w.db,
            &format!(
                "INSERT OR REPLACE INTO {} SELECT {cols},n FROM {totals} WHERE n>0",
                state.heads
            ),
        )?;
        let (delta, touched) = lattice::apply(state, w, &totals)?;
        return Ok((delta, contexts, groups + touched));
    }
    let inserted = execute(
        w.db,
        &format!("INSERT INTO {plus} SELECT {cols} FROM {totals} WHERE old_n=0 AND n>0"),
    )? as u64;
    let deleted = execute(
        w.db,
        &format!("INSERT INTO {minus} SELECT {cols} FROM {totals} WHERE old_n>0 AND n=0"),
    )? as u64;
    execute(
        w.db,
        &format!(
            "DELETE FROM {} WHERE ({cols}) IN (SELECT {cols} FROM {totals} WHERE n=0)",
            state.heads
        ),
    )?;
    execute(
        w.db,
        &format!(
            "INSERT OR REPLACE INTO {} SELECT {cols},n FROM {totals} WHERE n>0",
            state.heads
        ),
    )?;
    execute(
        w.db,
        &format!(
            "DELETE FROM {} WHERE ({cols}) IN (SELECT {cols} FROM {minus})",
            quote(&state.output)
        ),
    )?;
    execute(
        w.db,
        &format!("INSERT INTO {} SELECT * FROM {plus}", quote(&state.output)),
    )?;
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
