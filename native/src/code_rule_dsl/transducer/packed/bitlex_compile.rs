//! The Bitlex compiler: a Bitlex file (`docs/parser-generators/03-bitlex.md`) to a block program
//! (`block`), printed as `rir` rows so it goes through the transducer's own assembler and
//! verifier. It holds no language: everything comes from the file.
//!
//! The parser is the reference evaluator's (`tools/bitlex`), so Bitlex is parsed in one place, and
//! the evaluator is the compiler's test oracle (`bitlex_compile_tests`).
//!
//! Built one construct at a time (`docs/parser-generators/13-tape-on-the-gpu.md` §7, step 2).
//!
//! - **Classes.** Each class is a union of products of a set of high nibbles and a set of low
//!   nibbles. Products are shared between classes and packed eight to a nibble table. A one-byte
//!   class is an `eq`.
//! - **Masks** over `~ & | ^`, `empty`, `prev`, `next`, `starts`, `lasts`, `after`, `span`,
//!   `aligned`, `toggle`, `after_odd_run` and `fill`.
//! - **Errors and escapes.** Each mask joins the fallback output (`FALLBACK`); a file with a bit
//!   of it goes to tree-sitter.
//!
//! Anything else is refused by name.
//!
//! # Delays: `next` without lookahead
//!
//! Every value is computed some bytes late: its bit `p` is byte `p - d`'s, for its delay `d`.
//! Bytes and classes have delay 0. An operation's operands are brought to the later delay by
//! `prev`, and `next(a, k)` is `a` itself with `k` more delay. The outputs are brought to the
//! largest delay `D`, the program's `delay`; each executor runs `D` bytes past the stream and
//! shifts the outputs back (`block::undelay`).
//!
//! So that no value has bits outside the stream (which `prev` and `next` would carry in):
//!
//! - before the stream every value is zero: `prev` shifts in zeros, `~a` is `within & ~a` with
//!   `within` (the stream's bytes) at the same delay, and `next` clears the bytes it brings in
//!   before the start;
//! - past the end a value may be *dirty*: a byte test that the zero padding passes (`eq 0x00`),
//!   a constant, or a carried operation whose state runs on. `next` cleans its operand with
//!   `within` when it is dirty.

use super::block::{BlockOp, BlockProgram, ClassTable};
use ::bitlex::syntax::{Arg, Expr, Stmt};
use std::collections::HashMap;
use std::fmt::Write as _;

/// A compiled program: its `rir` rows (`block 0 …`), whose outputs are the requested masks in
/// order.
#[derive(Debug)]
pub(in crate::code_rule_dsl) struct Compiled {
    pub(super) rows: String,
}

/// The output name for the fallback bit: set somewhere when the file goes to tree-sitter.
pub(in crate::code_rule_dsl) const FALLBACK: &str = "fallback";

/// A product of nibble sets: bit `h` of `hi` and bit `l` of `lo` for byte `h << 4 | l`.
type Product = (u16, u16);

/// A compiled mask: its value, how many bytes late it is, and whether it may have bits past the
/// stream's end.
#[derive(Clone, Copy, Debug)]
struct Val {
    v: u16,
    d: u8,
    dirty: bool,
}

/// The largest delay: the outputs are brought to it by `prev`s, and the executors shift it out.
const MAX_DELAY: u8 = 63;

struct Lowering {
    program: BlockProgram,
    /// Each operation's value, so an operation is emitted once.
    seen: HashMap<BlockOp, u16>,
    /// Each product's table and bit.
    products: HashMap<Product, (u16, u8)>,
    /// Classes, lowered when first used.
    classes: HashMap<String, Vec<(u32, u32)>>,
    names: HashMap<String, Val>,
    /// The masks any of whose bits sends the file to tree-sitter: errors and escapes.
    fallbacks: Vec<Val>,
}

/// Compile `stmts`, with `outputs` (mask names) as the block program's outputs.
pub(in crate::code_rule_dsl) fn compile(stmts: &[Stmt], outputs: &[&str]) -> Result<Compiled, String> {
    let mut low = Lowering {
        program: BlockProgram::default(),
        seen: HashMap::new(),
        products: HashMap::new(),
        classes: HashMap::new(),
        names: HashMap::new(),
        fallbacks: Vec::new(),
    };
    // Every class's products first: the tables come before the operations.
    for stmt in stmts {
        if let Stmt::Class(name, ranges) = stmt {
            for product in products(ranges) {
                low.product(product);
            }
            low.classes.insert(name.clone(), ranges.clone());
        }
    }
    low.stmts(stmts)?;
    let mut vals = Vec::new();
    for name in outputs {
        vals.push(if *name == FALLBACK { low.fallback()? } else { low.name(name)? });
    }
    let delay = vals.iter().map(|x| x.d).max().unwrap_or(0);
    for x in vals {
        let v = low.at(x, delay)?;
        low.program.outputs.push(v);
    }
    low.program.delay = delay;
    let program = live(&low.program);
    Ok(Compiled { rows: rows(&program) })
}

impl Lowering {
    fn stmts(&mut self, stmts: &[Stmt]) -> Result<(), String> {
        for stmt in stmts {
            match stmt {
                Stmt::Lexer(_) | Stmt::Class(..) => {}
                Stmt::Mask(name, expr) => {
                    let value = self.expr(expr)?;
                    self.names.insert(name.clone(), value);
                }
                // Both send the file to tree-sitter.
                Stmt::Error(_, expr) | Stmt::Escape(_, expr) => {
                    let value = self.expr(expr)?;
                    self.fallbacks.push(value);
                }
                other => return Err(format!("not compiled yet: {}", statement(other))),
            }
        }
        Ok(())
    }

    /// The OR of the fallback masks (`empty` without any).
    fn fallback(&mut self) -> Result<Val, String> {
        let mut out = self.leaf(BlockOp::Const(0), false);
        for x in self.fallbacks.clone() {
            out = self.or(out, x)?;
        }
        Ok(out)
    }

    fn product(&mut self, product: Product) -> (u16, u8) {
        if let Some(&at) = self.products.get(&product) {
            return at;
        }
        let n = self.products.len();
        let (table, bit) = ((n / 8) as u16, 1u8 << (n % 8));
        if usize::from(table) == self.program.tables.len() {
            self.program.tables.push(ClassTable::default());
        }
        let t = &mut self.program.tables[usize::from(table)];
        for h in 0..16 {
            if product.0 >> h & 1 == 1 {
                t.hi[h] |= bit;
            }
            if product.1 >> h & 1 == 1 {
                t.lo[h] |= bit;
            }
        }
        self.products.insert(product, (table, bit));
        (table, bit)
    }

    fn op(&mut self, op: BlockOp) -> u16 {
        if let Some(&v) = self.seen.get(&op) {
            return v;
        }
        let v = self.program.ops.len() as u16;
        self.program.ops.push(op);
        self.seen.insert(op, v);
        v
    }

    /// An operation on bytes or constants: delay 0.
    fn leaf(&mut self, op: BlockOp, dirty: bool) -> Val {
        Val { v: self.op(op), d: 0, dirty }
    }

    /// `x`'s value brought to delay `d` (at least `x.d`), by `prev`s of at most 63 bytes.
    fn at(&mut self, x: Val, d: u8) -> Result<u16, String> {
        if d > MAX_DELAY {
            return Err(format!("a delay of {d} bytes: more than {MAX_DELAY} (`next` reaches too far)"));
        }
        let mut v = x.v;
        let mut left = d - x.d;
        while left > 0 {
            let k = left.min(63);
            v = self.op(BlockOp::Prev(v, k));
            left -= k;
        }
        Ok(v)
    }

    /// `within`, the stream's bytes, at delay `d`.
    fn within(&mut self, d: u8) -> Result<u16, String> {
        let w = self.leaf(BlockOp::Within, false);
        self.at(w, d)
    }

    fn two(&mut self, a: Val, b: Val, f: fn(u16, u16) -> BlockOp, dirty: bool) -> Result<Val, String> {
        let d = a.d.max(b.d);
        let (x, y) = (self.at(a, d)?, self.at(b, d)?);
        Ok(Val { v: self.op(f(x, y)), d, dirty })
    }

    fn and(&mut self, a: Val, b: Val) -> Result<Val, String> {
        self.two(a, b, BlockOp::And, a.dirty && b.dirty)
    }

    fn or(&mut self, a: Val, b: Val) -> Result<Val, String> {
        self.two(a, b, BlockOp::Or, a.dirty || b.dirty)
    }

    fn xor(&mut self, a: Val, b: Val) -> Result<Val, String> {
        self.two(a, b, BlockOp::Xor, a.dirty || b.dirty)
    }

    /// `a & ~b`: no bits outside the stream that `a` has none of.
    fn andnot(&mut self, a: Val, b: Val) -> Result<Val, String> {
        self.two(a, b, BlockOp::AndNot, a.dirty)
    }

    /// `~a`, inside the stream only.
    fn not(&mut self, a: Val) -> Result<Val, String> {
        let w = self.within(a.d)?;
        Ok(Val { v: self.op(BlockOp::AndNot(w, a.v)), d: a.d, dirty: false })
    }

    fn prev(&mut self, a: Val, k: u64) -> Result<Val, String> {
        let mut v = a.v;
        let mut left = k;
        while left > 0 {
            let step = left.min(63) as u8;
            v = self.op(BlockOp::Prev(v, step));
            left -= u64::from(step);
        }
        Ok(Val { v, ..a })
    }

    /// `next(a, k)`: `a` itself, `k` bytes later. Two cleanings: past the end in `a`'s own
    /// delay, if it is dirty; and before the start in the new delay, where `a`'s first `k`
    /// bytes now stand for the bytes before the stream.
    fn next(&mut self, a: Val, k: u64) -> Result<Val, String> {
        let d = u64::from(a.d) + k;
        if d > u64::from(MAX_DELAY) {
            return Err(format!("next by {k} at a delay of {}: more than {MAX_DELAY} bytes", a.d));
        }
        let d = d as u8;
        let mut v = a.v;
        if a.dirty {
            let w = self.within(a.d)?;
            v = self.op(BlockOp::And(v, w));
        }
        if k > 0 {
            let w = self.within(d)?;
            v = self.op(BlockOp::And(v, w));
        }
        Ok(Val { v, d, dirty: false })
    }

    /// A carried operation of one operand: its state may run on past the end.
    fn carried(&mut self, a: Val, f: fn(u16) -> BlockOp) -> Val {
        Val { v: self.op(f(a.v)), d: a.d, dirty: true }
    }

    fn fill(&mut self, s: Val, e: Val) -> Result<Val, String> {
        self.two(s, e, BlockOp::Fill, true)
    }

    /// A pattern over the stream's byte positions (bit `i % 64` for byte `i`).
    fn pattern(&mut self, c: u64) -> Val {
        self.leaf(BlockOp::Const(c), c != 0)
    }

    fn name(&mut self, name: &str) -> Result<Val, String> {
        if let Some(&v) = self.names.get(name) {
            return Ok(v);
        }
        let ranges = self.classes.get(name).cloned().ok_or_else(|| format!("undefined: {name}"))?;
        let value = self.class(&ranges);
        self.names.insert(name.to_owned(), value);
        Ok(value)
    }

    /// A class: an `eq` for one byte, otherwise one `class` per table its products use, ORed.
    /// Dirty when the class holds the padding's byte, 0x00.
    fn class(&mut self, ranges: &[(u32, u32)]) -> Val {
        let bytes: Vec<u8> = (0..=255u8).filter(|&b| ranges.iter().any(|&(lo, hi)| lo <= u32::from(b) && u32::from(b) <= hi)).collect();
        let dirty = bytes.first() == Some(&0);
        match bytes.as_slice() {
            [] => return self.leaf(BlockOp::Const(0), false),
            [b] => return self.leaf(BlockOp::Eq(*b), dirty),
            _ => {}
        }
        let mut per_table: Vec<(u16, u8)> = Vec::new();
        for product in products(ranges) {
            let (table, bit) = self.product(product);
            match per_table.iter_mut().find(|(t, _)| *t == table) {
                Some((_, bits)) => *bits |= bit,
                None => per_table.push((table, bit)),
            }
        }
        let mut value = None;
        for (table, bits) in per_table {
            let v = self.op(BlockOp::Class { table, bits });
            value = Some(match value {
                None => v,
                Some(acc) => self.op(BlockOp::Or(acc, v)),
            });
        }
        Val { v: value.expect("a class of two bytes or more has a product"), d: 0, dirty }
    }

    fn expr(&mut self, e: &Expr) -> Result<Val, String> {
        match e {
            Expr::Name(name) => self.name(name),
            Expr::Empty => Ok(self.leaf(BlockOp::Const(0), false)),
            Expr::Not(a) => {
                let a = self.expr(a)?;
                self.not(a)
            }
            // `a & ~b` is one `andnot`.
            Expr::And(a, b) => match (&**a, &**b) {
                (x, Expr::Not(y)) | (Expr::Not(y), x) => {
                    let (x, y) = (self.expr(x)?, self.expr(y)?);
                    self.andnot(x, y)
                }
                _ => {
                    let (a, b) = (self.expr(a)?, self.expr(b)?);
                    self.and(a, b)
                }
            },
            Expr::Or(a, b) => {
                let (a, b) = (self.expr(a)?, self.expr(b)?);
                self.or(a, b)
            }
            Expr::Xor(a, b) => {
                let (a, b) = (self.expr(a)?, self.expr(b)?);
                self.xor(a, b)
            }
            Expr::Call(f, args) => self.call(f, args),
            Expr::Test { target, method, .. } => Err(format!("not compiled yet: {target}.{method}")),
        }
    }

    fn call(&mut self, f: &str, args: &[Arg]) -> Result<Val, String> {
        let mask = |low: &mut Self, i: usize| -> Result<Val, String> {
            match args.get(i) {
                Some(Arg::E(e)) => low.expr(e),
                _ => Err(format!("{f}: argument {i} must be a mask")),
            }
        };
        let number = |i: usize, default: u64| -> Result<u64, String> {
            match args.get(i) {
                Some(Arg::N(v)) => Ok(*v),
                None => Ok(default),
                _ => Err(format!("{f}: argument {i} must be a number")),
            }
        };
        match f {
            "prev" => {
                let (a, k) = (mask(self, 0)?, number(1, 1)?);
                self.prev(a, k)
            }
            "next" => {
                let (a, k) = (mask(self, 0)?, number(1, 1)?);
                self.next(a, k)
            }
            "starts" => {
                let a = mask(self, 0)?;
                let p = self.prev(a, 1)?;
                self.andnot(a, p)
            }
            "lasts" => {
                let a = mask(self, 0)?;
                let n = self.next(a, 1)?;
                self.andnot(a, n)
            }
            "after" => {
                let a = mask(self, 0)?;
                let p = self.prev(a, 1)?;
                self.andnot(p, a)
            }
            "span" => {
                let (a, k) = (mask(self, 0)?, number(1, 1)?);
                let mut out = a;
                for j in 1..k {
                    let p = self.prev(a, j)?;
                    out = self.or(out, p)?;
                }
                Ok(out)
            }
            "aligned" => {
                let (a, k) = (mask(self, 0)?, number(1, 1)?);
                self.aligned(a, k)
            }
            "toggle" => {
                let a = mask(self, 0)?;
                Ok(self.carried(a, BlockOp::Toggle))
            }
            "after_odd_run" => {
                let a = mask(self, 0)?;
                Ok(self.carried(a, BlockOp::AfterOddRun))
            }
            "fill" => {
                let (start, end) = (mask(self, 0)?, mask(self, 1)?);
                self.fill(start, end)
            }
            _ => Err(format!("not compiled yet: {f}")),
        }
    }

    /// `aligned(a, k)`: the bytes of `a` at offsets 0, k, 2k, … in their run. For each residue
    /// `r` of a run's start mod `k`, a fill over the runs that start there, kept at the
    /// positions of the same residue. `k` divides 64, so the residues repeat every word.
    fn aligned(&mut self, a: Val, k: u64) -> Result<Val, String> {
        if !matches!(k, 1 | 2 | 4 | 8) {
            return Err(format!("aligned by {k}: only 1, 2, 4 or 8 are compiled"));
        }
        if k == 1 {
            return Ok(a);
        }
        let p = self.prev(a, 1)?;
        let starts = self.andnot(a, p)?;
        let outside = self.not(a)?;
        let mut out = self.leaf(BlockOp::Const(0), false);
        for r in 0..k {
            let residue = self.pattern((0..64).filter(|i| i % k == r).fold(0u64, |acc, i| acc | 1 << i));
            let here = self.and(starts, residue)?;
            let runs = self.fill(here, outside)?;
            let kept = self.and(runs, residue)?;
            out = self.or(out, kept)?;
        }
        self.and(out, a)
    }
}

/// The products whose union is the class `ranges`: high nibbles grouped by their set of low
/// nibbles.
fn products(ranges: &[(u32, u32)]) -> Vec<Product> {
    let mut out: Vec<Product> = Vec::new();
    for h in 0..16u32 {
        let lo = (0..16u32)
            .filter(|&l| ranges.iter().any(|&(a, b)| a <= (h << 4 | l) && (h << 4 | l) <= b))
            .fold(0u16, |acc, l| acc | 1 << l);
        if lo == 0 {
            continue;
        }
        match out.iter_mut().find(|p| p.1 == lo) {
            Some(p) => p.0 |= 1 << h,
            None => out.push((1 << h, lo)),
        }
    }
    out
}

/// The program with only the operations its outputs read, renumbered in order.
fn live(program: &BlockProgram) -> BlockProgram {
    let mut keep = vec![false; program.ops.len()];
    for &v in &program.outputs {
        keep[usize::from(v)] = true;
    }
    for at in (0..program.ops.len()).rev() {
        if keep[at] {
            for v in program.ops[at].operands() {
                keep[usize::from(v)] = true;
            }
        }
    }
    let mut number = vec![u16::MAX; program.ops.len()];
    let mut out = BlockProgram { tables: program.tables.clone(), delay: program.delay, ..BlockProgram::default() };
    for (at, op) in program.ops.iter().enumerate() {
        if keep[at] {
            number[at] = out.ops.len() as u16;
            out.ops.push(op.renumbered(|v| number[usize::from(v)]));
        }
    }
    out.outputs = program.outputs.iter().map(|&v| number[usize::from(v)]).collect();
    out
}

fn rows(program: &BlockProgram) -> String {
    let mut out = String::new();
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    for (t, table) in program.tables.iter().enumerate() {
        let _ = writeln!(out, "  block 0 table {t} lo={} hi={}", hex(&table.lo), hex(&table.hi));
    }
    for (value, op) in program.ops.iter().enumerate() {
        let _ = writeln!(out, "  block 0 %{value} = {}", op.text());
    }
    for value in &program.outputs {
        let _ = writeln!(out, "  block 0 output %{value}");
    }
    if program.delay != 0 {
        let _ = writeln!(out, "  block 0 delay {}", program.delay);
    }
    out
}

fn statement(stmt: &Stmt) -> String {
    let text = format!("{stmt:?}");
    text.split(['(', ' ', '{']).next().unwrap_or("?").to_owned()
}
