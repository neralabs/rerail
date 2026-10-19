//! `require_each` and `budget` answered in SQL over relation rows: the set-based counterparts of
//! `query::crossfile::{require_each, budget}`, which stay the parity oracle. Same arguments, same
//! results; a SQL error is returned apart (`SqlError`).
//!
//! # Input
//!
//! A relation's rows are loaded once per call into `cf_row(rel, rid)` and, for every operand
//! field the clause reads, `cf_value(rel, slot, rid, encoded)`: the field's value extracted
//! natively at load time, encoded as the operator reading it needs.
//!
//! - A `where` operand (`eq`/`ne`) reads the value's *canonical text* (`canonical`): an
//!   injective encoding that mirrors `same_text` term for term (integers by `(as_i64, as_u64)`,
//!   floats by their bits, arrays and objects item by item in order), so two values are
//!   `same_text` exactly where their canonical texts are equal. A missing field is NULL, and an
//!   operand pair compares with `IS`: missing equals only missing, as natively.
//! - A `group_by`/`distinct` operand reads the value's JSON text (`Display`, as `append_key`
//!   writes it); a missing field is NULL and reads `null` in the key (`COALESCE`).
//! - A literal operand is a bound parameter in the same encoding.
//!
//! # Statements
//!
//! A `where` predicate lowers to a boolean expression over those columns (`all` → `AND`, `any`
//! → `OR`, `not` → `NOT`, `eq`/`ne` → `IS`/`IS NOT`, constants → `1`/`0`). Every operand pair is
//! two-valued (`IS` never yields NULL), so the expression holds exactly where the native
//! predicate does. `require_each` is one statement: the subjects passing their predicate, minus
//! (`NOT IN`) those some evidence row satisfies the join predicate with (a join, which SQLite
//! answers through an automatic index on the evidence columns the predicate's `eq`s compare).
//! `budget` is one statement: the passing rows grouped by their key text, each group's first row,
//! row count and distinct-value count, filtered by the maximum unless some file is incomplete.
//!
//! The native bucket probe by key hash is an optimisation with no effect on the answer (a hash
//! collision only adds candidates the predicate then tests), so it has no counterpart here.
//!
//! Every predicate shape is lowered exactly; nothing is refused.

use super::SqlError;
use crate::code_rule_dsl::query::crossfile::{Group, Interrupted, Relation, Requirement};
use rusqlite::Connection;
use rusqlite::types::Value as SqlValue;
use serde_json::{Map, Number, Value};
use std::cell::RefCell;
use std::fmt::Write as _;

const SCHEMA: &str = "
    CREATE TABLE cf_row(
        rel INTEGER NOT NULL,
        rid INTEGER NOT NULL,
        PRIMARY KEY (rel, rid)
    ) WITHOUT ROWID;
    CREATE TABLE cf_value(
        rel INTEGER NOT NULL,
        slot INTEGER NOT NULL,
        rid INTEGER NOT NULL,
        encoded TEXT,
        PRIMARY KEY (rel, slot, rid)
    ) WITHOUT ROWID;
";

const CLEAR: &str = "DELETE FROM cf_row; DELETE FROM cf_value;";

thread_local! {
    /// This thread's connection, its tables created once.
    static CONNECTION: RefCell<Option<Connection>> = const { RefCell::new(None) };
}

fn with_connection<T>(
    body: impl FnOnce(&Connection) -> Result<T, SqlError>,
) -> Result<T, SqlError> {
    CONNECTION.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            let connection = Connection::open_in_memory()?;
            connection.execute_batch(SCHEMA)?;
            connection.set_prepared_statement_cache_capacity(64);
            *slot = Some(connection);
        }
        body(slot.as_ref().expect("connected"))
    })
}

/// The bound row the join predicate's evidence fields read; its subject fields read row 0.
const EVIDENCE: usize = 1;

/// How a loaded field value is encoded.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Encoding {
    /// `canonical`, for `eq`/`ne`.
    Canonical,
    /// JSON text, for `group_by`/`distinct` keys.
    Json,
}

impl Encoding {
    fn encode(self, value: &Value) -> String {
        match self {
            Self::Canonical => {
                let mut text = String::new();
                canonical(value, &mut text);
                text
            }
            Self::Json => value.to_string(),
        }
    }
}

/// Appends `value`'s canonical text: a prefix-free encoding, equal for two values exactly where
/// `same_text` holds of them.
fn canonical(value: &Value, text: &mut String) {
    match value {
        Value::Null => text.push('n'),
        Value::Bool(true) => text.push('t'),
        Value::Bool(false) => text.push('f'),
        Value::Number(number) => number_text(number, text),
        Value::String(string) => {
            let _ = write!(text, "s{}:{string}", string.len());
        }
        Value::Array(items) => {
            let _ = write!(text, "a{}:", items.len());
            items.iter().for_each(|item| canonical(item, text));
        }
        Value::Object(fields) => {
            let _ = write!(text, "o{}:", fields.len());
            for (key, value) in fields {
                let _ = write!(text, "{}:{key}", key.len());
                canonical(value, text);
            }
        }
    }
}

/// `same_number`: floats (either side) compare by bits and never equal an integer; integers by
/// `(as_i64, as_u64)`.
fn number_text(number: &Number, text: &mut String) {
    if number.is_f64() {
        let _ = write!(text, "d{:?};", number.as_f64().map(f64::to_bits));
    } else {
        let _ = write!(text, "i{:?}{:?};", number.as_i64(), number.as_u64());
    }
}

/// A predicate operand, as `query::crossfile` reads it: `alias.field` of a bound alias reads that
/// row's field (everything after the first dot); anything else is a literal.
enum Operand<'v> {
    Field(usize, &'v str),
    Literal(Value),
}

impl<'v> Operand<'v> {
    fn compile(value: &'v Value, aliases: &[&str]) -> Self {
        value
            .as_str()
            .and_then(|text| Self::field(text, aliases))
            .unwrap_or_else(|| Self::Literal(value.clone()))
    }

    fn named(names: &'v [String], aliases: &[&str]) -> Vec<Self> {
        names
            .iter()
            .map(|name| {
                Self::field(name, aliases)
                    .unwrap_or_else(|| Self::Literal(Value::String(name.clone())))
            })
            .collect()
    }

    fn field(text: &'v str, aliases: &[&str]) -> Option<Self> {
        let (alias, field) = text.split_once('.')?;
        let bound = aliases.iter().position(|name| *name == alias)?;
        Some(Self::Field(bound, field))
    }
}

/// A `where` predicate, compiled as `query::crossfile` compiles it.
enum Predicate<'v> {
    Constant(bool),
    All(Vec<Self>),
    Any(Vec<Self>),
    Not(Box<Self>),
    Equal(Operand<'v>, Operand<'v>, bool),
}

impl<'v> Predicate<'v> {
    /// No predicate holds; anything unreadable does not. An object's first of `all`, `any`,
    /// `not`, `eq`, `ne` decides; `all`/`any` of no items (or of a non-array) hold and do not.
    fn compile(predicate: Option<&'v Value>, aliases: &[&str]) -> Self {
        let Some(predicate) = predicate else {
            return Self::Constant(true);
        };
        let Some(object) = predicate.as_object() else {
            return Self::Constant(false);
        };
        let items = |items: &'v Value| {
            items
                .as_array()
                .into_iter()
                .flatten()
                .map(|item| Self::compile(Some(item), aliases))
                .collect()
        };
        if let Some(all) = object.get("all") {
            return Self::All(items(all));
        }
        if let Some(any) = object.get("any") {
            return Self::Any(items(any));
        }
        if let Some(inner) = object.get("not") {
            return Self::Not(Box::new(Self::compile(Some(inner), aliases)));
        }
        for (key, equal) in [("eq", true), ("ne", false)] {
            if let Some([left, right]) =
                object.get(key).and_then(Value::as_array).map(Vec::as_slice)
            {
                return Self::Equal(
                    Operand::compile(left, aliases),
                    Operand::compile(right, aliases),
                    equal,
                );
            }
        }
        Self::Constant(false)
    }

    /// Whether the native probe seeks evidence rows by key: an `eq` along the top-level
    /// conjunction comparing an evidence field with anything but an evidence field.
    fn seeks(&self) -> bool {
        match self {
            Self::All(items) => items.iter().any(Self::seeks),
            Self::Equal(left, right, true) => {
                [(left, right), (right, left)]
                    .into_iter()
                    .any(|(sought, other)| {
                        matches!(sought, Operand::Field(EVIDENCE, _))
                            && !matches!(other, Operand::Field(EVIDENCE, _))
                    })
            }
            _ => false,
        }
    }
}

/// The fields a statement reads, per relation: each `(field, encoding)` a slot, its column
/// `{prefix}{slot}`.
#[derive(Default)]
struct Slots<'v> {
    fields: Vec<(&'v str, Encoding)>,
}

impl<'v> Slots<'v> {
    fn slot(&mut self, field: &'v str, encoding: Encoding) -> usize {
        match self
            .fields
            .iter()
            .position(|known| *known == (field, encoding))
        {
            Some(slot) => slot,
            None => {
                self.fields.push((field, encoding));
                self.fields.len() - 1
            }
        }
    }
}

/// A statement's text under construction: its relations' slots and its literal parameters.
struct Lowering<'v> {
    /// Per bound alias: its relation's slots and its column prefix.
    relations: Vec<(Slots<'v>, &'static str)>,
    parameters: Vec<SqlValue>,
}

impl<'v> Lowering<'v> {
    fn new(prefixes: &[&'static str]) -> Self {
        Self {
            relations: prefixes
                .iter()
                .map(|prefix| (Slots::default(), *prefix))
                .collect(),
            parameters: Vec::new(),
        }
    }

    /// The SQL reading `operand` in `encoding`: its column, or a parameter holding the literal.
    fn operand(&mut self, operand: &Operand<'v>, encoding: Encoding) -> String {
        match operand {
            Operand::Field(bound, field) => {
                let (slots, prefix) = &mut self.relations[*bound];
                format!("{prefix}{}", slots.slot(field, encoding))
            }
            Operand::Literal(value) => {
                self.parameters.push(SqlValue::Text(encoding.encode(value)));
                format!("?{}", self.parameters.len())
            }
        }
    }

    fn predicate(&mut self, predicate: &Predicate<'v>) -> String {
        let join = |lowering: &mut Self, items: &[Predicate<'v>], operator: &str, empty: &str| {
            if items.is_empty() {
                return empty.to_owned();
            }
            let items: Vec<String> = items.iter().map(|item| lowering.predicate(item)).collect();
            format!("({})", items.join(operator))
        };
        match predicate {
            Predicate::Constant(true) => "1".to_owned(),
            Predicate::Constant(false) => "0".to_owned(),
            Predicate::All(items) => join(self, items, " AND ", "1"),
            Predicate::Any(items) => join(self, items, " OR ", "0"),
            Predicate::Not(inner) => format!("(NOT {})", self.predicate(inner)),
            Predicate::Equal(left, right, equal) => {
                let left = self.operand(left, Encoding::Canonical);
                let right = self.operand(right, Encoding::Canonical);
                let operator = if *equal { "IS" } else { "IS NOT" };
                format!("({left} {operator} {right})")
            }
        }
    }

    /// `group_by`/`distinct` operands' JSON texts joined by U+001F, a missing field `null`.
    fn key(&mut self, operands: &[Operand<'v>]) -> String {
        let parts: Vec<String> = operands
            .iter()
            .map(|operand| match operand {
                Operand::Field(..) => {
                    format!(
                        "COALESCE({}, 'null')",
                        self.operand(operand, Encoding::Json)
                    )
                }
                Operand::Literal(_) => self.operand(operand, Encoding::Json),
            })
            .collect();
        parts.join(" || char(31) || ")
    }

    /// A relation's rows `rel` with the columns of alias `bound`'s slots, as a `SELECT`.
    fn pivot(&self, bound: usize, rel: usize) -> String {
        let (slots, prefix) = &self.relations[bound];
        let mut columns = String::from("r.rid AS rid");
        let mut joins = String::new();
        for slot in 0..slots.fields.len() {
            let _ = write!(columns, ", v{slot}.encoded AS {prefix}{slot}");
            let _ = write!(
                joins,
                " JOIN cf_value AS v{slot} ON v{slot}.rel = {rel} AND v{slot}.slot = {slot} AND v{slot}.rid = r.rid"
            );
        }
        format!("SELECT {columns} FROM cf_row AS r{joins} WHERE r.rel = {rel}")
    }
}

fn integer(value: usize) -> Result<i64, SqlError> {
    i64::try_from(value).map_err(|_| SqlError::unsupported("cross-file row index past i64"))
}

/// Loads `rows` as relation `rel` with `slots`' values.
fn load(
    connection: &Connection,
    rel: i64,
    rows: &[Map<String, Value>],
    slots: &Slots<'_>,
) -> Result<(), SqlError> {
    let mut insert_row =
        connection.prepare_cached("INSERT INTO cf_row(rel, rid) VALUES (?1, ?2)")?;
    let mut insert_value = connection
        .prepare_cached("INSERT INTO cf_value(rel, slot, rid, encoded) VALUES (?1, ?2, ?3, ?4)")?;
    for (index, row) in rows.iter().enumerate() {
        let index = integer(index)?;
        insert_row.execute((rel, index))?;
        for (slot, (field, encoding)) in slots.fields.iter().enumerate() {
            let value = row.get(*field).map(|value| encoding.encode(value));
            insert_value.execute((rel, integer(slot)?, index, value))?;
        }
    }
    Ok(())
}

/// Polls `interrupted` between statements, where the native clause polls at all (it visits a
/// row).
struct Poll {
    interrupted: fn() -> bool,
    visits: bool,
}

impl Poll {
    fn check(&self) -> Result<(), Interrupted> {
        if self.visits && (self.interrupted)() {
            Err(Interrupted)
        } else {
            Ok(())
        }
    }
}

/// `require_each`: the subject rows (passing `subject_predicate`) no evidence row satisfies
/// `predicate` for, in row order; unknown, never missing, where evidence is incomplete. As
/// `query::crossfile::require_each`.
pub(in crate::code_rule_dsl) fn require_each(
    (subject_alias, subject): (&str, &Relation),
    subject_predicate: Option<&Value>,
    (evidence_alias, evidence): (&str, &Relation),
    predicate: Option<&Value>,
    interrupted: fn() -> bool,
) -> Result<Result<Vec<Requirement>, Interrupted>, SqlError> {
    let subject_predicate = Predicate::compile(subject_predicate, &[subject_alias]);
    let predicate = Predicate::compile(predicate, &[subject_alias, evidence_alias]);
    let poll = Poll {
        interrupted,
        visits: !subject.rows.is_empty() || (predicate.seeks() && !evidence.rows.is_empty()),
    };
    let mut lowering = Lowering::new(&["a", "b"]);
    let filter = lowering.predicate(&subject_predicate);
    let join = lowering.predicate(&predicate);
    let sql = format!(
        "WITH s AS MATERIALIZED (SELECT * FROM ({subjects}) WHERE {filter}),
              e AS MATERIALIZED ({evidence}),
              satisfied AS (SELECT DISTINCT s.rid AS rid FROM s JOIN e ON {join})
         SELECT rid FROM s WHERE rid NOT IN (SELECT rid FROM satisfied) ORDER BY rid",
        subjects = lowering.pivot(0, 0),
        evidence = lowering.pivot(EVIDENCE, 1),
    );
    let unknown = !evidence.incomplete.is_empty();
    with_connection(|connection| {
        if let Err(stopped) = poll.check() {
            return Ok(Err(stopped));
        }
        let transaction = connection.unchecked_transaction()?;
        transaction.execute_batch(CLEAR)?;
        load(&transaction, 0, &subject.rows, &lowering.relations[0].0)?;
        load(
            &transaction,
            1,
            &evidence.rows,
            &lowering.relations[EVIDENCE].0,
        )?;
        if let Err(stopped) = poll.check() {
            return Ok(Err(stopped));
        }
        let mut requirements = Vec::new();
        {
            let mut statement = transaction.prepare_cached(&sql)?;
            let mut rows =
                statement.query(rusqlite::params_from_iter(lowering.parameters.iter()))?;
            while let Some(row) = rows.next()? {
                let index = row.get::<_, i64>(0)? as usize;
                requirements.push(if unknown {
                    Requirement::Unknown(index)
                } else {
                    Requirement::Missing(index)
                });
            }
        }
        transaction.execute_batch(CLEAR)?;
        transaction.commit()?;
        Ok(Ok(requirements))
    })
}

/// `budget`: groups of the rows passing `predicate`, by `group_by` operands, whose count (of rows,
/// or of distinct `distinct` values) is over `maximum`; with incomplete files, every group, marked
/// partial (one open group where none). As `query::crossfile::budget`.
pub(in crate::code_rule_dsl) fn budget(
    (alias, relation): (&str, &Relation),
    predicate: Option<&Value>,
    group_by: &[String],
    distinct: &[String],
    maximum: usize,
    interrupted: fn() -> bool,
) -> Result<Result<Vec<Group>, Interrupted>, SqlError> {
    let aliases = [alias];
    let predicate = Predicate::compile(predicate, &aliases);
    let (group_by, distinct) = (
        Operand::named(group_by, &aliases),
        Operand::named(distinct, &aliases),
    );
    let poll = Poll {
        interrupted,
        visits: !relation.rows.is_empty(),
    };
    let mut lowering = Lowering::new(&["a"]);
    let filter = lowering.predicate(&predicate);
    let key = if group_by.is_empty() {
        "'all'".to_owned()
    } else {
        lowering.key(&group_by)
    };
    let (value, count) = if distinct.is_empty() {
        ("NULL".to_owned(), "row_count")
    } else {
        (lowering.key(&distinct), "distinct_values")
    };
    let partial = !relation.incomplete.is_empty();
    lowering
        .parameters
        .push(SqlValue::Integer(i64::from(partial)));
    let partial_parameter = lowering.parameters.len();
    // Exact: a count is at most the number of rows, below `i64::MAX`.
    lowering.parameters.push(SqlValue::Integer(
        i64::try_from(maximum).unwrap_or(i64::MAX),
    ));
    let maximum_parameter = lowering.parameters.len();
    let sql = format!(
        "WITH counted AS MATERIALIZED (
             SELECT rid, {key} AS group_key, {value} AS distinct_value FROM ({rows}) WHERE {filter}
         ),
         groups AS (
             SELECT group_key, MIN(rid) AS first, COUNT(*) AS row_count,
                    COUNT(DISTINCT distinct_value) AS distinct_values
             FROM counted GROUP BY group_key
         )
         SELECT first, group_key, {count} FROM groups
         WHERE ?{partial_parameter} OR {count} > ?{maximum_parameter}
         ORDER BY first",
        rows = lowering.pivot(0, 0),
    );
    with_connection(|connection| {
        if let Err(stopped) = poll.check() {
            return Ok(Err(stopped));
        }
        let transaction = connection.unchecked_transaction()?;
        transaction.execute_batch(CLEAR)?;
        load(&transaction, 0, &relation.rows, &lowering.relations[0].0)?;
        if let Err(stopped) = poll.check() {
            return Ok(Err(stopped));
        }
        let mut groups = Vec::new();
        {
            let mut statement = transaction.prepare_cached(&sql)?;
            let mut rows =
                statement.query(rusqlite::params_from_iter(lowering.parameters.iter()))?;
            while let Some(row) = rows.next()? {
                groups.push(Group {
                    first: row.get::<_, i64>(0)? as usize,
                    key: row.get(1)?,
                    count: row.get::<_, i64>(2)? as usize,
                    partial,
                });
            }
        }
        transaction.execute_batch(CLEAR)?;
        transaction.commit()?;
        if partial && groups.is_empty() {
            // Nothing counted and some file unknown: the whole domain is one open group.
            groups.push(Group {
                first: usize::MAX,
                key: String::new(),
                count: 0,
                partial: true,
            });
        }
        Ok(Ok(groups))
    })
}
