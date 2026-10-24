//! Executable result-only experiment. This is not an alternative policy engine.
//! Facts below are the normalized B615 callee/import/revision/local-path family.
//! SQL owns alias closure and witness reduction; the sole callback counts atomic
//! row visits. Work, native memo probes, recursion depth, following contexts and
//! three-valued capture withholding are deliberately outside this contract.
use rusqlite::{Connection, StatementStatus, functions::FunctionFlags};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
#[derive(Default)]
struct Counter(AtomicU64);
impl Counter {
    fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
    fn set(&self, value: u64) {
        self.0.store(value, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy, Debug)]
enum Shape {
    Correlated,
    SharedScalar,
    SharedJoin,
    SharedNotMaterialized,
}
impl Shape {
    fn name(self) -> &'static str {
        match self {
            Self::Correlated => "correlated",
            Self::SharedScalar => "shared-scalar",
            Self::SharedJoin => "shared-join",
            Self::SharedNotMaterialized => "not-materialized",
        }
    }
}
#[derive(Default)]
struct ObservableContract {
    work: bool,
    probes: bool,
    depth: bool,
    following: bool,
    unknown: bool,
}
fn transfer(contract: &ObservableContract) -> Result<(), &'static str> {
    if contract.work || contract.probes || contract.depth || contract.following || contract.unknown
    {
        Err(
            "result-only producer cannot preserve ordered work, memo probes, depth faults, binding following, or unknown rollback",
        )
    } else {
        Ok(())
    }
}
const KEYS: &str = "SELECT DISTINCT host,scope,node,typeof(live) cap_type,live FROM demand";
// The capture key has both SQLite storage class and value. Logical memo identity
// is deliberately absent here; it remains on every demand/output row.
fn query(shape: Shape) -> String {
    let witness = |alias: &str| {
        let local = if matches!(shape, Shape::Correlated) {
            format!(
                "WITH RECURSIVE reach(host,scope,node,cap_type,live,target) AS (SELECT {alias}.host,{alias}.scope,{alias}.node,typeof({alias}.live),{alias}.live,{alias}.node UNION SELECT r.host,r.scope,r.node,r.cap_type,r.live,e.target FROM reach r JOIN edge e ON e.host=r.host AND e.scope=r.scope AND e.source=r.target) "
            )
        } else {
            String::new()
        };
        format!(
            "(CASE WHEN produce({alias}.node)>0 THEN ({local}SELECT a.id FROM reach r JOIN api a ON a.host=r.host AND a.scope=r.scope AND a.node=r.target WHERE r.host={alias}.host AND r.scope={alias}.scope AND r.node={alias}.node AND r.cap_type=typeof({alias}.live) AND r.live IS {alias}.live AND a.callee IN ('from_pretrained','hf_hub_download') AND a.import_text IN ('transformers','huggingface_hub') AND a.revision_kind='string' AND NOT (length(a.revision_text)>=7 AND a.revision_text NOT GLOB '*[^0-9a-fA-F]*') AND substr(a.first_argument,1,1)<>'/' AND substr(a.first_argument,1,2)<>'./' AND substr(a.first_argument,1,3)<>'../' AND typeof(a.capture)=typeof({alias}.live) AND a.capture IS {alias}.live AND visit(a.id)>0 ORDER BY a.id LIMIT 1) END)"
        )
    };
    let common = format!(
        "keys AS MATERIALIZED ({KEYS}), reach(host,scope,node,cap_type,live,target) AS (SELECT host,scope,node,cap_type,live,node FROM keys UNION SELECT r.host,r.scope,r.node,r.cap_type,r.live,e.target FROM reach r JOIN edge e ON e.host=r.host AND e.scope=r.scope AND e.source=r.target)"
    );
    let output = match shape {
        Shape::Correlated => format!(
            "SELECT d.id,d.logical,d.live,{} witness FROM demand d",
            witness("d")
        ),
        Shape::SharedScalar | Shape::SharedJoin | Shape::SharedNotMaterialized => {
            let materialized = if matches!(shape, Shape::SharedNotMaterialized) {
                "NOT MATERIALIZED"
            } else {
                "MATERIALIZED"
            };
            let key = "p.host=d.host AND p.scope=d.scope AND p.node=d.node AND p.cap_type=typeof(d.live) AND p.live IS d.live";
            let output = if matches!(shape, Shape::SharedJoin) {
                format!(
                    "SELECT d.id,d.logical,d.live,p.witness FROM demand d JOIN producer p ON {key}"
                )
            } else {
                format!(
                    "SELECT d.id,d.logical,d.live,(SELECT p.witness FROM producer p WHERE {key}) witness FROM demand d"
                )
            };
            format!(
                "producer AS {materialized} (SELECT k.*,{} witness FROM keys k) SELECT * FROM ({output})",
                witness("k")
            )
        }
    };
    if matches!(shape, Shape::Correlated) {
        format!(
            "WITH RECURSIVE {common} SELECT id,logical,typeof(live),hex(live),witness FROM ({output}) ORDER BY id"
        )
    } else {
        format!(
            "WITH RECURSIVE {common}, {} ORDER BY id",
            output.replace(
                "SELECT * FROM (",
                "SELECT id,logical,typeof(live),hex(live),witness FROM ("
            )
        )
    }
}

#[derive(Default)]
struct Counters {
    atoms: Counter,
    producers: Counter,
}
fn database(
    files: usize,
    callers: usize,
    diversity: usize,
    recursive: bool,
) -> (Connection, Arc<Counters>) {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE demand(id INTEGER PRIMARY KEY,logical INTEGER,host INTEGER,scope INTEGER,node INTEGER,live,irrelevant TEXT); CREATE TABLE edge(host INTEGER,scope INTEGER,source INTEGER,target INTEGER,PRIMARY KEY(host,scope,source,target)); CREATE TABLE api(id INTEGER PRIMARY KEY,host INTEGER,scope INTEGER,node INTEGER,capture,callee TEXT,import_text TEXT,revision_kind TEXT,revision_text TEXT,first_argument TEXT); CREATE INDEX api_key ON api(host,scope,node,capture);").unwrap();
    let visits = Arc::new(Counters::default());
    let counter = visits.clone();
    db.create_scalar_function("visit", 1, FunctionFlags::SQLITE_UTF8, move |ctx| {
        counter.atoms.set(counter.atoms.get() + 1);
        ctx.get::<i64>(0)
    })
    .unwrap();
    let producer = visits.clone();
    db.create_scalar_function("produce", 1, FunctionFlags::SQLITE_UTF8, move |ctx| {
        producer.producers.set(producer.producers.get() + 1);
        ctx.get::<i64>(0)
    })
    .unwrap();
    let mut id = 0_i64;
    for host in 0..files {
        for scope in 0..2 {
            for capture in 0..diversity {
                let live = format!("renamed_api_{capture}");
                for caller in 0..callers {
                    id += 1;
                    db.execute(
                        "INSERT INTO demand VALUES (?1,?2,?3,?4,1,?5,?6)",
                        rusqlite::params![
                            id,
                            (caller % 3) as i64,
                            host as i64,
                            scope,
                            live,
                            format!("irrelevant_{caller}")
                        ],
                    )
                    .unwrap();
                }
                // Positive, renamed positive, and mutated controls: unsupported API,
                // absent import, nonliteral revision, pinned revision, local first arg.
                for variant in 0..7 {
                    let api_id =
                        (host * 100000 + scope as usize * 10000 + capture * 10 + variant + 1)
                            as i64;
                    let node = if recursive { 3 } else { 1 };
                    db.execute(
                        "INSERT INTO api VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                        rusqlite::params![
                            api_id,
                            host as i64,
                            scope,
                            node,
                            live,
                            if variant == 2 {
                                "mutated_download"
                            } else if variant == 1 {
                                "hf_hub_download"
                            } else {
                                "from_pretrained"
                            },
                            if variant == 3 {
                                "unrelated_module"
                            } else {
                                "transformers"
                            },
                            if variant == 4 { "identifier" } else { "string" },
                            if variant == 5 { "abcdef123" } else { "main" },
                            if variant == 6 {
                                "./local"
                            } else {
                                "organization/model"
                            }
                        ],
                    )
                    .unwrap();
                }
            }
            if recursive {
                db.execute(
                    "INSERT INTO edge VALUES (?1,?2,1,2),(?1,?2,2,3)",
                    rusqlite::params![host as i64, scope],
                )
                .unwrap();
            }
        }
    }
    (db, visits)
}
type OutputRow = (i64, i64, String, String, Option<i64>);
fn run(db: &Connection, visits: &Counters, shape: Shape) -> (Vec<OutputRow>, Value) {
    visits.atoms.set(0);
    visits.producers.set(0);
    let sql = query(shape);
    let start = std::time::Instant::now();
    let mut stmt = db.prepare(&sql).unwrap();
    let prepare = start.elapsed().as_nanos();
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let keys: i64 = db
        .query_row(&format!("SELECT COUNT(*) FROM ({KEYS})"), [], |r| r.get(0))
        .unwrap();
    let report = json!({"shape":shape.name(),"rows":rows.len(),"distinctKeys":keys,"producerAtomicVisits":visits.atoms.get(),"producerEvaluations":visits.producers.get(),"vmSteps":stmt.get_status(StatementStatus::VmStep),"sqlBytes":sql.len(),"prepareNs":prepare,"contract":"result-and-capture-only; observable native operations not equivalent"});
    (rows, report)
}

#[test]
fn captured_demand_materialization_matrix() {
    let policy: Value = serde_json::from_str(include_str!(
        "../../../../../../packs/python/rules/bandit-b615-huggingface-unsafe-download.json"
    ))
    .unwrap();
    let utils = &policy["clauses"][0]["forbid"]["ast"]["utils"];
    for name in [
        "hf-callee",
        "nonconstant-revision",
        "pinned-revision",
        "local-path-first-argument",
        "after-hf-import",
    ] {
        assert!(utils.get(name).is_some(), "B615 family drift: {name}");
    }
    let mut evidence = vec![];
    for files in [1, 4, 16] {
        for callers in [1, 2, 4, 8, 32] {
            for diversity in [1, 4] {
                for recursive in [false, true] {
                    let (db, visits) = database(files, callers, diversity, recursive);
                    let mut oracle = None;
                    for shape in [
                        Shape::Correlated,
                        Shape::SharedScalar,
                        Shape::SharedJoin,
                        Shape::SharedNotMaterialized,
                    ] {
                        let (rows, mut report) = run(&db, &visits, shape);
                        assert!(rows.iter().all(|r| r.4.is_some()));
                        if let Some(expected) = &oracle {
                            assert_eq!(&rows, expected);
                        } else {
                            oracle = Some(rows);
                        }
                        if matches!(shape, Shape::SharedScalar | Shape::SharedJoin) {
                            assert!(
                                report["producerAtomicVisits"].as_u64().unwrap()
                                    <= (files * 2 * diversity * 2) as u64,
                                "one early reduced producer per key"
                            );
                            assert_eq!(report["producerEvaluations"], report["distinctKeys"]);
                        }
                        report["files"] = json!(files);
                        report["callers"] = json!(callers);
                        report["captureDiversity"] = json!(diversity);
                        report["recursiveAliases"] = json!(recursive);
                        evidence.push(report);
                    }
                }
            }
        }
    }
    if let Ok(path) = std::env::var("RERAIL_DEMAND_RELATION_SPIKE_EVIDENCE") {
        std::fs::write(path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    }
}

#[test]
fn typed_live_capture_host_scope_and_logical_identity_are_independent() {
    let (db, visits) = database(1, 8, 1, true);
    db.execute_batch("INSERT INTO demand VALUES(100,90,99,0,1,'renamed_api_0','same'),(101,91,0,99,1,'renamed_api_0','same'),(102,92,0,0,1,'different','same'),(103,93,0,0,1,x'72656e616d65645f6170695f30','same');").unwrap();
    let mut oracle = None;
    for shape in [
        Shape::Correlated,
        Shape::SharedScalar,
        Shape::SharedJoin,
        Shape::SharedNotMaterialized,
    ] {
        let (rows, _) = run(&db, &visits, shape);
        assert_eq!(rows.iter().filter(|r| r.4.is_none()).count(), 4);
        assert!(rows.iter().filter(|r| r.0 < 100).all(|r| r.4.is_some()));
        if let Some(expected) = &oracle {
            assert_eq!(&rows, expected)
        } else {
            oracle = Some(rows)
        }
    }
}
#[test]
fn full_b615_transfer_refuses_unpreserved_observables() {
    assert!(transfer(&ObservableContract::default()).is_ok());
    for contract in [
        ObservableContract {
            work: true,
            ..Default::default()
        },
        ObservableContract {
            probes: true,
            ..Default::default()
        },
        ObservableContract {
            depth: true,
            ..Default::default()
        },
        ObservableContract {
            following: true,
            ..Default::default()
        },
        ObservableContract {
            unknown: true,
            ..Default::default()
        },
    ] {
        assert!(transfer(&contract).is_err());
    }
    let (db, visits) = database(1, 8, 1, true);
    let (_, per) = run(&db, &visits, Shape::Correlated);
    let (_, shared) = run(&db, &visits, Shape::SharedJoin);
    assert!(
        per["producerAtomicVisits"].as_u64().unwrap()
            > shared["producerAtomicVisits"].as_u64().unwrap(),
        "sharing changes observable operations even when answers agree"
    );
}

#[test]
fn extracted_b615_revision_keyword_matches_native_ast_and_capture() {
    use crate::code_rule_dsl::{
        CodeRuleDslRuntime,
        language::GrammarDialect,
        matcher,
        sql::tests::differential::tape,
        test_support::{CAPABILITIES, grammar_matchers},
    };
    use ast_grep_core::AstGrep;
    let policy: Value = serde_json::from_str(include_str!(
        "../../../../../../packs/python/rules/bandit-b615-huggingface-unsafe-download.json"
    ))
    .unwrap();
    let original = &policy["clauses"][0]["forbid"]["ast"]["utils"]["revision-keyword"];
    for utility in ["revision-keyword", "renamed_keyword"] {
        let rule = json!({"all":[{"matches":utility},{"has":{"field":"name","pattern":"$CAP"}}]});
        let mut utils = serde_json::Map::new();
        utils.insert(utility.into(), original.clone());
        let p=CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"extracted-b615","revision":1,"mode":"steer","clauses":[{"id":"c","forbid":{"ast":{"language":"python","rule":rule,"utils":utils}},"violation":{"code":"C","message":"match","anchor":"witness.span"}}]}).to_string(),"json".into(),CAPABILITIES.into()).unwrap();
        let grammar = &grammar_matchers(&p)[0];
        let source =
            "load(revision='main', commit_id='deadbee', revised='main')\nrenamed(revision=value)\n";
        let doc = tape(GrammarDialect::Python, source);
        let tree = AstGrep::doc(doc.clone());
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE ast(id INTEGER PRIMARY KEY,kind TEXT,text TEXT);CREATE TABLE field(parent INTEGER,name TEXT,child INTEGER);CREATE TABLE callers(logical INTEGER,node INTEGER,ignored TEXT);").unwrap();
        for node in tree.root().dfs() {
            db.execute(
                "INSERT INTO ast VALUES(?1,?2,?3)",
                rusqlite::params![
                    node.node_id() as i64,
                    node.kind().as_ref(),
                    node.text().as_ref()
                ],
            )
            .unwrap();
            if let Some(name) = node.field("name") {
                db.execute(
                    "INSERT INTO field VALUES(?1,'name',?2)",
                    rusqlite::params![node.node_id() as i64, name.node_id() as i64],
                )
                .unwrap();
            }
            if node.kind() == "keyword_argument" {
                for caller in 0..8 {
                    db.execute(
                        "INSERT INTO callers VALUES(?1,?2,?3)",
                        rusqlite::params![caller, node.node_id() as i64, format!("unused{caller}")],
                    )
                    .unwrap();
                }
            }
        }
        let mut native = vec![];
        let mut work = usize::MAX;
        let _scopes =
            matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
        matcher::with_file_memo(&tree, || {
            matcher::run_clause(
                &tree,
                grammar,
                None,
                doc.transducer_len(),
                &mut work,
                |root, env, _| {
                    native.push((
                        i64::from(root),
                        env.get_match("CAP").unwrap().node_id() as i64,
                    ));
                    Ok::<_, ()>(true)
                },
            )
            .unwrap()
            .unwrap_or_else(|_| panic!("native helper stopped"))
        });
        native.sort_unstable();
        assert_eq!(native.len(), 2);
        // SQL composes the extracted helper directly from raw kind/text/field
        // facts, preserving both original root and real captured AST node IDs.
        let sql = "WITH keys AS MATERIALIZED(SELECT DISTINCT node FROM callers), producer AS MATERIALIZED(SELECT k.node,f.child capture FROM keys k JOIN ast a ON a.id=k.node JOIN field f ON f.parent=a.id AND f.name='name' JOIN ast n ON n.id=f.child WHERE a.kind='keyword_argument' AND n.text='revision') SELECT DISTINCT p.node,p.capture FROM callers c JOIN producer p ON p.node=c.node ORDER BY p.node";
        let rows = db
            .prepare(sql)
            .unwrap()
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(rows, native);
    }
}

#[test]
fn atomic_text_mutations_and_recursive_cycle_controls() {
    for variant in 0..7 {
        let (db, visits) = database(1, 8, 1, true);
        db.execute("DELETE FROM api WHERE ((id-1)%10)<>?1", [variant])
            .unwrap();
        db.execute_batch("INSERT INTO edge VALUES(0,0,3,2),(0,1,3,2);")
            .unwrap();
        let mut oracle = None;
        for shape in [
            Shape::Correlated,
            Shape::SharedScalar,
            Shape::SharedJoin,
            Shape::SharedNotMaterialized,
        ] {
            let (rows, _) = run(&db, &visits, shape);
            assert!(
                rows.iter().all(|r| r.4.is_some() == (variant < 2)),
                "mutation {variant}, {shape:?}"
            );
            if let Some(expected) = &oracle {
                assert_eq!(&rows, expected)
            } else {
                oracle = Some(rows)
            }
        }
    }
}

fn membership_query(join: bool) -> String {
    let sql = query(if join {
        Shape::SharedJoin
    } else {
        Shape::SharedScalar
    });
    if join {
        sql.replace(
            "d.live,p.witness FROM demand d",
            "d.live,(p.witness IS NOT NULL) witness FROM demand d",
        )
    } else {
        let key = "p.host=d.host AND p.scope=d.scope AND p.node=d.node AND p.cap_type=typeof(d.live) AND p.live IS d.live";
        sql.replace(
            &format!("(SELECT p.witness FROM producer p WHERE {key}) witness"),
            &format!(
                "EXISTS(SELECT 1 FROM producer p WHERE {key} AND p.witness IS NOT NULL) witness"
            ),
        )
    }
}
#[test]
fn materialized_boolean_exists_and_join_membership_matrix() {
    let mut evidence = vec![];
    for files in [1, 4, 16] {
        for callers in [1, 2, 4, 8, 32] {
            for diversity in [1, 4] {
                for recursive in [false, true] {
                    let (db, visits) = database(files, callers, diversity, recursive);
                    db.execute_batch(
                        "INSERT INTO demand VALUES(9000001,101,999,0,1,'missing','ignored');",
                    )
                    .unwrap();
                    let (witnesses, _) = run(&db, &visits, Shape::SharedScalar);
                    let expected = witnesses
                        .into_iter()
                        .map(|(id, logical, kind, bytes, w)| {
                            (id, logical, kind, bytes, Some(i64::from(w.is_some())))
                        })
                        .collect::<Vec<_>>();
                    for join in [false, true] {
                        visits.atoms.set(0);
                        visits.producers.set(0);
                        let sql = membership_query(join);
                        assert_eq!(sql.contains("EXISTS(SELECT 1 FROM producer"), !join);
                        let mut stmt = db.prepare(&sql).unwrap();
                        let rows = stmt
                            .query_map([], |r| {
                                Ok((
                                    r.get::<_, i64>(0)?,
                                    r.get::<_, i64>(1)?,
                                    r.get::<_, String>(2)?,
                                    r.get::<_, String>(3)?,
                                    r.get::<_, Option<i64>>(4)?,
                                ))
                            })
                            .unwrap()
                            .collect::<Result<Vec<_>, _>>()
                            .unwrap();
                        assert_eq!(rows, expected);
                        let keys: i64 = db
                            .query_row(&format!("SELECT COUNT(*) FROM ({KEYS})"), [], |r| r.get(0))
                            .unwrap();
                        assert_eq!(visits.producers.get(), keys as u64);
                        evidence.push(json!({"shape":if join{"boolean-join"}else{"boolean-exists"},"files":files,"callers":callers,"captureDiversity":diversity,"recursiveAliases":recursive,"rows":rows.len(),"keys":keys,"producerEvaluations":visits.producers.get(),"producerAtomicVisits":visits.atoms.get(),"vmSteps":stmt.get_status(StatementStatus::VmStep),"sqlBytes":sql.len(),"contract":"boolean membership only, not witness return or native operation parity"}));
                    }
                }
            }
        }
    }
    if let Ok(path) = std::env::var("RERAIL_DEMAND_RELATION_SPIKE_EVIDENCE") {
        std::fs::write(
            format!("{path}.membership.json"),
            serde_json::to_vec_pretty(&evidence).unwrap(),
        )
        .unwrap();
    }
}
#[test]
fn projected_sql_storage_keys_preserve_null_empty_and_binary_values() {
    let (db, visits) = database(1, 1, 1, false);
    db.execute_batch("DELETE FROM demand;DELETE FROM api;
      INSERT INTO demand VALUES(1,1,0,0,1,NULL,'one'),(2,2,0,0,1,NULL,'two'),(3,3,0,0,1,'','one'),(4,4,0,0,1,x'','two'),(5,5,0,0,1,x'0001','one'),(6,6,0,0,1,1,'two');
      INSERT INTO api VALUES(1,0,0,1,NULL,'from_pretrained','transformers','string','main','org/model'),(2,0,0,1,'','from_pretrained','transformers','string','main','org/model'),(3,0,0,1,x'','from_pretrained','transformers','string','main','org/model'),(4,0,0,1,x'0001','from_pretrained','transformers','string','main','org/model');").unwrap();
    let mut oracle = None;
    for shape in [
        Shape::Correlated,
        Shape::SharedScalar,
        Shape::SharedJoin,
        Shape::SharedNotMaterialized,
    ] {
        let (rows, _) = run(&db, &visits, shape);
        assert_eq!(
            rows.iter().map(|r| r.4).collect::<Vec<_>>(),
            vec![Some(1), Some(1), Some(2), Some(3), Some(4), None]
        );
        if let Some(expected) = &oracle {
            assert_eq!(&rows, expected)
        } else {
            oracle = Some(rows)
        }
    }
    let keys: i64 = db
        .query_row(&format!("SELECT COUNT(*) FROM ({KEYS})"), [], |r| r.get(0))
        .unwrap();
    assert_eq!(keys, 5);
    // This storage-class experiment has no representation for distinct engine
    // Node/Count integers, list nodes, or spans with optional node shadows.
    let accepts_engine_cell = |variant: &str| {
        if variant == "scalar-storage" {
            Ok(())
        } else {
            Err("full engine Cell discriminant and payload are required before sharing")
        }
    };
    assert!(accepts_engine_cell("scalar-storage").is_ok());
    for variant in [
        "node",
        "count",
        "node-list",
        "span",
        "span-with-node-shadow",
    ] {
        assert!(accepts_engine_cell(variant).is_err());
    }
}
