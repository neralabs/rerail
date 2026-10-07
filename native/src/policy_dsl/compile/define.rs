//! `define` checks (a Rerail extension): definitions a document shares among its checks. A define
//! reports nothing. Its `utils` are visible to every ast query of its `language`, as if each query
//! that names one listed it in its own `utils`: the query compiles it with its own options.

use super::ast_rule::{compile_ast_rule, validate_ast_rule};
use super::config::Options;
use super::dialect::language_capability;
use super::refuse_unavailable_semantics;
use super::utils::{UtilDef, UtilIds, UtilScope, reject_util_cycles, util_map};
use crate::policy_dsl::json::{invalid, object, reject_unknown, required_str};
use crate::policy_dsl::language::PolicyLanguage;
use crate::policy_dsl::matcher::UtilTable;
use crate::policy_dsl::model::Capabilities;
use napi::Result;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// The constraint forms a check other than a define holds.
pub(super) const CHECK_FORMS: [&str; 4] = ["forbid", "require_each", "budget", "llm"];

/// Each language's shared utilities, by id.
pub(super) type SharedUtils<'a> = BTreeMap<String, BTreeMap<String, UtilDef<'a>>>;

/// What the document's defines hold: shared utilities and options.
pub(super) struct Defines<'a> {
    pub(super) utils: SharedUtils<'a>,
    pub(super) options: Options,
}

/// Whether a check is a define.
pub(super) fn is_define(check: &Value) -> bool {
    check.get("define").is_some()
}

/// Reads and validates the document's defines. Their utilities are checked as a query's are: shape,
/// cycles, and whole in some dialect of their language, used or not.
pub(super) fn collect_defines<'a>(
    checks: &'a [Value],
    capabilities: &Capabilities,
) -> Result<SharedUtils<'a>> {
    let mut shared = SharedUtils::new();
    for (index, check) in checks.iter().enumerate() {
        let Some(define) = check.get("define") else {
            continue;
        };
        let path = format!("/checks/{index}");
        let fields = object(check, &path)?;
        if fields.contains_key("finding") {
            return Err(invalid(format!(
                "{path}/finding: a define check reports nothing, so it has no finding"
            )));
        }
        if CHECK_FORMS.iter().any(|form| fields.contains_key(*form)) {
            return Err(invalid(format!(
                "{path} must contain exactly one constraint form"
            )));
        }
        reject_unknown(fields, &["id", "define"], &path)?;
        required_str(check, "id", &path)?;
        let define_path = format!("{path}/define");
        let body = object(define, &define_path)?;
        reject_unknown(
            body,
            &["language", "utils", "options", "config"],
            &define_path,
        )?;
        let language = body
            .get("language")
            .map(|language| {
                language
                    .as_str()
                    .and_then(PolicyLanguage::parse)
                    .map(PolicyLanguage::name)
                    .ok_or_else(|| {
                        invalid(format!(
                            "{define_path}/language must equal python, typescript, javascript or php"
                        ))
                    })
            })
            .transpose()?;
        // Options and their configuration were read before the document was resolved.
        let Some(utils) = body.get("utils") else {
            if body.contains_key("options") || body.contains_key("config") {
                continue;
            }
            return Err(invalid(format!(
                "{define_path} must hold utils, options or config"
            )));
        };
        let Some(language) = language else {
            return Err(invalid(format!(
                "{define_path}/language is required: utilities compile for one language"
            )));
        };
        let known = shared.entry(language.to_owned()).or_default();
        for (id, util) in util_map(Some(utils), &format!("{define_path}/utils"))? {
            if known.contains_key(&id) {
                return Err(invalid(format!(
                    "{}: utility rule `{id}` is already defined by another define of {language}",
                    util.path
                )));
            }
            known.insert(id, util);
        }
    }
    for (language, utils) in &shared {
        let ids = utils
            .iter()
            .map(|(id, util)| (id.as_str(), util.params.as_slice()))
            .collect::<UtilIds>();
        let policy_language =
            PolicyLanguage::parse(language).expect("a define's language was parsed above");
        for util in utils.values() {
            refuse_unavailable_semantics(util.rule, &util.rule_path, policy_language)?;
            validate_ast_rule(util.rule, &util.rule_path, &capabilities.limits, &ids)?;
        }
        reject_util_cycles(utils)?;
        check_dialects(language, utils, &ids, capabilities)?;
    }
    Ok(shared)
}

/// Compiles the shared utilities of one language for each of its dialects: each must be whole in
/// at least one, as a query's rule must.
fn check_dialects<'u>(
    language: &str,
    utils: &'u BTreeMap<String, UtilDef<'u>>,
    ids: &UtilIds,
    capabilities: &Capabilities,
) -> Result<()> {
    let (grammars, _) = language_capability(language, capabilities)?;
    let mut rejections = BTreeMap::<&str, Vec<(String, &str)>>::new();
    let mut whole = BTreeSet::new();
    // An `all` ordering conflict depends on what the callers bind; the queries that call a shared
    // utility settle it there.
    let ignored = std::cell::RefCell::new(BTreeSet::new());
    // A shared utility's `calls` terms are the calling queries' (their seeds carry its rule).
    let calls = crate::policy_dsl::calls::CallsCollector::new(String::new(), false);
    for (dialect, grammar) in &grammars {
        let table = Arc::new(UtilTable::new(
            utils.keys().cloned().collect(),
            utils.values().map(|util| !util.params.is_empty()).collect(),
        ));
        let scope = UtilScope {
            table: Arc::downgrade(&table),
            ids,
            options: None,
            python: None,
            transform: None,
            conflicts: Some(&ignored),
            captures: Default::default(),
            calls: Some(&calls),
        };
        let compile = |gaps: &mut BTreeMap<&'u str, Vec<String>>| {
            utils
                .iter()
                .map(|(id, util)| {
                    let gaps = gaps.entry(id.as_str()).or_default();
                    compile_ast_rule(util.rule, *grammar, &util.rule_path, gaps, &scope)
                })
                .collect::<Result<Vec<_>>>()
        };
        let mut gaps = BTreeMap::new();
        let compiled = compile(&mut gaps)?;
        if table.was_unfilled() {
            if table.draft.set(compiled).is_err() {
                unreachable!("a fresh table is drafted once");
            }
            gaps.clear();
            compile(&mut gaps)?;
        }
        for (id, gaps) in gaps {
            match gaps.into_iter().next() {
                Some(reason) => rejections.entry(id).or_default().push((reason, *dialect)),
                None => {
                    whole.insert(id);
                }
            }
        }
    }
    let Some((_, rejections)) = rejections
        .into_iter()
        .find(|(id, reasons)| !whole.contains(id) && !reasons.is_empty())
    else {
        return Ok(());
    };
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
    Err(invalid(message))
}

#[cfg(test)]
mod tests {
    use crate::policy_dsl::test_support::*;
    use serde_json::{Value, json};

    fn document(checks: Value) -> Value {
        json!({ "format": "rerail.policy/v1", "id": "shared", "revision": 1, "checks": checks })
    }

    fn query(id: &str, language: &str, ast: Value) -> Value {
        let mut ast = ast;
        ast["language"] = json!(language);
        json!({ "id": id, "forbid": { "ast": ast }, "finding": finding() })
    }

    fn compile(checks: Value) -> napi::Result<crate::policy_dsl::CompiledPolicyDsl> {
        compile_document_with(
            document(checks),
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
    }

    fn check_ids(result: &Value) -> Vec<String> {
        result["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|finding| finding["checkId"].as_str().expect("check id").to_owned())
            .collect()
    }

    #[test]
    fn a_define_shares_its_utilities_with_every_query_of_its_language() {
        let compiled = compile(json!([
            { "id": "shared", "define": { "language": "typescript", "utils": {
                "call-of": { "kind": "call_expression", "has": { "field": "function", "matches": "named-f" } },
                "named-f": { "kind": "identifier", "regex": "^f$" },
            } } },
            query("statement", "typescript", json!({ "rule": { "kind": "expression_statement", "has": { "matches": "call-of" } } })),
            query("argument", "typescript", json!({
                "rule": { "kind": "arguments", "has": { "matches": "local" } },
                "utils": { "local": { "matches": "call-of" } },
            })),
        ]))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "f(f());\ng();\n";
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        assert_eq!(check_ids(&result), ["statement", "argument"], "{result}");
    }

    #[test]
    fn a_shared_utility_compiles_with_each_querys_options() {
        let util = json!({ "kind": "identifier", "regex": "^window$", "resolves": { "global": ["window"] } });
        let compiled = compile(json!([
            { "id": "shared", "define": { "language": "typescript", "utils": { "window": util } } },
            query("declared", "typescript", json!({
                "rule": { "matches": "window" },
                "languageOptions": { "globals": { "window": "readonly" } },
            })),
            query("undeclared", "typescript", json!({ "rule": { "matches": "window" }, "languageOptions": {} })),
        ]))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "typescript", "a.ts", "window.x;\n");
        assert_eq!(check_ids(&result), ["declared"], "{result}");
    }

    #[test]
    fn a_shared_utility_is_invisible_to_another_language_and_reports_nothing() {
        let define = json!({ "id": "shared", "define": { "language": "typescript", "utils": { "u": { "kind": "identifier" } } } });
        assert_eq!(
            rejection(compile(json!([
                define,
                query("py", "python", json!({ "rule": { "matches": "u" } }))
            ]))),
            "/checks/1/forbid/ast/rule/matches: utility rule `u` is not defined in this query's utils or a define of its language"
        );
        assert_eq!(
            rejection(compile(json!([define]))),
            "/checks must hold a check besides define: a define reports nothing"
        );
    }

    #[test]
    fn a_shared_utility_is_refused_semantics_its_language_lacks_used_or_not() {
        let ts = |util: Value| json!({ "id": "shared", "define": { "language": "typescript", "utils": { "u": util } } });
        let calls =
            json!({ "kind": "call_expression", "calls": { "kind": "function_declaration" } });
        assert_eq!(
            rejection(compile(json!([
                ts(calls.clone()),
                query(
                    "q",
                    "typescript",
                    json!({ "rule": { "kind": "identifier" } })
                )
            ]))),
            "/checks/0/define/utils/u/calls: `calls` is not available for typescript policies"
        );
        assert_eq!(
            rejection(compile(json!([
                ts(calls),
                query("q", "typescript", json!({ "rule": { "matches": "u" } }))
            ]))),
            "/checks/0/define/utils/u/calls: `calls` is not available for typescript policies"
        );
    }

    #[test]
    fn defines_are_checked_where_they_are_written() {
        let ts = |utils: Value| json!({ "id": "shared", "define": { "language": "typescript", "utils": utils } });
        let rule = query(
            "q",
            "typescript",
            json!({ "rule": { "kind": "identifier" } }),
        );
        for (checks, reason) in [
            (
                json!([{ "id": "d", "define": { "language": "typescript", "utils": {} }, "finding": finding() }, rule]),
                "/checks/0/finding: a define check reports nothing, so it has no finding",
            ),
            (
                json!([{ "id": "d", "define": { "utils": { "u": { "kind": "identifier" } } } }, rule]),
                "/checks/0/define/language is required: utilities compile for one language",
            ),
            (
                json!([{ "id": "d", "define": { "language": "rust", "utils": {} } }, rule]),
                "/checks/0/define/language must equal python, typescript, javascript or php",
            ),
            (
                json!([{ "id": "d", "define": {} }, rule]),
                "/checks/0/define must hold utils, options or config",
            ),
            (
                json!([{ "id": "d", "define": { "language": "typescript", "utils": {}, "rules": {} } }, rule]),
                "unknown field `rules` at /checks/0/define",
            ),
            (
                json!([{ "id": "d", "define": { "language": "typescript", "utils": {} }, "forbid": {} }, rule]),
                "/checks/0 must contain exactly one constraint form",
            ),
            (
                json!([ts(json!({ "u": { "kind": "identifier" } })), { "id": "e", "define": { "language": "typescript", "utils": { "u": { "kind": "number" } } } }, rule]),
                "/checks/1/define/utils/u: utility rule `u` is already defined by another define of typescript",
            ),
            (
                json!([ts(json!({ "u": { "kind": "no_such_kind" } })), rule]),
                "/checks/0/define/utils/u/kind: Kind `no_such_kind` is invalid. (typescript, tsx, javascript grammars)",
            ),
            (
                json!([ts(json!({ "u": { "not": { "matches": "u" } } })), rule]),
                "/checks/0/define/utils/u: utility rule `u` has a cyclic dependency in its `matches` sub-rule",
            ),
            (
                json!([
                    ts(json!({ "u": { "matches": "local" } })),
                    query(
                        "q",
                        "typescript",
                        json!({ "rule": { "matches": "u" }, "utils": { "local": { "kind": "identifier" } } })
                    )
                ]),
                "/checks/0/define/utils/u/matches: utility rule `local` is not defined in this query's utils or a define of its language",
            ),
            (
                json!([
                    ts(json!({ "u": { "kind": "identifier" } })),
                    query(
                        "q",
                        "typescript",
                        json!({ "rule": { "matches": "u" }, "utils": { "u": { "kind": "number" } } })
                    )
                ]),
                "/checks/1/forbid/ast/utils/u: utility rule `u` is also defined by a define of typescript; rename one",
            ),
            (
                json!([ts(json!({ "u": { "kind": "identifier" } })), { "id": "shared", "forbid": { "regex": { "source": "file.content", "pattern": "x" } }, "finding": finding() }]),
                "duplicate check id `shared` at /checks/1",
            ),
        ] {
            assert_eq!(rejection(compile(checks)), reason);
        }
        // A utility only one dialect can express is shared like a query's own.
        compile(json!([
            ts(json!({ "jsx": { "kind": "jsx_self_closing_element" } })),
            rule
        ]))
        .unwrap_or_else(|error| panic!("{}", error.reason));
    }

    #[test]
    fn shared_utilities_recurse_and_a_query_compiles_only_those_it_reaches() {
        let compiled = compile(json!([
            { "id": "shared", "define": { "language": "typescript", "utils": {
                "tail-await": { "any": [
                    { "kind": "await_expression" },
                    { "kind": "parenthesized_expression", "has": { "matches": "tail-await" } },
                ] },
                "jsx-only": { "kind": "jsx_self_closing_element" },
            } } },
            query("tail", "typescript", json!({ "rule": { "kind": "return_statement", "has": { "matches": "tail-await" } } })),
        ]))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "async function f() { return ((await x)); }\n";
        assert_findings(&evaluate_file(&compiled, "typescript", "a.ts", source), 1);
        let utils = &grammar_matchers(&compiled)[0].utils.names;
        assert_eq!(utils, &["tail-await".to_owned()]);
    }
}
