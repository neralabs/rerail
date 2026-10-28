//! Write an input relation's projected rows into batched SQLite writes.
use super::{FactQuery, Rows, insert};
use crate::code_rule_dsl::{
    eval,
    model::{Anchor, CompiledClause, CompiledCodeRule, Metrics, Snapshot},
    violations::anchored_span,
};
use napi::Result;
use rusqlite::Connection;
use std::sync::{Arc, atomic::AtomicUsize};


/// A JSON projected value as the SQL value its column stores.
pub(super) fn sql(value: Option<&serde_json::Value>) -> rusqlite::types::Value {
    use rusqlite::types::Value as Sql;
    use serde_json::Value;
    match value {
        None | Some(Value::Null) => Sql::Null,
        Some(Value::Bool(b)) => Sql::Integer(i64::from(*b)),
        Some(Value::Number(n)) => n
            .as_i64()
            .map_or_else(|| Sql::Real(n.as_f64().unwrap_or(f64::NAN)), Sql::Integer),
        Some(Value::String(s)) => Sql::Text(s.clone()),
        Some(other) => Sql::Text(other.to_string()),
    }
}
#[allow(clippy::too_many_arguments)]
pub(in crate::code_rule_dsl) fn stream(
    db: &Connection,
    table: &str,
    input: &FactQuery,
    work: &Arc<AtomicUsize>,
    code_rule: &CompiledCodeRule,
    clause: &CompiledClause,
    snapshot: &Snapshot,
    metrics: &mut Metrics,
) -> Result<Rows> {
    #[cfg(feature = "code-rule-profile")]
    let mut trace = crate::code_rule_dsl::profile::trace::Span::start("input-stream", table);
    // W's witnesses carry their projection as one JSON object (P streamed typed column chunks
    // through `stateless_rows_to`): the rows are written once the relation is read.
    let rows = eval::stateless_rows(
        code_rule,
        clause,
        (input, eval::evidence::Side::Counted),
        snapshot,
        metrics,
    )?;
    let project = &input.ast.project;
    insert::insert_rows(
        db,
        table,
        project.len() + 3,
        rows.witnesses.iter().map(|witness| {
            let location = anchored_span(&Anchor::Span, witness).0;
            let projected = witness.captures.projection();
            [
                rusqlite::types::Value::Text(witness.path.clone()),
                rusqlite::types::Value::Integer(location.start_byte as i64),
                rusqlite::types::Value::Integer(location.end_byte as i64),
            ]
            .into_iter()
            .chain(
                project
                    .iter()
                    .map(move |p| sql(projected.and_then(|value| value.get(&p.field)))),
            )
        }),
        work,
        "relational input row budget exceeded",
    )?;
    #[cfg(feature = "code-rule-profile")]
    trace.detail(|| serde_json::json!({"rows":rows.witnesses.len(),"incompleteFiles":rows.incomplete.len()}));
    Ok(rows)
}
