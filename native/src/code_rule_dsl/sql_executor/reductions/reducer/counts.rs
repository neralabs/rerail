//! Count and existence summaries use one SQL transition and immutable native roots.
use super::*;

pub(super) fn eligible(r: &Reducer) -> bool {
    matches!(r.kind, Kind::Count | Kind::Matches(_))
}

pub(super) fn apply(r: &Reducer, w: &mut Work<'_>, delta: &Delta) -> Result<Option<(String, u64)>> {
    let mut phase = phase::Phase::default();
    let staged = append(r, w, delta, &mut phase)?;
    let _context = w.profile_scope(||json!({"stage":"count and existence summary transition","relation":r.relation,
        "summary_policy":"Signed input projections, support counts, and changed lookup keys share one SQL statement"}));
    let counts = phase.execute(w)?;
    let count = counts[&staged.touched].rows as u64;
    Ok((count > 0).then_some((staged.changed, count)))
}

pub(super) fn append(r: &Reducer, w: &mut Work<'_>, delta: &Delta, phase: &mut phase::Phase) -> Result<super::staged::Summary> {
    let store = w.versions.unwrap();
    let k = r.keys();
    let width = r.width();
    let cols = names("", k);
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
    let weights = phase.relation(
        k + 1,
        &format!("SELECT {cols},sum(c{width}) FROM {signed} GROUP BY {cols}"),
    );
    let total = format!("rr_nonnegative_count(coalesce(g.n,0)+d.c{k})");
    let next = phase.relation(
        k + 2,
        &format!(
            "SELECT {},{total},{total} FROM {weights} d LEFT JOIN {} g ON {}",
            names("d", k),
            quote(&r.groups),
            equal("g", "d", k)
        ),
    );
    let differs = if matches!(r.kind, Kind::Matches(_)) {
        format!("(coalesce(g.value,0)>0)<>(n.c{}>0)", k + 1)
    } else {
        format!("coalesce(g.value,0) IS NOT n.c{}", k + 1)
    };
    let changed = phase.relation(
        k,
        &format!(
            "SELECT {} FROM {next} n LEFT JOIN {} g ON {} WHERE {differs}",
            names("n", k),
            quote(&r.groups),
            equal("g", "n", k)
        ),
    );
    let active = phase.view(k + 2, &format!("SELECT * FROM {next} WHERE c{k}>0"));
    phase.patch_computed(store, &r.groups, &keys, &active)?;
    let touched = w.output_table(k)?;
    let output = w.output_table(k)?;
    phase.load_empty(store, &touched, &keys)?;
    phase.load_empty(store, &output, &changed)?;
    Ok(super::staged::Summary { keys, active, changed_query: changed, touched, changed: output, values:None })
}
