//! Test-only replay of emitted SELECT bodies against live frames. No addon switch or compiler
//! alternative. Query output parity is a multiset check, not proof of first-match scheduling.

#[path = "query_measure.rs"]
mod measure;
#[path = "query_variants.rs"]
mod variants;

use super::frames::FrameStore;
use rusqlite::{Connection, Result};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);
static STATE: Mutex<Option<State>> = Mutex::new(None);

struct State {
    directory: PathBuf,
    samples: usize,
    rounds: usize,
    counts: HashMap<&'static str, usize>,
    cases: usize,
    rewrite: Option<HashMap<String, String>>,
    targets: HashSet<String>,
    // Targeted captures retain the largest observed input for each SQL text. Recursive
    // prefixes often make the first few executions too small to represent the hot work.
    seen: HashMap<String, (usize, usize)>,
}

pub(in crate::code_rule_dsl) fn start(
    directory: PathBuf,
    samples: usize,
    rounds: usize,
    targets: Vec<String>,
) {
    std::fs::create_dir_all(&directory).unwrap();
    let mut state = STATE.lock().unwrap();
    assert!(state.is_none(), "one SQL shape experiment at a time");
    *state = Some(State {
        directory,
        samples,
        rounds,
        counts: HashMap::new(),
        cases: 0,
        rewrite: None,
        targets: targets.into_iter().collect(),
        seen: HashMap::new(),
    });
    ENABLED.store(true, Ordering::Release);
}

/// Runs a whole-workspace prototype with one chosen shape per family. Test builds only.
pub(in crate::code_rule_dsl) fn start_rewrite(rewrite: HashMap<String, String>) {
    let mut state = STATE.lock().unwrap();
    assert!(state.is_none());
    *state = Some(State {
        directory: PathBuf::new(),
        samples: 0,
        rounds: 0,
        counts: HashMap::new(),
        cases: 0,
        rewrite: Some(rewrite),
        targets: HashSet::new(),
        seen: HashMap::new(),
    });
    ENABLED.store(true, Ordering::Release);
}

pub(in crate::code_rule_dsl) fn finish() -> Value {
    ENABLED.store(false, Ordering::Release);
    let state = STATE.lock().unwrap().take().unwrap();
    let interpretation = if state.rewrite.is_some() {
        "Whole-workspace prototype metadata. Apply CPU includes rewrite dispatch. Replay, tracing, and EXPLAIN are disabled. See the sweep's paired controls and source provenance."
    } else {
        "Live-query replays use warmed native terminal caches. SELECT output multisets include duplicate rows and every projected capture. SQL preparation, copy/index/analyze setup, and execution are reported separately. These are not whole-workload speedups."
    };
    json!({"cases":state.cases,"families":state.counts,"rounds":state.rounds,"rewrite":state.rewrite,
        "interpretation":interpretation})
}

pub(super) fn before_statement(
    connection: &Connection,
    frames: &FrameStore,
    sql: &mut String,
) -> Result<()> {
    if !ENABLED.load(Ordering::Acquire) {
        return Ok(());
    }
    let Some(case) = variants::classify(sql) else {
        return Ok(());
    };
    // Serialize only diagnostic replays. Normal execution remains on its existing workers.
    let mut guard = STATE.lock().unwrap();
    let Some(state) = guard.as_mut() else {
        return Ok(());
    };
    if let Some(rewrite) = &state.rewrite {
        if let Some(name) = rewrite.get(case.family) {
            let variant = variants::variants(&case)
                .into_iter()
                .find(|v| v.name == name)
                .ok_or_else(|| {
                    rusqlite::Error::ModuleError(format!(
                        "shape {name} unavailable for {}",
                        case.family
                    ))
                })?;
            *sql = case.insert(&variant.sql);
            *state.counts.entry(case.family).or_default() += 1;
        }
        return Ok(());
    }
    let input_rows = frames.rows(&case.input).unwrap_or(0);
    if input_rows < 64 {
        return Ok(());
    }
    let replacement = if !state.targets.is_empty() {
        if !state.targets.contains(sql.as_str()) {
            return Ok(());
        }
        match state.seen.get(sql.as_str()) {
            Some((_, previous)) if input_rows <= *previous => return Ok(()),
            Some((index, _)) => Some(*index),
            None => None,
        }
    } else {
        None
    };
    let count = state.counts.entry(case.family).or_default();
    if replacement.is_none() && *count >= state.samples {
        return Ok(());
    }
    let index = replacement.unwrap_or_else(|| {
        *count += 1;
        let index = state.cases;
        state.cases += 1;
        index
    });
    if !state.targets.is_empty() {
        state.seen.insert(sql.clone(), (index, input_rows));
    }
    let report = measure::replay(connection, &case, state.rounds)?;
    let path = state.directory.join(format!("case-{index:04}.json"));
    let mut file = std::fs::File::create(path)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    serde_json::to_writer_pretty(
        &mut file,
        &json!({"case":index,"family":case.family,
        "input":case.input,"inputRows":input_rows,"originalSql":sql,"report":report}),
    )
    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    file.write_all(b"\n")
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    Ok(())
}
