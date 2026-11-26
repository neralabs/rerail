//! Tier 1 from the middle of a file, on the CPU (the design page "Lexing From the Middle").
//!
//! A file is cut into chunks of blocks. Each chunk is solved on its own, for every start the
//! grammar allows; the chunks are then joined by checking each one's start against the end of
//! the chunk before. This crate builds that algorithm on the CPU, in the phases a device will
//! run, and checks every result against the block program's serial interpreter
//! (`BlockProgram::all_values`).
//!
//! - [`chunk`]: the block program over a range of blocks from an explicit start (its carries),
//!   giving its values and its end. The foundation: a chunk run from the serial run's state at
//!   its edge is the serial run there.

pub mod chunk;

#[cfg(test)]
mod tests;
