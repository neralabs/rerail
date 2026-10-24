//! SQL owns finite pure control, searches, utility entries, and numeric continuation records.
use super::super::lower::pure_recursive_lower::{Direction, Instruction, Program, Walk};
use super::{Demand, Output, Sql};
use crate::code_rule_dsl::sql::relational::{emit::condition, ir::Set};
use serde_json::json;

fn predicate(set: Option<&Set>, alias: &str) -> Result<String, String> {
    set.map(|set| condition(set, alias))
        .transpose()
        .map(|p| p.unwrap_or("1".into()))
}
fn first(walk: &Walk) -> Result<String, String> {
    let filter = if walk.stop.is_none() {
        predicate(walk.filter.as_ref(), "candidate")?
    } else {
        "1".into()
    };
    Ok(match walk.direction {
        Direction::Child => {
            let domain = if let Some(field) = walk.field {
                format!(
                    "candidate.parent=w.node AND candidate.field={field} AND candidate.ffirst=1"
                )
            } else if walk.end {
                "candidate.id>w.node AND candidate.id<=n.\"last\"".into()
            } else {
                "candidate.parent=w.node".into()
            };
            format!(
                "(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND ({domain}) AND ({filter}) ORDER BY candidate.id LIMIT 1)"
            )
        }
        Direction::Parent => walk.field.map_or("n.parent".into(), |field| {
            format!("CASE WHEN n.field={field} AND n.ffirst=1 THEN n.parent END")
        }),
        Direction::Previous => "n.prev".into(),
        Direction::Next => "n.next".into(),
    })
}
fn next(walk: &Walk) -> Result<String, String> {
    if walk.field.is_some() || (!walk.end && !matches!(walk.direction, Direction::Child)) {
        return Ok("NULL".into());
    }
    let stop = predicate(walk.stop.as_ref(), "n")?;
    let stop = if walk.stop.is_some() {
        stop
    } else {
        "0".into()
    };
    Ok(match walk.direction {
        Direction::Child => {
            let filter = if walk.stop.is_none() {
                predicate(walk.filter.as_ref(), "candidate")?
            } else {
                "1".into()
            };
            let domain = if walk.end {
                format!(
                    "candidate.id>CASE WHEN ({stop}) THEN n.\"last\" ELSE p.c6 END AND candidate.id<=p.c7"
                )
            } else {
                "candidate.parent=p.c1 AND candidate.id>n.\"last\"".into()
            };
            format!(
                "(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND ({domain}) AND ({filter}) ORDER BY candidate.id LIMIT 1)"
            )
        }
        Direction::Parent => format!("CASE WHEN NOT ({stop}) THEN n.parent END"),
        Direction::Previous => format!("CASE WHEN NOT ({stop}) THEN n.prev END"),
        Direction::Next => format!("CASE WHEN NOT ({stop}) THEN n.next END"),
    })
}
fn push(alias: &str, mode: &str, parent: &str, cells: &[String]) -> String {
    let values = cells
        .iter()
        .enumerate()
        .map(|(i, value)| format!(" AND {alias}.v{i} IS {value}"))
        .collect::<String>();
    format!(
        "LEFT JOIN relation_continuation {alias} ON {alias}.mode IS ({mode}) AND {alias}.parent_input IS {parent}{values}"
    )
}
impl Sql {
    pub(super) fn pure_recursive(
        &mut self,
        program: &Program,
        input: &Demand,
    ) -> Result<Output, String> {
        let mut values = Vec::new();
        let mut calls = Vec::new();
        let mut tests = Vec::new();
        let mut starts = Vec::new();
        let mut following = Vec::new();
        let mut filters = Vec::new();
        for (pc, instruction) in program.instructions.iter().enumerate() {
            self.generation.operator_as("pure recursive instruction")?;
            let (op, a, b, c) = match instruction {
                Instruction::Return(true) => (0, 0, 0, 0),
                Instruction::Return(false) => (1, 0, 0, 0),
                Instruction::Dispatch => (2, 0, 0, 0),
                Instruction::Failure => (3, 0, 0, 0),
                Instruction::Test { predicate, yes, no } => {
                    tests.push(format!("WHEN {pc} THEN {}", condition(predicate, "n")?));
                    (4, *yes, *no, 0)
                }
                Instruction::Call { utility, yes, no } => {
                    calls.push(pc.to_string());
                    (5, program.functions[*utility], *yes, *no)
                }
                Instruction::Search {
                    walk,
                    body,
                    yes,
                    no,
                } => {
                    starts.push(format!("WHEN {pc} THEN {}", first(walk)?));
                    following.push(format!("WHEN {pc} THEN {}", next(walk)?));
                    let filter =
                        if matches!(walk.direction, Direction::Child) && walk.stop.is_none() {
                            "1".into()
                        } else {
                            predicate(walk.filter.as_ref(), "n")?
                        };
                    filters.push(format!("WHEN {pc} THEN {filter}"));
                    (6, *body, *yes, *no)
                }
            };
            values.push(format!("({pc},{op},{a},{b},{c})"));
        }
        let code=self.relation("pure recursive finite control",format!("SELECT column1 AS pc,column2 AS op,column3 AS a,column4 AS b,column5 AS c FROM (VALUES {})",values.join(",")),true,"pc,operation,typed control targets");
        let walk = format!("outcome{}", self.definitions.len());
        let depth = self.generation.depth_expression();
        let first = format!("CASE w.pc {} END", starts.join(" "));
        let next = format!("CASE p.c8 {} END", following.join(" "));
        let test = if tests.is_empty() {
            "0".into()
        } else {
            format!("CASE w.pc {} END", tests.join(" "))
        };
        let filter = if filters.is_empty() {
            "1".into()
        } else {
            format!("CASE p.c8 {} ELSE 1 END", filters.join(" "))
        };
        let call = push(
            "s",
            "CASE WHEN w.level<256 AND w.work>0 THEN 1 END",
            "w.handle",
            &[
                "0".into(),
                "w.node".into(),
                "i.b".into(),
                "i.c".into(),
                "w.level".into(),
            ],
        );
        let search = push(
            "s",
            "CASE WHEN target.c0 IS NOT NULL THEN 1 END",
            "w.handle",
            &[
                "1".into(),
                "w.node".into(),
                "i.b".into(),
                "i.c".into(),
                "w.level".into(),
                "i.a".into(),
                "target.c0".into(),
                "n.\"last\"".into(),
                "w.pc".into(),
            ],
        );
        let advance = push(
            "s",
            "CASE WHEN p.c0=1 AND w.pc=1 AND target.c0 IS NOT NULL THEN 1 END",
            "p.parent",
            &[
                "1".into(),
                "p.c1".into(),
                "p.c2".into(),
                "p.c3".into(),
                "p.c4".into(),
                "p.c5".into(),
                "target.c0".into(),
                "p.c7".into(),
                "p.c8".into(),
            ],
        );
        let note = if self.generation.peak_tracking {
            // WHERE may be evaluated before the code-table join. Guard by the
            // reached PC itself, so return/dispatch rows cannot note an entry.
            format!(
                " AND CASE WHEN w.pc IN ({}) AND w.level<256 THEN relation_peak_note(w.level+1) ELSE 1 END=1",
                calls.join(",")
            )
        } else {
            String::new()
        };
        let body = format!(
            r#"SELECT d.k,d.file,d.node,{entry} AS pc,d.work,({depth}) AS level,NULL AS handle,0 AS truth FROM {input} d
UNION ALL
SELECT w.k,w.file,w.node,CASE WHEN ({test}) THEN i.a ELSE i.b END,w.work,w.level,w.handle,w.truth
FROM {walk} w JOIN {code} i ON i.pc=w.pc AND i.op=4 JOIN node n ON n.file=w.file AND n.id=w.node
UNION ALL
SELECT w.k,w.file,w.node,CASE WHEN w.level>=256 OR w.work=0 THEN 3 ELSE i.a END,CASE WHEN w.level<256 AND w.work>0 THEN w.work-1 ELSE w.work END,CASE WHEN s.handle IS NOT NULL THEN w.level+1 ELSE w.level END,COALESCE(s.handle,w.handle),CASE WHEN w.level>=256 THEN -1 WHEN w.work=0 THEN -2 ELSE w.truth END
FROM {walk} w JOIN {code} i ON i.pc=w.pc AND i.op=5 {call} WHERE 1{note}
UNION ALL
SELECT w.k,w.file,COALESCE(target.c0,w.node),CASE WHEN target.c0 IS NULL THEN i.c ELSE 2 END,w.work,w.level,COALESCE(s.handle,w.handle),w.truth
FROM {walk} w JOIN {code} i ON i.pc=w.pc AND i.op=6 JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN relation_row target ON (target.c0) IS (SELECT {first}) {search}
UNION ALL
SELECT w.k,w.file,w.node,CASE WHEN ({filter}) THEN p.c5 ELSE 1 END,w.work,w.level,w.handle,w.truth
FROM {walk} w CROSS JOIN relation_continuation p ON p.mode IS CASE WHEN w.pc=2 AND w.handle IS NOT NULL THEN 0 END AND p.h IS w.handle JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.pc=2
UNION ALL
SELECT w.k,w.file,w.node,NULL,w.work,w.level,NULL,CASE WHEN w.pc=0 THEN 1 ELSE 0 END FROM {walk} w WHERE w.pc IN (0,1) AND w.handle IS NULL
UNION ALL
SELECT w.k,w.file,CASE WHEN s.handle IS NOT NULL THEN target.c0 ELSE p.c1 END,CASE WHEN s.handle IS NOT NULL THEN 2 WHEN w.pc=0 THEN p.c2 ELSE p.c3 END,w.work,p.c4,COALESCE(s.handle,p.parent),w.truth
FROM {walk} w CROSS JOIN relation_continuation p ON p.mode IS CASE WHEN w.pc IN (0,1) AND w.handle IS NOT NULL THEN 2 END AND p.h IS w.handle LEFT JOIN node n ON n.file=w.file AND n.id=p.c6 CROSS JOIN relation_row target ON (target.c0) IS (SELECT CASE WHEN p.c0=1 AND w.pc=1 THEN {next} END) {advance} WHERE w.pc IN (0,1) AND w.handle IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,CASE WHEN w.handle IS NULL THEN NULL ELSE 3 END,w.work,w.level,p.parent,w.truth FROM {walk} w LEFT JOIN relation_continuation p ON p.mode IS CASE WHEN w.pc=3 AND w.handle IS NOT NULL THEN 2 END AND p.h IS w.handle WHERE w.pc=3"#,
            entry = program.entry,
            input = input.name
        );
        let machine = self.relation(
            "pure structural recursive execution",
            body,
            true,
            "k,file,node,pc,work,utility depth,numeric continuation,truth",
        );
        self.profile.last_mut().unwrap()["pureControl"] = json!({"instructions":program.instructions.len(),"unfusedInstructions":program.unfused_instructions,"sourceOperators":program.source_operators,"recursiveColumns":8,"continuationCells":9,"stackCopies":false,"progress":"monotone structural SCC; zero-move edges acyclic"});
        Ok(Output {
            name: self.relation(
                "pure recursive completed outcome",
                format!("SELECT k,work,truth FROM {machine} WHERE pc IS NULL"),
                true,
                "k,remaining work,truth",
            ),
        })
    }
}
