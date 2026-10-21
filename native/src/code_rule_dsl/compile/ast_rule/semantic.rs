use super::super::arguments::{compile_arguments, validate_arguments};
use super::super::binds::{compile_binds, validate_binds};
use super::super::format_args::{compile_format_args, validate_format_args};
use super::super::layout::{compile_layout, validate_layout};
use super::super::parse::{capture_name, compile_parse, validate_parse};
use super::super::resolve::{
    BINDING_KEYS, OCCURRENCE_KEY, compile_binding, compile_resolves, validate_resolves,
};
use super::super::value::{compile_value, validate_value};
use super::*;
use crate::code_rule_dsl::language::Family;
use crate::code_rule_dsl::language::adapter::{
    Adapter, ValueFeature, ValueRules, gate, with_adapter,
};
use crate::code_rule_dsl::matcher::{Comparison, Semantic, SemanticText};

/// The semantic keys' grammar-independent checks, in `validate_ast_rule`'s order.
pub(super) fn validate(
    rule: &serde_json::Map<String, Value>,
    path: &str,
    limits: &Limits,
    utils: &UtilIds,
) -> Result<()> {
    if let Some(resolves) = rule.get("resolves") {
        validate_resolves(resolves, &format!("{path}/resolves"))?;
        for key in BINDING_KEYS.iter().chain(&[OCCURRENCE_KEY]) {
            if let Some(sub) = resolves
                .get("binding")
                .and_then(|binding| binding.get(*key))
            {
                validate_ast_rule(
                    sub,
                    &format!("{path}/resolves/binding/{key}"),
                    limits,
                    utils,
                )?;
            }
        }
    }
    if let Some(value) = rule.get("value") {
        validate_value(value, &format!("{path}/value"), limits)?;
    }
    if let Some(binds) = rule.get("binds") {
        validate_binds(binds, &format!("{path}/binds"), limits, utils)?;
    }
    if let Some(arguments) = rule.get("arguments") {
        validate_arguments(arguments, &format!("{path}/arguments"))?;
    }
    if let Some(parse) = rule.get("parse") {
        validate_parse(parse, &format!("{path}/parse"), limits, utils)?;
    }
    if let Some(format_args) = rule.get("formatArgs") {
        validate_format_args(format_args, &format!("{path}/formatArgs"), limits, utils)?;
    }
    if let Some(value) = rule.get("filePath") {
        super::super::options::text_condition(
            Some(value),
            &format!("{path}/filePath"),
            limits.max_regex_pattern_bytes,
        )?;
    }
    if let Some(layout) = rule.get("layout") {
        validate_layout(layout, &format!("{path}/layout"), limits)?;
    }
    if let Some(calls) = rule.get("calls") {
        let calls_path = format!("{path}/calls");
        let inner = crate::code_rule_dsl::calls::split(calls, &calls_path)?.inner;
        validate_ast_rule(&inner, &calls_path, limits, utils)?;
    }
    if rule.contains_key("capture") {
        return Err(invalid(format!(
            "{path}/capture: capture binds a node of a parsed tree; use it inside parse (a pattern's metavariables capture host nodes)"
        )));
    }
    Ok(())
}

/// A semantic `compare` mode checked against the grammar, and the reading it compares with.
pub(super) fn compare_reading(
    comparison: &mut Comparison,
    text: SemanticText,
    compare: &Value,
    compare_path: &str,
    language: GrammarDialect,
    utils: &UtilScope,
) -> Result<()> {
    let name = compare
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("source");
    gate(
        &format!("{compare_path}/text"),
        with_adapter!(Family::of(language), A => <<A as Adapter>::Value as ValueRules>::feature(
            ValueFeature::Compare(text, name)
        )),
    )?;
    let reading = Reading::new(language, utils.options.clone(), utils.python.clone());
    comparison.reading = Some(match text {
        // getStaticValue in the key's scope (TypeScript), a computed key's value through its
        // binding (PHP): names follow their bindings.
        SemanticText::Member => reading.following(),
        _ => reading,
    });
    Ok(())
}

#[derive(Clone, Copy)]
pub(super) enum Stage {
    Atomic,
    Ordered,
    Late,
}

pub(super) fn compile_key(
    key: &str,
    value: &Value,
    stage: Stage,
    language: GrammarDialect,
    path: &str,
    gaps: &mut Vec<String>,
    utils: &UtilScope,
) -> Option<Result<AstMatcher>> {
    match (stage, key) {
        (Stage::Atomic, "capture") => Some(
            capture_name(value, path)
                .map(|name| AstMatcher::Semantic(Semantic::Capture(name.to_owned()))),
        ),
        (Stage::Atomic, "parse") => Some(compile_parse(value, language, path, gaps, utils)),
        (Stage::Atomic, "arguments") => Some(compile_arguments(value, language, path)),
        (Stage::Ordered, "resolves")
            if value.get("binding").is_some() || value.get("write").is_some() =>
        {
            Some(compile_binding_key(value, language, path, gaps, utils))
        }
        (Stage::Ordered, "binds") => Some(compile_binds(value, language, path, gaps, utils)),
        (Stage::Ordered, "formatArgs") => {
            Some(compile_format_args(value, language, path, gaps, utils))
        }
        (Stage::Atomic, "filePath") => Some(
            super::super::options::text_condition(Some(value), path, usize::MAX).map(|condition| {
                AstMatcher::Semantic(Semantic::FilePath(std::sync::Arc::new(
                    condition.expect("provided value"),
                )))
            }),
        ),
        (Stage::Late, "layout") => Some(compile_layout(value, path)),
        (Stage::Late, "value") => Some(compile_value(
            value,
            language,
            path,
            utils.options.as_ref(),
            utils.python.as_ref(),
        )),
        (Stage::Late, "resolves")
            if value.get("binding").is_none() && value.get("write").is_none() =>
        {
            Some(compile_resolves(
                value,
                language,
                path,
                utils.options.as_ref(),
                utils.python.as_ref(),
            ))
        }
        (Stage::Late, "calls") => Some(compile_calls(value, language, path, utils)),
        _ => None,
    }
}

fn compile_binding_key(
    resolves: &Value,
    language: GrammarDialect,
    path: &str,
    gaps: &mut Vec<String>,
    utils: &UtilScope,
) -> Result<AstMatcher> {
    let empty = Value::Object(serde_json::Map::new());
    let binding = resolves.get("binding").unwrap_or(&empty);
    let write = resolves.get("write").is_some();
    let mut rules = Vec::with_capacity(BINDING_KEYS.len());
    for key in BINDING_KEYS {
        rules.push(
            binding
                .get(*key)
                .map(|sub| compile_ast_rule(sub, language, &format!("{path}/{key}"), gaps, utils))
                .transpose()?,
        );
    }
    let occurrence = binding
        .get(OCCURRENCE_KEY)
        .map(|sub| {
            compile_ast_rule(
                sub,
                language,
                &format!("{path}/{OCCURRENCE_KEY}"),
                gaps,
                utils,
            )
        })
        .transpose()?;
    rules.push(occurrence);
    if rules
        .iter()
        .any(|rule| matches!(rule, Some(AstMatcher::Never)))
    {
        Ok(AstMatcher::Never)
    } else {
        let occurrence = rules.pop().flatten();
        let rules =
            <[_; 3]>::try_from(rules).unwrap_or_else(|_| unreachable!("three binding keys"));
        compile_binding(binding, path, language, rules, occurrence, utils, write)
    }
}

fn compile_calls(
    calls: &Value,
    language: GrammarDialect,
    path: &str,
    utils: &UtilScope,
) -> Result<AstMatcher> {
    use crate::code_rule_dsl::language::adapter::{
        Adapter, CallFeature, CallRules, gate, with_adapter,
    };
    let definition = crate::code_rule_dsl::calls::split(calls, path)?.definition;
    let (verdict, kinds) = with_adapter!(Family::of(language), A => (
        <<A as Adapter>::Calls as CallRules>::feature(CallFeature::Calls),
        if definition { <<A as Adapter>::Calls as CallRules>::FUNCTION_KINDS }
        else { <<A as Adapter>::Calls as CallRules>::CALL_KINDS },
    ));
    gate(path, verdict)?;
    let collector = utils.calls.ok_or_else(|| {
        invalid(format!(
            "{path}: calls is only supported in a forbid query's rule and its utils"
        ))
    })?;
    // Blade numbers PHP's kinds after its own (`GrammarDialect::kind_id`).
    let call_kinds = kinds.iter().map(|kind| language.kind_id(kind)).collect();
    Ok(AstMatcher::Semantic(Semantic::Calls(Box::new(
        collector.term(calls, path, language, call_kinds)?,
    ))))
}
