//! One set merge for full runs and differential updates. Candidate and result rows are sets.
use rusqlite::Connection;
pub(in crate::code_rule_dsl) fn fold(
    db: &Connection,
    rel: &str,
    arity: usize,
    candidates: &str,
    delta: &str,
) -> rusqlite::Result<usize> {
    let same = (0..arity)
        .map(|i| format!("r.c{i}=n.c{i}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    db.execute_batch(&format!("DELETE FROM {delta}"))?;
    let n=db.execute(&format!("INSERT INTO {delta} SELECT n.* FROM {candidates} n WHERE NOT EXISTS (SELECT 1 FROM {rel} r WHERE {same})"),[])?;
    db.execute_batch(&format!(
        "INSERT INTO {rel} SELECT * FROM {delta}; DELETE FROM {candidates}"
    ))?;
    Ok(n)
}
