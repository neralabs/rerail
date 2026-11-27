//! How a type is declared in UKL, and the declarations it needs first (moved from `rerail_blocks`): C's fixed-width
//! types, a structure a `typedef`, its assertions `UKL_STATIC_ASSERT`s, for a kernel on every backend (decision 0440).
//!
//! Written into a `fmt::Write`, never built as a `String`: the floor has no allocator. A structure's declaration is
//! made once, by name (`Decls::declare`); `rerail_blocks::ukl_of` gathers them into its text.

use core::fmt::{self, Write};

/// Where a type's declarations go: each structure once, after the ones it needs.
pub trait Decls {
    /// Declares `name` with the text `text` writes, unless declared already.
    fn declare(&mut self, name: &'static str, text: &dyn Fn(&mut dyn Write) -> fmt::Result) -> fmt::Result;
}

/// A type as UKL declares it: its type's name, a field of it, and the structures it needs declared before it.
pub trait KernelField {
    fn ty(out: &mut dyn Write) -> fmt::Result;
    fn decl(name: &str, out: &mut dyn Write) -> fmt::Result {
        Self::ty(out)?;
        write!(out, " {name}")
    }
    fn deps(_out: &mut dyn Decls) -> fmt::Result {
        Ok(())
    }
}

macro_rules! scalar {
    ($($t:ty => $c:literal),*) => {$(
        impl KernelField for $t {
            fn ty(out: &mut dyn Write) -> fmt::Result { out.write_str($c) }
        }
    )*};
}
scalar!(u8 => "uint8_t", u16 => "uint16_t", u32 => "uint32_t", u64 => "uint64_t", i32 => "int32_t", i64 => "int64_t");
// A field that changes while its block is open (a counter, a chunk's offset, a span set at freeze) is an atomic:
// written through a shared reference, which Rust allows only by interior mutability. On ARM a relaxed load is a
// plain load. UKL declares it as the plain type; a kernel that claims passes it to `ukl_atomic_add`.
scalar!(core::sync::atomic::AtomicU16 => "uint16_t", core::sync::atomic::AtomicU32 => "uint32_t", core::sync::atomic::AtomicU64 => "uint64_t", core::sync::atomic::AtomicI16 => "int16_t", core::sync::atomic::AtomicI32 => "int32_t", core::sync::atomic::AtomicI64 => "int64_t");

impl<T: KernelField, const N: usize> KernelField for [T; N] {
    fn ty(out: &mut dyn Write) -> fmt::Result {
        T::ty(out)
    }
    fn decl(name: &str, out: &mut dyn Write) -> fmt::Result {
        T::ty(out)?;
        write!(out, " {name}[{N}]")
    }
    fn deps(out: &mut dyn Decls) -> fmt::Result {
        T::deps(out)
    }
}

/// Declares a structure that lives in a block: `#[repr(C)]`, every byte a named field (padding is an error, so
/// a kernel cannot lay it out differently), never `Copy`, and with its UKL text and offset assertions.
#[macro_export]
macro_rules! structure {
    ($(#[$m:meta])* pub struct $name:ident { $($(#[$fm:meta])* pub $f:ident : $t:ty),* $(,)? }) => {
        $(#[$m])*
        #[repr(C)]
        pub struct $name { $($(#[$fm])* pub $f: $t),* }
        const _: () = {
            let sum = 0 $(+ core::mem::size_of::<$t>())*;
            assert!(sum == core::mem::size_of::<$name>(), concat!(stringify!($name), ": implicit padding; name it as a field"));
        };
        unsafe impl $crate::Pod for $name {}
        impl $crate::kernel::KernelField for $name {
            fn ty(out: &mut dyn core::fmt::Write) -> core::fmt::Result { out.write_str(stringify!($name)) }
            fn deps(out: &mut dyn $crate::kernel::Decls) -> core::fmt::Result {
                $(<$t as $crate::kernel::KernelField>::deps(out)?;)*
                out.declare(stringify!($name), &|s: &mut dyn core::fmt::Write| {
                    write!(s, "typedef struct {} {{\n", stringify!($name))?;
                    $(
                        s.write_str("    ")?;
                        <$t as $crate::kernel::KernelField>::decl(stringify!($f), s)?;
                        s.write_str(";\n")?;
                    )*
                    write!(s, "}} {};\n", stringify!($name))?;
                    write!(s, "UKL_STATIC_ASSERT(sizeof({0}) == {1}, \"{0}: size\");\n", stringify!($name), core::mem::size_of::<$name>())?;
                    $(write!(s, "UKL_STATIC_ASSERT(UKL_OFFSETOF({0}, {1}) == {2}, \"{0}.{1}\");\n",
                        stringify!($name), stringify!($f), core::mem::offset_of!($name, $f))?;)*
                    Ok(())
                })
            }
        }
    };
}
