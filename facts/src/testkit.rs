//! Test support for the differential tests of `ir` analyses and `rewrite`: a deterministic
//! generator of small relations (NULLs, integers, reals, numeric and plain text, duplicates) and
//! of syntax trees in preorder, and the reference evaluator over them.
use crate::ir::{Column, Expr, Lit, Rel};
use crate::reference::{Evaluator, Row};
use crate::sql::value::{Aff, V};

/// A small deterministic PRNG (xorshift64*).
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    /// A value from a small domain, so joins and duplicates happen.
    pub fn value(&mut self) -> V {
        match self.below(9) {
            0 => V::Null,
            1 => V::Real(self.below(4) as f64),
            2 => V::text(&self.below(4).to_string()),
            3 => V::text(["a", "b", "x"][self.below(3) as usize]),
            _ => V::Int(self.below(4) as i64),
        }
    }
    pub fn rows(&mut self, width: usize, max: u64) -> Vec<Row> {
        (0..self.below(max + 1)).map(|_| (0..width).map(|_| self.value()).collect()).collect()
    }
}

pub fn schema(width: usize) -> Vec<Column> {
    (0..width).map(|i| Column::new(&format!("c{i}"), Aff::None)).collect()
}

pub fn scan(t: u32, width: usize) -> Rel<u32> {
    Rel::Scan { src: t, schema: schema(width) }
}

pub fn col(c: u16) -> Expr<u32> {
    Expr::Col { depth: 0, col: c }
}

pub fn outer(c: u16) -> Expr<u32> {
    Expr::Col { depth: 1, col: c }
}

pub fn int(i: i64) -> Expr<u32> {
    Expr::Lit(Lit::Int(i))
}

/// Evaluates `r` with table `t` = `tables[t]`.
pub fn eval(r: &Rel<u32>, tables: &[Vec<Row>]) -> Result<Vec<Row>, String> {
    let src = |t: &u32| Ok(tables[*t as usize].clone());
    Evaluator::new(&src).rel(r, &[])
}

/// Rows compared as SQLite values in order (`Real(1.0)` and `Int(1)` differ: storage class kept).
pub fn same(a: &[Row], b: &[Row]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.len() == y.len() && x.iter().zip(y).all(|(p, q)| format!("{p:?}") == format!("{q:?}")))
}
