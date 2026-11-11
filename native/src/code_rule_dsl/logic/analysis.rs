//! Logical proofs shared by planners and executors. No connection, SQL, or row statistics.
pub mod keys;
pub mod storage;
pub mod bags;
pub mod program;
pub mod scalars;
pub use program::ProgramAnalysis;

#[derive(Clone, Debug, serde::Serialize)]
pub struct Decision {
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
pub mod tests;
