//! SQL extension sessions over the content store. Fact/rule programs use the existing typed
//! SQL emitter and wave scheduler; AST matching remains a separate native fact producer.

use super::facts::core::{self, Built};
use super::facts::vtab::{Relation, Source, syntax};
use super::logic::sql::select;
use super::logic::{plan, rr, strings};
use super::logic::program_image::{Builder, Cell, Version, fields::ProgramFields, manager::Pipeline};
use super::waves::{owner::Database, schedule};
use crate::content_store::ContentStore;
use napi::{Error, Result};
use napi_derive::napi;
use rusqlite::OptionalExtension;
pub use rusqlite::types::{Value as SqlValue, ValueRef};
pub use serde_json::{Value, json};
pub use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
pub use std::sync::Arc;
// catalog: rerail-sessions' (it reads the executor or the retained traces; crates/sessions mounts it)
// compiler: rerail-sessions' (it reads the executor or the retained traces; crates/sessions mounts it)
// explain: rerail-sessions' (it reads the executor or the retained traces; crates/sessions mounts it)
pub mod frontiers;
pub mod constants;
// A row of a corpus binding is `PROBE | document << 32 | row`: a 64-bit target only.
#[cfg(target_pointer_width = "64")]
pub mod corpus;
// documents: rerail-sessions' (it reads the executor or the retained traces; crates/sessions mounts it)
pub mod indexes;
pub mod live;
pub mod maintenance;
pub mod overlay;
pub mod phase;
pub mod planning;
// producers: rerail-sessions' (it reads the executor or the retained traces; crates/sessions mounts it)
// profiling: rerail-sessions' (it reads the executor or the retained traces; crates/sessions mounts it)
pub mod recursive;
pub mod reductions;
pub mod scratch;
// temporal: rerail-sessions' (it reads the executor or the retained traces; crates/sessions mounts it)
// updates: rerail-sessions' (it reads the executor or the retained traces; crates/sessions mounts it)
// verification: rerail-sessions' (it reads the documents)
#[cfg(any(any(test, feature = "test-hooks"), feature = "headless"))]
pub mod component_loop;
pub mod versions;
pub mod walks;
use super::logic::native::lattices;
use super::logic::sql::program;
pub mod runtime;
pub mod work_graph;
pub mod session;
pub mod wire;
// executor: `CodeRuleSqlExecutor`, rerail-sessions' (crates/sessions mounts it).

pub fn error(e: impl std::fmt::Display) -> Error {
    Error::from_reason(e.to_string())
}

pub fn identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    name.len() <= 120
        && bytes
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

pub fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

pub fn parameter(value: Value) -> Result<SqlValue> {
    Ok(match value {
        Value::Null => SqlValue::Null,
        Value::Bool(b) => SqlValue::Integer(i64::from(b)),
        Value::Number(n) if n.is_i64() => SqlValue::Integer(n.as_i64().unwrap()),
        Value::Number(n) if n.is_u64() => {
            return Err(error(
                "integer SQL parameter exceeds the signed 64-bit range",
            ));
        }
        Value::Number(n) => SqlValue::Real(
            n.as_f64()
                .ok_or_else(|| error("SQL parameter is out of range"))?,
        ),
        Value::String(s) => SqlValue::Text(s),
        Value::Object(_) => SqlValue::Integer(wire::exact(&value)?),
        _ => return Err(error("SQL parameters must be scalars or tagged integers")),
    })
}
