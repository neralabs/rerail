//! Preserve the cold SQL wave contract without writable private tables.
//! SQL evaluates each rule and merge. Native roots hold candidate and delta sets.
use super::*;
use maintenance::Work;

pub fn declare(w: &Work<'_>, p: &schedule::Program) -> Result<()> {
    let store = w
        .versions
        .ok_or_else(|| error("native frontiers require storage"))?;
    for (r, width) in &p.relations {
        let keys = p
            .lattices
            .iter()
            .find(|(n, _, _)| n == r)
            .map_or(*width, |(_, k, _)| *k);
        // Opaque producers retain the cold schedule, including generated private projections.
        if r.starts_with("__sql_") && !store.contains(r) {
            store.create_readonly(w.db, r, versions::Schema::integers(*width, keys))?;
        }
        for (name, key) in [(format!("new_{r}"), *width), (format!("delta_{r}"), keys)] {
            store.declare_readonly_empty(w.db, &name, versions::Schema::integers(*width, key))?;
        }
        if p.lattice_operations
            .get(r)
            .is_some_and(|ops| ops.after.is_some())
        {
            let name = format!("__sql_changes_{r}");
            if !store.contains(&name) {
                let mut schema = versions::Schema::integers(keys + 1, keys);
                schema.columns[keys] = "changes".into();
                store.create_readonly(w.db, &name, schema)?;
            }
        }
    }
    Ok(())
}

fn same(a: &str, b: &str, keys: usize) -> String {
    (0..keys)
        .map(|c| format!("{a}.c{c}={b}.c{c}"))
        .collect::<Vec<_>>()
        .join(" AND ")
}
fn names(keys: usize) -> String {
    (0..keys)
        .map(|c| format!("c{c}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn fold(w: &Work<'_>, p: &schedule::Program, r: &str, width: usize) -> Result<usize> {
    let Some((_, keys, top)) = p.lattices.iter().find(|(name, _, _)| name == r) else {
        return w.fold_set(r, width, &format!("new_{r}"), &format!("delta_{r}"));
    };
    let store = w.versions.unwrap();
    let (keys, top) = (*keys, *top);
    let key = names(keys);
    let value = format!("c{}", width - 1);
    let ops = p.lattice_operations.get(r);
    let aggregate=ops.map_or_else(||format!("CASE WHEN count(DISTINCT {value})>1 OR max({value})={top} THEN {top} ELSE min({value}) END"),|ops|format!("{}({value})",ops.aggregate));
    let merge = ops.map_or_else(
        || format!("CASE WHEN r.{value}=j.{value} THEN r.{value} ELSE {top} END"),
        |ops| format!("{}(r.{value},j.{value})", ops.merge),
    );
    let proposed = format!(
        "CASE WHEN r.{value} IS NULL THEN j.{value} WHEN r.{value}={top} THEN {top} ELSE {merge} END"
    );
    let query = format!(
        "SELECT j.{}, {proposed} AS {value} FROM (SELECT {key},{aggregate} AS {value} FROM {} GROUP BY {key}) j LEFT JOIN {} r ON {}",
        key.replace(",", ",j."),
        quote(&format!("new_{r}")),
        quote(r),
        same("r", "j", keys)
    );
    let delta = format!("delta_{r}");
    w.clear(&delta)?;
    w.insert(&delta,&format!("SELECT j.* FROM ({query}) j LEFT JOIN {} r ON {} WHERE r.{value} IS NULL OR r.{value}<>j.{value}",quote(r),same("r","j",keys)),false)?;
    if let Some((after, widen)) = ops.and_then(|ops| Some((ops.after?, ops.widen.as_ref()?))) {
        let counts = format!("__sql_changes_{r}");
        // Keep the original merge, widening, equal-value removal, then counter order.
        let widened = format!(
            "CASE WHEN r.{value} IS NOT NULL AND coalesce(q.changes,0)>={after} THEN {widen}(r.{value},d.{value}) ELSE d.{value} END"
        );
        let query = format!(
            "SELECT d.{}, {widened} AS {value} FROM {} d LEFT JOIN {} r ON {} LEFT JOIN {} q ON {}",
            key.replace(",", ",d."),
            quote(&delta),
            quote(r),
            same("r", "d", keys),
            quote(&counts),
            same("q", "d", keys)
        );
        let mut phase = phase::Phase::default();
        let widened = phase.relation(width, &query);
        let changed=phase.relation(width,&format!("SELECT d.* FROM {widened} d LEFT JOIN {} r ON {} WHERE r.{value} IS NULL OR r.{value}<>d.{value}",quote(r),same("r","d",keys)));
        phase.replace_all(store, &delta, &changed)?;
        let touched = phase.view(keys, &format!("SELECT {key} FROM {changed}"));
        let next = phase.view(
            keys + 1,
            &format!(
                "SELECT d.{},coalesce(q.changes,0)+1 FROM {changed} d LEFT JOIN {} q ON {}",
                key.replace(",", ",d."),
                quote(&counts),
                same("q", "d", keys)
            ),
        );
        phase.patch_computed(store, &counts, &touched, &next)?;
        phase.execute(w)?;
    }
    let n = store.version(&delta).len();
    w.db.execute(
        &format!(
            "INSERT OR REPLACE INTO {} SELECT * FROM {}",
            quote(r),
            quote(&delta)
        ),
        [],
    )
    .map_err(error)?;
    w.clear(&format!("new_{r}"))?;
    Ok(n)
}

pub fn execute(
    w: &mut Work<'_>,
    p: &schedule::Program,
    limit: u32,
    mut before: impl FnMut(&mut Work<'_>, usize) -> Result<Option<String>>,
) -> Result<u32> {
    let mut after = BTreeSet::new();
    for s in &p.steps {
        for producer in &p.steps {
            if s.stratum != producer.stratum && s.reads.contains(&producer.output) {
                after.insert((s.stratum, producer.stratum));
            }
        }
    }
    loop {
        let previous = after.len();
        for (a, b) in after.clone() {
            for (c, d) in after.clone() {
                if b == c && a != d {
                    after.insert((a, d));
                }
            }
        }
        if after.len() == previous {
            break;
        }
    }
    let mut pending = (0..p.steps.len()).collect::<BTreeSet<_>>();
    let mut had = BTreeSet::new();
    let mut wave = 0;
    let mut diagnostics = vec![];
    w.db.execute("DELETE FROM temp.wave_delta", [])
        .map_err(error)?;
    while !pending.is_empty() {
        if wave == limit {
            return Err(error(format!("{}: no fixpoint in {limit} waves", p.name)));
        }
        wave += 1;
        let frontier = pending
            .iter()
            .copied()
            .filter(|&id| {
                !pending
                    .iter()
                    .any(|&other| after.contains(&(p.steps[id].stratum, p.steps[other].stratum)))
            })
            .collect::<Vec<_>>();
        if frontier.is_empty() {
            return Err(error("cold strata have no ready step"));
        }
        for id in &frontier {
            pending.remove(id);
        }
        let mut written = BTreeSet::new();
        for id in frontier {
            let step = &p.steps[id];
            let _context=w.profile_scope(||json!({"stage":"cold rule","executionPhase":"relationalEvaluation","wave":wave,"ruleStep":id,"relation":step.output,"stratum":step.stratum,"frontier_storage":"immutable native roots"}));
            let replacement = before(w, id)?;
            let prefix = format!("INSERT OR IGNORE INTO new_{} ", step.output);
            // This prefix belongs to the emitter. The authored producer SELECT is unchanged.
            let query = replacement
                .as_deref()
                .or_else(|| step.sql.strip_prefix(&prefix))
                .ok_or_else(|| error("cold rule does not have the emitted frontier contract"))?;
            w.insert(&format!("new_{}", step.output), query, true)?;
            written.insert(step.output.clone());
        }
        let mut changed = BTreeSet::new();
        for (r, width) in p
            .relations
            .iter()
            .map(|(r, width)| (r, width))
            .collect::<BTreeMap<_, _>>()
        {
            if !written.contains(r) && !had.contains(r) {
                continue;
            }
            let _context=w.profile_scope(||json!({"stage":"cold merge","executionPhase":"relationalEvaluation","wave":wave,"relation":r,"frontier_storage":"immutable native roots"}));
            let n = fold(w, p, r, *width)?;
            diagnostics.push(vec![
                SqlValue::Integer(wave as i64),
                SqlValue::Text(r.clone()),
                SqlValue::Integer(n as i64),
            ]);
            if n > 0 {
                changed.insert(r.clone());
            }
        }
        for (id, step) in p.steps.iter().enumerate() {
            if step.reads.iter().any(|r| changed.contains(r)) {
                pending.insert(id);
            }
        }
        had = changed;
    }
    plan::insert_many(
        w.db,
        "INSERT INTO temp.wave_delta(wave,rel,rows)",
        diagnostics,
    )
    .map_err(error)?;
    for relations in p.relations.chunks(256) {
        let counts = relations
            .iter()
            .map(|(r, _)| format!("SELECT '{}',count(*) FROM {}", r, quote(r)))
            .collect::<Vec<_>>()
            .join(" UNION ALL ");
        w.db.execute(
            &format!("INSERT OR REPLACE INTO wave_stats(tbl,rows) {counts}"),
            [],
        )
        .map_err(error)?;
    }
    Ok(wave)
}

pub fn execute_existing(w: &mut Work<'_>, p: &schedule::Program, limit: u32) -> Result<u32> {
    if w.versions.is_none_or(|store| {
        p.relations
            .iter()
            .any(|(r, _)| !store.contains(&format!("new_{r}")))
    }) {
        return schedule::execute_existing(w.db, p, limit).map_err(error);
    }
    declare(w, p)?;
    execute(w, p, limit, |_, _| Ok(None))
}
