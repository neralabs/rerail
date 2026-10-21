use super::variants::{Case, Variant, variants};
use crate::code_rule_dsl::sql::{profiling, terminals::calls, timing::Started};
use rusqlite::{Connection, Result, StatementStatus, types::ValueRef};
use serde_json::{Value, json};

fn terminals() -> [usize; 3] {
    [
        calls::PATTERN_FILTERS.get(),
        calls::FACT_FILTERS.get(),
        calls::FRESH_MATCHES.get(),
    ]
}

fn rows(connection: &Connection, sql: &str) -> (Result<Vec<String>>, Value) {
    let preparation = Started::now();
    let statement = connection.prepare(sql);
    let prepare = preparation.elapsed();
    let Ok(mut statement) = statement else {
        return (
            Err(statement.err().unwrap()),
            json!({"prepareCpuNs":prepare.cpu}),
        );
    };
    // Bound deliberately poor plans. This is not a timeout or an engine work-budget change.
    let mut polls = 0;
    let budget = Started::now();
    connection
        .progress_handler(
            10_000,
            Some(move || {
                polls += 1;
                polls > 10_000 || budget.elapsed().cpu > 1_000_000_000
            }),
        )
        .expect("installs replay instruction limit");
    let before = terminals();
    let started = Started::now();
    let (result, frame) = profiling::frame_counts(|| {
        let columns = statement.column_count();
        let mut result = Vec::new();
        let mut cursor = statement.query([])?;
        while let Some(row) = cursor.next()? {
            let mut values = Vec::with_capacity(columns);
            for column in 0..columns {
                values.push(match row.get_ref(column)? {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(n) => json!({"integer":n}),
                    ValueRef::Real(n) => json!({"realBits":n.to_bits()}),
                    ValueRef::Text(s) => json!({"textBytes":s}),
                    ValueRef::Blob(s) => json!({"blob":s}),
                });
            }
            result.push(serde_json::to_string(&values).unwrap());
        }
        Ok::<_, rusqlite::Error>(result)
    });
    let elapsed = started.elapsed();
    let after = terminals();
    connection
        .progress_handler(0, None::<fn() -> bool>)
        .expect("removes replay instruction limit");
    let metrics = json!({"cpuNs":elapsed.cpu,"wallNs":elapsed.wall,"prepareCpuNs":prepare.cpu,"prepareWallNs":prepare.wall,
        "vmSteps":statement.get_status(StatementStatus::VmStep),"fullscanSteps":statement.get_status(StatementStatus::FullscanStep),
        "sorts":statement.get_status(StatementStatus::Sort),"autoIndexRows":statement.get_status(StatementStatus::AutoIndex),
        "frameScans":frame[0],"frameLookups":frame[1],"frameNext":frame[2],"frameColumns":frame[3],
        "frameIndexBuilds":frame[5],"frameIndexedRows":frame[6],"frameIndexCpuNs":frame[7],
        "patternFilters":after[0]-before[0],"factFilters":after[1]-before[1],"freshMatches":after[2]-before[2]});
    (result, metrics)
}

fn plan(connection: &Connection, sql: &str) -> Value {
    let result = (|| {
        let mut statement = connection.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
        statement
            .query_map([], |row| row.get::<_, String>(3))?
            .collect::<Result<Vec<_>>>()
    })();
    result.map_or_else(|e| json!({"error":e.to_string()}), |p| json!(p))
}

fn command(connection: &Connection, sql: &str) -> Value {
    let started = Started::now();
    let result = connection.execute_batch(sql);
    let elapsed = started.elapsed();
    json!({"sql":sql,"cpuNs":elapsed.cpu,"wallNs":elapsed.wall,"error":result.err().map(|e|e.to_string())})
}

fn stats(connection: &Connection, schema: &str) -> Vec<(String, Option<String>, String)> {
    connection
        .prepare(&format!("SELECT tbl,idx,stat FROM {schema}.sqlite_stat1"))
        .and_then(|mut s| {
            s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect()
        })
        .unwrap_or_default()
}

fn restore(
    connection: &Connection,
    schema: &str,
    stats: &[(String, Option<String>, String)],
) -> Result<()> {
    if connection
        .prepare(&format!("SELECT * FROM {schema}.sqlite_stat1"))
        .is_err()
    {
        return Ok(());
    }
    connection.execute(&format!("DELETE FROM {schema}.sqlite_stat1"), [])?;
    for (table, index, stat) in stats {
        connection.execute(
            &format!("INSERT INTO {schema}.sqlite_stat1 VALUES (?1,?2,?3)"),
            rusqlite::params![table, index, stat],
        )?;
    }
    connection.execute_batch(&format!("ANALYZE {schema}.sqlite_schema"))
}

fn sample(
    connection: &Connection,
    variant: &Variant,
    oracle: &[String],
    ordered_oracle: &[String],
    round: usize,
) -> Value {
    let (actual, mut metrics) = rows(connection, &variant.sql);
    match actual {
        Ok(mut actual) => {
            metrics["orderedParity"] = json!(actual == ordered_oracle);
            actual.sort();
            metrics["rows"] = json!(actual.len());
            metrics["parity"] = json!(actual == oracle);
        }
        Err(error) => {
            metrics["error"] = json!(error.to_string());
            metrics["parity"] = Value::Null;
        }
    }
    json!({"variant":variant.name,"round":if round==usize::MAX {Value::Null} else {json!(round)},"metrics":metrics})
}

pub(super) fn replay(connection: &Connection, case: &Case, rounds: usize) -> Result<Value> {
    let (oracle, _) = rows(connection, &case.select);
    let mut oracle = oracle?;
    let ordered_oracle = oracle.clone();
    oracle.sort();
    let original_main = stats(connection, "main");
    let original_temp = stats(connection, "temp");
    let mut setup = Vec::new();
    // Separate copies keep the unanalyzed table free of statistics during interleaved rounds.
    for table in ["shape_raw", "shape_stats"] {
        let sql = format!(
            "CREATE TEMP TABLE {table} AS SELECT * FROM {}; CREATE INDEX {table}_at ON {table}(file,node); CREATE INDEX {table}_k ON {table}(k)",
            case.input
        );
        setup.push(command(connection, &sql));
    }
    setup.push(command(connection, "ANALYZE temp.shape_stats"));
    let mut choices = variants(case);
    for (name, table) in [
        ("btree-unanalyzed", "shape_raw"),
        ("btree-analyzed", "shape_stats"),
    ] {
        choices.push(Variant {
            name,
            sql: case
                .select
                .replace(&format!("FROM {}", case.input), &format!("FROM {table}")),
        });
    }
    let mut samples = Vec::new();
    // Every alternative sees warm terminal caches and warm frame indexes before timing.
    // The first run also establishes which variants preserve complete projected rows.
    for variant in &choices {
        samples.push(sample(
            connection,
            variant,
            &oracle,
            &ordered_oracle,
            usize::MAX,
        ));
    }
    let plans: Vec<_> = choices
        .iter()
        .map(|v| json!({"variant":v.name,"sql":v.sql,"plan":plan(connection,&v.sql)}))
        .collect();
    for round in 0..rounds {
        let order: Vec<_> = if round % 2 == 0 {
            choices.iter().collect()
        } else {
            choices.iter().rev().collect()
        };
        for variant in order {
            samples.push(sample(connection, variant, &oracle, &ordered_oracle, round));
        }
    }
    let copy_stats = stats(connection, "temp");
    let mut analyze = Vec::new();
    for (name, sql) in [
        ("analyze-virtual-input", format!("ANALYZE {}", case.input)),
        ("analyze-node", "ANALYZE node".to_owned()),
        ("analyze-main", "ANALYZE main".to_owned()),
        ("analyze-temp", "ANALYZE temp".to_owned()),
        ("pragma-optimize", "PRAGMA optimize=0x10002".to_owned()),
    ] {
        let setup = command(connection, &sql);
        let variant = Variant {
            name,
            sql: case.select.clone(),
        };
        let plan = plan(connection, &variant.sql);
        let mut runs = Vec::new();
        for round in 0..rounds {
            runs.push(sample(
                connection,
                &variant,
                &oracle,
                &ordered_oracle,
                round,
            ));
        }
        analyze.push(
            json!({"variant":name,"setup":setup,"plan":plan,"samples":runs,
            "mainStats":stats(connection,"main"),"tempStats":stats(connection,"temp")}),
        );
    }
    connection.execute_batch("DROP TABLE shape_raw; DROP TABLE shape_stats")?;
    restore(connection, "main", &original_main)?;
    restore(connection, "temp", &original_temp)?;
    Ok(
        json!({"oracleRows":oracle.len(),"setup":setup,"plans":plans,"samples":samples,
        "copyStats":copy_stats,"analyze":analyze,"stat1Restored":stats(connection,"main")==original_main && stats(connection,"temp")==original_temp}),
    )
}

#[test]
fn alternatives_preserve_duplicate_rows_captures_null_keys_and_restore_statistics() {
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::sql::{
        exec::with_nodes, frames::FrameStore, tests::differential::tape,
    };
    use std::sync::Arc;
    let doc = tape(
        GrammarDialect::TypeScript,
        "let x = 1;\nf(a);\nf(b);\n{ f(c); f(d); }\nf(e);\n",
    );
    with_nodes(vec![doc], |connection, batch| {
        let frames = Arc::new(FrameStore::default());
        frames
            .register(
                connection,
                "f0_0(k INTEGER PRIMARY KEY, file INTEGER, node INTEGER, s0 INTEGER)",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO f0_0 SELECT NULL,file,id,id FROM node WHERE named=1",
                [],
            )
            .unwrap();
        connection
            .execute("INSERT INTO f0_0 VALUES (-1,0,0,NULL),(-2,0,0,NULL)", [])
            .unwrap();
        let kind = batch.files[0]
            .root
            .root()
            .dfs()
            .find(|n| n.kind() == "expression_statement")
            .unwrap()
            .kind_id();
        let sql = format!(
            "INSERT INTO f1_0 SELECT NULL,c.file,c.node,c.s0,c.pk,-c.node FROM (SELECT f.k AS pk, c.file AS file, c.id AS node, f.s0 FROM f0_0 f JOIN node fn ON fn.file = f.file AND fn.id = f.node JOIN node c ON c.file = f.file AND c.parent = fn.wpar AND c.id <= fn.aprev AND c.kind IN ({kind})) c"
        );
        let case = super::variants::classify(&sql).unwrap();
        let report = replay(connection, &case, 1).unwrap();
        for sample in report["samples"].as_array().unwrap() {
            assert_eq!(sample["metrics"]["parity"], true, "{sample}");
        }
        assert_eq!(report["stat1Restored"], true);
        assert!(report["analyze"][0]["setup"]["error"].is_null());
        for phase in report["analyze"].as_array().unwrap() {
            for schema in ["mainStats", "tempStats"] {
                assert!(
                    phase[schema]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|row| row[0] != "f0_0" && row[0] != "node")
                );
            }
        }
        connection.execute_batch("CREATE TEMP TABLE c0_d0(file INTEGER,node INTEGER,c0 INTEGER); CREATE INDEX c0_at ON c0_d0(file,node,c0); INSERT INTO c0_d0 VALUES (0,0,NULL)").unwrap();
        let sql = "INSERT INTO c0_d0(file, node, c0) SELECT DISTINCT f.file, f.node, COALESCE((SELECT atom FROM node WHERE file = f.file AND id = f.s0), f.s0) FROM f0_0 f WHERE NOT EXISTS (SELECT 1 FROM c0_d0 d WHERE d.file = f.file AND d.node = f.node AND d.c0 IS COALESCE((SELECT atom FROM node WHERE file = f.file AND id = f.s0), f.s0))";
        let case = super::variants::classify(sql).unwrap();
        let report = replay(connection, &case, 1).unwrap();
        for sample in report["samples"].as_array().unwrap() {
            assert_eq!(sample["metrics"]["parity"], true, "{sample}");
        }
        for phase in report["analyze"].as_array().unwrap() {
            for sample in phase["samples"].as_array().unwrap() {
                assert_eq!(sample["metrics"]["parity"], true, "{sample}");
            }
        }
        assert_eq!(report["stat1Restored"], true);
    });
}
