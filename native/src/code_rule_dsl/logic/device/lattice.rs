//! A flat-lattice system and its solution on the device (`SolverDevice::lattice`).

/// A flat-lattice system: per cell its seed (`BOT` where none), its copy sources, and transfers
/// `(cell, op, left, right)` (`op` an ASCII byte: `+`, `-`, `*`, else TOP).
pub struct System<'a> {
    pub seeds: &'a [i64],
    /// (cell, source).
    pub copies: &'a [(u32, u32)],
    pub bins: &'a [(u32, u8, u32, u32)],
}

/// The solved values, the launches the queue took, the cells evaluated, and the evaluations
/// pushed again because another worker was publishing their cell.
pub struct Solved {
    pub values: Vec<i64>,
    pub launches: usize,
    pub visits: u64,
    pub retries: u64,
    /// The command buffers' GPU seconds, the host's wall seconds from its first buffer to the last
    /// completion, where they went (`profile::PHASES`), and the buffer bytes made and reused.
    pub gpu: f64,
    pub wall: f64,
    pub phases: [f64; 7],
    pub made: u64,
    pub reused: u64,
}
