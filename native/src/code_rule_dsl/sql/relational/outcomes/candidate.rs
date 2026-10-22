//! Node-local captures can only retain an input witness or replace it with the candidate.
//! Integer masks track candidate bindings and withheld inputs; no environment is serialized.
use super::super::super::Slots;
use super::{Demand, Op, Plan};

pub(super) struct Candidate {
    pub expression: String,
    pub bits: Vec<(usize, i64)>,
}
struct Local<'a> {
    input: &'a Demand,
    fresh: bool,
    bits: Vec<(usize, i64)>,
    definitions: Vec<String>,
}
impl Local<'_> {
    fn mask(&self, slots: &Slots) -> i64 {
        self.bits
            .iter()
            .filter(|(s, _)| slots.contains(s))
            .map(|(_, b)| *b)
            .sum()
    }
    fn relation(&mut self, body: String) -> String {
        let name = format!("candidate{}", self.definitions.len());
        self.definitions
            .push(format!("{name} AS MATERIALIZED ({body})"));
        name
    }
    fn scalar(&self, plan: &Plan, bound: &str) -> Result<Option<String>, String> {
        Ok(Some(match &plan.op {
            Op::Bind(slot) => {
                let bit = self.mask(&std::iter::once(*slot).collect());
                let input = if self.fresh {
                    "NULL".into()
                } else {
                    super::super::value(self.input, "d", *slot)
                };
                format!(
                    "CASE WHEN n.named AND (CASE WHEN ({bound}&{bit})<>0 OR {input} IS NULL THEN 1 ELSE relation_capture_equal(n.file,{input},n.id) END) THEN 1 ELSE 0 END"
                )
            }
            Op::Not(child) => {
                let Some(test) = self.scalar(child, bound)? else {
                    return Ok(None);
                };
                format!("CASE ({test}) WHEN 0 THEN 1 WHEN 1 THEN 0 ELSE 2 END")
            }
            Op::All(_) | Op::Any(_) => return Ok(None),
            _ => super::truth(plan, 0)?,
        }))
    }
    fn emit(&mut self, plan: &Plan, input: &str) -> Result<String, String> {
        if let Some(test) = self.scalar(plan, "i.bound")? {
            let tested = self.relation(format!("SELECT i.bound,{test} AS truth FROM {input} i"));
            let writes = self.mask(&plan.writes);
            return Ok(self.relation(format!("SELECT truth,CASE WHEN truth=1 THEN bound|{writes} ELSE bound END AS bound FROM {tested}")));
        }
        if let Op::Not(child) = &plan.op {
            let output = self.emit(child, input)?;
            // A negation's binding column is unobservable: every parent retains its own input.
            return Ok(self.relation(format!("SELECT CASE truth WHEN 0 THEN 1 WHEN 1 THEN 0 ELSE 2 END AS truth,0 AS bound FROM {output}")));
        }
        let (items, all) = match &plan.op {
            Op::All(items) => (items, true),
            Op::Any(items) => (items, false),
            _ => return Err("ordered captures: node-local Boolean rules required".into()),
        };
        let mut state = self.relation(format!(
            "SELECT {} AS truth,0 AS withheld,0 AS open,bound FROM {input}",
            usize::from(all)
        ));
        for child in items {
            let mentions = self.mask(&child.mentions);
            let writes = self.mask(&child.writes);
            let node_writes = self.mask(&child.node_writes);
            let reached = if all {
                format!("s.truth<>0 AND (s.withheld&{mentions})=0")
            } else {
                "s.truth=0".into()
            };
            let (source, test, bound) = if let Some(expression) = self.scalar(child, "s.bound")? {
                let tested = self.relation(format!(
                    "SELECT s.*,CASE WHEN {reached} THEN {expression} END AS tested FROM {state} s"
                ));
                (
                    format!("{tested} s"),
                    "s.tested".to_string(),
                    format!("s.bound|{writes}"),
                )
            } else {
                let demand = self.relation(format!("SELECT bound FROM {state} s WHERE {reached}"));
                let result = self.emit(child, &demand)?;
                (
                    format!("{state} s LEFT JOIN {result} r ON 1"),
                    "r.truth".into(),
                    "r.bound".into(),
                )
            };
            let committed = if writes != 0 {
                format!("CASE WHEN {test}=1 THEN {bound} ELSE s.bound END")
            } else {
                "s.bound".into()
            };
            let (truth, withheld, open) = if all {
                (
                    format!(
                        "CASE WHEN s.truth=0 OR {test}=0 THEN 0 WHEN s.truth=2 OR {test}=2 THEN 2 ELSE 1 END"
                    ),
                    format!(
                        "s.withheld|CASE WHEN {test}=2 OR ({test} IS NULL AND s.truth<>0) THEN {node_writes} ELSE 0 END"
                    ),
                    "0".into(),
                )
            } else {
                (
                    format!(
                        "CASE WHEN s.truth<>0 THEN s.truth WHEN {test}=1 THEN 1 WHEN {test}=2 AND {writes}<>0 THEN 2 ELSE 0 END"
                    ),
                    "0".into(),
                    format!("s.open OR COALESCE({test}=2,0)"),
                )
            };
            state=self.relation(format!("SELECT {truth} AS truth,{withheld} AS withheld,{open} AS open,{committed} AS bound FROM {source}"));
        }
        Ok(self.relation(format!(
            "SELECT {} AS truth,bound FROM {state}",
            if all {
                "truth"
            } else {
                "CASE WHEN truth=0 AND open THEN 2 ELSE truth END"
            }
        )))
    }
}
fn validate(plan: &Plan, depth: usize) -> Result<(), String> {
    if depth > 16 {
        return Err("ordered captures: Boolean depth exceeds 16".into());
    }
    match &plan.op {
        Op::Predicate(_) | Op::Unknown(_) | Op::Value(_) | Op::Bind(_) => Ok(()),
        Op::Not(child) => validate(child, depth + 1),
        Op::All(items) | Op::Any(items) => {
            for item in items {
                validate(item, depth + 1)?;
            }
            Ok(())
        }
        _ => Err("ordered captures: nested traversal requires a reached-demand contract".into()),
    }
}
pub(super) fn compile(
    plan: &Plan,
    input: &Demand,
    fresh: bool,
    export: bool,
) -> Result<Candidate, String> {
    validate(plan, 0)?;
    let slots: Slots = plan.mentions.union(&plan.writes).copied().collect();
    if slots.len() > 61 {
        return Err("ordered captures: more than 61 local capture slots".into());
    }
    let bits: Vec<_> = slots
        .into_iter()
        .enumerate()
        .map(|(index, slot)| (slot, 1i64 << index))
        .collect();
    let mut local = Local {
        input,
        fresh,
        bits: bits.clone(),
        definitions: vec![],
    };
    let initial = local.relation("SELECT 0 AS bound".into());
    let output = local.emit(plan, &initial)?;
    let exported = local.mask(&plan.writes);
    let result = if export {
        format!("CASE WHEN truth=1 THEN 1+((bound&{exported})<<2) ELSE truth END")
    } else {
        "truth".into()
    };
    let body = format!(
        "WITH {} SELECT {result} FROM {output}",
        local.definitions.join(",\n")
    );
    let expression = if fresh || input.slots.is_disjoint(&plan.mentions) {
        format!("({body})")
    } else {
        format!("(SELECT ({body}) FROM {} d WHERE d.k=w.k)", input.name)
    };
    Ok(Candidate { expression, bits })
}
