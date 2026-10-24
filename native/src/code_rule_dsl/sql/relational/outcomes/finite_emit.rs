//! Lazy SQL function calls with shared bodies and persistent capture snapshots.
use super::super::{
    Slots,
    finite_lower::{Instruction, Program},
};
use super::ordered::nested::bridge;
use super::{Demand, Output, Sql, fields, needed, value};
use serde_json::json;
#[path = "finite_binding.rs"]
mod binding;
#[path = "finite_search.rs"]
mod search;
#[path = "finite_shared_search.rs"]
mod shared_search;

fn cases(values: &[String], selector: &str, default: &str) -> String {
    if values.is_empty() {
        default.into()
    } else {
        format!("CASE {selector} {} ELSE {default} END", values.join(" "))
    }
}
fn push(alias: &str, guard: &str, parent: &str, values: &[&str]) -> String {
    let cells = values
        .iter()
        .enumerate()
        .map(|(i, v)| format!(" AND {alias}.v{i} IS ({v})"))
        .collect::<String>();
    format!(
        "LEFT JOIN relation_continuation {alias} ON {alias}.mode IS CASE WHEN {guard} THEN 1 END AND {alias}.parent_input IS {parent}{cells}"
    )
}
fn mask(slots: &Slots, high: bool) -> i64 {
    slots
        .iter()
        .filter(|s| (**s >= 64) == high)
        .fold(0_u64, |m, s| m | (1_u64 << (s % 64))) as i64
}
const STATE: [&str; 8] = [
    "node", "pc", "level", "work", "handle", "truth", "env", "scope",
];
fn seed(alias: &str, guard: &str, popped: bool) -> String {
    let mut cols = STATE
        .iter()
        .map(|c| format!("{alias}.{c} AS {c}"))
        .collect::<Vec<_>>();
    if popped {
        cols.push(format!("{alias}.parent AS parent"));
        cols.extend((0..12).map(|i| format!("{alias}.c{i} AS c{i}")));
    }
    format!("SELECT {} WHERE {guard}", cols.join(","))
}
impl Sql {
    pub(super) fn finite_program(&mut self, p: &Program, input: &Demand) -> Result<Output, String> {
        let mut generation = self.generation.clone();
        generation.peak_tracking = true;
        generation.return_cells = generation
            .return_cells
            .checked_add(8 + 2 + p.writes.len())
            .filter(|n| *n <= 128)
            .ok_or("finite control: active return payload exceeds128 cells")?;
        generation
            .return_peak
            .set(generation.return_peak.get().max(generation.return_cells));
        generation.dynamic_depth = Some("machine.level".into());
        generation.utility_depth = 0;
        let mut slots = input.slots.clone();
        slots.extend(&p.mentions);
        slots.extend(&p.writes);
        for n in &p.code {
            slots.extend(&n.mentions);
            slots.extend(&n.writes);
            if let Instruction::Binding { children, .. } = &n.op {
                generation
                    .following
                    .extend(children.iter().map(|(s, _)| *s));
            }
        }
        slots.extend(&generation.following);
        let mut code = vec![];
        let mut edges = vec![];
        for (i, n) in p.code.iter().enumerate() {
            let pc = i + 10;
            let (op, a) = match &n.op {
                Instruction::Eval(id) => (1, *id),
                Instruction::Call(id) => (2, *id),
                Instruction::Boolean { all, children } => {
                    for (ord, child) in children.iter().enumerate() {
                        edges.push(format!("({pc},{ord},{})", child + 10));
                    }
                    (if *all { 3 } else { 4 }, 0)
                }
                Instruction::Not(child) => (5, child + 10),
                Instruction::Search { .. } => (6, 0),
                Instruction::Binding { .. } => (7, 0),
            };
            if !(matches!(n.op, Instruction::Eval(_))
                || p.share_search && matches!(n.op, Instruction::Search { .. }))
            {
                self.generation.operator_as("shared finite control")?;
            }
            code.push(format!(
                "({pc},{op},{a},{},{},{},{},{})",
                mask(&n.mentions, false),
                mask(&n.mentions, true),
                mask(&n.node_writes, false),
                mask(&n.node_writes, true),
                usize::from(!n.writes.is_empty())
            ));
        }
        let code=self.constant_relation("shared finite code",format!("SELECT column1 pc,column2 op,column3 a,column4 ml,column5 mh,column6 wl,column7 wh,column8 strict FROM (VALUES {})",code.join(",")));
        let edges = self.constant_relation(
            "finite child order",
            if edges.is_empty() {
                "SELECT NULL pc,NULL ordinal,NULL child WHERE 0".into()
            } else {
                format!(
                    "SELECT column1 pc,column2 ordinal,column3 child FROM (VALUES {})",
                    edges.join(",")
                )
            },
        );
        let layout = self.generation.memo_layout.clone();
        let functions = p
            .functions
            .iter()
            .enumerate()
            .map(|(id, f)| {
                let identity = f.memo.filter(|_| !generation.memo_bypass).map(|m| {
                    layout
                        .as_ref()
                        .and_then(|l| l.contracts.get(&m))
                        .map_or(m, |c| c.identity)
                });
                format!(
                    "({id},{},{},{})",
                    f.entry + 10,
                    identity.map_or("NULL".into(), |m| m.to_string()),
                    usize::from(f.effects.cache_fills)
                )
            })
            .collect::<Vec<_>>();
        let functions = self.constant_relation(
            "shared finite functions",
            if functions.is_empty() {
                "SELECT NULL id,NULL entry,NULL memo,NULL fills WHERE 0".into()
            } else {
                format!(
                    "SELECT column1 id,column2 entry,column3 memo,column4 fills FROM (VALUES {})",
                    functions.join(",")
                )
            },
        );
        let key_sql = |env: &str| -> String {
            slots
                .iter()
                .filter_map(|slot| {
                    let owners = p
                        .functions
                        .iter()
                        .enumerate()
                        .filter(|(_, f)| {
                            let contract = f
                                .memo
                                .and_then(|id| layout.as_ref().and_then(|l| l.contracts.get(&id)));
                            generation.following.contains(slot)
                                || ((!matches!(&p.code[f.entry].op, Instruction::Eval(region) if matches!(p.regions[*region].op, super::super::Op::CallFrame { .. })) || f.mentions.contains(slot)) && layout.as_ref().is_none_or(|l| {
                                    l.capture(*slot, contract.and_then(|c| c.scope.as_ref()))
                                }))
                        })
                        .map(|(i, _)| i.to_string())
                        .collect::<Vec<_>>();
                    if owners.is_empty() {
                        return None;
                    }
                    let raw = format!("relation_state_get({env},{slot})");
                    let key = layout
                        .as_ref()
                        .map_or_else(|| raw.clone(), |l| l.normalize_key(*slot, raw.clone()));
                    let col = layout.as_ref().map_or(*slot, |l| l.column(*slot));
                    Some(format!(
                        " AND m.k{col} IS CASE WHEN f.id IN ({}) THEN {key} END",
                        owners.join(",")
                    ))
                })
                .collect()
        };
        let clause = generation
            .memo_clause
            .ok_or("finite functions need clause memo lifetime")?;
        let input_key = key_sql("s.env");
        let return_key = key_sql("p.c7");
        let hit_envs=p.functions.iter().enumerate().map(|(id,f)|{
            let env=f.writes.iter().fold("s.env".to_string(),|env,slot|{
                let col=layout.as_ref().map_or(*slot,|l|l.column(*slot));let stored=format!("m.o{col}");
                let v=if layout.as_ref().is_some_and(|l|l.span_capable(*slot)){format!("relation_capture_replay(d.file,relation_state_get(s.env,{slot}),{stored})")}else{stored};
                format!("relation_state_put(s.scope,{env},{slot},{v})")
            });format!("WHEN {id} THEN {env}")
        }).collect::<Vec<_>>();
        let hit_env = cases(&hit_envs, "f.id", "s.env");
        let outputs: Slots = p
            .functions
            .iter()
            .flat_map(|f| f.writes.iter().copied())
            .collect();
        let supplied=outputs.iter().map(|slot|{
            let owners=p.functions.iter().enumerate().filter(|(_,f)|f.writes.contains(slot)).map(|(i,_)|i.to_string()).collect::<Vec<_>>().join(",");
            let col=layout.as_ref().map_or(*slot,|l|l.column(*slot));format!(" AND m.v{col} IS CASE WHEN p.truth=1 AND f.id IN ({owners}) THEN relation_state_get(p.env,{slot}) END")
        }).collect::<String>();
        let enter = format!(
            r#"WITH reached AS MATERIALIZED ({}), s AS MATERIALIZED (SELECT s.*,f.id function,f.entry,f.memo,f.fills FROM reached s JOIN {code} i ON i.pc=s.pc JOIN {functions} f ON f.id=i.a),
lookup AS MATERIALIZED (SELECT s.*,m.truth hit,m.cost,m.peak,CASE WHEN m.truth=1 AND s.work>=m.cost THEN {hit_env} ELSE s.env END hit_env FROM s JOIN {functions} f ON f.id=s.function LEFT JOIN relation_memo m ON m.mode IS CASE WHEN f.memo IS NOT NULL THEN 0 END AND m.clause IS {clause} AND m.utility IS f.memo AND m.file IS d.file AND m.node IS s.node{input_key} AND m.peak<=256-s.level),
opened AS MATERIALIZED (SELECT s.*,CASE WHEN hit IS NULL AND level<256 THEN relation_peak_open(level) END token FROM lookup s),
noted AS MATERIALIZED (SELECT s.*,CASE WHEN hit IS NOT NULL THEN relation_peak_note(level+peak) WHEN token IS NOT NULL THEN relation_peak_note(level+1) ELSE 1 END ready FROM opened s)
SELECT s.node,CASE WHEN s.hit IS NOT NULL OR s.level>=256 OR s.work=0 THEN 0 ELSE s.entry END,CASE WHEN s.hit IS NULL AND s.level<256 THEN s.level+1 ELSE s.level END,
CASE WHEN s.hit IS NOT NULL THEN MAX(0,s.work-s.cost) WHEN s.level<256 AND s.work>0 THEN s.work-1 ELSE s.work END,COALESCE(frame.handle,s.handle),CASE WHEN s.hit IS NOT NULL THEN CASE WHEN s.work<s.cost THEN -2 ELSE s.hit END WHEN s.level>=256 THEN -1 WHEN s.work=0 THEN -2 ELSE 0 END,CASE WHEN s.hit IS NOT NULL AND s.work>=s.cost THEN s.hit_env ELSE s.env END,s.scope FROM noted s {} WHERE s.ready=1"#,
            seed(
                "machine",
                &format!("machine.pc IN (SELECT pc FROM {code} WHERE op=2)"),
                false
            ),
            push(
                "frame",
                "s.hit IS NULL AND s.level<256",
                "s.handle",
                &[
                    "0",
                    "s.node",
                    "s.level",
                    "s.function",
                    "s.work",
                    "CASE WHEN s.fills THEN relation_cache_fill_steps() END",
                    "s.token",
                    "s.env"
                ]
            )
        );
        let bool_start = format!(
            "WITH reached AS MATERIALIZED ({}),s AS MATERIALIZED (SELECT s.*,i.op,(SELECT child FROM {edges} WHERE pc=s.pc AND ordinal=0) child FROM reached s JOIN {code} i ON i.pc=s.pc) SELECT s.node,COALESCE(s.child,0),s.level,s.work,COALESCE(frame.handle,s.handle),CASE WHEN s.child IS NULL THEN s.op=3 ELSE 0 END,s.env,s.scope FROM s {}",
            seed(
                "machine",
                &format!("machine.pc IN (SELECT pc FROM {code} WHERE op IN (3,4))"),
                false
            ),
            push(
                "frame",
                "s.child IS NOT NULL",
                "s.handle",
                &[
                    "CASE s.op WHEN 3 THEN 2 ELSE 3 END",
                    "s.node",
                    "s.level",
                    "s.pc",
                    "0",
                    "0",
                    "s.env",
                    "s.env",
                    "0",
                    "0"
                ]
            )
        );
        let not_start = format!(
            "WITH s AS MATERIALIZED ({}) SELECT s.node,i.a,s.level,s.work,frame.handle,0,s.env,s.scope FROM s JOIN {code} i ON i.pc=s.pc {}",
            seed(
                "machine",
                &format!("machine.pc IN (SELECT pc FROM {code} WHERE op=5)"),
                false
            ),
            push(
                "frame",
                "1",
                "s.handle",
                &["4", "s.node", "s.level", "s.pc", "0", "0", "s.env"]
            )
        );
        let mut phases = vec![
            (
                enter,
                format!("machine.pc IN (SELECT pc FROM {code} WHERE op=2)"),
            ),
            (
                bool_start,
                format!("machine.pc IN (SELECT pc FROM {code} WHERE op IN (3,4))"),
            ),
            (
                not_start,
                format!("machine.pc IN (SELECT pc FROM {code} WHERE op=5)"),
            ),
        ];
        let finish_call = format!(
            r#"WITH reached AS MATERIALIZED ({}),closed AS MATERIALIZED (SELECT p.*,relation_peak_close(p.c6) measured FROM reached p)
SELECT p.c1,0,p.c2,p.work,p.parent,COALESCE(m.truth,p.truth),CASE WHEN p.truth=1 THEN p.env ELSE p.c7 END,p.scope FROM closed p JOIN {functions} f ON f.id=p.c3 LEFT JOIN relation_memo m ON m.mode IS CASE WHEN f.memo IS NOT NULL AND p.truth>=0 THEN 1 END AND m.clause IS {clause} AND m.utility IS f.memo AND m.file IS d.file AND m.node IS p.c1{return_key} AND m.vtruth IS p.truth AND m.vcost IS CASE WHEN p.c5 IS NULL THEN p.c4-p.work ELSE MAX(0,p.c4-p.work-relation_cache_fill_delta(p.c5)) END AND m.vpeak IS p.measured{supplied} WHERE p.measured>=0"#,
            seed("p", "p.c0=0", true)
        );
        let finish_not = "SELECT p.c1,0,p.c2,p.work,p.parent,CASE p.truth WHEN 0 THEN 1 WHEN 1 THEN 0 ELSE p.truth END,p.c6,p.scope WHERE p.c0=4".to_string();
        let again = push(
            "again",
            "p.more",
            "p.parent",
            &[
                "p.c0",
                "p.c1",
                "p.c2",
                "p.c3",
                "p.c4+1",
                "p.uncertain",
                "p.c6",
                "p.current_env",
                "p.held_lo",
                "p.held_hi",
            ],
        );
        let finish_bool = format!(
            r#"WITH reached AS MATERIALIZED ({}),edges AS MATERIALIZED (SELECT p.*,child.strict,child.wl,child.wh,(SELECT child FROM {edges} WHERE pc=p.c3 AND ordinal=p.c4+1) next FROM reached p JOIN {edges} edge ON edge.pc=p.c3 AND edge.ordinal=p.c4 JOIN {code} child ON child.pc=edge.child),
changed AS MATERIALIZED (SELECT p.*,p.c5 OR p.truth=2 uncertain,CASE WHEN p.c0=2 AND p.truth=1 THEN p.env ELSE p.c7 END current_env,p.c8|CASE WHEN p.truth=2 THEN p.wl ELSE 0 END held_lo,p.c9|CASE WHEN p.truth=2 THEN p.wh ELSE 0 END held_hi FROM edges p),
s AS MATERIALIZED (SELECT p.*,p.next IS NOT NULL AND p.truth>=0 AND CASE p.c0 WHEN 2 THEN p.truth<>0 ELSE p.truth<>1 AND NOT(p.truth=2 AND p.strict) END more,COALESCE((p.held_lo & child.ml)<>0 OR (p.held_hi & child.mh)<>0,0) blocked FROM changed p LEFT JOIN {code} child ON child.pc=p.next)
SELECT p.c1,CASE WHEN again.handle IS NOT NULL THEN CASE WHEN p.c0=2 AND p.blocked THEN 0 ELSE p.next END ELSE 0 END,p.c2,p.work,COALESCE(again.handle,p.parent),
CASE WHEN again.handle IS NOT NULL THEN CASE WHEN p.c0=2 AND p.blocked THEN 2 ELSE 0 END WHEN p.truth<0 THEN p.truth WHEN p.c0=2 AND p.truth=0 THEN 0 WHEN p.c0=3 AND p.truth=1 THEN 1 WHEN p.uncertain THEN 2 WHEN p.c0=2 THEN 1 ELSE 0 END,
CASE WHEN again.handle IS NOT NULL THEN CASE WHEN p.c0=2 THEN p.current_env ELSE p.c6 END WHEN p.c0=3 AND p.truth=1 THEN p.env WHEN p.c0=2 AND p.truth=1 AND NOT p.uncertain THEN p.current_env ELSE p.c6 END,p.scope FROM s p {again}"#,
            seed("p", "p.c0 IN (2,3)", true)
        );
        let mut finishes = vec![
            "SELECT p.node,-1,p.level,p.work,NULL,p.truth,p.env,p.scope WHERE p.handle IS NULL"
                .into(),
            finish_call,
            finish_not,
            finish_bool,
        ];
        let mut profiles = vec![];
        let mut region_operators = vec![];
        let search_count = p
            .code
            .iter()
            .filter(|node| matches!(node.op, Instruction::Search { .. }))
            .count();
        if p.share_search && search_count > 0 {
            // One actual SQL enter/return template replaces one pair per search.
            // Every instruction still counts against the separate 256-node IR cap.
            self.generation.operator_as("shared search transition")?;
            let table = self.constant_relation(
                "finite search instructions",
                shared_search::metadata(p)
                    .ok_or("finite control: missing shared search metadata")?,
            );
            let guard = format!("machine.pc IN (SELECT pc FROM {table})");
            phases.push((
                shared_search::enter_all(p, &table, &seed("machine", &guard, false))?,
                guard,
            ));
            finishes.push(shared_search::finish_all(
                p,
                &table,
                &seed("p", "p.c0 IN (1,7)", true),
            )?);
            profiles.push(json!({"physicalOperator":"shared search transition","instructionRows":search_count,"enterTemplates":1,"returnTemplates":1}));
        }
        for (id, node) in p.code.iter().enumerate() {
            let pc = id + 10;
            match &node.op {
                Instruction::Search { .. } => {
                    if p.share_search {
                        continue;
                    }
                    phases.push((
                        search::enter(
                            p,
                            pc,
                            node,
                            &seed("machine", &format!("machine.pc={pc}"), false),
                        )?,
                        format!("machine.pc={pc}"),
                    ));
                    finishes.push(search::finish(
                        p,
                        pc,
                        node,
                        &seed("p", &format!("p.c0 IN (1,7) AND p.c3={pc}"), true),
                    )?);
                }
                Instruction::Binding { .. } => {
                    phases.push((
                        binding::enter(
                            p,
                            pc,
                            node,
                            &seed("machine", &format!("machine.pc={pc}"), false),
                        )?,
                        format!("machine.pc={pc}"),
                    ));
                    finishes.push(binding::finish(
                        p,
                        pc,
                        node,
                        &seed("p", &format!("p.c0=6 AND p.c3={pc}"), true),
                    )?);
                }
                Instruction::Eval(region) => {
                    let plan = &p.regions[*region];
                    let mut child_generation = generation.clone();
                    child_generation.return_cells = child_generation
                        .return_cells
                        .checked_add(2 + plan.writes.len())
                        .filter(|n| *n <= 128)
                        .ok_or("finite region return payload exceeds128cells")?;
                    child_generation.return_peak.set(
                        child_generation
                            .return_peak
                            .get()
                            .max(child_generation.return_cells),
                    );
                    let mut child = Sql {
                        definitions: vec![],
                        profile: vec![],
                        correlated: true,
                        generation: child_generation,
                    };
                    let mut region_slots = needed(plan);
                    if plan.uses_memo() || generation.peak_tracking {
                        region_slots.extend(&slots);
                    }
                    let cols = fields(&region_slots, |slot| {
                        format!("relation_state_get(machine.env,{slot})")
                    });
                    let demand=Demand{name:child.relation("shared region input",format!("SELECT 1 k,d.file,machine.node,machine.work{cols} WHERE machine.pc={pc}"),true,"reached typed snapshot"),slots:region_slots};
                    let before = child.generation.operators.get();
                    let out = child.emit(plan, &demand).map_err(|error| {
                        format!("{error}; finite region {region} at pc {pc}, prior region operator counts {region_operators:?}")
                    })?;
                    let emitted = child.generation.operators.get() - before;
                    region_operators.push((*region, emitted));
                    let cols = fields(&plan.writes, |slot| format!("o.c{slot}"));
                    let query = format!(
                        "WITH RECURSIVE {} SELECT o.truth,o.work{cols} FROM {} o",
                        child.definitions.join(","),
                        out.name
                    );
                    let env = plan.writes.iter().enumerate().fold(
                        "s.env".to_string(),
                        |env, (i, slot)| {
                            format!("relation_state_put(s.scope,{env},{slot},r.c{})", i + 2)
                        },
                    );
                    phases.push((format!("WITH s AS MATERIALIZED ({}) SELECT s.node,0,s.level,r.c1,s.handle,r.c0,CASE WHEN r.c0=1 THEN {env} ELSE s.env END,s.scope FROM s {}",seed("machine",&format!("machine.pc={pc}"),false),bridge(&query,2+plan.writes.len())),format!("machine.pc={pc}")));
                    profiles.push(json!({"pc":pc,"region":region,"compositionOperators":emitted,"sqlBytes":query.len(),"typedReturnColumns":2+plan.writes.len(),"relations":child.profile}));
                }
                _ => {}
            }
        }
        let handler = finishes
            .into_iter()
            .map(|q| format!("SELECT * FROM ({q})"))
            .collect::<Vec<_>>()
            .join(" UNION ALL ");
        let framecols = (0..12).map(|i| format!(",frame.c{i}")).collect::<String>();
        let dispatch = format!(
            "WITH s AS MATERIALIZED ({}),popped AS MATERIALIZED (SELECT s.*,frame.parent{framecols} FROM s LEFT JOIN relation_continuation frame ON frame.mode IS CASE WHEN s.handle IS NOT NULL THEN 2 END AND frame.h IS s.handle) SELECT r.c0,r.c1,r.c2,r.c3,r.c4,r.c5,r.c6,r.c7 FROM popped p {}",
            seed("machine", "machine.pc=0", false),
            bridge(&handler, 8)
        );
        phases.push((dispatch, "machine.pc=0".into()));
        let depth = self.generation.depth_expression();
        let initial = input.slots.iter().fold("0".to_string(), |env, slot| {
            format!(
                "relation_state_put(owner.scope,{env},{slot},{})",
                value(input, "d", *slot)
            )
        });
        let mut arms = vec![format!(
            "SELECT d.node node,{} pc,({depth}) level,d.work work,NULL handle,0 truth,{initial} env,owner.scope scope FROM owner",
            p.entry + 10
        )];
        for (query, guard) in phases {
            arms.push(format!("SELECT r.c0,r.c1,r.c2,r.c3,r.c4,r.c5,r.c6,r.c7 FROM schedule machine {} WHERE {guard}",bridge(&query,8)));
        }
        let cols = p
            .writes
            .iter()
            .map(|slot| format!(",relation_state_get(env,{slot}) c{slot}"))
            .collect::<String>();
        let returned = p
            .writes
            .iter()
            .map(|slot| format!(",c{slot}"))
            .collect::<String>();
        let query = format!(
            "WITH RECURSIVE owner AS MATERIALIZED (SELECT relation_state_begin() scope WHERE d.file IS NOT NULL),schedule AS ({}),returned AS MATERIALIZED (SELECT scope,truth,work{cols} FROM schedule WHERE pc=-1),closed AS MATERIALIZED (SELECT returned.*,relation_state_end(scope) ready FROM returned) SELECT truth,work{returned} FROM closed WHERE ready=1",
            arms.join(" UNION ALL ")
        );
        let exports = p
            .writes
            .iter()
            .enumerate()
            .map(|(i, slot)| format!(",r.c{} c{slot}", i + 2))
            .collect::<String>();
        let name = self.relation(
            "shared finite execution",
            format!(
                "SELECT d.k,r.c1 work,r.c0 truth{exports} FROM {} d {}",
                input.name,
                bridge(&query, 2 + p.writes.len())
            ),
            true,
            "shared lazy SQL bodies; persistent capture snapshots",
        );
        self.profile.last_mut().unwrap()["sharedFinite"] = json!({"functions":p.functions.len(),"instructions":p.code.len(),"regions":profiles,"recursiveColumns":8,"captureVectorCopies":false});
        Ok(Output { name })
    }
}
