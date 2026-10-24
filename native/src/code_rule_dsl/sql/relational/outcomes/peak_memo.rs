//! Peak-aware utility transactions for compiler-selected recursive clauses.
//! Every program is one reached invocation; SQL supplies entry depth and completion.
use super::{
    Demand, Op, Output, Plan, Sql, fields,
    ordered::nested::{bridge, program},
    value,
};
use serde_json::json;
impl Sql {
    pub(super) fn peak_utility(
        &mut self,
        utility: Option<usize>,
        body: &Plan,
        input: &Demand,
    ) -> Result<Output, String> {
        // Keep every open/body/close transaction inside one correlated row. A
        // materialized batch of opens would create unrelated LIFO siblings.
        let width = 2 + body.writes.len();
        let mut generation = self.generation.clone();
        generation.return_cells = generation
            .return_cells
            .checked_add(width)
            .filter(|cells| *cells <= 128)
            .ok_or("peak utility return payload exceeds 128 active cells")?;
        generation
            .return_peak
            .set(generation.return_peak.get().max(generation.return_cells));
        let mut nested = Sql {
            definitions: vec![],
            profile: vec![],
            correlated: true,
            generation,
        };
        let columns = fields(&input.slots, |slot| value(input, "d", slot));
        let reached = Demand {
            name: nested.relation(
                "single reached peak utility",
                format!("SELECT 1 AS k,d.file,d.node,d.work{columns}"),
                true,
                "one reached invocation",
            ),
            slots: input.slots.clone(),
        };
        let answer = nested.peak_utility_single(utility, body, &reached)?;
        let payload = fields(&body.writes, |slot| format!("o.c{slot}"));
        let query = format!(
            "WITH RECURSIVE {} SELECT o.truth,o.work{payload} FROM {} o",
            nested.definitions.join(",\n"),
            answer.name
        );
        let exports = body
            .writes
            .iter()
            .enumerate()
            .map(|(index, slot)| format!(",r.c{} AS c{slot}", index + 2))
            .collect::<String>();
        let name = self.relation(
            "serialized peak utility transaction",
            format!(
                "SELECT d.k,r.c0 AS truth,r.c1 AS work{exports} FROM {} d {}",
                input.name,
                bridge(&query, width)
            ),
            true,
            "per-demand peak frame,completed answer,work,typed exports",
        );
        self.profile.last_mut().unwrap()["childPrograms"] = json!({"peakUtility":{"relations":nested.profile,"singleReachedDemand":true,"typedReturnColumns":width}});
        Ok(Output { name })
    }

    fn peak_utility_single(
        &mut self,
        utility: Option<usize>,
        body: &Plan,
        input: &Demand,
    ) -> Result<Output, String> {
        let utility = utility.filter(|_| !self.generation.memo_bypass);
        let memo_layout = self.generation.memo_layout.clone();
        let layout = memo_layout.as_ref();
        let contract = utility.and_then(|id| layout.and_then(|layout| layout.contracts.get(&id)));
        let identity = utility.map(|id| contract.map_or(id, |c| c.identity));
        let column = |s| layout.map_or(s, |layout| layout.column(s));
        let mut keys = if matches!(&body.op, Op::CallFrame { .. }) {
            body.mentions.clone()
        } else {
            input.slots.clone()
        };
        if let Some(layout) = layout {
            let scope = contract.and_then(|c| c.scope.as_ref());
            keys.retain(|s| layout.capture(*s, scope));
        }
        keys.extend(&self.generation.following);
        let key_sql = keys
            .iter()
            .map(|s| {
                let raw = value(input, "d", *s);
                let key = layout.map_or_else(
                    || raw.clone(),
                    |layout| layout.normalize_key(*s, raw.clone()),
                );
                format!(" AND m.k{} IS {}", column(*s), key)
            })
            .collect::<String>();
        let depth = self.generation.depth_expression();
        let clause = self.generation.memo_clause;
        if identity.is_some() && clause.is_none() {
            return Err("peak memo: keyed utility needs a clause lifetime".into());
        }
        let identity_sql=identity.zip(clause).map(|(id,clause)|format!("m.clause IS {clause} AND m.utility IS {id} AND m.file IS d.file AND m.node IS d.node"));
        let exports = body
            .writes
            .iter()
            .map(|s| {
                format!(
                    ",{} AS hit{s}",
                    if identity.is_some() {
                        let stored=format!("m.o{}",column(*s));
                        if layout.is_some_and(|layout|layout.span_capable(*s)){format!("CASE WHEN m.truth IS NOT NULL THEN relation_capture_replay(d.file,{},{stored}) END",value(input,"d",*s))}else{stored}
                    } else {
                        "NULL".into()
                    }
                )
            })
            .collect::<String>();
        let lookup = if let Some(identity_sql) = &identity_sql {
            format!(
                "SELECT d.*,({depth}) AS incoming,m.truth AS hit,m.cost,m.peak{exports} FROM {} d LEFT JOIN relation_memo m ON m.mode IS 0 AND {identity_sql}{key_sql} AND m.peak<=256-({depth})",
                input.name
            )
        } else {
            format!(
                "SELECT d.*,({depth}) AS incoming,NULL AS hit,NULL AS cost,NULL AS peak{exports} FROM {} d",
                input.name
            )
        };
        let lookup = self.relation(
            "peak-aware utility fit lookup",
            lookup,
            true,
            "incoming level,fitting answer or miss,typed exports",
        );
        let opened=self.relation("reached utility peak frame",format!("SELECT d.*,CASE WHEN d.hit IS NULL AND d.incoming<256 THEN relation_peak_open(d.incoming) END AS peak_token FROM {lookup} d"),true,"one numeric peak frame for a reached miss");
        let entered=self.relation("utility entry or replay peak",format!("SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN relation_peak_note(d.incoming+d.peak) WHEN d.peak_token IS NOT NULL THEN relation_peak_note(d.incoming+1) ELSE 1 END AS peak_ready FROM {opened} d"),true,"peak recorded before replay or entry work");
        let miss = "d.hit IS NULL AND d.incoming<256 AND d.work>0 AND d.peak_ready=1";
        let (mut query, profile) = program(
            body,
            input,
            false,
            miss,
            &body.writes,
            ("d.file", "d.node", "d.work-1"),
            self.generation.entered(),
        )?;
        let width = 2 + body.writes.len();
        let (mut return_width, mut fill_col, mut replay_cost) =
            (width, String::new(), "(d.work-d.remaining)");
        if body.effects.cache_fills {
            query = super::file_memo::fill_cost::bracket(&query, width, miss)?;
            return_width += 1;
            fill_col = format!(",COALESCE(r.c{width},0) AS fill_cost");
            replay_cost = "MAX(0,d.work-d.remaining-d.fill_cost)";
        }
        let exports = body
            .writes
            .iter()
            .enumerate()
            .map(|(i, s)| {
                format!(
                    ",CASE WHEN d.hit IS NOT NULL THEN d.hit{s} ELSE r.c{} END AS e{s}",
                    i + 2
                )
            })
            .collect::<String>();
        let tested=self.relation("peak-aware utility reached result",format!("SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.incoming>=256 THEN -1 WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.incoming>=256 THEN d.work WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining{exports}{fill_col} FROM {entered} d {}",bridge(&query,return_width)),true,"measured body answer or fitting replay; depth failure precedes work failure");
        self.profile.last_mut().unwrap()["childPrograms"] = json!({"peakAwareMiss":profile});
        let closed=self.relation("completed utility peak frame",format!("SELECT d.*,CASE WHEN d.peak_token IS NOT NULL THEN relation_peak_close(d.peak_token) ELSE d.peak END AS measured_peak FROM {tested} d"),true,"numeric child peak merged into caller before publication");
        let exports = fields(&body.writes, |s| format!("d.e{s}"));
        let publication = if let Some(identity_sql) = identity_sql {
            let supplied = body
                .writes
                .iter()
                .map(|s| {
                    format!(
                        " AND m.v{} IS CASE WHEN d.tested=1 THEN d.e{s} END",
                        column(*s)
                    )
                })
                .collect::<String>();
            format!(
                "SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth{exports} FROM {closed} d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND {identity_sql}{key_sql} AND m.vtruth IS d.tested AND m.vcost IS {replay_cost} AND m.vpeak IS d.measured_peak{supplied} WHERE d.peak_ready=1 AND COALESCE(d.measured_peak,0)>=0"
            )
        } else {
            format!(
                "SELECT d.k,d.remaining AS work,d.tested AS truth{exports} FROM {closed} d WHERE d.peak_ready=1 AND COALESCE(d.measured_peak,0)>=0"
            )
        };
        let name = self.relation(
            "peak-aware utility completion",
            publication,
            true,
            "completed answer,cost,relative utility peak,typed exports",
        );
        self.profile.last_mut().unwrap()["peakMemo"] = json!({"utility":identity,"fit":"incoming + stored peak <= 256","tooDeep":"miss and recompute","failurePublication":false,"bypass":identity.is_none(),"cost":"spent minus native cache fills"});
        Ok(Output { name })
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Generation, Slots};
    use super::*;
    use crate::code_rule_dsl::sql::relational::{ir::Set, memo_store, row};
    use rusqlite::Connection;
    fn chain(levels: usize) -> Plan {
        let op = if levels == 0 {
            Op::Predicate(Set::Kinds(vec![1]))
        } else {
            Op::Charge(levels, std::rc::Rc::new(chain(levels - 1)))
        };
        Plan {
            effects: super::super::super::effects::Effects::of(&op),
            op,
            mentions: Slots::new(),
            writes: Slots::new(),
            node_writes: Slots::new(),
            walk_filter: None,
        }
    }
    fn query(incoming: u32, work: i64, identity: usize, levels: usize) -> String {
        let mut sql = Sql {
            definitions: vec![],
            profile: vec![],
            correlated: true,
            generation: Generation {
                peak_tracking: true,
                memo_clause: Some(0),
                utility_depth: incoming,
                ..Generation::default()
            },
        };
        let input = Demand {
            name: sql.relation(
                "test demand",
                format!("SELECT 1 AS k,0 AS file,1 AS node,{work} AS work"),
                true,
                "single demand",
            ),
            slots: Slots::new(),
        };
        let out = sql
            .peak_utility(Some(identity), &chain(levels), &input)
            .unwrap();
        format!(
            "WITH RECURSIVE {} SELECT truth,work FROM {}",
            sql.definitions.join(",\n"),
            out.name
        )
    }
    fn database() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        memo_store::register(&db).unwrap();
        row::register(&db).unwrap();
        db.execute_batch("CREATE TABLE node(file,id,kind,named);INSERT INTO node VALUES(0,1,1,1)")
            .unwrap();
        db
    }
    #[test]
    fn fitting_hit_replay_and_too_deep_miss_preserve_measured_peak_and_work() {
        let db = database();
        let run = |depth, work| {
            db.query_row::<(i64, i64), _, _>(&query(depth, work, 99, 2), [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap()
        };
        assert_eq!(run(253, 10), (1, 7));
        assert_eq!(db.query_row::<i64,_,_>("SELECT peak FROM relation_memo WHERE mode IS 0 AND clause IS 0 AND utility IS 99 AND file IS 0 AND node IS 1",[],|r|r.get(0)).unwrap(),3);
        assert_eq!(run(254, 10), (-1, 8));
        assert_eq!(run(253, 10), (1, 7));
        let token: i64 = db
            .query_row("SELECT relation_peak_open(0)", [], |r| r.get(0))
            .unwrap();
        assert_eq!(run(4, 2), (-2, 0));
        assert_eq!(
            db.query_row::<i64, _, _>("SELECT relation_peak_close(?1)", [token], |r| r.get(0))
                .unwrap(),
            7
        );
    }
    #[test]
    fn batched_sibling_demands_serialize_open_body_close_transactions() {
        let db = database();
        let sql = query(0, 10, 99, 2).replace(
            "SELECT 1 AS k,0 AS file,1 AS node,10 AS work",
            "SELECT 1 AS k,0 AS file,1 AS node,10 AS work UNION ALL SELECT 2,0,1,10",
        );
        let rows = db
            .prepare(&sql)
            .unwrap()
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(rows, vec![(1, 7), (1, 7)]);
        let token: i64 = db
            .query_row("SELECT relation_peak_open(0)", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            db.query_row::<i64, _, _>("SELECT relation_peak_close(?1)", [token], |r| r.get(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn work_zero_and_depth_failure_close_every_miss_frame_without_publishing() {
        let db = database();
        for (depth, expected) in [(4, -2), (256, -1)] {
            let answer: (i64, i64) = db
                .query_row(&query(depth, 0, 100, 0), [], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap();
            assert_eq!(answer, (expected, 0));
            assert!(matches!(db.query_row::<i64,_,_>("SELECT peak FROM relation_memo WHERE mode IS 0 AND clause IS 0 AND utility IS 100 AND file IS 0 AND node IS 1",[],|r|r.get(0)),Err(rusqlite::Error::QueryReturnedNoRows)));
            let token: i64 = db
                .query_row("SELECT relation_peak_open(0)", [], |r| r.get(0))
                .unwrap();
            assert_eq!(
                db.query_row::<i64, _, _>("SELECT relation_peak_close(?1)", [token], |r| r.get(0))
                    .unwrap(),
                0
            );
        }
    }
}
