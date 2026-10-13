//! The arguments a SQL call passes, as PostgreSQL 18.6's parser (`gram.y`, `func_arg_list`,
//! `func_arg_expr`, `extract_list`, `position_list`, `substr_list`, `overlay_list`, `trim_list`)
//! reads them, over the DerekStride grammar's `invocation`.

use crate::code_rule_dsl::value::sql;
use ast_grep_core::{Doc, Node};

/// The grammar's call node: an `object_reference` callee, then `parameter:` children.
#[allow(dead_code)]
pub(crate) const SQL_CALL_KINDS: &[&str] = &["invocation"];

/// One argument of a call.
pub(crate) struct SqlArgument<'r, D: Doc> {
    /// A named argument's name as PostgreSQL stores it (`name => v` and `name := v`).
    pub(crate) name: Option<String>,
    /// The argument's expression: a `term`'s `value`, a named argument's `value`.
    pub(crate) value: Node<'r, D>,
}

/// The arguments of `call`, an `invocation`, in the order PostgreSQL passes them.
///
/// * `DISTINCT`/`ALL`, `ORDER BY` and `FILTER (...)` are not arguments and are skipped, as are
///   comments and the keywords of the SQL-syntax forms.
/// * A leading `ALL` (read by the grammar as a unary operator) is dropped from the first argument.
/// * `count(*)` passes no arguments (`Some(vec![])`): `FuncCall.agg_star` carries the star.
/// * `extract(unit FROM x)` passes the unit first (`unit:` child, then the bare `term` `x`).
/// * `position(sub IN str)` and `trim([both|leading|trailing] chars FROM str)` are written in the
///   opposite order to the call PostgreSQL builds (`position_list: b_expr IN_P b_expr` gives
///   `list_make2($3, $1)`, `TRIM ... trim_list: a_expr FROM expr_list` gives
///   `lappend($3, $1)`), so their two operands are swapped to `(str, sub)`. `substring` and
///   `overlay` operands keep their source order.
/// * `VARIADIC x`: the grammar reads it as a `field` named `VARIADIC` with `x` as its `alias`.
///   The `VARIADIC` marker is dropped and the argument is `x` (the alias identifier), positioned
///   last as PostgreSQL does (`func_arg_list` keeps it in place, `variadic` only flags the call).
///   Only the single-identifier operand is read that way; the grammar errors on other operands,
///   and then the result is `None` for the unreadable shape rather than a guess.
///
/// `None` when an argument is not one of those shapes (a parse error inside the call).
#[allow(dead_code)]
pub(crate) fn sql_arguments<'r, D: Doc>(call: &Node<'r, D>) -> Option<Vec<SqlArgument<'r, D>>> {
    if call.kind() != "invocation" || call.children().any(|child| child.is_error()) {
        return None;
    }
    let mut operands: Vec<Node<'r, D>> = call
        .field_children("parameter")
        .chain(call.field_children("unit"))
        .collect();
    // `extract(unit FROM source)`: the source is a bare `term` child, not a `parameter:`.
    for child in call.children() {
        if child.kind() == "term" && operands.iter().all(|o| o.range() != child.range()) {
            operands.push(child);
        }
    }
    operands.sort_by_key(|operand| operand.range().start);

    let mut arguments = Vec::with_capacity(operands.len());
    for operand in &operands {
        arguments.push(read_operand(operand)?);
    }
    if let Some(first) = arguments.first_mut()
        && let Some(operand) = all_quantified_operand(&first.value)
    {
        first.value = operand;
    }
    if is_swapped_form(call, arguments.len()) {
        arguments.swap(0, 1);
    }
    if arguments.len() == 1 && is_star(&operands[0]) {
        arguments.clear();
    }
    Some(arguments)
}

fn read_operand<'r, D: Doc>(operand: &Node<'r, D>) -> Option<SqlArgument<'r, D>> {
    match operand.kind().as_ref() {
        "named_argument" => Some(SqlArgument {
            name: sql::node_name(&operand.field("name")?),
            value: operand.field("value")?,
        })
        .filter(|argument| argument.name.is_some()),
        "term" => {
            if let Some(alias) = operand.field("alias") {
                let marker = operand.field("value")?;
                let is_variadic = marker.kind() == "field"
                    && marker.text().eq_ignore_ascii_case("variadic")
                    && alias.kind() == "identifier";
                return is_variadic.then_some(SqlArgument {
                    name: None,
                    value: alias,
                });
            }
            Some(SqlArgument {
                name: None,
                value: operand.field("value")?,
            })
        }
        "object_reference" | "literal" => Some(SqlArgument {
            name: None,
            value: operand.clone(),
        }),
        _ => None,
    }
}

/// `f(ALL a)`: the grammar reads the leading `ALL` as a unary operator on the first argument.
fn all_quantified_operand<'r, D: Doc>(value: &Node<'r, D>) -> Option<Node<'r, D>> {
    (value.kind() == "unary_expression"
        && value
            .field("operator")
            .is_some_and(|op| op.kind() == "keyword_all"))
    .then(|| value.field("operand"))
    .flatten()
}

/// `count(*)`: a lone `*` term.
fn is_star<D: Doc>(operand: &Node<'_, D>) -> bool {
    operand.kind() == "term"
        && operand.field("value").is_some_and(|value| {
            value.kind() == "all_fields" && value.children().all(|c| !c.is_named())
        })
}

/// `position(a IN b)` and `trim(... a FROM b)`, whose two operands PostgreSQL passes as `(b, a)`.
fn is_swapped_form<D: Doc>(call: &Node<'_, D>, count: usize) -> bool {
    if count != 2 {
        return false;
    }
    let has = |kind: &str| call.children().any(|child| child.kind() == kind);
    if has("keyword_in") {
        return true;
    }
    has("keyword_from")
        && call
            .field("function")
            .or_else(|| call.children().next())
            .and_then(|callee| sql::node_name(&callee.field("name")?))
            .is_some_and(|name| name == "trim")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::value::sql::test_grammar;
    use serde_json::Value as Json;

    /// 1-based line and 1-based code point column of a byte offset.
    fn position(source: &str, byte: usize) -> (u64, u64) {
        let before = &source[..byte];
        let line = before.matches('\n').count() as u64 + 1;
        let start = before.rfind('\n').map_or(0, |i| i + 1);
        (line, before[start..].chars().count() as u64 + 1)
    }

    /// Oracle calls the grammar does not read as an `invocation`, by (name, line): none; the
    /// oracle holds no `cast` (PostgreSQL's TypeCast is not a FuncCall).
    const MISMATCHES: &[(&str, u64, &str)] = &[];

    #[test]
    fn matches_postgres_oracle() {
        let dir = std::path::Path::new(crate::code_rule_dsl::fixtures_dir!()).join("sql/arguments");
        let source = std::fs::read_to_string(dir.join("postgres.sql")).unwrap();
        let oracle: Json = serde_json::from_str(
            &std::fs::read_to_string(dir.join("postgres.expected.json")).unwrap(),
        )
        .unwrap();
        let grep = test_grammar::parse(&source);
        let root = grep.root();
        let invocations: Vec<_> = root.dfs().filter(|n| n.kind() == "invocation").collect();

        let (mut matched, mut skipped) = (0, 0);
        for call in oracle["calls"].as_array().unwrap() {
            let at = (
                call["line"].as_u64().unwrap(),
                call["column"].as_u64().unwrap(),
            );
            let found = invocations
                .iter()
                .find(|n| position(&source, n.range().start) == at);
            let Some(node) = found else {
                assert!(
                    MISMATCHES.iter().any(|(name, line, _)| {
                        *line == at.0 && call["name"][0].as_str() == Some(name)
                    }),
                    "no invocation at {at:?}: {call}"
                );
                skipped += 1;
                continue;
            };
            let arguments = sql_arguments(node).unwrap_or_else(|| panic!("None at {at:?}"));
            let expected = call["args"].as_array().unwrap();
            assert_eq!(arguments.len(), expected.len(), "count at {at:?}: {call}");
            for (argument, want) in arguments.iter().zip(expected) {
                let got_value = position(&source, argument.value.range().start);
                let want_value = (
                    want.get("valueLine")
                        .unwrap_or(&want["line"])
                        .as_u64()
                        .unwrap(),
                    want.get("valueColumn")
                        .unwrap_or(&want["column"])
                        .as_u64()
                        .unwrap(),
                );
                assert_eq!(got_value, want_value, "value at {at:?}: {want}");
                assert_eq!(
                    argument.name.as_deref(),
                    want.get("name").and_then(Json::as_str),
                    "name at {at:?}: {want}"
                );
            }
            matched += 1;
        }
        eprintln!("matched {matched}, mismatches {skipped}");
        assert_eq!(skipped, MISMATCHES.len());
        assert!(matched > 40);
    }

    #[test]
    fn comments_and_keywords_are_not_arguments() {
        let grep = test_grammar::parse("SELECT f(DISTINCT a, /* c */ b ORDER BY a)");
        let call = grep
            .root()
            .dfs()
            .find(|n| n.kind() == "invocation")
            .unwrap();
        let values: Vec<_> = sql_arguments(&call)
            .unwrap()
            .iter()
            .map(|a| a.value.text().to_string())
            .collect();
        assert_eq!(values, ["a", "b"]);
    }
}
