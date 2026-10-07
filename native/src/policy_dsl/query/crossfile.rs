//! Cross-file checks as statements over relation rows.
//!
//! A cross-file check reads relations: rows an AST query projects from every file of its domain
//! (`require_each` subjects and evidence, `budget` usages). Each file contributes its rows, or is
//! incomplete (its query failed or could not finish), which no absence or upper bound may ignore.
//! The check itself is one statement over the rows: an anti-join for `require_each`, a grouped
//! count for `budget`.

use rusqlite::{Connection, params};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

/// One relation's rows and the files whose rows are unknown.
#[derive(Default)]
pub(in crate::policy_dsl) struct Relation {
    /// Projected fields per row, in the order the caller reports them.
    pub(in crate::policy_dsl) rows: Vec<Map<String, Value>>,
    /// Files of the relation's domain whose rows are incomplete.
    pub(in crate::policy_dsl) incomplete: BTreeSet<String>,
}

/// What `require_each` found for one subject row.
#[derive(Debug, Eq, PartialEq)]
pub(in crate::policy_dsl) enum Requirement {
    /// No evidence row satisfies the predicate, and the evidence is complete.
    Missing(usize),
    /// No complete evidence row satisfies it, but some evidence file is incomplete: unknown.
    Unknown(usize),
}

/// One `budget` group over its limit, or one that could be.
#[derive(Debug, Eq, PartialEq)]
pub(in crate::policy_dsl) struct Group {
    /// The group's first row (its anchor).
    pub(in crate::policy_dsl) first: usize,
    /// The group's key: its `group_by` values' JSON joined by U+001F, or `all`.
    pub(in crate::policy_dsl) key: String,
    pub(in crate::policy_dsl) count: usize,
    /// The count is a lower bound: some file of the domain is incomplete.
    pub(in crate::policy_dsl) partial: bool,
}

/// Rows loaded between polls of `interrupted`.
const ROWS_PER_POLL: usize = 1024;

/// The statement's error once `interrupted` says so, as SQLite's own interruption reads.
fn interruption() -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_INTERRUPT),
        None,
    )
}

/// The relations' rows in one table, keyed by `(alias, row)`: a probe of one relation seeks its
/// rows alone, in order. On top, an `(alias, field)` expression index for each of `indexed`: the
/// fields a correlated probe seeks by value. The connection's statements fail once `interrupted`
/// says so: it is polled while rows load and every few thousand SQLite steps.
fn open(
    relations: &[(&str, &Relation)],
    indexed: &BTreeSet<&str>,
    interrupted: fn() -> bool,
) -> rusqlite::Result<Connection> {
    let connection = Connection::open_in_memory()?;
    connection.progress_handler(4096, Some(interrupted))?;
    connection.execute_batch(
        "CREATE TABLE relation(alias TEXT NOT NULL, row INTEGER NOT NULL, fields TEXT NOT NULL, \
         PRIMARY KEY(alias, row)) WITHOUT ROWID",
    )?;
    {
        let mut insert =
            connection.prepare("INSERT INTO relation(alias, row, fields) VALUES (?1, ?2, ?3)")?;
        for (alias, relation) in relations {
            for (index, row) in relation.rows.iter().enumerate() {
                if index % ROWS_PER_POLL == 0 && interrupted() {
                    return Err(interruption());
                }
                insert.execute(params![
                    alias,
                    index as i64,
                    Value::Object(row.clone()).to_string()
                ])?;
            }
        }
    }
    #[cfg(test)]
    INDEXED_FIELDS.with(|recorded| {
        *recorded.borrow_mut() = indexed.iter().map(|field| (*field).to_owned()).collect();
    });
    for (index, field) in indexed.iter().enumerate() {
        connection.execute_batch(&format!(
            "CREATE INDEX relation_field_{index} ON relation(alias, fields -> {})",
            quote(&json_path_key(field))
        ))?;
    }
    Ok(connection)
}

#[cfg(test)]
thread_local! {
    /// The fields the last `open` on this thread indexed, and the query plan of the last
    /// `require_each` statement.
    static INDEXED_FIELDS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    static REQUIRE_EACH_PLAN: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

#[cfg(test)]
fn record_plan(connection: &Connection, sql: &str) -> rusqlite::Result<()> {
    let plan = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?
        .query_map([], |row| row.get::<_, String>(3))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .join("\n");
    REQUIRE_EACH_PLAN.with(|recorded| *recorded.borrow_mut() = plan);
    Ok(())
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

fn json_path_key(field: &str) -> String {
    format!("$.\"{}\"", field.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The relation field an operand reads: `(table, field)` for `alias.field` of a bound alias.
fn bound_field<'v, 't>(
    value: &'v Value,
    aliases: &[(&str, &'t str)],
) -> Option<(&'t str, &'v str)> {
    let (alias, field) = value.as_str()?.split_once('.')?;
    let (_, table) = aliases.iter().find(|(name, _)| *name == alias)?;
    Some((table, field))
}

/// An operand as the JSON text of its value: `alias.field` of a bound relation reads the row's
/// field (`->` keeps the JSON type, so `"1"` and `1` differ, as `Value` equality does); anything
/// else is a literal.
fn operand(value: &Value, aliases: &[(&str, &str)]) -> String {
    if let Some((table, field)) = bound_field(value, aliases) {
        return format!("({table}.fields -> {})", quote(&json_path_key(field)));
    }
    quote(&value.to_string())
}

/// The fields of `table` an `eq` of `predicate` compares with a value from elsewhere: what a
/// probe of `table` can seek by value. `ne` and anything under `not` seek no value; the probe
/// still seeks its relation's rows by the table's key. Mirrors `condition`'s reading of the
/// predicate.
fn probed_fields<'v>(
    predicate: Option<&'v Value>,
    aliases: &[(&str, &str)],
    table: &str,
    fields: &mut BTreeSet<&'v str>,
) {
    let Some(object) = predicate.and_then(Value::as_object) else {
        return;
    };
    if let Some(items) = object.get("all").or_else(|| object.get("any")) {
        for item in items.as_array().into_iter().flatten() {
            probed_fields(Some(item), aliases, table, fields);
        }
        return;
    }
    if object.contains_key("not") {
        return;
    }
    if let Some([left, right]) = object
        .get("eq")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
    {
        let (left, right) = (bound_field(left, aliases), bound_field(right, aliases));
        for (probed, other) in [(left, right), (right, left)] {
            if let Some((probed_table, field)) = probed
                && probed_table == table
                && other.is_none_or(|(other_table, _)| other_table != table)
            {
                fields.insert(field);
            }
        }
    }
}

/// A `where` predicate (`all`, `any`, `not`, `eq`, `ne`) as a SQL condition. `aliases` maps
/// each bound alias to its table alias in the statement.
pub(in crate::policy_dsl) fn condition(
    predicate: Option<&Value>,
    aliases: &[(&str, &str)],
) -> String {
    let Some(predicate) = predicate else {
        return "1".to_owned();
    };
    let Some(object) = predicate.as_object() else {
        return "0".to_owned();
    };
    let join = |items: &Value, separator: &str, empty: &str| {
        let parts = items
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|item| format!("({})", condition(Some(item), aliases)))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if parts.is_empty() {
            empty.to_owned()
        } else {
            parts.join(separator)
        }
    };
    if let Some(items) = object.get("all") {
        return join(items, " AND ", "1");
    }
    if let Some(items) = object.get("any") {
        return join(items, " OR ", "0");
    }
    if let Some(inner) = object.get("not") {
        return format!("NOT ({})", condition(Some(inner), aliases));
    }
    for (key, operator) in [("eq", "IS"), ("ne", "IS NOT")] {
        if let Some(operands) = object.get(key).and_then(Value::as_array)
            && operands.len() == 2
        {
            return format!(
                "{} {operator} {}",
                operand(&operands[0], aliases),
                operand(&operands[1], aliases)
            );
        }
    }
    "0".to_owned()
}

/// `require_each`: the subject rows (passing `subject_predicate`) no evidence row satisfies
/// `predicate` for. Where evidence is incomplete, such a subject is unknown, never missing. Fails
/// once `interrupted` says so.
pub(in crate::policy_dsl) fn require_each(
    (subject_alias, subject): (&str, &Relation),
    subject_predicate: Option<&Value>,
    (evidence_alias, evidence): (&str, &Relation),
    predicate: Option<&Value>,
    interrupted: fn() -> bool,
) -> rusqlite::Result<Vec<Requirement>> {
    let aliases = [(subject_alias, "s"), (evidence_alias, "e")];
    let mut probed = BTreeSet::new();
    probed_fields(predicate, &aliases, "e", &mut probed);
    let connection = open(
        &[("subject", subject), ("evidence", evidence)],
        &probed,
        interrupted,
    )?;
    let sql = format!(
        "SELECT s.row FROM relation s WHERE s.alias = 'subject' AND ({}) \
         AND NOT EXISTS (SELECT 1 FROM relation e WHERE e.alias = 'evidence' AND ({})) \
         ORDER BY s.row",
        condition(subject_predicate, &aliases[..1]),
        condition(predicate, &aliases),
    );
    #[cfg(test)]
    record_plan(&connection, &sql)?;
    let mut statement = connection.prepare(&sql)?;
    let unknown = !evidence.incomplete.is_empty();
    statement
        .query_map([], |row| {
            let index = row.get::<_, i64>(0)? as usize;
            Ok(if unknown {
                Requirement::Unknown(index)
            } else {
                Requirement::Missing(index)
            })
        })?
        .collect()
}

/// `budget`: groups of the rows passing `predicate`, by `group_by` operands, whose count (of rows,
/// or of distinct `distinct` values) is over `maximum`; with incomplete files, every group, marked
/// partial, since any count may be a lower bound. Rows keep their order; a group's anchor is its
/// first row. Fails once `interrupted` says so.
pub(in crate::policy_dsl) fn budget(
    (alias, relation): (&str, &Relation),
    predicate: Option<&Value>,
    group_by: &[String],
    distinct: &[String],
    maximum: usize,
    interrupted: fn() -> bool,
) -> rusqlite::Result<Vec<Group>> {
    let connection = open(&[("rows", relation)], &BTreeSet::new(), interrupted)?;
    let aliases = [(alias, "r")];
    let key = |fields: &[String]| {
        fields
            .iter()
            .map(|field| {
                format!(
                    "coalesce({}, 'null')",
                    operand(&Value::String(field.clone()), &aliases)
                )
            })
            .collect::<Vec<_>>()
            .join(" || char(31) || ")
    };
    let group = if group_by.is_empty() {
        "'all'".to_owned()
    } else {
        key(group_by)
    };
    let count = if distinct.is_empty() {
        "count(*)".to_owned()
    } else {
        format!("count(DISTINCT {})", key(distinct))
    };
    let partial = !relation.incomplete.is_empty();
    let sql = format!(
        "SELECT min(r.row), {group}, {count} FROM relation r WHERE r.alias = 'rows' AND ({}) \
         GROUP BY 2 {} ORDER BY 1",
        condition(predicate, &aliases),
        if partial {
            String::new()
        } else {
            format!("HAVING {count} > {maximum}")
        },
    );
    let mut statement = connection.prepare(&sql)?;
    let mut groups = statement
        .query_map([], |row| {
            Ok(Group {
                first: row.get::<_, i64>(0)? as usize,
                key: row.get(1)?,
                count: row.get::<_, i64>(2)? as usize,
                partial,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if partial && groups.is_empty() {
        // Nothing counted and some file unknown: the whole domain is one open group.
        groups.push(Group {
            first: usize::MAX,
            key: String::new(),
            count: 0,
            partial: true,
        });
    }
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn relation(rows: Vec<Value>, incomplete: &[&str]) -> Relation {
        Relation {
            rows: rows
                .into_iter()
                .map(|row| row.as_object().unwrap().clone())
                .collect(),
            incomplete: incomplete.iter().map(|path| (*path).to_owned()).collect(),
        }
    }

    fn running() -> bool {
        false
    }

    #[test]
    fn require_each_is_an_anti_join_and_incomplete_evidence_proves_no_absence() {
        let components = relation(
            vec![
                json!({"name": "Button", "dir": "ui"}),
                json!({"name": "Card", "dir": "ui"}),
            ],
            &[],
        );
        let stories = relation(vec![json!({"name": "Button", "dir": "ui"})], &[]);
        let join = json!({"all": [{"eq": ["c.name", "s.name"]}, {"eq": ["c.dir", "s.dir"]}]});
        assert_eq!(
            require_each(
                ("c", &components),
                None,
                ("s", &stories),
                Some(&join),
                running
            )
            .unwrap(),
            [Requirement::Missing(1)]
        );
        let partial = relation(
            vec![json!({"name": "Button", "dir": "ui"})],
            &["ui/Broken.stories.tsx"],
        );
        assert_eq!(
            require_each(
                ("c", &components),
                None,
                ("s", &partial),
                Some(&join),
                running
            )
            .unwrap(),
            [Requirement::Unknown(1)]
        );
    }

    #[test]
    fn fields_named_with_path_syntax_are_read_as_their_own_values() {
        // A double quote, a dot, a dollar, a single quote or a backslash in a field name must
        // still address that field: were both sides NULL, `IS` would join every subject to
        // unrelated evidence, and every row would group under `null`.
        for field in ["a\"b", "a.b", "$x", "it's", "c\\d"] {
            let subjects = relation(vec![json!({ field: "Button" })], &[]);
            let evidence = relation(vec![json!({ field: "Card" })], &[]);
            let join = json!({"eq": [format!("c.{field}"), format!("s.{field}")]});
            assert_eq!(
                require_each(
                    ("c", &subjects),
                    None,
                    ("s", &evidence),
                    Some(&join),
                    running
                )
                .unwrap(),
                [Requirement::Missing(0)],
                "{field}"
            );
            let groups = [format!("c.{field}")];
            assert_eq!(
                budget(("c", &subjects), None, &groups, &[], 0, running).unwrap(),
                [Group {
                    first: 0,
                    key: "\"Button\"".to_owned(),
                    count: 1,
                    partial: false
                }],
                "{field}"
            );
        }
    }

    /// Once `interrupted` says so, a cross-file statement fails as SQLite's own interruption
    /// does, instead of loading every row and answering late.
    #[test]
    fn an_interrupted_statement_fails_instead_of_answering() {
        fn stopped() -> bool {
            true
        }
        let subjects = relation(vec![json!({"name": "Button"})], &[]);
        let evidence = relation(vec![json!({"name": "Card"})], &[]);
        let join = json!({"eq": ["c.name", "s.name"]});
        let interrupted = |result: rusqlite::Result<()>| {
            matches!(
                result,
                Err(rusqlite::Error::SqliteFailure(error, _))
                    if error.code == rusqlite::ErrorCode::OperationInterrupted
            )
        };
        assert!(interrupted(
            require_each(
                ("c", &subjects),
                None,
                ("s", &evidence),
                Some(&join),
                stopped
            )
            .map(|_| ())
        ));
        assert!(interrupted(
            budget(("c", &subjects), None, &[], &[], 0, stopped).map(|_| ())
        ));
    }

    #[test]
    fn require_each_reads_projected_fields_with_quotes() {
        let subjects = relation(vec![json!({"na\"me": "Button"})], &[]);
        let evidence = relation(vec![json!({"na\"me": "Card"})], &[]);
        let join = json!({"eq": ["subject.na\"me", "evidence.na\"me"]});
        assert_eq!(
            require_each(
                ("subject", &subjects),
                None,
                ("evidence", &evidence),
                Some(&join),
                running
            )
            .unwrap(),
            vec![Requirement::Missing(0)]
        );
    }

    #[test]
    fn require_each_uses_an_indexed_evidence_probe() {
        let subjects = relation(
            (0..32)
                .map(|index| json!({"name": format!("subject-{index}"), "note": index}))
                .collect(),
            &[],
        );
        let evidence = relation(
            (0..2_000)
                .map(|index| json!({"name": format!("evidence-{index}"), "extra": index}))
                .collect(),
            &[],
        );
        let join = json!({"eq": ["subject.name", "evidence.name"]});
        assert_eq!(
            require_each(
                ("subject", &subjects),
                None,
                ("evidence", &evidence),
                Some(&join),
                running
            )
            .unwrap()
            .len(),
            32
        );
        let plan = REQUIRE_EACH_PLAN.with(|plan| plan.borrow().clone());
        assert!(
            plan.contains("SEARCH e USING") && plan.contains("<expr>=?"),
            "the evidence probe seeks the joined value: {plan}"
        );
        assert_eq!(
            INDEXED_FIELDS.with(|indexed| indexed.borrow().clone()),
            ["name"],
            "only the evidence field the predicate joins on is indexed"
        );
    }

    /// A predicate with no `eq` to seek by still probes the evidence rows alone, never every
    /// subject and evidence row.
    #[test]
    fn require_each_without_an_eq_probes_only_evidence_rows() {
        let subjects = relation(
            ["a", "b", "c"]
                .into_iter()
                .map(|owner| json!({"owner": owner}))
                .collect(),
            &[],
        );
        let evidence = relation(vec![json!({"owner": "a"}), json!({"owner": "a"})], &[]);
        let differ = json!({"ne": ["subject.owner", "evidence.owner"]});
        let same = json!({"not": differ.clone()});
        for (predicate, missing) in [(differ, vec![0]), (same, vec![1, 2])] {
            assert_eq!(
                require_each(
                    ("subject", &subjects),
                    None,
                    ("evidence", &evidence),
                    Some(&predicate),
                    running
                )
                .unwrap(),
                missing
                    .into_iter()
                    .map(Requirement::Missing)
                    .collect::<Vec<_>>(),
                "{predicate}"
            );
            let plan = REQUIRE_EACH_PLAN.with(|plan| plan.borrow().clone());
            assert!(
                plan.contains("SEARCH e USING") && !plan.contains("SCAN e"),
                "{predicate}: the evidence probe seeks the evidence alias: {plan}"
            );
        }
    }

    #[test]
    fn budget_builds_no_field_indexes() {
        let usages = relation(
            vec![
                json!({"package": "a", "path": "x"}),
                json!({"package": "a", "path": "y"}),
            ],
            &[],
        );
        let groups = ["u.package".to_owned()];
        assert_eq!(
            budget(("u", &usages), None, &groups, &[], 1, running)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            INDEXED_FIELDS.with(|indexed| indexed.borrow().clone()),
            Vec::<String>::new(),
            "a grouped count reads every row; no probe uses a field index"
        );
    }

    #[test]
    fn require_each_preserves_json_type_and_null_equality() {
        let subjects = relation(
            vec![json!({"key": 1}), json!({"key": null}), json!({"key": "1"})],
            &[],
        );
        let evidence = relation(vec![json!({"key": "1"}), json!({"key": null})], &[]);
        let join = json!({"eq": ["subject.key", "evidence.key"]});
        assert_eq!(
            require_each(
                ("subject", &subjects),
                None,
                ("evidence", &evidence),
                Some(&join),
                running
            )
            .unwrap(),
            [Requirement::Missing(0)]
        );
    }

    #[test]
    fn budget_counts_groups_and_never_calls_a_lower_bound_clean() {
        let usages = relation(
            vec![
                json!({"package": "a"}),
                json!({"package": "b"}),
                json!({"package": "a"}),
            ],
            &[],
        );
        let groups = ["u.package".to_owned()];
        assert_eq!(
            budget(("u", &usages), None, &groups, &[], 1, running).unwrap(),
            [Group {
                first: 0,
                key: "\"a\"".to_owned(),
                count: 2,
                partial: false
            }]
        );
        assert_eq!(
            budget(
                ("u", &usages),
                None,
                &groups,
                &["u.package".to_owned()],
                1,
                running
            )
            .unwrap(),
            [],
            "two rows of one distinct value count once"
        );
        let partial = relation(vec![json!({"package": "a"})], &["b/x.py"]);
        assert_eq!(
            budget(("u", &partial), None, &groups, &[], 1, running).unwrap(),
            [Group {
                first: 0,
                key: "\"a\"".to_owned(),
                count: 1,
                partial: true
            }]
        );
    }
}
