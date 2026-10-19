//! `evidence`: a cross-file relation projected from an AST query's matches.

use super::define::Defines;
use super::{compile_glob_set, compile_query, string_array};
use crate::code_rule_dsl::json::{invalid, non_empty_str, object, reject_unknown};
use crate::code_rule_dsl::model::{
    AstRelation, Capabilities, CodeRuleScope, FactQuery, NameClosure, Projection, ProjectionSource,
    Query,
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
        &[
            "as", "paths", "query", "project", "where", "closure", "each", "anchor",
        ],
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
            let (source, extract) = if source.is_object() {
                let spec = object(source, &at)?;
                reject_unknown(spec, &["source", "regex", "group"], &at)?;
                let pattern = non_empty_str(spec.get("regex"), &format!("{at}/regex"))?;
                if pattern.len() > capabilities.limits.max_regex_pattern_bytes {
                    return Err(invalid(format!("{at}/regex exceeds maxRegexPatternBytes")));
                }
                let regex = regex::Regex::new(pattern)
                    .map_err(|error| invalid(format!("{at}/regex: {error}")))?;
                let group = spec
                    .get("group")
                    .and_then(Value::as_u64)
                    .and_then(|group| usize::try_from(group).ok())
                    .filter(|group| *group < regex.captures_len())
                    .ok_or_else(|| {
                        invalid(format!(
                            "{at}/group must name an existing numeric regex group"
                        ))
                    })?;
                (
                    non_empty_str(spec.get("source"), &format!("{at}/source"))?,
                    Some((regex, group)),
                )
            } else {
                (non_empty_str(Some(source), &at)?, None)
            };
            let source = match source {
                "file.path" => ProjectionSource::Path,
                "file.directory" => ProjectionSource::Directory,
                "file.name" => ProjectionSource::Name,
                "file.stem" => ProjectionSource::Stem,
                "file.module" => ProjectionSource::Module,
                "file.package" => ProjectionSource::Package,
                other => match other.strip_prefix("capture.") {
                    Some(name) if !name.is_empty() => {
                        let (capture, suffix) = name.split_once('.').unwrap_or((name, ""));
                        if capture.is_empty() || name.ends_with('.') {
                            return Err(invalid(format!(
                                "{at}: invalid capture projection `{source}`"
                            )));
                        }
                        match suffix {
                            "" => ProjectionSource::Capture(capture.to_owned()),
                            "startByte" => ProjectionSource::CaptureStart(capture.to_owned()),
                            "startLine" => ProjectionSource::CaptureLine(capture.to_owned()),
                            "text" => ProjectionSource::CaptureText(capture.to_owned()),
                            _ => {
                                return Err(invalid(format!(
                                    "{at}: invalid capture projection `{source}`"
                                )));
                            }
                        }
                    }
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
                extract,
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
                placement: None,
            })
        })
        .transpose()?;
    let predicate = evidence.get("where").cloned();
    if let Some(predicate) = &predicate {
        validate_predicate(predicate, &format!("{path}/where"))?;
    }
    let each = evidence
        .get("each")
        .map(|value| non_empty_str(Some(value), &format!("{path}/each")).map(str::to_owned))
        .transpose()?;
    let anchor_text = evidence
        .get("anchor")
        .map(|value| non_empty_str(Some(value), &format!("{path}/anchor")))
        .transpose()?
        .unwrap_or("witness.span");
    let anchor =
        crate::code_rule_dsl::model::Anchor::parse(anchor_text, &format!("{path}/anchor"))?;
    let mut row_violation =
        serde_json::json!({"code":"row", "message":"row", "anchor":anchor_text});
    if let Some(each) = &each {
        row_violation["each"] = Value::String(each.clone());
    }
    let mut violation = super::parse_violation(&row_violation, &path)?;
    crate::code_rule_dsl::violations::bind(
        &mut violation,
        None,
        &crate::code_rule_dsl::model::ClauseKind::Forbid(query.clone()),
        &path,
    )?;
    let closure = compile_closure(evidence.get("closure"), &project, &path)?;
    Ok(FactQuery {
        alias,
        closure,
        each,
        anchor,
        predicate,
        ast: Arc::new(AstRelation {
            query,
            project,
            domain,
        }),
    })
}

/// Closure inputs name raw projected fields; stage dependencies refer only to earlier stages
/// with the same partition. Outputs cannot replace raw facts or another stage's outputs.
fn compile_closure(
    value: Option<&Value>,
    project: &[Projection],
    path: &str,
) -> Result<Vec<NameClosure>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let path = format!("{path}/closure");
    let stages = value
        .as_array()
        .ok_or_else(|| invalid(format!("{path} must be an array")))?;
    if stages.is_empty() || stages.len() > 32 {
        return Err(invalid(format!("{path} must contain 1 to 32 stages")));
    }
    let fields = project
        .iter()
        .map(|p| p.field.as_str())
        .collect::<BTreeSet<_>>();
    let mut outputs = BTreeSet::new();
    let mut compiled: Vec<NameClosure> = Vec::new();
    for (index, value) in stages.iter().enumerate() {
        let at = format!("{path}/{index}");
        let stage = object(value, &at)?;
        reject_unknown(
            stage,
            &[
                "name",
                "seeds",
                "seed_field",
                "edges",
                "extends",
                "partition_by",
                "contains",
            ],
            &at,
        )?;
        let name = non_empty_str(stage.get("name"), &format!("{at}/name"))?.to_owned();
        if compiled.iter().any(|s| s.name == name) {
            return Err(invalid(format!(
                "{at}/name: duplicate closure set `{name}`"
            )));
        }
        let list = |key: &str| -> Result<Vec<String>> {
            if stage.contains_key(key) {
                string_array(value, key, &at)
            } else {
                Ok(Vec::new())
            }
        };
        let field = |value: Option<&Value>, key: &str| -> Result<String> {
            let name = non_empty_str(value, &format!("{at}/{key}"))?;
            if !fields.contains(name) {
                return Err(invalid(format!(
                    "{at}/{key}: `{name}` names no projected field"
                )));
            }
            Ok(name.to_owned())
        };
        let seeds = list("seeds")?;
        let partition_by = list("partition_by")?;
        for key in &partition_by {
            field(Some(&Value::String(key.clone())), "partition_by")?;
        }
        let extends = list("extends")?;
        for dependency in &extends {
            if !compiled
                .iter()
                .any(|s| s.name == *dependency && s.partition_by == partition_by)
            {
                return Err(invalid(format!(
                    "{at}/extends: `{dependency}` must be an earlier set with the same partition_by"
                )));
            }
        }
        let seed_field = stage
            .get("seed_field")
            .map(|value| field(Some(value), "seed_field"))
            .transpose()?;
        let edges = stage
            .get("edges")
            .map(|value| -> Result<_> {
                let edges = object(value, &format!("{at}/edges"))?;
                reject_unknown(edges, &["from", "to"], &format!("{at}/edges"))?;
                Ok((
                    field(edges.get("from"), "edges/from")?,
                    field(edges.get("to"), "edges/to")?,
                ))
            })
            .transpose()?;
        let contains = object(
            stage
                .get("contains")
                .ok_or_else(|| invalid(format!("{at}/contains is required")))?,
            &format!("{at}/contains"),
        )?
        .iter()
        .map(|(output, input)| -> Result<_> {
            if output.is_empty()
                || fields.contains(output.as_str())
                || !outputs.insert(output.clone())
            {
                return Err(invalid(format!(
                    "{at}/contains: output `{output}` must be new and nonempty"
                )));
            }
            Ok((output.clone(), field(Some(input), "contains")?))
        })
        .collect::<Result<Vec<_>>>()?;
        compiled.push(NameClosure {
            name,
            seeds,
            seed_field,
            edges,
            extends,
            partition_by,
            contains,
        });
    }
    Ok(compiled)
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
        "eq" | "ne" | "lt" | "le" | "gt" | "ge" => match predicate[operator].as_array() {
            Some(operands) if operands.len() == 2 => Ok(()),
            _ => Err(invalid(format!("{path}/{operator} must have two operands"))),
        },
        other => Err(invalid(format!(
            "{path}: `{other}` is not available over evidence; use all, any, not, eq, ne, lt, le, gt or ge"
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
            || relation
                .closure
                .iter()
                .any(|stage| stage.contains.iter().any(|(output, _)| output == field))
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
            } else if let Some(items) = ["eq", "ne", "lt", "le", "gt", "ge"]
                .iter()
                .find_map(|key| predicate.get(*key))
            {
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
