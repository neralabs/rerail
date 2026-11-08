//! Recompute only changed keys, preserving the cold kernel's merge and widening order.
use super::*;

pub(super) fn apply(
    w: &mut Work<'_>,
    spec: &Relation,
    keys: &str,
    previous: &str,
    previous_counts: Option<&str>,
    output: &str,
    frontier: &str,
    counts: Option<&str>,
    candidates: Vec<&str>,
) -> Result<()> {
    let key = names("", spec.keys);
    let candidate = w.table(spec.width)?;
    for table in candidates {
        w.db.execute(&format!("INSERT OR IGNORE INTO {candidate} SELECT c.* FROM {keys} k CROSS JOIN {} c WHERE {}",quote(table),same("c","k",spec.keys)),[]).map_err(error)?;
    }
    let next = w.table(spec.width)?;
    if spec.lattice {
        let value = format!("c{}", spec.width - 1);
        let proposed = format!(
            "SELECT {key},{} AS {value} FROM {candidate} GROUP BY {key}",
            spec.aggregate(&value)
        );
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
        w.db.execute(&format!("INSERT INTO {next} SELECT {},{expression} FROM {keys} k LEFT JOIN {} p ON {} LEFT JOIN ({proposed}) c ON {} WHERE p.c0 IS NOT NULL OR c.c0 IS NOT NULL",names("k",spec.keys),quote(previous),same("p","k",spec.keys),same("c","k",spec.keys)),[]).map_err(error)?;
    } else {
        w.db.execute(
            &format!(
                "INSERT OR IGNORE INTO {next} SELECT p.* FROM {keys} k CROSS JOIN {} p WHERE {}",
                quote(previous),
                same("p", "k", spec.keys)
            ),
            [],
        )
        .map_err(error)?;
        w.db.execute(
            &format!("INSERT OR IGNORE INTO {next} SELECT * FROM {candidate}"),
            [],
        )
        .map_err(error)?;
    }
    w.db.execute(
        &format!(
            "DELETE FROM {} WHERE ({key}) IN(SELECT {key} FROM {keys})",
            quote(frontier)
        ),
        [],
    )
    .map_err(error)?;
    w.db.execute(
        &format!(
            "INSERT INTO {} SELECT n.* FROM {next} n LEFT JOIN {} p ON {} WHERE p.c0 IS NULL OR {}",
            quote(frontier),
            quote(previous),
            same("p", "n", spec.keys),
            if spec.lattice {
                format!("p.c{}<>n.c{}", spec.width - 1, spec.width - 1)
            } else {
                "0".into()
            }
        ),
        [],
    )
    .map_err(error)?;
    w.db.execute(
        &format!(
            "DELETE FROM {} WHERE ({key}) IN(SELECT {key} FROM {keys})",
            quote(output)
        ),
        [],
    )
    .map_err(error)?;
    w.db.execute(
        &format!("INSERT INTO {} SELECT * FROM {next}", quote(output)),
        [],
    )
    .map_err(error)?;
    if let Some((counts, previous_counts)) = counts.zip(previous_counts) {
        w.db.execute(
            &format!(
                "DELETE FROM {} WHERE ({key}) IN(SELECT {key} FROM {keys})",
                quote(counts)
            ),
            [],
        )
        .map_err(error)?;
        w.db.execute(&format!("INSERT INTO {} SELECT {},coalesce(p.c{},0)+(f.c0 IS NOT NULL) FROM {keys} k LEFT JOIN {} p ON {} LEFT JOIN {} f ON {} WHERE p.c0 IS NOT NULL OR f.c0 IS NOT NULL",quote(counts),names("k",spec.keys),spec.keys,quote(previous_counts),same("p","k",spec.keys),quote(frontier),same("f","k",spec.keys)),[]).map_err(error)?;
    }
    Ok(())
}
