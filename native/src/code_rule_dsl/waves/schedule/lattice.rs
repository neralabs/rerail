//! SQL reductions and improved-value deltas for registered lattices.
use super::*;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(in crate::code_rule_dsl) struct Operations {
    pub aggregate: String,
    pub merge: String,
    pub widen: Option<String>,
    pub after: Option<u32>,
    pub top: i64,
}

pub(super) fn fold(
    db: &impl SqlOwner,
    rel: &str,
    arity: usize,
    keys: usize,
    ops: &Operations,
) -> rusqlite::Result<usize> {
    let key = (0..keys)
        .map(|i| format!("c{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let v = format!("c{}", arity - 1);
    let on = |a: &str, b: &str| {
        (0..keys)
            .map(|i| format!("{a}.c{i}={b}.c{i}"))
            .collect::<Vec<_>>()
            .join(" AND ")
    };
    let projected = (0..keys)
        .map(|i| format!("j.c{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let joined = format!(
        "SELECT {key}, {}({v}) AS {v} FROM new_{rel} GROUP BY {key}",
        ops.aggregate
    );
    // Absence is bottom. The merge sees two present values only; top remains absorbing.
    let value = format!(
        "CASE WHEN r.{v} IS NULL THEN j.{v} WHEN r.{v}={} THEN {} ELSE {}(r.{v},j.{v}) END",
        ops.top, ops.top, ops.merge
    );
    let merged = format!(
        "SELECT {projected}, {value} AS {v} FROM ({joined}) j LEFT JOIN {rel} r ON {}",
        on("r", "j")
    );
    db.execute_batch(&format!("DELETE FROM delta_{rel}"))?;
    db.execute(&format!("INSERT INTO delta_{rel} SELECT j.* FROM ({merged}) j LEFT JOIN {rel} r ON {} WHERE r.{v} IS NULL OR r.{v}<>j.{v}",on("r","j")),[])?;
    if let (Some(after), Some(widen)) = (ops.after, &ops.widen) {
        let same = on("r", &format!("delta_{rel}"));
        let seen = on("c", &format!("delta_{rel}"));
        db.execute_batch(&format!(
            "UPDATE delta_{rel} SET {v}=(SELECT {widen}(r.{v},delta_{rel}.{v}) FROM {rel} r WHERE {same})
             WHERE EXISTS (SELECT 1 FROM {rel} r WHERE {same}) AND
             EXISTS (SELECT 1 FROM __sql_changes_{rel} c WHERE {seen} AND c.changes>={after});
             DELETE FROM delta_{rel} WHERE EXISTS (SELECT 1 FROM {rel} r WHERE {same} AND r.{v}=delta_{rel}.{v});
             INSERT INTO __sql_changes_{rel} SELECT {key},1 FROM delta_{rel} WHERE true
             ON CONFLICT ({key}) DO UPDATE SET changes=changes+1;"
        ))?;
    }
    let n = db.execute(
        &format!("INSERT OR REPLACE INTO {rel} SELECT * FROM delta_{rel}"),
        [],
    )?;
    db.execute_batch(&format!("DELETE FROM new_{rel}"))?;
    Ok(n)
}
