//! Three-valued recursive SQL control; captures remain immutable in the outer demand.
use super::super::lower::mixed_recursive_lower::{Direction, Instruction, Program, Walk};
use super::{
    Demand, Output, Sql,
    ordered::nested::{bridge, program},
    value,
};
use crate::code_rule_dsl::sql::relational::{emit::condition, ir::Set};
use serde_json::json;
fn predicate(p: Option<&Set>, alias: &str, default: &str) -> Result<String, String> {
    p.map(|p| condition(p, alias))
        .unwrap_or_else(|| Ok(default.into()))
}
fn first(w: &Walk) -> Result<String, String> {
    let filter = if w.stop.is_none() {
        predicate(w.filter.as_ref(), "candidate", "1")?
    } else {
        "1".into()
    };
    Ok(match w.direction {
        Direction::Child => {
            let domain = if let Some(field) = w.field {
                format!(
                    "candidate.parent=s.node AND candidate.field={field} AND candidate.ffirst=1"
                )
            } else if w.end {
                "candidate.id>s.node AND candidate.id<=n.\"last\"".into()
            } else {
                "candidate.parent=s.node".into()
            };
            format!(
                "(SELECT candidate.id FROM node candidate WHERE candidate.file=d.file AND ({domain}) AND ({filter}) ORDER BY candidate.id LIMIT 1)"
            )
        }
        Direction::Parent => w.field.map_or("n.parent".into(), |field| {
            format!("CASE WHEN n.field={field} AND n.ffirst=1 THEN n.parent END")
        }),
        Direction::Previous => "n.prev".into(),
        Direction::Next => "n.next".into(),
    })
}
fn next(w: &Walk) -> Result<String, String> {
    if w.field.is_some() || (!w.end && !matches!(w.direction, Direction::Child)) {
        return Ok("NULL".into());
    }
    let stop = predicate(w.stop.as_ref(), "n", "0")?;
    Ok(match w.direction {
        Direction::Child => {
            let filter = if w.stop.is_none() {
                predicate(w.filter.as_ref(), "candidate", "1")?
            } else {
                "1".into()
            };
            let domain = if w.end {
                format!(
                    "candidate.id>CASE WHEN ({stop}) THEN n.\"last\" ELSE p.c4 END AND candidate.id<=p.c5"
                )
            } else {
                "candidate.parent=p.c1 AND candidate.id>n.\"last\" AND candidate.id<=p.c5".into()
            };
            format!(
                "(SELECT candidate.id FROM node candidate WHERE candidate.file=d.file AND ({domain}) AND ({filter}) ORDER BY candidate.id LIMIT 1)"
            )
        }
        Direction::Parent => format!("CASE WHEN NOT ({stop}) THEN n.parent END"),
        Direction::Previous => format!("CASE WHEN NOT ({stop}) THEN n.prev END"),
        Direction::Next => format!("CASE WHEN NOT ({stop}) THEN n.next END"),
    })
}
fn push(alias: &str, guard: &str, parent: &str, cells: &[&str]) -> String {
    let cells = cells
        .iter()
        .enumerate()
        .map(|(i, c)| format!(" AND {alias}.v{i} IS ({c})"))
        .collect::<String>();
    format!(
        "LEFT JOIN relation_continuation {alias} ON {alias}.mode IS CASE WHEN {guard} THEN 1 END AND {alias}.parent_input IS {parent}{cells}"
    )
}
fn cases(cases: &[String], key: &str, default: &str) -> String {
    if cases.is_empty() {
        default.into()
    } else {
        format!("CASE {key} {} ELSE {default} END", cases.join(" "))
    }
}
impl Sql {
    pub(super) fn mixed_recursive(
        &mut self,
        p: &Program,
        entry: usize,
        input: &Demand,
    ) -> Result<Output, String> {
        let alias = format!("mixed_entry_{}", p.functions[0].memo);
        let mut code = vec![];
        let mut booleans = vec![];
        let mut tests = vec![];
        let mut starts = vec![];
        let mut advances = vec![];
        let mut filters = vec![];
        let mut regions = vec![];
        for (pc, i) in p.code.iter().enumerate() {
            self.generation.operator_as("mixed recursive control")?;
            let (op, a) = match i {
                Instruction::Dispatch => (0, 0),
                Instruction::Failure => (1, 0),
                Instruction::Test(set) => {
                    tests.push(format!("WHEN {pc} THEN {}", condition(set, "n")?));
                    (2, 0)
                }
                Instruction::Eval(id) => {
                    regions.push((pc, *id));
                    (3, *id)
                }
                Instruction::Call(id) => (4, *id),
                Instruction::Boolean { all, children } => {
                    for (index, child) in children.iter().enumerate() {
                        booleans.push(format!("({pc},{index},{child})"));
                    }
                    (5, usize::from(*all))
                }
                Instruction::Not(child) => (6, *child),
                Instruction::Search { walk, body } => {
                    starts.push(format!("WHEN {pc} THEN {}", first(walk)?));
                    advances.push(format!("WHEN {pc} THEN {}", next(walk)?));
                    let filter =
                        if matches!(walk.direction, Direction::Child) && walk.stop.is_none() {
                            "1".into()
                        } else {
                            predicate(walk.filter.as_ref(), "n", "1")?
                        };
                    filters.push(format!("WHEN {pc} THEN {filter}"));
                    (7, *body)
                }
            };
            code.push(format!("({pc},{op},{a})"));
        }
        let root_call = p.code.len();
        self.generation.operator_as("mixed recursive control")?;
        code.push(format!("({root_call},4,{entry})"));
        let code = self.relation(
            "mixed recursive control",
            format!(
                "SELECT column1 AS pc,column2 AS op,column3 AS a FROM (VALUES {})",
                code.join(",")
            ),
            true,
            "immutable finite control",
        );
        let children = self.relation(
            "mixed Boolean children",
            if booleans.is_empty() {
                "SELECT NULL AS pc,NULL AS ordinal,NULL AS child WHERE 0".into()
            } else {
                format!(
                    "SELECT column1 AS pc,column2 AS ordinal,column3 AS child FROM (VALUES {})",
                    booleans.join(",")
                )
            },
            true,
            "immutable ordered Boolean edges",
        );
        let layout = self.generation.memo_layout.clone();
        let column = |s| layout.as_ref().map_or(s, |l| l.column(s));
        let mut functions = vec![];
        for (id, f) in p.functions.iter().enumerate() {
            let identity = layout
                .as_ref()
                .and_then(|l| l.contracts.get(&f.memo))
                .map_or(f.memo, |c| c.identity);
            functions.push(format!("({id},{},{identity})", f.entry));
        }
        let functions = self.relation(
            "mixed utility entries",
            format!(
                "SELECT column1 AS id,column2 AS entry,column3 AS memo FROM (VALUES {})",
                functions.join(",")
            ),
            true,
            "original utility identity,entry control",
        );
        let mut slots = input.slots.clone();
        if let Some(l) = &layout {
            slots.retain(|s| l.capture(*s, None));
        }
        slots.extend(&self.generation.following);
        let key_sql = slots
            .iter()
            .map(|s| {
                let raw = value(input, "d", *s);
                let key = layout
                    .as_ref()
                    .map_or_else(|| raw.clone(), |l| l.normalize_key(*s, raw.clone()));
                let allowed = p
                    .functions
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| {
                        self.generation.following.contains(s)
                            || layout.as_ref().is_none_or(|l| {
                                l.capture(
                                    *s,
                                    l.contracts.get(&f.memo).and_then(|c| c.scope.as_ref()),
                                )
                            })
                    })
                    .map(|(id, _)| format!("WHEN {id} THEN {key}"))
                    .collect::<Vec<_>>();
                format!(
                    " AND m.k{} IS {}",
                    column(*s),
                    cases(&allowed, "f.id", "NULL")
                )
            })
            .collect::<String>();
        let clause = self
            .generation
            .memo_clause
            .ok_or("mixed recursion: missing memo lifetime")?;
        let bypass = p.bypass || self.generation.memo_bypass;
        let caching = if bypass { "0" } else { "1" };
        let entry_identity = format!(
            "m.clause IS {clause} AND m.utility IS f.memo AND m.file IS d.file AND m.node IS s.node"
        );
        let return_identity = format!(
            "m.clause IS {clause} AND m.utility IS f.memo AND m.file IS d.file AND m.node IS p.c1"
        );
        let seed = |predicate: &str| {
            format!(
                "SELECT {alias}.node AS node,{alias}.pc AS pc,{alias}.level AS level,{alias}.work AS work,{alias}.handle AS handle,{alias}.truth AS truth WHERE {predicate}"
            )
        };
        let enter = format!(
            r#"WITH reached AS MATERIALIZED ({}),
s AS MATERIALIZED (SELECT reached.*,f.id AS function,f.entry FROM reached JOIN {code} i ON i.pc=reached.pc JOIN {functions} f ON f.id=i.a),
lookup AS MATERIALIZED (SELECT s.*,m.truth AS hit,m.cost,m.peak FROM s JOIN {functions} f ON f.id=s.function LEFT JOIN relation_memo m ON m.mode IS CASE WHEN {caching} THEN 0 END AND {entry_identity}{key_sql} AND m.peak<=256-s.level),
opened AS MATERIALIZED (SELECT s.*,CASE WHEN s.hit IS NULL AND s.level<256 THEN relation_peak_open(s.level) END AS token FROM lookup s),
noted AS MATERIALIZED (SELECT s.*,CASE WHEN s.hit IS NOT NULL THEN relation_peak_note(s.level+s.peak) WHEN s.token IS NOT NULL THEN relation_peak_note(s.level+1) ELSE 1 END AS ready FROM opened s)
SELECT s.node,CASE WHEN s.hit IS NOT NULL OR s.level>=256 THEN 0 WHEN s.work=0 THEN 1 ELSE s.entry END,
CASE WHEN s.hit IS NULL AND s.level<256 THEN s.level+1 ELSE s.level END,
CASE WHEN s.hit IS NOT NULL THEN MAX(0,s.work-s.cost) WHEN s.level<256 AND s.work>0 THEN s.work-1 ELSE s.work END,
COALESCE(frame.handle,s.handle),CASE WHEN s.hit IS NOT NULL THEN CASE WHEN s.work<s.cost THEN -2 ELSE s.hit END WHEN s.level>=256 THEN -1 WHEN s.work=0 THEN -2 ELSE 0 END
FROM noted s {} WHERE s.ready=1"#,
            seed(&format!("{alias}.pc IN (SELECT pc FROM {code} WHERE op=4)")),
            push(
                "frame",
                "s.hit IS NULL AND s.level<256 AND s.ready=1",
                "s.handle",
                &[
                    "0",
                    "s.node",
                    "s.level",
                    "s.function",
                    "s.work",
                    "relation_cache_fill_steps()",
                    "s.token"
                ]
            )
        );
        let test = format!(
            "WITH s AS MATERIALIZED ({}) SELECT s.node,0,s.level,s.work,s.handle,CASE WHEN ({}) THEN 1 ELSE 0 END FROM s JOIN node n ON n.file=d.file AND n.id=s.node",
            seed(&format!("{alias}.pc IN (SELECT pc FROM {code} WHERE op=2)")),
            cases(&tests, "s.pc", "0")
        );
        let bool_start = format!(
            "WITH s AS MATERIALIZED ({}), i AS MATERIALIZED (SELECT s.*,i.a,(SELECT child FROM {children} WHERE pc=s.pc AND ordinal=0) AS child FROM s JOIN {code} i ON i.pc=s.pc) SELECT s.node,COALESCE(s.child,0),s.level,s.work,COALESCE(frame.handle,s.handle),CASE WHEN s.child IS NULL THEN s.a ELSE 0 END FROM i s {}",
            seed(&format!("{alias}.pc IN (SELECT pc FROM {code} WHERE op=5)")),
            push(
                "frame",
                "s.child IS NOT NULL",
                "s.handle",
                &[
                    "CASE WHEN s.a=1 THEN 2 ELSE 3 END",
                    "s.node",
                    "s.level",
                    "s.pc",
                    "0",
                    "0"
                ]
            )
        );
        let not_start = format!(
            "WITH s AS MATERIALIZED ({}) SELECT s.node,i.a,s.level,s.work,frame.handle,0 FROM s JOIN {code} i ON i.pc=s.pc {}",
            seed(&format!("{alias}.pc IN (SELECT pc FROM {code} WHERE op=6)")),
            push("frame", "1", "s.handle", &["4", "s.node", "s.level"])
        );
        let search_start = format!(
            "WITH reached AS MATERIALIZED ({}),s AS MATERIALIZED (SELECT s.*,n.\"last\" AS bound,{} AS child FROM reached s JOIN node n ON n.file=d.file AND n.id=s.node) SELECT COALESCE(s.child,s.node),CASE WHEN s.child IS NULL THEN 0 ELSE -2 END,s.level,s.work,COALESCE(frame.handle,s.handle),0 FROM s {}",
            seed(&format!("{alias}.pc IN (SELECT pc FROM {code} WHERE op=7)")),
            cases(&starts, "s.pc", "NULL"),
            push(
                "frame",
                "s.child IS NOT NULL",
                "s.handle",
                &["1", "s.node", "s.level", "s.pc", "s.child", "s.bound", "0"]
            )
        );
        let admit = format!(
            "WITH s AS MATERIALIZED ({}) SELECT s.node,CASE WHEN ({}) THEN i.a ELSE 0 END,s.level,s.work,s.handle,0 FROM s CROSS JOIN relation_continuation p ON p.mode IS 0 AND p.h IS s.handle JOIN {code} i ON i.pc=p.c3 JOIN node n ON n.file=d.file AND n.id=s.node",
            seed(&format!("{alias}.pc=-2")),
            cases(&filters, "p.c3", "1")
        );
        let bool_advance = push(
            "bool_again",
            "p.c0 IN (2,3) AND p.truth>=0 AND NOT ((p.c0=2 AND p.truth=0) OR (p.c0=3 AND p.truth=1)) AND p.child IS NOT NULL",
            "p.parent",
            &[
                "p.c0",
                "p.c1",
                "p.c2",
                "p.c3",
                "p.c4+1",
                "p.c5 OR p.truth=2",
            ],
        );
        let search_advance = push(
            "search_again",
            "p.c0=1 AND p.truth IN (0,2) AND p.candidate IS NOT NULL",
            "p.parent",
            &[
                "1",
                "p.c1",
                "p.c2",
                "p.c3",
                "p.candidate",
                "p.c5",
                "p.c6 OR p.truth=2",
            ],
        );
        // Keep one consumer of the pop/close/publication chain. Correlated CTEs
        // referenced by separate UNION branches can instantiate that chain more
        // than once, even with AS MATERIALIZED, consuming the frame twice.
        let dispatch = format!(
            r#"WITH reached AS MATERIALIZED ({}),
popped AS MATERIALIZED (SELECT s.*,frame.parent,frame.c0,frame.c1,frame.c2,frame.c3,frame.c4,frame.c5,frame.c6 FROM reached s LEFT JOIN relation_continuation frame ON frame.mode IS CASE WHEN s.handle IS NOT NULL THEN 2 END AND frame.h IS s.handle),
closed AS MATERIALIZED (SELECT p.*,CASE WHEN p.c0=0 THEN relation_peak_close(p.c6) END AS measured FROM popped p),
published AS MATERIALIZED (SELECT p.*,COALESCE(m.truth,p.truth) AS returned FROM closed p LEFT JOIN {functions} f ON p.c0=0 AND f.id=p.c3 LEFT JOIN relation_memo m ON m.mode IS CASE WHEN {caching} AND p.c0=0 AND p.truth>=0 THEN 1 END AND {return_identity}{key_sql} AND m.vtruth IS p.truth AND m.vcost IS CASE WHEN p.c0=0 AND p.truth>=0 THEN MAX(0,p.c4-p.work-relation_cache_fill_delta(p.c5)) END AND m.vpeak IS p.measured WHERE COALESCE(p.measured,0)>=0),
classified AS MATERIALIZED (SELECT p.*,CASE WHEN p.c0 IN (2,3) AND p.truth>=0 AND NOT ((p.c0=2 AND p.truth=0) OR (p.c0=3 AND p.truth=1)) THEN (SELECT child FROM {children} WHERE pc=p.c3 AND ordinal=p.c4+1) END AS child,CASE WHEN p.c0=1 AND p.truth IN (0,2) THEN {} END AS candidate FROM published p LEFT JOIN node n ON p.c0=1 AND n.file=d.file AND n.id=p.c4)
SELECT CASE WHEN p.handle IS NULL THEN p.node WHEN p.c0=1 AND search_again.handle IS NOT NULL THEN p.candidate ELSE p.c1 END,
CASE WHEN p.handle IS NULL THEN -1 WHEN bool_again.handle IS NOT NULL THEN p.child WHEN search_again.handle IS NOT NULL THEN -2 ELSE 0 END,
CASE WHEN p.handle IS NULL THEN p.level ELSE p.c2 END,p.work,
COALESCE(bool_again.handle,search_again.handle,p.parent),
CASE WHEN p.handle IS NULL THEN p.truth WHEN p.c0=0 THEN p.returned WHEN p.c0=4 THEN CASE p.truth WHEN 0 THEN 1 WHEN 1 THEN 0 ELSE p.truth END
WHEN bool_again.handle IS NOT NULL OR search_again.handle IS NOT NULL THEN 0 WHEN p.truth<0 THEN p.truth
WHEN p.c0=2 AND p.truth=0 THEN 0 WHEN p.c0=3 AND p.truth=1 THEN 1
WHEN p.c0 IN (2,3) THEN CASE WHEN p.c5 OR p.truth=2 THEN 2 WHEN p.c0=2 THEN 1 ELSE 0 END
WHEN p.c0=1 THEN CASE WHEN p.truth=1 THEN 1 WHEN p.c6 OR p.truth=2 THEN 2 ELSE 0 END END
FROM classified p {bool_advance} {search_advance}"#,
            seed(&format!("{alias}.pc=0")),
            cases(&advances, "p.c3", "NULL")
        );
        let mut generation = self.generation.clone();
        generation.return_cells = generation
            .return_cells
            .checked_add(8)
            .filter(|cells| *cells <= 128)
            .ok_or("mixed recursion: control return payload exceeds 128 active cells")?;
        generation
            .return_peak
            .set(generation.return_peak.get().max(generation.return_cells));
        generation.dynamic_depth = Some(format!("{alias}.level"));
        generation.utility_depth = 0;
        generation.memo_bypass |= p.bypass;
        let mut phase_queries = vec![
            (
                enter,
                format!("{alias}.pc IN (SELECT pc FROM {code} WHERE op=4)"),
            ),
            (
                test,
                format!("{alias}.pc IN (SELECT pc FROM {code} WHERE op=2)"),
            ),
            (
                bool_start,
                format!("{alias}.pc IN (SELECT pc FROM {code} WHERE op=5)"),
            ),
            (
                not_start,
                format!("{alias}.pc IN (SELECT pc FROM {code} WHERE op=6)"),
            ),
            (
                search_start,
                format!("{alias}.pc IN (SELECT pc FROM {code} WHERE op=7)"),
            ),
            (admit, format!("{alias}.pc=-2")),
            (dispatch, format!("{alias}.pc=0")),
        ];
        let mut region_profiles = vec![];
        for (pc, id) in regions {
            let (q, profile) = program(
                &p.regions[id],
                input,
                false,
                &format!("{alias}.pc={pc}"),
                &super::Slots::new(),
                ("d.file", &format!("{alias}.node"), &format!("{alias}.work")),
                generation.clone(),
            )?;
            let truth = if p.three {
                "r.c0"
            } else {
                "CASE WHEN r.c0=2 THEN 0 ELSE r.c0 END"
            };
            phase_queries.push((format!("WITH s AS MATERIALIZED ({}) SELECT s.node,0,s.level,r.c1,s.handle,{truth} FROM s {}",seed(&format!("{alias}.pc={pc}")),bridge(&q,2)),format!("{alias}.pc={pc}")));
            region_profiles.push(profile);
        }
        let depth = self.generation.depth_expression();
        let mut arms = vec![format!(
            "SELECT d.node AS node,{root_call} AS pc,({depth}) AS level,d.work AS work,NULL AS handle,0 AS truth"
        )];
        arms.push(format!("SELECT {alias}.node,0,{alias}.level,{alias}.work,{alias}.handle,{alias}.truth FROM schedule {alias} WHERE {alias}.pc=1"));
        for (q, reached) in phase_queries {
            arms.push(format!(
                "SELECT r.c0,r.c1,r.c2,r.c3,r.c4,r.c5 FROM schedule {alias} {} WHERE {reached}",
                bridge(&q, 6)
            ));
        }
        let q = format!(
            "WITH RECURSIVE schedule AS ({}) SELECT truth,work FROM schedule WHERE pc=-1",
            arms.join("\nUNION ALL\n")
        );
        let name = self.relation(
            "mixed recursive execution",
            format!(
                "SELECT d.k,r.c1 AS work,r.c0 AS truth FROM {} d {}",
                input.name,
                bridge(&q, 2)
            ),
            true,
            "one immutable capture context; six numeric recursive columns",
        );
        self.profile.last_mut().unwrap()["mixedControl"] = json!({"instructions":p.code.len()+1,"functions":p.functions.len(),"regions":region_profiles,"recursiveColumns":6,"continuationCells":7,"captureStackCopies":false,"progress":"monotone SCC; zero-progress edges acyclic","nativeComposition":false,"bypass":bypass});
        Ok(Output { name })
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::{lower::Lower, publication_tests::policy_utils};
    use super::super::{Generation, Op, Slots};
    use super::*;
    use crate::code_rule_dsl::{
        matcher::node_at,
        sql::{exec, tests::differential::tape},
        tape::TapeDoc,
        test_support::grammar_matchers,
    };
    use ast_grep_core::Node;
    use std::collections::BTreeMap;
    fn body() -> serde_json::Value {
        json!({"any":[{"kind":"number","value":{"gt":0}},{"kind":"identifier","value":{"gt":0}},{"kind":"array","not":{"has":{"stopBy":"neighbor","not":{"any":[{"regex":"^[\\[\\],]$"},{"matches":"u"}]}}}}]})
    }
    #[derive(Clone, Copy)]
    struct Answer {
        truth: i64,
        cost: i64,
        levels: u32,
    }
    fn oracle(
        node: Node<'_, TapeDoc>,
        depth: u32,
        work: &mut i64,
        peak: &mut u32,
        memo: &mut BTreeMap<usize, Answer>,
    ) -> i64 {
        if let Some(a) = memo
            .get(&node.node_id())
            .copied()
            .filter(|a| depth + a.levels <= 256)
        {
            *peak = (*peak).max(depth + a.levels);
            if *work < a.cost {
                *work = 0;
                return -2;
            }
            *work -= a.cost;
            return a.truth;
        }
        if depth >= 256 {
            return -1;
        }
        let start = *work;
        let mut entered = depth + 1;
        let truth = (|| {
            if *work == 0 {
                return -2;
            }
            *work -= 1;
            if matches!(node.kind().as_ref(), "number" | "identifier") {
                if *work == 0 {
                    return -4;
                }
                *work -= 1;
                if node.kind() == "identifier" {
                    return 2;
                }
                return i64::from(node.text() != "0");
            }
            if node.kind() != "array" {
                return 0;
            }
            let mut unknown = false;
            for child in node.children() {
                if matches!(child.text().as_ref(), "[" | "]" | ",") {
                    continue;
                }
                let answer = oracle(child, depth + 1, work, &mut entered, memo);
                if answer <= 0 {
                    return answer;
                }
                unknown |= answer == 2;
            }
            if unknown { 2 } else { 1 }
        })();
        *peak = (*peak).max(entered);
        if truth >= 0 {
            memo.insert(
                node.node_id(),
                Answer {
                    truth,
                    cost: start - *work,
                    levels: entered - depth,
                },
            );
        }
        truth
    }
    #[test]
    fn mixed_completed_memo_peaks_match_entry_oracle_at_static_and_dynamic_depths() {
        let p = policy_utils(&[json!({"matches":"u"})], json!({"u":body()}));
        let g = &grammar_matchers(&p)[0];
        for dynamic in [false, true] {
            let mut lower = Lower::new(g.language, true);
            let plan = lower.plan(&g.matcher).unwrap();
            assert!(matches!(&plan.op, Op::MixedRecursive { .. }));
            let layout = lower.memo_layout(true);
            exec::with_terminals(
                vec![(tape(g.language, "f([1,[1,x],0]);"), Default::default())],
                lower.pure.program.terminals,
                |db, batch, _| {
                    super::super::super::super::leaves::register(db, batch).unwrap();
                    let file = &batch.files[0];
                    let root = (0..file.facts.len())
                        .find(|id| node_at(&file.root, *id).kind() == "array")
                        .unwrap();
                    let mut memo = BTreeMap::new();
                    for (incoming, allowance) in [
                        (0, 2),
                        (0, 1000),
                        (254, 1000),
                        (253, 1000),
                        (255, 0),
                        (256, 0),
                        (0, 2),
                        (0, 1000),
                    ] {
                        let mut sql = Sql {
                            definitions: vec![],
                            profile: vec![],
                            correlated: false,
                            generation: Generation {
                                peak_tracking: true,
                                memo_clause: Some(0),
                                memo_layout: layout.clone(),
                                utility_depth: if dynamic { 0 } else { incoming },
                                dynamic_depth: dynamic.then(|| "?1".into()),
                                ..Generation::default()
                            },
                        };
                        let input = Demand {
                            name: sql.relation(
                                "test mixed invocation",
                                format!(
                                    "SELECT 1 AS k,0 AS file,{root} AS node,{allowance} AS work"
                                ),
                                true,
                                "single input",
                            ),
                            slots: Slots::new(),
                        };
                        let out = sql.emit(&plan, &input).unwrap();
                        let query = format!(
                            "WITH RECURSIVE {} SELECT truth,work FROM {}",
                            sql.definitions.join(",\n"),
                            out.name
                        );
                        let token: i64 = db
                            .query_row("SELECT relation_peak_open(0)", [], |r| r.get(0))
                            .unwrap();
                        let row =
                            |r: &rusqlite::Row<'_>| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?));
                        let actual = if dynamic {
                            db.query_row(&query, [incoming], row)
                        } else {
                            db.query_row(&query, [], row)
                        }
                        .unwrap();
                        let peak: i64 = db
                            .query_row("SELECT relation_peak_close(?1)", [token], |r| r.get(0))
                            .unwrap();
                        let mut work = allowance;
                        let mut expected_peak = 0;
                        let truth = oracle(
                            node_at(&file.root, root),
                            incoming,
                            &mut work,
                            &mut expected_peak,
                            &mut memo,
                        );
                        assert_eq!(
                            (actual, peak),
                            ((truth, work), i64::from(expected_peak)),
                            "dynamic={dynamic}, incoming={incoming}, work={allowance}"
                        );
                        assert_eq!(
                            db.query_row::<i64, _, _>(
                                "SELECT relation_continuation_rows()",
                                [],
                                |r| r.get(0)
                            )
                            .unwrap(),
                            0
                        );
                        assert!(
                            db.query_row::<i64, _, _>(
                                "SELECT relation_continuation_peak()",
                                [],
                                |r| r.get(0)
                            )
                            .unwrap()
                                < 64
                        );
                    }
                },
            );
        }
    }
    #[test]
    fn mixed_phase_and_region_typed_returns_obey_existing_active_cell_budget() {
        let p = policy_utils(&[json!({"matches":"u"})], json!({"u":body()}));
        let g = &grammar_matchers(&p)[0];
        let mut lower = Lower::new(g.language, true);
        let plan = lower.plan(&g.matcher).unwrap();
        for (initial, accepted) in [(118, true), (121, false)] {
            let mut sql = Sql {
                definitions: vec![],
                profile: vec![],
                correlated: false,
                generation: Generation {
                    peak_tracking: true,
                    memo_clause: Some(0),
                    return_cells: initial,
                    memo_layout: lower.memo_layout(true),
                    ..Generation::default()
                },
            };
            let input = Demand {
                name: "test_input".into(),
                slots: Slots::new(),
            };
            let result = sql.emit(&plan, &input);
            assert_eq!(result.is_ok(), accepted);
            if let Err(error) = result {
                assert!(error.contains("128"), "{error}");
            }
        }
    }
}
