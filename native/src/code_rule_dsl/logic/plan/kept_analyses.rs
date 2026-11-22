//! The analyses of a version kept by the version's identity (decision 0333 §4; t33 K4): an analysis that has rows
//! (`Analysis::rows`) and that a run computed of a version is kept in the run's kept plans' entry, beside the prepared
//! plans (Q228), and read back by a later process whose version is the same one (`manager::identity`): not computed
//! again. A warm run of an unchanged program computed every whole-program analysis of its parts anew in each process
//! (the base relations' shapes, the well-founded components), a fixed cost of every run.
//!
//! The entry keeps an analysis under its name and its rows' version (`Analysis::VERSION`): a build whose rows mean
//! something else reads none of them. An analysis kept with no rows is kept as its key alone, so it reads back.

use crate::code_rule_dsl::logic::front::Value;
use crate::code_rule_dsl::logic::program_image::Version;
use crate::code_rule_dsl::logic::program_image::manager::{self, Analysis, Row, VersionAnalyses as _};
use std::collections::HashMap;
use std::sync::Arc;

/// An analysis's key: the version's identity, the analysis's name and its rows' version.
pub type Key = ([u8; 32], String, u32);

/// The kept analyses' rows, by key.
pub type ByVersion = HashMap<Key, Arc<Vec<Row>>>;

/// Analysis `A` of `v`: held; else as the run's kept plans' entry keeps it of a version the same as `v`; else
/// computed, and kept for the store where it has rows. Outside a run that keeps plans, the version's analysis.
pub fn kept<A: Analysis>(v: &Arc<Version>) -> Result<Arc<A>, String> {
    if v.analyses().holds::<A>() || !super::run_holds_plans() {
        return manager::analysis::<A>(v);
    }
    let key: Key = (manager::identity(v), A::NAME.to_owned(), A::VERSION);
    if let Some(rows) = super::analysis_stored(&key)
        && let Some(a) = A::of_rows(v, &rows)
    {
        return Ok(v.analyses().hold(Arc::new(a)));
    }
    let a = manager::analysis::<A>(v)?;
    if let Some(rows) = a.rows(v) {
        // A debug build reads it back first: kept rows that read as another value would be a wrong answer later.
        if cfg!(debug_assertions) && A::of_rows(v, &rows).as_ref() != Some(&*a) {
            return Err(format!("analysis {} does not read back from its rows", A::NAME));
        }
        super::analysis_made(key, Arc::new(rows));
    }
    Ok(a)
}

/// The base relations' shapes of `v` (`Shapes::of_version`), kept by the version's identity.
pub fn shapes(v: &Arc<Version>) -> Arc<super::shapes::Shapes> {
    if std::env::var("RERAIL_LAYOUT_ESTIMATES").is_ok_and(|x| x == "off") {
        return Default::default();
    }
    kept::<super::shapes::Shapes>(v).unwrap_or_default()
}

/// `by`'s rows for the kept plans' entry: each the version's identity as four ints, the analysis's name and rows'
/// version, the row's place (-1: the key of an analysis with no rows), then the row.
pub fn store_rows(by: &ByVersion) -> Vec<Row> {
    let mut keys: Vec<&Key> = by.keys().collect();
    keys.sort();
    let mut out = Vec::new();
    for key in keys {
        let (id, name, version) = key;
        let head = || -> Row {
            let mut r: Row = id.chunks(8).map(|c| Value::Int(i64::from_le_bytes(c.try_into().expect("eight bytes")))).collect();
            r.push(Value::Str(name.as_str().into()));
            r.push(Value::Int(i64::from(*version)));
            r
        };
        let rows = &by[key];
        if rows.is_empty() {
            let mut r = head();
            r.push(Value::Int(-1));
            out.push(r);
        }
        for (at, row) in rows.iter().enumerate() {
            let mut r = head();
            r.push(Value::Int(at as i64));
            r.extend(row.iter().cloned());
            out.push(r);
        }
    }
    out
}

/// The kept analyses read back from `rows` (`store_rows`'s); a row that does not read is left out, and so is its
/// analysis.
pub fn of_store_rows(rows: &[Row]) -> ByVersion {
    let mut by: HashMap<Key, Vec<(i64, Row)>> = HashMap::new();
    let mut bad: Vec<Key> = Vec::new();
    for r in rows {
        let [a, b, c, d, Value::Str(name), Value::Int(version), Value::Int(at), rest @ ..] = r.as_slice() else { continue };
        let mut id = [0u8; 32];
        for (k, w) in [a, b, c, d].into_iter().enumerate() {
            let Value::Int(w) = w else { continue };
            id[k * 8..k * 8 + 8].copy_from_slice(&w.to_le_bytes());
        }
        let Ok(version) = u32::try_from(*version) else { continue };
        let key = (id, name.to_string(), version);
        if ![a, b, c, d].iter().all(|w| matches!(w, Value::Int(_))) {
            bad.push(key);
            continue;
        }
        let at_rows = by.entry(key).or_default();
        if *at >= 0 {
            at_rows.push((*at, rest.to_vec()));
        }
    }
    for key in bad {
        by.remove(&key);
    }
    by.into_iter()
        .map(|(key, mut rows)| {
            rows.sort_by_key(|(at, _)| *at);
            (key, Arc::new(rows.into_iter().map(|(_, r)| r).collect()))
        })
        .collect()
}
