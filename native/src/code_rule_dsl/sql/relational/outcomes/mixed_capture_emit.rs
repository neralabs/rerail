//! Three-valued recursive SQL control; captures remain immutable in the outer demand.
use super::super::lower::mixed_recursive_lower::{Direction, Instruction, Program, Walk};
use super::{Demand, Output, Sql, ordered::nested::bridge, value};
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
    pub(super) fn mixed_capture_recursive(
        &mut self,
        p: &Program,
        entry: usize,
        input: &Demand,
    ) -> Result<Output, String> {
        let result_writes = &p.functions[entry].writes;
        let scoped_input = self.relation(
            "capture recursive invocation scopes",
            format!(
                "SELECT d.*,relation_state_begin() AS state_scope FROM {} d",
                input.name
            ),
            true,
            "isolated immutable snapshot lifetime",
        );
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
        let code = self.constant_relation(
            "mixed recursive control",
            format!(
                "SELECT column1 AS pc,column2 AS op,column3 AS a FROM (VALUES {})",
                code.join(",")
            ),
        );
        let children = self.constant_relation(
            "mixed Boolean children",
            if booleans.is_empty() {
                "SELECT NULL AS pc,NULL AS ordinal,NULL AS child WHERE 0".into()
            } else {
                format!(
                    "SELECT column1 AS pc,column2 AS ordinal,column3 AS child FROM (VALUES {})",
                    booleans.join(",")
                )
            },
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
        let functions = self.constant_relation(
            "mixed utility entries",
            format!(
                "SELECT column1 AS id,column2 AS entry,column3 AS memo FROM (VALUES {})",
                functions.join(",")
            ),
        );
        let mut slots = input.slots.clone();
        slots.extend(&p.mentions);
        slots.extend(&p.writes);
        if let Some(l) = &layout {
            slots.retain(|s| l.capture(*s, None));
        }
        slots.extend(&self.generation.following);
        let key_sql = slots
            .iter()
            .map(|s| {
                let raw = format!("relation_state_get(s.env,{s})");
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
        let return_key_sql = key_sql.replace("s.env", "p.c7");
        let exports_slot = |f: &super::super::lower::mixed_recursive_lower::Function,
                            slot: usize| {
            layout.as_ref().is_none_or(|l| {
                l.capture(
                    slot,
                    l.contracts.get(&f.memo).and_then(|c| c.scope.as_ref()),
                )
            })
        };
        let exported = p
            .functions
            .iter()
            .flat_map(|f| {
                f.writes
                    .iter()
                    .copied()
                    .filter(|slot| exports_slot(f, *slot))
            })
            .collect::<super::Slots>();
        let output_columns = exported
            .iter()
            .map(|slot| format!(",m.o{} AS o{slot}", column(*slot)))
            .collect::<String>();
        let mut replay_ctes = String::new();
        let mut replay_source = "lookup".to_owned();
        let mut recalled = "s.env".to_owned();
        for (index, slot) in exported.iter().enumerate() {
            let current = format!("relation_state_get({recalled},{slot})");
            let stored = format!("s.o{slot}");
            let cell = if layout.as_ref().is_some_and(|l| l.span_capable(*slot)) {
                format!("relation_capture_replay(d.file,{current},{stored})")
            } else {
                stored.clone()
            };
            let name = format!("replay{index}");
            replay_ctes.push_str(&format!("{name} AS MATERIALIZED (SELECT s.*,CASE WHEN s.hit IS NOT NULL AND {stored} IS NOT NULL THEN relation_state_put(d.state_scope,{recalled},{slot},{cell}) ELSE {recalled} END AS replay_env{index} FROM {replay_source} s),\n"));
            replay_source = name;
            recalled = format!("s.replay_env{index}");
        }
        let mut export_sql = String::new();
        for slot in &exported {
            let variants = p
                .functions
                .iter()
                .enumerate()
                .filter(|(_, f)| f.writes.contains(slot) && exports_slot(f, *slot))
                .map(|(id, _)| format!("WHEN {id} THEN relation_state_get(p.env,{slot})"))
                .collect::<Vec<_>>();
            export_sql.push_str(&format!(
                " AND m.v{} IS CASE WHEN p.truth=1 THEN {} END",
                column(*slot),
                cases(&variants, "f.id", "NULL")
            ));
        }
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
                "SELECT {alias}.node AS node,{alias}.pc AS pc,{alias}.level AS level,{alias}.work AS work,{alias}.handle AS handle,{alias}.truth AS truth,{alias}.env AS env WHERE {predicate}"
            )
        };
        let enter = format!(
            r#"WITH reached AS MATERIALIZED ({}),
s AS MATERIALIZED (SELECT reached.*,f.id AS function,f.entry FROM reached JOIN {code} i ON i.pc=reached.pc JOIN {functions} f ON f.id=i.a),
lookup AS MATERIALIZED (SELECT s.*,m.truth AS hit,m.cost,m.peak{output_columns} FROM s JOIN {functions} f ON f.id=s.function LEFT JOIN relation_memo m ON m.mode IS CASE WHEN {caching} THEN 0 END AND {entry_identity}{key_sql} AND m.peak<=256-s.level),
{replay_ctes}opened AS MATERIALIZED (SELECT s.*,{recalled} AS recalled_env,CASE WHEN s.hit IS NULL AND s.level<256 THEN relation_peak_open(s.level) END AS token FROM {replay_source} s),
noted AS MATERIALIZED (SELECT s.*,CASE WHEN s.hit IS NOT NULL THEN relation_peak_note(s.level+s.peak) WHEN s.token IS NOT NULL THEN relation_peak_note(s.level+1) ELSE 1 END AS ready FROM opened s)
SELECT s.node,CASE WHEN s.hit IS NOT NULL OR s.level>=256 THEN 0 WHEN s.work=0 THEN 1 ELSE s.entry END,
CASE WHEN s.hit IS NULL AND s.level<256 THEN s.level+1 ELSE s.level END,
CASE WHEN s.hit IS NOT NULL THEN MAX(0,s.work-s.cost) WHEN s.level<256 AND s.work>0 THEN s.work-1 ELSE s.work END,
COALESCE(frame.handle,s.handle),CASE WHEN s.hit IS NOT NULL THEN CASE WHEN s.work<s.cost THEN -2 ELSE s.hit END WHEN s.level>=256 THEN -1 WHEN s.work=0 THEN -2 ELSE 0 END,CASE WHEN s.hit IS NOT NULL THEN s.recalled_env ELSE s.env END
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
                    "s.token",
                    "s.env"
                ]
            )
        );
        let test = format!(
            "WITH s AS MATERIALIZED ({}) SELECT s.node,0,s.level,s.work,s.handle,CASE WHEN ({}) THEN 1 ELSE 0 END,s.env FROM s JOIN node n ON n.file=d.file AND n.id=s.node",
            seed(&format!("{alias}.pc IN (SELECT pc FROM {code} WHERE op=2)")),
            cases(&tests, "s.pc", "0")
        );
        let bool_start = format!(
            "WITH s AS MATERIALIZED ({}), i AS MATERIALIZED (SELECT s.*,i.a,(SELECT child FROM {children} WHERE pc=s.pc AND ordinal=0) AS child FROM s JOIN {code} i ON i.pc=s.pc) SELECT s.node,COALESCE(s.child,0),s.level,s.work,COALESCE(frame.handle,s.handle),CASE WHEN s.child IS NULL THEN s.a ELSE 0 END,s.env FROM i s {}",
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
                    "0",
                    "s.env"
                ]
            )
        );
        let not_start = format!(
            "WITH s AS MATERIALIZED ({}) SELECT s.node,i.a,s.level,s.work,frame.handle,0,s.env FROM s JOIN {code} i ON i.pc=s.pc {}",
            seed(&format!("{alias}.pc IN (SELECT pc FROM {code} WHERE op=6)")),
            push(
                "frame",
                "1",
                "s.handle",
                &["4", "s.node", "s.level", "s.env"]
            )
        );
        let search_start = format!(
            "WITH reached AS MATERIALIZED ({}),s AS MATERIALIZED (SELECT s.*,n.\"last\" AS bound,{} AS child FROM reached s JOIN node n ON n.file=d.file AND n.id=s.node) SELECT COALESCE(s.child,s.node),CASE WHEN s.child IS NULL THEN 0 ELSE -2 END,s.level,s.work,COALESCE(frame.handle,s.handle),0,s.env FROM s {}",
            seed(&format!("{alias}.pc IN (SELECT pc FROM {code} WHERE op=7)")),
            cases(&starts, "s.pc", "NULL"),
            push(
                "frame",
                "s.child IS NOT NULL",
                "s.handle",
                &[
                    "1", "s.node", "s.level", "s.pc", "s.child", "s.bound", "0", "s.env"
                ]
            )
        );
        let admit = format!(
            "WITH s AS MATERIALIZED ({}) SELECT s.node,CASE WHEN ({}) THEN i.a ELSE 0 END,s.level,s.work,s.handle,0,s.env FROM s CROSS JOIN relation_continuation p ON p.mode IS 0 AND p.h IS s.handle JOIN {code} i ON i.pc=p.c3 JOIN node n ON n.file=d.file AND n.id=s.node",
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
                "p.c6",
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
                "p.c7",
            ],
        );
        // Keep one consumer of the pop/close/publication chain. Correlated CTEs
        // referenced by separate UNION branches can instantiate that chain more
        // than once, even with AS MATERIALIZED, consuming the frame twice.
        let dispatch = format!(
            r#"WITH reached AS MATERIALIZED ({}),
popped AS MATERIALIZED (SELECT s.*,frame.parent,frame.c0,frame.c1,frame.c2,frame.c3,frame.c4,frame.c5,frame.c6,frame.c7 FROM reached s LEFT JOIN relation_continuation frame ON frame.mode IS CASE WHEN s.handle IS NOT NULL THEN 2 END AND frame.h IS s.handle),
closed AS MATERIALIZED (SELECT p.*,CASE WHEN p.c0=0 THEN relation_peak_close(p.c6) END AS measured FROM popped p),
published AS MATERIALIZED (SELECT p.*,COALESCE(m.truth,p.truth) AS returned FROM closed p LEFT JOIN {functions} f ON p.c0=0 AND f.id=p.c3 LEFT JOIN relation_memo m ON m.mode IS CASE WHEN {caching} AND p.c0=0 AND p.truth>=0 THEN 1 END AND {return_identity}{return_key_sql}{export_sql} AND m.vtruth IS p.truth AND m.vcost IS CASE WHEN p.c0=0 AND p.truth>=0 THEN MAX(0,p.c4-p.work-relation_cache_fill_delta(p.c5)) END AND m.vpeak IS p.measured WHERE COALESCE(p.measured,0)>=0),
classified AS MATERIALIZED (SELECT p.*,CASE WHEN p.c0 IN (2,3) AND p.truth>=0 AND NOT ((p.c0=2 AND p.truth=0) OR (p.c0=3 AND p.truth=1)) THEN (SELECT child FROM {children} WHERE pc=p.c3 AND ordinal=p.c4+1) END AS child,CASE WHEN p.c0=1 AND p.truth IN (0,2) THEN {} END AS candidate FROM published p LEFT JOIN node n ON p.c0=1 AND n.file=d.file AND n.id=p.c4)
SELECT CASE WHEN p.handle IS NULL THEN p.node WHEN p.c0=1 AND search_again.handle IS NOT NULL THEN p.candidate ELSE p.c1 END,
CASE WHEN p.handle IS NULL THEN -1 WHEN bool_again.handle IS NOT NULL THEN p.child WHEN search_again.handle IS NOT NULL THEN -2 ELSE 0 END,
CASE WHEN p.handle IS NULL THEN p.level ELSE p.c2 END,p.work,
COALESCE(bool_again.handle,search_again.handle,p.parent),
CASE WHEN p.handle IS NULL THEN p.truth WHEN p.c0=0 THEN p.returned WHEN p.c0=4 THEN CASE p.truth WHEN 0 THEN 1 WHEN 1 THEN 0 ELSE p.truth END
WHEN bool_again.handle IS NOT NULL OR search_again.handle IS NOT NULL THEN 0 WHEN p.truth<0 THEN p.truth
WHEN p.c0=2 AND p.truth=0 THEN 0 WHEN p.c0=3 AND p.truth=1 THEN 1
WHEN p.c0 IN (2,3) THEN CASE WHEN p.c5 OR p.truth=2 THEN 2 WHEN p.c0=2 THEN 1 ELSE 0 END
WHEN p.c0=1 THEN CASE WHEN p.truth=1 THEN 1 WHEN p.c6 OR p.truth=2 THEN 2 ELSE 0 END END,
CASE WHEN p.handle IS NULL THEN p.env
WHEN p.c0=0 THEN CASE WHEN p.truth=1 THEN p.env ELSE p.c7 END
WHEN p.c0=4 THEN p.c3
WHEN p.c0 IN (2,3) THEN CASE WHEN bool_again.handle IS NOT NULL AND p.c0=2 THEN p.env WHEN bool_again.handle IS NOT NULL OR p.truth<>1 THEN p.c6 ELSE p.env END
WHEN p.c0=1 THEN CASE WHEN search_again.handle IS NOT NULL OR p.truth<>1 THEN p.c7 ELSE p.env END END
FROM classified p {bool_advance} {search_advance}"#,
            seed(&format!("{alias}.pc=0")),
            cases(&advances, "p.c3", "NULL")
        );
        let mut generation = self.generation.clone();
        generation.return_cells = generation
            .return_cells
            .checked_add(9 + result_writes.len())
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
            let region = &p.regions[id];
            let mut child_generation = generation.clone();
            child_generation.return_cells = child_generation
                .return_cells
                .checked_add(2 + region.writes.len())
                .filter(|cells| *cells <= 128)
                .ok_or("capture recursion: finite return payload exceeds 128 active cells")?;
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
            let mut live = input.slots.clone();
            live.extend(&p.mentions);
            live.extend(&p.writes);
            let capture_columns = super::fields(&live, |slot| {
                format!("relation_state_get({alias}.env,{slot})")
            });
            let demand = Demand {name:child.relation("capture recursive finite input",format!("SELECT 1 AS k,d.file,{alias}.node AS node,{alias}.work AS work{capture_columns} WHERE {alias}.pc={pc}"),true,"typed immutable snapshot reads"),slots:live};
            let output = child.emit(region, &demand)?;
            let columns = region
                .writes
                .iter()
                .map(|slot| format!(",o.c{slot}"))
                .collect::<String>();
            let q = format!(
                "WITH RECURSIVE {} SELECT o.truth,o.work{columns} FROM {} o",
                child.definitions.join(",\n"),
                output.name
            );
            let mut next_env = "s.env".to_owned();
            for (index, slot) in region.writes.iter().enumerate() {
                let update = if index == 0 {
                    "relation_state_put"
                } else {
                    "relation_state_put_owned"
                };
                next_env = format!("{update}(d.state_scope,{next_env},{slot},r.c{})", index + 2);
            }
            let width = 2 + region.writes.len();
            phase_queries.push((format!("WITH s AS MATERIALIZED ({}) SELECT s.node,0,s.level,r.c1,s.handle,r.c0,CASE WHEN r.c0=1 THEN {next_env} ELSE s.env END FROM s {}",seed(&format!("{alias}.pc={pc}")),bridge(&q,width)),format!("{alias}.pc={pc}")));
            region_profiles.push(json!({"relations":child.profile,"typedExports":region.writes}));
        }
        let depth = self.generation.depth_expression();
        let mut initial_env = "0".to_owned();
        for slot in &input.slots {
            initial_env = format!(
                "relation_state_put_owned(d.state_scope,{initial_env},{slot},{})",
                value(input, "d", *slot)
            );
        }
        let mut arms = vec![format!(
            "SELECT d.node AS node,{root_call} AS pc,({depth}) AS level,d.work AS work,NULL AS handle,0 AS truth,{initial_env} AS env"
        )];
        arms.push(format!("SELECT {alias}.node,0,{alias}.level,{alias}.work,{alias}.handle,{alias}.truth,{alias}.env FROM schedule {alias} WHERE {alias}.pc=1"));
        for (q, reached) in phase_queries {
            arms.push(format!(
                "SELECT r.c0,r.c1,r.c2,r.c3,r.c4,r.c5,r.c6 FROM schedule {alias} {} WHERE {reached}",
                bridge(&q, 7)
            ));
        }
        let exports = result_writes
            .iter()
            .map(|slot| format!(",relation_state_get(env,{slot})"))
            .collect::<String>();
        let q = format!(
            "WITH RECURSIVE schedule AS ({}) SELECT truth,work{exports} FROM schedule WHERE pc=-1",
            arms.join("\nUNION ALL\n")
        );
        let name = self.relation(
            "mixed recursive execution",
            format!(
                "SELECT d.k,r.c1 AS work,r.c0 AS truth{} FROM {} d {} WHERE relation_state_end(CASE WHEN r.c0 IS NOT NULL THEN d.state_scope END)=1",
                result_writes.iter().enumerate().map(|(index,slot)|format!(",r.c{} AS c{slot}",index+2)).collect::<String>(),
                scoped_input,
                bridge(&q, 2+result_writes.len())
            ),
            true,
            "one immutable capture context; seven numeric recursive columns",
        );
        self.profile.last_mut().unwrap()["mixedControl"] = json!({"instructions":p.code.len()+1,"functions":p.functions.len(),"regions":region_profiles,"recursiveColumns":7,"continuationCells":8,"captureStackCopies":false,"typedPersistentSnapshots":true,"progress":"monotone SCC; zero-progress edges acyclic","nativeComposition":false,"bypass":bypass});
        Ok(Output { name })
    }
}
