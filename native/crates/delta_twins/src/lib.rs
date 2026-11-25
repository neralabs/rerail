//! The Rust twins of the delta executor's operations (decision 0686; *0686: Operations* §3).
//!
//! An operation is a pure function of its blocks and its params: what it writes depends on them and on nothing else.
//! Its twin is that function in plain Rust, registered as the operation's reference version, against which every
//! other version (UKL, compiled) is checked. So this crate is `#![no_std]` with no `alloc` and no dependencies: a twin
//! cannot name a `Vec`, a map, a lock or a thread-local, and the shape check (`check_delta_exec_shape.py`) refuses a
//! `static` and `unsafe` here. Everything a twin reads or writes is a slot's bytes, read in place.
//!
//! A params struct is declared once, here, by `params!`; the executor gives UKL its declaration from `Params::FIELDS`.

#![no_std]
#![forbid(unsafe_code)]

/// One slot's block as a twin sees it: all its bytes, header first, as the executor pinned it (read, or written).
pub enum Slot<'a> {
    Read(&'a [u8]),
    Write(&'a mut [u8]),
}

impl Slot<'_> {
    pub fn bytes(&self) -> &[u8] {
        match self {
            Slot::Read(b) => b,
            Slot::Write(b) => b,
        }
    }

    /// The bytes to write: `None` for a slot pinned to read.
    pub fn bytes_mut(&mut self) -> Option<&mut [u8]> {
        match self {
            Slot::Read(_) => None,
            Slot::Write(b) => Some(b),
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
