use super::{leaves, witness};
use crate::code_rule_dsl::{
    matcher::{AstMatcher, GrammarMatcher, begin_clause, end_clause, node_at},
    sql::{self, exec::FileContext, profiling, tests::differential::tape, timing::Started},
};
use ast_grep_core::{AstGrep, matcher::Matcher, meta_var::MetaVarEnv};
use serde_json::{Value, json};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
};

type Row = (usize, u32, String, u32, u32, u32);

/// Native oracle derives candidate values from source trees, never from generated SQL results.
fn expected(
    grammar: &GrammarMatcher,
    search: &witness::Search,
    sources: &[(String, String)],
) -> Vec<Row> {
    let mut origin = grammar.clone();
    origin.matcher = search.origins.clone();
    let mut filter = grammar.clone();
    filter.matcher = search.ancestor_filter.clone();
    let mut stop = grammar.clone();
    stop.matcher = search.ancestor_stop.clone();
    let mut leaf = grammar.clone();
    leaf.matcher = search.leaf_filter.clone();
    let mut output = Vec::new();
    for (file, (_, source)) in sources.iter().enumerate() {
        let doc = tape(grammar.language, source);
        let len = doc.transducer_len();
        let tree = AstGrep::doc(doc);
        let mut work = usize::MAX;
        let mut values: BTreeMap<u32, BTreeMap<String, u32>> = BTreeMap::new();
        for id in 0..len as u32 {
            let n = node_at(&tree, id);
            if origin.match_root(n.clone(), &mut work).unwrap().is_none() {
                continue;
            }
            let mut ancestors = Vec::new();
            for (distance, a) in n.ancestors().enumerate() {
                if filter.match_root(a.clone(), &mut work).unwrap().is_some() {
                    let aid = a.node_id() as u32;
                    ancestors.push((aid, distance as u32 + 1));
                    values.entry(aid).or_insert_with(|| {
                        let mut v = BTreeMap::new();
                        for candidate in a.dfs().skip(1) {
                            if search.named && !candidate.is_named() {
                                continue;
                            }
                            if leaf
                                .match_root(candidate.clone(), &mut work)
                                .unwrap()
                                .is_some()
                            {
                                assert!(
                                    candidate.is_named_leaf(),
                                    "oracle admits only leaf capture keys"
                                );
                                v.entry(candidate.text().into_owned())
                                    .or_insert(candidate.node_id() as u32);
                            }
                        }
                        v
                    });
                }
                if stop.match_root(a, &mut work).unwrap().is_some() {
                    break;
                }
            }
            let mut keys = BTreeMap::new();
            for (a, _) in &ancestors {
                keys.extend(values[a].clone());
            }
            for (text, key) in keys {
                let mut env = MetaVarEnv::new();
                env.insert(&search.capture, node_at(&tree, key)).unwrap();
                let mut env = Cow::Owned(env);
                begin_clause(grammar, usize::MAX);
                let hit = search
                    .matcher
                    .match_node_with_env(n.clone(), &mut env)
                    .is_some();
                assert!(end_clause().1.is_none());
                if !hit {
                    continue;
                }
                let capture = env.get_match(&search.capture).unwrap().node_id() as u32;
                let AstMatcher::Inside(body, _) = &search.matcher else {
                    unreachable!()
                };
                let (ancestor, distance) = ancestors
                    .iter()
                    .find(|(a, _)| {
                        let mut env = MetaVarEnv::new();
                        env.insert(&search.capture, node_at(&tree, key)).unwrap();
                        begin_clause(grammar, usize::MAX);
                        let found = body
                            .match_node_with_env(node_at(&tree, *a), &mut Cow::Owned(env))
                            .is_some();
                        assert!(end_clause().1.is_none());
                        found
                    })
                    .copied()
                    .unwrap();
                output.push((file, id, text, capture, ancestor, distance));
            }
        }
    }
    output.sort();
    output
}

pub(in crate::code_rule_dsl::sql) fn measure(
    grammar: &GrammarMatcher,
    sources: &[(String, String)],
) -> Value {
    let searches = witness::discover(grammar).unwrap();
    assert_eq!(
        searches.len(),
        1,
        "this measurement expects one demand frontier"
    );
    let search = searches.into_iter().next().unwrap();
    let native = expected(grammar, &search, sources);
    let started = Started::now();
    let compiled = witness::compile(grammar, search).unwrap();
    let generation = started.elapsed();
    let docs = sources
        .iter()
        .map(|(_, s)| (tape(grammar.language, s), FileContext::default()))
        .collect();
    let ((mut actual, metrics, explain, rows), _) = profiling::frame_counts(|| {
        sql::exec::with_terminals(docs, compiled.program.terminals, |db, batch, _| {
            let patterns = leaves::register(db, batch).unwrap();
            let before = profiling::frame_snapshot();
            let started = Started::now();
            let mut stmt = db.prepare(&compiled.sql).unwrap();
            let prepare = started.elapsed();
            let actual = stmt
                .query_map([], |r| {
                    let file = r.get::<_, u32>(0)? as usize;
                    let atom = r.get::<_, u32>(2)?;
                    Ok((
                        file,
                        r.get(1)?,
                        node_at(&batch.files[file].root, atom).text().into_owned(),
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<Row>>>()
                .unwrap();
            let time = started.elapsed();
            let vm = stmt.get_status(rusqlite::StatementStatus::VmStep);
            let automatic = stmt.get_status(rusqlite::StatementStatus::AutoIndex);
            let memory = stmt.get_status(rusqlite::StatementStatus::MemUsed);
            let after = profiling::frame_snapshot();
            let counters: [u64; 14] = std::array::from_fn(|i| after[i] - before[i]);
            let metrics = json!({"vmSteps":vm,"automaticIndexRows":automatic,"statementMemoryBytes":memory,
            "generationCpuNs":generation.cpu,"prepareCpuNs":prepare.cpu,"prepareAndExecuteCpuNs":time.cpu,
            "nodeAdvances":counters[11],"nodeFilters":counters[13],"regexRequests":counters[10],"framePublications":counters[4],
            "patternRequests":patterns.load(std::sync::atomic::Ordering::Relaxed),"matchingStatements":1});
            assert_eq!(counters[4], 0);
            drop(stmt);
            let explain=db.prepare(&format!("EXPLAIN QUERY PLAN {}",compiled.sql)).unwrap().query_map([],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"parent":r.get::<_,i64>(1)?,"detail":r.get::<_,String>(3)?}))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
            let prefix = compiled
                .sql
                .rsplit_once("SELECT file,node,atom,witness,ancestor,distance FROM capture_demand")
                .unwrap()
                .0;
            let roots = compiled.profile["roots"].as_u64().unwrap();
            let names = [
                if compiled.sql.contains(&format!("search{roots}(")) {
                    format!("search{roots}")
                } else {
                    format!("r{}", compiled.profile["origins"])
                },
                format!("walk{roots}"),
                format!("pairs{roots}"),
                format!("r{roots}"),
                format!("regions{roots}"),
                "binding_seeds".into(),
                "binding_choices".into(),
                "capture_demand".into(),
            ];
            let counts = names
                .iter()
                .map(|n| format!("SELECT '{n}',COUNT(*) FROM {n}"))
                .collect::<Vec<_>>()
                .join(" UNION ALL ");
            let rows = db
                .prepare(&format!("{prefix}{counts}"))
                .unwrap()
                .query_map([], |r| {
                    Ok(json!({"relation":r.get::<_,String>(0)?,"rows":r.get::<_,i64>(1)?}))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            (actual, metrics, explain, rows)
        })
    });
    actual.sort();
    assert_eq!(
        actual, native,
        "exact native origin/value/witness/ancestor/distance parity"
    );
    json!({"files":sources.len(),"rows":actual.len(),"parity":true,"metrics":metrics,"profile":compiled.profile,"sql":compiled.sql,"explain":explain,"diagnosticRelationRows":rows,"output":actual,
        "scope":"One intermediate capture-demand statement, not complete clause matching or a complete cold job. Diagnostic cardinality/EXPLAIN queries are separate from matching counters."})
}

fn policy() -> crate::code_rule_dsl::CompiledCodeRuleDsl {
    use crate::code_rule_dsl::{CodeRuleDslRuntime, test_support::CAPABILITIES};
    CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"demand-semantics","revision":1,"mode":"steer","scope":{"include":["**/*"],"exclude":[]},"clauses":[{"id":"x","forbid":{"ast":{"language":"python","rule":{"all":[{"kind":"identifier","pattern":"$NAME"},{"inside":{"stopBy":"end","matches":"scope"}}]},"utils":{
        "scope":{"kind":"function_definition","inside":{"stopBy":{"kind":"function_definition"},"matches":"producer"}},
        "producer":{"kind":"for_statement","has":{"stopBy":{"kind":"function_definition"},"kind":"identifier","pattern":"$NAME","all":[{"not":{"follows":{"stopBy":"neighbor","regex":"(?s)."}}},{"inside":{"stopBy":"neighbor","kind":"assignment"}}]}}
    }}},"violation":{"code":"D","message":"demand","anchor":"witness.span"}}]}).to_string(),"json".into(),CAPABILITIES.into()).unwrap()
}

#[test]
fn ordered_demand_preserves_origins_values_and_witnesses() {
    let policy = policy();
    let grammar = &crate::code_rule_dsl::test_support::grammar_matchers(&policy)[0];
    let sources=vec![("choices.py".into(),"for outer in xs:\n    a = 1\n    a = 2\n    b = 1\n    def first():\n        return a\n    def second():\n        return a\n    for inner in ys:\n        a = 3\n        def third():\n            def blocked():\n                return b\n            return a, b\n".into()),("other.py".into(),"for other in xs:\n    a = 4\n    def fourth():\n        return a\n".into())];
    let result = measure(grammar, &sources);
    let rows = result["output"].as_array().unwrap();
    assert!(rows.len() >= 7);
    let origins: BTreeSet<_> = rows
        .iter()
        .map(|r| (r[0].as_u64(), r[1].as_u64()))
        .collect();
    assert_eq!(
        origins.len(),
        4,
        "blocked nested function cannot see the outer loops"
    );
}

#[test]
fn ordered_demand_requires_a_bound_input_key() {
    let p = policy();
    for optional in [false, true] {
        let mut grammar = crate::code_rule_dsl::test_support::grammar_matchers(&p)[0].clone();
        fn erase_same_node_bind(rule: &mut AstMatcher, optional: bool) {
            match rule {
                AstMatcher::Pattern(_) => {
                    *rule = if optional {
                        AstMatcher::Any(vec![rule.clone(), AstMatcher::All(vec![])])
                    } else {
                        AstMatcher::All(vec![])
                    }
                }
                AstMatcher::All(items) => {
                    for item in items {
                        erase_same_node_bind(item, optional);
                    }
                }
                _ => {}
            }
        }
        erase_same_node_bind(&mut grammar.matcher, optional);
        let Err(error) = witness::discover(&grammar) else {
            panic!("accepted an unbound ordered search")
        };
        assert!(error.contains("already bound"), "{error}");
    }
}

#[test]
fn ordered_demand_refuses_composite_equality_keys() {
    let p = policy();
    let grammar = &crate::code_rule_dsl::test_support::grammar_matchers(&p)[0];
    let mut search = witness::discover(grammar).unwrap().remove(0);
    search.leaf_filter = AstMatcher::Kind(ast_grep_core::matcher::KindMatcher::new(
        "assignment",
        grammar.language,
    ));
    let compiled = witness::compile(grammar, search).unwrap();
    sql::exec::with_terminals(
        vec![(
            tape(
                grammar.language,
                "for x in xs:\n    a = 1\n    def f():\n        return a\n",
            ),
            FileContext::default(),
        )],
        compiled.program.terminals,
        |db, batch, _| {
            leaves::register(db, batch).unwrap();
            let result = db
                .prepare(&compiled.sql)
                .unwrap()
                .query_map([], |r| r.get::<_, u32>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>();
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("composite capture")
            );
        },
    );
}

#[test]
fn complete_answers_keep_nearest_choices_and_capture_identity() {
    let policy = policy();
    let grammar = &crate::code_rule_dsl::test_support::grammar_matchers(&policy)[0];
    let sources = vec![("choices.py".into(),
        "for outer in xs:\n a=1\n a=2\n def first():\n  return a+a\n for inner in ys:\n  a=3\n  def second():\n   return a\n  def blocked():\n   def third():\n    return a\n".into()),
        ("other.py".into(),"for other in ys:\n a=4\n def fourth():\n  return a\n".into())];
    let result = super::tests::measure(grammar, &sources);
    assert_eq!(result["matches"], 5);
    // A later same-node pattern overwrites the witness; value equality still constrains it.
    let mut grammar = grammar.clone();
    fn move_binding(rule: &mut AstMatcher) -> Option<AstMatcher> {
        if let AstMatcher::All(items) = rule {
            for index in 0..items.len() {
                if matches!(items[index], AstMatcher::Pattern(_)) {
                    return Some(items.remove(index));
                }
                if let Some(binding) = move_binding(&mut items[index]) {
                    return Some(binding);
                }
            }
        }
        None
    }
    let binding = move_binding(&mut grammar.matcher).unwrap();
    // Keep the initial binding as input; repeat it after the ancestor search.
    let AstMatcher::All(items) = &mut grammar.matcher else {
        unreachable!()
    };
    items.insert(0, binding.clone());
    items.push(binding);
    let result = super::tests::measure(&grammar, &sources);
    for file in result["output"].as_array().unwrap() {
        for hit in file.as_array().unwrap() {
            assert_eq!(hit[0], hit[1][0]);
        }
    }
}

#[test]
fn complete_answers_reject_unbound_negative_inputs() {
    let policy = policy();
    let mut grammar = crate::code_rule_dsl::test_support::grammar_matchers(&policy)[0].clone();
    let pattern = AstMatcher::Pattern(std::sync::Arc::new(ast_grep_core::matcher::Pattern::new(
        "$NAME",
        grammar.language,
    )));
    grammar.matcher = AstMatcher::All(vec![AstMatcher::Not(Box::new(pattern)), grammar.matcher]);
    let super::answers::Eligibility::NotApplicable(reason) =
        super::answers::prepare(&grammar).unwrap()
    else {
        panic!("frontier accepted unbound negative input")
    };
    assert_eq!(reason, super::witness::ShapeReason::BoundProbeInput);
    // The ordered outcome representation can express this rule without a bound frontier.
    super::tests::measure(
        &grammar,
        &[("unbound.py".into(), "for x in xs:\n    f(x)\n".into())],
    );
}
