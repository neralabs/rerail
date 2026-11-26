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
//! relation made, its rows claimed and written (`Relation::put_own`), frozen an item a call (`relation::layer_write`, under the twins' rules), its orders
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

pub mod cache;
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

/// What a phase's items report of their bounded work (Operations §4, the bounds axis), held by the executor to the
/// phase's bounds: steps an item, a stack's depth, a sample's size, and the room a written slot needs. The executor
/// sets the phase's bounds before its first tile and reads the meter after each. A twin reports what it does: an item
/// past its bound stops, and the launch fails; a report of a quantity its phase's shape bounds not fails it too, so a
/// twin cannot do what its declared shape says it does not.
#[derive(Debug, Default)]
pub struct Meter {
    /// The phase's bound on an item's steps (`None`: its shape bounds none).
    pub steps: Option<u32>,
    /// The phase's bound on a stack's depth.
    pub depth: Option<u32>,
    /// The phase's sample size.
    pub sample: Option<u32>,
    /// Whether the phase bounds its output's capacity (refuse, grow or rerun).
    pub bounded_room: bool,
    /// Each slot's capacity: how far its block's allocations may reach, from the block's start.
    pub capacity: Vec<usize>,
    /// The first report past a bound, or of what the shape bounds not: the launch fails with it.
    pub fault: Option<String>,
    /// Slots an item found too small, each with the capacity it needs; the item wrote nothing past the capacity.
    pub needs: Vec<(usize, usize)>,
}

impl Meter {
    fn within(&mut self, bound: Option<u32>, n: u32, what: &str) -> bool {
        match bound {
            None => {
                self.fault.get_or_insert_with(|| format!("an item reports {what}, and its phase's shape bounds none"));
                false
            }
            Some(b) if n > b => {
                self.fault.get_or_insert_with(|| format!("an item's {what} reached {n}, past its bound of {b}"));
                false
            }
            Some(_) => true,
        }
    }
}

/// What a twin's phase reads and writes: its slots in its signature's order, the launch's descriptor table (the blocks
/// its slots refer to, by id), its params' bytes (`Params::read`) and the meter of its bounded work.
pub struct Ctx<'c, 'p, 't> {
    pub slots: &'c mut [Slot<'p>],
    pub table: &'c Table<'t>,
    pub params: &'c [u8],
    pub meter: &'c mut Meter,
}

impl<'p> Ctx<'_, 'p, '_> {
    /// An item has taken `n` steps: false past its phase's bound on steps (the item stops; the launch fails).
    pub fn steps(&mut self, n: u32) -> bool {
        let b = self.meter.steps;
        self.meter.within(b, n, "steps")
    }

    /// An item's stack has reached depth `d`: false past its phase's bound on stack depth.
    pub fn depth(&mut self, d: u32) -> bool {
        let b = self.meter.depth;
        self.meter.within(b, d, "stack depth")
    }

    /// The phase's sample size; 0, and the launch fails, when its shape bounds none.
    pub fn sample(&mut self) -> u32 {
        let b = self.meter.sample;
        if self.meter.within(b, 0, "a sample") { b.unwrap_or(0) } else { 0 }
    }

    /// Whether `bytes` more can be allocated in slot `slot`'s block within its capacity. When not, the slot's need is
    /// recorded (the capacity the allocation needs) and the item writes nothing past the capacity: the executor
    /// refuses, grows the capacity and runs the phase again, or runs the launch again, as the phase's bound says.
    pub fn room(&mut self, slot: usize, bytes: usize) -> bool {
        let need = self.slots[slot].pin().block().used().next_multiple_of(64) + bytes;
        self.reach(slot, need)
    }

    /// Whether slot `slot`'s block may be written up to byte `end` within its capacity (a structure it lays out in its
    /// bytes, past its allocations): as `room`, the need recorded when not.
    pub fn reach(&mut self, slot: usize, need: usize) -> bool {
        if !self.meter.bounded_room {
            self.meter.fault.get_or_insert_with(|| format!("an item asks room in slot {slot}, and its phase's shape bounds no capacity"));
            return false;
        }
        if need <= self.meter.capacity[slot] {
            return true;
        }
        match self.meter.needs.iter_mut().find(|n| n.0 == slot) {
            Some(n) => n.1 = n.1.max(need),
            None => self.meter.needs.push((slot, need)),
        }
        false
    }

    /// Slot `slot`'s capacity: how far its block's allocations may reach.
    pub fn capacity(&self, slot: usize) -> usize {
        self.meter.capacity[slot]
    }

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
