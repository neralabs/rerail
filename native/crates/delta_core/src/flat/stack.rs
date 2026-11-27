//! A stack whose top `N` items are in a fixed array on the machine stack (the stack bucket) and whose items below
//! them spill to a [`Grow`] in an arena: the preprocessor's frames (`lp_fpush`, `lp_fsync`). One owner.

use super::arena::{Arena, Full};
use super::grow::Grow;
use crate::Pod;
use core::mem::MaybeUninit;

/// A stack of `T`, its top `N` (a power of two) inline.
pub struct Stack<'s, 'r, T: Pod + Copy, const N: usize> {
    ring: [MaybeUninit<T>; N],
    len: usize,
    spill: &'s mut Grow<T>,
    arena: &'s Arena<'r>,
}

impl<'s, 'r, T: Pod + Copy, const N: usize> Stack<'s, 'r, T, N> {
    const POW2: () = assert!(N.is_power_of_two(), "a stack's inline slots: a power of two");

    /// An empty stack, spilling to `spill` (emptied) in `arena`, the arena it was made in.
    pub fn new(spill: &'s mut Grow<T>, arena: &'s Arena<'r>) -> Self {
        let () = Self::POW2;
        spill.clear();
        Stack { ring: [MaybeUninit::uninit(); N], len: 0, spill, arena }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Items below the inline ones, in the spill.
    pub fn spilled(&self) -> usize {
        self.len.saturating_sub(N)
    }

    /// Pushes `v`; with the inline slots full, the lowest of them goes to the spill first.
    pub fn push(&mut self, v: T) -> Result<(), Full> {
        if self.len >= N {
            let low = self.len - N;
            // SAFETY: slot `low` holds item `low`, the lowest inline one.
            let item = unsafe { self.ring[low & (N - 1)].assume_init() };
            self.spill.push(self.arena, item)?;
        }
        self.ring[self.len & (N - 1)].write(v);
        self.len += 1;
        Ok(())
    }

    /// Its top item, taken off; the highest spilled item comes back to the inline slot it leaves.
    pub fn pop(&mut self) -> Option<T> {
        self.len = self.len.checked_sub(1)?;
        // SAFETY: item `len` is inline (the top `N` always are).
        let v = unsafe { self.ring[self.len & (N - 1)].assume_init() };
        if self.len >= N {
            let back = self.spill.pop().expect("a spilled item below the inline ones");
            // Item `len - N` comes back to slot `(len - N) & (N - 1)`, the one just left.
            self.ring[(self.len - N) & (N - 1)].write(back);
        }
        Some(v)
    }

    /// Its top item.
    pub fn top(&self) -> Option<&T> {
        let i = self.len.checked_sub(1)?;
        // SAFETY: inline, written.
        Some(unsafe { self.ring[i & (N - 1)].assume_init_ref() })
    }
}
