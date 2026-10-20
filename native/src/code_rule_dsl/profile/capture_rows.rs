//! Profile-only SQL access to shared capture projections. No cell vectors are copied.
use crate::code_rule_dsl::{
    json::invalid,
    model::Anchor,
    violations::{Witness, anchored_span},
};
use rerail_reachability_spike::rows::{Charge, Source, register};
use rusqlite::{Connection, Result, types::Value as Sql, vtab::Context};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, atomic::AtomicUsize},
};

struct Row {
    path: usize,
    start: i64,
    end: i64,
    projection: Option<Arc<Value>>,
}
struct Captures {
    fields: Vec<String>,
    paths: Vec<String>,
    path_rows: HashMap<String, Vec<usize>>,
    rows: Vec<Row>,
    charge: Charge,
}
impl Source for Captures {
    fn columns(&self) -> Vec<String> {
        ["__path", "__start", "__end"]
            .into_iter()
            .map(str::to_owned)
            .chain(self.fields.iter().cloned())
            .collect()
    }
    fn column_types(&self) -> Vec<String> {
        ["TEXT", "INTEGER", "INTEGER"]
            .into_iter()
            .map(str::to_owned)
            .chain(self.fields.iter().map(|_| String::new()))
            .collect()
    }
    fn len(&self) -> usize {
        self.rows.len()
    }
    fn indexed(&self, column: usize) -> bool {
        column == 0
    }
    fn charge(&self, count: usize) -> Result<()> {
        (self.charge)(count)
    }
    fn lookup(&self, column: usize, values: &[Sql]) -> Result<Option<Vec<usize>>> {
        if column != 0
            || values
                .iter()
                .any(|v| !matches!(v, Sql::Text(_) | Sql::Null))
        {
            return Ok(None);
        }
        let mut result = Vec::new();
        for value in values {
            self.charge(1)?;
            if let Sql::Text(path) = value
                && let Some(rows) = self.path_rows.get(path)
            {
                self.charge(rows.len())?;
                result.extend_from_slice(rows);
            }
        }
        result.sort_unstable();
        result.dedup();
        Ok(Some(result))
    }
    fn column(&self, row: usize, column: usize, ctx: &mut Context) -> Result<()> {
        let row = &self.rows[row];
        match column {
            0 => ctx.set_result(&self.paths[row.path]),
            1 => ctx.set_result(&row.start),
            2 => ctx.set_result(&row.end),
            _ => match row
                .projection
                .as_deref()
                .and_then(Value::as_object)
                .and_then(|p| p.get(&self.fields[column - 3]))
                .unwrap_or(&Value::Null)
            {
                Value::Null => ctx.set_result(&Sql::Null),
                Value::Bool(v) => ctx.set_result(&i64::from(*v)),
                Value::Number(v) => {
                    if let Some(v) = v.as_i64() {
                        ctx.set_result(&v)
                    } else {
                        ctx.set_result(&v.as_f64().unwrap_or_default())
                    }
                }
                Value::String(v) => ctx.set_result(v),
                v => ctx.set_result(&v.to_string()),
            },
        }
    }
}

pub(crate) fn load(
    db: &Connection,
    name: &str,
    fields: &[String],
    witnesses: &[Witness],
    anchor: &Anchor,
    work: Arc<AtomicUsize>,
) -> napi::Result<()> {
    let mut span = super::trace::Span::start("sql-input", name);
    let charge = super::native_rows::charge(work);
    let mut source = Captures {
        fields: fields.to_vec(),
        paths: Vec::new(),
        path_rows: HashMap::new(),
        rows: Vec::with_capacity(witnesses.len()),
        charge,
    };
    let mut path_ids = HashMap::new();
    for witness in witnesses {
        source.charge(1).map_err(|e| invalid(e.to_string()))?;
        let next = source.paths.len();
        let path = *path_ids.entry(witness.path.as_str()).or_insert_with(|| {
            source.paths.push(witness.path.clone());
            next
        });
        source
            .path_rows
            .entry(witness.path.clone())
            .or_default()
            .push(source.rows.len());
        let location = anchored_span(anchor, witness).0;
        source.rows.push(Row {
            path,
            start: location.start_byte as i64,
            end: location.end_byte as i64,
            projection: witness.captures.shared_projection(),
        });
    }
    register(db, name, Arc::new(source)).map_err(|e| invalid(e.to_string()))?;
    span.detail(|| serde_json::json!({"rows":witnesses.len(),"insertStatements":0,"sharedCaptureProjections":true}));
    Ok(())
}
