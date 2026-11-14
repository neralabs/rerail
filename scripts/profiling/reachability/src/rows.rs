//! Generic immutable native rows, shared by SQL virtual tables and point access.
use crate::graph::failure;
use rusqlite::{Connection, Result, functions::FunctionFlags, types::Value, vtab::Context};
use serde_json::{Value as Json, json};
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

pub type Charge = Arc<dyn Fn(usize) -> Result<()> + Send + Sync>;
pub trait Source: Send + Sync {
    fn columns(&self) -> Vec<String>;
    fn column_types(&self) -> Vec<String> {
        vec![String::new(); self.columns().len()]
    }
    fn len(&self) -> usize;
    fn estimated_matches(&self, _column: usize, batch: bool) -> usize {
        if batch {
            self.estimated_len().min(1000)
        } else {
            self.estimated_len().div_ceil(20).max(1)
        }
    }
    fn estimated_len(&self) -> usize {
        self.len()
    }
    fn initialize(&self) -> Result<()> {
        Ok(())
    }
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn column(&self, row: usize, column: usize, ctx: &mut Context) -> Result<()>;
    /// A declared equality index. None leaves SQLite to filter a full scan.
    fn lookup(&self, _column: usize, _values: &[Value]) -> Result<Option<Vec<usize>>> {
        Ok(None)
    }
    fn indexed(&self, _column: usize) -> bool {
        false
    }
    fn charge(&self, _count: usize) -> Result<()> {
        Ok(())
    }
}
#[derive(Default)]
pub struct Counters {
    pub scans: AtomicU64,
    pub batches: AtomicU64,
    pub candidates: AtomicU64,
    pub columns: AtomicU64,
}
impl Counters {
    pub fn json(&self) -> Json {
        json!({"scans":self.scans.load(Ordering::Relaxed),"batches":self.batches.load(Ordering::Relaxed),
            "candidates":self.candidates.load(Ordering::Relaxed),"columns":self.columns.load(Ordering::Relaxed)})
    }
}
pub fn register(db: &Connection, name: &str, source: Arc<dyn Source>) -> Result<Arc<Counters>> {
    crate::row_table::register(db, name, source)
}
pub struct OwnedRows {
    pub columns: Vec<String>,
    pub types: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub charge: Charge,
    indexes: Mutex<HashMap<usize, HashMap<String, Vec<usize>>>>,
    indexed: Vec<usize>,
}
impl OwnedRows {
    pub fn new(
        columns: Vec<String>,
        rows: Vec<Vec<Value>>,
        indexed: Vec<usize>,
        charge: Charge,
    ) -> Result<Self> {
        if rows.iter().any(|r| r.len() != columns.len())
            || indexed.iter().any(|i| *i >= columns.len())
        {
            return Err(failure("invalid native row width or index"));
        }
        let types = vec![String::new(); columns.len()];
        Ok(Self {
            columns,
            types,
            rows,
            charge,
            indexes: Mutex::new(HashMap::new()),
            indexed,
        })
    }
}
fn key(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Integer(v) => Some(format!("n{v}")),
        Value::Real(v) if v.fract() == 0.0 && *v >= i64::MIN as f64 && *v < (i64::MAX as f64) => {
            Some(format!("n{}", *v as i64))
        }
        Value::Real(v) => Some(format!("f{:x}", v.to_bits())),
        Value::Text(v) => Some(format!("t{v}")),
        Value::Blob(v) => Some(format!("b{v:?}")),
    }
}
impl Source for OwnedRows {
    fn column_types(&self) -> Vec<String> {
        self.types.clone()
    }
    fn columns(&self) -> Vec<String> {
        self.columns.clone()
    }
    fn len(&self) -> usize {
        self.rows.len()
    }
    fn column(&self, row: usize, column: usize, ctx: &mut Context) -> Result<()> {
        ctx.set_result(&self.rows[row][column])
    }
    fn indexed(&self, column: usize) -> bool {
        self.indexed.contains(&column)
    }
    fn charge(&self, count: usize) -> Result<()> {
        (self.charge)(count)
    }
    fn lookup(&self, column: usize, values: &[Value]) -> Result<Option<Vec<usize>>> {
        if !self.indexed(column) {
            return Ok(None);
        }
        if self.types[column] == "TEXT"
            && values
                .iter()
                .any(|v| !matches!(v, Value::Text(_) | Value::Null))
        {
            return Ok(None);
        }
        if self.types[column] == "INTEGER"
            && values
                .iter()
                .any(|v| !matches!(v, Value::Integer(_) | Value::Null))
        {
            return Ok(None);
        }

        let mut indexes = self
            .indexes
            .lock()
            .map_err(|_| failure("row index poisoned"))?;
        if let std::collections::hash_map::Entry::Vacant(entry) = indexes.entry(column) {
            let mut index: HashMap<String, Vec<usize>> = HashMap::new();
            for (i, row) in self.rows.iter().enumerate() {
                self.charge(1)?;
                if let Some(key) = key(&row[column]) {
                    index.entry(key).or_default().push(i);
                }
            }
            entry.insert(index);
        }
        let index = &indexes[&column];
        let mut result = Vec::new();
        for value in values {
            self.charge(1)?;
            if let Some(rows) = key(value).and_then(|k| index.get(&k)) {
                result.extend_from_slice(rows);
            }
        }
        result.sort_unstable();
        result.dedup();
        Ok(Some(result))
    }
}
pub fn from_json(value: &Json) -> Result<Value> {
    Ok(match value {
        Json::Null => Value::Null,
        Json::Bool(v) => Value::Integer(i64::from(*v)),
        Json::Number(v) => v
            .as_i64()
            .map(Value::Integer)
            .or_else(|| v.as_f64().filter(|f| f.is_finite()).map(Value::Real))
            .ok_or_else(|| failure("nonfinite native fact"))?,
        Json::String(v) => Value::Text(v.clone()),
        _ => return Err(failure("native row cells must be JSON scalars")),
    })
}
pub fn register_loader(db: &Connection) -> Result<()> {
    let counters = Arc::new(Mutex::new(HashMap::<String, Arc<Counters>>::new()));
    let shared = Arc::clone(&counters);
    db.create_scalar_function(
        c"native_rows_load",
        4,
        FunctionFlags::SQLITE_UTF8,
        move |ctx| {
            let name = ctx.get::<String>(0)?;
            let columns: Vec<String> = serde_json::from_str(&ctx.get::<String>(1)?)
                .map_err(|e| failure(&e.to_string()))?;
            let rows: Vec<Vec<Json>> = serde_json::from_str(&ctx.get::<String>(2)?)
                .map_err(|e| failure(&e.to_string()))?;
            let indexed: Vec<usize> = serde_json::from_str(&ctx.get::<String>(3)?)
                .map_err(|e| failure(&e.to_string()))?;
            let rows = rows
                .iter()
                .map(|r| r.iter().map(from_json).collect())
                .collect::<Result<Vec<Vec<Value>>>>()?;
            let source = Arc::new(OwnedRows::new(
                columns,
                rows,
                indexed,
                Arc::new(|_| Ok(())),
            )?);
            // SQLite owns the connection; registration performs no recursive query.
            let db = unsafe { ctx.get_connection()? };
            let stats = register(&db, &name, source)?;
            shared
                .lock()
                .map_err(|_| failure("row stats poisoned"))?
                .insert(name, stats);
            Ok(1)
        },
    )?;
    db.create_scalar_function(
        c"native_rows_stats",
        1,
        FunctionFlags::SQLITE_UTF8,
        move |ctx| {
            let name = ctx.get::<String>(0)?;
            let stats = counters.lock().map_err(|_| failure("row stats poisoned"))?;
            Ok(stats
                .get(&name)
                .ok_or_else(|| failure("unknown native row source"))?
                .json()
                .to_string())
        },
    )?;
    Ok(())
}
