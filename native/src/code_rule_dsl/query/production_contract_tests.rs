//! Production conformance contracts for SQL lowering.
//!
//! Every input is a complete `rerail.policy/v1` document. The tests compile these documents with
//! the public policy compiler and then invoke `SqlCatalogPlan`; there is no compact-form frontend.

use super::relational::SqlCatalogPlan;
use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::test_support::CAPABILITIES;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Clone)]
struct ContractCase {
    name: String,
    form: String,
    input: Value,
}

fn document(clause: Value) -> Value {
    json!({
        "format": "rerail.policy/v1",
        "id": "sql-contract",
        "revision": 1,
        "mode": "block",
        "clauses": [clause]
    })
}

fn violation() -> Value {
    json!({ "anchor": "witness.span", "code": "SQL", "message": "contract" })
}

fn ast(rule: Value) -> Value {
    ast_query(json!({ "rule": rule }))
}

fn ast_query(mut query: Value) -> Value {
    if query.get("language").is_none() {
        query["language"] = json!("python");
    }
    document(json!({
        "id": "contract",
        "forbid": { "ast": query },
        "violation": violation()
    }))
}

fn ast_violation(rule: Value, violation: Value) -> Value {
    document(json!({
        "id": "contract",
        "forbid": { "ast": { "language": "python", "rule": rule } },
        "violation": violation
    }))
}

fn case(name: &'static str, form: &'static str, input: Value) -> ContractCase {
    ContractCase {
        name: name.to_owned(),
        form: form.to_owned(),
        input,
    }
}

fn authored_cases(source: &str) -> Vec<ContractCase> {
    let fixture: Value =
        serde_json::from_str(source).expect("the authored production contracts are valid JSON");
    fixture
        .as_array()
        .expect("the authored production contracts are an array")
        .iter()
        .map(|entry| ContractCase {
            name: entry["name"]
                .as_str()
                .expect("a production contract has a name")
                .to_owned(),
            form: entry["form"]
                .as_str()
                .expect("a production contract has a compact form")
                .to_owned(),
            input: entry["policy"].clone(),
        })
        .collect()
}

fn cases() -> Vec<ContractCase> {
    let pattern = |source: &str| json!({ "pattern": source });
    let count = |axis: &str, min: usize, max: Option<usize>| {
        let mut spec = json!({ axis: { "kind": "identifier", "stopBy": "end" }, "min": min });
        if let Some(max) = max {
            spec["max"] = json!(max);
        }
        json!({ "kind": "call", "count": spec })
    };
    let util = |rule: Value, utils: Value| ast_query(json!({ "rule": rule, "utils": utils }));
    let semantic = |term: Value| ast(json!({ "kind": "identifier", "all": [term] }));
    let mut cases = authored_cases(include_str!("fixtures/production_contracts_001_020.json"));
    cases.extend(authored_cases(include_str!(
        "fixtures/production_contracts_021_040.json"
    )));
    cases.extend([
        case(
            "nth-child-fixed",
            "nthChild(2)",
            ast(json!({ "nthChild": 2 })),
        ),
        case(
            "nth-child-formula",
            "nthChild(2n+1)",
            ast(json!({ "nthChild": "2n+1" })),
        ),
        case(
            "nth-child-of-rule",
            "nthChild(2, ofRule=not(comment))",
            ast(
                json!({ "nthChild": { "position": 2, "ofRule": { "not": { "kind": "comment" } } } }),
            ),
        ),
        case(
            "count-descendants-range",
            "count(has(kind), 2..4)",
            ast(count("has", 2, Some(4))),
        ),
        case(
            "count-ancestors-min",
            "count(inside(kind), min=1)",
            ast(count("inside", 1, None)),
        ),
        case(
            "count-following-max",
            "count(precedes(kind), max=3)",
            ast(count("precedes", 0, Some(3))),
        ),
        case(
            "count-preceding-exact",
            "count(follows(kind), exact=2)",
            ast(count("follows", 2, Some(2))),
        ),
        case(
            "count-distinct-capture",
            "count(has(pattern($A)), distinct=$A)",
            ast(
                json!({ "kind": "call", "count": { "has": { "pattern": "$A", "stopBy": "end" }, "min": 1 } }),
            ),
        ),
        case(
            "capture-single-binding",
            "pattern($A) then bind($A)",
            ast(pattern("$A")),
        ),
        case(
            "capture-multi-binding",
            "pattern($$$A) then bind-many($A)",
            ast(pattern("f($$$A)")),
        ),
        case(
            "capture-repeated-structural-equality",
            "compare($A, $A, structural)",
            ast(json!({ "pattern": "$A + $A" })),
        ),
        case(
            "capture-cross-term-equality",
            "all(pattern($A), pattern($B), eq($A,$B))",
            ast(
                json!({ "pattern": "$A", "has": { "pattern": "$B" }, "compare": { "left": "A", "op": "eq", "right": "B" } }),
            ),
        ),
        case(
            "capture-cross-term-inequality",
            "all(pattern($A), pattern($B), ne($A,$B))",
            ast(
                json!({ "pattern": "$A", "has": { "pattern": "$B" }, "compare": { "left": "A", "op": "ne", "right": "B" } }),
            ),
        ),
        case(
            "capture-alternative-rollback",
            "any(all(pattern($A),false), pattern($A))",
            ast(json!({ "any": [{ "pattern": "$A", "kind": "comment" }, { "pattern": "$A" }] })),
        ),
        case(
            "capture-negation-isolation",
            "all(pattern($A), not(pattern($B)))",
            ast(json!({ "pattern": "$A", "not": { "pattern": "$B" } })),
        ),
        case(
            "capture-each-distinct",
            "violation.each($A)",
            ast_violation(
                json!({ "kind": "call", "has": { "pattern": "$A", "stopBy": "end" } }),
                json!({ "anchor": "capture.A", "code": "SQL", "message": "$A", "each": "A" }),
            ),
        ),
        case(
            "capture-anchor",
            "anchor(capture=$A)",
            ast_violation(
                pattern("f($A)"),
                json!({ "anchor": "capture.A", "code": "SQL", "message": "$A" }),
            ),
        ),
        case(
            "capture-anchor-fallback",
            "anchor(capture=$MISSING, fallback=span)",
            ast_violation(
                json!({ "any": [{ "pattern": "f($A)" }, { "pattern": "g()" }] }),
                json!({ "anchor": "capture.A", "code": "SQL", "message": "optional" }),
            ),
        ),
        case(
            "capture-transform",
            "transform($A, downcase)",
            ast_query(
                json!({ "rule": pattern("$A"), "transform": { "L": { "convert": { "source": "$A", "toCase": "lowerCase" } } } }),
            ),
        ),
        case(
            "capture-message-template",
            "message(template($A))",
            ast_violation(
                pattern("f($A)"),
                json!({ "anchor": "witness.span", "code": "SQL", "message": "found $A" }),
            ),
        ),
        case(
            "capture-fix-range",
            "fix(range=$A, template=$B)",
            ast_violation(
                pattern("f($A, $B)"),
                json!({ "anchor": "capture.A", "code": "SQL", "message": "$A", "fix": { "template": "$B", "range": "capture.A" } }),
            ),
        ),
        case(
            "capture-projection",
            "project(name=$A, value=$B)",
            ast(pattern("f($A, $B)")),
        ),
        case(
            "utility-simple",
            "matches(simple-kind)",
            util(
                json!({ "matches": "u" }),
                json!({ "u": { "kind": "identifier" } }),
            ),
        ),
        case(
            "utility-nested",
            "matches(outer -> inner)",
            util(
                json!({ "matches": "outer" }),
                json!({ "outer": { "matches": "inner" }, "inner": { "regex": "^x$" } }),
            ),
        ),
        case(
            "utility-shared-component",
            "all(matches(shared), matches(shared))",
            util(
                json!({ "all": [{ "matches": "shared" }, { "matches": "shared" }] }),
                json!({ "shared": { "pattern": "$A" } }),
            ),
        ),
        case(
            "utility-self-recursive",
            "matches(recursive-self)",
            util(
                json!({ "matches": "u" }),
                json!({ "u": { "any": [{ "kind": "identifier" }, { "has": { "matches": "u" } }] } }),
            ),
        ),
        case(
            "utility-mutual-recursion",
            "matches(a -> b -> a)",
            util(
                json!({ "matches": "a" }),
                json!({ "a": { "any": [{ "kind": "identifier" }, { "has": { "matches": "b" } }] }, "b": { "has": { "matches": "a" } } }),
            ),
        ),
        case(
            "utility-parameterized-call",
            "call(util, node=$A, text=$B)",
            util(
                json!({ "pattern": "$A", "matches": { "u": { "P": "$A" } } }),
                json!({ "u": { "params": ["P"], "rule": { "pattern": "$P" } } }),
            ),
        ),
        case(
            "utility-argument-frame-isolation",
            "any(call(util,$A), call(util,$B))",
            util(
                json!({ "pattern": "f($A, $B)", "any": [{ "matches": { "u": { "P": "$A" } } }, { "matches": { "u": { "P": "$B" } } }] }),
                json!({ "u": { "params": ["P"], "rule": { "pattern": "$P" } } }),
            ),
        ),
        case(
            "utility-capture-return",
            "matches(util-binds-$A)",
            util(
                json!({ "matches": "u" }),
                json!({ "u": { "pattern": "$A" } }),
            ),
        ),
        case(
            "utility-depth-limit",
            "matches(recursive, maxDepth)",
            util(
                json!({ "matches": "u" }),
                json!({ "u": { "has": { "matches": "u" } } }),
            ),
        ),
        case(
            "utility-work-limit",
            "matches(recursive, maxVisitedNodes)",
            util(
                json!({ "matches": "u" }),
                json!({ "u": { "has": { "stopBy": "end", "matches": "u" } } }),
            ),
        ),
        case(
            "semantic-resolves",
            "resolves(symbol)",
            semantic(json!({ "resolves": { "global": ["len"] } })),
        ),
        case(
            "semantic-binding-site",
            "resolves.binding.site(rule)",
            semantic(json!({ "resolves": { "binding": { "site": { "kind": "assignment" } } } })),
        ),
        case(
            "semantic-binding-value",
            "resolves.binding.value(rule)",
            semantic(json!({ "resolves": { "binding": { "value": { "pattern": "$V" } } } })),
        ),
        case(
            "semantic-binding-annotation",
            "resolves.binding.annotation(rule)",
            semantic(json!({ "resolves": { "binding": { "annotation": { "pattern": "$T" } } } })),
        ),
        case(
            "semantic-binding-occurrence",
            "resolves.binding.occurrence(rule)",
            ast_query(
                json!({ "language": "typescript", "rule": { "kind": "identifier", "resolves": { "binding": { "occurrence": { "kind": "identifier" } } } } }),
            ),
        ),
        case(
            "semantic-binds",
            "binds(filter, subrules)",
            semantic(
                json!({ "binds": { "form": "assignment", "site": { "kind": "assignment" } } }),
            ),
        ),
        case(
            "semantic-redefines-any",
            "binds.redefines(any)",
            semantic(json!({ "binds": { "redefines": { "used": false } } })),
        ),
        case(
            "semantic-redefines-every",
            "binds.redefines(every)",
            ast_query(
                json!({ "language": "typescript", "rule": { "kind": "identifier", "binds": { "redefines": { "every": true } } } }),
            ),
        ),
        case(
            "semantic-value-literal",
            "value(literal-kind)",
            ast(json!({ "kind": "integer", "value": { "eq": 1 } })),
        ),
        case(
            "semantic-value-flow-unknown",
            "value(flow)",
            semantic(json!({ "value": { "follow": true, "eq": 1 } })),
        ),
        case(
            "semantic-arguments-bind",
            "arguments(signature, bind)",
            ast(
                json!({ "kind": "call", "arguments": { "signature": ["a"], "bind": { "A": "a" } } }),
            ),
        ),
        case(
            "semantic-arguments-filter",
            "arguments(signature, where)",
            ast_query(
                json!({ "rule": { "kind": "call", "arguments": { "signature": ["a"], "bind": { "A": "a" } } }, "constraints": { "A": { "kind": "identifier" } } }),
            ),
        ),
        case(
            "semantic-parse-literal",
            "parse(grammar, rule)",
            ast(
                json!({ "kind": "string", "parse": { "grammar": "annotation", "kind": "identifier" } }),
            ),
        ),
        case(
            "semantic-parse-host",
            "parse(grammar, host(rule))",
            ast(
                json!({ "kind": "string", "parse": { "grammar": "testChars", "host": { "kind": "string" } } }),
            ),
        ),
        case(
            "semantic-parse-capture-export",
            "parse(grammar, capture($A))",
            ast(
                json!({ "kind": "string", "parse": { "grammar": "annotation", "kind": "identifier", "capture": "A" } }),
            ),
        ),
        case(
            "semantic-format-unused-arguments",
            "formatArgs(unused)",
            ast(
                json!({ "pattern": "$S.format($$$A)", "formatArgs": { "style": "brace", "string": "S", "args": "A", "unused": { "kind": "identifier" } } }),
            ),
        ),
        case(
            "semantic-format-missing-fields",
            "formatArgs(missing)",
            ast(
                json!({ "pattern": "$S.format($$$A)", "formatArgs": { "style": "brace", "string": "S", "args": "A", "missing": { "kind": "field" } } }),
            ),
        ),
        case(
            "semantic-layout",
            "layout(lines, neighbour)",
            ast(json!({ "kind": "identifier", "layout": { "span": { "eq": 0 } } })),
        ),
        case(
            "semantic-calls-direct",
            "calls(definition)",
            ast(json!({ "kind": "call", "calls": { "kind": "function_definition" } })),
        ),
        case(
            "semantic-calls-through",
            "calls(definition, through)",
            ast(
                json!({ "kind": "call", "calls": { "through": "any", "kind": "function_definition" } }),
            ),
        ),
        case(
            "semantic-generated-document-map",
            "generated(source-map, rule)",
            ast(
                json!({ "kind": "string", "parse": { "grammar": "annotation", "kind": "identifier" } }),
            ),
        ),
        case(
            "semantic-fact-producer-failure",
            "binding-facts(capacity-failure)",
            semantic(json!({ "resolves": { "binding": {} } })),
        ),
        case(
            "clause-forbid-ast",
            "forbid(ast)",
            ast(pattern("print($A)")),
        ),
        case(
            "clause-forbid-content-regex",
            "forbid(regex(content))",
            document(
                json!({ "id": "contract", "forbid": { "regex": { "pattern": "unsafe", "source": "file.content" } }, "violation": violation() }),
            ),
        ),
        case(
            "clause-forbid-path-regex",
            "forbid(regex(path))",
            document(
                json!({ "id": "contract", "forbid": { "regex": { "pattern": "test", "source": "file.path" } }, "violation": violation() }),
            ),
        ),
        case(
            "clause-select-when-exists",
            "selectWhen(select, exists=true)",
            document(
                json!({ "id": "contract", "forbid": { "select_when": { "select": { "ast": { "language": "python", "rule": { "pattern": "print($A)" } } }, "exists": { "ast": { "language": "python", "rule": { "kind": "import_statement" } } } } }, "violation": violation() }),
            ),
        ),
        case(
            "clause-select-when-absent",
            "selectWhen(select, exists=false)",
            document(
                json!({ "id": "contract", "forbid": { "select_when": { "select": { "ast": { "language": "python", "rule": { "pattern": "print($A)" } } }, "exists": { "ast": { "language": "python", "rule": { "not": { "kind": "import_statement" } } } } } }, "violation": violation() }),
            ),
        ),
        case(
            "clause-require-each",
            "requireEach(subject, evidence)",
            require_each(),
        ),
        case(
            "clause-budget-maximum",
            "budget(query, maximum)",
            budget(false, false),
        ),
        case(
            "clause-budget-distinct",
            "budget(query, distinct=$A)",
            budget(true, false),
        ),
        case(
            "clause-budget-group",
            "budget(query, groupBy=$A)",
            budget(false, true),
        ),
        case(
            "clause-file-condition",
            "file(path, language, content)",
            ast_query(
                json!({ "file": { "path": { "regex": "py$" } }, "rule": { "kind": "identifier" } }),
            ),
        ),
        case(
            "clause-syntax-error-unknown",
            "unsound-rule(error-overlap)",
            ast(json!({ "not": { "has": { "kind": "identifier" } } })),
        ),
        case(
            "output-fix-and-suppression",
            "violation(fix, suppress)",
            ast_violation(
                pattern("print($A)"),
                json!({ "anchor": "capture.A", "code": "SQL", "message": "$A", "fix": "log($A)" }),
            ),
        ),
        case(
            "output-complete-negative",
            "forbid(no terminal matches)",
            ast(json!({ "not": { "pattern": "never" } })),
        ),
        case(
            "output-stable-multiplicity",
            "violation(each alternative and capture)",
            ast_violation(
                json!({ "kind": "call", "any": [{ "has": { "pattern": "$A", "stopBy": "end" } }, { "has": { "pattern": "$A", "stopBy": "neighbor" } }] }),
                json!({ "anchor": "capture.A", "code": "SQL", "message": "$A", "each": "A" }),
            ),
        ),
    ]);
    cases
}

fn evidence(alias: &str) -> Value {
    json!({
        "evidence": {
            "as": alias,
            "query": { "ast": { "language": "python", "rule": { "pattern": "$A" } } },
            "project": { "name": "capture.A", "path": "file.path" }
        }
    })
}

fn require_each() -> Value {
    document(json!({
        "id": "contract",
        "require_each": { "subject": evidence("subject"), "evidence": evidence("proof") },
        "violation": violation()
    }))
}

fn budget(distinct: bool, grouped: bool) -> Value {
    let mut body = json!({ "query": evidence("usage"), "count": { "lte": 1 } });
    if distinct {
        body["count"]["distinct"] = json!(["usage.name"]);
    }
    if grouped {
        body["group_by"] = json!(["usage.path"]);
    }
    document(json!({ "id": "contract", "budget": body, "violation": violation() }))
}

#[test]
fn all_108_contracts_are_real_production_inputs_and_lower_to_sql() {
    let cases = cases();
    assert_eq!(cases.len(), 108);
    let mut failures = Vec::new();
    for case in cases {
        let compiled = match CodeRuleDslRuntime::compile(
            case.input.to_string(),
            "json".to_owned(),
            CAPABILITIES.to_owned(),
        ) {
            Ok(compiled) => compiled,
            Err(error) => {
                failures.push(format!(
                    "{} (`{}`) policy: {}",
                    case.name, case.form, error.reason
                ));
                continue;
            }
        };
        if let Err(error) = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)]) {
            failures.push(format!(
                "{} (`{}`) lowering: {}",
                case.name,
                case.form,
                error.unsupported().join("; ")
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
