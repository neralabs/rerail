//! Cost inputs and comparisons shared by execution backends.
#[derive(Clone, Debug, serde::Serialize)]
pub(in crate::code_rule_dsl) struct InputChange {
    pub rows: usize,
    pub inserted: usize,
    pub deleted: usize,
    pub distinct_keys: Option<usize>,
    pub probe_rows: usize,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(in crate::code_rule_dsl) struct ProjectionCost {
    pub input_rows: usize,
    pub changed_rows: usize,
    pub estimated_probe_rows: usize,
    pub sparse: bool,
}

impl ProjectionCost {
    pub fn estimate(inputs: &[InputChange]) -> Self {
        let input_rows = inputs
            .iter()
            .map(|i| i.rows)
            .fold(0usize, usize::saturating_add);
        let changed_rows = inputs
            .iter()
            .map(|i| i.inserted.saturating_add(i.deleted))
            .fold(0usize, usize::saturating_add);
        let estimated_probe_rows = inputs
            .iter()
            .enumerate()
            .map(|(at, input)| {
                inputs.iter().enumerate().filter(|(i, _)| *i != at).fold(
                    input.inserted.saturating_add(input.deleted),
                    |rows, (_, other)| rows.saturating_mul(other.probe_rows.max(1)),
                )
            })
            .fold(0usize, usize::saturating_add);
        Self {
            input_rows,
            changed_rows,
            estimated_probe_rows,
            sparse: changed_rows > 0 && estimated_probe_rows.saturating_mul(4) < input_rows,
        }
    }
}
