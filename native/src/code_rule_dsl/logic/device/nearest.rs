//! Nearest ancestors on the device (`SolverDevice::nearest`).

/// The solved targets per cell (set-major), the most steps any cell took (`rounds`), the dispatches
/// (`launches`, one), the command buffer's GPU seconds, the
/// host's wall seconds from the first buffer made to the last completion, and where those went
/// (`profile::PHASES`), with the buffer bytes made and reused.
pub(in crate::code_rule_dsl) struct Solved {
    pub(in crate::code_rule_dsl) targets: Vec<u32>,
    pub(in crate::code_rule_dsl) rounds: u32,
    pub(in crate::code_rule_dsl) launches: usize,
    pub(in crate::code_rule_dsl) gpu: f64,
    pub(in crate::code_rule_dsl) wall: f64,
    pub(in crate::code_rule_dsl) phases: [f64; 7],
    pub(in crate::code_rule_dsl) made: u64,
    pub(in crate::code_rule_dsl) reused: u64,
}

/// How a solve gets its buffers and queue: made for the call and dropped after (`Fresh`, kept
/// to measure against), or leased from the pre-pinned blocks with the kept queue (`Blocks`,
/// decision 0039).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::code_rule_dsl) enum Buffers {
    Fresh,
    Blocks,
}
