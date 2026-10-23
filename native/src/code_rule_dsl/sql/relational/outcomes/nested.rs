//! Correlated child programs return typed columns once, before the parent resumes.
//! Only observable child exports cross the recursive boundary; inputs stay in keyed demand rows.
use super::super::super::Slots;
use super::super::{fields, needed, value};
use super::{Axis, Bounds, Demand, Op, Output, Plan, Sql};
use serde_json::{Value, json};

pub(super) fn contains_walk(plan: &Plan) -> bool {
    plan.effects.walk_depth > 0
}
fn check_depth(plan: &Plan, walks: usize) -> Result<(), String> {
    if walks + plan.effects.walk_depth > 4 {
        return Err("nested ordered traversal depth exceeds 4".into());
    }
    Ok(())
}

/// The outer SELECT binds w, n, and (for caller captures) d. Inner CTE names have lexical scope.
pub(in crate::code_rule_dsl::sql::relational::outcomes) fn program(
    plan: &Plan,
    input: &Demand,
    fresh: bool,
    guard: &str,
    exports: &Slots,
    location: (&str, &str, &str),
    generation: super::super::Generation,
) -> Result<(String, Value), String> {
    let (file, node, work) = location;
    // A node-local child need not allocate a demand/result CTE pair.
    // Unknown or binding prefixes keep the general typed composition contract.
    if plan.mentions.is_empty() && plan.writes.is_empty() && exports.is_empty() {
        let local_value = match &plan.op {
            Op::Value(term) => Some((*term, "1".to_string())),
            Op::All(items) => {
                if let Some((last, prefix)) = items.split_last()
                    && let Op::Value(term) = last.op
                {
                    let predicates = prefix
                        .iter()
                        .map(|p| super::boolean(p, "probe"))
                        .collect::<Result<Option<Vec<_>>, _>>()?;
                    predicates.map(|terms| {
                        (
                            term,
                            if terms.is_empty() {
                                "1".into()
                            } else {
                                terms
                                    .into_iter()
                                    .map(|t| format!("({t})"))
                                    .collect::<Vec<_>>()
                                    .join(" AND ")
                            },
                        )
                    })
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some((term, prefix)) = local_value {
            generation.tree(plan)?;
            return Ok((
                format!(
                    "SELECT COALESCE(v.truth,0),COALESCE(v.work,{work}) FROM node probe LEFT JOIN relation_value_budget({term},{file},{node},CASE WHEN ({guard}) AND ({prefix}) THEN {work} END) v ON 1 WHERE probe.file={file} AND probe.id={node} AND ({guard})"
                ),
                json!({"operator":"guarded node-local value return","singleReachedDemand":true,"exports":exports,"typedReturnColumns":2}),
            ));
        }
        if !plan.fallible()
            && let Ok(test) = super::truth(plan, 0, "probe")
        {
            generation.tree(plan)?;
            return Ok((
                format!(
                    "SELECT {test},{work} FROM node probe WHERE probe.file={file} AND probe.id={node} AND ({guard})"
                ),
                json!({"operator":"node-local total return","singleReachedDemand":true,"exports":exports,"typedReturnColumns":2}),
            ));
        }
    }
    let mut sql = Sql {
        definitions: vec![],
        profile: vec![],
        correlated: true,
        generation,
    };
    let slots = if fresh {
        Slots::new()
    } else if plan.uses_memo() {
        input.slots.clone()
    } else {
        needed(plan).intersection(&input.slots).copied().collect()
    };
    let columns = fields(&slots, |slot| value(input, "d", slot));
    let demand = Demand {
        name: sql.relation(
            "correlated child demand",
            format!(
                "SELECT 1 AS k,{file} AS file,{node} AS node,{work} AS work{columns} WHERE {guard}"
            ),
            true,
            "k,file,node,live input captures",
        ),
        slots,
    };
    let output = sql.emit(plan, &demand)?;
    let payload = fields(exports, |slot| format!("o.c{slot}"));
    let query = format!(
        "WITH RECURSIVE {} SELECT o.truth,o.work{payload} FROM {} o",
        sql.definitions.join(",\n"),
        output.name
    );
    Ok((
        query,
        json!({"relations":sql.profile,"exports":exports,"singleReachedDemand":true}),
    ))
}
pub(in crate::code_rule_dsl::sql::relational::outcomes) fn bridge(
    query: &str,
    width: usize,
) -> String {
    let columns = (0..width)
        .map(|i| format!("r.c{i}"))
        .collect::<Vec<_>>()
        .join(",");
    format!("CROSS JOIN relation_row r ON ({columns}) IS ({query})")
}

pub(super) fn emit(
    sql: &mut Sql,
    axis: Axis,
    end: bool,
    rule: &Plan,
    stop: Option<&Plan>,
    bounds: Option<Bounds>,
    input: &Demand,
) -> Result<Output, String> {
    check_depth(rule, 1)?;
    if let Some(stop) = stop {
        check_depth(stop, 1)?;
    }
    let exports = if bounds.is_none() {
        rule.writes.clone()
    } else {
        Slots::new()
    };
    let strict = bounds.is_none() && !rule.writes.is_empty();
    let child = matches!(axis, Axis::Child);
    // Native bounded descendant walks exclude both kinds and required text before
    // testing a body. The node cursor delivers bounded kind postings in id order.
    // Seek the next eligible candidate instead of allocating three recursive rows
    // (and a typed child return) for every irrelevant AST node.
    // Three-valued native walks disable their general postings filter. A pure
    // prefix can still prove a candidate false before any charge, value, or write
    // becomes observable. This physical proof does not skip charged count visits
    // or stop rules. Restrict the new shape to clauses with traversal memo scope.
    let prefix_kinds = sql.generation.memo_clause.is_some() && super::super::kinds(rule).is_some();
    let filter = if rule.walk_filter.is_some() {
        super::candidate_filter(rule, "candidate")
    } else if prefix_kinds {
        super::super::kind_filter(rule, "candidate")
    } else {
        "1".into()
    };
    let seek = child
        && end
        && stop.is_none()
        && bounds.is_none()
        && (rule.walk_filter.is_some() || prefix_kinds);
    let seek_after = |file: &str, after: &str, bound: &str| {
        format!(
            "(SELECT candidate.id FROM node candidate WHERE candidate.file={file} AND candidate.id>{after} AND candidate.id<={bound} AND ({}) ORDER BY candidate.id LIMIT 1)",
            filter
        )
    };
    let first = match axis {
        Axis::Child => "CASE WHEN origin.id<origin.\"last\" THEN origin.id+1 END",
        Axis::Parent => "origin.parent",
        Axis::Previous => "origin.prev",
        Axis::Next => "origin.next",
    };
    let first = if seek {
        seek_after("origin.file", "origin.id", "origin.\"last\"")
    } else {
        first.into()
    };
    let next = match (axis, end) {
        (Axis::Child, false) => "n.next".into(),
        (Axis::Child, true) => "CASE WHEN w.stopped=0 THEN n.id+1 ELSE n.\"last\"+1 END".into(),
        (_, false) => "NULL".into(),
        (axis, true) => format!(
            "CASE WHEN w.stopped=0 THEN n.{} END",
            match axis {
                Axis::Parent => "parent",
                Axis::Previous => "prev",
                Axis::Next => "next",
                _ => unreachable!(),
            }
        ),
    };
    let next = if seek {
        seek_after("w.file", "w.node", "w.bound")
    } else if child {
        format!("CASE WHEN ({next})<=w.bound THEN ({next}) END")
    } else {
        next
    };
    let low = "(w.low+(w.own=1))";
    let uncertain = "(w.uncertain+(w.own=2))";
    let open = "(w.open OR w.stopped=2)";
    let decisive = if let Some((min, max, binding)) = bounds {
        max.map(|m| format!("{low}>{m}")).unwrap_or_else(|| {
            if binding.is_none() {
                format!("w.own=1 AND {low}>={min}")
            } else {
                "0".into()
            }
        })
    } else if strict {
        "w.own=1 OR w.own=2 OR w.open".into()
    } else {
        "w.own=1".into()
    };
    let decisive = format!("w.own<0 OR w.stopped<0 OR ({decisive})");
    let body_phase = usize::from(!child);
    let candidate_filter = super::candidate_filter(rule, "n");
    let body_guard = if bounds.is_some() {
        "w.stopped>=0 AND w.work>0".into()
    } else if rule.walk_filter.is_some() && !seek {
        format!("w.stopped>=0 AND ({candidate_filter})")
    } else {
        "w.stopped>=0".into()
    };
    let (body, body_profile) = program(
        rule,
        input,
        false,
        &body_guard,
        &exports,
        (
            "w.file",
            "n.id",
            if bounds.is_some() {
                "w.work-1"
            } else {
                "w.work"
            },
        ),
        sql.generation.clone(),
    )?;
    let (stop_sql, stop_profile) = if let Some(stop) = stop {
        program(
            stop,
            input,
            true,
            &if child {
                format!("NOT ({decisive})")
            } else {
                "1".into()
            },
            &Slots::new(),
            ("w.file", "n.id", "w.work"),
            sql.generation.clone(),
        )?
    } else {
        ("SELECT 0".into(), Value::Null)
    };
    let write_values = exports
        .iter()
        .enumerate()
        .map(|(index, _)| format!(",r.c{}", index + 2))
        .collect::<String>();
    let keep_values = exports
        .iter()
        .map(|slot| format!(",w.v{slot}"))
        .collect::<String>();
    let null_values = exports.iter().map(|_| ",NULL").collect::<String>();
    let seed_values = exports
        .iter()
        .map(|slot| format!(",NULL AS v{slot}"))
        .collect::<String>();
    // The seed both projects and checks its first candidate. A text-filtered seek
    // can scan many postings: materialize that result once, without caller bindings.
    let seed = if seek
        && rule
            .walk_filter
            .as_ref()
            .is_some_and(|f| !f.text.is_empty())
    {
        Some(sql.relation(
            "first text-bounded candidate",
            format!("SELECT d.k,d.file,{first} AS node,origin.\"last\" AS bound,d.work FROM {} d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node",input.name),
            true,
            "k,file,node,bound,work",
        ))
    } else {
        None
    };
    let walk = format!("outcome{}", sql.definitions.len());
    let mut arms = vec![if let Some(seed) = &seed {
        format!(
            "SELECT k,file,node,bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,work{seed_values} FROM {seed} WHERE node IS NOT NULL"
        )
    } else {
        format!(
            "SELECT d.k,d.file,{first} AS node,origin.\"last\" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work{seed_values} FROM {} d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ({first}) IS NOT NULL",
            input.name
        )
    }];
    for phase in 0..2 {
        let body_test = phase == body_phase;
        let projected = if body_test {
            &write_values
        } else {
            &keep_values
        };
        let join = if body_test {
            format!(
                "CROSS JOIN {} d ON d.k=w.k {}",
                input.name,
                bridge(&body, 2 + exports.len())
            )
        } else if stop.is_some() {
            bridge(&stop_sql, 2)
        } else {
            String::new()
        };
        let own = if body_test && bounds.is_some() {
            "CASE WHEN w.stopped<0 THEN 0 WHEN w.work=0 THEN -2 ELSE COALESCE(r.c0,0) END"
        } else if body_test {
            "COALESCE(r.c0,0)"
        } else {
            "w.own"
        };
        let stopped = if body_test {
            "w.stopped"
        } else if stop.is_some() {
            "COALESCE(r.c0,0)"
        } else {
            "0"
        };
        let work = if body_test || stop.is_some() {
            "COALESCE(r.c1,w.work)"
        } else {
            "w.work"
        };
        arms.push(format!("SELECT w.k,w.file,w.node,w.bound,{}, {own},{stopped},w.low,w.uncertain,w.open,{work}{projected} FROM {walk} w CROSS JOIN node n ON n.file=w.file AND n.id=w.node {join} WHERE w.phase={phase}",phase+1));
    }
    arms.push(format!("SELECT w.k,w.file,{next},w.bound,0,0,0,{low},{uncertain},{open},w.work{null_values} FROM {walk} w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT ({decisive}) AND ({next}) IS NOT NULL"));
    let schema = format!(
        "k,file,node,bound,phase,own,stopped,low,uncertain,open,work{}",
        exports
            .iter()
            .map(|slot| format!(",v{slot}"))
            .collect::<String>()
    );
    sql.relation(
        "nested ordered traversal",
        arms.join("\nUNION ALL\n"),
        true,
        &schema,
    );
    let profile = sql.profile.last_mut().unwrap();
    profile["childPrograms"] = json!({"body":body_profile,"stop":stop_profile});
    profile["recursiveColumns"] = json!(11 + exports.len());
    profile["typedReturnColumns"] = json!(2 + exports.len());
    if let Some(filter) = &rule.walk_filter {
        profile["candidatePruning"] = json!({"kinds":filter.kinds,"requiredTextTerms":filter.text,"shape":if seek {"ordered bounded kind seek"} else {"guard before utility charge"}});
        profile["candidatePruning"]["materializedFirstCandidate"] = json!(seed.is_some());
    }
    if seek && rule.walk_filter.is_none() {
        profile["candidatePruning"] = json!({"shape":"ordered bounded kind seek","proof":"pure prefix rejects before effects","filter":filter,"skipsChargedCountVisits":false});
    }
    let terminal=sql.relation("completed nested traversal",format!("SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,{low} AS low,{uncertain} AS uncertain,{open} AS open{keep_values} FROM {walk} w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND (({decisive}) OR ({next}) IS NULL)"),true,"k,failure,low,uncertain,open,observable exports");
    let low = "COALESCE(t.low,0)";
    let high = "(COALESCE(t.low,0)+COALESCE(t.uncertain,0))";
    let closed = "NOT COALESCE(t.open,0)";
    let (verdict, columns) = if let Some((min, max, binding)) = bounds {
        let too_many = max.map_or("0".into(), |m| format!("{low}>{m}"));
        let within = max.map_or("1".into(), |m| format!("{closed} AND {high}<={m}"));
        let exact = if binding.is_some() {
            format!("{closed} AND {low}={high}")
        } else {
            "1".into()
        };
        (
            format!(
                "CASE WHEN ({closed} AND {high}<{min}) OR {too_many} THEN 0 WHEN {low}>={min} AND ({within}) AND ({exact}) THEN 1 ELSE 2 END"
            ),
            binding.map_or(String::new(), |(slot, offset)| {
                format!(",{low}+({offset}) AS c{slot}")
            }),
        )
    } else {
        (
            format!(
                "CASE WHEN {low}>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END"
            ),
            fields(&exports, |slot| format!("t.v{slot}")),
        )
    };
    Ok(Output{name:sql.relation("nested traversal outcome",format!("SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE {verdict} END AS truth{columns} FROM {} d LEFT JOIN {terminal} t ON t.k=d.k",input.name),true,"k,truth,observable capture or count exports")})
}

impl Sql {
    pub(in crate::code_rule_dsl::sql::relational::outcomes) fn correlated_step(
        &mut self,
        state: &Demand,
        child: &Plan,
        guard: &str,
    ) -> Result<String, String> {
        let (query, profile) = program(
            child,
            state,
            false,
            guard,
            &child.writes,
            ("d.file", "d.node", "d.work"),
            self.generation.clone(),
        )?;
        let columns = child
            .writes
            .iter()
            .enumerate()
            .map(|(index, slot)| format!(",r.c{} AS e{slot}", index + 2))
            .collect::<String>();
        let name = self.relation(
            "correlated operand",
            format!(
                "SELECT d.*,r.c0 AS tested,r.c1 AS remaining{columns} FROM {} d {}",
                state.name,
                bridge(&query, child.writes.len() + 2)
            ),
            true,
            "input state,tested outcome,child exports",
        );
        self.profile.last_mut().unwrap()["childPrograms"] = json!({"operand":profile});
        Ok(name)
    }
}
