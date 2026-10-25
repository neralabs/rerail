//! Integrated public compiler versus native, including fresh parsing and result assembly.
use super::*;
use crate::code_rule_dsl::model::{ClauseKind, Query};
use ast_grep_core::{meta_var::MetaVarEnv, source::Content};
use std::path::{Path, PathBuf};
type Fixture = crate::code_rule_dsl::sql::relational::cutover_tests::execution::Fixture;
struct Rule {
    name: String,
    grammar: GrammarMatcher,
    each: Option<String>,
}
fn rules(paths: Vec<PathBuf>) -> Vec<Rule> {
    let mut catalog = vec![];
    for path in paths {
        let p = CodeRuleDslRuntime::compile(
            std::fs::read_to_string(&path).unwrap(),
            "json".into(),
            CAPABILITIES.into(),
        )
        .unwrap();
        for clause in &p.inner.clauses {
            if let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &clause.kind {
                for g in matchers {
                    if g.language == crate::code_rule_dsl::language::GrammarDialect::Python {
                        catalog.push(Rule {
                            name: format!("{}:{}", p.inner.id, clause.id),
                            grammar: g.clone(),
                            each: clause.violation.each.clone(),
                        });
                    }
                }
            }
        }
    }
    catalog
}
fn env_match(node: u32, env: &MetaVarEnv<'_, TapeDoc>, template: &planning::Template) -> SqlMatch {
    let captures = template
        .names
        .iter()
        .filter_map(|name| {
            let value = if let Some(nodes) = env.get_labels(name) {
                Captured::List(
                    nodes
                        .first()
                        .zip(nodes.last())
                        .map(|(a, b)| (a.node_id() as u32, b.node_id() as u32)),
                )
            } else if let Some(node) = env.get_match(name) {
                Captured::Node(node.node_id() as u32)
            } else if let Some(range) = matcher::exported_span(env, name) {
                Captured::Text(matcher::exported_text(env, name).unwrap_or_default(), range)
            } else if let Some(bytes) = env.get_transformed(name) {
                Captured::Count(
                    <TapeDoc as ast_grep_core::Doc>::Source::encode_bytes(bytes)
                        .parse()
                        .expect("integer count"),
                )
            } else {
                return None;
            };
            Some((name.clone(), value))
        })
        .collect();
    SqlMatch {
        root: node,
        captures,
    }
}
fn native(catalog: &[Rule], fixtures: &[Fixture]) -> Vec<Vec<FileOutcome>> {
    let templates = catalog
        .iter()
        .map(|r| planning::Template::compile(&r.grammar, r.each.as_deref()))
        .collect::<Vec<_>>();
    let mut output = vec![vec![FileOutcome::default(); fixtures.len()]; catalog.len()];
    for (file, f) in fixtures.iter().enumerate() {
        let doc = crate::code_rule_dsl::sql::tests::differential::tape(
            catalog[0].grammar.language,
            &f.source,
        );
        let errors = crate::code_rule_dsl::soundness::SyntaxErrors::of_doc(&doc);
        let tree = ast_grep_core::AstGrep::doc(doc);
        let _scopes =
            matcher::FileScopeSlot::install(Default::default(), None, errors.hide_bindings())
                .path(&f.path);
        matcher::with_file_memo(&tree, || {
            for (id, r) in catalog.iter().enumerate() {
                if !r.grammar.applies
                    || r.grammar
                        .file
                        .as_ref()
                        .is_some_and(|c| !c.holds(&f.path, None))
                {
                    continue;
                }
                let out = &mut output[id][file];
                let mut work = usize::MAX;
                let result = matcher::run_clause(
                    &tree,
                    &r.grammar,
                    None,
                    tree.root().get_doc().transducer_len(),
                    &mut work,
                    |node, env, work| {
                        if let Some(each) = &r.each {
                            matcher::enumerate(
                                &tree,
                                &r.grammar,
                                node,
                                env,
                                work,
                                each,
                                |env, _| {
                                    out.matches.push(env_match(node, &env, &templates[id]));
                                    Ok(true)
                                },
                            )
                        } else {
                            out.matches.push(env_match(node, &env, &templates[id]));
                            Ok(true)
                        }
                    },
                );
                let stopped = match result {
                    Ok(Ok(())) => None,
                    Ok(Err(s)) | Err(s) => Some(s),
                };
                if let Some(stopped) = stopped {
                    out.unknown = Some(match stopped {
                        matcher::Stopped::Limit(l) => Unknown::Limit(l),
                        matcher::Stopped::Deadline => Unknown::Deadline,
                    });
                    out.matches.clear();
                }
            }
        });
    }
    output
}
fn hybrid(
    compiled: &Compiled,
    fixtures: &[Fixture],
    language: crate::code_rule_dsl::language::GrammarDialect,
) -> (Vec<Vec<FileOutcome>>, serde_json::Value) {
    let mut session = compiled.session().unwrap();
    let docs = fixtures
        .iter()
        .map(|f| {
            (
                crate::code_rule_dsl::sql::tests::differential::tape(language, &f.source),
                FileContext {
                    path: Some(f.path.clone()),
                    ..Default::default()
                },
            )
        })
        .collect();
    let runs = (0..compiled.clauses.len())
        .map(|statement| ClauseRun {
            statement,
            files: (0..fixtures.len()).map(|f| (f, None)).collect(),
        })
        .collect::<Vec<_>>();
    let rows = session.run(docs, &runs).unwrap();
    assert!(session.stats().statements <= 100, "hard execution budget");
    let mut detail = session.last_profile.clone();
    detail["statementsExecuted"] = session.stats().statements.into();
    detail["setupCpuNs"] = session.stats().setup.cpu.into();
    detail["nodeFactsCpuNs"] = session.stats().node_facts.cpu.into();
    detail["sql"] = serde_json::json!(*compiled.queries);
    detail["plans"] = serde_json::json!(session.plans().unwrap());
    (rows, detail)
}
fn run(catalog: &[Rule], fixtures: &[Fixture], repeats: usize) -> serde_json::Value {
    let timer = Started::now();
    let mut compiler = ProgramCompiler::default();
    for r in catalog {
        compiler.clause(&r.grammar, r.each.as_deref()).unwrap();
    }
    let compiled = compiler.finish().0;
    let compilation = timer.elapsed();
    let mut runs = vec![];
    for iteration in 0..repeats {
        let n = || {
            let t = Started::now();
            let r = native(catalog, fixtures);
            (r, t.elapsed())
        };
        let h = || {
            let t = Started::now();
            let r = hybrid(&compiled, fixtures, catalog[0].grammar.language);
            (r, t.elapsed())
        };
        let ((expected, nt), ((actual, detail), ht)) = if iteration % 2 == 0 {
            (n(), h())
        } else {
            let h = h();
            (n(), h)
        };
        let mut unknown_native = 0;
        let mut unknown_sql = 0;
        let mut compared = 0;
        let mut matches = 0;
        let mut mismatches = vec![];
        for (rule, (a, e)) in actual.iter().zip(&expected).enumerate() {
            for (file, (a, e)) in a.iter().zip(e).enumerate() {
                unknown_native += usize::from(e.unknown.is_some());
                unknown_sql += usize::from(a.unknown.is_some());
                if e.unknown.is_some() || a.unknown.is_some() {
                    continue;
                }
                compared += 1;
                matches += a.len();
                if a.matches != e.matches {
                    mismatches.push(serde_json::json!({"rule":catalog[rule].name,"file":fixtures[file].path,"native":format!("{:?}",e.matches),"sql":format!("{:?}",a.matches)}));
                }
            }
        }
        eprintln!(
            "integrated job files={} rules={} nativeCpuMs={:.3} sqlCpuMs={:.3} compared={} unknownNative={} unknownSql={} mismatches={}",
            fixtures.len(),
            catalog.len(),
            nt.cpu as f64 / 1e6,
            ht.cpu as f64 / 1e6,
            compared,
            unknown_native,
            unknown_sql,
            mismatches.len()
        );
        assert_eq!(
            unknown_sql, unknown_native,
            "complete catalog executions must not acquire coverage gaps"
        );
        let run = serde_json::json!({"nativeCpuNs":nt.cpu,"nativeWallNs":nt.wall,"sqlCpuNs":ht.cpu,"sqlWallNs":ht.wall,"compared":compared,"matches":matches,"unknownNative":unknown_native,"unknownSql":unknown_sql,"mismatches":mismatches,"detail":detail});
        if let Ok(path) = std::env::var("RERAIL_FRAGMENT_INTEGRATION_EVIDENCE") {
            std::fs::write(path, serde_json::to_vec_pretty(&run).unwrap()).unwrap();
        }
        assert!(
            mismatches.is_empty(),
            "first differences: {:?}",
            &mismatches[..mismatches.len().min(5)]
        );
        runs.push(run);
    }
    serde_json::json!({"files":fixtures.len(),"grammars":catalog.len(),"sourceBytes":fixtures.iter().map(|f|f.source.len()).sum::<usize>(),"compileCpuNs":compilation.cpu,"compileWallNs":compilation.wall,"rules":catalog.iter().map(|r|&r.name).collect::<Vec<_>>(),"runs":runs})
}
#[test]
fn fragment_integration_catalog_job() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut paths = std::fs::read_dir(root.join("packs/python/rules"))
        .unwrap()
        .map(|p| p.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect::<Vec<_>>();
    paths.sort();
    let catalog = rules(paths);
    assert!(!catalog.is_empty());
    let fixtures = crate::code_rule_dsl::sql::relational::cutover_tests::execution::fixtures(root);
    let report = run(&catalog, &fixtures, 1);
    if let Ok(path) = std::env::var("RERAIL_FRAGMENT_INTEGRATION_EVIDENCE") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
#[test]
#[ignore = "requires explicit full repository and policy paths"]
fn fragment_integration_repository_job() {
    let root = PathBuf::from(std::env::var("RERAIL_FRAGMENT_CORPUS").expect("corpus path"));
    let policies = PathBuf::from(std::env::var("RERAIL_FRAGMENT_POLICIES").expect("policies path"));
    fn collect(root: &Path, extension: &str, files: &mut Vec<PathBuf>) {
        for p in std::fs::read_dir(root).unwrap() {
            let p = p.unwrap();
            if p.file_type().unwrap().is_dir() {
                collect(&p.path(), extension, files);
            } else if p.path().extension().is_some_and(|e| e == extension) {
                files.push(p.path());
            }
        }
    }
    let mut paths = vec![];
    collect(&policies, "json", &mut paths);
    paths.sort();
    let catalog = rules(paths);
    let tracked = std::process::Command::new("git")
        .args(["-C", root.to_str().unwrap(), "ls-files", "-z"])
        .output()
        .unwrap();
    assert!(tracked.status.success());
    let fixtures = std::str::from_utf8(&tracked.stdout)
        .unwrap()
        .split('\0')
        .filter(|p| p.ends_with(".py"))
        .map(|path| Fixture {
            path: path.into(),
            source: std::fs::read_to_string(root.join(path)).unwrap(),
        })
        .collect::<Vec<_>>();
    assert!(!fixtures.is_empty());
    assert!(!catalog.is_empty());
    let report = run(&catalog, &fixtures, 1);
    if let Ok(path) = std::env::var("RERAIL_FRAGMENT_INTEGRATION_EVIDENCE") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
