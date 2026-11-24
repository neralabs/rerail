//! Where a batch's projections keep their arrays (decision 0289 §3): in the batch's block, which its chunk is nested
//! in and the device reads in place (`rerail_blocks::Cells`), on every native target (decision 0440 step E). On WASM
//! a block's address space reservation has no counterpart and no device reads them, so they are heap arrays with the
//! same API.

#[cfg(not(target_family = "wasm"))]
pub use rerail_blocks::{Block, Cells};

/// A block of `bytes` for a batch's (or one projection's) arrays: address space, not memory.
#[cfg(not(target_family = "wasm"))]
pub fn block(bytes: usize) -> std::sync::Arc<Block> {
    // From the one block policy (decision 0342): host memory (a reservation of a GB and more, never a pool
    // slab), which the device reads in place through a no-copy buffer.
    std::sync::Arc::new(rerail_blocks::alloc::host(rerail_blocks::block::kind::CHUNK, bytes))
}

/// What cells can hold: plain data a block holds as its bytes (`rerail_blocks::Pod`).
#[cfg(not(target_family = "wasm"))]
pub trait Cell: rerail_blocks::Pod + Copy + Default {}
#[cfg(not(target_family = "wasm"))]
impl<T: rerail_blocks::Pod + Copy + Default> Cell for T {}

#[cfg(target_family = "wasm")]
pub use heap::{Block, Cells, block};

/// What cells can hold (heap arrays: any plain value).
#[cfg(target_family = "wasm")]
pub trait Cell: Copy + Default {}
#[cfg(target_family = "wasm")]
impl<T: Copy + Default> Cell for T {}

#[cfg(target_family = "wasm")]
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

/// One `T` that readers find by its address, in `block` beside the arrays it describes (a descriptor whose runs point
/// at them, `rerail_clif::ids::Numbering`). Written while it is built, read only after.
pub struct Desc<T> {
    #[cfg(not(target_family = "wasm"))]
    p: *mut T,
    #[cfg(not(target_family = "wasm"))]
    _block: std::sync::Arc<Block>,
    #[cfg(target_family = "wasm")]
    b: Box<T>,
}

// SAFETY: written by its builder alone, before it is shared; read only after.
unsafe impl<T: Sync> Send for Desc<T> {}
unsafe impl<T: Sync> Sync for Desc<T> {}

impl<T: Default> Desc<T> {
    /// A default `T` in `block`.
    pub fn new(block: &std::sync::Arc<Block>) -> Self {
        #[cfg(not(target_family = "wasm"))]
        {
            let p: *mut T = block.try_alloc_t(1).expect("a batch's block holds its projection");
            // SAFETY: just allocated for this alone.
            unsafe { p.write(T::default()) };
            Desc { p, _block: std::sync::Arc::clone(block) }
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = block;
            Desc { b: Box::default() }
        }
    }
}

impl<T> Desc<T> {
    /// Its address: for the builder to write through (before it is shared) and the reads to read.
    pub fn as_ptr(&self) -> *mut T {
        #[cfg(not(target_family = "wasm"))]
        {
            self.p
        }
        #[cfg(target_family = "wasm")]
        {
            std::ptr::from_ref::<T>(&self.b).cast_mut()
        }
    }
}

impl<T> std::ops::Deref for Desc<T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: built by `new`, and written only before it is shared.
        unsafe { &*self.as_ptr() }
    }
}
