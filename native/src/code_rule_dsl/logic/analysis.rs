//! Logical proofs shared by planners and executors. No connection, SQL, or row statistics.
pub(in crate::code_rule_dsl) mod keys;
pub(in crate::code_rule_dsl) mod storage;
pub(in crate::code_rule_dsl) mod bags;
mod program;
pub(in crate::code_rule_dsl) mod scalars;
pub(in crate::code_rule_dsl) use program::ProgramAnalysis;

#[derive(Clone, Debug, serde::Serialize)]
pub(in crate::code_rule_dsl) struct Decision {
    pub proven: bool,
    pub reason: String,
}
impl Decision {
    pub fn new(proven: bool, reason: impl Into<String>) -> Self {
        Self {
            proven,
            reason: reason.into(),
        }
    }
}
#[cfg(test)]
mod tests;
