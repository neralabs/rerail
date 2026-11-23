//! Registered aggregate, merge, and widening functions for SQL lattice folds: the native lattices' algebra (`Lattice`)
//! as SQL functions. (Their transfers' tops in the SQL a plan emits are the planner's, `plan::sql_kernel`.)
use crate::code_rule_dsl::logic::native::aggregation::Reduction;
use napi::Result;
use std::collections::BTreeMap;

/// A lattice's operations as a wave program's fold runs them (`waves::schedule::lattice`, which re-exports it): its
/// aggregate, merge and widening functions by SQL name, the changes after which it widens, and its top.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Operations {
    /// Planner-owned proof. External SQL callbacks cannot claim builtin laws.
    #[serde(skip)]
    pub reduction: crate::code_rule_dsl::logic::native::aggregation::Reduction,
    pub aggregate: String,
    pub merge: String,
    pub widen: Option<String>,
    pub after: Option<u32>,
    pub top: i64,
}
/// The name the wave schedule gives it (`waves::schedule::LatticeOperations`).
pub use Operations as LatticeOperations;
fn error(message: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(message.to_string())
}
fn identifier(name: &str) -> bool {
    let mut b = name.bytes();
    b.next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && b.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}
use rusqlite::functions::FunctionFlags;

pub fn builtins(db: &rusqlite::Connection) -> Result<()> {
    let det = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
    let top = crate::code_rule_dsl::logic::native::TOP;
    db.create_scalar_function(c"rr_sql_min_widen", 2, det, move |ctx| {
        let (old, new): (i64, i64) = (ctx.get(0)?, ctx.get(1)?);
        Ok(if new < old { -top } else { new })
    })
    .map_err(error)?;
    db.create_scalar_function(c"rr_sql_max_widen", 2, det, move |ctx| {
        let (old, new): (i64, i64) = (ctx.get(0)?, ctx.get(1)?);
        Ok(if new > old { top } else { new })
    })
    .map_err(error)?;
    Ok(())
}

pub fn specification(name: &str) -> Option<LatticeOperations> {
    let top = crate::code_rule_dsl::logic::native::TOP;
    match name {
        "min" | "max" => Some(LatticeOperations {
            reduction: if name == "min" {
                Reduction::Min
            } else {
                Reduction::Max
            },
            aggregate: name.into(),
            merge: name.into(),
            widen: Some(format!("rr_sql_{name}_widen")),
            after: None,
            top: if name == "min" { -top } else { top },
        }),
        _ => None,
    }
}

/// Expose a registered native lattice's algebra as SQL aggregate/merge/widen functions.
/// Both SQL entry points use these operations; only their connection setup differs.
pub fn registered(
    db: &rusqlite::Connection,
    name: &str,
) -> Result<LatticeOperations> {
    use crate::code_rule_dsl::logic::native::{self, Lattice};
    use rusqlite::functions::{Aggregate, Context};
    use std::{ffi::CString, sync::Arc};
    struct Reduce(Arc<dyn Lattice>);
    impl Aggregate<Option<i64>, Option<i64>> for Reduce {
        fn init(&self, _: &mut Context<'_>) -> rusqlite::Result<Option<i64>> {
            Ok(None)
        }
        fn step(&self, c: &mut Context<'_>, v: &mut Option<i64>) -> rusqlite::Result<()> {
            let x = c.get(0)?;
            *v = Some(v.map_or(x, |old| self.0.lub(old, x)));
            Ok(())
        }
        fn finalize(
            &self,
            _: &mut Context<'_>,
            v: Option<Option<i64>>,
        ) -> rusqlite::Result<Option<i64>> {
            Ok(v.flatten())
        }
    }
    if let Some(ops) = specification(name) {
        builtins(db)?;
        return Ok(ops);
    }
    if !identifier(name) {
        return Err(error("lattice names must be identifiers"));
    }
    let lattice = native::lattice(name).map_err(error)?;
    let aggregate = format!("rr_sql_reduce_{name}");
    let merge = format!("rr_sql_merge_{name}");
    let widen = format!("rr_sql_widen_{name}");
    let det = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
    db.create_aggregate_function(
        CString::new(aggregate.as_str()).unwrap().as_c_str(),
        1,
        det,
        Reduce(Arc::clone(&lattice)),
    )
    .map_err(error)?;
    let join = Arc::clone(&lattice);
    db.create_scalar_function(
        CString::new(merge.as_str()).unwrap().as_c_str(),
        2,
        det,
        move |c| Ok(join.lub(c.get(0)?, c.get(1)?)),
    )
    .map_err(error)?;
    let widening = Arc::clone(&lattice);
    db.create_scalar_function(
        CString::new(widen.as_str()).unwrap().as_c_str(),
        2,
        det,
        move |c| Ok(widening.widen(c.get(0)?, c.get(1)?)),
    )
    .map_err(error)?;
    Ok(LatticeOperations {
        reduction: lattice.reduction(),
        aggregate,
        merge,
        widen: Some(widen),
        after: None,
        top: lattice.top(),
    })
}

/// Non-null flat values agree exactly when their extrema agree. Empty groups stay absent.
pub fn flat_aggregate(value: &str, top: i64) -> String {
    format!("CASE WHEN min({value})<>max({value}) THEN {top} ELSE min({value}) END")
}

pub fn validate(
    db: &rusqlite::Connection,
    ops: &LatticeOperations,
) -> Result<()> {
    for (function, arity, aggregate) in [
        Some((&ops.aggregate, 1, true)),
        Some((&ops.merge, 2, false)),
        ops.widen.as_ref().map(|f| (f, 2, false)),
    ]
    .into_iter()
    .flatten()
    {
        if !identifier(function) {
            return Err(error("lattice functions must be SQL identifiers"));
        }
        let exists: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM pragma_function_list WHERE name=?1 COLLATE NOCASE AND (narg=?2 OR narg=-1) AND type IN (?3,?4) AND flags & 2048 <> 0)",rusqlite::params![function,arity,if aggregate {"a"} else {"s"},if aggregate {"w"} else {"s"}],|r|r.get(0)).map_err(error)?;
        if !exists {
            return Err(error(format!(
                "lattice function {function}/{arity} must be registered and deterministic"
            )));
        }
    }
    Ok(())
}
