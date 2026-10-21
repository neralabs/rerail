//! Typed set publication only: aggregates never evaluate rules or execute nested SQL.
use super::graph::Graph;
use crate::code_rule_dsl::sql::frames::{BUDGET, FrameStore};
use rusqlite::{
    Connection, Error, Result,
    functions::{Aggregate, Context, FunctionFlags},
    types::Value,
};
use std::{panic::AssertUnwindSafe, sync::Arc};

#[derive(Default)]
struct Buffer {
    rows: Vec<Vec<Value>>,
}
struct Publish {
    frames: AssertUnwindSafe<Arc<FrameStore>>,
    name: String,
}
impl Aggregate<Buffer, String> for Publish {
    fn init(&self, _: &mut Context<'_>) -> Result<Buffer> {
        Ok(Buffer::default())
    }
    fn step(&self, ctx: &mut Context<'_>, acc: &mut Buffer) -> Result<()> {
        if acc.rows.len() >= BUDGET {
            return Err(Error::ModuleError("relation publication budget".into()));
        }
        acc.rows.push(
            (0..ctx.len())
                .map(|i| ctx.get(i))
                .collect::<Result<Vec<Value>>>()?,
        );
        Ok(())
    }
    fn finalize(&self, _: &mut Context<'_>, acc: Option<Buffer>) -> Result<String> {
        self.frames
            .0
            .publish_relation(&self.name, &acc.unwrap_or_default().rows)?;
        Ok(self.name.clone())
    }
}
pub(super) fn register(
    connection: &Connection,
    frames: &Arc<FrameStore>,
    graph: &Graph,
) -> Result<()> {
    for (handle, input) in &graph.handles {
        let columns = &graph.relations[*input].columns;
        let mut declaration = columns
            .iter()
            .map(|c| {
                format!(
                    "{}{}",
                    c.name,
                    if c.name == "k" {
                        " INTEGER PRIMARY KEY"
                    } else if c.integer {
                        " INTEGER"
                    } else {
                        ""
                    }
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        if columns.iter().any(|c| c.name == "file") && columns.iter().any(|c| c.name == "node") {
            declaration.push_str(", INDEX (file,node)");
        }
        if columns.iter().any(|c| c.name == "pk") {
            declaration.push_str(", INDEX (pk)");
        }
        frames.register(connection, &format!("{handle}({declaration})"))?;
        connection.create_aggregate_function(
            format!("publish_{handle}").as_str(),
            -1,
            FunctionFlags::SQLITE_UTF8,
            Publish {
                frames: AssertUnwindSafe(Arc::clone(frames)),
                name: handle.clone(),
            },
        )?;
    }
    connection.create_scalar_function("profile_begin", 1, FunctionFlags::SQLITE_UTF8, |_| Ok(1))?;
    connection.create_scalar_function("profile_end", 2, FunctionFlags::SQLITE_UTF8, |ctx| {
        ctx.get::<String>(1)
    })?;
    let store = AssertUnwindSafe(Arc::clone(frames));
    connection.create_scalar_function(
        "release_buffers",
        2,
        FunctionFlags::SQLITE_UTF8,
        move |ctx| {
            let token: String = ctx.get(0)?;
            let names: String = ctx.get(1)?;
            let names: Vec<String> =
                serde_json::from_str(&names).map_err(|e| Error::ModuleError(e.to_string()))?;
            for name in names {
                store.0.clear(&name);
            }
            Ok(token)
        },
    )
}
