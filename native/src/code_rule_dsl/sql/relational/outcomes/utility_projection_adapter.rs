//! Complete pure-function fast path. Original instructions remain the failure
//! path; fitting completed summaries never stand in for partial native traces.
use super::super::{finite_lower::Program, utility_projection_lower, utility_projection_producer};
use super::{Demand, Sql};
use serde_json::json;

pub(super) struct Adapter {
    pub entries: Vec<Option<usize>>,
    pub(super) phases: Vec<(String, String)>,
}
impl Adapter {
    pub fn phases(self) -> Vec<(String, String)> {
        self.phases
    }
}
impl Sql {
    pub(super) fn pure_function_projections(
        &mut self,
        p: &Program,
        input: &Demand,
    ) -> Result<Adapter, String> {
        let mut adapter = Adapter {
            entries: vec![None; p.functions.len()],
            phases: vec![],
        };
        if !input.slots.is_empty()
            || !p.mentions.is_empty()
            || !p.writes.is_empty()
            || p.code.iter().any(|n| {
                !n.mentions.is_empty() || !n.writes.is_empty() || !n.node_writes.is_empty()
            })
            || p.functions
                .iter()
                .any(|f| !f.mentions.is_empty() || !f.writes.is_empty())
            || !self.generation.following.is_empty()
            || self.generation.memo_bypass
        {
            return Ok(adapter);
        }
        let Some(clause) = self.generation.memo_clause else {
            return Ok(adapter);
        };
        let roots = self.generation.components.borrow().roots.clone();
        if roots.is_empty() {
            return Ok(adapter);
        }
        let eligible = p
            .functions
            .iter()
            .enumerate()
            .filter_map(|(index, f)| {
                let body = f.original.as_ref()?;
                let candidate = utility_projection_lower::lower_plan(body)?;
                (candidate.has_calls()
                    || candidate
                        .expressions
                        .iter()
                        .any(|e| matches!(e, utility_projection_lower::Expr::Search { .. })))
                .then_some((index, body))
            })
            .collect::<Vec<_>>();
        let Some((program, entries)) = utility_projection_lower::lower_forest(
            &eligible.iter().map(|(_, body)| *body).collect::<Vec<_>>(),
        ) else {
            return Ok(adapter);
        };
        let prefix = self.projection_namespace();
        let entry_rows = (0..entries.len())
            .map(|e| format!("({e})"))
            .collect::<Vec<_>>()
            .join(",");
        let mut produced = utility_projection_producer::compile_forest(
            &program,
            &entries,
            &format!(
                "SELECT e.column1 entry,n.file,n.id node,0 incoming FROM (VALUES {entry_rows}) e CROSS JOIN node n WHERE EXISTS(SELECT 1 FROM {roots} r WHERE r.file=n.file)"
            ),
            &prefix,
        )?;
        let completed_calls = produced.completion_select().to_owned();
        let staged = produced.summaries.iter().any(|s| s.cut);
        let mut initial = vec![];
        if staged {
            let Some(root) = self.generation.components.borrow().stage_root.clone() else {
                // This caller does not own physical storage; retain its reached body.
                return Ok(adapter);
            };
            let table = format!("{prefix}_materialized_demand");
            let create = format!(
                "CREATE TEMP TABLE IF NOT EXISTS {table}(owner INTEGER NOT NULL,file INTEGER NOT NULL,node INTEGER NOT NULL,incoming INTEGER NOT NULL,PRIMARY KEY(owner,file,node,incoming)) WITHOUT ROWID"
            );
            let insert = format!(
                "INSERT INTO {table} WITH RECURSIVE {root},{} SELECT owner,file,node,incoming FROM {}",
                produced.seed_definitions.join(","),
                produced.demand
            );
            if insert.len() > 256000 {
                return Err("SQL text exceeds256000 bytes in projection demand population".into());
            }
            self.generation
                .components
                .borrow_mut()
                .physical_stages
                .extend([create, format!("DELETE FROM {table}"), insert]);
            let seeds = produced.seed_definitions.len();
            produced.definitions.drain(..seeds);
            for summary in &mut produced.summaries {
                summary.index -= seeds;
                for reduction in &mut summary.reductions {
                    *reduction -= seeds;
                }
            }
            initial.push(format!("{}(entry,file,node,incoming) AS NOT MATERIALIZED (SELECT owner,file,node,incoming FROM {table} WHERE owner<{})",produced.roots,entries.len()));
            initial.push(format!("{}(owner,file,node,incoming) AS NOT MATERIALIZED (SELECT owner,file,node,incoming FROM {table})",produced.demand));
        }
        let searches = program
            .expressions
            .iter()
            .filter(|e| matches!(e, utility_projection_lower::Expr::Search { .. }))
            .count();
        for _ in 0..entries.len() + program.functions.len() + 2 * searches {
            self.generation.operator_as("pure projection boundary")?;
        }
        let summary = format!("{prefix}_complete");
        let completions = format!("{prefix}_completed_calls");
        let mut definitions = self.cut_projection_definitions(
            initial,
            produced.definitions,
            &produced.summaries,
            staged,
        )?;
        definitions.push(format!(
            "{summary} AS MATERIALIZED ({})",
            produced.summary_select
        ));
        definitions.push(format!(
            "{completions} AS MATERIALIZED ({})",
            completed_calls
        ));
        self.register_projection_definitions(definitions);
        for (entry, (function, _)) in eligible.iter().enumerate() {
            let function = *function;
            let f = &p.functions[function];
            self.generation.operator_as("pure projection adapter")?;
            let pc = p.code.len() + 10 + function;
            let fallback = f.entry + 10;
            let identities = program
                .functions
                .iter()
                .map(|f| {
                    let identity = self
                        .generation
                        .memo_layout
                        .as_ref()
                        .and_then(|l| l.contracts.get(&f.utility))
                        .map_or(f.utility, |c| c.identity);
                    format!("WHEN {} THEN {identity}", f.utility)
                })
                .collect::<Vec<_>>()
                .join(" ");
            let identity = if identities.is_empty() {
                "c.utility".into()
            } else {
                format!("CASE c.utility {identities} ELSE c.utility END")
            };
            let phase = format!(
                r#"WITH reached AS MATERIALIZED (SELECT machine.node,machine.pc,machine.level,machine.work,machine.handle,machine.truth,machine.env,machine.scope WHERE machine.pc={pc}),
s AS MATERIALIZED (SELECT r.*,z.truth projected_truth,z.cost projected_cost,z.peak projected_peak,coalesce(z.truth IN (0,1) AND r.work>=z.cost AND r.level+z.peak<=256,0) fit FROM reached r LEFT JOIN {summary} z ON z.entry={entry} AND z.file=d.file AND z.node=r.node AND z.incoming=0),
published AS MATERIALIZED (SELECT s.*,(SELECT count(m.truth) FROM {completions} c LEFT JOIN relation_memo m ON m.mode IS CASE WHEN s.fit THEN 1 END AND m.clause IS {clause} AND m.utility IS ({identity}) AND m.file IS d.file AND m.node IS c.node AND m.vtruth IS c.truth AND m.vcost IS c.cost AND m.vpeak IS c.peak WHERE s.fit AND c.root_entry={entry} AND c.root_file=d.file AND c.root_node=s.node AND c.root_incoming=0) publications FROM s),
noted AS MATERIALIZED (SELECT s.*,CASE WHEN s.fit THEN relation_peak_note(s.level+s.projected_peak) ELSE 1 END ready FROM published s WHERE s.publications>=0)
SELECT s.node,CASE WHEN s.fit THEN 0 ELSE {fallback} END,s.level,CASE WHEN s.fit THEN s.work-s.projected_cost ELSE s.work END,s.handle,CASE WHEN s.fit THEN s.projected_truth ELSE s.truth END,s.env,s.scope FROM noted s WHERE s.ready=1"#
            );
            adapter.entries[function] = Some(pc);
            adapter.phases.push((phase, format!("machine.pc={pc}")));
            self.profile.push(json!({"operator":"complete pure utility projection","function":function,"entry":pc,"fallback":fallback,"summary":summary,"completionRecipe":completions,"demand":"publication files, node keys, incoming0","guard":"complete truth, whole work cost and relative peak fit","emptyCaptureScope":true,"nativeComposition":false,"booleanContinuations":0,"sharedEntries":entries.len(),"sourceOperators":program.source_operators,"sourceRetained":program.original.is_some()}));
        }
        Ok(adapter)
    }
}
