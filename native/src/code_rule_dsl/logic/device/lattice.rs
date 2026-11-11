//! A flat-lattice system and its solution on the device (`SolverDevice::lattice`).

/// A flat-lattice system: per cell its seed (`BOT` where none), its copy sources, and transfers
/// `(cell, op, left, right)` (`op` an ASCII byte: `+`, `-`, `*`, else TOP).
pub(in crate::code_rule_dsl) struct System<'a> {
    pub(in crate::code_rule_dsl) seeds: &'a [i64],
    /// (cell, source).
    pub(in crate::code_rule_dsl) copies: &'a [(u32, u32)],
    pub(in crate::code_rule_dsl) bins: &'a [(u32, u8, u32, u32)],
}

/// The solved values, the launches the queue took, the cells evaluated, and the evaluations
/// pushed again because another worker was publishing their cell.
pub(in crate::code_rule_dsl) struct Solved {
    pub(in crate::code_rule_dsl) values: Vec<i64>,
    pub(in crate::code_rule_dsl) launches: usize,
    pub(in crate::code_rule_dsl) visits: u64,
    pub(in crate::code_rule_dsl) retries: u64,
    /// The command buffers' GPU seconds, the host's wall seconds from its first buffer to the last
    /// completion, where they went (`profile::PHASES`), and the buffer bytes made and reused.
    pub(in crate::code_rule_dsl) gpu: f64,
    pub(in crate::code_rule_dsl) wall: f64,
    pub(in crate::code_rule_dsl) phases: [f64; 7],
    pub(in crate::code_rule_dsl) made: u64,
    pub(in crate::code_rule_dsl) reused: u64,
}
