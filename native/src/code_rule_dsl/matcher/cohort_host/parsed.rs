//! Parsed-document child cohorts and their explicit parent bridge.
use super::{Context, Cost, NativeHost, ParentAnswer, ParentRequest, ParsedContext};
use crate::code_rule_dsl::{
    literal::{Parsed, SubDoc, SubTree},
    matcher::{
        Semantic,
        cohort::{self, Programs, Request, SemanticRequest},
        exported::{export, exported_span, exported_text},
        format_args::Prepared,
        parse,
    },
};
use ast_grep_core::{AstGrep, Doc, Node, meta_var::MetaVarEnv};
use std::{borrow::Cow, collections::BTreeMap, ops::Range, sync::Arc};
type Exports = Vec<(String, String, Range<usize>)>;

fn exports<D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>>(
    env: &MetaVarEnv<'_, D>,
    tree: &SubTree,
    names: &[String],
    nested: bool,
) -> Exports {
    names
        .iter()
        .filter_map(|name| {
            if let Some(node) = env.get_match(name) {
                Some((
                    name.clone(),
                    node.text().into_owned(),
                    tree.host_range(node.node_id())?,
                ))
            } else if nested {
                Some((
                    name.clone(),
                    exported_text(env, name)?,
                    tree.host_span(exported_span(env, name)?)?,
                ))
            } else {
                None
            }
        })
        .collect()
}
fn bind<'t, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>>(
    env: &Arc<MetaVarEnv<'t, D>>,
    values: &Exports,
) -> Arc<MetaVarEnv<'t, D>> {
    let mut env = Cow::Borrowed(env.as_ref());
    for (name, text, range) in values {
        export(&mut env, name, text, range.clone());
    }
    Arc::new(env.into_owned())
}

struct TreeJob {
    slot: usize,
    tree: Arc<SubTree>,
    table: Option<Arc<crate::code_rule_dsl::scope::ScopeTable>>,
    program: u32,
    nodes: Option<Vec<(usize, bool)>>,
    names: Vec<String>,
    hosted: bool,
}

// The SubDocs stay at stable addresses until all child runs and export projections finish.
fn trees<'t, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>, C: Context>(
    host: &mut NativeHost<C>,
    programs: &Programs,
    leaves: &[SemanticRequest<'t, D>],
    jobs: Vec<TreeJob>,
) -> Vec<(usize, u8, Cost, Exports)> {
    let roots = jobs
        .iter()
        .map(|j| {
            AstGrep::doc(SubDoc {
                tree: j.tree.clone(),
            })
        })
        .collect::<Vec<_>>();
    let mut child = host.fork(None);
    let mut contexts = Vec::new();
    for job in &jobs {
        let id = child.next_context.get();
        child
            .next_context
            .set(id.checked_add(1).expect("context ids"));
        contexts.push(id);
        let q = &leaves[job.slot];
        let mut maps = host
            .parsed
            .get(&q.request.context)
            .map_or_else(Vec::new, |p| p.maps.clone());
        maps.push(parse::host_map(&job.tree));
        child.parsed.insert(
            id,
            ParsedContext {
                owner: host.owner(q.request.context),
                slot: job.slot,
                tree: job.tree.clone(),
                tables: host
                    .parsed
                    .get(&q.request.context)
                    .map_or_else(Vec::new, |p| p.tables.clone())
                    .into_iter()
                    .chain(job.table.clone())
                    .collect(),
                maps,
            },
        );
    }
    let mut bridge = |p: &Programs, requests: Vec<ParentRequest>| {
        let mut groups = BTreeMap::<u32, Vec<(usize, Request<'t, D>)>>::new();
        let mut answers: Vec<Option<ParentAnswer>> = (0..requests.len()).map(|_| None).collect();
        for (i, r) in requests.into_iter().enumerate() {
            if !jobs.iter().any(|j| j.slot == r.slot && j.hosted) {
                answers[i] = Some(ParentAnswer {
                    truth: 0,
                    cost: Cost::default(),
                });
                continue;
            }
            let mut req = leaves[r.slot].request.clone();
            req.program = r.program;
            req.each = None;
            req.env = bind(&req.env, &r.exports);
            groups.entry(r.depth).or_default().push((i, req));
        }
        for (depth, group) in groups {
            let (ids, reqs): (Vec<_>, Vec<_>) = group.into_iter().unzip();
            for (i, r) in ids.into_iter().zip(host.run_host(p, reqs, depth)) {
                answers[i] = Some(ParentAnswer {
                    truth: r.truth,
                    cost: Cost::of(&r),
                });
            }
        }
        answers.into_iter().map(Option::unwrap).collect()
    };
    child.parent = Some(&mut bridge);
    let candidates = jobs
        .iter()
        .zip(&roots)
        .map(|(job, root)| match &job.nodes {
            None => vec![(root.root(), true)],
            Some(ids) => root
                .root()
                .dfs()
                .filter_map(|n| {
                    ids.iter()
                        .find(|(id, _)| *id == n.node_id())
                        .map(|(_, k)| (n, *k))
                })
                .collect(),
        })
        .collect::<Vec<Vec<(Node<'_, SubDoc>, bool)>>>();
    let mut output = jobs
        .iter()
        .map(|j| (j.slot, 0, Cost::default(), Vec::new()))
        .collect::<Vec<_>>();
    let mut done = vec![false; jobs.len()];
    let mut unknown = vec![false; jobs.len()];
    for round in 0..candidates.iter().map(Vec::len).max().unwrap_or(0) {
        let mut groups = BTreeMap::<u32, Vec<(usize, Request<'_, SubDoc>)>>::new();
        for i in 0..jobs.len() {
            if done[i] || round >= candidates[i].len() {
                continue;
            }
            let q = &leaves[jobs[i].slot];
            groups.entry(q.utility_depth).or_default().push((
                i,
                Request {
                    program: jobs[i].program,
                    context: contexts[i],
                    node: candidates[i][round].0.clone(),
                    env: Arc::new(MetaVarEnv::new()),
                    three: q.request.three,
                    each: q.request.each.clone(),
                    following: q.request.following.clone(),
                },
            ));
        }
        for (depth, group) in groups {
            let (ids, reqs): (Vec<_>, Vec<_>) = group.into_iter().unzip();
            for (i, r) in ids.into_iter().zip(child.run(programs, reqs, depth)) {
                output[i].2.add(Cost::of(&r));
                if r.failure.is_some() {
                    done[i] = true;
                    continue;
                }
                if r.truth == 2 && jobs[i].nodes.is_some() {
                    if let Some(each) = &leaves[jobs[i].slot].request.each {
                        if let Some(range) =
                            cohort::Host::binding_range(&mut child, contexts[i], &r.env, &each.name)
                        {
                            if each.ranges.contains(&(range.start, range.end)) {
                                continue;
                            }
                        }
                    }
                }
                if r.truth == 2 && candidates[i][round].1 {
                    output[i].1 = 2;
                    output[i].3 = exports(
                        &r.env,
                        &jobs[i].tree,
                        &jobs[i].names,
                        jobs[i].nodes.is_none(),
                    );
                    done[i] = true;
                } else {
                    unknown[i] |= r.truth != 0;
                }
            }
        }
    }
    for i in 0..jobs.len() {
        if !done[i] && unknown[i] {
            output[i].1 = 1;
        }
    }
    output
}

pub(super) fn evaluate<
    't,
    D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>,
    C: Context,
>(
    host: &mut NativeHost<C>,
    programs: &Programs,
    leaves: Vec<SemanticRequest<'t, D>>,
) -> Vec<cohort::Result<'t, D>> {
    match leaves.first().map(|q| q.semantic.as_ref()) {
        Some(Semantic::Host(_)) => {
            let mut requests = Vec::new();
            let mut ids = Vec::new();
            let contexts = host
                .parsed
                .iter()
                .map(|(id, p)| (*id, (p.slot, p.tree.clone())))
                .collect::<BTreeMap<_, _>>();
            let projected = host.project(&leaves, |q| {
                let Semantic::Host(test) = q.semantic.as_ref() else {
                    unreachable!()
                };
                contexts
                    .get(&q.request.context)
                    .map(|(slot, tree)| ParentRequest {
                        slot: *slot,
                        program: programs.lookup(&test.rule).expect("host child"),
                        exports: parse::host_exports(&q.request.env, tree),
                        depth: q.utility_depth,
                    })
            });
            let mut out = Vec::new();
            for (i, (q, p)) in leaves.iter().zip(projected).enumerate() {
                out.push(p.cost.answer(0, q.request.env.clone()));
                if let Some(request) = p.value.flatten() {
                    requests.push(request);
                    ids.push(i);
                }
            }
            if let Some(parent) = host.parent.as_mut() {
                for (i, r) in ids.into_iter().zip(parent.evaluate(programs, requests)) {
                    let mut cost = Cost::of(&out[i]);
                    cost.add(r.cost);
                    out[i] = cost.answer(r.truth, leaves[i].request.env.clone());
                }
            }
            out
        }
        Some(Semantic::Parse(_)) => {
            let projected = host.project(&leaves, |q| {
                let Semantic::Parse(test) = q.semantic.as_ref() else {
                    unreachable!()
                };
                test.tree_at(&q.request.node)
            });
            let mut out = Vec::new();
            let mut jobs = Vec::new();
            for (i, (q, p)) in leaves.iter().zip(projected).enumerate() {
                let Semantic::Parse(test) = q.semantic.as_ref() else {
                    unreachable!()
                };
                let mut truth = 0;
                match p.value.flatten() {
                    Some((Parsed::Unknown, _)) => truth = 1,
                    Some((Parsed::Tree(tree), table)) => jobs.push(TreeJob {
                        slot: i,
                        tree,
                        table,
                        program: programs.lookup(&test.rule).expect("parse child"),
                        nodes: None,
                        names: test.exports.clone(),
                        hosted: test.host_sound.is_some(),
                    }),
                    _ => {}
                }
                out.push(p.cost.answer(truth, q.request.env.clone()));
            }
            for (i, truth, cost, values) in trees(host, programs, &leaves, jobs) {
                let mut total = Cost::of(&out[i]);
                total.add(cost);
                out[i] = total.answer(
                    truth,
                    if truth == 2 {
                        bind(&leaves[i].request.env, &values)
                    } else {
                        leaves[i].request.env.clone()
                    },
                );
            }
            out
        }
        Some(Semantic::FormatArgs(_)) => format(host, programs, leaves),
        _ => unreachable!(),
    }
}

fn format<'t, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>, C: Context>(
    host: &mut NativeHost<C>,
    programs: &Programs,
    leaves: Vec<SemanticRequest<'t, D>>,
) -> Vec<cohort::Result<'t, D>> {
    let prepared = host.project(&leaves, |q| {
        let Semantic::FormatArgs(test) = q.semantic.as_ref() else {
            unreachable!()
        };
        test.prepare(&q.request.env)
    });
    let mut facts = Vec::new();
    let mut out = Vec::new();
    for (q, p) in leaves.iter().zip(prepared) {
        let (truth, fact) = match p.value {
            Some(Prepared::Sides {
                tree,
                unused,
                missing,
            }) => (2, Some((tree, unused, missing))),
            Some(Prepared::Unknown) => (1, None),
            _ => (0, None),
        };
        out.push(p.cost.answer(truth, q.request.env.clone()));
        facts.push(fact);
    }
    let mut unknown = vec![false; leaves.len()];
    let mut found = vec![false; leaves.len()];
    let rounds = facts
        .iter()
        .filter_map(Option::as_ref)
        .map(|(_, u, _)| u.candidates.len())
        .max()
        .unwrap_or(0);
    for round in 0..rounds {
        let mut groups = BTreeMap::<u32, Vec<(usize, Request<'t, D>)>>::new();
        for (i, q) in leaves.iter().enumerate() {
            let Semantic::FormatArgs(test) = q.semantic.as_ref() else {
                unreachable!()
            };
            let Some(rule) = &test.unused else { continue };
            let Some((_, unused, _)) = &facts[i] else {
                continue;
            };
            if found[i] || out[i].failure.is_some() || round >= unused.candidates.len() {
                continue;
            }
            let mut req = q.request.clone();
            req.program = programs.lookup(rule).expect("unused child");
            req.node = unused.candidates[round].0.clone();
            groups.entry(q.utility_depth).or_default().push((i, req));
        }
        for (depth, group) in groups {
            let (ids, reqs): (Vec<_>, Vec<_>) = group.into_iter().unzip();
            for (i, r) in ids.into_iter().zip(host.run(programs, reqs, depth)) {
                let mut cost = Cost::of(&out[i]);
                cost.add(Cost::of(&r));
                out[i] = cost.answer(out[i].truth, out[i].env.clone());
                if r.truth == 2 {
                    if let Some(each) = &leaves[i].request.each {
                        if let Some(range) = cohort::Host::binding_range(
                            host,
                            leaves[i].request.context,
                            &r.env,
                            &each.name,
                        ) {
                            if each.ranges.contains(&(range.start, range.end)) {
                                continue;
                            }
                        }
                    }
                    if facts[i].as_ref().unwrap().1.candidates[round].1 {
                        found[i] = true;
                        out[i].env = r.env;
                        continue;
                    }
                }
                unknown[i] |= r.truth != 0;
            }
        }
    }
    let mut jobs = Vec::new();
    for (i, q) in leaves.iter().enumerate() {
        let Semantic::FormatArgs(test) = q.semantic.as_ref() else {
            unreachable!()
        };
        let Some((tree, unused, missing)) = &facts[i] else {
            continue;
        };
        if test.unused.is_some() {
            out[i].truth = if found[i] {
                2
            } else if unknown[i] || unused.incomplete {
                1
            } else {
                0
            };
        }
        if out[i].truth == 0 || out[i].failure.is_some() {
            out[i].env = q.request.env.clone();
            continue;
        }
        if let Some(rule) = &test.missing {
            jobs.push(TreeJob {
                slot: i,
                tree: tree.clone(),
                table: None,
                program: programs.lookup(rule).expect("missing child"),
                nodes: Some(missing.candidates.clone()),
                names: test.exports.clone(),
                hosted: false,
            });
        }
    }
    for (i, mut truth, cost, values) in trees(host, programs, &leaves, jobs) {
        if truth == 0 && facts[i].as_ref().unwrap().2.incomplete {
            truth = 1;
        }
        let result = if truth == 0 || out[i].truth == 0 {
            0
        } else if truth == 2 && out[i].truth == 2 {
            2
        } else {
            1
        };
        let mut total = Cost::of(&out[i]);
        total.add(cost);
        let env = if result == 2 {
            bind(&out[i].env, &values)
        } else {
            leaves[i].request.env.clone()
        };
        out[i] = total.answer(result, env);
    }
    for (i, r) in out.iter_mut().enumerate() {
        if r.truth != 2 {
            r.env = leaves[i].request.env.clone();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::{
        language::GrammarDialect,
        matcher::{self, FileScopeSlot, GrammarMatcher},
        tape::{Grammar, TapeDoc, TapeImage},
        test_support::*,
    };
    use ast_grep_core::tree_sitter::StrDoc;
    use serde_json::{Value, json};

    struct TestContext<'a> {
        file: &'a AstGrep<TapeDoc>,
        grammar: &'a GrammarMatcher,
    }
    impl Context for TestContext<'_> {
        fn with<T>(&mut self, _: u32, run: impl FnOnce() -> T) -> T {
            matcher::begin_clause(self.grammar, 100_000);
            let result = matcher::with_file_memo(self.file, run);
            matcher::end_clause();
            result
        }
        fn cancelled(&self, _: u32) -> bool {
            false
        }
    }
    fn file(source: &str, language: GrammarDialect) -> AstGrep<TapeDoc> {
        let grammar = Grammar::new(language).unwrap();
        let native = StrDoc::new(source, language);
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
        AstGrep::doc(
            TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap(),
        )
    }
    fn parity(
        language: &str,
        dialect: GrammarDialect,
        rule: Value,
        source: &str,
        kind: &str,
        names: &[&str],
    ) -> Vec<bool> {
        let compiled=compile_document_with(clause_document(json!({"id":"cohort","violation":violation(),"forbid":{"ast":{"language":language,"rule":rule}}})),&serde_json::from_str::<Value>(CAPABILITIES).unwrap()).unwrap_or_else(|e|panic!("{}",e.reason));
        let grammar = &grammar_matchers(&compiled)[0];
        let file = file(source, dialect);
        let _scope = FileScopeSlot::install(Vec::new(), None, false);
        let nodes = file
            .root()
            .dfs()
            .filter(|n| n.kind() == kind)
            .collect::<Vec<_>>();
        let mut programs = Programs::default();
        let program = programs.lower(&grammar.matcher);
        let mut host = NativeHost::new(
            TestContext {
                file: &file,
                grammar,
            },
            [(0, 100_000)].into(),
        );
        let requests = nodes
            .iter()
            .map(|node| Request {
                program,
                context: 0,
                node: node.clone(),
                env: Arc::new(MetaVarEnv::new()),
                three: grammar.three_valued,
                each: None,
                following: Arc::default(),
            })
            .collect();
        let answers = host.run(&programs, requests, 0);
        assert_eq!(answers.len(), nodes.len());
        let mut found = Vec::new();
        for (node, answer) in nodes.into_iter().zip(answers) {
            let expected =
                matcher::with_file_memo(&file, || grammar.match_root(node, &mut 100_000).unwrap());
            assert!(answer.failure.is_none());
            assert_eq!(answer.truth == 2, expected.is_some());
            if let Some(expected) = expected {
                for name in names {
                    let text = |env: &MetaVarEnv<'_, TapeDoc>| {
                        env.get_match(name)
                            .map(|n| n.text().into_owned())
                            .or_else(|| exported_text(env, name))
                    };
                    assert_eq!(
                        text(&answer.env),
                        text(expected.get_env()),
                        "capture {name}"
                    );
                    let range = |env: &MetaVarEnv<'_, TapeDoc>| {
                        env.get_match(name)
                            .map(|n| n.range())
                            .or_else(|| exported_span(env, name))
                    };
                    assert_eq!(
                        range(&answer.env),
                        range(expected.get_env()),
                        "capture range {name}"
                    );
                }
            }
            found.push(answer.truth == 2);
        }
        assert_eq!(host.report()["scalar_matcher_calls"], 0);
        found
    }
    #[test]
    fn parse_cohorts_export_decoded_capture_spans_with_scalar_parity() {
        assert_eq!(
            parity(
                "python",
                GrammarDialect::Python,
                json!({"kind":"string","parse":{"grammar":"testChars","has":{"kind":"char","value":{"eq":"y"},"capture":"C"}}}),
                "a='q\\x79z'\nb='zz'\n",
                "string",
                &["C"]
            ),
            [true, false]
        );
    }
    #[test]
    fn host_cohorts_test_parent_rules_per_parsed_candidate() {
        assert_eq!(
            parity(
                "typescript",
                GrammarDialect::TypeScript,
                json!({"kind":"comment","parse":{"grammar":"eslintDirective","has":{"kind":"item","capture":"N","host":{"regex":"global"}}}}),
                "/*global b, a*/\n/*eslint no-alert*/\n",
                "comment",
                &["N"]
            ),
            [true, false]
        );
    }
    #[test]
    fn format_cohorts_search_unused_then_missing_candidates_and_export_spans() {
        assert_eq!(
            parity(
                "python",
                GrammarDialect::Python,
                json!({"pattern":"$S.format($$$A)","formatArgs":{"style":"brace","string":"S","args":"A","unused":{"pattern":"$U"},"missing":{"kind":"field","capture":"F"}}}),
                "'{x}'.format(1)\n'{}'.format(1)\n",
                "call",
                &["U", "F"]
            ),
            [true, false]
        );
    }
}
