//! Compose pure callers from answer rows and ordered entry recipes. The entry
//! prefix handles work/depth failures directly; no original Boolean program is emitted.
use super::super::{
    pure_kind_summary, utility_projection_emit,
    utility_projection_lower::{Expr, Program},
};
use super::{Demand, Output, Sql};
use crate::code_rule_dsl::sql::relational::emit::condition;
use serde_json::json;
use std::collections::BTreeMap;

fn kind_case(
    strata: &[(Option<u16>, Option<pure_kind_summary::Answer>)],
    field: impl Fn(usize, &Option<pure_kind_summary::Answer>) -> String,
) -> String {
    let cases = strata
        .iter()
        .enumerate()
        .filter_map(|(i, (k, a))| k.map(|k| format!("WHEN {k} THEN {}", field(i, a))))
        .collect::<Vec<_>>()
        .join(" ");
    let last = strata.len() - 1;
    let otherwise = field(last, &strata[last].1);
    if cases.is_empty() {
        otherwise
    } else {
        format!("CASE n.kind {cases} ELSE {otherwise} END")
    }
}
impl Sql {
    pub(super) fn recursive_projection(
        &mut self,
        p: &Program,
        input: &Demand,
    ) -> Result<Option<Output>, String> {
        if self.generation.memo_bypass
            || !self.generation.parse_hosts.is_empty()
            || self.generation.components.borrow().roots.is_empty()
        {
            return Ok(None);
        }
        // All admission checks precede producer publication. A captured domain
        // must already have a closed-demand proof for every recursive input.
        if !input.slots.is_empty()
            && p.expressions.iter().any(|e| {
                if let Expr::Recursive(r) = e {
                    let projection = r.projection.as_ref().unwrap();
                    !self
                        .generation
                        .components
                        .borrow()
                        .closed_demands
                        .contains_key(&(projection as *const _ as usize))
                } else {
                    false
                }
            })
        {
            return Ok(None);
        }
        let mut identities = BTreeMap::new();
        let mut bindings = vec![];
        let mut columns = vec![];
        let mut joins = vec![];
        for e in &p.expressions {
            let Expr::Recursive(program) = e else {
                continue;
            };
            let key = std::rc::Rc::as_ptr(program) as usize;
            if identities.contains_key(&key) {
                continue;
            }
            let index = bindings.len();
            identities.insert(key, index);
            let strata = pure_kind_summary::compile(program);
            let filter = kind_case(&strata, |_, a| usize::from(a.is_none()).to_string());
            let Some(closed) =
                self.closed_projected_tail(program.projection.as_ref().unwrap(), input, &filter)?
            else {
                return Err("closed recursive producer lost its admission proof".into());
            };
            let metadata = kind_case(&strata, |i, a| {
                if a.is_some() {
                    i.to_string()
                } else {
                    "NULL".into()
                }
            });
            columns.push(format!("{metadata} class{index}"));
            for field in ["truth", "cost", "peak"] {
                let expr = kind_case(&strata, |_, a| {
                    a.as_ref().map_or_else(
                        || format!("s{index}.{field}"),
                        |a| match field {
                            "truth" => usize::from(a.truth).to_string(),
                            "cost" => a.cost.to_string(),
                            _ => a
                                .entries
                                .iter()
                                .map(|d| d + 1)
                                .max()
                                .unwrap_or(0)
                                .to_string(),
                        },
                    )
                });
                columns.push(format!("{expr} answer{index}_{field}"));
            }
            joins.push(format!(
                "LEFT JOIN {} s{index} ON s{index}.file=d.file AND s{index}.node=d.node",
                closed.summary
            ));
            let rows = strata
                .iter()
                .enumerate()
                .flat_map(|(class, (_, answer))| {
                    answer.iter().flat_map(move |a| {
                        a.entries.iter().enumerate().map(move |(ordinal, depth)| {
                            format!("({class},{},{depth})", ordinal + 1)
                        })
                    })
                })
                .collect::<Vec<_>>();
            let metadata = self.constant_relation(
                "closed recursive metadata entries",
                if rows.is_empty() {
                    "SELECT NULL class,NULL ordinal,NULL depth WHERE 0".into()
                } else {
                    format!(
                        "SELECT column1 class,column2 ordinal,column3 depth FROM (VALUES {})",
                        rows.join(",")
                    )
                },
            );
            bindings.push((closed, metadata));
        }
        let bound = self.relation(
            "closed recursive caller inputs",
            format!(
                "SELECT d.*,{} FROM {} d JOIN node n ON n.file=d.file AND n.id=d.node {}",
                columns.join(","),
                input.name,
                joins.join(" ")
            ),
            true,
            "one answer lookup per distinct recursive input",
        );
        let mut occurrences = vec![];
        let rendered = utility_projection_emit::render_mode(
            p,
            p.entry,
            |set| condition(set, "n"),
            |_, expr| {
                let Expr::Recursive(program) = expr else {
                    return Err(
                        "pure recursive caller contains an unbound utility or search".into(),
                    );
                };
                let index = identities[&(std::rc::Rc::as_ptr(program) as usize)];
                occurrences.push(index);
                Ok(utility_projection_emit::Summary {
                    truth: format!("d.answer{index}_truth"),
                    cost: format!("d.answer{index}_cost"),
                    peak: format!("d.answer{index}_peak"),
                })
            },
            true,
        )?;
        for _ in &p.expressions {
            self.generation.operator_as("closed recursive scalar")?;
        }
        let reached = rendered
            .reached
            .iter()
            .enumerate()
            .map(|(i, r)| format!(",({}) reached{i}", r.guard))
            .collect::<String>();
        let classified=self.relation("closed recursive caller projection",format!("SELECT d.*,{} truth,{} cost,{} peak{reached} FROM {bound} d JOIN node n ON n.file=d.file AND n.id=d.node",rendered.summary.truth,rendered.summary.cost,rendered.summary.peak),true,"ordered scalar composition without Boolean continuations");
        let mut entry_rows = vec![];
        for (site, index) in occurrences.iter().copied().enumerate() {
            let (closed, metadata) = &bindings[index];
            entry_rows.push(format!("SELECT d.k,{site} site,e.ordinal,e.depth FROM {classified} d JOIN {} e ON e.file=d.file AND e.root=d.node WHERE d.reached{site} AND d.class{index} IS NULL",closed.entries));
            entry_rows.push(format!("SELECT d.k,{site} site,e.ordinal,e.depth FROM {classified} d JOIN {metadata} e ON e.class=d.class{index} WHERE d.reached{site}"));
        }
        let ordinal = if occurrences.len() == 1 {
            "ordinal"
        } else {
            "row_number() OVER (PARTITION BY k ORDER BY site,ordinal)"
        };
        let entries = self.relation(
            "closed recursive caller entries",
            format!(
                "SELECT k,depth,{ordinal} ordinal FROM ({})",
                entry_rows.join(" UNION ALL ")
            ),
            true,
            "logical calls retain repeated occurrences and source order",
        );
        let depth = self.generation.depth_expression();
        let limits=self.relation("closed recursive caller limits",format!("SELECT d.*,CASE WHEN ({depth})+d.peak>256 THEN (SELECT min(e.ordinal) FROM {entries} e WHERE e.k=d.k AND ({depth})+e.depth>=256) END breach FROM {classified} d"),true,"first rejected entry precedes work charge");
        let note = if self.generation.peak_tracking {
            let peak = format!(
                "CASE WHEN d.breach IS NULL AND d.work>=d.cost THEN d.peak ELSE coalesce((SELECT max(e.depth+1) FROM {entries} e WHERE e.k=d.k AND e.ordinal-1<=d.work AND (d.breach IS NULL OR e.ordinal<d.breach)),0) END"
            );
            format!(
                " AND CASE WHEN ({peak})>0 THEN relation_peak_note(({depth})+({peak})) ELSE 1 END=1"
            )
        } else {
            String::new()
        };
        self.profile.push(json!({"operator":"closed recursive caller","recursiveInputs":bindings.len(),"logicalOccurrences":occurrences.len(),"booleanContinuations":0,"partialFailure":"ordered entry prefix, no fallback program"}));
        Ok(Some(Output {name:self.relation("closed recursive caller result",format!("SELECT d.k,d.work-min(d.work,d.cost,coalesce(d.breach-1,d.cost)) work,CASE WHEN d.breach IS NOT NULL AND d.breach-1<=d.work THEN -1 WHEN d.work<d.cost THEN -2 ELSE d.truth END truth FROM {limits} d WHERE 1{note}"),true,"native result, remaining work, and reached peak")}))
    }
}
