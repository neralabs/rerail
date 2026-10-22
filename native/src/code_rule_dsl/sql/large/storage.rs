//! Table buffers only. Control flow and relational predicates live in generated SQL.
use super::*;
use rusqlite::functions::{Aggregate, Context, FunctionFlags};
use std::panic::AssertUnwindSafe;

#[derive(Default)]
struct Rows {
    chunks: Vec<Vec<Value>>,
    len: usize,
}
struct Publish {
    frames: AssertUnwindSafe<Arc<FrameStore>>,
    name: String,
    replace: bool,
    ignore: bool,
    width: usize,
}

#[derive(Default)]
struct Changes {
    columns: Vec<usize>,
    rows: Rows,
}
struct Patch {
    frames: AssertUnwindSafe<Arc<FrameStore>>,
    name: String,
}
impl Aggregate<Changes, i64> for Patch {
    fn init(&self, _: &mut Context<'_>) -> rusqlite::Result<Changes> {
        Ok(Changes::default())
    }
    fn step(&self, ctx: &mut Context<'_>, changes: &mut Changes) -> rusqlite::Result<()> {
        if ctx.len() < 4
            || !ctx.len().is_multiple_of(2)
            || changes.rows.len >= super::super::super::frames::BUDGET
        {
            return Err(rusqlite::Error::ModuleError(
                "program patch width/budget".into(),
            ));
        }
        if changes.rows.len == 0 {
            changes.columns = (2..ctx.len())
                .step_by(2)
                .map(|i| ctx.get::<u32>(i).map(|c| c as usize))
                .collect::<rusqlite::Result<_>>()?;
        }
        let width = 1 + changes.columns.len();
        if ctx.len() != width * 2 {
            return Err(rusqlite::Error::ModuleError(
                "program patch width changed".into(),
            ));
        }
        if changes
            .rows
            .chunks
            .last()
            .is_none_or(|c| c.len() == c.capacity())
        {
            changes
                .rows
                .chunks
                .push(Vec::with_capacity((4096 / width).max(1) * width));
        }
        let chunk = changes.rows.chunks.last_mut().unwrap();
        chunk.push(ctx.get(1)?);
        for i in (3..ctx.len()).step_by(2) {
            chunk.push(ctx.get(i)?);
        }
        changes.rows.len += 1;
        Ok(())
    }
    fn finalize(&self, _: &mut Context<'_>, changes: Option<Changes>) -> rusqlite::Result<i64> {
        let Some(changes) = changes else {
            return Ok(0);
        };
        self.frames
            .0
            .program_patch(&self.name, &changes.columns, changes.rows.chunks)
            .map(|n| n as i64)
    }
}
impl Aggregate<Rows, i64> for Publish {
    fn init(&self, _: &mut Context<'_>) -> rusqlite::Result<Rows> {
        Ok(Rows::default())
    }
    fn step(&self, ctx: &mut Context<'_>, rows: &mut Rows) -> rusqlite::Result<()> {
        // Argument zero is the SQL iteration token, forcing evaluation at every back-edge.
        if rows.len >= super::super::super::frames::BUDGET {
            return Err(rusqlite::Error::ModuleError(
                "program publication budget".into(),
            ));
        }
        if ctx.len() - 1 != self.width {
            return Err(rusqlite::Error::ModuleError(
                "program publication width".into(),
            ));
        }
        // Bound unused capacity independently of relation size. One growing Vec doubles
        // multi-million-value buffers and retains that excess while the frame is published.
        if rows.chunks.last().is_none_or(|c| c.len() == c.capacity()) {
            rows.chunks
                .push(Vec::with_capacity((4096 / self.width).max(1) * self.width));
        }
        let chunk = rows.chunks.last_mut().unwrap();
        for i in 1..ctx.len() {
            chunk.push(ctx.get(i)?);
        }
        rows.len += 1;
        Ok(())
    }
    fn finalize(&self, _: &mut Context<'_>, rows: Option<Rows>) -> rusqlite::Result<i64> {
        self.frames
            .0
            .program_chunks(
                &self.name,
                rows.unwrap_or_default().chunks,
                self.width,
                self.replace,
                self.ignore,
            )
            .map(|n| n as i64)
    }
}
pub(super) fn register(
    db: &Connection,
    frames: &Arc<FrameStore>,
    schemas: &BTreeMap<String, Schema>,
) -> Result<(), SqlError> {
    super::profile::register(db)?;
    let store = AssertUnwindSafe(Arc::clone(frames));
    db.create_scalar_function("p_rows", 2, FunctionFlags::SQLITE_UTF8, move |ctx| {
        let name: String = ctx.get(1)?;
        store
            .0
            .rows(&name)
            .map(|n| n as i64)
            .ok_or_else(|| rusqlite::Error::ModuleError(format!("unknown program relation {name}")))
    })?;
    for (name, schema) in schemas {
        frames.register(db, &schema.declaration)?;
        db.create_aggregate_function(
            format!("p_patch_{name}").as_str(),
            -1,
            FunctionFlags::SQLITE_UTF8,
            Patch {
                frames: AssertUnwindSafe(Arc::clone(frames)),
                name: name.clone(),
            },
        )?;
        for (prefix, replace, ignore) in [
            ("append", false, false),
            ("ignore", false, true),
            ("replace", true, false),
        ] {
            db.create_aggregate_function(
                format!("p_{prefix}_{name}").as_str(),
                -1,
                FunctionFlags::SQLITE_UTF8,
                Publish {
                    frames: AssertUnwindSafe(Arc::clone(frames)),
                    name: name.clone(),
                    replace,
                    ignore,
                    width: schema.columns.len(),
                },
            )?;
        }
    }
    let store = AssertUnwindSafe(Arc::clone(frames));
    db.create_scalar_function("p_clear", 2, FunctionFlags::SQLITE_UTF8, move |ctx| {
        let name: String = ctx.get(1)?;
        if !store.0.clear(&name) {
            return Err(rusqlite::Error::ModuleError(format!(
                "unknown program table {name}"
            )));
        }
        Ok(0i64)
    })?;
    let store = AssertUnwindSafe(Arc::clone(frames));
    db.create_scalar_function("p_release", 2, FunctionFlags::SQLITE_UTF8, move |ctx| {
        let name: String = ctx.get(1)?;
        if !store.0.program_release(&name) {
            return Err(rusqlite::Error::ModuleError(format!(
                "unknown producer {name}"
            )));
        }
        Ok(0i64)
    })?;
    db.create_scalar_function("p_depth", 1, FunctionFlags::SQLITE_UTF8, |ctx| {
        let depth: i64 = ctx.get(0)?;
        if depth > MAX_LEVELS as i64 {
            Err(rusqlite::Error::ModuleError(
                "rerail: recursion deeper than 256 levels".into(),
            ))
        } else {
            Ok(depth)
        }
    })?;
    db.create_scalar_function("p_loop_pass", 3, FunctionFlags::SQLITE_UTF8, |ctx| {
        #[cfg(any(test, feature = "unbounded-work"))]
        {
            let description: String = ctx.get(1)?;
            let steps: i64 = ctx.get(2)?;
            super::super::record_loop(&description, 1, steps as usize);
        }
        #[cfg(not(any(test, feature = "unbounded-work")))]
        let _ = ctx;
        Ok(0i64)
    })?;
    db.create_aggregate_function("p_acyclic", 6, FunctionFlags::SQLITE_UTF8, CheckReads)?;
    Ok(())
}
struct CheckReads;
impl Aggregate<Vec<[i64; 5]>, i64> for CheckReads {
    fn init(&self, _: &mut Context<'_>) -> rusqlite::Result<Vec<[i64; 5]>> {
        Ok(Vec::new())
    }
    fn step(&self, ctx: &mut Context<'_>, rows: &mut Vec<[i64; 5]>) -> rusqlite::Result<()> {
        rows.push([
            ctx.get(1)?,
            ctx.get(2)?,
            ctx.get(3)?,
            ctx.get(4)?,
            ctx.get(5)?,
        ]);
        Ok(())
    }
    fn finalize(&self, _: &mut Context<'_>, rows: Option<Vec<[i64; 5]>>) -> rusqlite::Result<i64> {
        super::super::acyclic_rows(rows.unwrap_or_default())
            .map_err(|e| rusqlite::Error::ModuleError(e.to_string()))?;
        Ok(0)
    }
}
