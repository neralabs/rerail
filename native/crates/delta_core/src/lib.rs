//! The floor under the delta executor (decision 0686): the block formats every reader and writer of a trace shares,
//! read and written where they lie. `no_std`, and no `alloc`: nothing here can allocate, so nothing built on it can hold
//! a row on the heap. A format moves here from `rerail_blocks` (or `rerail_vector`) when the executor's code that
//! reads or writes it moves onto this crate; the old paths re-export it, so it stays one definition.
//!
//! - `pod`: `Pod`, a type whose bytes are its value, and the typed word reads over a block's bytes.
//! - `block`: an operation block's reference table and where its body starts (`BODY_AT`).
//! - `runs`: a trace's shape as its blocks hold it (`ShapeW`) and the δ runs block, the rows a layer is made from.

#![no_std]

pub mod block;
pub mod pod;
pub mod runs;

pub use pod::{Pod, set_word, word, words, words_mut};

#[cfg(test)]
mod tests;
