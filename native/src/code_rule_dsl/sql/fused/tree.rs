//! Pure tree metadata for typed recursive SQL. No rule evaluation or nested SQL.
use super::super::exec::BatchSlot;
use rusqlite::{Connection, Error, Result, functions::FunctionFlags};
use std::{panic::AssertUnwindSafe, sync::Arc};

pub(in crate::code_rule_dsl::sql) fn register(
    connection: &Connection,
    slot: &Arc<BatchSlot>,
) -> Result<()> {
    let files = AssertUnwindSafe(Arc::clone(slot));
    connection.create_scalar_function(
        "fused_file_dialect",
        1,
        FunctionFlags::SQLITE_UTF8,
        move |ctx| {
            let file = ctx.get::<u32>(0)? as usize;
            files
                .0
                .current()
                .and_then(|b| b.files.get(file).map(|f| f.dialect.to_string()))
                .ok_or_else(|| Error::ModuleError("dialect input outside batch".into()))
        },
    )?;
    let files = AssertUnwindSafe(Arc::clone(slot));
    connection.create_scalar_function(
        "fused_depth_guard",
        3,
        FunctionFlags::SQLITE_UTF8,
        move |ctx| {
            let file = ctx.get::<u32>(0)? as usize;
            let root = ctx.get::<u32>(1)?;
            let end = ctx.get::<u32>(2)?;
            let batch = files
                .0
                .current()
                .ok_or_else(|| Error::ModuleError("depth input outside batch".into()))?;
            let facts = &batch
                .files
                .get(file)
                .ok_or_else(|| Error::ModuleError("depth file outside batch".into()))?
                .facts;
            if root > end || end >= facts.len() {
                return Err(Error::ModuleError("invalid dependency prefix".into()));
            }
            if facts.dependency_depth(root, end) >= 256 {
                return Err(Error::ModuleError(
                    "rerail: recursion deeper than 256 levels".into(),
                ));
            }
            Ok(1)
        },
    )
}
