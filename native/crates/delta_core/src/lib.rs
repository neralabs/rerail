//! The floor under the delta executor (decision 0686): the block formats every reader and writer of a trace shares,
//! read and written where they lie. `no_std`, and no `alloc`: nothing here can allocate, so nothing built on it can hold
//! a row on the heap. A format moves here from `rerail_blocks` (or `rerail_vector`) when the executor's code that
//! reads or writes it moves onto this crate; the old paths re-export it, so it stays one definition.
//!
//! - `pod`: `Pod`, a type whose bytes are its value, and the typed word reads over a block's bytes.
//! - `block`: an operation block's reference table and where its body starts (`BODY_AT`).
//! - `runs`: a trace's shape as its blocks hold it (`ShapeW`) and the δ runs block, the rows a layer is made from.
//! - `refs`: references inside a block, relative to their own address (`Ptr`, `Span`).
//! - `kernel`: how a structure in a block is declared (`structure!`) and its UKL text (`KernelField`), written with no
//!   allocator.
//! - `hash`: the hash every index and kernel uses for a key.
//! - `col`: a frozen integer column's format (`IntCol`) and its reads (`Dec`).
//! - `rows`: a relation's rows, its columns in chunks that grow by quarter steps (`Rows`, `ColDir`), and their reads.
//! - `relation`: a relation's descriptor (`Relation`), its indexes and orders, and its reads.
//! - `bloom`: a blocked Bloom filter's format, its hash and its reads.

#![no_std]

pub mod block;
pub mod bloom;
pub mod col;
pub mod hash;
pub mod kernel;
pub mod pod;
pub mod refs;
pub mod relation;
pub mod rows;
pub mod runs;

pub use pod::{Pod, set_word, word, words, words_mut};

#[cfg(test)]
mod tests;
