//! Architectural experiments only. Production retains a single execution path.
mod native;
mod publication;
#[cfg(test)]
mod tests;

use crate::composition::{Meter, code, cpu_ms, facts, peak_bytes, vocabulary};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn demand(db: &Connection, scale: usize) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TEMP TABLE demand(scope INTEGER PRIMARY KEY,fid INTEGER,lo INTEGER,hi INTEGER) WITHOUT ROWID")?;
    // A descriptor is indivisible. Overlapping scopes and equal preorders in different files
    // are intentionally retained. Scale zero tests the empty admission gate.
    db.execute("WITH RECURSIVE ids(id) AS (SELECT 0 WHERE ?1>0 UNION ALL SELECT id+1 FROM ids WHERE id+1<?1) INSERT INTO demand SELECT id,fid,CASE WHEN id%4=3 THEN 100 ELSE 0 END,CASE WHEN id%4=3 THEN min(nodes,4000) ELSE nodes END FROM ids JOIN file ON fid=id%3+1",[scale as i64])?;
    Ok(())
}

fn arm(v: &BTreeMap<String, i64>, which: &str) -> String {
    let function = code(v, "kind:function_definition");
    let identifier = code(v, "kind:identifier");
    let field = code(v, "field:name");
    match which {
        "parent" => format!(
            "SELECT d.scope,p.fid,p.pre,c.pre AS child FROM demand d CROSS JOIN n p INDEXED BY n_kind CROSS JOIN n c INDEXED BY n_parent WHERE p.kind={function} AND p.fid=d.fid AND p.pre>=d.lo AND p.pre<d.hi AND c.fid=p.fid AND c.parent=p.pre AND c.field={field} AND c.kind={identifier}"
        ),
        "child" => format!(
            "SELECT d.scope,p.fid,p.pre,c.pre AS child FROM demand d CROSS JOIN n c INDEXED BY n_kind CROSS JOIN n p WHERE c.kind={identifier} AND c.fid=d.fid AND c.pre>=d.lo AND p.fid=c.fid AND p.pre=c.parent AND p.kind={function} AND p.pre>=d.lo AND p.pre<d.hi AND c.field={field}"
        ),
        _ => panic!("invalid anchor"),
    }
}

fn choose(
    db: &Connection,
    v: &BTreeMap<String, i64>,
    m: &mut Meter,
) -> rusqlite::Result<(String, Vec<Vec<i64>>)> {
    let a = code(v, "kind:function_definition");
    let b = code(v, "kind:identifier");
    // One batch of persisted statistics. No execution-time scan or count of postings.
    let stats=m.statement(db,&format!("SELECT s.code,sum(s.n) FROM file_kind_counts s JOIN demand d ON d.fid=s.fid WHERE s.code IN ({a},{b}) GROUP BY s.code"))?;
    let estimate = |kind| stats.iter().find(|r| r[0] == kind).map_or(0, |r| r[1]);
    Ok((
        if estimate(a) <= estimate(b) {
            "parent"
        } else {
            "child"
        }
        .into(),
        stats,
    ))
}

fn fusion(
    db: &Connection,
    base: &str,
    variant: &str,
    m: &mut Meter,
) -> rusqlite::Result<Vec<Vec<i64>>> {
    let project = "SELECT 1 AS consumer,scope,fid,pre,child FROM shared WHERE child%2=0 UNION ALL SELECT 2,scope,fid,pre,child FROM shared WHERE child%3=0";
    match variant {
        "staged"=>{
            m.statement(db,&format!("CREATE TEMP TABLE shared AS {base}"))?;
            m.statement(db,"CREATE INDEX temp.shared_scope ON shared(scope,pre)")?;
            m.statement(db,"ANALYZE temp.shared")?;
            let rows=m.statement(db,project)?;
            m.statement(db,"DROP TABLE shared")?; Ok(rows)
        }
        "inline"|"materialized"=>m.statement(db,&format!("WITH shared(scope,fid,pre,child) AS {} ({base}) {project}",if variant=="materialized" {"MATERIALIZED"}else{"NOT MATERIALIZED"})),
        "fused"=>m.statement(db,&format!("WITH shared(scope,fid,pre,child) AS NOT MATERIALIZED ({base}), consumers(id,modulus) AS (VALUES(1,2),(2,3)) SELECT consumers.id,shared.* FROM shared CROSS JOIN consumers WHERE shared.child%consumers.modulus=0")),
        _=>Err(rusqlite::Error::InvalidParameterName("fusion variant".into())),
    }
}

fn planning(
    db: &Connection,
    base: &str,
    variant: &str,
    scale: usize,
    m: &mut Meter,
) -> rusqlite::Result<Vec<Vec<i64>>> {
    let mut result = Vec::new();
    let query = format!(
        "WITH shared(scope,fid,pre,child) AS NOT MATERIALIZED ({base}) SELECT scope,fid,pre,child FROM shared WHERE scope=?1"
    );
    if variant == "batch" {
        return m.statement(db, base);
    } else if variant == "reuse" {
        let start = cpu_ms();
        let mut stmt = db.prepare(&query)?;
        let prepare = cpu_ms() - start;
        let start = cpu_ms();
        for scope in 0..scale {
            for row in stmt.query_map([scope as i64], |r| {
                Ok(vec![r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?])
            })? {
                result.push(row?);
            }
        }
        // Include every reset/execution and all output conversion, not only prepare.
        m.record_repeated(&stmt, query.len(), scale, prepare, cpu_ms() - start);
    } else if variant == "private-schema" {
        for scope in 0..scale {
            m.statement(db,&format!("CREATE TEMP VIEW consumer_{scope} AS SELECT * FROM ({base}) WHERE scope={scope}"))?;
            result.extend(m.statement(db, &format!("SELECT * FROM consumer_{scope}"))?);
            m.statement(db, &format!("DROP VIEW consumer_{scope}"))?;
        }
    } else if variant == "reprepare" {
        for scope in 0..scale {
            result.extend(m.statement(db, &query.replace("?1", &scope.to_string()))?);
        }
    } else {
        return Err(rusqlite::Error::InvalidParameterName(
            "planning variant".into(),
        ));
    }
    Ok(result)
}

pub fn run(capture: &Path, experiment: &str, variant: &str, scale: usize) -> Result<Value> {
    if scale > 2048 {
        return Err("scale must not exceed 2048".into());
    }
    if experiment == "publication" {
        return publication::run(capture, variant, scale);
    }
    let process = cpu_ms();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("capture.sqlite");
    std::fs::copy(capture, &path)?;
    let store = crate::store::Store::open(&path)?;
    let db = &store.db;
    let copy_open_cpu = cpu_ms() - process;
    let mut setup = Meter::default();
    let common = cpu_ms();
    let direct = experiment == "composed" && variant.starts_with("direct-");
    if !direct {
        facts(db, &[1, 2, 3], &mut setup)?;
    }
    demand(db, scale)?;
    let v = vocabulary(db)?;
    let common_cpu = cpu_ms() - common;
    let mut m = Meter::default();
    let mut counters = json!(null);
    let started = cpu_ms();
    let (chosen, stats) = choose(db, &v, &mut m)?;
    let base = arm(&v, &chosen);
    let mut rows = match experiment {
        "anchor" => match variant {
            "broad" => {
                let f = code(&v, "kind:function_definition");
                let i = code(&v, "kind:identifier");
                let field = code(&v, "field:name");
                m.statement(db,&format!("CREATE TEMP TABLE broad AS SELECT p.fid,p.pre,c.pre AS child FROM n p INDEXED BY n_kind JOIN n c INDEXED BY n_parent ON c.fid=p.fid AND c.parent=p.pre WHERE p.kind={f} AND c.kind={i} AND c.field={field}"))?;
                let r=m.statement(db,"SELECT d.scope,b.fid,b.pre,b.child FROM demand d CROSS JOIN broad b WHERE b.fid=d.fid AND b.pre>=d.lo AND b.pre<d.hi")?;
                m.statement(db, "DROP TABLE broad")?;
                r
            }
            "parent" | "child" => m.statement(db, &arm(&v, variant))?,
            "chosen" => m.statement(db, &base)?,
            _ => return Err("anchor variant".into()),
        },
        "fusion" => fusion(db, &base, variant, &mut m)?,
        "native" => {
            let (r, c) = native::run(&store, &v, &chosen, variant, &mut m)?;
            counters = c;
            r
        }
        "planning" => planning(db, &base, variant, scale, &mut m)?,
        "composed" => {
            if variant == "staged" {
                fusion(db, &arm(&v, "parent"), "staged", &mut m)?
            } else if variant == "fused" {
                fusion(db, &base, "fused", &mut m)?
            } else if variant == "native" || variant == "direct-native" {
                let (out, c) = native::run(
                    &store,
                    &v,
                    &chosen,
                    if direct {
                        "direct-batch-consumers"
                    } else {
                        "batch-consumers"
                    },
                    &mut m,
                )?;
                counters = c;
                out
            } else if variant == "direct-scalar" {
                let (out, c) = native::run(&store, &v, &chosen, "direct-scalar-consumers", &mut m)?;
                counters = c;
                out
            } else {
                return Err("composed variant".into());
            }
        }
        _ => return Err("unknown experiment".into()),
    };
    let kernel = cpu_ms() - started;
    rows.sort();
    let temp: i64 = db.query_row("PRAGMA temp.page_count", [], |r| r.get(0))?;
    let bytes: i64 = db.query_row("PRAGMA temp.page_size", [], |r| r.get(0))?;
    let native = store.readers.0.lock().counters;
    Ok(
        json!({"format":"rerail.cold-cpu-spike/v1","experiment":experiment,"variant":variant,"scale":scale,
        "cpuMs":common_cpu+kernel,"kernelCpuMs":kernel,"commonCpuMs":common_cpu,"copyOpenCpuMs":copy_open_cpu,
        "processCpuMs":cpu_ms()-process,"peakRssBytes":peak_bytes(),"tempBytes":temp*bytes,
        "chosenArm":chosen,"storedStats":stats,"common":setup,"sql":m,"emittedSql":m.last_query(),"operator":counters,"native":native,"rows":rows,
        "sqliteVersion":rusqlite::version(),
        "scope":"Captured Python tape; generic field-qualified child relation. Includes facts, indexes, demand, planning, outputs and cleanup. Excludes parsing, matcher, reporting, repository scheduling and publication; this is not an end-to-end engine benchmark."}),
    )
}
