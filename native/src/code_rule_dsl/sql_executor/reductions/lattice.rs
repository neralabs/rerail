//! Refold supported candidate values at changed lattice keys, including retractions from top.
use super::*;

#[derive(Clone)]
pub(super) struct Lattice {
    pub keys: usize,
    pub top: i64,
    pub aggregate: Option<String>,
}

pub(super) fn apply(state: &Retained, w: &mut Work<'_>, changed: &str) -> Result<(Delta, u64)> {
    let lattice = state.lattice.as_ref().unwrap();
    let k = lattice.keys;
    let keys = w.table(k)?;
    let key_columns = names("", k);
    let touched = execute(
        w.db,
        &format!("INSERT INTO {keys} SELECT DISTINCT {key_columns} FROM {changed}"),
    )? as u64;
    let old = w.table(state.width)?;
    execute(
        w.db,
        &format!(
            "INSERT INTO {old} SELECT r.* FROM {keys} k CROSS JOIN {} r WHERE {}",
            quote(&state.output),
            equal("r", "k", k)
        ),
    )?;
    let next = w.table(state.width)?;
    let value = format!("c{}", state.width - 1);
    let candidates = format!("FROM {} h WHERE {}", state.heads, equal("h", "k", k));
    // The primary key starts with the lattice key, then value. SQLite can find extrema with
    // one indexed probe. Opaque registered joins scan only the affected key's candidates.
    let folded = match &lattice.aggregate {
        Some(aggregate) => format!("(SELECT {aggregate}(h.{value}) {candidates})"),
        None => format!(
            "CASE WHEN (SELECT min(h.{value}) {candidates})=(SELECT max(h.{value}) {candidates}) THEN (SELECT min(h.{value}) {candidates}) ELSE {} END",
            lattice.top
        ),
    };
    execute(
        w.db,
        &format!(
            "INSERT INTO {next} SELECT {},{folded} FROM {keys} k WHERE EXISTS(SELECT 1 {candidates})",
            names("k", k)
        ),
    )?;
    let delta = w.difference(&next, &old, state.width)?;
    execute(
        w.db,
        &format!(
            "DELETE FROM {} WHERE ({key_columns}) IN (SELECT {key_columns} FROM {keys})",
            quote(&state.output)
        ),
    )?;
    execute(
        w.db,
        &format!("INSERT INTO {} SELECT * FROM {next}", quote(&state.output)),
    )?;
    Ok((delta, touched))
}
