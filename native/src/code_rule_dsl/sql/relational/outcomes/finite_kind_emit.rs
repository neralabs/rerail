//! Reached metadata answer dispatch; no precomputation of effectful bodies.
use super::super::{finite_kind_summary::Summaries, finite_lower::Program};
use super::{Demand, Sql, utility_projection_adapter::Adapter};
use serde_json::json;
impl Sql {
    pub(super) fn extend_kind_projections(
        &mut self,
        p: &Program,
        input: &Demand,
        summaries: &Summaries,
        adapter: &mut Adapter,
    ) -> Result<(), String> {
        if !input.slots.is_empty()
            || !self.generation.following.is_empty()
            || self.generation.memo_bypass
        {
            return Ok(());
        }
        let Some(clause) = self.generation.memo_clause else {
            return Ok(());
        };
        let strata = summaries
            .strata
            .iter()
            .filter(|s| adapter.entries[s.function].is_none())
            .collect::<Vec<_>>();
        if strata.is_empty() {
            return Ok(());
        }
        let mut functions = std::collections::BTreeSet::new();
        let mut answers = vec![];
        let mut completions = vec![];
        let mut guards = vec![];
        for (id, s) in strata.iter().enumerate() {
            functions.insert(s.function);
            let pc = p.code.len() + 10 + p.functions.len() + s.function;
            let a = &s.answer;
            answers.push(format!(
                "({id},{pc},{},{},{})",
                usize::from(a.truth),
                a.cost,
                a.peak
            ));
            let accepted = s.kinds.iter().flatten().copied().collect::<Vec<_>>();
            let guard = if s.kinds.contains(&None) {
                let excluded = summaries
                    .palette
                    .iter()
                    .filter(|k| !accepted.contains(k))
                    .map(u16::to_string)
                    .collect::<Vec<_>>();
                if excluded.is_empty() {
                    "1".into()
                } else {
                    format!("n.kind NOT IN ({})", excluded.join(","))
                }
            } else {
                format!(
                    "n.kind IN ({})",
                    accepted
                        .iter()
                        .map(u16::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                )
            };
            guards.push(format!("(a.id={id} AND ({guard}))"));
            for (ordinal, c) in a.completions.iter().enumerate() {
                let identity = self
                    .generation
                    .memo_layout
                    .as_ref()
                    .and_then(|l| l.contracts.get(&c.utility))
                    .map_or(c.utility, |c| c.identity);
                completions.push(format!(
                    "({id},{ordinal},{identity},{},{},{})",
                    usize::from(c.truth),
                    c.cost,
                    c.peak
                ));
            }
        }
        let entries = functions
            .iter()
            .map(|f| {
                format!(
                    "({},{})",
                    p.code.len() + 10 + p.functions.len() + f,
                    p.functions[*f].entry + 10
                )
            })
            .collect::<Vec<_>>();
        let entry_table = self.constant_relation(
            "metadata utility entries",
            format!(
                "SELECT column1 pc,column2 fallback FROM (VALUES {})",
                entries.join(",")
            ),
        );
        let answer_table=self.constant_relation("metadata utility answers",format!("SELECT column1 id,column2 pc,column3 truth,column4 cost,column5 peak FROM (VALUES {})",answers.join(",")));
        let completion_table=self.constant_relation("metadata utility completions",if completions.is_empty(){"SELECT 0 answer,0 ordinal,0 utility,0 truth,0 cost,0 peak WHERE 0".into()}else{format!("SELECT column1 answer,column2 ordinal,column3 utility,column4 truth,column5 cost,column6 peak FROM (VALUES {}) ORDER BY column1,column2",completions.join(","))});
        let guard = format!("machine.pc IN (SELECT pc FROM {entry_table})");
        let kinds = guards.join(" OR ");
        let phase = format!(
            r#"WITH reached AS MATERIALIZED (SELECT machine.node,machine.pc,machine.level,machine.work,machine.handle,machine.truth,machine.env,machine.scope WHERE {guard}),
s AS MATERIALIZED (SELECT r.*,e.fallback,a.id answer,a.truth projected_truth,a.cost projected_cost,a.peak projected_peak,coalesce(r.work>=a.cost AND r.level+a.peak<=256,0) fit FROM reached r JOIN {entry_table} e ON e.pc=r.pc JOIN node n ON n.file=d.file AND n.id=r.node LEFT JOIN {answer_table} a ON a.pc=r.pc AND ({kinds})),
published AS MATERIALIZED (SELECT s.*,(SELECT count(m.truth) FROM {completion_table} c LEFT JOIN relation_memo m ON m.mode IS CASE WHEN s.fit THEN 1 END AND m.clause IS {clause} AND m.utility IS c.utility AND m.file IS d.file AND m.node IS s.node AND m.vtruth IS c.truth AND m.vcost IS c.cost AND m.vpeak IS c.peak WHERE s.fit AND c.answer=s.answer) publications FROM s),
noted AS MATERIALIZED (SELECT s.*,CASE WHEN s.fit THEN relation_peak_note(s.level+s.projected_peak) ELSE 1 END ready FROM published s WHERE s.publications>=0)
SELECT s.node,CASE WHEN s.fit THEN 0 ELSE s.fallback END,s.level,CASE WHEN s.fit THEN s.work-s.projected_cost ELSE s.work END,s.handle,CASE WHEN s.fit THEN s.projected_truth ELSE s.truth END,s.env,s.scope FROM noted s WHERE s.ready=1"#
        );
        self.generation.operator_as("metadata utility segment")?;
        for f in &functions {
            adapter.entries[*f] = Some(p.code.len() + 10 + p.functions.len() + f);
        }
        adapter.phases.push((phase, guard));
        self.profile.push(json!({"operator":"metadata utility segment","functions":functions.len(),"strata":strata.len(),"completionRows":completions.len(),"guard":"complete metadata proof, whole work cost and relative peak fit","fallback":"original reached function body","speculativeCallbacks":false,"booleanContinuations":0}));
        Ok(())
    }
}
