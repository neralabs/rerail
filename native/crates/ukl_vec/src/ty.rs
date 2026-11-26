//! C's types as the lowering checks them, and their layout (the host's: LP64, as UKL is written for).

use crate::k::M;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    Void,
    Bool,
    Int { bits: u8, signed: bool },
    Float(u8),
    Ptr(Box<Ty>),
    Array(Box<Ty>, u64),
    /// A structure or union, by its index in the program's (`Structs`).
    Struct(u32),
    /// A function, by its index in the program's.
    Func(u32),
}

pub const U32: Ty = Ty::Int { bits: 32, signed: false };
pub const I32: Ty = Ty::Int { bits: 32, signed: true };
pub const U64: Ty = Ty::Int { bits: 64, signed: false };
pub const I64: Ty = Ty::Int { bits: 64, signed: true };

/// A structure or union's fields and layout.
#[derive(Clone, Debug, Default)]
pub struct Struct {
    pub name: String,
    pub union: bool,
    /// Each field: its name, type and offset.
    pub fields: Vec<(String, Ty, u64)>,
    pub size: u64,
    pub align: u64,
    /// Whether its fields are known (a definition seen).
    pub complete: bool,
}

impl Ty {
    pub fn is_int(&self) -> bool {
        matches!(self, Ty::Int { .. } | Ty::Bool)
    }

    pub fn is_arith(&self) -> bool {
        matches!(self, Ty::Int { .. } | Ty::Bool | Ty::Float(_))
    }

    pub fn is_scalar(&self) -> bool {
        self.is_arith() || matches!(self, Ty::Ptr(_))
    }

    pub fn is_ptr(&self) -> bool {
        matches!(self, Ty::Ptr(_))
    }

    /// The type a pointer or array points to.
    pub fn pointee(&self) -> Option<&Ty> {
        match self {
            Ty::Ptr(t) | Ty::Array(t, _) => Some(t),
            _ => None,
        }
    }

    pub fn signed(&self) -> bool {
        matches!(self, Ty::Int { signed: true, .. })
    }

    /// Its size and alignment.
    pub fn layout(&self, structs: &[Struct]) -> (u64, u64) {
        match self {
            Ty::Void => (1, 1),
            Ty::Bool => (1, 1),
            Ty::Int { bits, .. } => (u64::from(*bits / 8), u64::from(*bits / 8)),
            Ty::Float(b) => (u64::from(*b / 8), u64::from(*b / 8)),
            Ty::Ptr(_) => (8, 8),
            Ty::Array(t, n) => {
                let (s, a) = t.layout(structs);
                (s * n, a)
            }
            Ty::Struct(k) => {
                let s = &structs[*k as usize];
                (s.size, s.align.max(1))
            }
            Ty::Func(_) => (1, 1),
        }
    }

    pub fn size(&self, structs: &[Struct]) -> u64 {
        self.layout(structs).0
    }

    /// Its machine type: a scalar's own, an aggregate's bytes.
    pub fn m(&self, structs: &[Struct]) -> M {
        match self {
            Ty::Void => M::VOID,
            Ty::Bool => M::B,
            Ty::Int { bits: 8, signed: true } => M::I8,
            Ty::Int { bits: 8, signed: false } => M::U8,
            Ty::Int { bits: 16, signed: true } => M::I16,
            Ty::Int { bits: 16, signed: false } => M::U16,
            Ty::Int { bits: 32, signed: true } => M::I32,
            Ty::Int { bits: 32, signed: false } => M::U32,
            Ty::Int { signed: true, .. } => M::I64,
            Ty::Int { .. } => M::U64,
            Ty::Float(32) => M::F32,
            Ty::Float(_) => M::F64,
            Ty::Ptr(_) | Ty::Func(_) => M::PTR,
            Ty::Array(..) | Ty::Struct(_) => M::agg(self.size(structs) as u32),
        }
    }

    /// Its rank among the integers for the usual conversions (C11 6.3.1.1): by its bits.
    fn rank(&self) -> u8 {
        match self {
            Ty::Bool => 1,
            Ty::Int { bits, .. } => *bits,
            _ => 0,
        }
    }

    /// The integer promotion (C11 6.3.1.1 §2): below `int`, `int`.
    pub fn promoted(&self) -> Ty {
        match self {
            Ty::Bool => I32,
            Ty::Int { bits, .. } if *bits < 32 => I32,
            t => t.clone(),
        }
    }

    /// The usual arithmetic conversions of `a` and `b` (C11 6.3.1.8): their common type.
    pub fn common(a: &Ty, b: &Ty) -> Ty {
        if let (Ty::Float(x), Ty::Float(y)) = (a, b) {
            return Ty::Float(*x.max(y));
        }
        if let Ty::Float(x) = a {
            return Ty::Float(*x);
        }
        if let Ty::Float(y) = b {
            return Ty::Float(*y);
        }
        let (a, b) = (a.promoted(), b.promoted());
        if a == b {
            return a;
        }
        let (ra, rb) = (a.rank(), b.rank());
        match (a.signed(), b.signed()) {
            (x, y) if x == y => {
                if ra >= rb {
                    a
                } else {
                    b
                }
            }
            // One signed, one not: the unsigned one if its rank is not below; else the signed one if it holds every
            // value of the unsigned (a greater rank does, here); else the signed one's unsigned type.
            (false, true) => {
                if ra >= rb {
                    a
                } else {
                    b
                }
            }
            (true, false) => {
                if rb >= ra {
                    b
                } else {
                    a
                }
            }
            _ => unreachable!(),
        }
    }
}

/// Lays `s`'s fields out (C's rules: each at its alignment, the whole to the largest).
pub fn lay_out(s: &mut Struct, structs: &[Struct]) {
    let (mut at, mut align, mut size) = (0u64, 1u64, 0u64);
    for f in &mut s.fields {
        let (fs, fa) = f.1.layout(structs);
        align = align.max(fa);
        if s.union {
            f.2 = 0;
            size = size.max(fs);
        } else {
            at = at.next_multiple_of(fa);
            f.2 = at;
            at += fs;
            size = at;
        }
    }
    s.align = align;
    s.size = size.next_multiple_of(align);
    s.complete = true;
}
