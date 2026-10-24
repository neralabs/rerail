//! SQL owns cross-document recursion, typed snapshots, rollback and parse exports.
use super::super::lower::parse_recursive_lower::{Instruction, Program};
use super::{Demand, Output, Slots, Sql, fields, needed, ordered, value};
use crate::code_rule_dsl::sql::relational::emit::condition;
use serde_json::json;

fn push(alias: &str, guard: &str, parent: &str, cells: &[&str]) -> String {
    let fields = cells
        .iter()
        .enumerate()
        .map(|(index, value)| format!(" AND {alias}.v{index} IS ({value})"))
        .collect::<String>();
    format!(
        "LEFT JOIN relation_continuation {alias} ON {alias}.mode IS CASE WHEN {guard} THEN 1 END AND {alias}.parent_input IS {parent}{fields}"
    )
}
fn cases(arms: &[String], key: &str, default: &str) -> String {
    if arms.is_empty() {
        default.into()
    } else {
        format!("CASE {key} {} ELSE {default} END", arms.join(" "))
    }
}
impl Sql {
    pub(super) fn parsed_recursive(
        &mut self,
        p: &Program,
        entry: usize,
        input: &Demand,
    ) -> Result<Output, String> {
        let mut generation = self.generation.clone();
        generation.return_cells = generation
            .return_cells
            .checked_add(9)
            .filter(|n| *n <= 128)
            .ok_or("parsed recursive numeric return exceeds 128 active cells")?;
        generation
            .return_peak
            .set(generation.return_peak.get().max(generation.return_cells));
        generation.memo_bypass = true;
        generation.utility_depth = 0;
        generation.dynamic_depth = Some("state.level".into());
        let slots: Slots = input
            .slots
            .union(&p.mentions)
            .copied()
            .chain(p.writes.iter().copied())
            .collect();
        if slots.iter().any(|slot| *slot >= 128) {
            return Err("parsed recursive captures exceed 128 existing slots".into());
        }
        let mut code = vec![];
        let mut children = vec![];
        let mut first = vec![];
        let mut next = vec![];
        let mut strict = vec![];
        let mut parse_terms = vec![];
        let mut parse_exports = vec![];
        let mut regions = vec![];
        for (pc, instruction) in p.code.iter().enumerate() {
            self.generation.operator_as("parsed recursive control")?;
            let (op, arg) = match instruction {
                Instruction::Return => (0, 0),
                Instruction::Eval(id) => {
                    regions.push((pc, *id));
                    (1, *id)
                }
                Instruction::Call(id) => (2, *id),
                Instruction::Boolean {
                    all,
                    children: items,
                } => {
                    for (ordinal, (child, binds)) in items.iter().enumerate() {
                        children.push(format!("({pc},{ordinal},{child},{})", usize::from(*binds)));
                    }
                    (3, usize::from(*all))
                }
                Instruction::Not(child) => (4, *child),
                Instruction::Search {
                    end,
                    body,
                    strict: binds,
                    filter,
                } => {
                    let filter = filter
                        .as_ref()
                        .map(|set| condition(set, "candidate"))
                        .transpose()?
                        .unwrap_or_else(|| "1".into());
                    let domain = if *end {
                        "candidate.id>s.node AND candidate.id<=n.\"last\""
                    } else {
                        "candidate.parent=s.node"
                    };
                    first.push(format!("WHEN {pc} THEN (SELECT candidate.id FROM node candidate WHERE candidate.file=s.file AND ({domain}) AND ({filter}) ORDER BY candidate.id LIMIT 1)"));
                    let domain = if *end {
                        "candidate.id>p.c8 AND candidate.id<=p.c5"
                    } else {
                        "candidate.parent=p.c2 AND candidate.id>n.\"last\" AND candidate.id<=p.c5"
                    };
                    next.push(format!("WHEN {pc} THEN (SELECT candidate.id FROM node candidate WHERE candidate.file=p.c1 AND ({domain}) AND ({filter}) ORDER BY candidate.id LIMIT 1)"));
                    strict.push(format!("WHEN {pc} THEN {}", usize::from(*binds)));
                    (5, *body)
                }
                Instruction::Parse {
                    term,
                    body,
                    exports,
                } => {
                    parse_terms.push(format!("WHEN {pc} THEN {term}"));
                    let mut env = "p.c7".to_string();
                    for (index, slot) in exports.iter().enumerate() {
                        let mapped = format!(
                            "relation_parse_export(p.file,p.c1,relation_state_get(p.env,{slot}))"
                        );
                        let cell = format!(
                            "relation_capture_export(relation_state_get(p.c7,{slot}),{mapped})"
                        );
                        let update = if index == 0 {
                            "relation_state_put"
                        } else {
                            "relation_state_put_owned"
                        };
                        env = format!("{update}(p.scope,{env},{slot},{cell})");
                    }
                    parse_exports.push(format!("WHEN {pc} THEN {env}"));
                    (6, *body)
                }
            };
            code.push(format!("({pc},{op},{arg})"));
        }
        self.generation.operator_as("parsed recursive control")?;
        let root_call = p.code.len();
        code.push(format!("({root_call},2,{entry})"));
        let code = self.constant_relation(
            "parsed recursive control",
            format!(
                "SELECT column1 pc,column2 op,column3 a FROM (VALUES {})",
                code.join(",")
            ),
        );
        let functions = self.constant_relation(
            "parsed utility entries",
            format!(
                "SELECT column1 id,column2 entry FROM (VALUES {})",
                p.functions
                    .iter()
                    .enumerate()
                    .map(|(id, entry)| format!("({id},{entry})"))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        );
        let children=self.constant_relation("parsed Boolean child order",if children.is_empty(){"SELECT NULL pc,NULL ordinal,NULL child,NULL strict WHERE 0".into()}else{format!("SELECT column1 pc,column2 ordinal,column3 child,column4 strict FROM (VALUES {})",children.join(","))});
        let seed = |guard: &str| {
            format!(
                "SELECT machine.scope AS scope,machine.file AS file,machine.node AS node,machine.pc AS pc,machine.level AS level,machine.work AS work,machine.handle AS handle,machine.env AS env,machine.truth AS truth WHERE {guard}"
            )
        };
        let mut phases = vec![];
        let call = format!(
            r#"WITH s AS MATERIALIZED ({}), entered AS MATERIALIZED (SELECT s.*,f.entry,CASE WHEN s.level<256 THEN relation_peak_note(s.level+1) ELSE 1 END ready FROM s JOIN {code} i ON i.pc=s.pc JOIN {functions} f ON f.id=i.a)
SELECT s.scope,s.file,s.node,CASE WHEN s.level>=256 OR s.work=0 THEN 0 ELSE s.entry END,CASE WHEN s.level<256 THEN s.level+1 ELSE s.level END,CASE WHEN s.level<256 AND s.work>0 THEN s.work-1 ELSE s.work END,COALESCE(frame.handle,s.handle),s.env,CASE WHEN s.level>=256 THEN -1 WHEN s.work=0 THEN -2 ELSE 0 END FROM entered s {} WHERE s.ready=1"#,
            seed(&format!("machine.pc IN (SELECT pc FROM {code} WHERE op=2)")),
            push(
                "frame",
                "s.level<256",
                "s.handle",
                &[
                    "0", "s.file", "s.node", "s.level", "s.pc", "0", "0", "s.env"
                ]
            )
        );
        phases.push((
            call,
            format!("machine.pc IN (SELECT pc FROM {code} WHERE op=2)"),
        ));
        let boolean = format!(
            "WITH reached AS MATERIALIZED ({}),s AS MATERIALIZED (SELECT s.*,i.a,c.child FROM reached s JOIN {code} i ON i.pc=s.pc LEFT JOIN {children} c ON c.pc=s.pc AND c.ordinal=0) SELECT s.scope,s.file,s.node,COALESCE(s.child,0),s.level,s.work,COALESCE(frame.handle,s.handle),s.env,CASE WHEN s.child IS NULL THEN s.a ELSE 0 END FROM s {}",
            seed(&format!("machine.pc IN (SELECT pc FROM {code} WHERE op=3)")),
            push(
                "frame",
                "s.child IS NOT NULL",
                "s.handle",
                &[
                    "CASE WHEN s.a THEN 2 ELSE 3 END",
                    "s.file",
                    "s.node",
                    "s.level",
                    "s.pc",
                    "0",
                    "0",
                    "s.env"
                ]
            )
        );
        phases.push((
            boolean,
            format!("machine.pc IN (SELECT pc FROM {code} WHERE op=3)"),
        ));
        let not = format!(
            "WITH s AS MATERIALIZED ({}) SELECT s.scope,s.file,s.node,i.a,s.level,s.work,frame.handle,s.env,0 FROM s JOIN {code} i ON i.pc=s.pc {}",
            seed(&format!("machine.pc IN (SELECT pc FROM {code} WHERE op=4)")),
            push(
                "frame",
                "1",
                "s.handle",
                &[
                    "4", "s.file", "s.node", "s.level", "s.pc", "0", "0", "s.env"
                ]
            )
        );
        phases.push((
            not,
            format!("machine.pc IN (SELECT pc FROM {code} WHERE op=4)"),
        ));
        let search = format!(
            "WITH reached AS MATERIALIZED ({}),s AS MATERIALIZED (SELECT s.*,n.\"last\" bound,{} child FROM reached s JOIN node n ON n.file=s.file AND n.id=s.node) SELECT s.scope,s.file,COALESCE(s.child,s.node),CASE WHEN s.child IS NULL THEN 0 ELSE i.a END,s.level,s.work,COALESCE(frame.handle,s.handle),s.env,0 FROM s JOIN {code} i ON i.pc=s.pc {}",
            seed(&format!("machine.pc IN (SELECT pc FROM {code} WHERE op=5)")),
            cases(&first, "s.pc", "NULL"),
            push(
                "frame",
                "s.child IS NOT NULL",
                "s.handle",
                &[
                    "1", "s.file", "s.node", "s.level", "s.pc", "s.bound", "0", "s.env", "s.child"
                ]
            )
        );
        phases.push((
            search,
            format!("machine.pc IN (SELECT pc FROM {code} WHERE op=5)"),
        ));
        let parse = format!(
            "WITH s AS MATERIALIZED ({}),parsed AS MATERIALIZED (SELECT s.*,p.truth parsed_truth,p.work parsed_work,p.parsed_file,p.parsed_node FROM s CROSS JOIN relation_parse_budget({},s.file,s.node,s.work) p) SELECT s.scope,CASE WHEN s.parsed_truth=1 THEN s.parsed_file ELSE s.file END,CASE WHEN s.parsed_truth=1 THEN s.parsed_node ELSE s.node END,CASE WHEN s.parsed_truth=1 THEN i.a ELSE 0 END,s.level,s.parsed_work,COALESCE(frame.handle,s.handle),CASE WHEN s.parsed_truth=1 THEN 0 ELSE s.env END,CASE WHEN s.parsed_truth=1 THEN 0 ELSE s.parsed_truth END FROM parsed s JOIN {code} i ON i.pc=s.pc {}",
            seed(&format!("machine.pc IN (SELECT pc FROM {code} WHERE op=6)")),
            cases(&parse_terms, "s.pc", "NULL"),
            push(
                "frame",
                "s.parsed_truth=1",
                "s.handle",
                &[
                    "5", "s.file", "s.node", "s.level", "s.pc", "0", "0", "s.env"
                ]
            )
        );
        phases.push((
            parse,
            format!("machine.pc IN (SELECT pc FROM {code} WHERE op=6)"),
        ));
        let bool_again = push(
            "again",
            "p.c0 IN (2,3) AND p.truth>=0 AND NOT (p.c0=2 AND p.truth=0) AND NOT (p.c0=3 AND (p.truth=1 OR (p.truth=2 AND p.strict))) AND p.child IS NOT NULL",
            "p.parent",
            &[
                "p.c0",
                "p.c1",
                "p.c2",
                "p.c3",
                "p.c4",
                "p.c5+1",
                "p.c6 OR p.truth=2",
                "p.c7",
            ],
        );
        let search_again = push(
            "search_again",
            "p.c0=1 AND p.truth IN (0,2) AND NOT (p.truth=2 AND p.strict) AND p.candidate IS NOT NULL",
            "p.parent",
            &[
                "1",
                "p.c1",
                "p.c2",
                "p.c3",
                "p.c4",
                "p.c5",
                "p.c6 OR p.truth=2",
                "p.c7",
                "p.candidate",
            ],
        );
        // No recursive conjunction sibling can write before another call: all
        // siblings are total pure predicates. A newly produced snapshot therefore
        // cannot become a later frame's saved input. After a materialized rollback
        // or parse mapping, its displaced handle is exclusive unless it is the
        // current frame's original input, which remains owned by ancestor frames.
        let dispatch = format!(
            r#"WITH reached AS MATERIALIZED ({}),popped AS MATERIALIZED (SELECT s.*,frame.parent,frame.c0,frame.c1,frame.c2,frame.c3,frame.c4,frame.c5,frame.c6,frame.c7,frame.c8 FROM reached s LEFT JOIN relation_continuation frame ON frame.mode IS CASE WHEN s.handle IS NOT NULL THEN 2 END AND frame.h IS s.handle),
classified AS MATERIALIZED (SELECT p.*,CASE WHEN p.c0 IN (2,3) THEN (SELECT child FROM {children} WHERE pc=p.c4 AND ordinal=p.c5+1) END child,CASE WHEN p.c0 IN (2,3) THEN COALESCE((SELECT strict FROM {children} WHERE pc=p.c4 AND ordinal=p.c5),0) WHEN p.c0=1 THEN {} ELSE 0 END strict,CASE WHEN p.c0=1 AND p.truth IN (0,2) THEN {} END candidate FROM popped p LEFT JOIN node n ON p.c0=1 AND n.file=p.c1 AND n.id=p.c8),
transition AS MATERIALIZED (SELECT p.scope AS scope,CASE WHEN p.handle IS NULL THEN p.file ELSE p.c1 END AS file,
CASE WHEN search_again.handle IS NOT NULL THEN p.candidate WHEN p.handle IS NULL THEN p.node ELSE p.c2 END AS node,
CASE WHEN p.handle IS NULL THEN -1 WHEN again.handle IS NOT NULL THEN p.child WHEN search_again.handle IS NOT NULL THEN (SELECT a FROM {code} WHERE pc=p.c4) ELSE 0 END AS pc,
CASE WHEN p.handle IS NULL THEN p.level ELSE p.c3 END AS level,p.work,COALESCE(again.handle,search_again.handle,p.parent) AS handle,
CASE WHEN p.handle IS NULL THEN p.env WHEN again.handle IS NOT NULL THEN CASE WHEN p.c0=2 THEN p.env ELSE p.c7 END WHEN search_again.handle IS NOT NULL THEN p.c7 WHEN p.c0=4 OR p.truth<>1 OR (p.c0=2 AND p.c6=1) THEN p.c7 WHEN p.c0=5 THEN {} ELSE p.env END AS env,
CASE WHEN p.handle IS NULL THEN p.truth WHEN again.handle IS NOT NULL OR search_again.handle IS NOT NULL THEN 0 WHEN p.c0=4 THEN CASE p.truth WHEN 0 THEN 1 WHEN 1 THEN 0 ELSE p.truth END WHEN p.truth=0 AND p.c6=1 AND p.c0 IN (1,3) THEN 2 WHEN p.truth=1 AND p.c6=1 AND p.c0=2 THEN 2 ELSE p.truth END AS truth,p.env AS previous_env,p.c7 AS saved_env,p.handle AS previous_frame
FROM classified p {bool_again} {search_again})
SELECT scope,file,node,pc,level,work,handle,env,truth FROM transition WHERE relation_state_drop(CASE WHEN previous_frame IS NOT NULL AND previous_env<>env AND previous_env<>COALESCE(saved_env,0) THEN previous_env END)=1"#,
            seed("machine.pc=0"),
            cases(&strict, "p.c4", "0"),
            cases(&next, "p.c4", "NULL"),
            cases(&parse_exports, "p.c4", "p.c7")
        );
        phases.push((dispatch, "machine.pc=0".into()));
        let mut profiles = vec![];
        for (pc, id) in regions {
            let plan = &p.regions[id];
            let region_slots = needed(plan);
            let columns = fields(&region_slots, |slot| {
                format!("relation_state_get(state.env,{slot})")
            });
            let exports = plan.writes.clone();
            let mut region_generation = generation.clone();
            region_generation.return_cells = region_generation
                .return_cells
                .checked_add(2 + exports.len())
                .filter(|cells| *cells <= 128)
                .ok_or("parsed recursive region exceeds 128 active return cells")?;
            region_generation.return_peak.set(
                region_generation
                    .return_peak
                    .get()
                    .max(region_generation.return_cells),
            );
            let mut region = Sql {
                definitions: vec![],
                profile: vec![],
                correlated: true,
                generation: region_generation,
            };
            let demand = Demand {
                name: region.relation("parsed recursive current capture demand", format!("SELECT 1 AS k,state.file,state.node,state.work{columns} WHERE state.pc={pc}"), true, "current document and fields read from persistent typed snapshot"),
                slots: region_slots,
            };
            let answer = region.emit(plan, &demand)?;
            let payload = fields(&exports, |slot| format!("o.c{slot}"));
            let query = format!(
                "WITH RECURSIVE {} SELECT o.truth,o.work{payload} FROM {} o",
                region.definitions.join(",\n"),
                answer.name
            );
            let profile = json!({"relations":region.profile,"exports":exports,"singleReachedDemand":true,"typedReturnColumns":2+exports.len()});
            let mut env = "state.env".to_string();
            for (index, slot) in exports.iter().enumerate() {
                let update = if index == 0 {
                    "relation_state_put"
                } else {
                    "relation_state_put_owned"
                };
                env = format!("{update}(state.scope,{env},{slot},r.c{})", index + 2);
            }
            let truth = if p.three {
                "r.c0"
            } else {
                "CASE WHEN r.c0=2 THEN 0 ELSE r.c0 END"
            };
            let q = format!(
                "WITH state AS MATERIALIZED ({}) SELECT state.scope,state.file,state.node,0,state.level,r.c1,state.handle,CASE WHEN r.c0=1 THEN {env} ELSE state.env END,{truth} FROM state {}",
                seed(&format!("machine.pc={pc}")),
                ordered::nested::bridge(&query, 2 + exports.len())
            );
            phases.push((q, format!("machine.pc={pc}")));
            profiles.push(profile);
        }
        let depth = self.generation.depth_expression();
        let mut initial = "0".to_string();
        for slot in &input.slots {
            initial = format!(
                "relation_state_put_owned(owner.scope,{initial},{slot},{})",
                value(input, "d", *slot)
            );
        }
        let mut arms = vec![format!(
            "SELECT owner.scope scope,d.file file,d.node node,{root_call} pc,({depth}) level,d.work work,NULL handle,{initial} env,0 truth FROM owner"
        )];
        for (query, guard) in phases {
            arms.push(format!("SELECT r.c0,r.c1,r.c2,r.c3,r.c4,r.c5,r.c6,r.c7,r.c8 FROM schedule machine {} WHERE {guard}",ordered::nested::bridge(&query,9)));
        }
        let output_columns = p.outputs[entry]
            .iter()
            .map(|slot| format!(",relation_state_get(env,{slot}) c{slot}"))
            .collect::<String>();
        let selected = p.outputs[entry]
            .iter()
            .map(|slot| format!(",c{slot}"))
            .collect::<String>();
        let query = format!(
            "WITH RECURSIVE owner AS MATERIALIZED (SELECT relation_state_begin() scope WHERE d.file IS NOT NULL),schedule AS ({}),returned AS MATERIALIZED (SELECT scope,truth,work{output_columns} FROM schedule WHERE pc=-1),closed AS MATERIALIZED (SELECT returned.*,relation_state_end(returned.scope) ready FROM returned) SELECT truth,work{selected} FROM closed WHERE ready=1",
            arms.join(" UNION ALL ")
        );
        let columns = p.outputs[entry]
            .iter()
            .enumerate()
            .map(|(index, slot)| format!(",r.c{} c{slot}", index + 2))
            .collect::<String>();
        let name=self.relation("cross-document parsed recursive execution",format!("SELECT d.k,r.c1 work,r.c0 truth{columns} FROM {} d {}",input.name,ordered::nested::bridge(&query,2+p.outputs[entry].len())),true,"SQL-owned recursion; typed persistent snapshot handles; fresh parsed environments; successful mapped exports");
        self.profile.last_mut().unwrap()["childPrograms"] = json!({"parsedRecursion":{"regions":profiles,"controlInstructions":p.code.len(),"numericRecursiveColumns":9,"singleReachedDemand":true}});
        Ok(Output { name })
    }
}
