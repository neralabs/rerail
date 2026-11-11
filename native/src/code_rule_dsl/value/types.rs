//! The value domain's language types (`Value`'s): Rust's integer and float types, PHP's array keys. The
//! languages read them (`value::rust`, `value::php`); the core holds them in its values.

use num_bigint::BigInt;

/// A Rust integer type (suffix or annotation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IntTy {
    I8,
    I16,
    I32,
    I64,
    I128,
    Isize,
    U8,
    U16,
    U32,
    U64,
    U128,
    Usize,
}
impl IntTy {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::I128 => "i128",
            Self::Isize => "isize",
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
            Self::U128 => "u128",
            Self::Usize => "usize",
        }
    }
    pub(crate) fn limits(self) -> (BigInt, BigInt) {
        let bits = match self {
            Self::I8 | Self::U8 => 8,
            Self::I16 | Self::U16 => 16,
            Self::I32 | Self::U32 => 32,
            Self::I64 | Self::U64 => 64,
            Self::I128 | Self::U128 => 128,
            Self::Isize | Self::Usize => 64,
        };
        let one = BigInt::from(1u8);
        if self.name().starts_with('i') || self == Self::Isize {
            (
                -(one.clone() << (bits - 1usize)),
                (one << (bits - 1usize)) - 1,
            )
        } else {
            (BigInt::from(0), (one << bits) - 1)
        }
    }
}

/// A Rust float type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FloatTy {
    F32,
    F64,
}
impl FloatTy {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::F32 => "f32",
            Self::F64 => "f64",
        }
    }
}

/// A PHP array key: an integer, or a string that is not a canonical decimal integer.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Key {
    Int(i64),
    Str(Vec<u8>),
}
