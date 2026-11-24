//! Update scalar bag summaries in SQL, then publish all roots atomically.
use super::*;
use crate::code_rule_dsl::sql_executor::phase::Phase;

pub fn extreme(r: &Reducer, changed: &str, next: &str, function: &str) -> String {
    let k = r.keys();
    let order = if function == "min" { "ASC" } else { "DESC" };
    // Probe the persistent ordered index. Never scan an untouched large value group.
    // All value-key columns are NOT NULL. NOT IN builds one searchable membership
    // set instead of scanning a materialized CTE for each ordered candidate.
    format!(
        "(SELECT {function}(v) FROM (SELECT v FROM (SELECT v.c{k} AS v FROM {} v WHERE {} AND ({}) NOT IN(SELECT {} FROM {changed}) ORDER BY v.c{k} {order} LIMIT 1) UNION ALL SELECT n.c{k} AS v FROM {next} n WHERE {}))",
        quote(&r.values),
        equal("v", "d", k),
        names("v",r.width()), names("",r.width()),
        equal("n", "d", k)
    )
}

pub fn apply(
    r: &Reducer,
    shared: &Shared,
    w: &mut Work<'_>,
    delta: &Delta,
) -> Result<Option<(String, u64)>> {
    let mut phase = Phase::default();
    let mut publications = vec![];
    let staged = append(r, shared, w, delta, &mut phase, &mut publications)?;
    let counts = phase.execute(w)?;
    super::publication::publish(w, &publications, &counts)?;
    let touched = counts[&staged.touched].rows as u64;
    Ok((touched > 0).then_some((staged.changed, touched)))
}

pub fn append(
    r: &Reducer,
    shared: &Shared,
    w: &mut Work<'_>,
    delta: &Delta,
    phase: &mut Phase,
    publications: &mut Vec<super::publication::Publication>,
) -> Result<super::super::staged::Summary> {
    let store = w.versions.unwrap();
    let k = r.keys();
    let width = r.width();
    let cols = names("", k);
    if w.profiler.is_some() {
        phase.analyses.push(json!({"operation":"shared scalar summaries","storage_group":r.groups,"context_consumers":r.pool_consumers,"storage_proof":"Same non-null bag, grouping keys, pure filters and value; each consumer retains its own lookup mapping","members":shared.members,"layout":shared.layout,"extrema_access":"Persistent ordered probes plus changed values","sum_contract":"Exact 128-bit state; 64-bit range checked only at rule lookup"}));
    }
    let mut incoming = vec![];
    for (table, sign, count) in [
        (&delta.plus, 1, delta.inserted),
        (&delta.minus, -1, delta.deleted),
    ] {
        if count == 0 {
            continue;
        }
        let mut q = r.projection.clone();
        q.body.from[0].relation = source(table, &r.columns);
        incoming.push(format!("SELECT p.*,{sign} FROM ({}) p", q.render()));
    }
    let signed = phase.relation(width + 1, &incoming.join(" UNION ALL "));
    let keys = phase.relation(k, &format!("SELECT DISTINCT {cols} FROM {signed}"));
    let value_changes = if shared.layout.min || shared.layout.max {
        let fields = names("", width);
        let delta = phase.relation(
            width + 1,
            &format!("SELECT {fields},sum(c{width}) FROM {signed} GROUP BY {fields}"),
        );
        let totals = phase.relation(width+1,&format!("SELECT {},rr_nonnegative_count(coalesce(v.n,0)+d.c{width}) FROM {delta} d LEFT JOIN {} v ON {}",names("d",width),quote(&r.values),equal("v","d",width)));
        let next = phase.view(
            width + 1,
            &format!("SELECT * FROM {totals} WHERE c{width}>0"),
        );
        super::publication::append(
            w,
            phase,
            publications,
            &r.values,
            &delta,
            &next,
            width,
            width + 1,
        )?;
        Some((delta, next))
    } else {
        None
    };
    let sums = if shared.layout.sum {
        let rows=phase.view(k+3,&format!("SELECT {cols},c{k},c{width},NULL FROM {signed} UNION ALL SELECT {},g.value,1,g.high FROM {keys} k CROSS JOIN {} g WHERE {}",names("g",k),quote(&r.groups),equal("g","k",k)));
        Some(phase.relation(k+2,&format!("SELECT {cols},rr_sum_low(c{k},c{},c{}),rr_sum_high(c{k},c{},c{}) FROM {rows} GROUP BY {cols}",k+1,k+2,k+1,k+2)))
    } else {
        None
    };
    let sums = sums.map_or("0,0".into(), |s| {
        format!(
            "(SELECT s.c{k} FROM {s} s WHERE {}),(SELECT s.c{} FROM {s} s WHERE {})",
            equal("s", "d", k),
            k + 1,
            equal("s", "d", k)
        )
    });
    let min = value_changes
        .as_ref()
        .filter(|_| shared.layout.min)
        .map_or("0".into(), |(d, n)| extreme(r, d, n, "min"));
    let max = value_changes
        .as_ref()
        .filter(|_| shared.layout.max)
        .map_or("0".into(), |(d, n)| extreme(r, d, n, "max"));
    let counts = phase.relation(
        k + 1,
        &format!("SELECT {cols},sum(c{width}) FROM {signed} GROUP BY {cols}"),
    );
    let next=phase.relation(k+5,&format!("SELECT {},rr_nonnegative_count(coalesce(g.n,0)+d.c{k}),{sums},{min},{max} FROM {counts} d LEFT JOIN {} g ON {}",names("d",k),quote(&r.groups),equal("g","d",k)));
    let mut differs = vec![];
    if shared.layout.count {
        differs.push(format!("coalesce(g.n,0) IS NOT n.c{k}"));
    }
    if shared.layout.sum {
        differs.push(format!(
            "coalesce(g.value,0) IS NOT n.c{} OR coalesce(g.high,0) IS NOT n.c{}",
            k + 1,
            k + 2
        ));
    }
    if shared.layout.min {
        differs.push(format!("g.minimum IS NOT n.c{}", k + 3));
    }
    if shared.layout.max {
        differs.push(format!("g.maximum IS NOT n.c{}", k + 4));
    }
    let changed = phase.relation(
        k,
        &format!(
            "SELECT {} FROM {next} n LEFT JOIN {} g ON {} WHERE {}",
            names("n", k),
            quote(&r.groups),
            equal("g", "n", k),
            differs.join(" OR ")
        ),
    );
    let active = phase.view(k + 5, &format!("SELECT * FROM {next} WHERE c{k}>0"));
    super::publication::append(
        w,
        phase,
        publications,
        &r.groups,
        &keys,
        &active,
        k,
        k + 5,
    )?;
    let changed_rows = w.output_table(k)?;
    let touched_rows = w.output_table(k)?;
    phase.load_empty(store, &changed_rows, &changed)?;
    phase.load_empty(store, &touched_rows, &keys)?;
    Ok(super::super::staged::Summary { keys, active, changed_query: changed, touched: touched_rows, changed: changed_rows, values:value_changes })
}
