//! Hand-written complete B023 query. Test-only: no production selector or rule evaluator callback.
use super::{graph::Graph, run};
use crate::code_rule_dsl::{
    CodeRuleDslRuntime,
    language::GrammarDialect,
    matcher::node_at,
    sql::{
        self,
        exec::{Captured, FileContext, SqlMatch},
        profiling,
        tests::differential::tape,
        timing::Started,
    },
    test_support::{CAPABILITIES, grammar_matchers},
};
use ast_grep_core::{
    AstGrep,
    matcher::{MatcherExt, Pattern},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

fn query() -> String {
    let tokens = regex::Regex::new(r"@([a-z_]+)@").unwrap();
    let template = std::fs::read_to_string(
        root().join("native/src/code_rule_dsl/sql/spikes/whole_call/b023.sql"),
    )
    .unwrap();
    tokens
        .replace_all(&template, |m: &regex::Captures<'_>| {
            GrammarDialect::Python.kinds().kind_id(&m[1]).to_string()
        })
        .into_owned()
}

fn policy() -> crate::code_rule_dsl::CompiledCodeRuleDsl {
    CodeRuleDslRuntime::compile(
        std::fs::read_to_string(
            root().join("packs/python/rules/ruff-b023-function-uses-loop-variable.json"),
        )
        .unwrap(),
        "json".into(),
        CAPABILITIES.into(),
    )
    .unwrap()
}
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned()
}

fn counters(c: [u64; 14]) -> Value {
    json!({"frameAdvances":c[2],"frameColumns":c[3],"framePublications":c[4],"frameIndexedRows":c[6],
      "regexNodeRequests":c[10],"nodeAdvances":c[11],"nodeColumns":c[12],"nodeFilters":c[13]})
}

fn check(sources: &[(String, String)], out: Option<&Path>, diagnostic: bool) -> Value {
    let compiled = policy();
    let grammars = grammar_matchers(&compiled);
    let grammar = &grammars[0];
    assert!(
        !grammar.three_valued,
        "this query requires B023's total leaf contract"
    );
    let parse_start = Started::now();
    let docs: Vec<_> = sources
        .iter()
        .map(|(_, s)| (tape(GrammarDialect::Python, s), FileContext::default()))
        .collect();
    let parse = parse_start.elapsed();
    let native_start = Started::now();
    let native: Vec<Vec<SqlMatch>> = docs
        .iter()
        .map(|(doc, _)| {
            let tree = AstGrep::doc(doc.clone());
            let mut work = usize::MAX;
            (0..doc.transducer_len() as u32)
                .filter_map(|id| {
                    let n = node_at(&tree, id);
                    // A necessary top-level kind test, independent of the SQL result.
                    if n.kind().as_ref() != "identifier" {
                        return None;
                    }
                    grammar
                            .match_root(n, &mut work)
                            .expect("native work")
                            .map(|m| SqlMatch {
                                root: id,
                                captures: vec![(
                                    "N".into(),
                                    Captured::Node(
                                        m.get_env().get_match("N").unwrap().node_id() as u32
                                    ),
                                )],
                            })
                })
                .collect()
        })
        .collect();
    let native_time = native_start.elapsed();
    let baseline_start = Started::now();
    let mut session = sql::exec::compile(&[grammar]).unwrap().session().unwrap();
    let runs = [sql::ClauseRun {
        statement: 0,
        files: (0..docs.len()).map(|f| (f, None)).collect(),
    }];
    let (baseline, count) = if diagnostic {
        profiling::frame_counts(|| session.run(docs.clone(), &runs).unwrap())
    } else {
        (session.run(docs.clone(), &runs).unwrap(), [0; 14])
    };
    let baseline_time = baseline_start.elapsed();
    assert_eq!(baseline[0], native, "current SQL/native roots and captures");
    let mut report = json!({"files":sources.len(),"sourceBytes":sources.iter().map(|(_,s)|s.len()).sum::<usize>(),
      "inputs":sources.iter().map(|(path,source)|json!({"path":path,"sha256":format!("{:x}",Sha256::digest(source.as_bytes()))})).collect::<Vec<_>>(),
      "matches":native.iter().map(Vec::len).sum::<usize>(),"parseCpuNs":parse.cpu,"nativeCpuNs":native_time.cpu,
      "baseline":{"coldKernelCpuNs":baseline_time.cpu,"matchingCpuNs":session.stats().time.cpu,
        "scheduledStatements":session.stats().statements,"counters":if diagnostic {counters(count)} else {Value::Null}}});
    let generation_start = Started::now();
    let sql = query();
    let generation = generation_start.elapsed();
    report["querySha256"] = json!(format!("{:x}", Sha256::digest(sql.as_bytes())));
    report["policySha256"] = json!(format!(
        "{:x}",
        Sha256::digest(
            std::fs::read(
                root().join("packs/python/rules/ruff-b023-function-uses-loop-variable.json")
            )
            .unwrap()
        )
    ));
    if let Some(out) = out {
        std::fs::create_dir_all(out).unwrap();
        std::fs::write(out.join("query.sql"), &sql).unwrap();
    }
    let direct_start = Started::now();
    sql::exec::with_terminals(docs, vec![], |connection, batch, frames| {
        struct Leaves(std::panic::AssertUnwindSafe<Arc<sql::exec::Batch>>, Pattern);
        impl Leaves {
            fn text(&self, f: usize, n: u32) -> String {
                node_at(&self.0.files[f].root, n).text().into_owned()
            }
            fn reduce(&self, f: usize, n: u32) -> bool {
                self.1
                    .match_node(node_at(&self.0.files[f].root, n))
                    .is_some()
            }
        }
        let leaves = Arc::new(Leaves(
            std::panic::AssertUnwindSafe(Arc::clone(batch)),
            Pattern::new("functools.reduce", GrammarDialect::Python),
        ));
        let leaf_calls = Arc::new(AtomicUsize::new(0));
        for (name, is_pattern) in [("spike_node_text", false), ("spike_is_reduce", true)] {
            let l = Arc::clone(&leaves);
            let c = Arc::clone(&leaf_calls);
            connection
                .create_scalar_function(
                    name,
                    2,
                    rusqlite::functions::FunctionFlags::SQLITE_UTF8,
                    move |ctx| {
                        c.fetch_add(1, Ordering::Relaxed);
                        let f = ctx.get::<i64>(0)? as usize;
                        let n = ctx.get::<u32>(1)?;
                        Ok(if is_pattern {
                            rusqlite::types::Value::Integer(i64::from(l.reduce(f, n)))
                        } else {
                            rusqlite::types::Value::Text(l.text(f, n))
                        })
                    },
                )
                .unwrap();
        }
        let execute = || {
            run::execute_program(
                connection,
                frames,
                &Graph::default(),
                std::slice::from_ref(&sql),
                false,
            )
        };
        let (result, count) = if diagnostic {
            profiling::frame_counts(execute)
        } else {
            (execute(), [0; 14])
        };
        let cold = direct_start.elapsed().plus(generation);
        let result = result.unwrap();
        let mut actual = vec![vec![]; sources.len()];
        for (_, file, id, value) in &result.hits {
            actual[*file].push(SqlMatch {
                root: *id,
                captures: vec![(
                    "N".into(),
                    Captured::Node(value[0].as_u64().unwrap() as u32),
                )],
            });
        }
        for (i, (got, want)) in actual.iter().zip(&native).enumerate() {
            assert_eq!(got, want, "direct/native: {}", sources[i].0);
        }
        assert_eq!(result.metrics["sqlExecutions"], 1);
        if diagnostic {
            assert_eq!(count[4], 0);
        }
        report["direct"] = json!({"coldKernelCpuNs":cold.cpu,"generationCpuNs":generation.cpu,"metrics":result.metrics,"counters":if diagnostic {counters(count)} else {Value::Null},"leafCalls":leaf_calls.load(Ordering::Relaxed)});
        if diagnostic {
            report["explain"] = super::profile::explain(connection, &sql);
            let prefix = sql
                .rsplit_once("SELECT 0,file,id,json_array(capture)")
                .unwrap()
                .0;
            let names = [
                "loops",
                "safe_lambda",
                "functions",
                "function_climb",
                "function_loops",
                "relevant_loops",
                "regions",
                "target_slot",
                "targets",
                "assignment_climb",
                "loop_assigns",
                "demand",
                "store_slot",
                "stores",
                "parameter_walk",
                "parameters",
                "unbound",
                "bodies",
                "loads",
                "matches",
            ];
            let count_sql = format!(
                "{prefix}{}",
                names
                    .iter()
                    .map(|name| format!("SELECT '{name}',COUNT(*) FROM {name}"))
                    .collect::<Vec<_>>()
                    .join(" UNION ALL ")
            );
            let counts: Vec<(String, i64)> = connection
                .prepare(&count_sql)
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            report["diagnosticRelationRows"] = json!(counts);
        }
    });
    report["parity"] = json!(true);
    if let Some(out) = out {
        std::fs::write(
            out.join("results.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        let outcomes:Vec<_>=native.iter().enumerate().flat_map(|(file,hits)|hits.iter().map(move|hit|json!({"file":file,"root":hit.root,"captures":format!("{:?}",hit.captures)}))).collect();
        std::fs::write(
            out.join("outcomes.json"),
            serde_json::to_vec_pretty(&outcomes).unwrap(),
        )
        .unwrap();
    }
    eprintln!(
        "B023 {} files {} matches: current {}ms direct {}ms / {} VM",
        sources.len(),
        report["matches"],
        report["baseline"]["coldKernelCpuNs"].as_u64().unwrap() / 1_000_000,
        report["direct"]["coldKernelCpuNs"].as_u64().unwrap() / 1_000_000,
        report["direct"]["metrics"]["vmSteps"]
    );
    report
}

#[test]
fn b023_direct_fixtures() {
    let paths = [
        "packs/python/upstream/ruff/fixtures/flake8_bugbear/B023.py",
        "packs/python/upstream/ruff/fixtures/rerail/noqa/B023.py",
    ];
    let sources = paths
        .iter()
        .map(|p| {
            (
                p.to_string(),
                std::fs::read_to_string(root().join(p)).unwrap(),
            )
        })
        .collect::<Vec<_>>();
    check(&sources, None, true);
}

#[test]
fn b023_direct_adversarial() {
    let snippets = [
        "for x in xs:\n def f():\n  return x\n",
        "for x in xs:\n def f(x=x):\n  return x\n def g():\n  x=0\n  return x\n",
        "for x in xs:\n def f():\n  def nested():\n   x=0\n  return x\n",
        "for x in xs:\n a=lambda: x\n b=(lambda: x)()\n c=map(lambda: x, xs)\n d=sorted(xs,key=(lambda: x))\n e=functools.reduce(lambda: x,xs)\n",
        "for x in xs:\n a=functools . reduce((lambda: x),xs)\n b=other.reduce(lambda: x,xs)\n c=filter((lambda: x),xs)\n",
        "for (x,[y,*z]) in xs:\n def f():\n  return x,y,z\n",
        "while xs:\n x,y=xs\n x+=1\n def f():\n  return x,y\n",
        "a=[lambda: x for x in xs]\nb={x:lambda: x for x in xs}\nc=(lambda: x for x in xs)\n",
        "for x in xs:\n def f(*x):\n  return x\n def g(**x):\n  return x\n def h(x:int=0):\n  return x\n",
        "for x in xs:\n def f():\n  with resource() as (x,y):\n   pass\n  return x,y\n",
        "for x in xs:\n def f():\n  a=(x:=1)\n  return x\n",
        "for x in xs:\n def f():\n  return obj.x, call(x=x), x\n",
        "for x in xs:\n def f():\n  global x\n  del x\n  return x\n",
        "for x in xs:\n def f():\n  def g():\n   return x\n  return g\n",
        "for x in xs:\n for x in ys:\n  def f():\n   return x+x+x\n",
        "x=0\ndef f():\n return x\n",
        "for x in xs:\n def f():\n  return lambda: x\n",
        "for x in xs:\n f=lambda y=(lambda: x): y\n",
        "for x in xs:\n f=lambda: missing(x,x)\nfor y in ys:\n f=lambda: x\n",
        "for café in xs:\n f=lambda: café\n",
        "for x in xs:\n def f():\n  match obj:\n   case Point(x=x): pass\n  return x\n",
        "for x in xs:\n def f():\n  try: pass\n  except Error as x: pass\n  return x\n",
    ];
    let sources = snippets
        .iter()
        .enumerate()
        .map(|(i, s)| (format!("adversarial-{i}.py"), s.to_string()))
        .collect::<Vec<_>>();
    check(&sources, None, true);
}

#[test]
fn b023_direct_scaling() {
    let mut reports = Vec::new();
    for family in ["irrelevant_assignments", "repeated_names"] {
        let mut first_vm = 0;
        for size in [32, 128, 512] {
            let source = if family == "irrelevant_assignments" {
                format!(
                    "{}for x in xs:\n def f():\n  return x\n",
                    (0..size)
                        .map(|i| format!("noise_{i}={i}\n"))
                        .collect::<String>()
                )
            } else {
                format!(
                    "for x in xs:\n{} def f():\n{}",
                    " x=0\n".repeat(size),
                    "  consume(x)\n".repeat(size)
                )
            };
            let mut report = check(&[(format!("{family}-{size}.py"), source)], None, true);
            let vm = report["direct"]["metrics"]["vmSteps"].as_u64().unwrap();
            if size == 32 {
                first_vm = vm;
            }
            assert!(
                vm <= first_vm * (size as u64 / 32) * 2,
                "superlinear SQL work for {family}: {report}"
            );
            let expected = if family == "repeated_names" { size } else { 1 };
            assert_eq!(report["matches"], expected);
            report["family"] = json!(family);
            report["size"] = json!(size);
            reports.push(report);
        }
    }
    if let Ok(path) = std::env::var("RERAIL_B023_SCALE_OUTPUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&reports).unwrap()).unwrap();
    }
}

#[test]
#[ignore = "set RERAIL_B023_CONFIG and RERAIL_B023_OUTPUT"]
fn b023_direct_corpus() {
    let config: Value = serde_json::from_str(
        &std::fs::read_to_string(std::env::var("RERAIL_B023_CONFIG").unwrap()).unwrap(),
    )
    .unwrap();
    let sources = config["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let p = p.as_str().unwrap();
            (
                p.to_owned(),
                std::fs::read_to_string(Path::new(config["corpus"].as_str().unwrap()).join(p))
                    .unwrap(),
            )
        })
        .collect::<Vec<_>>();
    check(
        &sources,
        Some(Path::new(&std::env::var("RERAIL_B023_OUTPUT").unwrap())),
        std::env::var_os("RERAIL_SQL_PROFILE_DIR").is_some(),
    );
}
