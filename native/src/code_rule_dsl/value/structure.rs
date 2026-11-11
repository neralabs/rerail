//! Structural equality: a node read as a sequence of tokens (`Tok`) under one of the compare text modes
//! (`Equality`), each language's by its own rules (`ValueRules::structure`: Python's is `python_structure`,
//! as Ruff compares; PHP's `php_structure`).


/// Which of Ruff's equalities a comparison applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Equality {
    Plain,
    Hash,
    Parts,
}

/// One token of a node's structure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tok {
    Open(String),
    Close,
    /// A named leaf (an identifier, NFKC-normalized as Ruff's AST holds it; a keyword argument's
    /// name): its kind and text.
    Leaf(String, String),
    /// An anonymous token (an operator, a keyword): its kind.
    Token(String),
    /// An empty slot of a slice (`x[:1]` has no lower bound).
    Missing,
    /// An `int` below 2**64.
    Int(u64),
    /// An `int` of 2**64 or more: Ruff keeps its token, underscores dropped for a decimal one.
    BigInt(String),
    Float(u64),
    Complex(u64, u64),
    Str(String),
    Bytes(Vec<u8>),
    /// A number folded as `HashableExpr` folds it: real and imaginary parts.
    Number(Real, Real),
    /// Debug text of an f-string replacement field (`{x = }`).
    Debug(String),
}

/// A real part of a folded number: an integer when the value is one below 2**64, else its bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Real {
    Integer(i128),
    Float(u64),
}

impl Real {
    pub fn from_float(value: f64) -> Self {
        const BOUND: f64 = 18_446_744_073_709_551_616.0;
        if value.is_finite() && value.abs() < BOUND {
            let integer = value as i128;
            if integer as f64 == value {
                return Self::Integer(integer);
            }
        }
        Self::Float(value.to_bits())
    }

    pub fn into_float(self) -> Self {
        match self {
            Self::Integer(integer) => Self::from_float(integer as f64),
            Self::Float(_) => self,
        }
    }

    pub fn negate(self) -> Self {
        match self {
            Self::Integer(integer) => Self::Integer(-integer),
            Self::Float(bits) => Self::Float(bits ^ (1 << 63)),
        }
    }
}
