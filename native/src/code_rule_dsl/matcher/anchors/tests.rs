use super::*;
use crate::code_rule_dsl::CompiledCodeRuleDsl;
use crate::code_rule_dsl::matcher::GrammarMatcher;
use crate::code_rule_dsl::model::{ClauseKind, Query};
use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
use crate::code_rule_dsl::test_support::{compiled, grammar_matchers};
use ast_grep_core::matcher::MatcherExt;
use ast_grep_core::tree_sitter::StrDoc;
use serde_json::{Value, json};
use std::ops::Range;
use std::sync::Arc;

/// Compiles a rule with no `utils` in scope.
fn compile_rule(
    rule: &Value,
    language: GrammarDialect,
    path: &str,
    gaps: &mut Vec<String>,
) -> napi::Result<AstMatcher> {
    let ids = std::collections::BTreeMap::new();
    let utils = crate::code_rule_dsl::compile::UtilScope {
        table: std::sync::Weak::new(),
        ids: &ids,
        options: None,
        python: None,
        transform: None,
        conflicts: None,
        captures: Default::default(),
        calls: None,
    };
    crate::code_rule_dsl::compile::compile_ast_rule(rule, language, path, gaps, &utils)
}

#[derive(Debug, PartialEq, Eq)]
struct Found {
    range: Range<usize>,
    captures: Vec<(String, Vec<Range<usize>>)>,
}

fn matched<D: Doc>(node: Node<'_, D>, matcher: &AstMatcher) -> Option<Found> {
    let found = matcher.match_node(node)?;
    let mut names = found.get_env().get_matched_variables().collect::<Vec<_>>();
    names.sort_by_key(|name| format!("{name:?}"));
    let captures = names
        .into_iter()
        .filter_map(|name| match &name {
            ast_grep_core::meta_var::MetaVariable::Capture(key, _) => Some((
                format!("{name:?}"),
                found
                    .get_env()
                    .get_match(key)
                    .into_iter()
                    .map(|node| node.range())
                    .collect(),
            )),
            ast_grep_core::meta_var::MetaVariable::MultiCapture(key) => Some((
                format!("{name:?}"),
                found
                    .get_env()
                    .get_multiple_matches(key)
                    .into_iter()
                    .map(|node| node.range())
                    .collect(),
            )),
            _ => None,
        })
        .collect();
    Some(Found {
        range: found.get_node().range(),
        captures,
    })
}

fn actual<D: Doc>(root: &AstGrep<D>, matcher: &AstMatcher) -> Vec<Found> {
    root.root()
        .dfs()
        .filter_map(|node| matched(node, matcher))
        .collect()
}

fn selected<D: Doc<Source = String, Lang = GrammarDialect>>(
    root: &AstGrep<D>,
    matcher: &AstMatcher,
    plan: &AnchorPlan,
    max_nodes: usize,
    interrupted: impl FnMut() -> bool,
) -> (Vec<Found>, usize, usize, bool, bool)
where
    for<'a> D::Node<'a>: AnchorNode<'a>,
{
    let index = FileAnchorIndex::build(root, &[plan], max_nodes, interrupted);
    let selection = index.select(plan);
    let all = matches!(selection, Selection::All);
    let ordinals = match &selection {
        Selection::All => (0..index.len()).collect::<Vec<_>>(),
        Selection::Roots(roots) => roots.to_vec(),
    };
    let matches = ordinals
        .iter()
        .filter_map(|&i| matched(index.node(i), matcher))
        .collect();
    (
        matches,
        ordinals.len(),
        index.visited_nodes(),
        index.complete(),
        all,
    )
}

fn compare(source: &str, language: GrammarDialect, rules: &[Value]) {
    compare_parsed(source, language, rules, false);
}

fn compare_parsed(source: &str, language: GrammarDialect, rules: &[Value], damaged: bool) {
    let matchers: Vec<_> = rules
        .iter()
        .map(|rule| {
            let matcher = compile_rule(rule, language, "test/rule", &mut Vec::new()).unwrap();
            (rule.to_string(), matcher)
        })
        .collect();
    let matchers: Vec<_> = matchers
        .iter()
        .map(|(rule, matcher)| (rule.as_str(), matcher))
        .collect();
    compare_matchers(source, language, &matchers, damaged);
}

/// Each matcher (named for its failures) selects the same nodes from the anchor index of the
/// native tree and of the restored tape as a full walk does.
fn compare_matchers(
    source: &str,
    language: GrammarDialect,
    matchers: &[(&str, &AstMatcher)],
    damaged: bool,
) {
    let native = StrDoc::new(source, language);
    assert_eq!(
        native.tree.root_node().has_error(),
        damaged,
        "invalid fixture {source:?}"
    );
    let grammar = Grammar::new(language).unwrap();
    let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
    let tape = TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap();
    let native = AstGrep::doc(native);
    let tape = AstGrep::doc(tape);
    assert_eq!(native.root().dfs().count(), image.node_count());
    for &(rule, matcher) in matchers {
        let plan = AnchorPlan::compile(matcher);
        let expected = actual(&native, matcher);
        let (native_found, _, native_visited, native_complete, _) =
            selected(&native, matcher, &plan, usize::MAX, || false);
        let (tape_found, _, tape_visited, tape_complete, _) =
            selected(&tape, matcher, &plan, usize::MAX, || false);
        assert!(native_complete && tape_complete, "{language:?} {rule}");
        assert_eq!(
            native_visited,
            image.node_count(),
            "charge anonymous nodes too"
        );
        assert_eq!(tape_visited, native_visited);
        assert_eq!(
            native_found, expected,
            "native selection {language:?} {rule}"
        );
        assert_eq!(tape_found, expected, "tape selection {language:?} {rule}");
        assert_eq!(
            actual(&tape, matcher),
            expected,
            "unchanged matcher {language:?} {rule}"
        );
    }
}

// Error-tolerant evaluation builds the index over damaged trees: postings come from the whole
// tree, including ERROR subtrees and MISSING tokens, and must select exactly as natively.
#[test]
fn damaged_trees_select_identically_on_native_and_restored_tape() {
    let rules = [
        json!({"pattern": "console.log($A)"}),
        json!({"kind": "debugger_statement"}),
        json!({"kind": "ERROR"}),
        json!({"kind": "identifier", "regex": "^m$"}),
        json!({"kind": "call_expression", "inside": {"kind": "ERROR", "stopBy": "end"}}),
        json!({"kind": "statement_block", "has": {"kind": "debugger_statement"}}),
    ];
    for source in [
        "const m = importOriginal(1 load(\"./api\"));\nconsole.log(m);\n",
        "debugger;\nfunction f( {\n  debugger;\n",
        "function f() {\n  debugger;\n  x = 1;\n",
    ] {
        compare_parsed(source, GrammarDialect::TypeScript, &rules, true);
    }
    compare_parsed(
        "x = 1\n) = 2\nprint(x)\n",
        GrammarDialect::Python,
        &[json!({"pattern": "print($A)"}), json!({"kind": "ERROR"})],
        true,
    );
}

#[test]
fn ordered_alternatives_conjunction_pattern_captures_and_unrestricted_branches() {
    let source = "foo(1); bar(2);";
    let rule = json!({"any": [{"pattern": "foo($FIRST)"}, {"pattern": "foo($SECOND)"}]});
    let native = AstGrep::doc(StrDoc::new(source, GrammarDialect::TypeScript));
    let matcher = compile_rule(
        &rule,
        GrammarDialect::TypeScript,
        "test/rule",
        &mut Vec::new(),
    )
    .unwrap();
    let expected = actual(&native, &matcher);
    assert_eq!(
        expected,
        vec![Found {
            range: 0..6,
            captures: vec![(
                "Capture(\"FIRST\", false)".into(),
                vec![Range { start: 4, end: 5 }]
            )],
        }]
    );
    compare(source, GrammarDialect::TypeScript, &[rule]);

    compare(
        "const value = foo(1, 2); foo(3); bar(4); foo(5);",
        GrammarDialect::TypeScript,
        &[
            json!({"any": [{"pattern": "bar($A)"}, {"pattern": "foo($A)"}]}),
            json!({"all": [{"regex": "foo"}, {"pattern": "foo($$$ARGS)"}]}),
            json!({"any": [{"kind": "call_expression", "pattern": "foo($A)"}, {"regex": "^.$"}]}),
            json!({"pattern": "foo($$$ARGS)"}),
            json!({"regex": "^[,]$"}),
        ],
    );
    let dense = (0..40)
        .map(|index| format!("print(value_{index})\n"))
        .collect::<String>();
    let capturing = json!({"pattern": "print($ARG)"});
    let regex = json!({"regex": "^print\\(value_(?:7|31)\\)$"});
    let rules = [
        json!({"all": [regex.clone(), capturing.clone()]}),
        json!({"all": [capturing, regex]}),
    ];
    let native = AstGrep::doc(StrDoc::new(&dense, GrammarDialect::Python));
    let seventh = dense.find("print(value_7)").unwrap();
    let thirty_first = dense.find("print(value_31)").unwrap();
    let expected = vec![
        Found {
            range: seventh..seventh + "print(value_7)".len(),
            captures: vec![(
                "Capture(\"ARG\", false)".into(),
                vec![Range {
                    start: seventh + 6,
                    end: seventh + 6 + "value_7".len(),
                }],
            )],
        },
        Found {
            range: thirty_first..thirty_first + "print(value_31)".len(),
            captures: vec![(
                "Capture(\"ARG\", false)".into(),
                vec![Range {
                    start: thirty_first + 6,
                    end: thirty_first + 6 + "value_31".len(),
                }],
            )],
        },
    ];
    for rule in &rules {
        let matcher =
            compile_rule(rule, GrammarDialect::Python, "test/rule", &mut Vec::new()).unwrap();
        assert_eq!(actual(&native, &matcher), expected);
        let plan = AnchorPlan::compile(&matcher);
        let (found, roots, visited, complete, all) =
            selected(&native, &matcher, &plan, usize::MAX, || false);
        assert_eq!(found, expected);
        assert!(
            complete && !all && roots < visited / 8,
            "capturing conjunction must narrow roots"
        );
    }
    compare(&dense, GrammarDialect::Python, &rules);
}

#[test]
fn exact_negation_approximate_negation_and_relations_preserve_matcher_results() {
    let source = "const a = foo(1); const b = bar(2); const c = foo(3);";
    let inner = json!({"kind": "call_expression", "regex": "foo"});
    let rules = vec![
        json!({"kind": "call_expression", "not": {"regex": "foo"}}),
        json!({"kind": "call_expression", "not": {"pattern": "foo($A)"}}),
        json!({"any": [{"not": {"regex": "foo"}}, {"pattern": "foo($A)"}]}),
        json!({"kind": "lexical_declaration", "has": {"kind": "call_expression"}}),
        json!({"kind": "lexical_declaration", "has": {"kind": "call_expression", "stopBy": "end"}}),
        json!({"kind": "call_expression", "inside": {"kind": "lexical_declaration"}}),
        json!({"kind": "call_expression", "inside": {"kind": "program", "stopBy": "end"}}),
        json!({"kind": "call_expression", "precedes": {"kind": "call_expression"}}),
        json!({"kind": "call_expression", "precedes": {"kind": "call_expression", "stopBy": "end"}}),
        json!({"kind": "call_expression", "follows": {"kind": "call_expression"}}),
        json!({"kind": "call_expression", "follows": {"kind": "call_expression", "stopBy": "end"}}),
        json!({"all": [inner, {"has": {"regex": "^\\($"}}]}),
    ];
    compare(source, GrammarDialect::TypeScript, &rules);
}

#[test]
fn nearest_relations_use_real_immediate_children_and_siblings() {
    for (source, language, statement, call) in [
        (
            "foo(1); bar(2); foo(3);",
            GrammarDialect::TypeScript,
            "expression_statement",
            "call_expression",
        ),
        (
            "foo(1)\nbar(2)\nfoo(3)\n",
            GrammarDialect::Python,
            "expression_statement",
            "call",
        ),
    ] {
        let native = AstGrep::doc(StrDoc::new(source, language));
        let first = source.find("foo(1)").unwrap();
        let middle = source.find("bar(2)").unwrap();
        let last = source.find("foo(3)").unwrap();
        let suffix = usize::from(language == GrammarDialect::TypeScript);
        let statement_ranges = vec![
            first..first + 6 + suffix,
            middle..middle + 6 + suffix,
            last..last + 6 + suffix,
        ];
        let call_ranges = vec![first..first + 6, middle..middle + 6, last..last + 6];
        let rules = [
            json!({"kind": statement, "has": {"kind": call}}),
            json!({"kind": call, "inside": {"kind": statement}}),
            json!({"kind": statement, "has": {"kind": "identifier"}}),
            json!({"kind": call, "inside": {"kind": "identifier"}}),
            json!({"kind": statement, "precedes": {"kind": statement, "regex": "^bar"}}),
            json!({"kind": statement, "follows": {"kind": statement, "regex": "^bar"}}),
            json!({"kind": statement, "precedes": {"kind": statement, "regex": "^foo\\(3"}}),
            json!({"kind": statement, "follows": {"kind": statement, "regex": "^foo\\(1"}}),
            json!({"kind": statement, "precedes": {"kind": statement, "regex": "^foo\\(1"}}),
            json!({"kind": statement, "follows": {"kind": statement, "regex": "^foo\\(3"}}),
        ];
        let expected = [
            statement_ranges,
            call_ranges,
            vec![],
            vec![],
            vec![Range {
                start: first,
                end: first + 6 + suffix,
            }],
            vec![Range {
                start: last,
                end: last + 6 + suffix,
            }],
            vec![Range {
                start: middle,
                end: middle + 6 + suffix,
            }],
            vec![Range {
                start: middle,
                end: middle + 6 + suffix,
            }],
            vec![],
            vec![],
        ];
        for (rule, ranges) in rules.iter().zip(expected) {
            let matcher = compile_rule(rule, language, "test/rule", &mut Vec::new()).unwrap();
            assert_eq!(
                actual(&native, &matcher)
                    .into_iter()
                    .map(|found| found.range)
                    .collect::<Vec<_>>(),
                ranges,
                "native {language:?} {rule}"
            );
        }
        compare(source, language, &rules);
    }
}

#[test]
fn string_comments_unicode_prefixes_long_residue_and_python_variadics() {
    let mut long = format!(
        "const é = 'αβγ{}needle';\n// αβγ note\nconst other = 'αβγ{}tail';\n",
        "x".repeat(300),
        "x".repeat(300)
    );
    for i in 0..12 {
        long.push_str(&format!(
            "const extra{i} = 'αβγ{}item{i}';\n",
            "x".repeat(300)
        ));
    }
    compare(
        &long,
        GrammarDialect::TypeScript,
        &[
            json!({"kind": "string", "regex": "^'αβγ.*needle'$"}),
            json!({"regex": "^// αβγ note$"}),
            json!({"pattern": "const é = $VALUE"}),
        ],
    );
    let root = AstGrep::doc(StrDoc::new(&long, GrammarDialect::TypeScript));
    let rule = json!({"kind": "string", "regex": "^'αβγ.*needle'$"});
    let matcher = compile_rule(
        &rule,
        GrammarDialect::TypeScript,
        "test/rule",
        &mut Vec::new(),
    )
    .unwrap();
    let plan = AnchorPlan::compile(&matcher);
    let (found, roots, visited, complete, all) =
        selected(&root, &matcher, &plan, usize::MAX, || false);
    assert_eq!(found.len(), 1);
    assert!(
        complete && !all && roots < visited / 8,
        "large Unicode values must narrow without losing residue"
    );
    compare(
        "foo(1, 2)\n# note\nfoo(3)\n",
        GrammarDialect::Python,
        &[
            json!({"pattern": "foo($$$ARGS)"}),
            json!({"pattern": "foo(3)"}),
            json!({"regex": "^,$"}),
            json!({"regex": "^# note$"}),
        ],
    );
}

/// Every AST query of the shipped catalog (the TSLint pack core ships) and of the Python pack,
/// compiled with its utilities, constraints and its document's definitions, selects the same nodes on the native tree and on
/// the restored tape.
#[test]
fn all_catalog_ast_rules_select_identically_on_native_and_restored_tape() {
    fn collect(value: &Value, queries: &mut Vec<Value>) {
        match value {
            Value::Object(object) => {
                for (key, child) in object {
                    if key == "ast" {
                        queries.push(child.clone());
                    } else {
                        collect(child, queries);
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    collect(item, queries);
                }
            }
            _ => {}
        }
    }
    /// The query's matchers: the first AST clause's.
    fn ast_matchers(compiled: &CompiledCodeRuleDsl) -> &[GrammarMatcher] {
        compiled
            .inner
            .clauses
            .iter()
            .find_map(|clause| match &clause.kind {
                ClauseKind::Forbid(Query::Ast { matchers, .. }) => Some(matchers.as_slice()),
                _ => None,
            })
            .expect("an AST clause")
    }

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut pending = vec![
        root.join("packs/tslint/rules"),
        root.join("packs/complexity/rules"),
        root.join("packs/python/rules"),
    ];
    // Each query with its document's `define` clauses, which declare the options and utilities
    // it may use.
    let mut queries: Vec<(Value, Vec<Value>)> = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "json") {
                let document: Value =
                    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
                let clauses = document["clauses"].as_array().expect("clauses");
                let defines: Vec<_> = clauses
                    .iter()
                    .filter(|clause| clause.get("define").is_some())
                    .cloned()
                    .collect();
                let mut found = Vec::new();
                collect(&document["clauses"], &mut found);
                queries.extend(found.into_iter().map(|query| (query, defines.clone())));
            }
        }
    }
    assert_eq!(queries.len(), 123, "pinned catalog query inventory");
    // Every catalog grammar needs a fixture here; an unknown language fails rather than being skipped.
    let fixtures = [
        (
            "python",
            GrammarDialect::Python,
            "from data_layer import AbstractData\nclass Example:\n    def run(self, txn):\n        value = txn.execute('select * from users')\n        assert value != None\n        return sorted(value)\n",
        ),
        (
            "typescript",
            GrammarDialect::TypeScript,
            "function run(db: any, id: string): string {\n  const rows = db.query(\"select * from users where id = \" + id);\n  return rows as any;\n}\n",
        ),
        (
            "javascript",
            GrammarDialect::JavaScript,
            "function route(a, b, c) {\n  if (a) return 1;\n  if (b) return 2;\n  if (c) return 3;\n  for (const x of a) { if (x) return 4; }\n  while (b) { if (c) break; }\n  return a && b ? 5 : 6;\n}\n",
        ),
    ];
    for (query, _) in &queries {
        let language = query["language"].as_str().expect("AST query language");
        assert!(
            fixtures.iter().any(|(name, ..)| *name == language),
            "catalog AST language {language} has no restored-tape fixture"
        );
    }
    for (name, language, source) in fixtures {
        let compiled: Vec<_> = queries
            .iter()
            .filter(|(query, _)| query["language"] == name)
            .map(|(query, defines)| {
                let mut clauses = defines.clone();
                clauses.push(json!({
                    "id": "ast-match",
                    "forbid": { "ast": query },
                    "violation": { "anchor": "witness.span", "code": "AST_MATCH", "message": "match" },
                }));
                let document = json!({
                    "format": "rerail.policy/v1",
                    "id": "catalog-query",
                    "revision": 1,
                    "mode": "block",
                    "clauses": clauses,
                });
                let compiled = crate::code_rule_dsl::CodeRuleDslRuntime::compile(
                    document.to_string(),
                    "json".to_owned(),
                    crate::code_rule_dsl::test_support::CAPABILITIES.to_owned(),
                )
                .unwrap_or_else(|error| panic!("{query}: {}", error.reason));
                (query.to_string(), compiled)
            })
            .collect();
        let matchers: Vec<_> = compiled
            .iter()
            .flat_map(|(query, compiled)| {
                ast_matchers(compiled)
                    .iter()
                    .filter(|grammar| grammar.language == language)
                    .map(move |grammar| (query.as_str(), &grammar.matcher))
            })
            .collect();
        let root = AstGrep::doc(StrDoc::new(source, language));
        assert!(
            matchers
                .iter()
                .any(|(_, matcher)| !actual(&root, matcher).is_empty()),
            "the {name} catalog fixture must exercise at least one positive rule"
        );
        compare_matchers(source, language, &matchers, false);
    }
}

#[test]
fn incomplete_index_is_not_a_clean_absence() {
    let source = "const a = foo(1); const b = bar(2);";
    let native = StrDoc::new(source, GrammarDialect::TypeScript);
    let grammar = Grammar::new(GrammarDialect::TypeScript).unwrap();
    let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
    let tape = TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap();
    let matcher = compile_rule(
        &json!({"pattern": "bar(2)"}),
        GrammarDialect::TypeScript,
        "test/rule",
        &mut Vec::new(),
    )
    .unwrap();
    let plan = AnchorPlan::compile(&matcher);
    assert!(image.node_count() > 2);
    let native = AstGrep::doc(native);
    let tape = AstGrep::doc(tape);
    for budget in [0, 1, 2] {
        let (_, _, visited, complete, all) = selected(&native, &matcher, &plan, budget, || false);
        assert!(
            !complete && all && visited == budget + 1,
            "native budget {budget}"
        );
        let (_, _, visited, complete, all) = selected(&tape, &matcher, &plan, budget, || false);
        assert!(
            !complete && all && visited == budget + 1,
            "tape budget {budget}"
        );
    }
    let (_, _, _, complete, all) = selected(&native, &matcher, &plan, usize::MAX, || true);
    assert!(!complete && all, "interruption cannot imply absence");
    let (_, _, _, complete, all) = selected(&tape, &matcher, &plan, usize::MAX, || true);
    assert!(!complete && all);
}

#[test]
fn traversal_polls_closing_nodes_before_another_preorder_batch() {
    use std::cell::Cell;
    let source = "foo(1);\n".repeat(120);
    let native = StrDoc::new(&source, GrammarDialect::TypeScript);
    let grammar = Grammar::new(GrammarDialect::TypeScript).unwrap();
    let image = TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
    let tape = TapeDoc::restore(Arc::new(source), grammar, &image, &|| false).unwrap();
    let matcher = compile_rule(
        &json!({"kind": "call_expression"}),
        GrammarDialect::TypeScript,
        "test/rule",
        &mut Vec::new(),
    )
    .unwrap();
    let plan = AnchorPlan::compile(&matcher);
    let native = AstGrep::doc(native);
    let tape = AstGrep::doc(tape);
    for visited in [
        {
            let calls = Cell::new(0);
            let index = FileAnchorIndex::build(&native, &[&plan], usize::MAX, || {
                calls.set(calls.get() + 1);
                calls.get() == 3
            });
            assert!(!index.complete());
            index.visited_nodes()
        },
        {
            let calls = Cell::new(0);
            let index = FileAnchorIndex::build(&tape, &[&plan], usize::MAX, || {
                calls.set(calls.get() + 1);
                calls.get() == 3
            });
            assert!(!index.complete());
            index.visited_nodes()
        },
    ] {
        assert!(
            (256..512).contains(&visited),
            "closure polling should interrupt before 512 visits: {visited}"
        );
    }
}

#[test]
fn deep_assignment_chain_has_exact_bounded_visits_and_incomplete_fallback() {
    let source = format!("{}b;", "a=".repeat(640));
    let native = StrDoc::new(&source, GrammarDialect::TypeScript);
    assert!(!native.tree.root_node().has_error());
    let grammar = Grammar::new(GrammarDialect::TypeScript).unwrap();
    let image = TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
    let tape = TapeDoc::restore(Arc::new(source), grammar, &image, &|| false).unwrap();
    let rule = json!({"kind": "class_declaration"});
    let matcher = compile_rule(
        &rule,
        GrammarDialect::TypeScript,
        "test/rule",
        &mut Vec::new(),
    )
    .unwrap();
    let plan = AnchorPlan::compile(&matcher);
    let native = AstGrep::doc(native);
    let tape = AstGrep::doc(tape);
    assert!(image.node_count() > 1024);
    println!(
        "DEEP_CHAIN nodes={} max_visits=[0,1,7,513,{}]",
        image.node_count(),
        image.node_count() - 1
    );
    for max_nodes in [0, 1, 7, 513, image.node_count() - 1] {
        let (found, roots, visited, complete, all) =
            selected(&native, &matcher, &plan, max_nodes, || false);
        assert!(found.is_empty() && !complete && all);
        assert_eq!(visited, max_nodes + 1);
        assert_eq!(roots, max_nodes);
        let (found, roots, visited, complete, all) =
            selected(&tape, &matcher, &plan, max_nodes, || false);
        assert!(found.is_empty() && !complete && all);
        assert_eq!(visited, max_nodes + 1);
        assert_eq!(roots, max_nodes);
    }
    let (found, _, visited, complete, all) =
        selected(&native, &matcher, &plan, usize::MAX, || false);
    assert!(found.is_empty() && complete && !all);
    assert_eq!(visited, image.node_count());
    let (found, _, visited, complete, all) = selected(&tape, &matcher, &plan, usize::MAX, || false);
    assert!(found.is_empty() && complete && !all);
    assert_eq!(visited, image.node_count());
}

fn sibling_matcher(rule: Value, language: GrammarDialect) -> AstMatcher {
    compile_rule(&rule, language, "/rule", &mut Vec::new())
        .unwrap_or_else(|error| panic!("{}", error.reason))
}

const ZERO_WIDTH_SOURCES: [&str; 8] = [
    "",
    "a;\nb;\n",
    "f(a,\n",
    "if (a) { b(\n",
    "const x = [1, 2\ny();\n",
    "a();\nb()\nc(",
    "class A { m() {\n",
    "x = (1 + \n",
];

fn exact_sibling_projection<D: Doc<Source = String, Lang = GrammarDialect>>(
    root: &AstGrep<D>,
    source: &str,
    rules: &[Value],
) -> Vec<Vec<bool>>
where
    for<'a> D::Node<'a>: AnchorNode<'a>,
{
    let mut results = Vec::new();
    for rule in rules {
        for negated in [false, true] {
            let rule = if negated {
                json!({ "regex": "", "not": rule.clone() })
            } else {
                rule.clone()
            };
            let matcher = sibling_matcher(rule.clone(), *root.root().get_doc().get_lang());
            let plan = AnchorPlan::compile(&matcher);
            assert!(plan.exact, "{rule}");
            let index = FileAnchorIndex::build(root, &[&plan], usize::MAX, || false);
            let selection = index.select(&plan);
            let mut matched = Vec::new();
            for ordinal in 0..index.len() {
                let found = matcher.match_node(index.node(ordinal)).is_some();
                assert_eq!(
                    selection.contains(ordinal),
                    found,
                    "{source:?} {rule} node {ordinal} {:?}",
                    index.node(ordinal).range()
                );
                matched.push(found);
            }
            results.push(matched);
        }
    }
    results
}

#[test]
fn sibling_projections_are_exact_around_zero_width_nodes_and_the_root() {
    // `regex: ""` admits every node, zero-width MISSING tokens included, so each sibling
    // walk is exercised from and onto them. A sound plan selects every matching node; an
    // exact one (used under `not`) selects nothing else. The restored tape must project,
    // and match, exactly as the native tree does.
    let mut rules = Vec::new();
    for relation in ["precedes", "follows"] {
        for stop in ["neighbor", "end"] {
            for target in ["", "^$", "^[;,)}\\]]$", "^[a-z]"] {
                rules.push(json!({ "regex": "", relation: { "stopBy": stop, "regex": target } }));
            }
        }
    }
    for source in ZERO_WIDTH_SOURCES {
        let native = StrDoc::new(source, GrammarDialect::TypeScript);
        let grammar = Grammar::new(GrammarDialect::TypeScript).unwrap();
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
        let tape =
            TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap();
        let native = exact_sibling_projection(&AstGrep::doc(native), source, &rules);
        let tape = exact_sibling_projection(&AstGrep::doc(tape), source, &rules);
        assert_eq!(native, tape, "{source:?}");
    }
}

// A Blade tape's embedded program is the root's last child, spanning the template bytes it
// stands for, so it overlaps the template's nodes: for `{{ $x }}` the program and the echo's
// `php_statement` both span the echo. ast-grep's walks are byte-positioned there too (`next`
// skips the program, `prev_all` from the program starts at the echo), and an exact plan, and
// its negation, must select what the matcher matches.
#[test]
fn sibling_projections_are_exact_among_blade_overlapping_siblings() {
    let mut rules = Vec::new();
    for relation in ["precedes", "follows"] {
        for stop in ["neighbor", "end"] {
            for target in ["", "^\\{\\{", "^\\$", "^[a-z]"] {
                rules.push(json!({ "regex": "", relation: { "stopBy": stop, "regex": target } }));
            }
        }
    }
    let sources = [
        "{{ $x }}",
        "<p>{{ $x }}</p>\n{!! $y !!}\n",
        "@if($a)\n{{ $b }}\n@endif\n",
    ];
    for source in sources {
        let native = StrDoc::new(source, GrammarDialect::Blade);
        let grammar = Grammar::new(GrammarDialect::Blade).unwrap();
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
        let tape = AstGrep::doc(
            TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap(),
        );
        let root = tape.root();
        let program = root.children().last().expect("the embedded program");
        assert_eq!(program.kind(), "program", "{source:?}");
        let start = program.range().start;
        assert!(
            root.children()
                .any(|child| child.node_id() != program.node_id() && child.range().end > start),
            "{source:?}: the program overlaps a template node"
        );
        if source == "{{ $x }}" {
            // No zero-width node: only the overlap puts the walk off the preorder ordinals.
            assert!(root.dfs().all(|node| !node.range().is_empty()));
        }
        exact_sibling_projection(&tape, source, &rules);
    }
}

// ast-grep's sibling walks are byte-positioned on the native tree: `next` skips zero-width
// siblings at the node's end, and `next_all`/`prev_all` start from the first child of the
// parent that ends after the node's start. The tape reproduces each walk.
#[test]
fn tape_sibling_walks_match_native_around_zero_width_nodes() {
    fn walks<D: Doc>(root: &AstGrep<D>) -> Vec<[Vec<Range<usize>>; 4]> {
        let ranges = |nodes: Vec<Node<'_, D>>| {
            nodes
                .into_iter()
                .map(|node| node.range())
                .collect::<Vec<_>>()
        };
        root.root()
            .dfs()
            .map(|node| {
                [
                    ranges(node.next().into_iter().collect()),
                    ranges(node.prev().into_iter().collect()),
                    ranges(node.next_all().collect()),
                    ranges(node.prev_all().collect()),
                ]
            })
            .collect()
    }
    let extra = [
        "function f() {\n  debugger;\n  x = 1;\n",
        "a();\nb();\nif (c) { d(); e(\n",
        "function f() { a(); b()\n",
        "x = [1, 2\ny();\nz();\n",
        "  \n// lead\nf(",
    ];
    for source in ZERO_WIDTH_SOURCES.into_iter().chain(extra) {
        let native = StrDoc::new(source, GrammarDialect::TypeScript);
        let grammar = Grammar::new(GrammarDialect::TypeScript).unwrap();
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
        let tape =
            TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap();
        assert_eq!(
            walks(&AstGrep::doc(native)),
            walks(&AstGrep::doc(tape)),
            "{source:?}"
        );
    }
}

/// One zero-width node puts every sibling relation of the file on ast-grep's own walk. An `end`
/// walk whose only match lies behind the candidate, or at the far end of the run, reads the rest of
/// the run from each candidate. The selection reads each parent's run once instead, and still
/// projects exactly what the matcher matches.
#[test]
fn zero_width_end_sibling_selection_is_linear_in_the_run() {
    use crate::code_rule_dsl::test_support::{Walk, reset_walks, walks};
    let statements = 2000;
    // The unclosed body's statements run from its `{` to a zero-width MISSING `}`.
    let source = format!("function f() {{\n{}", "a();\n".repeat(statements));
    let native = AstGrep::doc(StrDoc::new(&source, GrammarDialect::TypeScript));
    let mut rules = Vec::new();
    for relation in ["precedes", "follows"] {
        for target in ["^\\{$", "^$"] {
            rules.push(json!({
                "kind": "expression_statement",
                relation: { "stopBy": "end", "regex": target },
            }));
        }
    }
    for rule in &rules {
        let matcher = sibling_matcher(rule.clone(), GrammarDialect::TypeScript);
        let plan = AnchorPlan::compile(&matcher);
        let index = FileAnchorIndex::build(&native, &[&plan], usize::MAX, || false);
        assert!(index.walked_siblings, "the MISSING `}}` is zero-width");
        reset_walks();
        index.select(&plan);
        let walked = walks(Walk::AnchorSibling);
        assert!(
            walked <= 4 * statements,
            "{rule}: {walked} sibling steps for a run of {statements} statements"
        );
    }
    let projected = exact_sibling_projection(&native, &source, &rules);
    // Every statement precedes the MISSING `}` and follows the `{`; none does the opposite.
    let count = |matched: &Vec<bool>| matched.iter().filter(|&&found| found).count();
    assert_eq!(
        [0, 2, 4, 6].map(|rule| count(&projected[rule])),
        [0, statements, statements, 0]
    );
}

fn assert_stop_rule_supersets<D: Doc<Source = String, Lang = GrammarDialect>>(
    root: &AstGrep<D>,
    rules: &[Value],
) where
    for<'a> D::Node<'a>: AnchorNode<'a>,
{
    for rule in rules {
        let matcher = sibling_matcher(rule.clone(), *root.root().get_doc().get_lang());
        let plan = AnchorPlan::compile(&matcher);
        assert!(!plan.exact, "{rule}");
        let index = FileAnchorIndex::build(root, &[&plan], usize::MAX, || false);
        let selection = index.select(&plan);
        for ordinal in 0..index.len() {
            if matcher.match_node(index.node(ordinal)).is_some() {
                assert!(selection.contains(ordinal), "{rule} node {ordinal}");
            }
        }
    }
}

#[test]
fn stop_rule_relations_plan_as_unbounded_supersets() {
    // A stop rule only shortens the walk: the plan keeps the unbounded relation, so it must
    // select every match and must not be exact (a `not` over it would subtract true matches).
    let source = "function f() { a(); const k = () => { return b(); }; }\nfunction g() { return c(); }\n\
                  class A { m() { return this; } }\nx();\nstop;\ny();\n";
    let stop = json!({ "any": [{ "kind": "arrow_function" }, { "kind": "method_definition" }, { "pattern": "stop;" }] });
    let mut rules = Vec::new();
    for relation in ["has", "inside", "precedes", "follows"] {
        for target in [
            json!({ "kind": "return_statement" }),
            json!({ "kind": "class_body" }),
            json!({ "pattern": "y();" }),
            json!({ "pattern": "x();" }),
        ] {
            let mut relation_rule = target.clone();
            relation_rule["stopBy"] = stop.clone();
            rules.push(json!({ "regex": "", relation: relation_rule.clone() }));
            rules.push(json!({ "regex": "", "not": { relation: relation_rule } }));
        }
    }
    let native = StrDoc::new(source, GrammarDialect::TypeScript);
    let grammar = Grammar::new(GrammarDialect::TypeScript).unwrap();
    let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
    let tape = TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap();
    assert_stop_rule_supersets(&AstGrep::doc(native), &rules);
    assert_stop_rule_supersets(&AstGrep::doc(tape), &rules);
}

#[test]
fn anchor_plans_inline_references_and_stay_unrestricted_where_they_recurse() {
    let kinds = |ast: Value| {
        grammar_matchers(&compiled(ast))[0]
            .anchors
            .root_kinds()
            .map(|kinds| kinds.len())
    };
    assert_eq!(
        kinds(
            json!({ "rule": { "matches": "number" }, "utils": { "number": { "kind": "number" } } })
        ),
        Some(1)
    );
    assert_eq!(
        kinds(json!({
            "rule": { "matches": "walk" },
            "utils": { "walk": { "any": [{ "kind": "number" }, { "kind": "parenthesized_expression", "has": { "matches": "walk" } }] } },
        })),
        Some(2)
    );
    assert_eq!(
        kinds(
            json!({ "rule": { "pattern": "$F()" }, "constraints": { "F": { "kind": "identifier" } } })
        ),
        Some(1)
    );
}

/// A rule factored into `utils` selects the same roots, from the same value postings, as the rule
/// written inline: utility plans keep their kind, text, pattern and exactness terms (a `not` over
/// an exact utility subtracts), and an enclosing conjunction's kinds still bound their text needs.
#[test]
fn utility_plans_select_as_precisely_as_the_rule_written_inline() {
    let mut source: String = (0..200).map(|n| format!("bar{n}(baz{n});\n")).collect();
    source.push_str("foo(x);\n");
    let native = AstGrep::doc(StrDoc::new(source.as_str(), GrammarDialect::TypeScript));
    let selection = |ast: Value| {
        let compiled = compiled(ast);
        let grammar = &grammar_matchers(&compiled)[0];
        let (found, roots, _, complete, _) = selected(
            &native,
            &grammar.matcher,
            &grammar.anchors,
            usize::MAX,
            || false,
        );
        assert!(complete);
        assert_eq!(found, actual(&native, &grammar.matcher), "sound selection");
        let index = FileAnchorIndex::build(&native, &[&grammar.anchors], usize::MAX, || false);
        let mut postings: Vec<_> = index
            .values
            .iter()
            .map(|(kind, values)| (*kind, values.complete, values.terms.len()))
            .collect();
        postings.sort_unstable();
        (roots, postings)
    };
    let foo = json!({ "kind": "identifier", "regex": "^foo$" });
    let ba = json!({ "regex": "^ba" });
    for (utility, inline, utils, expected) in [
        (
            json!({ "matches": "u" }),
            foo.clone(),
            json!({ "u": foo.clone() }),
            1,
        ),
        (
            json!({ "matches": "u" }),
            json!({ "pattern": "foo($A)" }),
            json!({ "u": { "pattern": "foo($A)" } }),
            1,
        ),
        (
            json!({ "kind": "identifier", "not": { "matches": "u" } }),
            json!({ "kind": "identifier", "not": ba.clone() }),
            json!({ "u": ba }),
            2,
        ),
        // Both diamond paths reach one shared plan for `v`.
        (
            json!({ "matches": "w" }),
            json!({ "all": [foo.clone(), foo.clone()] }),
            json!({ "w": { "all": [{ "matches": "v" }, { "matches": "v" }] }, "v": foo }),
            1,
        ),
    ] {
        let inline_selection = selection(json!({ "rule": inline }));
        assert_eq!(inline_selection.0, expected, "inline {inline}");
        assert_eq!(
            selection(json!({ "rule": utility, "utils": utils })),
            inline_selection,
            "utility {utility} with {utils}"
        );
    }
}

/// A utility's plan is inlined once per plan and reused by every diamond path that reaches it.
#[test]
fn utility_diamonds_inline_anchor_plans_in_linear_visits() {
    use crate::code_rule_dsl::test_support::{
        DIAMOND_DEPTH, Walk, compiled_parameterized_utility_diamonds, compiled_utility_diamonds,
        linear_walk, reset_walks, walks,
    };
    let bound = linear_walk(DIAMOND_DEPTH);
    for compiled in [
        compiled_utility_diamonds(DIAMOND_DEPTH),
        compiled_parameterized_utility_diamonds(DIAMOND_DEPTH),
    ] {
        reset_walks();
        AnchorPlan::compile(&grammar_matchers(&compiled)[0].matcher);
        assert!(
            walks(Walk::AnchorInlining) <= bound,
            "anchor planning inlined {} utilities (bound {bound})",
            walks(Walk::AnchorInlining)
        );
    }
}

/// Selecting with a diamond's shared plans evaluates, and collects index needs for, each utility a
/// constant number of times however many paths reach it, and selects as the rule does.
#[test]
fn utility_diamonds_select_in_linear_visits() {
    use crate::code_rule_dsl::test_support::{
        DIAMOND_DEPTH, Walk, compiled_parameterized_utility_diamonds, compiled_utility_diamonds,
        linear_walk, reset_walks, walks,
    };
    let bound = linear_walk(DIAMOND_DEPTH);
    let native = AstGrep::doc(StrDoc::new(
        "foo(x);\nbar(y, z);\n",
        GrammarDialect::TypeScript,
    ));
    for compiled in [
        compiled_utility_diamonds(DIAMOND_DEPTH),
        compiled_parameterized_utility_diamonds(DIAMOND_DEPTH),
    ] {
        let grammar = &grammar_matchers(&compiled)[0];
        reset_walks();
        let (found, _, _, complete, _) = selected(
            &native,
            &grammar.matcher,
            &grammar.anchors,
            usize::MAX,
            || false,
        );
        assert!(complete);
        assert_eq!(found, actual(&native, &grammar.matcher));
        for walk in [Walk::AnchorNeeds, Walk::AnchorSelection] {
            assert!(
                (1..=bound).contains(&walks(walk)),
                "{walk:?} visited {} shared utility plans (bound {bound})",
                walks(walk)
            );
        }
    }
}

/// A shared utility plan reached from scopes that are not nested (identifiers, then property
/// identifiers) tests only the roots of those scopes, as the rule written inline does: a later
/// scope evaluates the roots the earlier evaluations missed, never every node of the file.
#[test]
fn utility_plans_reached_from_disjoint_scopes_test_only_those_scopes() {
    use crate::code_rule_dsl::test_support::{Walk, reset_walks, walks};
    let mut source: String = (0..400)
        .map(|n| format!("function f{n}(a{n}) {{ return a{n}.p{n}.q + [n{n}, {n}]; }}\n"))
        .collect();
    source.push_str("zzz.zzz;\n");
    let native = AstGrep::doc(StrDoc::new(source.as_str(), GrammarDialect::TypeScript));
    let visits = |ast: Value| {
        let compiled = compiled(ast);
        let grammar = &grammar_matchers(&compiled)[0];
        reset_walks();
        let (found, roots, _, complete, _) = selected(
            &native,
            &grammar.matcher,
            &grammar.anchors,
            usize::MAX,
            || false,
        );
        assert!(complete);
        assert_eq!(found, actual(&native, &grammar.matcher), "sound selection");
        (roots, walks(Walk::AnchorTextNode))
    };
    let zzz = json!({ "regex": "zzz" });
    let identifiers =
        json!({ "any": [{ "kind": "identifier" }, { "kind": "property_identifier" }] });
    for (utility, inline) in [
        (
            json!({ "any": [
                { "kind": "identifier", "matches": "u" },
                { "kind": "property_identifier", "matches": "u" }
            ] }),
            json!({ "any": [
                { "kind": "identifier", "regex": "zzz" },
                { "kind": "property_identifier", "regex": "zzz" }
            ] }),
        ),
        // The second scope overlaps the first: the roots both hold are reused, not tested again.
        (
            json!({ "any": [
                { "kind": "identifier", "matches": "u" },
                { "all": [identifiers.clone(), { "matches": "u" }] }
            ] }),
            json!({ "any": [
                { "kind": "identifier", "regex": "zzz" },
                { "all": [identifiers.clone(), zzz.clone()] }
            ] }),
        ),
    ] {
        let (inline_roots, inline_visits) = visits(json!({ "rule": inline }));
        let (roots, utility_visits) =
            visits(json!({ "rule": utility, "utils": { "u": zzz.clone() } }));
        assert_eq!(roots, inline_roots, "utility {utility}");
        assert!(
            utility_visits <= inline_visits,
            "utility {utility} tested {utility_visits} roots; inline {inline_visits}"
        );
    }
}

/// A relation selects its inner rule among the nodes it relates to the scope's roots (their
/// siblings, parents or children), and a root-kind test of a narrow scope reads the scope, not
/// the kinds' postings: the budget units a selection spends do not grow with code no root is near.
/// The shapes are the TypeScript pack's (no-this-alias, no-self-compare, eqeqeq), which spent most
/// of a large file's work budget reading every node of it once per relation.
#[test]
fn relation_selections_spend_units_on_the_related_nodes_only() {
    use crate::code_rule_dsl::budget::MatchBudget;
    use std::sync::atomic::AtomicBool;
    let rules = [
        json!({ "kind": "variable_declarator", "has": {
            "stopBy": "neighbor", "kind": "identifier", "precedes": { "stopBy": "end", "regex": "^=$" },
        } }),
        json!({ "kind": "binary_expression", "has": {
            "stopBy": "neighbor", "regex": "^===$", "follows": { "stopBy": "end", "not": { "kind": "comment" } },
        } }),
        json!({ "regex": "^==$", "inside": { "stopBy": "neighbor", "kind": "binary_expression" } }),
        json!({ "kind": "variable_declarator", "has": {
            "stopBy": "neighbor", "any": [{ "kind": "identifier" }, { "kind": "number" }],
        } }),
    ];
    let source = |filler: usize| {
        let mut source = (0..20)
            .map(|n| format!("const s{n} = this;\nif (a{n} === b{n} || c{n} == 1) {{}}\n"))
            .collect::<String>();
        // Identifiers, `=` tokens and comments, but no declarator or binary expression.
        for n in 0..filler {
            source.push_str(&format!("x{n}.y = f(a{n}, b{n}.c, [d{n}]); // n\n"));
        }
        source
    };
    for rule in &rules {
        let matcher = compile_rule(
            rule,
            GrammarDialect::TypeScript,
            "test/rule",
            &mut Vec::new(),
        )
        .unwrap();
        let plan = AnchorPlan::compile(&matcher);
        let units = |filler: usize| {
            let source = source(filler);
            let native = AstGrep::doc(StrDoc::new(&source, GrammarDialect::TypeScript));
            let index = FileAnchorIndex::build(&native, &[&plan], usize::MAX, || false);
            let budget = MatchBudget::work(&Arc::new(AtomicBool::new(false)), u64::MAX);
            let selection = index.select(&plan);
            let spent = budget.spent();
            drop(budget);
            let found = match &selection {
                Selection::All => panic!("{rule}: a complete index selects roots"),
                Selection::Roots(roots) => roots
                    .iter()
                    .filter_map(|&ordinal| matched(index.node(ordinal), &matcher))
                    .collect::<Vec<_>>(),
            };
            assert_eq!(found, actual(&native, &matcher), "{rule}: sound selection");
            assert_eq!(found.len(), 20, "{rule}");
            (spent, index.len())
        };
        let (small, small_nodes) = units(500);
        let (large, large_nodes) = units(8_000);
        assert!(large_nodes > 10 * small_nodes, "{rule}");
        assert!(
            large <= small + 2,
            "{rule}: {small} units with {small_nodes} nodes, {large} with {large_nodes}"
        );
    }
}
