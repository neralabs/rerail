//! Typed SQL for the logic language's plans (05 Phase 5 row 4), from
//! ner-1562-scale-test-the-re-railer's `query/plan/render/{sql_ast, typed_boundary}.rs` and
//! `query/plan/physical.rs`: a SQL AST rendered and checked against the relations a statement may
//! read, so emitted SQL names only declared fact families. N's `relational_ir.rs`/`relational.rs`
//! (tree axes in SQL) are not kept: tree navigation is the transducer's.
pub(in crate::code_rule_dsl) mod physical;
pub(in crate::code_rule_dsl) mod select;
pub(in crate::code_rule_dsl) mod sql_ast;
pub(in crate::code_rule_dsl) mod typed_boundary;

pub(in crate::code_rule_dsl) mod program;
pub(in crate::code_rule_dsl) mod lattices;
pub(in crate::code_rule_dsl) mod properties;

/// SQLite's keywords (sqlite.org/lang_keywords.html, 147 of them), sorted.
const SQLITE_KEYWORDS: [&str; 147] = [
    "ABORT", "ACTION", "ADD", "AFTER", "ALL", "ALTER", "ALWAYS", "ANALYZE", "AND", "AS", "ASC", "ATTACH",
    "AUTOINCREMENT", "BEFORE", "BEGIN", "BETWEEN", "BY", "CASCADE", "CASE", "CAST", "CHECK", "COLLATE", "COLUMN",
    "COMMIT", "CONFLICT", "CONSTRAINT", "CREATE", "CROSS", "CURRENT", "CURRENT_DATE", "CURRENT_TIME",
    "CURRENT_TIMESTAMP", "DATABASE", "DEFAULT", "DEFERRABLE", "DEFERRED", "DELETE", "DESC", "DETACH", "DISTINCT",
    "DO", "DROP", "EACH", "ELSE", "END", "ESCAPE", "EXCEPT", "EXCLUDE", "EXCLUSIVE", "EXISTS", "EXPLAIN", "FAIL",
    "FILTER", "FIRST", "FOLLOWING", "FOR", "FOREIGN", "FROM", "FULL", "GENERATED", "GLOB", "GROUP", "GROUPS",
    "HAVING", "IF", "IGNORE", "IMMEDIATE", "IN", "INDEX", "INDEXED", "INITIALLY", "INNER", "INSERT", "INSTEAD",
    "INTERSECT", "INTO", "IS", "ISNULL", "JOIN", "KEY", "LAST", "LEFT", "LIKE", "LIMIT", "MATCH", "MATERIALIZED",
    "NATURAL", "NO", "NOT", "NOTHING", "NOTNULL", "NULL", "NULLS", "OF", "OFFSET", "ON", "OR", "ORDER", "OTHERS",
    "OUTER", "OVER", "PARTITION", "PLAN", "PRAGMA", "PRECEDING", "PRIMARY", "QUERY", "RAISE", "RANGE", "RECURSIVE",
    "REFERENCES", "REGEXP", "REINDEX", "RELEASE", "RENAME", "REPLACE", "RESTRICT", "RETURNING", "RIGHT", "ROLLBACK",
    "ROW", "ROWS", "SAVEPOINT", "SELECT", "SET", "TABLE", "TEMP", "TEMPORARY", "THEN", "TIES", "TO", "TRANSACTION",
    "TRIGGER", "UNBOUNDED", "UNION", "UNIQUE", "UPDATE", "USING", "VACUUM", "VALUES", "VIEW", "VIRTUAL", "WHEN",
    "WHERE", "WINDOW", "WITH", "WITHOUT",
];

/// A table as SQL: its name, perhaps schema-qualified (`x.values`), with each dotted part an identifier (`ident`).
/// A part already quoted is kept, and so is what follows the name (setfix's inputs carry a filter, `t WHERE c0 = 1`).
pub(in crate::code_rule_dsl) fn table(name: &str) -> String {
    let (name, rest) = name.split_once(' ').map_or((name, ""), |(n, r)| (n, r));
    let quoted = name.split('.').map(|part| if part.starts_with('"') { std::borrow::Cow::Borrowed(part) } else { ident(part) }).collect::<Vec<_>>().join(".");
    if rest.is_empty() { quoted } else { format!("{quoted} {rest}") }
}
/// A relation's name as an SQL identifier: as written, unless it is a SQLite keyword (`values`, `order`) or not a
/// plain identifier, which is quoted. Every SQL statement that names a program's relation writes it so (O106).
pub(in crate::code_rule_dsl) fn ident(name: &str) -> std::borrow::Cow<'_, str> {
    let plain = name.bytes().next().is_some_and(|c| c.is_ascii_alphabetic() || c == b'_') && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_');
    if plain && SQLITE_KEYWORDS.binary_search(&name.to_ascii_uppercase().as_str()).is_err() {
        std::borrow::Cow::Borrowed(name)
    } else {
        std::borrow::Cow::Owned(format!("\"{}\"", name.replace('"', "\"\"")))
    }
}
