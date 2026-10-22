//! Set-step diagnostics within a single SQL execution. These callbacks never run SQL.
use super::*;
use rusqlite::functions::FunctionFlags;
use serde_json::json;
use std::{
    io::{BufWriter, Write},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
fn directory() -> Option<std::path::PathBuf> {
    std::env::var_os("RERAIL_SQL_PROFILE_DIR")
        .or_else(|| std::env::var_os("RERAIL_SQL_STEP_TRACE_DIR"))
        .map(Into::into)
}
pub(super) fn definitions(
    phase: usize,
    code: &[Instruction],
    scopes: &[Scope],
) -> Result<(), String> {
    let Some(dir) = directory() else {
        return Ok(());
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let data = code
        .iter()
        .enumerate()
        .map(|(pc, i)| {
            let source = scopes
                .iter()
                .filter(|s| s.start <= pc && pc < s.end)
                .min_by_key(|s| s.end - s.start)
                .map_or_else(
                    || json!({"scope":"program","statement":phase}),
                    |s| json!({"scope":s.kind,"statement":s.statement}),
                );
            json!({"pc":pc,"sql":i.effect,"next":i.next,"source":source})
        })
        .collect::<Vec<_>>();
    std::fs::write(
        dir.join(format!("cte-program-{}-{phase}.json", std::process::id())),
        serde_json::to_vec(&data).unwrap(),
    )
    .map_err(|e| e.to_string())
}
pub(super) fn register(db: &Connection) -> rusqlite::Result<()> {
    static SERIAL: AtomicUsize = AtomicUsize::new(0);
    let writer = directory()
        .map(|dir| {
            std::fs::create_dir_all(&dir)?;
            std::fs::File::create(dir.join(format!(
                "cte-control-{}-{}.jsonl",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            )))
            .map(BufWriter::new)
        })
        .transpose()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let state = Mutex::new((writer, None::<(i64, i64, i64, Started, [u64; 9])>));
    db.create_scalar_function("p_position", 4, FunctionFlags::SQLITE_UTF8, move |ctx| {
        let mut state = state.lock().unwrap();
        if state.0.is_none() {
            return Ok(1i64);
        }
        let phase: i64 = ctx.get(0)?;
        let pc: i64 = ctx.get(1)?;
        let tick: i64 = ctx.get(2)?;
        #[cfg(any(test, feature = "unbounded-work"))]
        let frame = super::super::super::profiling::frame_snapshot();
        #[cfg(not(any(test, feature = "unbounded-work")))]
        let frame = [0u64; 9];
        if let Some((p, c, t, start, before)) = state.1.take() {
            let elapsed = start.elapsed();
            let published: Option<i64> = ctx.get(3)?;
            let counters = std::array::from_fn::<_, 9, _>(|i| frame[i].saturating_sub(before[i]));
            writeln!(
                state.0.as_mut().unwrap(),
                "{}",
                json!({"phase":p,"pc":c,"tick":t,"cpuNs":elapsed.cpu,"wallNs":elapsed.wall,
                    "frame":counters,"publishedRows":published})
            )
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        }
        if pc >= 0 {
            writeln!(
                state.0.as_mut().unwrap(),
                "{}",
                json!({"phase":phase,"pc":pc,"tick":tick,"started":true})
            )
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            state.1 = Some((phase, pc, tick, Started::now(), frame));
        } else {
            state
                .0
                .as_mut()
                .unwrap()
                .flush()
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        }
        Ok(1i64)
    })
}
