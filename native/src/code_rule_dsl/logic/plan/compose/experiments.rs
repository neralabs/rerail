//! Test-binary-only query replays over the exact bound production sources.
//! Each case records statement counters, input cursor work, EXPLAIN, and output parity.
//! Snapshot cases measure downstream SQL alone; their capture cost is recorded separately.
use super::*;
use crate::code_rule_dsl::facts::vtab::{self, Relation as Rows};
use rusqlite::StatementStatus;
use serde::Deserialize;
use serde_json::{Value as Json, json};
use std::{
    io::Write,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

pub(super) fn native_only() -> bool {
    std::env::var_os("RERAIL_COMPOSE_LAB_NATIVE").is_some()
}

#[derive(Deserialize)]
struct Case {
    name: String,
    #[serde(default)]
    replacements: Vec<[String; 2]>,
    #[serde(default)]
    select: Option<String>,
    #[serde(default)]
    snapshots: Vec<String>,
}

fn cpu() -> f64 {
    let (u, s) = super::super::corpus::thread_usage();
    u + s
}

/// Locate CTE bodies without confusing nested queries or quoted SQL text with boundaries.
fn bodies(sql: &str) -> Vec<(String, std::ops::Range<usize>)> {
    let Some(mut at) = sql.find("WITH RECURSIVE ").map(|n| n + 15) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    loop {
        let Some(cols) = sql[at..].find('(').map(|n| n + at) else {
            break;
        };
        let name = sql[at..cols].trim().trim_matches('"').to_owned();
        let Some(as_at) = sql[cols..].find(" AS ").map(|n| n + cols + 4) else {
            break;
        };
        let Some(open) = sql[as_at..].find('(').map(|n| n + as_at) else {
            break;
        };
        let bytes = sql.as_bytes();
        let mut n = open + 1;
        let mut depth = 1;
        let mut quote = 0;
        while n < bytes.len() && depth > 0 {
            let c = bytes[n];
            if quote != 0 {
                if c == quote {
                    if n + 1 < bytes.len() && bytes[n + 1] == quote {
                        n += 1;
                    } else {
                        quote = 0;
                    }
                }
            } else if c == b'\'' || c == b'"' {
                quote = c;
            } else if c == b'(' {
                depth += 1;
            } else if c == b')' {
                depth -= 1;
            }
            n += 1;
        }
        out.push((name, open + 1..n - 1));
        at = n;
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        if at == bytes.len() || bytes[at] != b',' {
            break;
        }
        at += 1;
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
    }
    out
}

fn query(db: &Connection, sql: &str) -> Result<(Vec<Vec<SqlValue>>, Json)> {
    let _scope = vtab::demand::Scope::enter();
    let before = vtab::counters(db);
    let prep = cpu();
    let mut stmt = db.prepare_cached(sql)?;
    let prep = cpu() - prep;
    for s in [
        StatementStatus::VmStep,
        StatementStatus::FullscanStep,
        StatementStatus::Sort,
        StatementStatus::AutoIndex,
        StatementStatus::RePrepare,
    ] {
        stmt.reset_status(s);
    }
    let width = stmt.column_count();
    let ops = Arc::new(AtomicUsize::new(0));
    // rusqlite 0.41's `progress_handler` takes the connection mutably (the statement holds it shared): SQLite's own,
    // set here and taken away after the query, while `ops` lives.
    unsafe extern "C" fn tick(ops: *mut std::ffi::c_void) -> std::ffi::c_int {
        // SAFETY: `ops` is the `AtomicUsize` below, alive until the handler is taken away.
        std::ffi::c_int::from(unsafe { (*(ops as *const AtomicUsize)).fetch_add(1, Ordering::Relaxed) } > 2_000)
    }
    // SAFETY: the handle is the open connection's; the handler is removed before `ops` drops.
    let handle = unsafe { db.handle() };
    unsafe { rusqlite::ffi::sqlite3_progress_handler(handle, 10_000, Some(tick), Arc::as_ptr(&ops) as *mut std::ffi::c_void) };
    let start = Instant::now();
    let c = cpu();
    let rows = (|| -> rusqlite::Result<Vec<Vec<SqlValue>>> {
        let mut cur = stmt.query([])?;
        let mut rows = Vec::new();
        while let Some(r) = cur.next()? {
            rows.push(
                (0..width)
                    .map(|i| r.get(i))
                    .collect::<rusqlite::Result<_>>()?,
            );
        }
        Ok(rows)
    })();
    let spent = cpu() - c;
    let wall = start.elapsed().as_secs_f64();
    unsafe { rusqlite::ffi::sqlite3_progress_handler(handle, 0, None, std::ptr::null_mut()) };
    let work: BTreeMap<_, _> = vtab::counters(db)
        .into_iter()
        .filter_map(|(name, now)| {
            let old = before.get(&name).copied().unwrap_or_default();
            let delta = std::array::from_fn::<_, 4, _>(|i| now[i].saturating_sub(old[i]));
            delta.iter().any(|&n| n != 0).then_some((name, delta))
        })
        .collect();
    let report = json!({"prepareCpu":prep,"cpu":spent,"wall":wall,"vm":stmt.get_status(StatementStatus::VmStep),"scans":stmt.get_status(StatementStatus::FullscanStep),"sorts":stmt.get_status(StatementStatus::Sort),"autoindex":stmt.get_status(StatementStatus::AutoIndex),"reprepare":stmt.get_status(StatementStatus::RePrepare),"inputs":work,"error":rows.as_ref().err().map(ToString::to_string)});
    // An interrupted experiment remains a recorded failure; it does not abort the corpus run.
    Ok((rows.unwrap_or_default(), report))
}

pub(super) fn replay(
    plan: &Compiled,
    db: &Connection,
    expected: &BTreeMap<String, Vec<Vec<SqlValue>>>,
) -> Result<()> {
    let Some(config) = std::env::var_os("RERAIL_COMPOSE_LAB_CASES") else {
        return Ok(());
    };
    if !plan.sql.contains("async_site(") {
        return Ok(());
    }
    let path =
        PathBuf::from(std::env::var_os("RERAIL_COMPOSE_LAB_OUT").expect("lab output directory"));
    std::fs::create_dir_all(&path).unwrap();
    let cases: Vec<Case> = serde_json::from_slice(&std::fs::read(config).unwrap()).unwrap();
    let ctes = bodies(&plan.sql);
    let tail = plan
        .sql
        .find("SELECT 0 AS __output")
        .expect("tagged output");
    let prefix = &plan.sql[..tail];
    let mut expected: Vec<_> = plan
        .outputs
        .iter()
        .enumerate()
        .flat_map(|(id, name)| expected[name].iter().map(move |r| format!("{id}:{r:?}")))
        .collect();
    expected.sort();
    expected.dedup();
    let bindings = vtab::Modules::default();
    let mut snapshots = HashMap::new();
    let wanted: BTreeSet<_> = cases
        .iter()
        .flat_map(|c| &c.snapshots)
        .flat_map(|pattern| {
            ctes.iter()
                .filter(move |(n, _)| n.contains(pattern))
                .map(|(n, _)| n.clone())
        })
        .collect();
    let mut records = Vec::new();
    for name in wanted {
        let sql = format!("{prefix}SELECT * FROM {}", ident(&name));
        let (rows, mut report) = query(db, &sql)?;
        assert!(report["error"].is_null(), "snapshot {name}: {report}");
        let width = db.prepare_cached(&sql)?.column_count();
        let cols: Vec<_> = (0..width).map(|i| format!("c{i}")).collect();
        let mut source = Rows::new(&cols.iter().map(String::as_str).collect::<Vec<_>>());
        for row in &rows {
            source.push(
                &row.iter()
                    .map(|v| {
                        if let SqlValue::Integer(n) = v {
                            *n
                        } else {
                            panic!("snapshot cell {v:?}")
                        }
                    })
                    .collect::<Vec<_>>(),
            );
        }
        let table = format!("rr_lab_{}", snapshots.len());
        bindings.bind(db, &table, Arc::new(source))?;
        snapshots.insert(name.clone(), table);
        report["case"] = json!(format!("capture:{name}"));
        report["rows"] = json!(rows.len());
        records.push(report);
    }
    for case in cases {
        let mut sql = plan.sql.clone();
        let mut patches = Vec::new();
        for (name, range) in &ctes {
            if case.snapshots.iter().any(|p| name.contains(p)) {
                patches.push((range.clone(), format!("SELECT * FROM {}", snapshots[name])));
            }
        }
        for (range, text) in patches.into_iter().rev() {
            sql.replace_range(range, &text);
        }
        for [from, to] in &case.replacements {
            assert!(
                sql.contains(from),
                "{}: absent replacement {from}",
                case.name
            );
            sql = sql.replace(from, to);
        }
        if let Some(select) = &case.select {
            let tail = sql.find("SELECT 0 AS __output").unwrap();
            sql.truncate(tail);
            sql += select;
        }
        let _span = super::super::super::profile::span(|| format!("SQL shape lab {}", case.name));
        let (rows, mut report) = query(db, &sql)?;
        let parity = if case.select.is_none() && report["error"].is_null() {
            let mut got: Vec<_> = rows
                .iter()
                .map(|row| {
                    let SqlValue::Integer(id) = row[0] else {
                        panic!("output tag")
                    };
                    let width = plan
                        .outputs
                        .get(id as usize)
                        .and_then(|name| super_width(plan, name))
                        .unwrap_or(row.len() - 1);
                    format!("{id}:{:?}", &row[1..=width])
                })
                .collect();
            got.sort();
            got.dedup();
            Some(got == expected)
        } else {
            None
        };
        report["case"] = json!(case.name);
        report["rows"] = json!(rows.len());
        report["parity"] = json!(parity);
        if case.select.is_some() {
            report["result"] = json!(format!("{rows:?}"));
        }
        let file = path.join(format!("{}.sql", case.name));
        if !file.exists() {
            std::fs::write(&file, &sql).unwrap();
            let mut explain = db.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
            let text = explain
                .query_map([], |r| r.get::<_, String>(3))?
                .collect::<rusqlite::Result<Vec<_>>>()?
                .join("\n");
            std::fs::write(path.join(format!("{}.eqp", case.name)), text).unwrap();
        }
        records.push(report);
    }
    let mut output = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path.join("runs.jsonl"))
        .unwrap();
    for record in records {
        writeln!(output, "{}", serde_json::to_string(&record).unwrap()).unwrap();
    }
    Ok(())
}

fn super_width(plan: &Compiled, name: &str) -> Option<usize> {
    // The generated definition contains exactly one cN per declared output field.
    let start = plan.sql.find(&format!("{}(", ident(name)))? + ident(name).len() + 1;
    let end = plan.sql[start..].find(')')? + start;
    Some(plan.sql[start..end].split(',').count())
}

/// Test-only physical rewrite: the authored rules and semantic operators remain unchanged.
pub(super) fn physical_sql(mut sql: String, prog: &Program) -> String {
    if !sql.contains("async_site(") { return sql; }
    let candidate = std::env::var_os("RERAIL_CANDIDATE_SPIKE").is_some();
    let membership = std::env::var_os("RERAIL_LOOKUP_SQL_IN").is_some();
    if !candidate && !membership {return sql;}
    let ctes=bodies(&sql);
    if candidate {
        super::super::super::profile::decision("candidate spike rewrite", || "after projection planning".into());
        let target_relations:BTreeSet<_>=prog.natives.values().filter_map(|native| match native {
            super::super::super::front::Native::Python {kind,input,targets:Some(target)} if kind=="qualified" && input=="async_reference" => Some(target),
            _=>None,
        }).collect();
        let targets=target_relations.into_iter().map(|n|format!("SELECT c0 FROM {}",ident(n))).collect::<Vec<_>>().join(" UNION ");
        assert!(!targets.is_empty());
        let mut patches=Vec::new();
        for (name,range) in ctes {
            let body=match name.as_str() {
                "async_site"=>Some("SELECT c0,c1,c2 FROM __rr_candidates"),
                "async_reference"=>Some("SELECT c2 FROM __rr_candidates WHERE (c4 & 1) != 0 AND c2 >= 0"),
                "async_open_receiver"=>Some("SELECT c1,c3 FROM __rr_candidates WHERE (c4 & 2) != 0"),
                _=>None,
            };
            if let Some(body)=body {patches.push((range,body));}
        }
        assert_eq!(patches.len(),3);
        for (range,body) in patches.into_iter().rev() {sql.replace_range(range,body);}
        let definition=format!("__rr_candidates(c0,c1,c2,c3,c4) AS MATERIALIZED (SELECT c0,c1,c2,c3,c4 FROM rr_batch_python_candidate_spike WHERE frame=0 AND targets=(SELECT json_group_array(sym_text(c0)) FROM ({targets}))),\n");
        sql.insert_str("WITH RECURSIVE ".len(),&definition);
        if std::env::var_os("RERAIL_DEMAND_FRAME_GUARD").is_some() {
            let mut gates=Vec::new();
            for (name,range) in bodies(&sql) {
                let body=&sql[range.clone()];
                if name.starts_with("__rr_demand_") {
                    assert!(body.contains("rr_demand_frame("));
                    gates.push((range,format!("{body} HAVING count(*) > 0")));
                }else if let Some(super::super::super::front::Native::Python{kind,..})=prog.natives.get(&name) {
                    let frame=body.split("frame=(SELECT handle FROM ").nth(1).unwrap().split(')').next().unwrap();
                    let guarded=body.replace(&format!("FROM rr_batch_python_{kind} WHERE"),&format!("FROM {frame} AS demanded CROSS JOIN rr_batch_python_{kind} WHERE"))
                        .replace(&format!("frame=(SELECT handle FROM {frame})"),"frame=demanded.handle");
                    gates.push((range,guarded));
                }
            }
            for (range,body) in gates.into_iter().rev(){sql.replace_range(range,&body);}
        }
        if std::env::var_os("RERAIL_EMPTY_DEMAND_GUARD").is_some() {
            let mut guards=Vec::new();
            for (name,range) in bodies(&sql) {
                if let Some(super::super::super::front::Native::Python{input,..})=prog.natives.get(&name) {
                    let body=&sql[range.clone()];
                    let guarded=body.replacen(" WHERE ",&format!(" WHERE EXISTS (SELECT 1 FROM {}) AND ",ident(input)),1);
                    assert_ne!(&guarded,body);guards.push((range,guarded));
                }
            }
            for (range,body) in guards.into_iter().rev(){sql.replace_range(range,&body);}
        }
    } else {
        let range=ctes.into_iter().find(|(n,_)|n=="async_open_receiver").unwrap().1;
        let old=&sql[range.clone()];
        let body=old.replace("t1.c1 AS c0","t2.node AS c0").replace("FROM async_site t1 CROSS JOIN main.py_attribute t2","FROM main.py_attribute t2").replace("(t2.node = t1.c1)","(t2.node IN (SELECT c1 FROM async_site))");
        assert!(body!=old && !body.contains("t1."));sql.replace_range(range,&body);
    }
    sql
}
