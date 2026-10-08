//! `evidence`: a cross-file relation projected from an AST query's matches.

use super::define::Defines;
use super::{compile_glob_set, compile_query, string_array};
use crate::code_rule_dsl::json::{invalid, non_empty_str, object, reject_unknown};
use crate::code_rule_dsl::model::{
    AstRelation, Capabilities, CodeRuleScope, FactQuery, Projection, ProjectionSource, Query,
};
use napi::Result;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::Arc;

/// An AST `evidence` relation projected from matches across the code rule's scoped files.
pub(super) fn compile_relation(
    value: &Value,
    path: &str,
    capabilities: &Capabilities,
    defines: &Defines,
    required_ast: &mut BTreeSet<(String, String)>,
) -> Result<FactQuery> {
    let wrapper = object(value, path)?;
    reject_unknown(wrapper, &["evidence"], path)?;
    let path = format!("{path}/evidence");
    let evidence = object(&wrapper["evidence"], &path)?;
    reject_unknown(
        evidence,
        &["as", "paths", "query", "project", "where"],
        &path,
    )?;
    let alias = non_empty_str(evidence.get("as"), &format!("{path}/as"))?.to_owned();
    let query = compile_query(
        evidence
            .get("query")
            .ok_or_else(|| invalid(format!("{path}/query is required")))?,
        &format!("{path}/query"),
        capabilities,
        defines,
        required_ast,
    )?;
    if !matches!(query, Query::Ast { .. }) {
        return Err(invalid(format!("{path}/query must be an ast query")));
    }
    if !query.calls().is_empty() {
        return Err(invalid(format!(
            "{path}/query: calls is not supported in evidence"
        )));
    }
    let project_value = evidence
        .get("project")
        .ok_or_else(|| invalid(format!("{path}/project is required")))?;
    let project = object(project_value, &format!("{path}/project"))?
        .iter()
        .map(|(field, source)| {
            let at = format!("{path}/project/{field}");
            let source = non_empty_str(Some(source), &at)?;
            let source = match source {
                "file.path" => ProjectionSource::Path,
                "file.directory" => ProjectionSource::Directory,
                "file.name" => ProjectionSource::Name,
                "file.stem" => ProjectionSource::Stem,
                "file.module" => ProjectionSource::Module,
                "file.package" => ProjectionSource::Package,
                other => match other.strip_prefix("capture.") {
                    Some(name) if !name.is_empty() => ProjectionSource::Capture(name.to_owned()),
                    _ => {
                        return Err(invalid(format!(
                            "{at}: unknown source `{other}`; expected capture.<NAME>, file.path, \
                             file.directory, file.name, file.stem, file.module or file.package"
                        )));
                    }
                },
            };
            Ok(Projection {
                field: field.clone(),
                source,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    if project.is_empty() {
        return Err(invalid(format!(
            "{path}/project must name at least one field"
        )));
    }
    let domain = evidence
        .get("paths")
        .map(|paths| -> Result<CodeRuleScope> {
            let at = format!("{path}/paths");
            let object = object(paths, &at)?;
            reject_unknown(object, &["include", "exclude"], &at)?;
            let include = if object.contains_key("include") {
                string_array(paths, "include", &at)?
            } else {
                vec!["**".to_owned()]
            };
            let exclude = if object.contains_key("exclude") {
                string_array(paths, "exclude", &at)?
            } else {
                Vec::new()
            };
            Ok(CodeRuleScope {
                include: compile_glob_set(&include, &format!("{at}/include"))?,
                exclude: compile_glob_set(&exclude, &format!("{at}/exclude"))?,
            })
        })
        .transpose()?;
    let predicate = evidence.get("where").cloned();
    if let Some(predicate) = &predicate {
        validate_predicate(predicate, &format!("{path}/where"))?;
    }
    Ok(FactQuery {
        alias,
        predicate,
        ast: Arc::new(AstRelation {
            query,
            project,
            domain,
        }),
    })
}

/// An evidence `where` is a boolean over `eq`/`ne` comparisons.
fn validate_predicate(value: &Value, path: &str) -> Result<()> {
    let predicate = object(value, path)?;
    let mut keys = predicate.keys();
    let (Some(operator), None) = (keys.next(), keys.next()) else {
        return Err(invalid(format!("{path} must contain exactly one operator")));
    };
    match operator.as_str() {
        "all" | "any" => {
            for (index, item) in predicate[operator]
                .as_array()
                .ok_or_else(|| invalid(format!("{path}/{operator} must be an array")))?
                .iter()
                .enumerate()
            {
                validate_predicate(item, &format!("{path}/{operator}/{index}"))?;
            }
            Ok(())
        }
        "not" => validate_predicate(&predicate[operator], &format!("{path}/not")),
        "eq" | "ne" => match predicate[operator].as_array() {
            Some(operands) if operands.len() == 2 => Ok(()),
            _ => Err(invalid(format!("{path}/{operator} must have two operands"))),
        },
        other => Err(invalid(format!(
            "{path}: `{other}` is not available over evidence; use all, any, not, eq or ne"
        ))),
    }
}

/// Every `alias.field` operand in a `where` over evidence relations names a projected field of
/// a relation bound at that point.
pub(super) fn check_operands(
    predicate: Option<&Value>,
    relations: &[&FactQuery],
    path: &str,
) -> Result<()> {
    let Some(predicate) = predicate else {
        return Ok(());
    };
    let check_operand = |value: &Value| {
        let Value::String(text) = value else {
            return Ok(());
        };
        let Some((alias, field)) = text.split_once('.') else {
            return Ok(());
        };
        let Some(relation) = relations.iter().find(|relation| relation.alias == alias) else {
            return Ok(());
        };
        if relation
            .ast
            .project
            .iter()
            .any(|projection| projection.field == field)
        {
            Ok(())
        } else {
            Err(invalid(format!(
                "{path}: `{text}` names no projected field of evidence `{alias}`"
            )))
        }
    };
    match predicate {
        Value::Array(items) => {
            for item in items {
                check_operand(item)?;
            }
            Ok(())
        }
        Value::Object(predicate) => {
            if let Some(items) = predicate.get("all").or_else(|| predicate.get("any")) {
                for item in items.as_array().into_iter().flatten() {
                    check_operands(Some(item), relations, path)?;
                }
            } else if let Some(item) = predicate.get("not") {
                check_operands(Some(item), relations, path)?;
            } else if let Some(items) = predicate.get("eq").or_else(|| predicate.get("ne")) {
                for item in items.as_array().into_iter().flatten() {
                    check_operand(item)?;
                }
            }
            Ok(())
        }
        _ => check_operand(predicate),
    }
}

#[cfg(test)]
mod tests {
    use crate::code_rule_dsl::test_support::*;
    use serde_json::{Value, json};

    #[test]
    fn literal_comparison_operands_do_not_name_relation_fields() {
        let document = json!({
            "format": "rerail.policy/v1",
            "id": "literal-operands",
            "revision": 1,
            "mode": "steer",
            "clauses": [{
                "id": "relation",
                "violation": violation(),
                "require_each": {
                    "subject": { "evidence": {
                        "as": "row",
                        "query": { "ast": { "language": "typescript", "rule": { "kind": "identifier" } } },
                        "project": { "name": "file.name" }
                    } },
                    "evidence": { "evidence": {
                        "as": "other",
                        "query": { "ast": { "language": "typescript", "rule": { "kind": "identifier" } } },
                        "project": { "name": "file.name" },
                        "where": { "eq": [["row.missing"], ["row.missing"]] }
                    } }
                }
            }]
        });
        compile_document_with(
            document,
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
    }
}
