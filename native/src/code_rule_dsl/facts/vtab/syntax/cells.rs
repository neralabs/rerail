//! Where a batch's projections keep their arrays (decision 0289 §3). With Metal, in the batch's block, which its
//! chunk is nested in and the device reads in place (`rerail_blocks::Cells`). Elsewhere no device reads them, and a
//! block's address space reservation has no counterpart (WASM), so they are heap arrays with the same API.

#[cfg(target_os = "macos")]
pub use rerail_blocks::{Block, Cells};

/// A block of `bytes` for a batch's (or one projection's) arrays: address space, not memory.
#[cfg(target_os = "macos")]
pub fn block(bytes: usize) -> std::sync::Arc<Block> {
    std::sync::Arc::new(Block::anon(rerail_blocks::block::kind::CHUNK, bytes))
}

/// What cells can hold: plain data a block holds as its bytes (`rerail_blocks::Pod`).
#[cfg(target_os = "macos")]
pub trait Cell: rerail_blocks::Pod + Copy + Default {}
#[cfg(target_os = "macos")]
impl<T: rerail_blocks::Pod + Copy + Default> Cell for T {}

#[cfg(not(target_os = "macos"))]
pub use heap::{Block, Cells, block};

/// What cells can hold (heap arrays: any plain value).
#[cfg(not(target_os = "macos"))]
pub trait Cell: Copy + Default {}
#[cfg(not(target_os = "macos"))]
impl<T: Copy + Default> Cell for T {}

#[cfg(not(target_os = "macos"))]
mod heap {
    use std::ops::Deref;
    use std::sync::Arc;

    /// No block: each array is its own allocation.
    pub struct Block;

    pub fn block(_bytes: usize) -> Arc<Block> {
        Arc::new(Block)
    }

    pub struct Cells<T> {
        v: Vec<T>,
        block: Arc<Block>,
    }

    impl<T: Copy + Default> Cells<T> {
        /// As `rerail_blocks::Cells::build`: up to `capacity` `T`s, default, filled by `fill`, which returns how many
        /// it used.
        pub fn build(block: &Arc<Block>, capacity: usize, fill: impl FnOnce(&mut [T]) -> usize) -> Option<Self> {
            let mut v = vec![T::default(); capacity];
            let len = fill(&mut v);
            assert!(len <= capacity, "cells filled past their capacity");
            v.truncate(len);
            v.shrink_to_fit();
            Some(Cells { v, block: Arc::clone(block) })
        }

        pub fn block(&self) -> &Arc<Block> {
            &self.block
        }
    }

    impl<T> Deref for Cells<T> {
        type Target = [T];
        fn deref(&self) -> &[T] {
            &self.v
        }
    }
}
