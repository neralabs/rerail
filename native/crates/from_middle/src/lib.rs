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
//! - [`derive`]: a 1-bit carry's other start, from the base run's words by word operations.
//! - [`join`]: the chunks' leaves checked against their neighbours' from the file's start;
//!   a chunk whose kept leaves leave an output bit unknown is run again.
//! - [`solve`]: a chunk run from an unknown start, in three values a bit; its bytes refute
//!   starts and make the rest known again; what is left is split, only where it matters.
//! - [`starts`]: a start's components, and which of them a chunk reads in place from the bytes
//!   before it rather than solves for.

pub mod chunk;
pub mod derive;
pub mod join;
pub mod solve;
pub mod starts;

#[cfg(test)]
mod tests;
