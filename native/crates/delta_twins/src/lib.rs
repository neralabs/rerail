//! The Rust twins of the delta executor's operations (decision 0686; *0686: Operations* §3).
//!
//! An operation is a function of its blocks and its params: what it writes depends on them and on nothing else. Its
//! twin is that function in plain Rust, registered as the operation's reference version, against which every other
//! version (UKL, compiled) is checked.
//!
//! **The rules.** A twin is `TwinFn`: its slots, pinned by the executor from its signature, and its params' bytes.
//! It reads only those, and writes only its write slots' blocks. Rust data structures inside a call are allowed for
//! now (the user, 2026-11-25: "using Rust data structures internally is going to be slow but not wrong"), and so is
//! the blocks crate's code (a relation's freeze, its layouts, read in place). Nothing may outlive a call or come from
//! outside it: no `static`, thread-local, `Once`/`Lazy` cell, lock or atomic of the twin's own; no thread's scratch
//! (`rerail_blocks::scratch::with`: make a `Scratch` of the call's own); no file, environment, process, network or
//! time. `unsafe` is forbidden. `check_delta_exec_shape.py` refuses each of these here, and this crate's
//! `clippy.toml` disallows them by path.
//!
//! A params struct is declared once, here, by `params!`; the executor gives UKL its declaration from `Params::FIELDS`.

#![forbid(unsafe_code)]

use rerail_blocks::region::BlockPin;

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

/// A twin: its slots in its signature's order, and its params' bytes (`Params::read`).
pub type TwinFn = fn(slots: &mut [Slot<'_>], params: &[u8]);

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
