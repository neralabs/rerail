//! Immutable column batches for projected facts. JSON is built only at wire/JSON-semantic boundaries.
use rusqlite::types::ValueRef;
use serde::{Serialize, ser::SerializeMap};
use serde_json::{Map, Value};
use std::{
    borrow::Cow,
    cell::RefCell,
    collections::HashMap,
    ops::Range,
    sync::{Arc, OnceLock},
};

const CHUNK: usize = 256;
#[derive(Clone, Debug)]
enum Cell {
    Null,
    Integer(i64),
    Text(u32, u32, u32),
    Json(Arc<(Value, String)>),
}
#[derive(Debug)]
struct Data {
    names: Arc<[String]>,
    rows: usize,
    raw: Option<Value>,
    columns: Vec<Vec<Cell>>,
    sources: Vec<Arc<String>>,
    owned: Vec<bool>,
}
#[derive(Clone, Debug)]
pub(crate) struct Row {
    data: Arc<OnceLock<Data>>,
    row: usize,
}

pub(crate) enum Input<'a> {
    Null,
    Integer(i64),
    Text(Cow<'a, str>),
    Source(&'a Arc<String>, Range<usize>),
}
impl Input<'_> {
    pub(crate) fn text(&self) -> Option<&str> {
        match self {
            Self::Text(v) => Some(v),
            Self::Source(v, r) => v.get(r.clone()),
            _ => None,
        }
    }
    pub(crate) fn slice(self, range: Range<usize>) -> Self {
        match self {
            Self::Source(v, r) => Self::Source(v, r.start + range.start..r.start + range.end),
            Self::Text(Cow::Borrowed(v)) => Self::Text(Cow::Borrowed(&v[range])),
            Self::Text(Cow::Owned(v)) => Self::Text(Cow::Owned(v[range].to_owned())),
            _ => Self::Null,
        }
    }
}
#[derive(Hash, PartialEq, Eq)]
struct Interned(Arc<String>);
impl std::borrow::Borrow<str> for Interned {
    fn borrow(&self) -> &str {
        self.0.as_str()
    }
}
struct Builder {
    names: Arc<[String]>,
    rows: usize,
    data: Arc<OnceLock<Data>>,
    columns: Vec<Vec<Cell>>,
    sources: Vec<Arc<String>>,
    owned: Vec<bool>,
    pointers: HashMap<usize, u32>,
    strings: HashMap<Interned, u32>,
}
impl Builder {
    fn new(names: Arc<[String]>) -> Self {
        Self {
            rows: 0,
            columns: names.iter().map(|_| Vec::new()).collect(),
            names,
            data: Arc::default(),
            sources: Vec::new(),
            owned: Vec::new(),
            pointers: HashMap::new(),
            strings: HashMap::new(),
        }
    }
    fn source(&mut self, value: &Arc<String>) -> u32 {
        let ptr = Arc::as_ptr(value) as usize;
        if let Some(&id) = self.pointers.get(&ptr) {
            return id;
        }
        let id = u32::try_from(self.sources.len()).expect("bounded projection sources");
        self.sources.push(Arc::clone(value));
        self.owned.push(false);
        self.pointers.insert(ptr, id);
        id
    }
    fn string(&mut self, value: Cow<'_, str>) -> u32 {
        if let Some(&id) = self.strings.get(value.as_ref()) {
            return id;
        }
        let value = Arc::new(value.into_owned());
        let id = u32::try_from(self.sources.len()).expect("bounded projection sources");
        self.sources.push(Arc::clone(&value));
        self.owned.push(true);
        self.strings.insert(Interned(value), id);
        id
    }
    fn push<'a>(&mut self, values: impl Iterator<Item = Input<'a>>) -> Row {
        let row = self.rows;
        self.rows += 1;
        if row == 0 {
            for column in &mut self.columns {
                column.reserve(CHUNK);
            }
        }
        for (column, value) in values.enumerate() {
            let value = match value {
                Input::Null => Cell::Null,
                Input::Integer(v) => Cell::Integer(v),
                Input::Source(source, range) => Cell::Text(
                    self.source(source),
                    range.start.try_into().expect("bounded source"),
                    range.end.try_into().expect("bounded source"),
                ),
                Input::Text(v) => {
                    let len = v.len().try_into().expect("bounded text");
                    Cell::Text(self.string(v), 0, len)
                }
            };
            self.columns[column].push(value);
        }
        assert!(self.columns.iter().all(|c| c.len() == row + 1));
        let result = Row {
            data: Arc::clone(&self.data),
            row,
        };
        if row + 1 == CHUNK {
            self.seal();
        }
        result
    }
    fn seal(&mut self) {
        if self.rows == 0 {
            return;
        }
        let data = Data {
            raw: None,
            rows: std::mem::take(&mut self.rows),
            names: Arc::clone(&self.names),
            columns: std::mem::replace(
                &mut self.columns,
                self.names.iter().map(|_| Vec::new()).collect(),
            ),
            sources: std::mem::take(&mut self.sources),
            owned: std::mem::take(&mut self.owned),
        };
        self.data.set(data).expect("one projection publication");
        self.data = Arc::default();
        self.pointers.clear();
        self.strings.clear();
    }
}
#[derive(Default)]
struct Arena {
    builders: HashMap<(usize, usize, usize), Builder>,
}
thread_local! {static CURRENT: RefCell<Option<Arena>>=const {RefCell::new(None)};}
pub(crate) struct Scope(Option<Arena>);
impl Scope {
    pub(crate) fn enter() -> Self {
        Self(CURRENT.replace(Some(Arena::default())))
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with_borrow_mut(|slot| {
            if let Some(mut arena) = slot.take() {
                for builder in arena.builders.values_mut() {
                    builder.seal();
                }
            }
            *slot = self.0.take();
        });
    }
}
pub(crate) fn append<'a>(
    key: (usize, usize, usize),
    names: impl FnOnce() -> Vec<String>,
    values: impl Iterator<Item = Input<'a>>,
) -> Row {
    CURRENT.with_borrow_mut(|slot| {
        if let Some(arena) = slot {
            arena
                .builders
                .entry(key)
                .or_insert_with(|| Builder::new(names().into()))
                .push(values)
        } else {
            let mut builder = Builder::new(names().into());
            let row = builder.push(values);
            builder.seal();
            row
        }
    })
}
impl Row {
    fn data(&self) -> &Data {
        self.data
            .get()
            .expect("projection scope publishes before consumption")
    }
    pub(crate) fn published(&self) -> bool {
        self.data.get().is_some()
    }

    pub(crate) fn identity(&self) -> usize {
        Arc::as_ptr(&self.data) as usize
    }
    pub(crate) fn schema_identity(&self) -> usize {
        self.data().names.as_ptr() as usize
    }
    pub(crate) fn index(&self, name: &str) -> Option<usize> {
        self.data().names.iter().position(|n| n == name)
    }
    pub(crate) fn sql(&self, column: usize) -> ValueRef<'_> {
        match &self.data().columns[column][self.row] {
            Cell::Null => ValueRef::Null,
            Cell::Integer(v) => ValueRef::Integer(*v),
            Cell::Text(source, start, end) => ValueRef::Text(
                &self.data().sources[*source as usize].as_bytes()[*start as usize..*end as usize],
            ),
            Cell::Json(v) => match &v.0 {
                Value::Bool(b) => ValueRef::Integer(i64::from(*b)),
                Value::Number(n) => ValueRef::Real(n.as_f64().unwrap()),
                _ => ValueRef::Text(v.1.as_bytes()),
            },
        }
    }
    fn value(&self, column: usize) -> Value {
        match &self.data().columns[column][self.row] {
            Cell::Null => Value::Null,
            Cell::Integer(v) => (*v).into(),
            Cell::Text(source, start, end) => {
                self.data().sources[*source as usize][*start as usize..*end as usize].into()
            }
            Cell::Json(v) => v.0.clone(),
        }
    }
    pub(crate) fn object(&self) -> Map<String, Value> {
        self.data()
            .names
            .iter()
            .enumerate()
            .map(|(c, n)| (n.clone(), self.value(c)))
            .collect()
    }
    #[cfg(test)]
    pub(crate) fn json(&self) -> Value {
        self.data()
            .raw
            .clone()
            .unwrap_or_else(|| Value::Object(self.object()))
    }
    pub(crate) fn from_json(value: Value) -> Self {
        // Wire input can contain arbitrary JSON. Normal AST projections never use this path.
        let Some(object) = value.as_object() else {
            let data = OnceLock::from(Data {
                names: Arc::from([]),
                rows: 1,
                raw: Some(value),
                columns: Vec::new(),
                sources: Vec::new(),
                owned: Vec::new(),
            });
            return Self {
                data: Arc::new(data),
                row: 0,
            };
        };
        let mut builder = Builder::new(object.keys().cloned().collect::<Vec<_>>().into());
        for (c, value) in object.values().enumerate() {
            let cell = match value {
                Value::Null => Cell::Null,
                Value::Number(n) if n.as_i64().is_some() => Cell::Integer(n.as_i64().unwrap()),
                Value::String(s) => Cell::Text(
                    builder.string(Cow::Borrowed(s)),
                    0,
                    s.len().try_into().unwrap(),
                ),
                other => Cell::Json(Arc::new((other.clone(), other.to_string()))),
            };
            builder.columns[c].push(cell);
        }
        let row = Self {
            data: Arc::clone(&builder.data),
            row: 0,
        };
        builder.rows = 1;
        builder.seal();
        row
    }
    pub(crate) fn memory(&self) -> (usize, usize, usize, usize) {
        let d = self.data();
        let bytes = d.columns.capacity() * std::mem::size_of::<Vec<Cell>>()
            + d.columns
                .iter()
                .map(|c| c.capacity() * std::mem::size_of::<Cell>())
                .sum::<usize>()
            + d.names.iter().map(String::capacity).sum::<usize>()
            + d.sources
                .iter()
                .zip(&d.owned)
                .filter(|(_, owned)| **owned)
                .map(|(s, _)| s.capacity())
                .sum::<usize>();
        (
            bytes,
            d.rows,
            d.columns.len(),
            d.sources
                .iter()
                .zip(&d.owned)
                .filter(|(_, owned)| !**owned)
                .map(|(s, _)| s.len())
                .sum(),
        )
    }
}
impl PartialEq for Row {
    fn eq(&self, other: &Self) -> bool {
        if Arc::ptr_eq(&self.data, &other.data) && self.row == other.row {
            return true;
        }
        self.data().raw == other.data().raw
            && self.data().names == other.data().names
            && (0..self.data().columns.len()).all(|c| {
                match (
                    &self.data().columns[c][self.row],
                    &other.data().columns[c][other.row],
                ) {
                    (Cell::Null, Cell::Null) => true,
                    (Cell::Integer(a), Cell::Integer(b)) => a == b,
                    (Cell::Text(..), Cell::Text(..)) => self.sql(c) == other.sql(c),
                    _ => self.value(c) == other.value(c),
                }
            })
    }
}
impl Serialize for Row {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if let Some(raw) = &self.data().raw {
            return raw.serialize(serializer);
        }
        let mut map = serializer.serialize_map(Some(self.data().names.len()))?;
        for (c, n) in self.data().names.iter().enumerate() {
            map.serialize_entry(n, &Serialized(self, c))?;
        }
        map.end()
    }
}

struct Serialized<'a>(&'a Row, usize);
impl Serialize for Serialized<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match &self.0.data().columns[self.1][self.0.row] {
            Cell::Null => serializer.serialize_none(),
            Cell::Integer(v) => serializer.serialize_i64(*v),
            Cell::Text(source, start, end) => serializer.serialize_str(
                &self.0.data().sources[*source as usize][*start as usize..*end as usize],
            ),
            Cell::Json(v) => v.0.serialize(serializer),
        }
    }
}
#[cfg(test)]
mod tests;
