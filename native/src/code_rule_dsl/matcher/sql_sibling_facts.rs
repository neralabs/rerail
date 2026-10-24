//! SQL supplies the measured cost of a completed or interrupted sibling cache fill.
pub(in crate::code_rule_dsl) fn note_sql_sibling_fill(steps: usize) {
    super::filled(steps);
}
