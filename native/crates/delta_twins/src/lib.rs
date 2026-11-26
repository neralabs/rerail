//! The Rust twins of the delta executor's operations (decision 0686; *0686: Operations* §3).
//!
//! An operation is a function of its blocks and its params: what it writes depends on them and on nothing else. Its
//! twin is that function in plain Rust, registered as the operation's reference version, against which every other
//! version (UKL, compiled) is checked.
//!
//! **The rules.** A twin is a list of phases (`Twin`), as its operation's shape is (Operations §4). Each phase says
//! how many items it has (`items`) and runs a tile of them (`run`): the items `lo..hi`, in any order of tiles and at
//! any tiling the executor picks, so a phase's items must be independent, as the shape says. A phase may run again
//! (`again`: rounds to a fixpoint, a queue's turns, a merge's passes). Between phases is a sync point: what one phase
//! leaves for the next lies in the operation's blocks (its scratch and control slots), never in Rust state, as it
//! would on a GPU.
//!
//! A twin reads only its slots, the blocks its descriptor table reaches (`Table`: its slots and what they refer to, by
//! block id) and its params, and writes only its write slots. Rust data structures inside one tile's call are allowed
//! for now (the user, 2026-11-25: "using Rust data structures internally is going to be slow but not wrong"). A twin
//! writes a block's structures with blocks' own code, in place, through its slot's pin: "the twins depend on blocks,
//! and a slot is the pinned block, so a twin uses blocks' layouts and freeze in place" (the rebuild plan, B1): a
//! relation made, its rows claimed and written (`Relation::put_own`), frozen (`relation::freeze_layer`), its orders
//! and sketches added; the reads every reader shares (`relation::View`). Nothing may outlive a call or come from outside it: no
//! `static`, thread-local, `Once`/`Lazy` cell, lock or atomic of the twin's own; no thread's scratch (a call makes its
//! own, `Scratch::anon`); no file, environment, process, network or time. `unsafe` is forbidden.
//! `check_delta_exec_shape.py` refuses each of these here, and this crate's `clippy.toml` disallows them by path.
//!
//! A params struct is declared once, here, by `params!`; the executor gives UKL its declaration from `Params::FIELDS`.

#![forbid(unsafe_code)]
// The twins' rules (`clippy.toml`) refuse, not warn.
#![deny(clippy::disallowed_methods, clippy::disallowed_types, clippy::disallowed_macros)]

use rerail_blocks::gens::Table;
use rerail_blocks::region::BlockPin;

#[cfg(any(test, feature = "test-hooks"))]
pub mod samples;
pub mod sort;
pub mod trace;

/// One slot's block as a twin sees it: pinned by the executor, to read or to write (`BlockPin`). A twin reads a
/// block's structures through `pin.block()` and its bytes through `pin.bytes()`, and writes only a `Write` slot's.
pub enum Slot<'a> {
    Read(&'a BlockPin),
    Write(&'a mut BlockPin),
}

impl Slot<'_> {
    pub fn pin(&self) -> &BlockPin {
        match self {
            Slot::Read(p) => p,
            Slot::Write(p) => p,
        }
    }

    pub fn bytes(&self) -> &[u8] {
        self.pin().bytes()
    }

    /// The bytes to write: `None` for a slot pinned to read.
    pub fn bytes_mut(&mut self) -> Option<&mut [u8]> {
        match self {
            Slot::Read(_) => None,
            Slot::Write(p) => p.bytes_mut(),
        }
    }
}

/// What a twin's phase reads and writes: its slots in its signature's order, the launch's descriptor table (the blocks
/// its slots refer to, by id) and its params' bytes (`Params::read`).
pub struct Ctx<'c, 'p, 't> {
    pub slots: &'c mut [Slot<'p>],
    pub table: &'c Table<'t>,
    pub params: &'c [u8],
}

impl<'p> Ctx<'_, 'p, '_> {
    /// The bytes of slots `reads`, to read, and of slots `writes`, to write, at once (each slot once).
    pub fn views<const R: usize, const W: usize>(&mut self, reads: [usize; R], writes: [usize; W]) -> ([&[u8]; R], [&mut [u8]; W]) {
        let mut r: [Option<&[u8]>; R] = [None; R];
        let mut w: [Option<&mut [u8]>; W] = std::array::from_fn(|_| None);
        for (i, s) in self.slots.iter_mut().enumerate() {
            if let Some(k) = writes.iter().position(|&x| x == i) {
                w[k] = Some(s.bytes_mut().unwrap_or_else(|| panic!("slot {i} is written, and pinned to read")));
            } else if let Some(k) = reads.iter().position(|&x| x == i) {
                r[k] = Some(s.bytes());
            }
        }
        (r.map(|x| x.expect("a slot read")), w.map(|x| x.expect("a slot written")))
    }

    /// Slot `i`'s bytes, to read.
    pub fn read(&self, i: usize) -> &[u8] {
        self.slots[i].bytes()
    }

    /// Slot `read`'s bytes, to read, and slot `write`'s pin, to write a block's structures through (a relation's
    /// rows, `Relation::put_own`; its freeze), at once.
    pub fn read_and_pin(&mut self, read: usize, write: usize) -> (&[u8], &mut BlockPin) {
        assert_ne!(read, write, "two slots");
        let (mut r, mut w) = (None, None);
        for (i, s) in self.slots.iter_mut().enumerate() {
            if i == write {
                match s {
                    Slot::Write(p) => w = Some(&mut **p),
                    Slot::Read(_) => panic!("slot {i} is written, and pinned to read"),
                }
            } else if i == read {
                r = Some(s.bytes());
            }
        }
        (r.expect("a slot read"), w.expect("a slot written"))
    }

    /// Slot `i`'s pin, to write through.
    pub fn pin_mut(&mut self, i: usize) -> &mut BlockPin {
        match &mut self.slots[i] {
            Slot::Write(p) => p,
            Slot::Read(_) => panic!("slot {i} is written, and pinned to read"),
        }
    }
}

/// A phase's tile: its items `lo..hi`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    pub lo: u32,
    pub hi: u32,
}

impl Tile {
    pub fn items(self) -> std::ops::Range<usize> {
        self.lo as usize..self.hi as usize
    }
}

/// One phase of a twin, by its signature's phase's name.
#[derive(Clone, Copy)]
pub struct TwinPhase {
    pub name: &'static str,
    /// How many items the phase has now: a fixed count, or one an earlier phase made (an indirect size).
    pub items: fn(&Ctx<'_, '_, '_>) -> u32,
    /// Runs a tile of its items.
    pub run: fn(&mut Ctx<'_, '_, '_>, Tile),
    /// After every tile ran: whether the phase runs again (its finisher, one item).
    pub again: Option<fn(&mut Ctx<'_, '_, '_>) -> bool>,
}

/// A twin: its phases, in order.
pub type Twin = &'static [TwinPhase];

/// A fixed-width integer a params struct holds, with its C type for UKL. Native byte order, as blocks are.
pub trait Scalar: Copy {
    const C: &'static str;
    const SIZE: usize;
    fn read(b: &[u8]) -> Self;
    fn write(self, b: &mut [u8]);
}

macro_rules! scalar {
    ($($t:ty => $c:literal),*) => {$(
        impl Scalar for $t {
            const C: &'static str = $c;
            const SIZE: usize = core::mem::size_of::<$t>();
            fn read(b: &[u8]) -> Self {
                let mut a = [0u8; core::mem::size_of::<$t>()];
                a.copy_from_slice(&b[..Self::SIZE]);
                <$t>::from_ne_bytes(a)
            }
            fn write(self, b: &mut [u8]) {
                b[..Self::SIZE].copy_from_slice(&self.to_ne_bytes());
            }
        }
    )*};
}
scalar!(u32 => "uint32_t", u64 => "uint64_t", i32 => "int32_t", i64 => "int64_t");

/// One field of a params struct: its name, C type and offset.
pub struct Field {
    pub name: &'static str,
    pub c: &'static str,
    pub at: usize,
}

/// A params struct (`params!`): its layout, and its bytes read and written field by field.
pub trait Params: Sized {
    const NAME: &'static str;
    const FIELDS: &'static [Field];
    const SIZE: usize;
    fn read(b: &[u8]) -> Self;
    fn write(&self, b: &mut [u8]);
}

/// The params an operation takes and no others.
pub struct NoParams;

impl Params for NoParams {
    const NAME: &'static str = "NoParams";
    const FIELDS: &'static [Field] = &[];
    const SIZE: usize = 0;
    fn read(_: &[u8]) -> Self {
        NoParams
    }
    fn write(&self, _: &mut [u8]) {}
}

/// Declares an operation's params: `#[repr(C)]`, every byte a named field (padding is an error, so a kernel cannot
/// lay it out differently), fields of `Scalar` types only.
#[macro_export]
macro_rules! params {
    ($(#[$m:meta])* pub struct $name:ident { $($(#[$fm:meta])* pub $f:ident : $t:ty),* $(,)? }) => {
        $(#[$m])*
        #[repr(C)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct $name { $($(#[$fm])* pub $f: $t),* }
        const _: () = {
            let sum = 0 $(+ core::mem::size_of::<$t>())*;
            assert!(sum == core::mem::size_of::<$name>(), concat!(stringify!($name), ": implicit padding; name it as a field"));
        };
        impl $crate::Params for $name {
            const NAME: &'static str = stringify!($name);
            const FIELDS: &'static [$crate::Field] = &[$($crate::Field {
                name: stringify!($f),
                c: <$t as $crate::Scalar>::C,
                at: core::mem::offset_of!($name, $f),
            }),*];
            const SIZE: usize = core::mem::size_of::<$name>();
            fn read(b: &[u8]) -> Self {
                $name { $($f: <$t as $crate::Scalar>::read(&b[core::mem::offset_of!($name, $f)..])),* }
            }
            fn write(&self, b: &mut [u8]) {
                $(<$t as $crate::Scalar>::write(self.$f, &mut b[core::mem::offset_of!($name, $f)..]);)*
            }
        }
    };
}
