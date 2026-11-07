//! Helpers shared by the code-rule DSL's unit tests.

use super::matcher::GrammarMatcher;
use super::model::{ClauseKind, Query};
use super::{CodeRuleDslRuntime, CompiledCodeRuleDsl};
use napi::Result;
use serde_json::{Map, Value, json};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(super) const CAPABILITIES: &str = include_str!("capabilities.json");

/// Pack test data at `relative` (a repository path under `packs/<id>/upstream/<tool>/`), which is not in
/// git (scripts/s3/pack-fixtures.mjs): its path in the checkout, or a failure saying to fetch and derive it.
pub(super) fn pack_test_data(relative: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(relative);
    assert!(
        path.exists(),
        "{relative} is missing: pack test data is not in git: run `npm run fixtures:pull` (each \
         tool-pack's archive, which its test-data.lock pins, from S3) (CONTRIBUTING.md, \"Pack test \
         data\")"
    );
    path
}

pub(super) const JSX_SOURCE: &str =
    "export const View = () => <button type=\"button\">Go</button>;\n";

pub(super) fn compile_typescript_rule(rule: Value) -> Result<CompiledCodeRuleDsl> {
    let document = json!({
        "format": "rerail.policy/v1",
        "id": "dialects",
        "revision": 1,
        "mode": "block",
        "clauses": [{
            "id": "ast-match",
            "forbid": { "ast": { "language": "typescript", "rule": rule } },
            "violation": { "anchor": "witness.span", "code": "AST_MATCH", "message": "match" },
        }],
    });
    CodeRuleDslRuntime::compile(
        document.to_string(),
        "json".to_owned(),
        CAPABILITIES.to_owned(),
    )
}

pub(super) fn evaluate_file(
    compiled: &CompiledCodeRuleDsl,
    language: &str,
    path: &str,
    source: &str,
) -> Value {
    let snapshot = json!({
        "facts": [],
        "files": [{ "language": language, "path": path, "source": source }],
        "format": "rerail.repository-snapshot/v1",
        "revision": 0,
    });
    let output = compiled
        .evaluate(snapshot.to_string(), "report".to_owned())
        .expect("evaluation succeeds");
    serde_json::from_str(&output).expect("evaluation is JSON")
}

pub(super) fn assert_violations(result: &Value, expected: usize) {
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(result["failures"], json!([]), "{result}");
    assert_eq!(
        result["violations"].as_array().map(Vec::len),
        Some(expected),
        "{result}"
    );
}

pub(super) fn compile_document_with(
    document: Value,
    capabilities: &Value,
) -> Result<CompiledCodeRuleDsl> {
    CodeRuleDslRuntime::compile(
        document.to_string(),
        "json".to_owned(),
        capabilities.to_string(),
    )
}

pub(super) fn clause_document(clause: Value) -> Value {
    json!({ "format": "rerail.policy/v1", "id": "strict", "revision": 1, "mode": "block", "clauses": [clause] })
}

pub(super) fn violation() -> Value {
    json!({ "anchor": "witness.span", "code": "X", "message": "x" })
}

pub(super) fn capabilities_with(edit: impl FnOnce(&mut Value)) -> Value {
    let mut capabilities: Value = serde_json::from_str(CAPABILITIES).expect("capabilities");
    edit(&mut capabilities);
    capabilities
}

pub(super) fn rejection(result: Result<CompiledCodeRuleDsl>) -> String {
    result.err().expect("compile is rejected").reason
}

/// Compiles one typescript ast query; `ast` holds `rule` and optionally `utils`/`constraints`.
pub(super) fn compile_typescript_ast(mut ast: Value) -> Result<CompiledCodeRuleDsl> {
    ast["language"] = json!("typescript");
    compile_document_with(
        clause_document(
            json!({ "id": "ast-match", "forbid": { "ast": ast }, "violation": violation() }),
        ),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
}

pub(super) fn compiled(ast: Value) -> CompiledCodeRuleDsl {
    compile_typescript_ast(ast).unwrap_or_else(|error| panic!("{}", error.reason))
}

/// ESLint no-dupe-keys as the TypeScript conformance pack ports it (`eslint-no-dupe-keys`): a
/// property whose static key (`compare` text `key`) an earlier property of the same object has;
/// a getter pairs with a setter, and a non-computed `__proto__: value` is no key. It reports the
/// key, `K`.
pub(super) fn no_dupe_keys_ast() -> Value {
    let earlier = |extra: Value| {
        let mut all = vec![json!({ "matches": "cand" })];
        all.extend(extra.as_array().cloned().unwrap_or_default());
        json!({ "count": { "min": 1, "follows": {
            "stopBy": "end",
            "all": all,
            "pattern": "$Q",
            "compare": { "left": "Q", "op": "eq", "right": "P", "text": "key" },
        } } })
    };
    json!({
        "language": "typescript",
        "rule": { "all": [
            { "matches": "cand" },
            { "pattern": "$P" },
            { "any": [
                { "kind": "shorthand_property_identifier", "pattern": "$K" },
                { "has": { "stopBy": "neighbor", "matches": "key", "not": { "kind": "computed_property_name" }, "pattern": "$K" } },
                { "has": { "stopBy": "neighbor", "matches": "key", "kind": "computed_property_name",
                    "has": { "stopBy": "neighbor", "not": { "regex": "^[\\[\\]]$" }, "pattern": "$K" } } },
            ] },
            { "any": [
                { "all": [{ "matches": "getter" }, earlier(json!([{ "not": { "matches": "setter" } }]))] },
                { "all": [{ "matches": "setter" }, earlier(json!([{ "not": { "matches": "getter" } }]))] },
                { "all": [
                    { "not": { "matches": "getter" } },
                    { "not": { "matches": "setter" } },
                    earlier(json!([])),
                ] },
            ] },
        ] },
        "utils": {
            "cand": { "all": [
                { "any": [{ "kind": "pair" }, { "kind": "method_definition" }, { "kind": "shorthand_property_identifier" }] },
                { "inside": { "kind": "object", "stopBy": "neighbor" } },
                { "not": { "matches": "proto" } },
            ] },
            "getter": { "kind": "method_definition", "has": { "stopBy": "neighbor", "regex": "^get$", "not": { "kind": "property_identifier" } } },
            "setter": { "kind": "method_definition", "has": { "stopBy": "neighbor", "regex": "^set$", "not": { "kind": "property_identifier" } } },
            "key": {
                "any": [{ "kind": "property_identifier" }, { "kind": "string" }, { "kind": "number" }, { "kind": "computed_property_name" }],
                "not": { "follows": { "stopBy": "end", "regex": "^:$" } },
            },
            "proto": { "kind": "pair", "has": { "stopBy": "neighbor", "matches": "key", "any": [
                { "kind": "property_identifier", "regex": "^__proto__$" },
                { "kind": "string", "value": { "eq": "__proto__" } },
            ] } },
        },
    })
}

/// Each violation's text, followed by its single-node captures' texts in name order.
pub(super) fn matched_texts(result: &Value, source: &str) -> Vec<String> {
    let text = |span: &Value| {
        let start = span["startByte"].as_u64().expect("start") as usize;
        let end = span["endByte"].as_u64().expect("end") as usize;
        source[start..end].to_owned()
    };
    result["violations"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|violation| {
            let mut parts = vec![text(&violation["span"])];
            for (name, spans) in violation["captures"].as_object().expect("captures") {
                if let [span] = spans.as_array().expect("spans").as_slice() {
                    parts.push(format!("{name}={}", text(span)));
                }
            }
            parts.join(" ")
        })
        .collect()
}

pub(super) fn grammar_matchers(compiled: &CompiledCodeRuleDsl) -> &[GrammarMatcher] {
    let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &compiled.inner.clauses[0].kind else {
        unreachable!()
    };
    matchers
}

/// A utility-graph walk whose work a test counts: each time it expands a utility's rule (for
/// `BindingSitePatterns`, each binding-site pattern it compiles).
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Walk {
    SoundVerdict,
    OptionalCaptures,
    CaptureVariables,
    AstCaptures,
    NodeRegexes,
    OrderBinds,
    OrderReads,
    OrderMentions,
    BindingSitePatterns,
    AnchorInlining,
    /// P's: an anchor index build (P's SQL anchor engine).
    AnchorBuild,
    /// A shared utility plan's conditions evaluated by one anchor selection.
    AnchorSelection,
    /// A shared utility plan whose value-posting needs an anchor index build collects.
    AnchorNeeds,
    /// A root an anchor text selection visits: its scope, which the regex or a value posting then
    /// tests.
    AnchorTextNode,
    /// A sibling an anchor selection's sibling walk around zero-width nodes steps over, or a child
    /// it reads while building a parent's run of siblings.
    AnchorSibling,
    /// A node the Rust usage index visits to find the nodes its references span.
    UsageNode,
}

thread_local! {
    static WALKS: RefCell<BTreeMap<Walk, usize>> = const { RefCell::new(BTreeMap::new()) };
}

pub(super) fn record_walk(walk: Walk) {
    WALKS.with(|walks| *walks.borrow_mut().entry(walk).or_default() += 1);
}

pub(super) fn reset_walks() {
    WALKS.with(|walks| walks.borrow_mut().clear());
}

pub(super) fn walks(walk: Walk) -> usize {
    WALKS.with(|walks| walks.borrow().get(&walk).copied().unwrap_or_default())
}

/// Levels of the utility diamond fixtures: small enough that a walk doubling per level still ends
/// quickly, large enough that it exceeds `linear_walk`.
pub(super) const DIAMOND_DEPTH: usize = 12;

/// The most expansions a walk may spend on a utility chain of `depth` levels: a constant number
/// per utility, whatever the number of paths reaching it.
pub(super) fn linear_walk(depth: usize) -> usize {
    4 * (depth + 1)
}

/// Utilities `u0..=u{depth}`, each `u{n}` matching `u{n-1}` twice: `2^depth` paths reach `u0`.
pub(super) fn utility_diamonds(depth: usize) -> Value {
    let mut utils = Map::new();
    utils.insert("u0".to_owned(), json!({ "kind": "identifier" }));
    for level in 1..=depth {
        let prior = format!("u{}", level - 1);
        utils.insert(
            format!("u{level}"),
            json!({ "all": [{ "matches": prior }, { "matches": prior }] }),
        );
    }
    Value::Object(utils)
}

/// `utility_diamonds` with parameterized utilities, each passing its `P` to both calls.
pub(super) fn parameterized_utility_diamonds(depth: usize) -> Value {
    let mut utils = Map::new();
    utils.insert(
        "u0".to_owned(),
        json!({ "params": ["P"], "rule": { "pattern": "$P" } }),
    );
    for level in 1..=depth {
        let call = json!({ format!("u{}", level - 1): { "P": "$P" } });
        utils.insert(
            format!("u{level}"),
            json!({ "params": ["P"], "rule": { "all": [{ "matches": call }, { "matches": call }] } }),
        );
    }
    Value::Object(utils)
}

/// A query matching the top of `utility_diamonds(depth)`.
pub(super) fn compiled_utility_diamonds(depth: usize) -> CompiledCodeRuleDsl {
    compiled(
        json!({ "rule": { "matches": format!("u{depth}") }, "utils": utility_diamonds(depth) }),
    )
}

/// A query calling the top of `parameterized_utility_diamonds(depth)` with a capture it binds.
pub(super) fn compiled_parameterized_utility_diamonds(depth: usize) -> CompiledCodeRuleDsl {
    compiled(json!({
        "rule": { "all": [
            { "pattern": "$X" },
            { "matches": { format!("u{depth}"): { "P": "$X" } } },
        ] },
        "utils": parameterized_utility_diamonds(depth),
    }))
}

/// Utilities `u0..=u{depth}`, each `u{n}` matching `u{n-1}` once, under a relation beside a kind
/// test: one path reaches each utility, so every walk over it is as long as the chain.
pub(super) fn utility_chain(depth: usize) -> Value {
    let mut utils = Map::new();
    utils.insert("u0".to_owned(), json!({ "pattern": "$A" }));
    for level in 1..=depth {
        utils.insert(
            format!("u{level}"),
            json!({ "has": { "all": [{ "matches": format!("u{}", level - 1) }, { "kind": "identifier" }] } }),
        );
    }
    Value::Object(utils)
}

/// Parameterized utilities that call each other: `V(R)` binds `R` through an outward relation
/// under `any`, or calls `W` with a capture of its own; `W(P)` calls `V` with `P`. Each binds its
/// parameter only through the other's summary, so whichever of them a walk meets first, it reaches
/// the other while the first is still being summarized.
pub(super) fn mutually_recursive_calls() -> Map<String, Value> {
    let utils = json!({
        "V": { "params": ["R"], "rule": { "any": [
            { "follows": { "kind": "expression_statement", "pattern": "$R" } },
            { "has": { "matches": { "W": { "P": "$S" } } } },
        ] } },
        "W": { "params": ["P"], "rule": { "matches": { "V": { "R": "$P" } } } },
    });
    let Value::Object(utils) = utils else {
        unreachable!()
    };
    utils
}

/// `mutually_recursive_calls`' two calls, `V` binding `$A` and `W` binding `$X`, as `any`
/// branches in both orders.
pub(super) fn recursive_call_branch_orders() -> [Value; 2] {
    let v = json!({ "matches": { "V": { "R": "$A" } } });
    let w = json!({ "matches": { "W": { "P": "$X" } } });
    [json!([v.clone(), w.clone()]), json!([w, v])]
}

/// The text `json_encode` with `JSON_INVALID_UTF8_SUBSTITUTE` writes for bytes: one U+FFFD per
/// sequence `php_next_utf8_char` refuses, skipping as far as it does.
pub(super) fn php_lossy(bytes: &[u8]) -> String {
    let lead = |b: u8| b < 0x80 || (0xc2..=0xf4).contains(&b);
    let trail = |b: Option<&u8>| b.is_some_and(|b| (0x80..=0xbf).contains(b));
    let mut out = String::new();
    let mut at = 0;
    while at < bytes.len() {
        let c = bytes[at];
        let width = match c {
            0..=0x7f => Ok(1),
            0xc2..=0xdf => {
                if trail(bytes.get(at + 1)) {
                    Ok(2)
                } else {
                    Err(if bytes.get(at + 1).is_none_or(|b| lead(*b)) {
                        1
                    } else {
                        2
                    })
                }
            }
            0xe0..=0xf4 => {
                let width = if c < 0xf0 { 3 } else { 4 };
                match (1..width).find(|offset| !trail(bytes.get(at + offset))) {
                    Some(offset) => Err((1..=offset)
                        .find(|i| bytes.get(at + i).is_none_or(|b| lead(*b)))
                        .unwrap_or(offset + 1)),
                    None => {
                        if std::str::from_utf8(&bytes[at..at + width]).is_ok() {
                            Ok(width)
                        } else {
                            Err(width)
                        }
                    }
                }
            }
            _ => Err(1),
        };
        match width {
            Ok(width) => {
                out.push_str(std::str::from_utf8(&bytes[at..at + width]).expect("UTF-8"));
                at += width;
            }
            Err(skip) => {
                out.push('\u{fffd}');
                at += skip;
            }
        }
    }
    out
}
