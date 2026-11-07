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
//! - **Counts** as bit-planes, and `C.frame(a)` (below).
//! - **Tokens**, `keywords` and `skip`: each token kind's start and last-byte masks, and the
//!   coverage and overlap checks.
//! - **The tree section** (`docs/parser-generators/13-tape-on-the-gpu.md` §3.1): its masks, and
//!   its `node` and `code` rows, with grammar ids from `Symbols`.
//! - **Errors and escapes.** Each mask joins the fallback output (`FALLBACK`); a file with a bit
//!   of it goes to tree-sitter.
//!
//! Anything else is refused by name.
//!
//! # Counts and frames
//!
//! A count `count(up, down) max M` is kept as `w` bit-planes of its value after each byte, with
//! `2^w >= M + 2`. Bit `k` flips at an up byte whose bits below `k` were all 1, and at a down
//! byte whose bits below `k` were all 0, so plane `k` is a `toggle` of a mask of the planes
//! below it one byte back: no new operation. The value is kept mod `2^w`; a count that leaves
//! `0..=M` passes through `M + 1` or `-1`, both above `M` as `w`-bit numbers, and one compare
//! escapes the file. (The evaluator holds a count at 0 instead: a file that goes below 0 escapes
//! here and is lexed there; such a file is malformed for JSON.)
//!
//! `C.frame(a)` is one `fill` per level `L` in `1..=M` (`10-bit-programs.md` §2): from the
//! openers at level `L` in `a` to the next opener at level `L` not in `a`, kept where the count
//! before the byte is `L`.
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

use super::block::{BlockOp, BlockProgram, ClassTable, CodeRow, Fact, KeywordRow, NodeRow, RowEnd, RowStart, Validator};
use ::bitlex::syntax::{Arg, Cond, Expr, FactWhen, Nest, NodeEnd, NodeStart, Stmt};
use std::collections::HashMap;

/// A compiled program: its `rir` rows (`block 0 …`), whose outputs are the requested masks in
/// order.
#[derive(Debug)]
pub(in crate::code_rule_dsl) struct Compiled {
    pub(super) rows: String,
    /// Each requested name, then each row's mask (`node KIND start`, `node KIND end`,
    /// `level KIND up`, `level KIND down`, `code KIND NAME`), and its output: a value asked for
    /// twice is one output.
    pub(super) labels: Vec<(String, u16)>,
    /// The fallback's outputs (`FALLBACK`): one per delay its masks have, none brought to
    /// another delay; the file goes to tree-sitter when any has a bit.
    pub(super) fallbacks: Vec<u16>,
}

/// The grammar's ids for the tree section's rows.
pub(in crate::code_rule_dsl) struct Symbols<'a> {
    /// A node kind's id (named first, then anonymous), or a supertype's.
    pub(super) kind: &'a dyn Fn(&str) -> Option<u16>,
    pub(super) field: &'a dyn Fn(&str) -> Option<u16>,
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
    /// The bytes it holds, when it is a byte class at delay 0 (a function of the byte alone):
    /// two classes combined are one class, a single bitmask (decision 0047).
    set: Option<ByteSet>,
}

/// A set of bytes, bit `b` for byte `b`.
type ByteSet = [u64; 4];

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
    counts: HashMap<String, Count>,
    tokens: Vec<(String, Val, Option<TokenEnd>)>,
    keywords: Vec<(String, Vec<(Vec<u8>, Option<String>)>, bool)>,
    skip: Option<Val>,
    tree: Tree,
}

/// A tree row before outputs are numbered: its masks as indexes into the compiled values.
struct RowSpec {
    kind: String,
    start: SpecStart,
    end: SpecEnd,
    leaf: bool,
    nested: Option<u32>,
    when: Option<Cond>,
}

enum SpecStart {
    At(usize),
    First,
}

enum SpecEnd {
    At(usize),
    With(String),
    Last,
}

#[derive(Clone)]
enum FactSpec {
    At(usize),
    StartOf(String),
    NotStartOf(String),
    After(Vec<String>),
    Always,
}

/// The rows of the nodes of `kind`: its own row; a supertype's kinds' rows; a keyword's token
/// row (a keyword is a token of that row, retyped).
fn kind_rows(specs: &[RowSpec], facts: &[(bool, String, Vec<String>, TreeFact)], keywords: &[(String, Vec<(Vec<u8>, Option<String>)>, bool)], kind: &str) -> Result<Vec<usize>, String> {
    if let Some(r) = specs.iter().position(|r| r.kind == kind) {
        return Ok(vec![r]);
    }
    for (token, words, _) in keywords {
        let named = |(w, alias): &(Vec<u8>, Option<String>)| alias.as_deref().map_or(w.as_slice() == kind.as_bytes(), |a| a == kind);
        if words.iter().any(named) {
            return kind_rows(specs, facts, keywords, token);
        }
    }
    let members: Vec<&String> = facts.iter().filter(|f| !f.0 && f.1 == kind).flat_map(|f| f.2.iter()).collect();
    if members.is_empty() {
        return Err(format!("no node kind, keyword or supertype `{kind}`"));
    }
    let mut out = Vec::new();
    for m in members {
        for r in kind_rows(specs, facts, keywords, m)? {
            if !out.contains(&r) {
                out.push(r);
            }
        }
    }
    Ok(out)
}

/// How a token ends: through a run of a mask (a `fill`), or at the first bit of its last-byte
/// mask (tier 1: no carry beyond a byte).
#[derive(Clone, Copy)]
enum TokenEnd {
    Through(Val),
    Last(Val),
}

/// A count: its up and down bytes (an up wins at a byte that is both), its bound and its planes
/// after each byte.
#[derive(Clone)]
struct Count {
    up: Val,
    down: Val,
    max: u32,
    planes: Vec<Val>,
}

/// The tree section, as read (decision 0049: tier-2 rules on the token stream).
#[derive(Default)]
struct Tree {
    nodes: Vec<TreeNode>,
    facts: Vec<(bool, String, Vec<String>, TreeFact)>,
    checks: Vec<(String, Validator)>,
    extra: Vec<String>,
    untoken: Vec<String>,
}

/// A `node` statement, its masks lowered.
struct TreeNode {
    kind: String,
    /// A mask, another kind's starts (by name), or the first token.
    start: Result<Option<Val>, String>,
    when: Option<Cond>,
    /// A mask, `with` a field, or the last token.
    end: NodeEnd,
    end_mask: Option<Val>,
    nested: Option<u32>,
}

/// A code fact's test, its mask lowered.
enum TreeFact {
    Mask(Val),
    StartOf(String),
    NotStartOf(String),
    Cond(Cond),
    Always,
}

/// Compile `stmts`, with `outputs` (mask names, or `FALLBACK`) as the block program's first
/// outputs, then the tree section's row masks. `symbols` names the grammar's ids; a program
/// with a tree section needs it.
pub(in crate::code_rule_dsl) fn compile(stmts: &[Stmt], outputs: &[&str], symbols: Option<&Symbols<'_>>) -> Result<Compiled, String> {
    let mut low = Lowering {
        program: BlockProgram::default(),
        seen: HashMap::new(),
        products: HashMap::new(),
        classes: HashMap::new(),
        names: HashMap::new(),
        fallbacks: Vec::new(),
        counts: HashMap::new(),
        tokens: Vec::new(),
        keywords: Vec::new(),
        skip: None,
        tree: Tree::default(),
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
    let token_rows = low.tokens()?;
    let mut vals: Vec<(String, Val)> = Vec::new();
    for name in outputs {
        if *name == FALLBACK {
            for x in low.fallback()? {
                vals.push((FALLBACK.to_owned(), x));
            }
        } else {
            let x = low.name(name)?;
            vals.push(((*name).to_owned(), x));
        }
    }
    // The rows: declared kinds in rank order, then the tokens; each mask an output.
    let has_tree = !low.tree.nodes.is_empty();
    let push = |vals: &mut Vec<(String, Val)>, label: String, x: Val| {
        vals.push((label, x));
        vals.len() - 1
    };
    let mut specs: Vec<RowSpec> = Vec::new();
    let mut facts: Vec<(usize, bool, String, FactSpec)> = Vec::new();
    let mut skip = None;
    if has_tree {
        let untoken = low.tree.untoken.clone();
        let token_rows: Vec<(String, Val, Val)> = token_rows.into_iter().filter(|t| !untoken.contains(&t.0)).collect();
        let declared = std::mem::take(&mut low.tree.nodes);
        // `start K`: kind K's own start mask.
        let start_of = |name: &str| -> Result<Val, String> {
            if let Some(t) = token_rows.iter().find(|t| t.0 == name) {
                return Ok(t.1);
            }
            match declared.iter().find(|n| n.kind == name).map(|n| &n.start) {
                Some(Ok(Some(v))) => Ok(*v),
                _ => Err(format!("`start {name}`: no mask, token or node kind of that name")),
            }
        };
        for n in &declared {
            let start = match &n.start {
                Ok(None) => SpecStart::First,
                Ok(Some(v)) => SpecStart::At(push(&mut vals, format!("node {} start", n.kind), *v)),
                Err(name) => SpecStart::At(push(&mut vals, format!("node {} start", n.kind), start_of(name)?)),
            };
            let end = match (&n.end, n.end_mask) {
                (NodeEnd::At(_), Some(m)) => SpecEnd::At(push(&mut vals, format!("node {} end", n.kind), m)),
                (NodeEnd::With(f), _) => SpecEnd::With(f.clone()),
                _ => SpecEnd::Last,
            };
            specs.push(RowSpec { kind: n.kind.clone(), start, end, leaf: false, nested: n.nested, when: n.when.clone() });
        }
        for (kind, start, last) in &token_rows {
            let s = push(&mut vals, format!("node {kind} start"), *start);
            let e = push(&mut vals, format!("node {kind} end"), *last);
            specs.push(RowSpec { kind: kind.clone(), start: SpecStart::At(s), end: SpecEnd::At(e), leaf: true, nested: None, when: None });
        }
        let tree_facts = std::mem::take(&mut low.tree.facts);
        let rows_of = |kind: &str| kind_rows(&specs, &tree_facts, &low.keywords, kind);
        for (field, name, kinds, fact) in &tree_facts {
            let spec = match fact {
                TreeFact::Mask(m) => FactSpec::At(push(&mut vals, format!("code {name}"), *m)),
                TreeFact::StartOf(k) => FactSpec::StartOf(k.clone()),
                TreeFact::NotStartOf(k) => FactSpec::NotStartOf(k.clone()),
                TreeFact::Cond(c) if c.parent.is_none() => FactSpec::After(c.after.clone()),
                TreeFact::Cond(_) => return Err(format!("{name}: a fact's condition is `after` only")),
                TreeFact::Always => FactSpec::Always,
            };
            let mut rows: Vec<usize> = Vec::new();
            for kind in kinds {
                for r in rows_of(kind)? {
                    if !rows.contains(&r) {
                        rows.push(r);
                    }
                }
            }
            for r in rows {
                facts.push((r, *field, name.clone(), spec.clone()));
            }
        }
        low.tree.facts = tree_facts;
        if let Some(sk) = low.skip {
            skip = Some(push(&mut vals, "skip".to_owned(), sk));
        }
    }
    let delay = vals.iter().map(|x| x.1.d).max().unwrap_or(0);
    // Each output once.
    let mut index: HashMap<u16, u16> = HashMap::new();
    let mut labels = Vec::new();
    let mut at = Vec::new();
    for (label, x) in &vals {
        // Outputs hold no bits past the end: a file falls back on a bit of its own.
        let x = if x.dirty {
            let inside = low.within_val(x.d)?;
            low.and(*x, inside)?
        } else {
            *x
        };
        // At its own delay: the host shifts each output by its own (decision 0047).
        let v = x.v;
        let k = *index.entry(v).or_insert_with(|| {
            low.program.outputs.push(v);
            low.program.output_delays.push(x.d);
            (low.program.outputs.len() - 1) as u16
        });
        labels.push((label.clone(), k));
        at.push(low.program.outputs[usize::from(k)]);
    }
    if has_tree {
        let symbols = symbols.ok_or("a tree section needs the grammar's ids")?;
        let id = |name: &str| (symbols.kind)(name).ok_or_else(|| format!("no kind `{name}` in the grammar"));
        let field_id = |name: &str| (symbols.field)(name).ok_or_else(|| format!("no field `{name}`"));
        let leaf_rows = |kinds: &[String]| -> Result<Vec<u16>, String> {
            kinds
                .iter()
                .map(|k| specs.iter().position(|r| r.leaf && r.kind == *k).map(|r| r as u16).ok_or_else(|| format!("`after {k}`: no token of that kind")))
                .collect()
        };
        let row_of = |k: &str| specs.iter().position(|r| r.kind == k).map(|r| r as u16).ok_or_else(|| format!("no node kind `{k}`"));
        for spec in &specs {
            let (after, parent) = match &spec.when {
                Some(c) => (leaf_rows(&c.after)?, c.parent.as_deref().map(row_of).transpose()?),
                None => (Vec::new(), None),
            };
            low.program.nodes.push(NodeRow {
                kind: id(&spec.kind)?,
                start: match spec.start {
                    SpecStart::At(v) => RowStart::At(at[v]),
                    SpecStart::First => RowStart::FirstToken,
                },
                end: match &spec.end {
                    SpecEnd::At(v) => RowEnd::At(at[*v]),
                    SpecEnd::With(f) => RowEnd::With(field_id(f)?),
                    SpecEnd::Last => RowEnd::LastToken,
                },
                extra: low.tree.extra.contains(&spec.kind),
                leaf: spec.leaf,
                nested: spec.nested.map(|d| d.min(63) as u8),
                after,
                parent,
                check: None,
            });
        }
        for (kind, v) in low.tree.checks.clone() {
            for r in kind_rows(&specs, &low.tree.facts, &low.keywords, &kind)? {
                low.program.nodes[r].check = Some((id(&kind)?, v));
            }
        }
        for (row, field, name, spec) in facts {
            let fid = if field { field_id(&name)? } else { id(&name)? };
            let when = match spec {
                FactSpec::At(w) => Fact::At(at[w]),
                FactSpec::StartOf(k) => Fact::StartOf(row_of(&k)?),
                FactSpec::NotStartOf(k) => Fact::NotStartOf(row_of(&k)?),
                FactSpec::After(ks) => Fact::After(leaf_rows(&ks)?),
                FactSpec::Always => Fact::Always,
            };
            low.program.codes.push(CodeRow { node: row as u16, field, id: fid, when });
        }
        for (kind, words, required) in low.keywords.clone() {
            if required {
                return Err(format!("keywords {kind}: with a tree section a token that is no keyword keeps its kind; drop `required` and check it (decision 0049)"));
            }
            let node = specs.iter().position(|r| r.leaf && r.kind == kind).ok_or_else(|| format!("keywords {kind}: no token of that kind"))? as u16;
            for (word, alias) in words {
                let name = alias.unwrap_or_else(|| String::from_utf8_lossy(&word).into_owned());
                low.program.keywords.push(KeywordRow { node, word, kind: id(&name)? });
            }
        }
        low.program.skip = skip.map(|k| at[k]);
    }
    low.program.delay = delay;
    let program = live(&low.program);
    let fallbacks = labels.iter().filter(|(l, _)| l == FALLBACK).map(|&(_, k)| k).collect();
    Ok(Compiled { rows: rows(&program), labels, fallbacks })
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
                Stmt::Count { name, up, down, reset, max } => {
                    if reset.is_some() {
                        return Err(format!("count {name}: `reset` is not compiled yet"));
                    }
                    let (up, down) = (self.expr(up)?, self.expr(down)?);
                    let count = self.count(up, down, *max)?;
                    self.counts.insert(name.clone(), count);
                }
                Stmt::Token { kind, start, through, last } => {
                    let start = self.expr(start)?;
                    let through = match through {
                        Some(t) => Some(TokenEnd::Through(self.expr(t)?)),
                        None => match last {
                            Some(l) => Some(TokenEnd::Last(self.expr(l)?)),
                            None => None,
                        },
                    };
                    self.tokens.push((kind.clone(), start, through));
                }
                Stmt::Keywords { kind, words, soft, required } => {
                    if !soft.is_empty() {
                        return Err(format!("keywords {kind}: soft keywords are not compiled yet"));
                    }
                    self.keywords.push((kind.clone(), words.clone(), *required));
                }
                Stmt::Skip(expr) => {
                    let x = self.expr(expr)?;
                    self.skip = Some(match self.skip {
                        Some(s) => self.or(s, x)?,
                        None => x,
                    });
                }
                Stmt::Tree { body, .. } => self.stmts(body)?,
                Stmt::Node { kind, start, when, end, level } => {
                    let start = match start {
                        NodeStart::FirstToken => Ok(None),
                        // Another node kind's starts, resolved with the rows.
                        NodeStart::At(Expr::Name(n)) if !self.names.contains_key(n) && !self.classes.contains_key(n) => Err(n.clone()),
                        NodeStart::At(e) => Ok(Some(self.expr(e)?)),
                    };
                    let end_mask = match end {
                        NodeEnd::At(e) => Some(self.expr(e)?),
                        _ => None,
                    };
                    let nested = match level {
                        None => None,
                        Some(Nest::Nested(max)) => Some(max.unwrap_or(63)),
                        Some(Nest::Level(_)) => return Err(format!("node {kind}: a count level is not compiled; nest it on the writer's stack (`nested`, decision 0049)")),
                    };
                    self.tree.nodes.push(TreeNode { kind: kind.clone(), start, when: when.clone(), end: end.clone(), end_mask, nested });
                }
                Stmt::Fact { field, name, kinds, expr } => {
                    let fact = match expr {
                        FactWhen::Mask(e) => TreeFact::Mask(self.expr(e)?),
                        FactWhen::StartOf(k) => TreeFact::StartOf(k.clone()),
                        FactWhen::NotStartOf(k) => TreeFact::NotStartOf(k.clone()),
                        FactWhen::Cond(c) => TreeFact::Cond(c.clone()),
                        FactWhen::Always => TreeFact::Always,
                    };
                    self.tree.facts.push((*field, name.clone(), kinds.clone(), fact));
                }
                Stmt::Check { kind, validator } => {
                    let v = Validator::named(validator).ok_or_else(|| format!("check {kind}: no built-in validator `{validator}`"))?;
                    self.tree.checks.push((kind.clone(), v));
                }
                Stmt::Extra(kinds) => self.tree.extra.extend(kinds.iter().cloned()),
                Stmt::Untoken(kinds) => self.tree.untoken.extend(kinds.iter().cloned()),
                other => return Err(format!("not compiled yet: {}", statement(other))),
            }
        }
        Ok(())
    }

    /// The fallback masks, ORed per delay (`empty` without any): a mask at one delay is never
    /// brought to another, since the fallback asks only whether any bit is set.
    fn fallback(&mut self) -> Result<Vec<Val>, String> {
        let mut by_delay: Vec<Val> = Vec::new();
        for x in self.fallbacks.clone() {
            match by_delay.iter_mut().find(|y| y.d == x.d) {
                Some(y) => *y = self.or(*y, x)?,
                None => by_delay.push(x),
            }
        }
        if by_delay.is_empty() {
            by_delay.push(self.leaf(BlockOp::Const(0), false));
        }
        by_delay.sort_by_key(|x| x.d);
        Ok(by_delay)
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
        Val { v: self.op(op), d: 0, dirty, set: None }
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
            v = self.prev_op(v, k);
            left -= k;
        }
        Ok(v)
    }

    /// `prev(v, k)`, folded into `v`'s own `prev` when it has one: `prev(prev(x, a), k)` is
    /// `prev(x, a + k)`, on `x`'s carried word.
    fn prev_op(&mut self, v: u16, k: u8) -> u16 {
        match self.program.ops[usize::from(v)] {
            BlockOp::Prev(x, a) if a + k <= MAX_DELAY => self.op(BlockOp::Prev(x, a + k)),
            _ => self.op(BlockOp::Prev(v, k)),
        }
    }

    /// `within`, the stream's bytes, at delay `d`: a leaf, never a carry.
    fn within(&mut self, d: u8) -> Result<u16, String> {
        if d > MAX_DELAY {
            return Err(format!("a delay of {d} bytes: more than {MAX_DELAY}"));
        }
        Ok(self.op(BlockOp::Within(d)))
    }

    fn two(&mut self, a: Val, b: Val, f: fn(u16, u16) -> BlockOp, dirty: bool) -> Result<Val, String> {
        let d = a.d.max(b.d);
        let (x, y) = (self.at(a, d)?, self.at(b, d)?);
        Ok(Val { v: self.op(f(x, y)), d, dirty, set: None })
    }

    /// Two byte classes combined by `f`, bit by bit over their sets: one class.
    fn sets(&mut self, a: Val, b: Val, f: fn(u64, u64) -> u64) -> Option<Val> {
        let (x, y) = (a.set?, b.set?);
        Some(self.class_set(std::array::from_fn(|i| f(x[i], y[i]))))
    }

    fn and(&mut self, a: Val, b: Val) -> Result<Val, String> {
        if let Some(c) = self.sets(a, b, |x, y| x & y) {
            return Ok(c);
        }
        self.two(a, b, BlockOp::And, a.dirty && b.dirty)
    }

    fn or(&mut self, a: Val, b: Val) -> Result<Val, String> {
        if let Some(c) = self.sets(a, b, |x, y| x | y) {
            return Ok(c);
        }
        self.two(a, b, BlockOp::Or, a.dirty || b.dirty)
    }

    fn xor(&mut self, a: Val, b: Val) -> Result<Val, String> {
        if let Some(c) = self.sets(a, b, |x, y| x ^ y) {
            return Ok(c);
        }
        self.two(a, b, BlockOp::Xor, a.dirty || b.dirty)
    }

    /// `a & ~b`: no bits outside the stream that `a` has none of.
    fn andnot(&mut self, a: Val, b: Val) -> Result<Val, String> {
        if let Some(c) = self.sets(a, b, |x, y| x & !y) {
            return Ok(c);
        }
        self.two(a, b, BlockOp::AndNot, a.dirty)
    }

    /// `~a`, inside the stream only. A class's complement is a class, dirty when it holds the
    /// padding's 0x00.
    fn not(&mut self, a: Val) -> Result<Val, String> {
        if let Some(x) = a.set {
            return Ok(self.class_set(x.map(|w| !w)));
        }
        let w = self.within(a.d)?;
        Ok(Val { v: self.op(BlockOp::AndNot(w, a.v)), d: a.d, dirty: false, set: None })
    }

    /// `prev(a, k)`. When `a` is `k` or more bytes late, its bits already stand for the bytes
    /// `k` earlier at a delay `k` smaller: no operation. Its bits for the bytes before the stream
    /// are clean (`next` cleans them), so the bytes shifted in are 0, as a carry's.
    fn prev(&mut self, a: Val, k: u64) -> Result<Val, String> {
        if u64::from(a.d) >= k {
            return Ok(Val { d: a.d - k as u8, set: None, ..a });
        }
        let mut v = a.v;
        let mut left = k;
        while left > 0 {
            let step = left.min(63) as u8;
            v = self.prev_op(v, step);
            left -= u64::from(step);
        }
        Ok(Val { v, set: None, ..a })
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
        Ok(Val { v, d, dirty: false, set: None })
    }

    /// A carried operation of one operand: its state may run on past the end.
    fn carried(&mut self, a: Val, f: fn(u16) -> BlockOp) -> Val {
        Val { v: self.op(f(a.v)), d: a.d, dirty: true, set: None }
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

    /// A class of byte ranges (`class_set`).
    fn class(&mut self, ranges: &[(u32, u32)]) -> Val {
        let mut set = [0u64; 4];
        for &(lo, hi) in ranges {
            for b in lo..=hi.min(255) {
                set[b as usize / 64] |= 1 << (b % 64);
            }
        }
        self.class_set(set)
    }

    /// A class: an `eq` for one byte, otherwise one `class` per table its products use, ORed.
    /// Dirty when the class holds the padding's byte, 0x00.
    fn class_set(&mut self, set: ByteSet) -> Val {
        let has = |b: u32| set[b as usize / 64] >> (b % 64) & 1 == 1;
        let bytes: Vec<u8> = (0..=255u8).filter(|&b| has(u32::from(b))).collect();
        let dirty = has(0);
        match bytes.as_slice() {
            [] => return Val { set: Some(set), ..self.leaf(BlockOp::Const(0), false) },
            [b] => return Val { set: Some(set), ..self.leaf(BlockOp::Eq(*b), dirty) },
            _ => {}
        }
        // The set as ranges of consecutive bytes, for its products.
        let mut ranges: Vec<(u32, u32)> = Vec::new();
        for &b in &bytes {
            match ranges.last_mut() {
                Some((_, hi)) if *hi + 1 == u32::from(b) => *hi = u32::from(b),
                _ => ranges.push((u32::from(b), u32::from(b))),
            }
        }
        let mut per_table: Vec<(u16, u8)> = Vec::new();
        for product in products(&ranges) {
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
        Val { v: value.expect("a class of two bytes or more has a product"), d: 0, dirty, set: Some(set) }
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
            Expr::Test { target, method, arg: Some(arg), .. } if method == "frame" => {
                let count = self.counts.get(target).cloned().ok_or_else(|| format!("{target}.frame: `{target}` is not a count"))?;
                let a = self.expr(arg)?;
                self.frame(&count, a)
            }
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
            // simdjson's escaped bytes; `after_odd_run` is the ones outside the run.
            "escaped" => {
                let a = mask(self, 0)?;
                Ok(self.carried(a, BlockOp::Escaped))
            }
            "after_odd_run" => {
                let a = mask(self, 0)?;
                let escaped = self.carried(a, BlockOp::Escaped);
                self.andnot(escaped, a)
            }
            "fill" => {
                let (start, end) = (mask(self, 0)?, mask(self, 1)?);
                self.fill(start, end)
            }
            _ => Err(format!("not compiled yet: {f}")),
        }
    }

    /// A count's planes (see the module's notes), and its range check as a fallback.
    fn count(&mut self, up: Val, down: Val, max: u32) -> Result<Count, String> {
        let down = self.andnot(down, up)?;
        let mut w = 1u32;
        while (1u64 << w) < u64::from(max) + 2 {
            w += 1;
        }
        let mut planes: Vec<Val> = Vec::new();
        let mut before: Vec<Val> = Vec::new();
        for k in 0..w as usize {
            let flips = if k == 0 {
                self.or(up, down)?
            } else {
                // All of the planes below k were 1 (an up carries into k), or all 0 (a down
                // borrows from it).
                let mut ones = before[0];
                let mut any = before[0];
                for &b in &before[1..k] {
                    ones = self.and(ones, b)?;
                    any = self.or(any, b)?;
                }
                let carry = self.and(up, ones)?;
                let borrow = self.andnot(down, any)?;
                self.or(carry, borrow)?
            };
            let plane = self.carried(flips, BlockOp::Toggle);
            before.push(self.prev(plane, 1)?);
            planes.push(plane);
        }
        let over = self.above(&planes, u64::from(max))?;
        self.fallbacks.push(over);
        Ok(Count { up, down, max, planes })
    }

    /// The bytes where the value in `planes` is above `m` (as a `planes.len()`-bit number).
    fn above(&mut self, planes: &[Val], m: u64) -> Result<Val, String> {
        let mut out = self.leaf(BlockOp::Const(0), false);
        // From the top bit down: above at bit k when the bits above k equal m's, bit k is 1 and
        // m's bit k is 0.
        let mut equal: Option<Val> = None;
        for k in (0..planes.len()).rev() {
            let p = planes[k];
            if m >> k & 1 == 0 {
                let here = match equal {
                    Some(e) => self.and(e, p)?,
                    None => p,
                };
                out = self.or(out, here)?;
                equal = Some(match equal {
                    Some(e) => self.andnot(e, p)?,
                    None => self.not(p)?,
                });
            } else {
                equal = Some(match equal {
                    Some(e) => self.and(e, p)?,
                    None => p,
                });
            }
        }
        Ok(out)
    }

    /// The bytes where the value in `planes` is `m`.
    fn equals(&mut self, planes: &[Val], m: u64) -> Result<Val, String> {
        let mut out: Option<Val> = None;
        for (k, &p) in planes.iter().enumerate() {
            out = Some(match (out, m >> k & 1 == 1) {
                (None, true) => p,
                (None, false) => self.not(p)?,
                (Some(e), true) => self.and(e, p)?,
                (Some(e), false) => self.andnot(e, p)?,
            });
        }
        Ok(out.expect("a count has a plane"))
    }

    /// `C.frame(a)`: the frame open before each byte was opened by a byte of `a`. Per level
    /// `L`, the openers there (`up` where the count after is `L`) fill from those in `a` to the
    /// next one not in `a`; a byte reads its level's fill when the count before it is `L`.
    fn frame(&mut self, count: &Count, a: Val) -> Result<Val, String> {
        let before: Vec<Val> = count.planes.iter().map(|&p| self.prev(p, 1)).collect::<Result<_, _>>()?;
        let mut out = self.leaf(BlockOp::Const(0), false);
        for level in 1..=u64::from(count.max) {
            let after = self.equals(&count.planes, level)?;
            let openers = self.and(count.up, after)?;
            let from = self.and(openers, a)?;
            let to = self.andnot(openers, a)?;
            let open = self.fill(from, to)?;
            let here = self.equals(&before, level)?;
            let kept = self.and(here, open)?;
            out = self.or(out, kept)?;
        }
        Ok(out)
    }

    /// The tokens: each kind's start and last-byte masks (keywords retyped), and the overlap,
    /// coverage and keyword checks as fallbacks.
    fn tokens(&mut self) -> Result<Vec<(String, Val, Val)>, String> {
        if self.tokens.is_empty() {
            return Ok(Vec::new());
        }
        let tokens = std::mem::take(&mut self.tokens);
        // With a tree section the tape writer checks, once per token, that the tokens tile the
        // file and that no two start at one byte, and retypes keywords (decision 0049): tier 1
        // keeps each token's start and last byte only.
        let tier2 = !self.tree.nodes.is_empty();
        if tier2 {
            let mut out = Vec::new();
            for (kind, start, end) in tokens {
                let last = match end {
                    None => start,
                    Some(TokenEnd::Last(l)) => l,
                    Some(TokenEnd::Through(_)) => return Err(format!("token {kind}: `through` needs a fill; with a tree section write its last byte (`last`, decision 0049)")),
                };
                out.push((kind, start, last));
            }
            return Ok(out);
        }
        // Two kinds starting at one byte.
        let mut starts = tokens[0].1;
        let mut overlap = self.leaf(BlockOp::Const(0), false);
        for t in &tokens[1..] {
            let both = self.and(starts, t.1)?;
            overlap = self.or(overlap, both)?;
            starts = self.or(starts, t.1)?;
        }
        self.fallbacks.push(overlap);
        let mut covered = self.leaf(BlockOp::Const(0), false);
        let mut out = Vec::new();
        for (kind, start, through) in tokens {
            // A token runs from its start through `through`'s bytes, up to the next start.
            let span = match through.map(|t| match t {
                TokenEnd::Through(t) => t,
                TokenEnd::Last(l) => l,
            }) {
                Some(t) if matches!(through, Some(TokenEnd::Through(_))) => {
                    let outside = self.not(t)?;
                    let stop = self.or(outside, starts)?;
                    let run = self.fill(start, stop)?;
                    let inside = self.within_val(run.d)?;
                    self.and(run, inside)?
                }
                // A token with a last-byte mask: its bytes are not a mask here (no fill); it is
                // covered from its start only.
                _ => start,
            };
            covered = self.or(covered, span)?;
            // A token's last byte: the last of its span before a byte that does not continue
            // it. Without `through` the span is the start byte alone, which is the last.
            let last = match through {
                None => start,
                Some(TokenEnd::Last(l)) => l,
                Some(TokenEnd::Through(_)) => {
                    let inner = self.andnot(span, starts)?;
                    let continues = self.next(inner, 1)?;
                    self.andnot(span, continues)?
                }
            };
            match self.keywords.iter().position(|k| k.0 == kind) {
                None => out.push((kind, start, last)),
                Some(at) => {
                    let (_, words, required) = self.keywords[at].clone();
                    let mut any = self.leaf(BlockOp::Const(0), false);
                    for (word, alias) in words {
                        let here = self.keyword(start, last, &word)?;
                        any = self.or(any, here)?;
                        let back = self.prev(here, word.len() as u64 - 1)?;
                        let word_last = self.and(last, back)?;
                        let name = alias.unwrap_or_else(|| String::from_utf8_lossy(&word).into_owned());
                        out.push((name, here, word_last));
                    }
                    let other = self.andnot(start, any)?;
                    if !required {
                        return Err(format!("keywords {kind}: only `required` keywords are compiled yet"));
                    }
                    self.fallbacks.push(other);
                }
            }
        }
        let skipped = match self.skip {
            Some(s) => self.or(covered, s)?,
            None => covered,
        };
        let gap = self.not(skipped)?;
        self.fallbacks.push(gap);
        Ok(out)
    }

    /// `within` as a value at delay `d`.
    fn within_val(&mut self, d: u8) -> Result<Val, String> {
        Ok(Val { v: self.within(d)?, d, dirty: false, set: None })
    }

    /// The starts of tokens whose text is `word`: the word's bytes from the start, and the
    /// token's first last byte at its length.
    ///
    /// Each term is brought to the word's delay once, from its own value: the carried words are
    /// those of `start`, `last` and the letters, shared by every keyword (a chain, each step
    /// aligned in turn, would carry a new value per letter). The terms are not cleaned before
    /// the stream: `start`, ANDed with each, is 0 there.
    fn keyword(&mut self, start: Val, last: Val, word: &[u8]) -> Result<Val, String> {
        if word.is_empty() {
            return Err("an empty keyword".into());
        }
        let later = |x: Val, j: usize| Val { d: x.d + j as u8, set: None, ..x };
        let mut terms: Vec<(Val, bool)> = Vec::new();
        for (j, &b) in word.iter().enumerate() {
            terms.push((later(self.leaf(BlockOp::Eq(b), b == 0), j), true));
            terms.push((later(last, j), j + 1 == word.len()));
        }
        let d = terms.iter().map(|t| t.0.d).max().unwrap_or(0).max(start.d);
        if d > MAX_DELAY {
            return Err(format!("keyword {}: a delay of {d} bytes", String::from_utf8_lossy(word)));
        }
        let mut out = self.at(start, d)?;
        for (t, holds) in terms {
            let x = self.at(t, d)?;
            out = self.op(if holds { BlockOp::And(out, x) } else { BlockOp::AndNot(out, x) });
        }
        Ok(Val { v: out, d, dirty: start.dirty, set: None })
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
    let n = |v: u16| number[usize::from(v)];
    out.outputs = program.outputs.iter().map(|&v| n(v)).collect();
    out.output_delays = program.output_delays.clone();
    out.nodes = program
        .nodes
        .iter()
        .map(|r| NodeRow {
            start: match r.start {
                RowStart::At(v) => RowStart::At(n(v)),
                other => other,
            },
            end: match r.end {
                RowEnd::At(v) => RowEnd::At(n(v)),
                other => other,
            },
            ..r.clone()
        })
        .collect();
    out.codes = program
        .codes
        .iter()
        .map(|c| CodeRow {
            when: match &c.when {
                Fact::At(v) => Fact::At(n(*v)),
                other => other.clone(),
            },
            ..c.clone()
        })
        .collect();
    out.keywords = program.keywords.clone();
    out.skip = program.skip.map(n);
    out
}

fn rows(program: &BlockProgram) -> String {
    let mut out = String::new();
    super::rir::block_rows(&mut out, 0, program);
    out
}

fn statement(stmt: &Stmt) -> String {
    let text = format!("{stmt:?}");
    text.split(['(', ' ', '{']).next().unwrap_or("?").to_owned()
}
