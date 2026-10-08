mod arguments;
mod ast_rule;
#[cfg(test)]
mod binding_tests;
mod binds;
mod compare;
mod config;
mod define;
mod dialect;
mod evidence;
#[cfg(test)]
mod examples_tests;
mod format_args;
mod layout;
mod nth_child;
mod options;
mod order;
#[cfg(test)]
mod params_tests;
mod parse;
mod resolve;
mod transform;
mod utils;
mod value;

use super::json::{
    bounded_text, invalid, json_error, non_empty_str, object, reject_unknown, required_str,
};
use super::language::{CodeRuleLanguage, Semantic};
use super::matcher::{
    AnchorPlan, AstMatcher, Capture, Constrained, GrammarMatcher, Reading, RequiredText,
    UtilOrigin, UtilTable, collect_variables, reads_values,
};
use super::model::{
    Anchor, Capabilities, ClauseKind, CodeRuleScope, CompiledClause, Limits, Query, RegexSource,
    RuleMode, ViolationSpec,
};
use super::soundness::sound_under_errors;
use super::violations;
use crate::code_rule_dsl::language::{GrammarDialect, PatternSource, rule_pattern};
pub(super) use ast_rule::compile_ast_rule;
use ast_rule::validate_ast_rule;
use compare::{binding_site_index, bound_elsewhere, compare_sites, frame_scope};
use define::{CLAUSE_KINDS, Defines, collect_defines, is_define};
use dialect::language_capability;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use napi::Result;
use regex::Regex;
use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
pub(super) use utils::UtilScope;
use utils::{UtilDef, UtilIds, reachable, reject_util_cycles, rule_map, util_map};

/// Parses a code-rule document. An object with two equal keys is an error: a plain `Value` keeps the
/// last one, so the first term would vanish silently (ast-grep 0.45.3 rejects such a rule).
pub(super) fn parse_document(source: &str, format: &str) -> Result<Value> {
    match format {
        "json" => {
            let mut deserializer = serde_json::Deserializer::from_str(source);
            let value = UniqueKeys {
                path: String::new(),
            }
            .deserialize(&mut deserializer)
            .map_err(json_error)?;
            deserializer.end().map_err(json_error)?;
            Ok(value)
        }
        "yaml" => UniqueKeys {
            path: String::new(),
        }
        .deserialize(serde_yaml_ng::Deserializer::from_str(source))
        .map_err(|error| invalid(format!("invalid YAML: {error}"))),
        _ => Err(invalid(format!(
            "unsupported policy source format `{format}`"
        ))),
    }
}

/// Deserializes any document into a `Value`, rejecting an object that repeats a key. `path` is the
/// JSON pointer of the value being read, for the error message.
struct UniqueKeys {
    path: String,
}

impl<'de> DeserializeSeed<'de> for UniqueKeys {
    type Value = Value;

    fn deserialize<D: Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for UniqueKeys {
    type Value = Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> std::result::Result<Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> std::result::Result<Value, E> {
        Ok(Value::from(value))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> std::result::Result<Value, E> {
        Ok(Value::from(value))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> std::result::Result<Value, E> {
        Ok(serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E: de::Error>(self, value: String) -> std::result::Result<Value, E> {
        Ok(Value::String(value))
    }

    fn visit_none<E: de::Error>(self) -> std::result::Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E: de::Error>(self) -> std::result::Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D: Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<Value, D::Error> {
        self.deserialize(deserializer)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut items: A) -> std::result::Result<Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = items.next_element_seed(UniqueKeys {
            path: format!("{}/{}", self.path, values.len()),
        })? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> std::result::Result<Value, A::Error> {
        let mut values = Map::new();
        while let Some(key) = entries.next_key::<String>()? {
            if values.contains_key(&key) {
                let at = if self.path.is_empty() {
                    "/"
                } else {
                    self.path.as_str()
                };
                return Err(de::Error::custom(format!("duplicate key `{key}` at {at}")));
            }
            let value = entries.next_value_seed(UniqueKeys {
                path: format!("{}/{key}", self.path),
            })?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}

pub(super) fn parse_capabilities(source: &str) -> Result<Capabilities> {
    let raw: Value = serde_json::from_str(source).map_err(json_error)?;
    let limits = raw
        .get("limits")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("capabilities.limits must be an object"))?;
    let limits = Limits {
        max_ast_pattern_bytes: limit(limits, "maxAstPatternBytes")?,
        max_files: limit(limits, "maxFiles")?,
        max_regex_pattern_bytes: limit(limits, "maxRegexPatternBytes")?,
        max_source_bytes: limit(limits, "maxSourceBytes")?,
        max_visited_nodes: limit(limits, "maxVisitedNodes")?,
        max_witnesses: limit(limits, "maxWitnesses")?,
    };
    Ok(Capabilities { raw, limits })
}

fn limit(limits: &Map<String, Value>, name: &str) -> Result<usize> {
    limits
        .get(name)
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .ok_or_else(|| {
            invalid(format!(
                "capabilities.limits.{name} must be a positive integer"
            ))
        })
}

pub(super) fn compile_scope(value: Option<&Value>) -> Result<CodeRuleScope> {
    let (include, exclude) = if let Some(value) = value {
        let scope = object(value, "/scope")?;
        reject_unknown(scope, &["include", "exclude"], "/scope")?;
        (
            string_array(value, "include", "/scope")?,
            string_array(value, "exclude", "/scope")?,
        )
    } else {
        (vec!["**".to_owned()], Vec::new())
    };
    if include.is_empty() {
        return Err(invalid("/scope/include must not be empty"));
    }
    Ok(CodeRuleScope {
        include: compile_glob_set(&include, "/scope/include")?,
        exclude: compile_glob_set(&exclude, "/scope/exclude")?,
    })
}

fn string_array(value: &Value, key: &str, path: &str) -> Result<Vec<String>> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| invalid(format!("{path}/{key} must be an array")))?
        .iter()
        .enumerate()
        .map(|(index, item)| {
            item.as_str()
                .map(str::to_owned)
                .ok_or_else(|| invalid(format!("{path}/{key}/{index} must be a string")))
        })
        .collect()
}

fn compile_glob_set(patterns: &[String], path: &str) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for (index, pattern) in patterns.iter().enumerate() {
        let glob = GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .map_err(|error| invalid(format!("{path}/{index}: {error}")))?;
        builder.add(glob);
    }
    builder
        .build()
        .map_err(|error| invalid(format!("{path}: {error}")))
}

/// The code rule's id, scope, clauses and required capabilities.
pub(super) fn compile_document(
    document: &Value,
    capabilities: &Capabilities,
) -> Result<(String, RuleMode, CodeRuleScope, Vec<CompiledClause>, Value)> {
    let (resolved, options) = config::resolve(document)?;
    let document = &resolved;
    let root = object(document, "/")?;
    reject_unknown(
        root,
        &[
            "format",
            "id",
            "revision",
            "mode",
            "description",
            "scope",
            "clauses",
        ],
        "/",
    )?;
    if required_str(document, "format", "/")? != "rerail.policy/v1" {
        return Err(invalid("/format must equal rerail.policy/v1"));
    }
    let id = required_str(document, "id", "/")?.to_owned();
    let rule_mode = match document.get("mode").and_then(Value::as_str) {
        Some("log") => RuleMode::Log,
        Some("steer") => RuleMode::Steer,
        Some("block") => RuleMode::Block,
        _ => return Err(invalid("/mode must equal log, steer, or block")),
    };
    let scope = compile_scope(document.get("scope"))?;
    let clauses = document
        .get("clauses")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("/clauses must be an array"))?;
    if clauses.is_empty() {
        return Err(invalid("/clauses must not be empty"));
    }
    let defines = Defines {
        utils: collect_defines(clauses, capabilities)?,
        options,
    };
    let mut compiled = Vec::with_capacity(clauses.len());
    let mut required_ast = BTreeSet::new();
    let mut requires_llm = false;
    let mut ids = BTreeSet::new();
    for (index, clause) in clauses.iter().enumerate() {
        let path = format!("/clauses/{index}");
        let id = if is_define(clause) {
            required_str(clause, "id", &path)?.to_owned()
        } else {
            let value = compile_clause(
                clause,
                &path,
                capabilities,
                &defines,
                &mut required_ast,
                &mut requires_llm,
            )?;
            let id = value.id.clone();
            compiled.push(value);
            id
        };
        if !ids.insert(id.clone()) {
            return Err(invalid(format!("duplicate clause id `{id}` at {path}")));
        }
    }

    if compiled.is_empty() {
        return Err(invalid(
            "/clauses must hold a clause besides define: a define reports nothing",
        ));
    }
    let ast = required_ast
        .into_iter()
        .map(|(language, grammar)| json!({ "engine": "ast-grep-core", "grammar": grammar, "language": language }))
        .collect::<Vec<_>>();
    Ok((
        id,
        rule_mode,
        scope,
        compiled,
        json!({ "ast": ast, "llm": requires_llm }),
    ))
}

fn compile_clause(
    value: &Value,
    path: &str,
    capabilities: &Capabilities,
    defines: &Defines,
    required_ast: &mut BTreeSet<(String, String)>,
    requires_llm: &mut bool,
) -> Result<CompiledClause> {
    let clause = object(value, path)?;
    reject_unknown(
        clause,
        &["id", "forbid", "require_each", "budget", "llm", "violation"],
        path,
    )?;
    let id = required_str(value, "id", path)?.to_owned();
    let violation_value = value
        .get("violation")
        .ok_or_else(|| invalid(format!("{path}/violation is required")))?;
    let forms = CLAUSE_KINDS
        .into_iter()
        .filter(|key| clause.contains_key(*key))
        .collect::<Vec<_>>();
    if forms.len() != 1 {
        return Err(invalid(format!(
            "{path} must contain exactly one constraint form"
        )));
    }
    let kind = match forms[0] {
        "forbid" => ClauseKind::Forbid(compile_query(
            clause.get("forbid").expect("form exists"),
            &format!("{path}/forbid"),
            capabilities,
            defines,
            required_ast,
        )?),
        "require_each" => {
            let requirement = object(
                clause.get("require_each").expect("form exists"),
                &format!("{path}/require_each"),
            )?;
            reject_unknown(
                requirement,
                &["subject", "evidence"],
                &format!("{path}/require_each"),
            )?;
            let subject = evidence::compile_relation(
                requirement
                    .get("subject")
                    .ok_or_else(|| invalid(format!("{path}/require_each/subject is required")))?,
                &format!("{path}/require_each/subject"),
                capabilities,
                defines,
                required_ast,
            )?;
            let evidence = evidence::compile_relation(
                requirement
                    .get("evidence")
                    .ok_or_else(|| invalid(format!("{path}/require_each/evidence is required")))?,
                &format!("{path}/require_each/evidence"),
                capabilities,
                defines,
                required_ast,
            )?;
            evidence::check_operands(
                subject.predicate.as_ref(),
                &[&subject],
                &format!("{path}/require_each/subject"),
            )?;
            evidence::check_operands(
                evidence.predicate.as_ref(),
                &[&subject, &evidence],
                &format!("{path}/require_each/evidence"),
            )?;
            ClauseKind::RequireEach { subject, evidence }
        }
        "budget" => {
            let budget = object(
                clause.get("budget").expect("form exists"),
                &format!("{path}/budget"),
            )?;
            reject_unknown(
                budget,
                &["query", "group_by", "count"],
                &format!("{path}/budget"),
            )?;
            let query = evidence::compile_relation(
                budget
                    .get("query")
                    .ok_or_else(|| invalid(format!("{path}/budget/query is required")))?,
                &format!("{path}/budget/query"),
                capabilities,
                defines,
                required_ast,
            )?;
            let group_by = budget
                .get("group_by")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .map(|value| {
                            value
                                .as_str()
                                .map(str::to_owned)
                                .ok_or_else(|| invalid("group_by fields must be strings"))
                        })
                        .collect::<Result<Vec<_>>>()
                })
                .transpose()?
                .unwrap_or_default();
            let count = object(
                budget
                    .get("count")
                    .ok_or_else(|| invalid(format!("{path}/budget/count is required")))?,
                "count",
            )?;
            reject_unknown(count, &["lte", "distinct"], &format!("{path}/budget/count"))?;
            let maximum = count
                .get("lte")
                .and_then(Value::as_u64)
                .ok_or_else(|| invalid("budget count.lte is required"))?
                as usize;
            let distinct = match count.get("distinct") {
                None => Vec::new(),
                Some(_) => string_array(
                    &Value::Object(count.clone()),
                    "distinct",
                    &format!("{path}/budget/count"),
                )?,
            };
            for (key, fields) in [("group_by", &group_by), ("count/distinct", &distinct)] {
                evidence::check_operands(
                    Some(&Value::Array(
                        fields.iter().cloned().map(Value::String).collect(),
                    )),
                    &[&query],
                    &format!("{path}/budget/{key}"),
                )?;
            }
            evidence::check_operands(
                query.predicate.as_ref(),
                &[&query],
                &format!("{path}/budget/query"),
            )?;
            ClauseKind::Budget {
                query,
                group_by,
                maximum,
                distinct,
            }
        }
        "llm" => {
            *requires_llm = true;
            ClauseKind::Llm
        }
        _ => unreachable!(),
    };
    let mut violation = parse_violation(violation_value, &format!("{path}/violation"))?;
    violations::bind(
        &mut violation,
        violation_value.get("fix").and_then(|fix| match fix {
            Value::Object(fix) => fix.get("template").and_then(Value::as_str),
            other => other.as_str(),
        }),
        &kind,
        &format!("{path}/violation"),
    )?;
    Ok(CompiledClause {
        id,
        violation,
        kind,
        synthetic: false,
        decision: false,
    })
}

fn compile_query(
    value: &Value,
    path: &str,
    capabilities: &Capabilities,
    defines: &Defines,
    required_ast: &mut BTreeSet<(String, String)>,
) -> Result<Query> {
    compile_query_as(value, path, capabilities, defines, required_ast, false)
}

/// `compile_query`, for a `calls` term's seed query where `seed`: its rules cannot hold `calls`.
fn compile_query_as(
    value: &Value,
    path: &str,
    capabilities: &Capabilities,
    defines: &Defines,
    required_ast: &mut BTreeSet<(String, String)>,
    seed: bool,
) -> Result<Query> {
    let query = object(value, path)?;
    reject_unknown(query, &["regex", "ast", "select_when"], path)?;
    let forms = ["regex", "ast", "select_when"]
        .into_iter()
        .filter(|key| query.contains_key(*key))
        .collect::<Vec<_>>();
    if forms.len() != 1 {
        return Err(invalid(format!(
            "{path} must contain exactly one query source"
        )));
    }
    match forms[0] {
        "regex" => {
            let regex = object(
                query.get("regex").expect("form exists"),
                &format!("{path}/regex"),
            )?;
            reject_unknown(regex, &["source", "pattern"], &format!("{path}/regex"))?;
            let pattern = bounded_text(
                regex.get("pattern"),
                &format!("{path}/regex/pattern"),
                "maxRegexPatternBytes",
                capabilities.limits.max_regex_pattern_bytes,
            )?;
            let source = match regex
                .get("source")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid(format!("{path}/regex/source is required")))?
            {
                "file.content" => RegexSource::Content,
                "file.path" => RegexSource::Path,
                value => {
                    return Err(invalid(format!(
                        "{path}/regex/source: unsupported source `{value}`"
                    )));
                }
            };
            Ok(Query::Regex {
                regex: Regex::new(pattern)
                    .map_err(|error| invalid(format!("{path}/regex/pattern: {error}")))?,
                source,
            })
        }
        "ast" => {
            let ast = object(
                query.get("ast").expect("form exists"),
                &format!("{path}/ast"),
            )?;
            reject_unknown(
                ast,
                &[
                    "language",
                    "rule",
                    "utils",
                    "constraints",
                    "languageOptions",
                    "when",
                    "file",
                    "transform",
                ],
                &format!("{path}/ast"),
            )?;
            let language_name = ast
                .get("language")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid(format!("{path}/ast/language is required")))?;
            let code_rule_language = CodeRuleLanguage::parse(language_name).ok_or_else(|| {
                invalid(format!(
                    "{path}/ast/language must equal python, typescript, javascript or php"
                ))
            })?;
            let rule = ast
                .get("rule")
                .ok_or_else(|| invalid(format!("{path}/ast/rule is required")))?;
            let (grammars, grammar) = language_capability(language_name, capabilities)?;
            let (options, python) = match ast.get("languageOptions") {
                None => (None, None),
                Some(value) => match code_rule_language.family() {
                    super::language::Family::Python => (
                        None,
                        Some(Arc::new(options::compile_python_options(
                            value,
                            &format!("{path}/ast"),
                        )?)),
                    ),
                    super::language::Family::Ecma => (
                        Some(Arc::new(options::compile_language_options(
                            value,
                            &format!("{path}/ast"),
                        )?)),
                        None,
                    ),
                    super::language::Family::Php => {
                        return Err(invalid(format!(
                            "{path}/ast/languageOptions: `languageOptions` is not available for php policies"
                        )));
                    }
                    super::language::Family::Blade => {
                        return Err(invalid(format!(
                            "{path}/ast/languageOptions: `languageOptions` is not available for blade policies"
                        )));
                    }
                },
            };
            let applies = ast
                .get("when")
                .map(|when| {
                    options::compile_when(
                        when,
                        &format!("{path}/ast"),
                        matches!(code_rule_language.family(), super::language::Family::Python),
                        options.as_deref(),
                        python.as_deref(),
                        &defines.options,
                    )
                })
                .transpose()?
                .unwrap_or(true);
            let file = ast
                .get("file")
                .map(|file| {
                    options::compile_file_condition(
                        file,
                        &format!("{path}/ast"),
                        language_name,
                        capabilities.limits.max_regex_pattern_bytes,
                    )
                    .map(Arc::new)
                })
                .transpose()?;
            let rule_path = format!("{path}/ast/rule");
            let utils_path = format!("{path}/ast/utils");
            let constraints_path = format!("{path}/ast/constraints");
            let local = util_map(ast.get("utils"), &utils_path)?;
            let constraints = rule_map(ast.get("constraints"), &constraints_path)?;
            // The document's shared utilities of this language that the query reaches join its
            // own; the others are not compiled here (their define checked them).
            let empty = BTreeMap::new();
            let shared = defines.utils.get(language_name).unwrap_or(&empty);
            let mut utils = local.clone();
            let roots = std::iter::once(rule)
                .chain(local.values().map(|util| util.rule))
                .chain(constraints.values().copied());
            for id in reachable(roots, shared) {
                if let Some(util) = local.get(&id) {
                    return Err(invalid(format!(
                        "{}: utility rule `{id}` is also defined by a define of {language_name}; \
                         rename one",
                        util.path
                    )));
                }
                let util: &UtilDef = &shared[&id];
                utils.insert(id, util.clone());
            }
            // Shape errors do not depend on a grammar: report them once, without a dialect.
            let util_ids = utils
                .iter()
                .map(|(id, util)| (id.as_str(), util.params.as_slice()))
                .collect::<UtilIds>();
            let limits = &capabilities.limits;
            for util in local.values() {
                refuse_unavailable_semantics(util.rule, &util.rule_path, code_rule_language)?;
                validate_ast_rule(util.rule, &util.rule_path, limits, &util_ids)?;
            }
            refuse_unavailable_semantics(rule, &rule_path, code_rule_language)?;
            validate_ast_rule(rule, &rule_path, limits, &util_ids)?;
            for (name, constraint) in &constraints {
                let constraint_path = format!("{constraints_path}/{name}");
                refuse_unavailable_semantics(constraint, &constraint_path, code_rule_language)?;
                validate_ast_rule(constraint, &constraint_path, limits, &util_ids)?;
            }
            reject_util_cycles(&utils)?;
            let transform_path = format!("{path}/ast/transform");
            let transform = ast
                .get("transform")
                .map(|value| {
                    let reading = Reading::new(
                        code_rule_language.primary(),
                        options.clone(),
                        python.clone(),
                    );
                    transform::compile_transform(value, &transform_path, limits, &reading)
                        .map(Arc::new)
                })
                .transpose()?;
            // A transform variable needs a name no capture of the query uses; said before any
            // use of the name is checked as a transform variable's.
            if let Some(transform) = &transform {
                let languages = grammars
                    .iter()
                    .map(|(_, language)| *language)
                    .collect::<Vec<_>>();
                let mut declared = Vec::new();
                declared_captures(rule, &rule_path, &languages, &mut declared);
                for util in utils.values().filter(|util| util.params.is_empty()) {
                    declared_captures(util.rule, &util.rule_path, &languages, &mut declared);
                }
                for (name, constraint) in &constraints {
                    let constraint_path = format!("{constraints_path}/{name}");
                    declared_captures(constraint, &constraint_path, &languages, &mut declared);
                }
                for name in transform.names() {
                    if let Some((_, at)) = declared.iter().find(|(known, _)| known == name) {
                        return Err(invalid(format!(
                            "{transform_path}/{name}: `{name}` names a capture of the query (`${name}` \
                             at {at}) and a transform variable; rename one"
                        )));
                    }
                }
            }
            // A rule may use syntax only one dialect parses (JSX, `<T>x` assertions). A sub-rule
            // a dialect cannot express never matches there; the rule must be whole in at least one.
            // Utilities and constraints compile per dialect too, and their gaps count.
            let mut matchers = Vec::new();
            let mut rejections = Vec::new();
            let mut whole = false;
            let mut variables = BTreeMap::new();
            let mut framed_variables = BTreeMap::new();
            let conflicts = std::cell::RefCell::new(BTreeSet::new());
            // `calls` terms of the rule and its utilities; a term's seed depends on the utilities
            // and language options its inner rule reads, so they salt its key.
            let salt = serde_json::to_string(&(
                language_name,
                utils
                    .iter()
                    .map(|(id, util)| (id, &util.params, util.rule))
                    .collect::<Vec<_>>(),
                ast.get("languageOptions"),
            ))
            .map_err(|error| invalid(error.to_string()))?;
            let calls = crate::code_rule_dsl::calls::CallsCollector::new(salt, seed);
            let mut bindable = BTreeSet::new();
            let origins = util_origins(&utils, language_name)?;
            for (dialect, language) in &grammars {
                let mut gaps = Vec::new();
                let table = Arc::new(UtilTable::new(
                    utils.keys().cloned().collect(),
                    utils.values().map(|util| !util.params.is_empty()).collect(),
                    origins.iter().cloned().map(Some).collect(),
                ));
                let scope = UtilScope {
                    table: Arc::downgrade(&table),
                    ids: &util_ids,
                    options: options.clone(),
                    python: python.clone(),
                    transform: transform.clone(),
                    conflicts: Some(&conflicts),
                    captures: Default::default(),
                    calls: Some(&calls),
                };
                let compile_utils = |gaps: &mut Vec<String>| {
                    utils
                        .values()
                        .map(|util| {
                            compile_ast_rule(util.rule, *language, &util.rule_path, gaps, &scope)
                        })
                        .collect::<Result<Vec<_>>>()
                };
                // A utility's terms are ordered by what the utilities they call bind, and its
                // `parse` exports what they export, so utilities that met one not compiled yet are
                // compiled again against this first compilation.
                let mut first_gaps = Vec::new();
                let mut compiled_utils = compile_utils(&mut first_gaps)?;
                if table.was_unfilled() {
                    if table.draft.set(compiled_utils).is_err() {
                        unreachable!("a fresh table is drafted once");
                    }
                    compiled_utils = compile_utils(&mut gaps)?;
                } else {
                    gaps.extend(first_gaps);
                }
                // A parameterized utility's captures are its frame's; its calls report what returns.
                for (util, def) in compiled_utils.iter().zip(utils.values()) {
                    if def.params.is_empty() {
                        collect_variables(util, &mut variables);
                    } else {
                        collect_variables(util, &mut framed_variables);
                    }
                }
                if table.rules.set(compiled_utils).is_err() {
                    unreachable!("a fresh table is filled once");
                }
                let mut matcher = compile_ast_rule(rule, *language, &rule_path, &mut gaps, &scope)?;
                collect_variables(&matcher, &mut variables);
                let mut compiled_constraints = Vec::with_capacity(constraints.len());
                for (name, constraint) in &constraints {
                    let constraint_path = format!("{constraints_path}/{name}");
                    let constraint = compile_ast_rule(
                        constraint,
                        *language,
                        &constraint_path,
                        &mut gaps,
                        &scope,
                    )?;
                    collect_variables(&constraint, &mut variables);
                    compiled_constraints.push((name.clone(), constraint));
                }
                match gaps.into_iter().next() {
                    Some(reason) => rejections.push((reason, *dialect)),
                    None => whole = true,
                }
                if !compiled_constraints.is_empty() && !matches!(matcher, AstMatcher::Never) {
                    matcher = AstMatcher::Constrained(Box::new(Constrained {
                        rule: matcher,
                        constraints: compiled_constraints,
                    }));
                }
                // Every utility is compiled, used or not, and binds its own names when it runs.
                let mut names = BTreeSet::new();
                let mut summaries = violations::CaptureSummaries::new();
                violations::ast_captures(&matcher, &table, &mut summaries, &mut names);
                for util in table.shared_rules() {
                    violations::ast_captures(util, &table, &mut summaries, &mut names);
                }
                bindable.extend(names.into_iter().map(str::to_owned));
                if !matches!(matcher, AstMatcher::Never) {
                    let anchors = Arc::new(AnchorPlan::compile(&matcher));
                    let three_valued = reads_values(&matcher, &mut BTreeSet::new());
                    let text = (!three_valued)
                        .then(|| RequiredText::of(&matcher).map(Arc::new))
                        .flatten();
                    matchers.push(GrammarMatcher {
                        language: *language,
                        sound_under_errors: sound_under_errors(&matcher),
                        three_valued,
                        matcher,
                        anchors,
                        text,
                        utils: table,
                        file: file.clone(),
                        applies,
                        transform: transform.clone(),
                    });
                }
            }
            if !whole {
                let (first, _) = &rejections[0];
                let message = if rejections.iter().all(|(reason, _)| reason == first) {
                    let dialects = rejections.iter().map(|(_, dialect)| *dialect);
                    let dialects = dialects.collect::<Vec<_>>().join(", ");
                    let plural = if rejections.len() > 1 { "s" } else { "" };
                    format!("{first} ({dialects} grammar{plural})")
                } else {
                    let each = rejections
                        .iter()
                        .map(|(reason, dialect)| format!("{reason} ({dialect} grammar)"));
                    each.collect::<Vec<_>>().join("; ")
                };
                return Err(invalid(message));
            }
            // ast-grep-config's check: a constraint must name a metavariable the rule, a utility,
            // or a constraint defines. Constraints only ever read single-node captures, so one on
            // a name that is only ever a `$$$` capture would silently never apply.
            for name in constraints.keys() {
                match variables.get(name.as_str()) {
                    None => {
                        return Err(invalid(format!(
                            "{constraints_path}/{name}: metavariable `${name}` is not defined by the rule, its utils, or its constraints"
                        )));
                    }
                    Some(Capture::Multiple) => {
                        return Err(invalid(format!(
                            "{constraints_path}/{name}: `$$${name}` captures a node list; constraints apply to single-node captures only"
                        )));
                    }
                    Some(Capture::Single) => {}
                }
            }
            // Format arguments read a string and arguments the query binds.
            let mut format_sites = Vec::new();
            format_args::format_args_sites(rule, &rule_path, &mut format_sites);
            for util in utils.values() {
                format_args::format_args_sites(util.rule, &util.rule_path, &mut format_sites);
            }
            let mut format_variables = framed_variables;
            format_variables.extend(
                variables
                    .iter()
                    .map(|(name, capture)| (name.clone(), *capture)),
            );
            for (site_path, test) in format_sites {
                format_args::check_captures(test, &site_path, &format_variables)?;
            }
            // ast-grep-config's `check_var_in_transform`: a transform variable is no capture, and its
            // source is a capture the query can bind or another transform variable.
            // A named group of a regex that tests the witness itself is a source too (Rerail's):
            // text, read once the rule matched.
            if let Some(transform) = &transform {
                let groups = matchers
                    .iter()
                    .flat_map(|grammar| {
                        violations::witness_groups(&grammar.matcher, &grammar.utils)
                    })
                    .filter(|group| !bindable.contains(group) && !variables.contains_key(group))
                    .collect::<BTreeSet<_>>();
                for (name, trans) in &transform.steps {
                    if bindable.contains(name) || variables.contains_key(name) {
                        return Err(invalid(format!(
                            "{transform_path}/{name}: `${name}` is already a capture of the query"
                        )));
                    }
                    let source = trans.source_name();
                    let group = groups.contains(source);
                    if !transform.contains(source) && !bindable.contains(source) && !group {
                        return Err(invalid(format!(
                            "{transform_path}/{name}/source: `${source}` names no capture the query can bind"
                        )));
                    }
                    if group && trans.reads_value() {
                        let kind = ast["transform"][name.as_str()]
                            .as_object()
                            .and_then(|entry| entry.keys().next())
                            .map_or("", String::as_str);
                        return Err(invalid(format!(
                            "{transform_path}/{name}/{kind}/value: `${source}` is a regex group, text; read the value of a capture"
                        )));
                    }
                }
            }
            // A comparison reads two single-node captures that can be bound where it runs.
            let pattern_names = |pattern: &Value| -> BTreeSet<String> {
                grammars
                    .iter()
                    .find_map(|(_, language)| {
                        let source = match pattern {
                            Value::String(source) => PatternSource::Plain(source),
                            other => PatternSource::Contextual {
                                context: other
                                    .get("context")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default(),
                                selector: other
                                    .get("selector")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default(),
                            },
                        };
                        rule_pattern(source, *language).ok().map(|(compiled, _)| {
                            compiled
                                .defined_vars()
                                .into_iter()
                                .map(str::to_owned)
                                .collect()
                        })
                    })
                    .unwrap_or_default()
            };
            let mut sites = Vec::new();
            compare_sites(
                rule,
                &rule_path,
                &utils,
                &pattern_names,
                Vec::new(),
                &mut sites,
            );
            // A parameterized utility's comparisons read its frame only: its parameters and its own.
            for util in utils.values() {
                let scopes = if util.params.is_empty() {
                    Vec::new()
                } else {
                    frame_scope(util, &utils, &pattern_names)
                };
                compare_sites(
                    util.rule,
                    &util.rule_path,
                    &utils,
                    &pattern_names,
                    scopes,
                    &mut sites,
                );
            }
            for (name, constraint) in &constraints {
                let constraint_path = format!("{constraints_path}/{name}");
                compare_sites(
                    constraint,
                    &constraint_path,
                    &utils,
                    &pattern_names,
                    Vec::new(),
                    &mut sites,
                );
            }
            // A utility's comparison has a site per scope it runs in; a name is never bound
            // there only when no call (nor the utility alone) can bind it.
            let seen = sites
                .iter()
                .filter(|site| site.visible(&bindable))
                .map(|site| (site.path.clone(), site.side))
                .collect::<BTreeSet<_>>();
            let transform_seen = sites
                .iter()
                .filter(|site| {
                    transform.as_ref().is_some_and(|transform| {
                        transform.contains(site.name)
                            && transform
                                .roots(site.name)
                                .into_iter()
                                .all(|root| site.visible_as(root, &bindable))
                    })
                })
                .map(|site| (site.path.clone(), site.side))
                .collect::<BTreeSet<_>>();
            let mut reported = BTreeSet::new();
            for site in sites {
                let (at, name) = (format!("{}/{}", site.path, site.side), site.name);
                if !reported.insert(at.clone()) {
                    continue;
                }
                // A transform variable is text computed from captures, which must be visible where
                // the comparison runs (at one of its sites: a utility's has one per call).
                if let Some(transform) = transform.as_ref().filter(|t| t.contains(name)) {
                    if site.side == "within" || site.side == "to" {
                        return Err(invalid(format!(
                            "{at}: `${name}` is a transform variable, text; {} reads a node",
                            site.side
                        )));
                    }
                    if !transform_seen.contains(&(site.path.clone(), site.side))
                        && let Some(root) = transform
                            .roots(name)
                            .into_iter()
                            .find(|root| !site.visible_as(root, &bindable))
                    {
                        return Err(invalid(format!(
                            "{at}: `${name}` transforms `${root}`, which is never bound where this \
                             comparison runs"
                        )));
                    }
                    continue;
                }
                let visible = seen.contains(&(site.path.clone(), site.side));
                if site.side == "to" {
                    if !visible {
                        return Err(invalid(format!(
                            "{at}: `${name}` is never bound where this layout runs (names bound \
                             under `not` or `count` are visible only inside it; a stopBy rule sees \
                             only the names it binds itself)"
                        )));
                    }
                    if variables.get(name) == Some(&Capture::Multiple) {
                        return Err(invalid(format!(
                            "{at}: `$$${name}` captures a node list; a gap runs to a single node"
                        )));
                    }
                    continue;
                }
                if !visible && site.side == "within" {
                    return Err(invalid(format!(
                        "{at}: `${name}` is never bound where this binding filter runs (names bound \
                         under `not` or `count` are visible only inside it; a stopBy rule sees only \
                         the names it binds itself)"
                    )));
                }
                if variables.get(name) == Some(&Capture::Multiple) && site.side == "within" {
                    return Err(invalid(format!(
                        "{at}: `$$${name}` captures a node list; within reads single-node captures only"
                    )));
                }
                if !visible {
                    return Err(invalid(format!(
                        "{at}: `${name}` is never bound where this comparison runs (names bound under \
                         `not` or `count` are visible only inside it; a stopBy rule sees only the \
                         names it binds itself)"
                    )));
                }
                if variables.get(name) == Some(&Capture::Multiple) {
                    return Err(invalid(format!(
                        "{at}: `$$${name}` captures a node list; compare applies to single-node captures only"
                    )));
                }
            }
            // An `all` item reading what only a later item binds is refused, unless something outside
            // those items binds the name too (an enclosing rule, a caller), which may run first.
            // The query's binding sites are indexed once, and only for a query with a conflict.
            let conflicts = conflicts.into_inner();
            if !conflicts.is_empty() {
                let binding_sites = binding_site_index(
                    rule,
                    &rule_path,
                    &utils,
                    &constraints,
                    &constraints_path,
                    &pattern_names,
                );
                for conflict in conflicts {
                    if !bound_elsewhere(&conflict, &utils, &binding_sites) {
                        return Err(conflict.error());
                    }
                }
            }
            // Each term's inner rule, over definitions, with this query's utilities: its seed.
            let mut terms = Vec::new();
            for found in calls.take() {
                let (seed, seed_scope) = crate::code_rule_dsl::calls::seed_query(
                    &found.inner,
                    ast,
                    &found.path,
                    language_name,
                    code_rule_language.family(),
                )?;
                let seed = compile_query_as(
                    &seed,
                    &found.path,
                    capabilities,
                    defines,
                    required_ast,
                    true,
                )?;
                terms.push(Arc::new(super::model::CallsTerm {
                    key: found.key,
                    seed_key: found.seed_key,
                    seed_scope,
                    through: found.through,
                    seed,
                }));
            }
            required_ast.insert((language_name.to_owned(), grammar));
            Ok(Query::Ast {
                matchers,
                calls: terms,
            })
        }
        "select_when" => {
            let select_when = object(
                query.get("select_when").expect("form exists"),
                &format!("{path}/select_when"),
            )?;
            reject_unknown(
                select_when,
                &["select", "exists"],
                &format!("{path}/select_when"),
            )?;
            let select = compile_query(
                select_when
                    .get("select")
                    .ok_or_else(|| invalid(format!("{path}/select_when/select is required")))?,
                &format!("{path}/select_when/select"),
                capabilities,
                defines,
                required_ast,
            )?;
            let exists = compile_query(
                select_when
                    .get("exists")
                    .ok_or_else(|| invalid(format!("{path}/select_when/exists is required")))?,
                &format!("{path}/select_when/exists"),
                capabilities,
                defines,
                required_ast,
            )?;
            if !select.calls().is_empty() || !exists.calls().is_empty() {
                return Err(invalid(format!(
                    "{path}/select_when: calls is not supported in select_when"
                )));
            }
            Ok(Query::SelectWhen {
                select: Box::new(select),
                exists: Box::new(exists),
            })
        }
        _ => unreachable!(),
    }
}

fn parse_violation(value: &Value, path: &str) -> Result<ViolationSpec> {
    let violation = object(value, path)?;
    reject_unknown(
        violation,
        &[
            "code",
            "anchor",
            "message",
            "variables",
            "fix",
            "each",
            "suppress",
        ],
        path,
    )?;
    let anchor = Anchor::parse(
        required_str(value, "anchor", path)?,
        &format!("{path}/anchor"),
    )?;
    // A fix replaces the witness, or with `range` a capture's range inside it.
    let fix_range = match violation.get("fix") {
        None | Some(Value::String(_)) => None,
        Some(Value::Object(fix)) => {
            reject_unknown(fix, &["template", "range"], &format!("{path}/fix"))?;
            if !fix.get("template").is_some_and(Value::is_string) {
                return Err(invalid(format!(
                    "{path}/fix/template must be an ast-grep fix template string"
                )));
            }
            match fix.get("range").and_then(Value::as_str) {
                Some("witness.span") => None,
                Some(range) => match range.strip_prefix("capture.") {
                    Some(name)
                        if !name.is_empty()
                            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') =>
                    {
                        Some(name.to_owned())
                    }
                    _ => {
                        return Err(invalid(format!(
                            "{path}/fix/range: unknown range `{range}`; expected witness.span or capture.<NAME>"
                        )));
                    }
                },
                None => {
                    return Err(invalid(format!(
                        "{path}/fix/range is required: witness.span or capture.<NAME>"
                    )));
                }
            }
        }
        Some(_) => {
            return Err(invalid(format!(
                "{path}/fix must be an ast-grep fix template string or {{ template, range }}"
            )));
        }
    };
    if let Some(variables) = violation.get("variables")
        && !object(variables, &format!("{path}/variables"))?.is_empty()
    {
        return Err(invalid(format!(
            "{path}/variables: violation variables are not supported yet; \
             violations carry only the kernel's own variables (subjectId, count, anchorFallback)"
        )));
    }
    Ok(ViolationSpec {
        code: required_str(value, "code", path)?.to_owned(),
        message: required_str(value, "message", path)?.to_owned(),
        anchor,
        message_template: None,
        fix: None,
        node_regexes: Vec::new(),
        each: violation
            .get("each")
            .map(|each| non_empty_str(Some(each), &format!("{path}/each")).map(str::to_owned))
            .transpose()?,
        fix_range,
        group_transform: None,
        project: Vec::new(),
        suppress: violation
            .get("suppress")
            .map(|suppress| super::suppress::parse(suppress, &format!("{path}/suppress")))
            .transpose()?,
    })
}

/// The captures a rule declares, with where: a pattern's metavariables (as the first grammar that
/// compiles it reads them), a `capture`, the names a parameterized call returns into, and a count's
/// `as`. Nested rules at any depth; `matches` by name is not followed.
fn declared_captures(
    value: &Value,
    path: &str,
    languages: &[GrammarDialect],
    found: &mut Vec<(String, String)>,
) {
    match value {
        Value::Object(map) => {
            for (key, item) in map {
                let at = format!("{path}/{key}");
                match (key.as_str(), item) {
                    ("pattern", pattern) => {
                        let vars = languages.iter().find_map(|language| {
                            let source = match pattern {
                                Value::String(source) => PatternSource::Plain(source),
                                other => PatternSource::Contextual {
                                    context: other
                                        .get("context")
                                        .and_then(Value::as_str)
                                        .unwrap_or_default(),
                                    selector: other
                                        .get("selector")
                                        .and_then(Value::as_str)
                                        .unwrap_or_default(),
                                },
                            };
                            rule_pattern(source, *language).ok().map(|(compiled, _)| {
                                compiled
                                    .defined_vars()
                                    .into_iter()
                                    .map(str::to_owned)
                                    .collect::<Vec<_>>()
                            })
                        });
                        found.extend(
                            vars.unwrap_or_default()
                                .into_iter()
                                .map(|name| (name, at.clone())),
                        );
                    }
                    ("capture", Value::String(name)) => found.push((name.clone(), at)),
                    ("count", Value::Object(count)) => {
                        if let Some(Value::String(name)) = count.get("as") {
                            found.push((name.clone(), format!("{at}/as")));
                        }
                        declared_captures(item, &at, languages, found);
                    }
                    ("matches", Value::Object(calls)) => {
                        for (util, arguments) in calls {
                            for (parameter, argument) in arguments.as_object().into_iter().flatten()
                            {
                                if let Some(name) =
                                    argument.as_str().and_then(|text| text.strip_prefix('$'))
                                {
                                    found.push((
                                        name.to_owned(),
                                        format!("{at}/{util}/{parameter}"),
                                    ));
                                }
                            }
                        }
                    }
                    _ => declared_captures(item, &at, languages, found),
                }
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                declared_captures(item, &format!("{path}/{index}"), languages, found);
            }
        }
        _ => {}
    }
}

/// Each utility's `UtilOrigin`, in `utils` order: a digest of it and of every utility it reaches.
fn util_origins(utils: &BTreeMap<String, UtilDef>, language: &str) -> Result<Vec<UtilOrigin>> {
    use sha2::{Digest, Sha256};
    utils
        .iter()
        .map(|(id, util)| {
            let mut reached = reachable(std::iter::once(util.rule), utils);
            reached.insert(id.clone());
            let body = serde_json::to_vec(&(
                language,
                id,
                reached
                    .iter()
                    .map(|id| (id, &utils[id].params, utils[id].rule))
                    .collect::<Vec<_>>(),
            ))
            .map_err(|error| invalid(error.to_string()))?;
            Ok(UtilOrigin {
                digest: Sha256::digest(&body).into(),
            })
        })
        .collect()
}

/// Refuse predicates a code-rule language's semantic family does not implement before any dialect
/// reaches a family-specific compiler.
pub(super) fn refuse_unavailable_semantics(
    value: &Value,
    path: &str,
    language: CodeRuleLanguage,
) -> Result<()> {
    let semantic = |predicate: &str| match predicate {
        "resolves" | "binds" => Some(Semantic::Scope),
        "value" | "compare" => Some(Semantic::Value),
        "arguments" => Some(Semantic::Arguments),
        "calls" => Some(Semantic::Calls),
        "formatArgs" => Some(Semantic::FormatArgs),
        _ => None,
    };
    match value {
        Value::Object(object) => {
            for (predicate, child) in object {
                if semantic(predicate)
                    .is_some_and(|semantic| !language.family().implements(semantic))
                {
                    return Err(invalid(format!(
                        "{path}/{predicate}: `{predicate}` is not available for {} policies",
                        language.name()
                    )));
                }
                refuse_unavailable_semantics(child, &format!("{path}/{predicate}"), language)?;
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                refuse_unavailable_semantics(child, &format!("{path}/{index}"), language)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::code_rule_dsl::CodeRuleDslRuntime;
    use crate::code_rule_dsl::test_support::*;
    use serde_json::{Value, json};

    #[test]
    fn duplicate_keys_are_rejected_in_json_and_yaml() {
        // A JSON object keeps the last of two equal keys, so the first term vanished silently.
        // ast-grep 0.45.3 rejects the rule ("duplicate field `not`").
        let json_source = r#"{"format": "rerail.policy/v1", "id": "p", "revision": 1, "mode": "steer", "clauses": [{
            "id": "c",
            "forbid": {"ast": {"language": "python", "rule":
                {"kind": "identifier", "not": {"regex": "^a$"}, "not": {"regex": "^b$"}}}},
            "violation": {"anchor": "witness.span", "code": "X", "message": "x"}}]}"#;
        let reason = rejection(CodeRuleDslRuntime::compile(
            json_source.to_owned(),
            "json".to_owned(),
            CAPABILITIES.to_owned(),
        ));
        assert!(
            reason.starts_with("duplicate key `not` at /clauses/0/forbid/ast/rule at line 4"),
            "{reason}"
        );
        let yaml_source = "format: rerail.policy/v1\nid: p\nrevision: 1\nid: q\nclauses: []\n";
        let reason = rejection(CodeRuleDslRuntime::compile(
            yaml_source.to_owned(),
            "yaml".to_owned(),
            CAPABILITIES.to_owned(),
        ));
        assert!(
            reason.starts_with("invalid YAML: duplicate key `id` at /"),
            "{reason}"
        );
        // Equal keys in different objects are fine.
        let distinct = r#"{"format": "rerail.policy/v1", "id": "p", "revision": 1, "mode": "steer", "clauses": [{
            "id": "c",
            "forbid": {"ast": {"language": "python", "rule":
                {"kind": "identifier", "not": {"regex": "^a$", "not": {"regex": "^b$"}}}}},
            "violation": {"anchor": "witness.span", "code": "X", "message": "x"}}]}"#;
        CodeRuleDslRuntime::compile(
            distinct.to_owned(),
            "json".to_owned(),
            CAPABILITIES.to_owned(),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
    }

    #[test]
    fn violation_variables_are_validated() {
        let forbid = json!({ "regex": { "source": "file.content", "pattern": "x" } });
        let compile = |violation: Value| {
            compile_document_with(
                clause_document(json!({ "id": "c", "forbid": forbid, "violation": violation })),
                &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
            )
        };
        let mut variables = violation();
        variables["variables"] = json!({ "name": "x" });
        assert!(rejection(compile(variables)).starts_with(
            "/clauses/0/violation/variables: violation variables are not supported yet"
        ));
        let mut empty = violation();
        empty["variables"] = json!({});
        assert!(compile(empty).is_ok());
    }

    fn regex_document(violation: Value) -> Value {
        clause_document(json!({
            "id": "c",
            "forbid": { "regex": { "source": "file.content", "pattern": "x" } },
            "violation": violation,
        }))
    }

    #[test]
    fn mode_is_required_and_severity_is_unknown() {
        let capabilities = serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities");
        let mut missing = regex_document(violation());
        missing.as_object_mut().expect("document").remove("mode");
        let mut non_string = regex_document(violation());
        non_string["mode"] = json!(1);
        let mut severity_name = regex_document(violation());
        severity_name["mode"] = json!("advisory");
        for document in [missing, non_string, severity_name] {
            assert_eq!(
                rejection(compile_document_with(document, &capabilities)),
                "/mode must equal log, steer, or block"
            );
        }
        let mut with_severity = violation();
        with_severity["severity"] = json!("error");
        assert_eq!(
            rejection(compile_document_with(
                regex_document(with_severity),
                &capabilities
            )),
            "unknown field `severity` at /clauses/0/violation"
        );
    }

    #[test]
    fn violations_carry_the_rule_mode() {
        let capabilities = serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities");
        for mode in ["log", "steer", "block"] {
            let mut document = regex_document(violation());
            document["mode"] = json!(mode);
            let compiled = compile_document_with(document, &capabilities)
                .unwrap_or_else(|error| panic!("{}", error.reason));
            let result = evaluate_file(&compiled, "typescript", "a.ts", "x;\n");
            assert_violations(&result, 1);
            assert_eq!(result["violations"][0]["mode"], mode);
            assert!(result["violations"][0].get("severity").is_none());
        }
    }

    /// A constraint may name what a call binds through a cycle of parameterized calls, whichever
    /// `any` branch the walk collecting the query's metavariables meets first.
    #[test]
    fn constraints_on_captures_bound_through_recursive_calls_do_not_depend_on_branch_order() {
        for branches in recursive_call_branch_orders() {
            if let Err(error) = compile_typescript_ast(json!({
                "rule": { "kind": "expression_statement", "any": branches },
                "constraints": { "X": { "regex": "^g" } },
                "utils": mutually_recursive_calls(),
            })) {
                panic!("{branches}: {}", error.reason);
            }
        }
    }
}
