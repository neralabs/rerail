//! Experimental SQLite interfaces. Rule evaluation and joins remain in SQLite.
mod flow;
mod forest;
pub mod graph;
mod regex;
mod row_table;
pub mod rows;
mod table;

use graph::{Graph, Registry, failure};
#[cfg(feature = "extension")]
use rusqlite::ffi;
use rusqlite::{Connection, Result, functions::FunctionFlags};
#[cfg(feature = "extension")]
use std::ffi::{c_char, c_int};
use std::sync::{Arc, Mutex};

/// # Safety
/// SQLite supplies a live connection and its extension API table.
#[cfg(feature = "extension")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite3_extension_init(
    db: *mut ffi::sqlite3,
    error: *mut *mut c_char,
    api: *mut ffi::sqlite3_api_routines,
) -> c_int {
    unsafe { Connection::extension_init2(db, error, api, register) }
}

#[cfg(feature = "extension")]
fn register(db: Connection) -> Result<bool> {
    register_graph(&db)?;
    rows::register_loader(&db)?;
    register_flow(&db)?;
    Ok(false)
}

pub fn register_graph(db: &Connection) -> Result<graph::Shared> {
    if rusqlite::version_number() < 3_038_000 {
        return Err(failure("reachability spike requires SQLite 3.38 or newer"));
    }
    regex::register(db)?;
    let registry = Arc::new(Mutex::new(Registry::default()));
    let shared = Arc::clone(&registry);
    // Stateful functions deliberately do not claim SQLITE_DETERMINISTIC.
    db.create_scalar_function(c"reach_load", 4, FunctionFlags::SQLITE_UTF8, move |ctx| {
        let nodes: Vec<i64> =
            serde_json::from_str(&ctx.get::<String>(0)?).map_err(|e| failure(&e.to_string()))?;
        let edges: Vec<[i64; 2]> =
            serde_json::from_str(&ctx.get::<String>(1)?).map_err(|e| failure(&e.to_string()))?;
        let graph = Graph::new(nodes, edges, ctx.get(2)?, ctx.get(3)?)?;
        let mut registry = shared.lock().map_err(|_| failure("registry poisoned"))?;
        registry.next += 1;
        let key = registry.next;
        registry.graphs.insert(key, Arc::new(Mutex::new(graph)));
        Ok(key)
    })?;
    let shared = Arc::clone(&registry);
    db.create_scalar_function(c"tree_load", 4, FunctionFlags::SQLITE_UTF8, move |ctx| {
        let nodes: Vec<i64> =
            serde_json::from_str(&ctx.get::<String>(0)?).map_err(|e| failure(&e.to_string()))?;
        let edges: Vec<[i64; 2]> =
            serde_json::from_str(&ctx.get::<String>(1)?).map_err(|e| failure(&e.to_string()))?;
        let graph = Graph::new_tree(nodes, edges, ctx.get(2)?, ctx.get(3)?)?;
        let mut registry = shared.lock().map_err(|_| failure("registry poisoned"))?;
        registry.next += 1;
        let id = registry.next;
        registry.graphs.insert(id, Arc::new(Mutex::new(graph)));
        Ok(id)
    })?;
    let shared = Arc::clone(&registry);
    db.create_scalar_function(c"is_reachable", 3, FunctionFlags::SQLITE_UTF8, move |ctx| {
        let graph = graph::lookup(&shared, ctx.get(0)?)?;
        let mut graph = graph.lock().map_err(|_| failure("graph poisoned"))?;
        graph.scalar_calls += 1;
        let root = ctx.get(1)?;
        let node = ctx.get(2)?;
        Ok(i64::from(graph.contains(root, node)?))
    })?;
    let shared = Arc::clone(&registry);
    db.create_scalar_function(c"reach_stats", 1, FunctionFlags::SQLITE_UTF8, move |ctx| {
        let graph = graph::lookup(&shared, ctx.get(0)?)?;
        let graph = graph.lock().map_err(|_| failure("graph poisoned"))?;
        Ok(graph.stats().to_string())
    })?;
    let shared = Arc::clone(&registry);
    db.create_scalar_function(c"reach_drop", 1, FunctionFlags::SQLITE_UTF8, move |ctx| {
        let mut registry = shared.lock().map_err(|_| failure("registry poisoned"))?;
        Ok(i64::from(
            registry.graphs.remove(&ctx.get::<i64>(0)?).is_some(),
        ))
    })?;
    table::register(db, Arc::clone(&registry))?;
    Ok(registry)
}

pub fn register_flow(db: &Connection) -> Result<()> {
    flow::register(db)
}
