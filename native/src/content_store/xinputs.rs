//! The cross-file families' per-content inputs (`code_rule_dsl::facts::xinputs`; schema.rs): what
//! Y1's reference graph and the dead-code symbol table read of a content, produced from its tape.
//! Rows are keyed by `tape_objects.tape_id` and cascade from it, as the prefilter's are, so a
//! collected tape takes its rows with it and every path or workspace holding the content shares
//! them. A set row (`xinput_sets`) names the producer the rows were built by and their digest;
//! publishing again replaces the set. Text values are atoms, interned in the publication's
//! transaction and read back through `atoms`, so the process mirror is not grown by them.
//!
//! Each relation is one narrow table (`TABLES`): `tape_id`, the summary variant where it has one,
//! the row's ordinal, then its columns. Integer columns hold integers, atom columns atoms; either
//! may be NULL where the relation's column may.

use super::{ContentStore, StoreError, StoreResult, TapeId};
use rusqlite::types::Value;
use rusqlite::{OptionalExtension, params};
use std::collections::BTreeMap;

/// A column's storage: an integer, or a text held as its atom.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Col {
    Int,
    Atom,
}

/// One relation's table: its name, whether rows carry a summary variant, and its value columns.
#[derive(Debug)]
pub(crate) struct Table {
    pub name: &'static str,
    pub variant: bool,
    pub cols: &'static [(&'static str, Col)],
}

const fn t(name: &'static str, variant: bool, cols: &'static [(&'static str, Col)]) -> Table {
    Table { name, variant, cols }
}

use Col::{Atom, Int};

/// Every relation, in the order a set's rows are given and returned.
pub(crate) const TABLES: [Table; 20] = [
    t("xinput_cgn_defs", true, &[("name", Atom), ("start_byte", Int), ("end_byte", Int), ("flags", Int)]),
    t("xinput_cgn_edges", true, &[("def", Int), ("name", Atom)]),
    t("xinput_cgn_aliases", true, &[("key", Atom), ("target", Atom)]),
    t("xinput_cgn_opaque", true, &[("name", Atom)]),
    t("xinput_cgn_modules", true, &[("name", Atom)]),
    t("xinput_cgn_open", true, &[("open", Int)]),
    t("xinput_cgn_base_slots", true, &[("key", Atom), ("target", Atom)]),
    t("xinput_functions", false, &[("start_byte", Int), ("end_byte", Int), ("name", Atom), ("class_name", Atom), ("return_annotation", Atom), ("class_start", Int), ("body_start", Int)]),
    t("xinput_classes", false, &[("start_byte", Int), ("end_byte", Int), ("name", Atom), ("body_start", Int)]),
    t("xinput_decorators", false, &[("name", Atom), ("owner", Int)]),
    t("xinput_parameters", false, &[("start_byte", Int), ("end_byte", Int), ("annotation", Atom), ("name", Atom), ("owner", Int)]),
    t("xinput_parameter_annotations", false, &[("name", Atom), ("parameter", Int)]),
    t("xinput_return_annotations", false, &[("name", Atom), ("owner", Int)]),
    t("xinput_imports", false, &[("start_byte", Int), ("base", Atom), ("items", Atom), ("top", Int)]),
    t("xinput_bases", false, &[("start_byte", Int), ("name", Atom), ("owner", Int)]),
    t("xinput_all", false, &[("start_byte", Int), ("array", Atom)]),
    t("xinput_all_names", false, &[("name", Atom), ("owner", Int)]),
    t("xinput_module_names", false, &[("name", Atom)]),
    t("xinput_dynamic_imports", false, &[("entry", Atom)]),
    t("xinput_execution_calls", false, &[("name", Atom), ("owner", Int)]),
];

/// One relation's rows: (variant, values in `Table::cols` order); the variant is 0 where the
/// table has none. Atom columns are `Value::Text` (or `Null`), integer columns `Value::Integer`.
pub(crate) type Rows = Vec<(u8, Vec<Value>)>;

/// A tape's stored inputs.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct XinputSet {
    pub producer: [u8; 32],
    pub digest: [u8; 32],
    /// Why the producer could not read the content; then no rows.
    pub failure: Option<String>,
    /// Per table of `TABLES`, its rows in ordinal order.
    pub rows: Vec<Rows>,
}

fn digest32(bytes: Vec<u8>) -> StoreResult<[u8; 32]> {
    bytes.try_into().map_err(|_| StoreError::Corrupt("an xinput digest is not 32 bytes".to_owned()))
}

impl ContentStore {
    /// Publishes `set` as `tape`'s inputs, replacing any it had, in one transaction.
    pub(crate) fn put_xinputs(&self, tape: TapeId, set: &XinputSet) -> StoreResult<()> {
        if set.rows.len() != TABLES.len() {
            return Err(StoreError::Invalid(format!("an xinput set has {} relations, not {}", set.rows.len(), TABLES.len())));
        }
        self.immediate_transaction(|transaction| {
            transaction.execute("DELETE FROM xinput_sets WHERE tape_id=?1", [tape.row()])?;
            transaction.execute(
                "INSERT INTO xinput_sets(tape_id,producer,digest,failure) VALUES (?1,?2,?3,?4)",
                params![tape.row(), &set.producer[..], &set.digest[..], set.failure],
            )?;
            // Each text's atom: new texts appended (dense), then looked up once per text.
            let mut atoms: BTreeMap<&str, i64> = BTreeMap::new();
            for (table, rows) in TABLES.iter().zip(&set.rows) {
                for (_, values) in rows {
                    for ((_, col), value) in table.cols.iter().zip(values) {
                        if let (Col::Atom, Value::Text(text)) = (col, value) {
                            atoms.entry(text.as_str()).or_insert(-1);
                        }
                    }
                }
            }
            {
                let mut insert = transaction.prepare_cached("INSERT INTO atoms(value) VALUES (?1) ON CONFLICT(value) DO NOTHING")?;
                let mut atom = transaction.prepare_cached("SELECT atom FROM atoms WHERE value=?1")?;
                for (text, id) in atoms.iter_mut() {
                    insert.execute([*text])?;
                    *id = atom.query_row([*text], |row| row.get(0))?;
                }
            }
            for (table, rows) in TABLES.iter().zip(&set.rows) {
                if rows.is_empty() {
                    continue;
                }
                let names = table.cols.iter().map(|(name, _)| *name).collect::<Vec<_>>().join(",");
                let keys = if table.variant { "tape_id,variant,ord" } else { "tape_id,ord" };
                let marks = std::iter::repeat_n("?", table.cols.len() + 2 + usize::from(table.variant)).collect::<Vec<_>>().join(",");
                let mut insert = transaction.prepare_cached(&format!("INSERT INTO {}({keys},{names}) VALUES ({marks})", table.name))?;
                let mut ords = [0i64; 3];
                for (variant, values) in rows {
                    if values.len() != table.cols.len() || (!table.variant && *variant != 0) || *variant > 2 {
                        return Err(StoreError::Invalid(format!("a malformed {} row", table.name)));
                    }
                    let ord = &mut ords[*variant as usize];
                    let mut bound: Vec<Value> = vec![Value::Integer(tape.row())];
                    if table.variant {
                        bound.push(Value::Integer(i64::from(*variant)));
                    }
                    bound.push(Value::Integer(*ord));
                    for ((name, col), value) in table.cols.iter().zip(values) {
                        bound.push(match (col, value) {
                            (_, Value::Null) => Value::Null,
                            (Col::Int, Value::Integer(v)) => Value::Integer(*v),
                            (Col::Atom, Value::Text(text)) => Value::Integer(atoms[text.as_str()]),
                            _ => return Err(StoreError::Invalid(format!("{}.{name}: a value of the wrong type", table.name))),
                        });
                    }
                    insert.execute(rusqlite::params_from_iter(bound))?;
                    *ord += 1;
                }
            }
            Ok(())
        })
    }

    /// `tape`'s inputs, if a set is stored.
    pub(crate) fn xinputs(&self, tape: TapeId) -> StoreResult<Option<XinputSet>> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let Some((producer, digest, failure)) = connection
            .prepare_cached("SELECT producer,digest,failure FROM xinput_sets WHERE tape_id=?1")?
            .query_row([tape.row()], |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, Option<String>>(2)?)))
            .optional()?
        else {
            return Ok(None);
        };
        let mut rows = Vec::with_capacity(TABLES.len());
        for table in &TABLES {
            let cols = table
                .cols
                .iter()
                .map(|(name, col)| match col {
                    Col::Int => format!("r.{name}"),
                    Col::Atom => format!("(SELECT value FROM atoms WHERE atom=r.{name})"),
                })
                .collect::<Vec<_>>()
                .join(",");
            let (variant, order) = if table.variant { ("r.variant", "r.variant,r.ord") } else { ("0", "r.ord") };
            let mut select = connection.prepare_cached(&format!("SELECT {variant},{cols} FROM {} r WHERE r.tape_id=?1 ORDER BY {order}", table.name))?;
            let n = table.cols.len();
            let got = select
                .query_map([tape.row()], |row| {
                    let variant: i64 = row.get(0)?;
                    let values = (1..=n).map(|i| row.get::<_, Value>(i)).collect::<rusqlite::Result<Vec<_>>>()?;
                    Ok((variant as u8, values))
                })?
                .collect::<rusqlite::Result<Rows>>()?;
            rows.push(got);
        }
        Ok(Some(XinputSet { producer: digest32(producer)?, digest: digest32(digest)?, failure, rows }))
    }
}

#[cfg(test)]
mod tests;
