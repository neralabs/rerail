//! The Bitlex compiler: a Bitlex file (`docs/parser-generators/03-bitlex.md`) to a block program
//! (`block`), printed as `rir` rows so it goes through the transducer's own assembler and
//! verifier. It holds no language: everything comes from the file.
//!
//! The parser is the reference evaluator's (`tools/bitlex`), so Bitlex is parsed in one place, and
//! the evaluator is the compiler's test oracle (`bitlex_compile_tests`).
//!
//! Built one construct at a time (`docs/parser-generators/13-tape-on-the-gpu.md` §7, step 2).
//! Handled so far:
//!
//! - `class`: each class is a union of products of a set of high nibbles and a set of low nibbles.
//!   Products are shared between classes and packed eight to a nibble table. A one-byte class is
//!   an `eq`.
//! - `mask` over `~ & | ^`, `empty`, `prev`, `starts`, `after`, `span`, `toggle` and
//!   `after_odd_run`. These read only bytes at or before their own, so the bits a `~` sets past
//!   the end of the input never reach a byte inside it.
//!
//! Anything else is refused by name.

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

/// A product of nibble sets: bit `h` of `hi` and bit `l` of `lo` for byte `h << 4 | l`.
type Product = (u16, u16);

struct Lowering {
    program: BlockProgram,
    /// Each operation's value, so an operation is emitted once.
    seen: HashMap<BlockOp, u16>,
    /// Each product's table and bit.
    products: HashMap<Product, (u16, u8)>,
    /// Classes, lowered when first used.
    classes: HashMap<String, Vec<(u32, u32)>>,
    names: HashMap<String, u16>,
}

/// Compile `stmts`, with `outputs` (mask names) as the block program's outputs.
pub(in crate::code_rule_dsl) fn compile(stmts: &[Stmt], outputs: &[&str]) -> Result<Compiled, String> {
    let mut low = Lowering {
        program: BlockProgram::default(),
        seen: HashMap::new(),
        products: HashMap::new(),
        classes: HashMap::new(),
        names: HashMap::new(),
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
    for stmt in stmts {
        match stmt {
            Stmt::Lexer(_) | Stmt::Class(..) => {}
            Stmt::Mask(name, expr) => {
                let value = low.expr(expr)?;
                low.names.insert(name.clone(), value);
            }
            other => return Err(format!("not compiled yet: {}", statement(other))),
        }
    }
    for name in outputs {
        let value = low.name(name)?;
        low.program.outputs.push(value);
    }
    let program = live(&low.program);
    Ok(Compiled { rows: rows(&program) })
}

impl Lowering {
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

    fn name(&mut self, name: &str) -> Result<u16, String> {
        if let Some(&v) = self.names.get(name) {
            return Ok(v);
        }
        let ranges = self.classes.get(name).cloned().ok_or_else(|| format!("undefined: {name}"))?;
        let value = self.class(&ranges);
        self.names.insert(name.to_owned(), value);
        Ok(value)
    }

    /// A class: an `eq` for one byte, otherwise one `class` per table its products use, ORed.
    fn class(&mut self, ranges: &[(u32, u32)]) -> u16 {
        let bytes: Vec<u8> = (0..=255u8).filter(|&b| ranges.iter().any(|&(lo, hi)| lo <= u32::from(b) && u32::from(b) <= hi)).collect();
        match bytes.as_slice() {
            [] => return self.op(BlockOp::Const(0)),
            [b] => return self.op(BlockOp::Eq(*b)),
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
        value.expect("a class of two bytes or more has a product")
    }

    fn expr(&mut self, e: &Expr) -> Result<u16, String> {
        Ok(match e {
            Expr::Name(name) => self.name(name)?,
            Expr::Empty => self.op(BlockOp::Const(0)),
            Expr::Not(a) => {
                let a = self.expr(a)?;
                self.op(BlockOp::Not(a))
            }
            // `a & ~b` is one `andnot`.
            Expr::And(a, b) => match (&**a, &**b) {
                (x, Expr::Not(y)) | (Expr::Not(y), x) => {
                    let (x, y) = (self.expr(x)?, self.expr(y)?);
                    self.op(BlockOp::AndNot(x, y))
                }
                _ => {
                    let (a, b) = (self.expr(a)?, self.expr(b)?);
                    self.op(BlockOp::And(a, b))
                }
            },
            Expr::Or(a, b) => {
                let (a, b) = (self.expr(a)?, self.expr(b)?);
                self.op(BlockOp::Or(a, b))
            }
            Expr::Xor(a, b) => {
                let (a, b) = (self.expr(a)?, self.expr(b)?);
                self.op(BlockOp::Xor(a, b))
            }
            Expr::Call(f, args) => self.call(f, args)?,
            Expr::Test { target, method, .. } => return Err(format!("not compiled yet: {target}.{method}")),
        })
    }

    fn call(&mut self, f: &str, args: &[Arg]) -> Result<u16, String> {
        let mask = |low: &mut Self, i: usize| -> Result<u16, String> {
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
        Ok(match f {
            "prev" => {
                let (a, k) = (mask(self, 0)?, number(1, 1)?);
                self.prev(a, k)?
            }
            "starts" => {
                let a = mask(self, 0)?;
                let p = self.prev(a, 1)?;
                self.op(BlockOp::AndNot(a, p))
            }
            "after" => {
                let a = mask(self, 0)?;
                let p = self.prev(a, 1)?;
                self.op(BlockOp::AndNot(p, a))
            }
            "span" => {
                let (a, k) = (mask(self, 0)?, number(1, 1)?);
                let mut out = a;
                for j in 1..k {
                    let p = self.prev(a, j)?;
                    out = self.op(BlockOp::Or(out, p));
                }
                out
            }
            "toggle" => {
                let a = mask(self, 0)?;
                self.op(BlockOp::Toggle(a))
            }
            "after_odd_run" => {
                let a = mask(self, 0)?;
                self.op(BlockOp::AfterOddRun(a))
            }
            _ => return Err(format!("not compiled yet: {f}")),
        })
    }

    /// `prev(a, k)`: the identity for 0; one `prev` up to 63 bytes, the halo's size.
    fn prev(&mut self, a: u16, k: u64) -> Result<u16, String> {
        match k {
            0 => Ok(a),
            1..=63 => Ok(self.op(BlockOp::Prev(a, k as u8))),
            _ => Err(format!("prev by {k}: more than 63 bytes is not compiled yet")),
        }
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
    let mut out = BlockProgram { tables: program.tables.clone(), ..BlockProgram::default() };
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
    out
}

fn statement(stmt: &Stmt) -> String {
    let text = format!("{stmt:?}");
    text.split(['(', ' ', '{']).next().unwrap_or("?").to_owned()
}
