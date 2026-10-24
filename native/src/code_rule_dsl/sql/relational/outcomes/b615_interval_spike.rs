//! Utility-entry interval frontier over the batched captured B615 SQL producer.
//! No Boolean program counter exists: immutable intervals describe the already
//! proved structural path; only native utility memo transactions are dynamic.
use super::*;
use crate::code_rule_dsl::{language::GrammarDialect, sql::relational::outcomes::emit::bridge};
pub(super) fn intervals(db: &rusqlite::Connection, language: GrammarDialect) {
    let parenthesized = language.kinds().kind_id("parenthesized_expression");
    let comment = language.kinds().kind_id("comment");
    let attribute = language.kinds().kind_id("attribute");
    let object = language.field_id("object").unwrap();
    let sql = format!(
        r#"CREATE TEMP TABLE b615_events AS WITH RECURSIVE path(key,node,depth) AS(SELECT key,node,0 FROM b615_summary UNION ALL SELECT p.key,c.id,p.depth+1 FROM path p JOIN b615_summary s ON s.key=p.key JOIN node n ON n.file=s.host AND n.id=p.node JOIN node c ON c.file=n.file AND c.parent=n.id WHERE n.kind={parenthesized} AND c.kind<>{comment} AND NOT node_regex(3,c.file,c.id)), base AS MATERIALIZED(SELECT p.key,p.node,p.depth wrappers FROM path p JOIN b615_summary s ON s.key=p.key JOIN node n ON n.file=s.host AND n.id=p.node WHERE n.kind={attribute})
SELECT s.key,p.depth+1 ordinal,CASE WHEN s.truth=1 THEN s.cost ELSE p.depth+4*(b.wrappers-p.depth+1) END finish,10 utility,p.node,p.depth+1 relative,CASE WHEN s.truth=1 THEN b.wrappers-p.depth+4 ELSE 4*(b.wrappers-p.depth+1) END cost,b.wrappers-p.depth+CASE WHEN s.truth=1 THEN 3 ELSE 2 END peak,s.truth,s.kind input_kind,s.api input,CASE WHEN s.truth=1 THEN s.capture ELSE s.api END output
FROM b615_summary s JOIN path p ON p.key=s.key JOIN base b ON b.key=s.key
UNION ALL SELECT s.key,b.wrappers+2,b.wrappers+2,20,b.node,b.wrappers+2,1,1,1,s.kind,s.api,s.capture FROM b615_summary s JOIN base b ON b.key=s.key WHERE s.truth=1
UNION ALL SELECT s.key,b.wrappers+3,b.wrappers+4,30,b.node,b.wrappers+2,2,2,1,1,s.capture,s.capture FROM b615_summary s JOIN base b ON b.key=s.key WHERE s.truth=1
UNION ALL SELECT s.key,b.wrappers+4,b.wrappers+4,30,n.id,b.wrappers+3,1,1,1,1,s.capture,s.capture FROM b615_summary s JOIN base b ON b.key=s.key JOIN node n ON n.file=s.host AND n.parent=b.node AND n.field={object} AND n.ffirst=1 WHERE s.truth=1
UNION ALL SELECT s.key,b.wrappers+2+3*(b.wrappers-p.depth)+j.column1,b.wrappers+2+3*(b.wrappers-p.depth)+j.column1,20+j.column1,p.node,p.depth+2,1,1,0,s.kind,s.api,s.api FROM b615_summary s JOIN path p ON p.key=s.key JOIN base b ON b.key=s.key CROSS JOIN(VALUES(0),(1),(2))j WHERE s.truth=0;
CREATE UNIQUE INDEX b615_event_key ON b615_events(key,ordinal);"#
    );
    db.execute_batch(&sql).unwrap();
}
pub(super) fn query() -> String {
    let identity = "m.clause IS ?1 AND m.utility IS e.utility AND m.file IS s.host AND m.node IS e.node AND m.k0 IS e.input AND m.k1 IS e.input_kind AND m.k2 IS s.scope";
    let hit = "m.truth IS NOT NULL AND ?3+e.relative-1+m.peak<=256";
    let fail =
        format!("CASE WHEN ({hit}) THEN w.work<m.cost ELSE ?3+e.relative>256 OR w.work=0 END");
    let push = format!("w.phase=0 AND NOT ({hit}) AND ?3+e.relative<=256 AND w.work>0");
    let entry = format!(
        "SELECT CASE WHEN ({fail}) THEN w.ordinal WHEN ({hit}) THEN e.finish+1 ELSE w.ordinal+1 END,CASE WHEN ({hit}) THEN MAX(0,w.work-m.cost) WHEN ?3+e.relative>256 THEN w.work ELSE MAX(0,w.work-1) END,CASE WHEN ({hit}) THEN MAX(w.peak,?3+e.relative-1+m.peak) WHEN ?3+e.relative<=256 THEN MAX(w.peak,?3+e.relative) ELSE w.peak END,COALESCE(frame.handle,w.handle),CASE WHEN ({fail}) THEN CASE WHEN NOT ({hit}) AND ?3+e.relative>256 THEN -1 ELSE -2 END ELSE w.truth END,CASE WHEN ({fail}) THEN 3 ELSE 1 END FROM b615_summary s JOIN b615_events e ON e.key=s.key AND e.ordinal=w.ordinal LEFT JOIN relation_memo m ON m.mode IS CASE WHEN w.phase=0 THEN 0 END AND {identity} LEFT JOIN relation_continuation frame ON frame.mode IS CASE WHEN {push} THEN 1 END AND frame.parent_input IS w.handle AND frame.v0 IS e.ordinal AND frame.v1 IS e.finish WHERE s.key=?2 AND w.phase=0"
    );
    let close = "w.phase=1 AND p.handle IS NOT NULL AND p.c1<w.ordinal";
    let retirement = format!(
        "SELECT w.ordinal,w.work,w.peak,CASE WHEN {close} THEN popped.parent ELSE w.handle END,w.truth,CASE WHEN {close} THEN 1 WHEN w.handle IS NULL AND w.ordinal>s.cost THEN 2 ELSE 0 END FROM b615_summary s LEFT JOIN relation_continuation p ON p.mode IS CASE WHEN w.phase=1 AND w.handle IS NOT NULL THEN 0 END AND p.h IS w.handle LEFT JOIN b615_events e ON e.key=s.key AND e.ordinal=p.c0 LEFT JOIN relation_memo m ON m.mode IS CASE WHEN {close} THEN 1 END AND {identity} AND m.vtruth IS e.truth AND m.vcost IS e.cost AND m.vpeak IS e.peak AND m.v0 IS e.output LEFT JOIN relation_continuation popped ON popped.mode IS CASE WHEN {close} THEN 2 END AND popped.h IS p.handle WHERE s.key=?2 AND w.phase=1"
    );
    let abort = "SELECT w.ordinal,w.work,w.peak,p.parent,w.truth,CASE WHEN p.parent IS NULL THEN 2 ELSE 3 END FROM (SELECT 1) guard LEFT JOIN relation_continuation p ON p.mode IS CASE WHEN w.phase=3 AND w.handle IS NOT NULL THEN 2 END AND p.h IS w.handle WHERE w.phase=3";
    let dispatch = format!("{entry} UNION ALL {retirement} UNION ALL {abort}");
    format!(
        "WITH RECURSIVE machine(ordinal,work,peak,handle,truth,phase) AS(SELECT 1,?4,0,NULL,0,0 UNION ALL SELECT r.c0,r.c1,r.c2,r.c3,r.c4,r.c5 FROM machine w {} WHERE w.phase<>2) SELECT CASE WHEN w.truth<0 THEN w.truth ELSE s.truth END,w.work,w.peak,CASE WHEN w.truth<0 OR s.truth=0 THEN s.api ELSE s.capture END FROM machine w JOIN b615_summary s ON s.key=?2 WHERE w.phase=2",
        bridge(&dispatch, 6)
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::{
        CodeRuleDslRuntime, CompiledCodeRuleDsl, matcher,
        sql::{exec, tests::differential::tape},
        test_support::{CAPABILITIES, grammar_matchers},
    };
    use ast_grep_core::AstGrep;
    use serde_json::{Value, json};
    fn policy(repeat: usize, wrong: bool) -> CompiledCodeRuleDsl {
        let mut p: Value = serde_json::from_str(include_str!(
            "../../../../../../packs/python/rules/bandit-b615-huggingface-unsafe-download.json"
        ))
        .unwrap();
        let mut all = vec![];
        if wrong {
            all.push(json!({"has":{"stopBy":"end","kind":"identifier","regex":"^transformers$","pattern":"$API"}}));
        }
        for _ in 0..repeat {
            all.push(json!({"matches":"hf-callee"}));
        }
        p["clauses"][0]["forbid"]["ast"]["rule"] = json!({"all":all});
        CodeRuleDslRuntime::compile(p.to_string(), "json".into(), CAPABILITIES.into()).unwrap()
    }
    #[test]
    fn structural_spike_b615_interval_memo_cross_root_and_capture_order() {
        let mut evidence = vec![];
        for wrong in [false, true] {
            for repeat in [1, 3] {
                let p = policy(repeat, wrong);
                let g = &grammar_matchers(&p)[0];
                let doc = tape(g.language, "((((wrapper.transformers.from_pretrained))))\n");
                let tree = AstGrep::doc(doc.clone());
                let roots = tree
                    .root()
                    .dfs()
                    .filter(|n| {
                        matches!(n.kind().as_ref(), "parenthesized_expression" | "attribute")
                    })
                    .filter(|n| prove(n.clone()).is_ok())
                    .collect::<Vec<_>>();
                assert_eq!(roots.len(), 5);
                let proofs = roots
                    .iter()
                    .cloned()
                    .map(|n| prove(n).unwrap())
                    .collect::<Vec<_>>();
                let atom = pattern(&g.matcher).unwrap();
                let AstMatcher::Pattern(atom) = atom else {
                    panic!("atom")
                };
                let demands = proofs
                    .iter()
                    .flat_map(|p| {
                        [
                            key(0, 0, p, 0, Cell::Null).unwrap(),
                            key(0, 0, p, 1, Cell::Integer(i64::from(p.api))).unwrap(),
                            key(0, 0, p, 1, Cell::Integer(i64::from(p.incompatible))).unwrap(),
                        ]
                    })
                    .collect::<Vec<_>>();
                let cases = [
                    (4, 0, 64),
                    (0, 0, 4),
                    (0, 0, 64),
                    (2, 0, 64),
                    (0, 0, 0),
                    (0, 0, 8),
                    (0, 0, 10),
                    (1, 253, 64),
                    (0, 255, 64),
                    (4, 256, 64),
                    (0, 0, 64),
                    (4, 0, 0),
                    (0, 250, 8),
                ];
                let _scope =
                    matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
                let native = matcher::with_file_memo(&tree, || {
                    cases
                        .iter()
                        .map(|&(root, level, work)| {
                            matcher::match_at_depth(g, roots[root].clone(), level, work)
                        })
                        .collect::<Vec<_>>()
                });
                exec::with_terminals(
                    vec![(doc, Default::default())],
                    terminals(atom, g.language),
                    |db, batch, _| {
                        crate::code_rule_dsl::sql::relational::leaves::register(db, batch).unwrap();
                        let producer = Producer::batch(db, 0, g.language, &demands);
                        intervals(db, g.language);
                        let sql = query();
                        for ((root, level, work), native) in cases.into_iter().zip(native) {
                            let p = &proofs[root];
                            let mut capture = if wrong {
                                Cell::Integer(i64::from(p.incompatible))
                            } else {
                                Cell::Null
                            };
                            let mut remaining = work as i64;
                            let mut peak = 0;
                            let mut truth = 1;
                            for _ in 0..repeat {
                                let typed = key(
                                    0,
                                    0,
                                    p,
                                    u32::from(!matches!(capture, Cell::Null)),
                                    capture.clone(),
                                )
                                .unwrap();
                                let id = producer.demand(db, typed);
                                let row: (i64, i64, u32, Cell) = db
                                    .query_row(
                                        &sql,
                                        rusqlite::params![9000000, id, level, remaining],
                                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                                    )
                                    .unwrap();
                                truth = row.0;
                                remaining = row.1;
                                peak = peak.max(row.2);
                                capture = row.3;
                                if truth != 1 {
                                    break;
                                }
                            }
                            assert_eq!(
                                (truth, remaining, peak.saturating_sub(level)),
                                (
                                    native.truth,
                                    native.work as i64,
                                    native.peak.saturating_sub(level)
                                ),
                                "wrong={wrong} repeat={repeat} root={root} incoming={level} work={work}"
                            );
                            assert_eq!(
                                db.query_row("SELECT relation_continuation_rows()", [], |r| r
                                    .get::<_, i64>(0))
                                    .unwrap(),
                                0
                            );
                            evidence.push(json!({"wrongAPI":wrong,"repeat":repeat,"root":root,"incoming":level,"allowance":work,"truth":truth,"work":remaining,"peak":peak,"eventContract":"immutable utility intervals; cached subtree cuts; no Boolean PC"}));
                        }
                    },
                );
            }
        }
        if let Ok(path) = std::env::var("RERAIL_B615_CAPTURED_RELATION_SPIKE_EVIDENCE") {
            std::fs::write(
                format!("{path}.intervals.json"),
                serde_json::to_vec_pretty(&evidence).unwrap(),
            )
            .unwrap();
        }
    }
    #[derive(Default)]
    struct ProfileRows(Vec<Value>);
    struct ProfileTrace<'a> {
        db: &'a rusqlite::Connection,
        rows: Box<ProfileRows>,
    }
    impl Drop for ProfileTrace<'_> {
        fn drop(&mut self) {
            // SAFETY: remove the callback before freeing its stable boxed rows.
            unsafe {
                rusqlite::ffi::sqlite3_trace_v2(self.db.handle(), 0, None, std::ptr::null_mut());
            }
        }
    }
    fn measured<T>(db: &rusqlite::Connection, body: impl FnOnce() -> T) -> (T, Value) {
        unsafe extern "C" fn profile(
            mask: u32,
            context: *mut std::ffi::c_void,
            statement: *mut std::ffi::c_void,
            duration: *mut std::ffi::c_void,
        ) -> i32 {
            if mask == rusqlite::ffi::SQLITE_TRACE_PROFILE {
                // SAFETY: SQLite supplies a live statement and nanosecond integer
                // during PROFILE; the stable box lives until callback removal.
                let (rows, sql, vm, ns) = unsafe {
                    let statement = statement.cast::<rusqlite::ffi::sqlite3_stmt>();
                    let text = rusqlite::ffi::sqlite3_sql(statement);
                    let sql = if text.is_null() {
                        std::borrow::Cow::Borrowed("<internal>")
                    } else {
                        std::ffi::CStr::from_ptr(text).to_string_lossy()
                    };
                    (
                        &mut (*context.cast::<ProfileRows>()).0,
                        sql,
                        rusqlite::ffi::sqlite3_stmt_status(
                            statement,
                            rusqlite::ffi::SQLITE_STMTSTATUS_VM_STEP,
                            1,
                        ),
                        *duration.cast::<i64>(),
                    )
                };
                rows.push(json!({"sqlPrefix":sql.split_whitespace().take(3).collect::<Vec<_>>().join(" "),"sqlBytes":sql.len(),"vmSteps":vm,"executionWallNs":ns}));
            }
            0
        }
        let mut trace = ProfileTrace {
            db,
            rows: Box::default(),
        };
        // SAFETY: the connection and stable boxed vector outlive the callback.
        let status = unsafe {
            rusqlite::ffi::sqlite3_trace_v2(
                db.handle(),
                rusqlite::ffi::SQLITE_TRACE_PROFILE,
                Some(profile),
                (&mut *trace.rows as *mut ProfileRows).cast(),
            )
        };
        assert_eq!(status, rusqlite::ffi::SQLITE_OK);
        let start = std::time::Instant::now();
        let result = body();
        let wall = start.elapsed().as_nanos() as u64;
        // SAFETY: stop callbacks before reading or moving the profile vector.
        unsafe {
            rusqlite::ffi::sqlite3_trace_v2(db.handle(), 0, None, std::ptr::null_mut());
        }
        let statements = std::mem::take(&mut trace.rows.0);
        let vm = statements
            .iter()
            .map(|v| v["vmSteps"].as_u64().unwrap())
            .sum::<u64>();
        let execution_wall = statements
            .iter()
            .map(|v| v["executionWallNs"].as_u64().unwrap())
            .sum::<u64>();
        (
            result,
            json!({"vmSteps":vm,"envelopeWallNs":wall,"executionWallNs":execution_wall,"executionClock":"SQLite PROFILE duration, coarse clock resolution","statements":statements}),
        )
    }
    fn seeded_policy(bound: bool) -> CompiledCodeRuleDsl {
        let mut p: Value = serde_json::from_str(include_str!(
            "../../../../../../packs/python/rules/bandit-b615-huggingface-unsafe-download.json"
        ))
        .unwrap();
        p["clauses"][0]["forbid"]["ast"]["rule"] = if bound {
            json!({"all":[{"has":{"stopBy":"end","kind":"identifier","regex":"^from_pretrained$","pattern":"$API"}},{"matches":"hf-callee"}]})
        } else {
            json!({"matches":"hf-callee"})
        };
        CodeRuleDslRuntime::compile(p.to_string(), "json".into(), CAPABILITIES.into()).unwrap()
    }
    #[test]
    fn structural_spike_b615_interval_setup_and_suffix_scaling() {
        let mut evidence = vec![];
        for depth in [1, 8, 32] {
            for mode in ["inner", "outer", "all"] {
                for diversity in ["absent", "bound", "both"] {
                    let absent = seeded_policy(false);
                    let bound = seeded_policy(true);
                    let g = &grammar_matchers(&absent)[0];
                    let source = format!(
                        "{}wrapper.transformers.from_pretrained{}\n",
                        "(".repeat(depth),
                        ")".repeat(depth)
                    );
                    let doc = tape(g.language, &source);
                    let tree = AstGrep::doc(doc.clone());
                    let all = tree
                        .root()
                        .dfs()
                        .filter(|n| {
                            matches!(n.kind().as_ref(), "parenthesized_expression" | "attribute")
                        })
                        .filter(|n| prove(n.clone()).is_ok())
                        .collect::<Vec<_>>();
                    assert_eq!(all.len(), depth + 1);
                    let selected = match mode {
                        "inner" => vec![all.len() - 1],
                        "outer" => vec![0],
                        _ => (0..all.len()).rev().collect::<Vec<_>>(),
                    };
                    let variants = match diversity {
                        "absent" => vec![false],
                        "bound" => vec![true],
                        _ => vec![false, true],
                    };
                    let proofs = selected
                        .iter()
                        .map(|&root| prove(all[root].clone()).unwrap())
                        .collect::<Vec<_>>();
                    let demands = proofs
                        .iter()
                        .flat_map(|p| {
                            variants.iter().map(|present| {
                                key(
                                    0,
                                    0,
                                    p,
                                    u32::from(*present),
                                    if *present {
                                        Cell::Integer(i64::from(p.api))
                                    } else {
                                        Cell::Null
                                    },
                                )
                                .unwrap()
                            })
                        })
                        .collect::<Vec<_>>();
                    let _scope = matcher::FileScopeSlot::install(
                        Default::default(),
                        Default::default(),
                        false,
                    );
                    let native = variants
                        .iter()
                        .map(|present| {
                            let native_grammar = if *present {
                                &grammar_matchers(&bound)[0]
                            } else {
                                g
                            };
                            matcher::with_file_memo(&tree, || {
                                selected
                                    .iter()
                                    .map(|&root| {
                                        matcher::match_at_depth(
                                            native_grammar,
                                            all[root].clone(),
                                            0,
                                            10000,
                                        )
                                    })
                                    .collect::<Vec<_>>()
                            })
                        })
                        .collect::<Vec<_>>();
                    let atom = pattern(&g.matcher).unwrap();
                    let AstMatcher::Pattern(atom) = atom else {
                        panic!("atom")
                    };
                    exec::with_terminals(
                        vec![(doc, Default::default())],
                        terminals(atom, g.language),
                        |db, batch, _| {
                            crate::code_rule_dsl::sql::relational::leaves::register(db, batch)
                                .unwrap();
                            let (producer, producer_metrics) =
                                measured(db, || Producer::batch(db, 0, g.language, &demands));
                            let (_, interval_metrics) = measured(db, || intervals(db, g.language));
                            let distinct = producer.key_count(db);
                            let rows: i64 = db
                                .query_row("SELECT COUNT(*) FROM b615_events", [], |r| r.get(0))
                                .unwrap();
                            let unique_suffixes:i64=db.query_row("SELECT COUNT(*) FROM(SELECT DISTINCT s.host,s.scope,e.utility,e.node,e.input_kind,typeof(e.input),e.input,e.truth,e.cost,e.peak,e.output FROM b615_events e JOIN b615_summary s ON s.key=e.key)",[],|r|r.get(0)).unwrap();
                            let before =
                                crate::code_rule_dsl::sql::relational::continuation_store::traffic(
                                );
                            let sql = query();
                            let prepared = std::time::Instant::now();
                            let mut frontier_stmt = db.prepare(&sql).unwrap();
                            let mut key_stmt=db.prepare("SELECT key FROM b615_summary WHERE host=?1 AND scope=?2 AND node=?3 AND kind=?4 AND api IS ?5").unwrap();
                            let frontier_prepare = prepared.elapsed().as_nanos() as u64;
                            let (_, frontier_metrics) = measured(db, || {
                                for (variant_index, present) in variants.iter().enumerate() {
                                    for (root_index, p) in proofs.iter().enumerate() {
                                        let typed = key(
                                            0,
                                            0,
                                            p,
                                            u32::from(*present),
                                            if *present {
                                                Cell::Integer(i64::from(p.api))
                                            } else {
                                                Cell::Null
                                            },
                                        )
                                        .unwrap();
                                        let id: i64 = key_stmt
                                            .query_row(
                                                rusqlite::params![
                                                    typed.host,
                                                    typed.scope,
                                                    typed.node,
                                                    typed.kind,
                                                    typed.api
                                                ],
                                                |r| r.get(0),
                                            )
                                            .unwrap();
                                        let row: (i64, i64, u32, Cell) = frontier_stmt
                                            .query_row(
                                                rusqlite::params![
                                                    9200000 + variant_index as i64,
                                                    id,
                                                    0,
                                                    10000
                                                ],
                                                |r| {
                                                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                                                },
                                            )
                                            .unwrap();
                                        let expected = &native[variant_index][root_index];
                                        assert_eq!(
                                            (row.0, row.1, row.2),
                                            (expected.truth, expected.work as i64, expected.peak)
                                        );
                                        assert_eq!(row.3.single_node().unwrap(), Some(p.api));
                                    }
                                }
                            });
                            let after =
                                crate::code_rule_dsl::sql::relational::continuation_store::traffic(
                                );
                            let traffic = after
                                .into_iter()
                                .zip(before)
                                .map(|(a, b)| a - b)
                                .collect::<Vec<_>>();
                            let(live,peak):(i64,i64)=db.query_row("SELECT relation_continuation_rows(),relation_continuation_peak()",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
                            assert_eq!(live, 0);
                            let total_vm = producer_metrics["vmSteps"].as_u64().unwrap()
                                + interval_metrics["vmSteps"].as_u64().unwrap()
                                + frontier_metrics["vmSteps"].as_u64().unwrap();
                            let total_wall = producer_metrics["envelopeWallNs"].as_u64().unwrap()
                                + interval_metrics["envelopeWallNs"].as_u64().unwrap()
                                + frontier_metrics["envelopeWallNs"].as_u64().unwrap();
                            let total_execution_wall =
                                producer_metrics["executionWallNs"].as_u64().unwrap()
                                    + interval_metrics["executionWallNs"].as_u64().unwrap()
                                    + frontier_metrics["executionWallNs"].as_u64().unwrap();
                            evidence.push(json!({"depth":depth,"rootMode":mode,"captureDiversity":diversity,"distinctDemands":distinct,"producedIntervalRows":rows,"uniqueSuffixAnswers":unique_suffixes,"duplicateIntervalRows":rows-unique_suffixes,"setupProducer":producer_metrics,"intervalCTASAndIndex":interval_metrics,"frontierConsumption":frontier_metrics,"totalVmSteps":total_vm,"totalEnvelopeWallNs":total_wall,"totalExecutionWallNs":total_execution_wall,"frontierPrepareWallNs":frontier_prepare,"continuationTraffic":traffic,"continuationLive":live,"continuationPeak":peak,"qualification":"native truth/work/peak/API parity for this pure structural captured domain only","representation":"flattened per-demand intervals; suffix uniqueness is diagnostic, no suffix reuse is executed","accounting":"all SQL schema/insert/CTAS/index statements profiled; diagnostic COUNT reads excluded; frontier and routing prepared once before consumption; per-demand calls are a diagnostic harness, not a job execution-budget claim"}));
                        },
                    );
                }
            }
        }
        let largest = evidence
            .iter()
            .find(|r| r["depth"] == 32 && r["rootMode"] == "all" && r["captureDiversity"] == "both")
            .unwrap();
        assert!(
            largest["duplicateIntervalRows"].as_i64().unwrap() > 0,
            "scaling probe must expose repeated suffix representation"
        );
        if let Ok(path) = std::env::var("RERAIL_B615_CAPTURED_RELATION_SPIKE_EVIDENCE") {
            std::fs::write(
                format!("{path}.scaling.json"),
                serde_json::to_vec_pretty(&evidence).unwrap(),
            )
            .unwrap();
        }
    }
}
