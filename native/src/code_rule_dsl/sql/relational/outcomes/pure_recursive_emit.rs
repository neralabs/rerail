//! Direct pure summaries and structural projections. Generic instruction execution is refused.
use super::super::lower::pure_recursive_lower::Program;
use super::{Demand, Output, Sql};

impl Sql {
    pub(super) fn pure_recursive(
        &mut self,
        program: &Program,
        input: &Demand,
    ) -> Result<Output, String> {
        let strata = super::super::pure_kind_summary::compile(program);
        if program.projection.is_none()
            && !strata
                .iter()
                .any(|(_, a)| a.as_ref().is_some_and(|a| a.cost > 1))
        {
            return self.pure_recursive_machine(program, input);
        }
        self.generation.operator_as("metadata pure summary")?;
        let depth = self.generation.depth_expression();
        let expressions = strata
            .iter()
            .map(|(k, a)| {
                (
                    *k,
                    a.as_ref()
                        .map(|a| super::super::pure_kind_summary::fields(a, &depth, "d.work")),
                )
            })
            .collect::<Vec<_>>();
        let field = |which: usize| {
            let value =
                |a: &Option<[String; 3]>| a.as_ref().map_or("NULL".into(), |a| a[which].clone());
            let cases = expressions
                .iter()
                .filter_map(|(k, a)| k.map(|k| format!("WHEN {k} THEN {}", value(a))))
                .collect::<Vec<_>>()
                .join(" ");
            let fallback = value(&expressions.last().unwrap().1);
            if cases.is_empty() {
                fallback
            } else {
                format!("CASE n.kind {cases} ELSE {fallback} END")
            }
        };
        let root_cases = strata
            .iter()
            .filter_map(|(k, a)| k.map(|k| format!("WHEN {k} THEN {}", usize::from(a.is_none()))))
            .collect::<Vec<_>>()
            .join(" ");
        let root_filter = if root_cases.is_empty() {
            usize::from(strata.last().unwrap().1.is_none()).to_string()
        } else {
            format!(
                "CASE n.kind {root_cases} ELSE {} END",
                usize::from(strata.last().unwrap().1.is_none())
            )
        };
        let classified=self.relation("metadata pure summary",format!("SELECT d.*,{} known_truth,{} known_cost,{} known_peak FROM {} d JOIN node n ON n.file=d.file AND n.id=d.node",field(0),field(1),field(2),input.name),true,"exact metadata entry prefixes, including budget and depth failure");
        let slow = Demand {
            name: self.relation(
                "unresolved pure demands",
                format!("SELECT * FROM {classified} WHERE known_truth IS NULL"),
                true,
                "metadata-unresolved input",
            ),
            slots: input.slots.clone(),
        };
        let original = if let Some(projection) = &program.projection
            && let Some(out) = self.projected_tail(projection, &slow, &root_filter)?
        {
            out
        } else {
            self.pure_recursive_machine(program, &slow)?
        };
        let note = if self.generation.peak_tracking {
            format!(
                " AND CASE WHEN known_peak>0 THEN relation_peak_note(({depth})+known_peak) ELSE 1 END=1"
            )
        } else {
            String::new()
        };
        Ok(Output{name:self.relation("completed metadata and structural outcomes",format!("SELECT k,work-known_cost work,known_truth truth FROM {classified} WHERE known_truth IS NOT NULL{note} UNION ALL SELECT k,work,truth FROM {}",original.name),true,"exact native entry prefixes")})
    }
    fn pure_recursive_machine(&mut self, _: &Program, _: &Demand) -> Result<Output, String> {
        Err(
            "SQL pure interpreter removed: use structural closure or a batched matcher fragment"
                .into(),
        )
    }
}
