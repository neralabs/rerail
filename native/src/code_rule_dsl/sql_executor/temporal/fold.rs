//! Recompute only changed keys, preserving the cold kernel's merge and widening order.
use super::*;

pub fn append(
    w: &Work<'_>,
    phase: &mut phase::Phase,
    spec: &Relation,
    keys: &str,
    previous: &str,
    previous_counts: Option<&str>,
    output: &str,
    frontier: &str,
    counts: Option<&str>,
    candidates: &str,
    grouping: Option<Grouping>,
    computed: bool,
) -> Result<()> {
    let key = names("", spec.keys);
    if w.profiler.is_some() {
        phase.analyses.push(json!({"operation":"lattice fold","keys":spec.keys,"single_grouped_producer":grouping.is_some(),"aggregate_reused":grouping.is_some_and(|input|spec.grouping().accepts(input)),"reason":if grouping.is_some_and(|input|spec.grouping().accepts(input)){"Matching builtin reduction and prefix keys before merge/widen"}else{"Multiple producers or an opaque reduction require a fold"}}));
    }
    let candidate = if w.preparing {
        phase.view(spec.width, candidates)
    } else {
        phase.relation(spec.width, candidates)
    };
    let next_query = if spec.lattice {
        let value = format!("c{}", spec.width - 1);
        let proposed = if grouping.is_some_and(|input| spec.grouping().accepts(input)) {
            format!("SELECT * FROM {candidate}")
        } else {
            format!(
                "SELECT {key},{} AS {value} FROM {candidate} GROUP BY {key}",
                spec.aggregate(&value)
            )
        };
        let merged = spec.merge(&format!("p.{value}"), &format!("c.{value}"));
        let mut expression = format!(
            "CASE WHEN c.{value} IS NULL THEN p.{value} WHEN p.{value} IS NULL THEN c.{value} ELSE {merged} END"
        );
        if let Some((counts, ops)) = previous_counts
            .zip(spec.ops.as_ref())
            .filter(|(_, o)| o.after.is_some() && o.widen.is_some())
        {
            let widen = ops.widen.as_ref().unwrap();
            let after = ops.after.unwrap();
            expression = format!(
                "CASE WHEN p.{value} IS NOT NULL AND ({expression})<>p.{value} AND coalesce((SELECT q.c{} FROM {} q WHERE {}),0)>={after} THEN {widen}(p.{value},({expression})) ELSE ({expression}) END",
                spec.keys,
                quote(counts),
                same("q", "k", spec.keys)
            );
        }
        format!(
            "SELECT {},{expression} FROM {keys} k LEFT JOIN {} p ON {} LEFT JOIN ({proposed}) c ON {} WHERE p.c0 IS NOT NULL OR c.c0 IS NOT NULL",
            names("k", spec.keys),
            quote(previous),
            same("p", "k", spec.keys),
            same("c", "k", spec.keys)
        )
    } else {
        format!(
            "SELECT p.* FROM {keys} k CROSS JOIN {} p WHERE {} UNION SELECT * FROM {candidate}",
            quote(previous),
            same("p", "k", spec.keys)
        )
    };
    let next = phase.relation(spec.width, &next_query);
    let frontier_query = format!(
        "SELECT n.* FROM {next} n LEFT JOIN {} p ON {} WHERE p.c0 IS NULL OR {}",
        quote(previous),
        same("p", "n", spec.keys),
        if spec.lattice {
            format!("p.c{}<>n.c{}", spec.width - 1, spec.width - 1)
        } else {
            "0".into()
        }
    );
    let frontier_rows = phase.relation(spec.width, &frontier_query);
    let store = w
        .versions
        .ok_or_else(|| error("temporal fold requires native storage"))?;
    if computed {
        phase.patch_computed(store, frontier, keys, &frontier_rows)?;
        phase.patch_computed(store, output, keys, &next)?;
    } else {
        phase.patch(store, frontier, keys, &frontier_rows)?;
        phase.patch(store, output, keys, &next)?;
    }
    if let Some((counts, previous_counts)) = counts.zip(previous_counts) {
        let query = format!(
            "SELECT {},coalesce(p.c{},0)+(f.c0 IS NOT NULL) FROM {keys} k LEFT JOIN {} p ON {} LEFT JOIN {} f ON {} WHERE p.c0 IS NOT NULL OR f.c0 IS NOT NULL",
            names("k", spec.keys),
            spec.keys,
            quote(previous_counts),
            same("p", "k", spec.keys),
            frontier_rows,
            same("f", "k", spec.keys)
        );
        let rows = phase.relation(spec.keys + 1, &query);
        if computed {
            phase.patch_computed(store, counts, keys, &rows)?;
        } else {
            phase.patch(store, counts, keys, &rows)?;
        }
    }
    Ok(())
}
