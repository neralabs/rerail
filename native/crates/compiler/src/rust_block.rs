//! Tier 1 as Rust source, written from the block program and its records (decision 0451), in
//! the lean lexer's two stages (`kotlin_lean`'s `block.rs` and `lex.rs`), not from the JIT's
//! Cranelift IR:
//!
//! - **Stage 1** reads the file a 64-byte block a step and keeps, a word a block, the masks the
//!   token loop and tier 2 read. A block's values are the program's operations as named `let`s,
//!   the carried ones' state in locals across the loop. The regions (strings, comments) are a
//!   walk of their own between two block loops: the first makes the walk's openers and closers,
//!   the walk goes from region to region over the file, and the second loop reads what it made.
//! - **Stage 2** is the token loop: past the blanks, the token's first byte picks its end rule,
//!   a reader reads a region, and the token's record is its kind, its word and its marks.
//!
//! Its meaning is the interpreter's (`BlockProgram::run_files`, `records::walk`, `records::pass`,
//! `records::gap_marks`), and it writes what the CPU's tier 1 writes into a `Batch`: the records,
//! the counts, and the masks tier 2 reads. The code bits of the gap marks tier 2 on the CPU tests
//! itself (`native_cpu_block::lazy_code_bits`) are left out, as the CPU's tier 1 leaves them.

use crate::block::{BlockOp, BlockProgram};
use crate::records::{CheckThen, EndRule, ReadNone, Records, WORD_ALONE};
use std::collections::HashMap;
use std::fmt::Write as _;

/// What the harness gives the generated tier 1 besides the program: the record layout its
/// `Batch` holds (`native_cpu_block::RecordTables::layout`), the code bits it leaves to tier 2,
/// and the grammar's token names by record kind (`program::Names::tokens`) and mark names by bit
/// (`program::Names::marks`).
pub struct Inputs<'a> {
    pub program: &'a BlockProgram,
    pub layout: crate::records::RecordLayout,
    pub lazy: u32,
    pub token_names: &'a [(u8, String)],
    pub mark_names: &'a [(u8, String)],
    /// The outputs the CPU's tier 1 does not make (`BlockNative::records_made`): of tier 2's
    /// masks, those tier 2 on the CPU reads from the bytes itself.
    pub unmade: Vec<usize>,
}

/// The program as Rust source: `lex(src)`, the two stages over one file, and `tier1_entry`, the
/// harness's entry (`native_cpu_block::Entry`), with its fingerprint `SOURCE` on the last line.
pub fn rust_source(inputs: &Inputs<'_>) -> Result<String, String> {
    let g = Gen::new(inputs)?;
    let mut out = String::new();
    g.header(&mut out);
    g.tables(&mut out);
    g.masks_struct(&mut out);
    g.regions_code(&mut out)?;
    g.stage1(&mut out)?;
    g.stage2(&mut out);
    g.helpers(&mut out);
    g.entry(&mut out);
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in out.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    let _ = writeln!(out, "pub const SOURCE: u64 = 0x{h:016x};");
    Ok(out)
}

struct Gen<'a> {
    p: &'a BlockProgram,
    r: &'a Records,
    inputs: &'a Inputs<'a>,
    /// Each stored output's name (an index into the program's outputs).
    stored: Vec<(usize, String)>,
    /// The outputs only the refusal reads (the totals, the untaken bytes): no mask each, but
    /// one word stage 1 ORs them into, as the lean lexer keeps one error flag.
    refusals: Vec<(usize, String)>,
    /// Each value's name in stage 1.
    value_names: Vec<String>,
    /// The token names by record kind.
    tokens: HashMap<u8, String>,
    /// The walk, if the program has one: its made values and its inputs.
    walk: Option<&'a crate::block::Walk>,
    /// Each reader's name, distinct (its token's).
    reader_names: Vec<String>,
}

fn rust_name(name: &str) -> String {
    crate::rust_stream::rust_name_of(name, false)
}

impl<'a> Gen<'a> {
    fn new(inputs: &'a Inputs<'a>) -> Result<Self, String> {
        let p = inputs.program;
        let r = p.records.as_ref().ok_or("a block program without records")?;
        // Every field of the records, named: a new one fails to build here until this emitter
        // writes it or refuses it. The emitter writes the interpreter's meaning (`records::walk`,
        // `pass`, `gap_marks`); the fields it does not read are the CPU JIT's lowering hints,
        // whose meaning the masks and retags already carry.
        let crate::records::Records {
            starts: _,
            lasts: _,
            kinds: _,
            words: _,
            marks: _,
            ends: _,
            rules: _,
            skip,
            gaps: _,
            extra: _,
            kind_rows: _,
            untaken: _,
            unless: _,
            unplaced: _,
            checks,
            readers,
            retags: _,
            nexts: _,
            tested_only: _,
            // Which code bits hold no mark (0407) and which `without` reads (0430): `lazy` and
            // the marks' masks give the code's bits.
            alone: _,
            read: _,
            // The CPU loop's reading of a gap mark from the blanks (0430): the mark's mask has
            // the same bits between tokens.
            blanks: _,
            // The marks an extra keeps of its own gap: `token()` writes them from the masks.
            own: _,
            // Tier 2 reads these from the bytes on the CPU (`native_cpu_bytes`).
            check_masks: _,
            prefix,
            gap,
        } = r;
        if skip.is_none() {
            return Err("records without a skip (`walk`): not written yet".into());
        }
        for c in checks {
            // `all` (every byte of the token) and `Cut` (0413's `run R once D`) are written below.
            let crate::records::TokenCheck { rule: _, count: _, o: _, least: _, then, all: _ } = c;
            // Each way a test ends, named: a new one fails to build here.
            match then {
                CheckThen::Escape | CheckThen::At(_) | CheckThen::After(_) | CheckThen::Cut => {}
            }
        }
        for rd in readers {
            // `retag` and the arms' are the CPU's form of the retags (`Records::retags`), which
            // the token's record reads; `quiet` is a proved hint; a split by tier 2 is refused.
            let crate::records::Reader { arms, other: _, retag: _, parts, split, quiet: _ } = rd;
            if !parts.is_empty() && *split != crate::records::Split::Tier1 {
                return Err("a region tier 2 splits (0323): not written yet".into());
            }
            for a in arms {
                let crate::records::ReadArm { opener: _, closer: _, off: _, last: _, none: _, retag: _ } = a;
            }
            for pt in parts {
                let crate::records::ReadPart { start: _, bytes: _, last: _, kind: _ } = pt;
            }
        }
        let (prefix, gap) = (prefix, gap);
        if !prefix.is_empty() || gap.is_some() || p.rows.is_some() {
            return Err("a prefix, a gap rule or rows: not written yet".into());
        }
        if p.rounds.len() != p.walks.len() {
            return Err("a fixed point that is no walk: not written yet".into());
        }
        if p.walks.len() > 1 {
            return Err("more than one walk: not written yet".into());
        }
        for op in &p.ops {
            match op {
                BlockOp::BackFill(..) | BlockOp::Pair { .. } | BlockOp::Prefix(..) | BlockOp::Phase(..) | BlockOp::Position(..) | BlockOp::Cmp { .. } | BlockOp::Bit { .. } => {
                    return Err(format!("{op:?}: not written yet"));
                }
                _ => {}
            }
        }
        let tokens: HashMap<u8, String> = inputs.token_names.iter().map(|(k, n)| (*k, rust_name(n))).collect();
        let mut g = Gen { p, r, inputs, stored: Vec::new(), refusals: Vec::new(), value_names: Vec::new(), tokens, walk: p.walks.first(), reader_names: Vec::new() };
        let mut names: Vec<String> = Vec::new();
        for i in 0..r.readers.len() {
            let n = g.rule_token(g.reader_rule(i));
            names.push(if names.contains(&n) { format!("{n}_{i}") } else { n });
        }
        g.reader_names = names;
        g.name_outputs();
        g.name_values();
        Ok(g)
    }

    /// A token's name by the first bytes whose end rule is `rule`.
    fn rule_token(&self, rule: usize) -> String {
        let b = (0..256).find(|&b| usize::from(self.r.ends.get(b).copied().unwrap_or(crate::records::NO_RULE)) == rule);
        b.and_then(|b| self.tokens.get(&self.r.kinds[b]).cloned()).unwrap_or_else(|| format!("rule{rule}"))
    }

    fn reader_rule(&self, i: usize) -> usize {
        self.r.rules.iter().position(|x| *x == EndRule::Read(i as u16)).unwrap_or(usize::MAX)
    }

    /// The outputs stage 2 and tier 2 read, each named by what reads it.
    fn name_outputs(&mut self) {
        let r = self.r;
        let mut named: Vec<(usize, String)> = Vec::new();
        let mut add = |o: usize, n: String| {
            if !named.iter().any(|x| x.0 == o) {
                named.push((o, n));
            }
        };
        add(usize::from(r.skip.expect("checked")), "blank".into());
        for (k, rule) in r.rules.iter().enumerate() {
            let t = self.rule_token(k);
            match *rule {
                EndRule::Run(o) => add(usize::from(o), format!("{t}_run")),
                EndRule::Pair(o) => add(usize::from(o), format!("{t}_pair")),
                EndRule::Last(o) => add(usize::from(o), format!("{t}_last")),
                EndRule::Read(_) | EndRule::Next(_) => {}
            }
        }
        for (i, rd) in r.readers.iter().enumerate() {
            let t = self.reader_names[i].clone();
            for (j, a) in rd.arms.iter().enumerate() {
                let arm = if rd.arms.len() > 1 { format!("{t}{j}") } else { t.clone() };
                add(usize::from(a.opener), format!("{arm}_opens"));
                add(usize::from(a.closer), format!("{arm}_closes"));
                if let Some(l) = a.last {
                    add(usize::from(l), format!("{arm}_closer_last"));
                }
            }
            for (j, pt) in rd.parts.iter().enumerate() {
                let n = pt.kind.and_then(|k| self.tokens.get(&k).cloned()).unwrap_or_else(|| format!("{t}_part{j}"));
                add(usize::from(pt.start), format!("{n}_starts"));
                if let Some(l) = pt.last {
                    add(usize::from(l), format!("{n}_last"));
                }
            }
        }
        for t in &r.retags {
            let n = self.tokens.get(&t.kind).cloned().unwrap_or_else(|| format!("kind{}", t.kind));
            add(usize::from(t.o), format!("{n}_tag"));
        }
        for (k, &(o, _)) in r.marks.iter().enumerate() {
            if o != WORD_ALONE {
                let n = self.inputs.mark_names.iter().find(|x| usize::from(x.0) == k).map_or_else(|| format!("{k}"), |x| rust_name(&x.1));
                add(usize::from(o), format!("mark_{n}"));
            }
        }
        for (i, c) in r.checks.iter().enumerate() {
            add(usize::from(c.o), format!("check{i}"));
        }
        for (i, o) in self.tier2_masks().into_iter().enumerate() {
            add(o, format!("tier2_{i}"));
        }
        let starts_lasts: Vec<usize> = self.p.extract_outputs().iter().enumerate().filter(|(e, _)| *e == usize::from(r.starts) || *e == usize::from(r.lasts)).map(|(_, &o)| o).collect();
        for (e, o) in self.p.extract_outputs().into_iter().enumerate() {
            if !starts_lasts.contains(&o) || (e != usize::from(r.starts) && e != usize::from(r.lasts)) {
                add(o, format!("extract{e}"));
            }
        }
        // What only the refusal reads: the untaken bytes and the totals no other reader has,
        // named after every other reader's.
        let mut rest: Vec<(usize, String)> = Vec::new();
        for (i, &o) in r.untaken.iter().enumerate() {
            rest.push((usize::from(o), format!("untaken{i}")));
        }
        for (i, o) in self.p.total_outputs().into_iter().enumerate() {
            rest.push((o, format!("total{i}")));
        }
        let totals = self.p.total_outputs();
        let mut refusals: Vec<(usize, String)> = Vec::new();
        for (o, n) in rest {
            if named.iter().any(|x| x.0 == o) || refusals.iter().any(|x| x.0 == o) {
                continue;
            }
            if totals.contains(&o) {
                refusals.push((o, n));
            } else {
                named.push((o, n));
            }
        }
        // Distinct names, in order.
        let mut seen: HashMap<String, usize> = HashMap::default();
        for (_, n) in &mut named {
            let c = seen.entry(n.clone()).or_default();
            *c += 1;
            if *c > 1 {
                n.push_str(&format!("_{c}"));
            }
        }
        self.stored = named;
        self.refusals = refusals;
    }

    /// Each value's name: a stored output's, a byte compare's (`is_quote`), a class's, a
    /// region's; else its number.
    fn name_values(&mut self) {
        let p = self.p;
        let mut names: Vec<String> = (0..p.ops.len()).map(|x| format!("v{x}")).collect();
        for (x, op) in p.ops.iter().enumerate() {
            match *op {
                BlockOp::Eq(b) => names[x] = format!("is_{}", byte_name(b)),
                BlockOp::Class { table, bits } => names[x] = format!("class{table}_{bits:02x}"),
                _ => {}
            }
        }
        if let Some(w) = self.walk {
            names[usize::from(w.hidden)] = "hidden".into();
            for (k, kind) in w.kinds.iter().enumerate() {
                names[usize::from(kind.open)] = format!("region{k}_open");
                names[usize::from(kind.fill)] = format!("region{k}_fill");
                if let Some((e, _)) = kind.empty {
                    names[usize::from(e)] = format!("region{k}_empty");
                }
            }
        }
        for (o, n) in self.stored.iter().chain(&self.refusals) {
            let v = usize::from(p.outputs[*o]);
            if names[v].starts_with('v') {
                names[v] = n.clone();
            }
        }
        // Distinct.
        let mut seen: HashMap<String, usize> = HashMap::default();
        for (x, n) in names.iter_mut().enumerate() {
            let c = seen.entry(n.clone()).or_default();
            *c += 1;
            if *c > 1 {
                n.push_str(&format!("_{x}"));
            }
        }
        self.value_names = names;
    }

    fn v(&self, x: u16) -> &str {
        &self.value_names[usize::from(x)]
    }

    fn mask(&self, o: u16) -> String {
        let n = self.stored.iter().find(|s| s.0 == usize::from(o)).map(|s| s.1.clone()).expect("a stored output");
        format!("m.{n}")
    }

    fn header(&self, out: &mut String) {
        out.push_str(
            "//! Tier 1 of the grammar, written from its block program by `rerail_compiler::rust_block`\n\
             //! (decision 0451), in the lean lexer's two stages: stage 1 (`masks`) a 64-byte block a\n\
             //! step, the masks the token loop and tier 2 read a word a block each; stage 2 (`tokens`)\n\
             //! the token loop, a token's end by its first byte's rule and its record by its kind, its\n\
             //! word and its marks.\n\n",
        );
        out.push_str("use rerail_simd8::Simd8x64;\n\n");
        let _ = writeln!(out, "/// The bytes the program runs past a file's end, its outputs that much late.\nconst DELAY: usize = {};\n", self.p.delay);
    }

    fn tables(&self, out: &mut String) {
        let p = self.p;
        let r = self.r;
        let used: Vec<u16> = {
            let mut t: Vec<u16> = p.ops.iter().filter_map(|op| if let BlockOp::Class { table, .. } = op { Some(*table) } else { None }).collect();
            t.sort_unstable();
            t.dedup();
            t
        };
        for t in used {
            let c = &p.tables[usize::from(t)];
            let _ = writeln!(out, "/// Class table {t}: a byte's classes are `LO_{t}[b & 15] & HI_{t}[b >> 4]`.\nstatic LO_{t}: [u8; 16] = {:?};\nstatic HI_{t}: [u8; 16] = {:?};", c.lo, c.hi);
        }
        out.push('\n');
        let _ = writeln!(out, "/// Each byte's record kind as a token's first byte (`t::…` in tier 2).\nstatic KIND: [u8; 256] = {:?};\n", r.kinds);
        let ends: Vec<u8> = if r.ends.is_empty() { vec![crate::records::NO_RULE; 256] } else { r.ends.clone() };
        let _ = writeln!(out, "/// Each byte's end rule as a token's first byte (`{}`: none, the byte alone).\nstatic END: [u8; 256] = {ends:?};\n", crate::records::NO_RULE);
        let table = crate::records::WordTable::new(&r.words).expect("the program's words fit a table");
        let _ = writeln!(out, "/// The words a record names, by slot (`records::WordTable`): `slot = ((key ^ len) * WORD_MULT) >> (64 - WORD_BITS)`.");
        let _ = writeln!(out, "const WORD_MULT: u64 = 0x{:016x};\nconst WORD_BITS: u32 = {};", table.mult, table.bits);
        let _ = writeln!(out, "static WORD_KEYS: [u64; {}] = {:?};", table.keys.len(), table.keys);
        let _ = writeln!(out, "static WORD_META: [u32; {}] = {:?};", table.meta.len(), table.meta);
        let _ = writeln!(out, "/// A word's bytes past its first eight, by its code less one.\nstatic WORD_TAILS: [[u64; 7]; {}] = {:?};\n", table.tails.len(), table.tails);
        for (i, t) in r.nexts.iter().enumerate() {
            let _ = writeln!(out, "/// Pair table {i}: by a token's first byte its row, by the next byte its bit.\nstatic PAIR{i}_ROW: [u8; 256] = {:?};\nstatic PAIR{i}_SETS: [[u64; 4]; {}] = {:?};\n", t.row, t.sets.len(), t.sets);
        }
        let words: Vec<String> = r.words.iter().map(|w| String::from_utf8_lossy(w).into_owned()).collect();
        let _ = writeln!(out, "/// The words, by code less one: {words:?}.\n");
    }

    fn masks_struct(&self, out: &mut String) {
        out.push_str("/// The masks stage 2 and tier 2 read: each a word a block, bit `i` byte `64 w + i`'s.\npub struct Masks {\n");
        for (o, n) in &self.stored {
            let _ = writeln!(out, "    /// Output {o}.\n    pub {n}: Vec<u64>,");
        }
        out.push_str("    /// Whether a byte of the file is refused (the totals, the untaken bytes).\n    pub refused: bool,\n}\n\n");
    }

    /// The values a block computes for `roots`, stopping at `stop` (made elsewhere): in order.
    fn cone(&self, roots: &[u16], stop: &[u16]) -> Vec<usize> {
        let mut keep = vec![false; self.p.ops.len()];
        let mut todo: Vec<usize> = roots.iter().map(|&v| usize::from(v)).collect();
        while let Some(x) = todo.pop() {
            if std::mem::replace(&mut keep[x], true) {
                continue;
            }
            if stop.contains(&(x as u16)) {
                continue;
            }
            todo.extend(self.p.ops[x].operands().into_iter().map(usize::from));
        }
        (0..keep.len()).filter(|&x| keep[x]).collect()
    }

    /// The walk's inputs: each kind's opener, closer and second closer.
    fn walk_inputs(&self) -> Vec<u16> {
        let Some(w) = self.walk else { return Vec::new() };
        let mut v: Vec<u16> = w.kinds.iter().flat_map(|k| [k.opener, k.closer].into_iter().chain(k.or.map(|o| o.0))).collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    fn made(&self) -> rerail_blocks::vector::Vector<u16> {
        self.walk.map_or(rerail_blocks::vector::Vector::new(), |w| w.made())
    }

    /// A block loop's body over `values` (in order): each a `let`, the carried operations'
    /// state in `carry_…` and `last_…` locals declared before the loop (`decls`), updated after.
    fn body(&self, values: &[usize], loaded: &dyn Fn(usize) -> Option<String>, decls: &mut String, body: &mut String, after: &mut String) -> Result<(), String> {
        let p = self.p;
        let mut tables: Vec<u16> = Vec::new();
        let mut lasts: Vec<u16> = Vec::new();
        body.push_str("        let v = S::load(&block);\n");
        for &x in values {
            let n = &self.value_names[x];
            if let Some(load) = loaded(x) {
                let _ = writeln!(body, "        let {n} = {load};");
                continue;
            }
            let e = match p.ops[x] {
                BlockOp::Eq(b) => format!("S::eq_mask(v, {})", byte_lit(b)),
                BlockOp::Class { table, bits } => {
                    if !tables.contains(&table) {
                        tables.push(table);
                        let _ = writeln!(body, "        let classes{table} = S::classify(v, &LO_{table}, &HI_{table});");
                    }
                    format!("S::to_bitmask(S::test(classes{table}, S::splat(0x{bits:02x})))")
                }
                BlockOp::Const(c) => format!("0x{c:016x}"),
                BlockOp::Not(a) => format!("!{}", self.v(a)),
                BlockOp::And(a, b) => format!("{} & {}", self.v(a), self.v(b)),
                BlockOp::Or(a, b) => format!("{} | {}", self.v(a), self.v(b)),
                BlockOp::Xor(a, b) => format!("{} ^ {}", self.v(a), self.v(b)),
                BlockOp::AndNot(a, b) => format!("{} & !{}", self.v(a), self.v(b)),
                BlockOp::Early(a, k) => format!("{} >> {k}", self.v(a)),
                BlockOp::Within(d) => format!("within(i, len, {d})"),
                BlockOp::Prev(a, k) => {
                    if !lasts.contains(&a) {
                        lasts.push(a);
                        let _ = writeln!(decls, "    let mut last_{} = 0u64;", self.v(a));
                        let _ = writeln!(after, "        last_{0} = {0};", self.v(a));
                    }
                    format!("{0} << {k} | last_{0} >> {1}", self.v(a), 64 - u32::from(k))
                }
                BlockOp::Toggle(a) => {
                    let _ = writeln!(decls, "    let mut carry_{n} = 0u64;");
                    let _ = writeln!(body, "        let {n}; ({n}, carry_{n}) = toggle({}, carry_{n});", self.v(a));
                    continue;
                }
                BlockOp::Escaped(a) => {
                    let _ = writeln!(decls, "    let mut carry_{n} = 0u64;");
                    let _ = writeln!(body, "        let {n}; ({n}, carry_{n}) = escaped({}, carry_{n});", self.v(a));
                    continue;
                }
                BlockOp::Fill(a, b) => {
                    let _ = writeln!(decls, "    let mut carry_{n} = 0u64;");
                    let _ = writeln!(body, "        let {n}; ({n}, carry_{n}) = fill({}, {}, carry_{n});", self.v(a), self.v(b));
                    continue;
                }
                BlockOp::Carry(_) => return Err("a fixed point's carry outside its walk".into()),
                op => return Err(format!("{op:?}: not written yet")),
            };
            let _ = writeln!(body, "        let {n} = {e};");
        }
        Ok(())
    }

    /// Stage 1a and the walk: the regions.
    fn regions_code(&self, out: &mut String) -> Result<(), String> {
        let Some(w) = self.walk else { return Ok(()) };
        let inputs = self.walk_inputs();
        let made = self.made();
        let values = self.cone(&inputs, &[]);
        if values.iter().any(|x| made.contains(&(*x as u16))) {
            return Err("a walk's input reads what the walk makes".into());
        }
        out.push_str("/// The regions' openers and closers (stage 1a), a word a block, as the program makes them\n/// (late by `DELAY`).\nstruct RegionInputs {\n");
        for &x in &inputs {
            let _ = writeln!(out, "    {}: Vec<u64>,", self.v(x));
        }
        out.push_str("}\n\n");
        let (mut decls, mut body, mut after) = (String::new(), String::new(), String::new());
        self.body(&values, &|_| None, &mut decls, &mut body, &mut after)?;
        out.push_str("/// Stage 1a: a block a step, the values the regions' walk reads.\n#[inline(always)]\nunsafe fn region_inputs<S: Simd8x64>(src: &[u8]) -> RegionInputs {\n    let len = src.len();\n    let blocks = (len + DELAY).div_ceil(64);\n");
        let _ = writeln!(out, "    let mut r = RegionInputs {{ {} }};", inputs.iter().map(|&x| format!("{}: Vec::with_capacity(blocks)", self.v(x))).collect::<Vec<_>>().join(", "));
        out.push_str(&decls);
        out.push_str("    for i in 0..blocks {\n        let block = block_at(src, i);\n        // SAFETY: the dispatcher runs this on a backend the CPU has.\n        unsafe {\n");
        out.push_str(&indent(&body, 2));
        for &x in &inputs {
            let _ = writeln!(out, "            r.{0}.push({0});", self.v(x));
        }
        out.push_str(&indent(&after, 2));
        out.push_str("        }\n    }\n    r\n}\n\n");
        out.push_str("rerail_simd8::dispatch! {\n    fn region_inputs_best(src: &[u8]) -> RegionInputs = region_inputs, on region_inputs_on;\n}\n\n");
        // The walk.
        out.push_str("/// The regions (`block::Walk`): each word a block, late by `DELAY`.\nstruct Regions {\n");
        for &x in &made {
            let _ = writeln!(out, "    {}: Vec<u64>,", self.v(x));
        }
        out.push_str("}\n\n");
        out.push_str(
            "/// The regions, left to right over the file (`BlockProgram::walk_group`): from each opener\n\
             /// outside a region, the first after the hidden bytes (the first kind at a place), its region\n\
             /// to its closer's first bit after it.\n\
             fn regions(src: &[u8]) -> Regions {\n    let i = region_inputs_best(src);\n    let blocks = (src.len() + DELAY).div_ceil(64);\n    let end = 64 * blocks;\n",
        );
        let _ = writeln!(out, "    let mut r = Regions {{ {} }};", made.iter().map(|&x| format!("{}: vec![0; blocks]", self.v(x))).collect::<Vec<_>>().join(", "));
        out.push_str("    let mut resume = 0usize;\n    // The last kind with no `after` opened (none: 16).\n    let mut last = 16u32;\n    loop {\n        let mut best: Option<(usize, usize)> = None;\n");
        for (k, kind) in w.kinds.iter().enumerate() {
            let allowed = if kind.after == 0 { String::new() } else { format!("last < 16 && (0x{:x}u32 >> last) & 1 != 0 && ", kind.after) };
            let _ = writeln!(out, "        if {allowed}let Some(t) = next_bit(&i.{}, resume, end) {{\n            if best.is_none_or(|b| t < b.0) {{\n                best = Some((t, {k}));\n            }}\n        }}", self.v(kind.opener));
        }
        out.push_str("        let Some((t, k)) = best else { break };\n        match k {\n");
        for (k, kind) in w.kinds.iter().enumerate() {
            let _ = writeln!(out, "            {k} => {{");
            let _ = writeln!(out, "                let closer = next_bit(&i.{}, t + 1, end);", self.v(kind.closer));
            if let Some((e, m)) = kind.empty {
                let _ = writeln!(out, "                if closer.is_none() {{\n                    set(&mut r.{}, t, t + 1);\n                    set(&mut r.hidden, t + 1, (t + {m}).min(end));\n                    resume = t + {m};\n                    continue;\n                }}", self.v(e));
            }
            let _ = writeln!(out, "                set(&mut r.{}, t, t + 1);", self.v(kind.open));
            if kind.after == 0 {
                let _ = writeln!(out, "                last = {k};");
            }
            let _ = writeln!(out, "                let Some(c) = closer else {{\n                    set(&mut r.{0}, t, end);\n                    set(&mut r.hidden, t + 1, end);\n                    break;\n                }};\n                set(&mut r.{0}, t, c);", self.v(kind.fill));
            let to = if kind.outside {
                "c".to_owned()
            } else if let Some((c2, m2)) = kind.or {
                format!("c + if bit(&i.{}, c) {{ {m2} }} else {{ {} }}", self.v(c2), kind.bytes)
            } else {
                format!("c + {}", kind.bytes)
            };
            let _ = writeln!(out, "                let to = {to};\n                set(&mut r.hidden, t + 1, to.min(end));\n                resume = to.max(t + 1);\n            }}");
        }
        out.push_str("            _ => unreachable!(\"a region's kind\"),\n        }\n    }\n    r\n}\n\n");
        Ok(())
    }

    /// Stage 1b: the masks.
    fn stage1(&self, out: &mut String) -> Result<(), String> {
        let made = self.made();
        let roots: Vec<u16> = self.stored.iter().chain(&self.refusals).map(|(o, _)| self.p.outputs[*o]).collect();
        let values = self.cone(&roots, &made);
        let made_names: Vec<usize> = made.iter().map(|&m| usize::from(m)).collect();
        let loaded = |x: usize| made_names.contains(&x).then(|| format!("regions.{}[i]", self.value_names[x]));
        let (mut decls, mut body, mut after) = (String::new(), String::new(), String::new());
        self.body(&values, &loaded, &mut decls, &mut body, &mut after)?;
        out.push_str("/// Stage 1: a block a step, the masks stage 2 and tier 2 read, as the stream's bytes (each\n/// output's delay taken off, cut at the file's end).\n#[inline(always)]\nunsafe fn masks<S: Simd8x64>(src: &[u8]) -> Masks {\n    let len = src.len();\n    let blocks = (len + DELAY).div_ceil(64);\n");
        if self.walk.is_some() {
            out.push_str("    let regions = regions(src);\n");
        }
        let _ = writeln!(out, "    let mut raw = Masks {{ {}refused: false }};", self.stored.iter().map(|(_, n)| format!("{n}: Vec::with_capacity(blocks), ")).collect::<String>());
        if !self.refusals.is_empty() {
            out.push_str("    let mut refused = 0u64;\n");
        }
        out.push_str(&decls);
        out.push_str("    for i in 0..blocks {\n        let block = block_at(src, i);\n        // SAFETY: the dispatcher runs this on a backend the CPU has.\n        unsafe {\n");
        out.push_str(&indent(&body, 2));
        for (o, n) in &self.stored {
            let _ = writeln!(out, "            raw.{n}.push({});", self.v(self.p.outputs[*o]));
        }
        // Each refusal's bits inside the file (`within`, by its delay), into the one word.
        if !self.refusals.is_empty() {
            let mut delays: Vec<u8> = self.refusals.iter().map(|(o, _)| self.p.output_delay(*o)).collect();
            delays.sort_unstable();
            delays.dedup();
            let terms: Vec<String> = delays
                .iter()
                .map(|&d| {
                    let of: Vec<&str> = self.refusals.iter().filter(|(o, _)| self.p.output_delay(*o) == d).map(|(o, _)| self.v(self.p.outputs[*o])).collect();
                    let or = if of.len() > 1 { format!("({})", of.join(" | ")) } else { of[0].to_owned() };
                    format!("{or} & within(i, len, {d})")
                })
                .collect();
            let _ = writeln!(out, "            refused |= {};", terms.join("\n                | "));
        }
        out.push_str(&indent(&after, 2));
        out.push_str("        }\n    }\n");
        let _ = writeln!(
            out,
            "    Masks {{ {}refused: {} }}\n}}\n",
            self.stored.iter().map(|(o, n)| format!("{n}: undelay(&raw.{n}, {}, len), ", self.p.output_delay(*o))).collect::<String>(),
            if self.refusals.is_empty() { "false" } else { "refused != 0" }
        );
        out.push_str("rerail_simd8::dispatch! {\n    fn masks_best(src: &[u8]) -> Masks = masks, on masks_on;\n}\n\n");
        Ok(())
    }

    /// Stage 2: the token loop.
    fn stage2(&self, out: &mut String) {
        let r = self.r;
        let gaps = r.gaps;
        let lazy = self.inputs.lazy >> crate::records::MARK_SHIFT;
        out.push_str(
            "/// A file's tier 1: the masks, the tokens' first and last bytes and codes, and whether the\n\
             /// file goes to the fallback.\npub struct Lexed {\n    pub masks: Masks,\n    pub starts: Vec<u32>,\n    pub lasts: Vec<u32>,\n    pub codes: Vec<u32>,\n    pub refused: bool,\n}\n\n\
             /// Tier 1 over `src`: stage 1, then the token loop.\npub fn lex(src: &[u8]) -> Lexed {\n    let m = masks_best(src);\n    let mut t = Tokens::new(src.len());\n    t.run(src, &m);\n",
        );
        // A total another reader has is a mask; the others are stage 1's one word (`refused`).
        let totals: Vec<usize> = self.p.total_outputs().into_iter().filter(|o| !self.refusals.iter().any(|x| x.0 == *o)).collect();
        let any_total = totals.iter().map(|&o| format!("m.{}.iter().any(|&w| w != 0) || ", self.name_of(o))).chain(std::iter::once("m.refused".to_owned())).collect::<String>();
        let untaken = if r.untaken.is_empty() { "false" } else { "t.untaken" };
        let _ = writeln!(out, "    let refused = {any_total} || {untaken} || t.read || t.checked || t.bad || t.unplaced(&m, src.len());");
        out.push_str("    Lexed { masks: m, starts: t.starts, lasts: t.lasts, codes: t.codes, refused }\n}\n\n");
        // The loop's state.
        out.push_str(
            "/// The token loop's state: the records so far, the refusals, and what the gap marks carry\n\
             /// from token to token (`records::gap_marks`).\nstruct Tokens {\n    starts: Vec<u32>,\n    lasts: Vec<u32>,\n    codes: Vec<u32>,\n    /// The byte after the last token's.\n    after: usize,\n    untaken: bool,\n    read: bool,\n    checked: bool,\n    bad: bool,\n    /// The byte after the last grammar token's first, its word, and the marks' tests' hits\n    /// carried to the next grammar token.\n    from: usize,\n    word_before: u32,\n    pending: u32,\n    /// The byte after the last record's first: an extra's own gap starts there.\n    from_record: usize,\n    /// A region's tokens, read before they are written.\n    parts: Vec<(usize, usize)>,\n}\n\n\
             impl Tokens {\n    fn new(n: usize) -> Self {\n        let room = n / 4 + 16;\n        Tokens { starts: Vec::with_capacity(room), lasts: Vec::with_capacity(room), codes: Vec::with_capacity(room), after: 0, untaken: false, read: false, checked: false, bad: false, from: 0, word_before: 0, pending: 0, from_record: 0, parts: Vec::new() }\n    }\n\n",
        );
        // The loop.
        let _ = writeln!(out, "    /// The tokens, as the lean lexer's loop: each at the first byte past the blanks, to its last\n    /// by its first byte's end rule (`records::walk`). A token the file is refused at ends the\n    /// loop before its record, as a lexer's error does (the CPU's tier 1 stops there too).\n    fn run(&mut self, src: &[u8], m: &Masks) {{\n        let n = src.len();\n        let mut at = 0usize;\n        while at < n && !self.stops() {{\n            let Some(s) = next_clear(&{}, at, n) else {{ break }};\n            let rule = END[usize::from(src[s])];\n            let l = match rule {{", self.mask(r.skip.expect("checked")));
        for (k, rule) in r.rules.iter().enumerate() {
            let name = self.rule_token(k);
            let checks = r.checks.iter().any(|c| c.rule as usize == k && c.then == CheckThen::Escape);
            let check = if checks { format!("\n                    self.checked |= escapes(m, {k}, s, l);") } else { String::new() };
            // A rule with a `Cut` test: its token cut before its tests (`records::cut`).
            let cuts = r.checks.iter().any(|c| c.rule == k as u8 && c.then == CheckThen::Cut);
            let (cut_open, cut_close) = if cuts { (format!("cut(m, {k}, s, "), ")".to_owned()) } else { (String::new(), String::new()) };
            match *rule {
                EndRule::Read(i) => {
                    let rd = &r.readers[usize::from(i)];
                    let name = &self.reader_names[usize::from(i)];
                    let _ = write!(out, "\n                // {name}\n                {k} => {{");
                    if rd.parts_in_tier1() {
                        let a = &rd.arms[0];
                        let _ = write!(
                            out,
                            "\n                    if bit(&{}, s) {{\n                        let mut parts = std::mem::take(&mut self.parts);\n                        match read_{name}_parts(m, s, &mut parts) {{\n                            Ok(()) => {{\n                                let l = parts.last().map_or(s, |p| p.1);\n                                for &(a, b) in &parts {{\n                                    self.token(src, m, a, b);\n                                }}\n                                at = l + 1;\n                            }}\n                            Err(()) => {{\n                                self.read = true;\n                                break;\n                            }}\n                        }}\n                        self.parts = parts;\n                        continue;\n                    }}",
                            self.mask(a.opener)
                        );
                    }
                    let _ = write!(out, "\n                    let l = match read_{name}(src, m, s) {{\n                        Ok(l) => l.max(s),\n                        Err(()) => {{\n                            self.read = true;\n                            s\n                        }}\n                    }};{check}\n                    l\n                }}");
                }
                EndRule::Run(o) => {
                    let _ = write!(out, "\n                // {name}\n                {k} => {{\n                    let l = {cut_open}run_end(&{}, s).max(s){cut_close};{check}\n                    l\n                }}", self.mask(o));
                }
                EndRule::Pair(o) => {
                    let _ = write!(out, "\n                // {name}\n                {k} => {{\n                    let l = {cut_open}s + usize::from(bit(&{}, s)){cut_close};{check}\n                    l\n                }}", self.mask(o));
                }
                EndRule::Last(o) => {
                    let _ = write!(out, "\n                // {name}\n                {k} => {{\n                    let l = {cut_open}first_bit(&{}, s).unwrap_or(s).max(s){cut_close};{check}\n                    l\n                }}", self.mask(o));
                }
                EndRule::Next(i) => {
                    let _ = write!(out, "\n                // {name}\n                {k} => {{\n                    let l = {cut_open}next{i}(src, s, n){cut_close};{check}\n                    l\n                }}");
                }
            }
        }
        out.push_str("\n                _ => {\n                    self.untaken = true;\n                    s\n                }\n            };\n            if self.stops() {\n                break;\n            }\n            self.token(src, m, s, l);\n            at = l + 1;\n        }\n    }\n\n");
        let untaken = if r.untaken.is_empty() { "" } else { "self.untaken || " };
        let _ = writeln!(out, "    /// Whether a token read so far refuses the file: a byte no token begins with (where the\n    /// grammar's errors say so), a region with no closer, a token test's escape.\n    fn stops(&self) -> bool {{\n        {untaken}self.read || self.checked\n    }}\n");
        // A token's record.
        out.push_str("    /// Token `s..=l`'s record (`records::pass`): its first byte's kind or a reader's, its word,\n    /// and its marks, the gap marks from the bytes since the grammar token before.\n    fn token(&mut self, src: &[u8], m: &Masks, s: usize, l: usize) {\n        self.bad |= l < s || s < self.after;\n        self.after = l + 1;\n        let mut kind = u32::from(KIND[usize::from(src[s])]);\n        let mut word = word(src, s, (l + 1).saturating_sub(s));\n        let mut marks = 0u32;\n");
        for t in &r.retags {
            let word = if t.part { String::new() } else { " word = 0;".into() };
            let kind = self.tokens.get(&t.kind).map_or(t.kind.to_string(), |n| format!("{} /* {n} */", t.kind));
            let marks = if t.marks == 0 { String::new() } else { format!(" marks |= 0x{:x};", t.marks) };
            let _ = writeln!(out, "        if bit(&{}, s) {{\n            kind = {kind};{marks}{word}\n        }}", self.mask(t.o));
        }
        let retagged: u32 = r.retags.iter().fold(0, |a, t| a | t.marks);
        for (k, &(o, w)) in r.marks.iter().enumerate().filter(|&(k, _)| retagged >> k & 1 == 0) {
            let mut terms = Vec::new();
            if w != 0 {
                terms.push(format!("word == {w}"));
            }
            if o != WORD_ALONE {
                terms.push(format!("bit(&{}, s)", self.mask(o)));
            }
            let cond = if terms.is_empty() { "true".to_owned() } else { terms.join(" && ") };
            let _ = writeln!(out, "        // mark {k}\n        if {cond} {{\n            marks |= 1 << {k};\n        }}");
        }
        out.push_str("        let mut code = kind | word << 8 | marks << 16;\n");
        if gaps != 0 {
            let low = (1u32 << crate::records::AFTER_SHIFT) - 1;
            let hits = if r.checks.is_empty() { "0".to_owned() } else { "hits(m, END[usize::from(src[s])], s, l)".to_owned() };
            let extra = r.extra.map_or("false".to_owned(), |e| format!("code >> {} & 1 == 1", 16 + u32::from(e)));
            // An extra keeps the marks of its own gap (`Records::own`).
            let own: String = (0..r.marks.len())
                .filter(|&k| r.own >> k & 1 == 1 && gaps >> k & 1 == 1 && r.tested_only >> k & 1 == 0)
                .map(|k| format!("\n            if any_in(&{}, self.from_record, s) {{\n                code |= 1 << {};\n            }}", self.mask(r.marks[k].0), 16 + k))
                .collect();
            let _ = writeln!(out, "        let h: u32 = {hits};\n        if {extra} {{\n            // An extra's token: no gap mark but those it keeps of its own gap; its hits go on\n            // to the next grammar token.\n            code &= !(0x{gaps:x} << 16);{own}\n            self.pending |= h & 0x{low:x} | h >> 16;\n        }} else {{\n            let mut gap = 0u32;");
            for (k, &(o, _)) in r.marks.iter().enumerate().filter(|&(k, _)| gaps >> k & 1 == 1) {
                let unless: Vec<String> = r.unless.iter().filter(|&&(u, _)| usize::from(u) == k).map(|&(_, w)| format!("self.word_before == {w}")).collect();
                let none = if unless.is_empty() { String::new() } else { format!("!({}) && ", unless.join(" || ")) };
                let from_mask = if r.tested_only >> k & 1 == 0 { format!(" || any_in(&{}, self.from, s)", self.mask(o)) } else { String::new() };
                let _ = writeln!(out, "            // gap mark {k}\n            if {none}((self.pending | h) >> {k} & 1 == 1{from_mask}) {{\n                gap |= 1 << {k};\n            }}");
            }
            let _ = writeln!(out, "            code = code & !(0x{gaps:x} << 16) | gap << 16;\n            self.from = s + 1;\n            self.word_before = code >> 8 & 0xff;\n            self.pending = h >> 16;\n        }}");
        }
        let from_record = if gaps != 0 && r.extra.is_some() && r.own != 0 { "        self.from_record = s + 1;\n" } else { "" };
        let _ = writeln!(out, "{from_record}        self.starts.push(s as u32);\n        self.lasts.push(l as u32);\n        // The gap marks tier 2 tests itself.\n        self.codes.push(code & !(0x{lazy:x} << 16));\n    }}\n");
        // Unplaced.
        out.push_str("    /// Whether a bit of an `else escape` gap mark has no grammar token after it to hold it.\n    fn unplaced(&self, m: &Masks, n: usize) -> bool {\n        let _ = (m, n);\n");
        let unplaced: Vec<String> = r
            .marks
            .iter()
            .enumerate()
            .filter(|&(k, _)| r.unplaced >> k & 1 == 1)
            .map(|(k, &(o, _))| {
                let mask = if r.tested_only >> k & 1 == 0 { format!(" || n > 0 && any_in(&{}, self.from, 64 * n.div_ceil(64) - 1)", self.mask(o)) } else { String::new() };
                format!("self.pending >> {k} & 1 == 1{mask}")
            })
            .collect();
        let _ = writeln!(out, "        {}\n    }}\n}}\n", if unplaced.is_empty() { "false".to_owned() } else { unplaced.join("\n            || ") });
        self.readers(out);
        self.checks_code(out);
        for (i, t) in r.nexts.iter().enumerate() {
            let _ = writeln!(out, "/// Whether `a` then `b` is a pair of table {i}.\nfn pair{i}(a: u8, b: u8) -> bool {{\n    let row = usize::from(PAIR{i}_ROW[usize::from(a)]);\n    PAIR{i}_SETS.get(row).is_some_and(|set| set[usize::from(b) / 64] >> (b % 64) & 1 == 1)\n}}\n");
            // A token's last byte by its first and the next, then the one after where a triple
            // goes on from the pair, as the lean lexer reads `===` after `==`.
            let pair = if t.thirds.is_empty() {
                "s + 1".to_owned()
            } else {
                let arms: Vec<String> = t.thirds.iter().map(|&[a, b, c]| format!("({}, {}, Some(&{}))", byte_lit(a), byte_lit(b), byte_lit(c))).collect();
                format!("match (src[s], b, src.get(s + 2).filter(|_| s + 2 < n)) {{\n            {} => s + 2,\n            _ => s + 1,\n        }}", arms.join("\n            | "))
            };
            let _ = writeln!(out, "/// The last byte of a token of table {i} from `s`: its first, the next where they are a pair{}.\nfn next{i}(src: &[u8], s: usize, n: usize) -> usize {{\n    match src.get(s + 1) {{\n        Some(&b) if s + 1 < n && pair{i}(src[s], b) => {pair},\n        _ => s,\n    }}\n}}\n", if t.thirds.is_empty() { "" } else { ", the one after where the three are a triple" });
        }
    }

    /// The masks tier 2 reads from tier 1's words (`BlockNative::masks`), not from the bytes.
    fn tier2_masks(&self) -> Vec<usize> {
        let mut v: Vec<usize> = self.p.mask_outputs().into_iter().filter(|o| !self.inputs.unmade.contains(o)).collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    fn name_of(&self, o: usize) -> String {
        self.stored.iter().find(|s| s.0 == o).map(|s| s.1.clone()).expect("a stored output")
    }

    /// Each reader's functions: a region's last byte (`records::read_last`), and a region read in
    /// parts (`records::read_parts`).
    fn readers(&self, out: &mut String) {
        let r = self.r;
        for (i, rd) in r.readers.iter().enumerate() {
            let name = self.reader_names[i].clone();
            let _ = writeln!(out, "/// A {name} token's last byte from its first, `s`: its region's, by the first arm whose opener\n/// holds there (`records::read_last`).\nfn read_{name}(src: &[u8], m: &Masks, s: usize) -> Result<usize, ()> {{\n    let n = src.len();\n    let _ = n;");
            for a in &rd.arms {
                let none = match a.none {
                    ReadNone::Refuse => "Err(())".to_owned(),
                    ReadNone::End => "Ok(n.saturating_sub(1))".to_owned(),
                    ReadNone::Bytes(b) => format!("Ok(s + {b} - 1)"),
                };
                let _ = writeln!(out, "    if bit(&{}, s) {{\n        return match first_bit(&{}, s + 1) {{\n            Some(c) => Ok(c.wrapping_add_signed({})),\n            None => {none},\n        }};\n    }}", self.mask(a.opener), self.mask(a.closer), a.off);
            }
            let other = match r.rules.get(usize::from(rd.other)) {
                Some(EndRule::Read(_)) | None => "Err(())".to_owned(),
                Some(EndRule::Run(o)) => format!("Ok(run_end(&{}, s))", self.mask(*o)),
                Some(EndRule::Pair(o)) => format!("Ok(s + usize::from(bit(&{}, s)))", self.mask(*o)),
                Some(EndRule::Last(o)) => format!("Ok(first_bit(&{}, s).unwrap_or(s))", self.mask(*o)),
                Some(EndRule::Next(j)) => format!("Ok(next{j}(src, s, n))"),
            };
            let _ = writeln!(out, "    {other}\n}}\n");
            if !rd.parts_in_tier1() {
                continue;
            }
            let a = &rd.arms[0];
            let _ = writeln!(
                out,
                "/// A {name} region read in parts from its opener at `s` (`records::read_parts`), as the lean\n/// lexer's string reader: the opener's byte; then the nearer of the closer and each part, the\n/// bytes before it a token of the content's, a part its bytes; to the closer.\nfn read_{name}_parts(m: &Masks, s: usize, out: &mut Vec<(usize, usize)>) -> Result<(), ()> {{\n    out.clear();\n    out.push((s, s));\n    let mut at = s + 1;\n    loop {{\n        let c = first_bit(&{}, at).ok_or(())?;\n        // The nearest part before the closer, by its start, its bytes and its mask.\n        let mut part: Option<(usize, u8, u16, usize)> = None;",
                self.mask(a.closer)
            );
            for (j, pt) in rd.parts.iter().enumerate() {
                let _ = writeln!(out, "        if let Some(e) = first_bit(&{}, at).filter(|&e| e < c) {{\n            if part.is_none_or(|p| (e, {}u8, {}u16) < (p.0, p.1, p.2)) {{\n                part = Some((e, {}, {}, {j}));\n            }}\n        }}", self.mask(pt.start), pt.bytes, pt.start, pt.bytes, pt.start);
            }
            out.push_str("        let to = part.map_or(c, |p| p.0);\n        if to > at {\n            out.push((at, to - 1));\n        }\n        match part {\n            Some((e, _, _, j)) => {\n                let l = match j {\n");
            for (j, pt) in rd.parts.iter().enumerate() {
                let l = match pt.last {
                    Some(o) => format!("first_bit(&{}, e).ok_or(())?", self.mask(o)),
                    None => format!("e + {} - 1", pt.bytes),
                };
                let _ = writeln!(out, "                    {j} => {l},");
            }
            let last = match a.last {
                Some(o) => format!("first_bit(&{}, c).ok_or(())?", self.mask(o)),
                None => format!("c.wrapping_add_signed({})", a.off),
            };
            let _ = writeln!(out, "                    _ => unreachable!(\"a part\"),\n                }};\n                out.push((e, l));\n                at = l + 1;\n            }}\n            None => {{\n                let l = {last};\n                out.push((c, l));\n                return Ok(());\n            }}\n        }}\n    }}\n}}\n");
        }
    }

    /// The token tests: `escapes` (a token that passes one refuses the file) and `hits` (the
    /// marks' tests' bits), by the token's rule (`records::check_hits`, `test_holds`).
    fn checks_code(&self, out: &mut String) {
        let r = self.r;
        let holds = |c: &crate::records::TokenCheck| -> String {
            if c.always() {
                match r.rules.get(usize::from(c.rule)) {
                    Some(EndRule::Read(i)) => r.readers[usize::from(*i)].arms.iter().map(|a| format!("bit(&{}, s)", self.mask(a.opener))).collect::<Vec<_>>().join(" || "),
                    _ => "true".into(),
                }
            } else if c.count {
                format!("count_in(&{}, s, l) >= {}", self.mask(c.o), c.least)
            } else if c.all {
                format!("l + 1 - s >= {0} && all_in(&{1}, s, l + 1 - s)", c.least, self.mask(c.o))
            } else {
                format!("l + 1 - s >= {0} && all_in(&{1}, s, {0})", c.least, self.mask(c.o))
            }
        };
        out.push_str("/// Whether token `s..=l` of rule `rule` passes a test that refuses the file (`escape … at`).\n#[allow(unused_variables)]\nfn escapes(m: &Masks, rule: u8, s: usize, l: usize) -> bool {\n    match rule {\n");
        let mut rules: Vec<u8> = r.checks.iter().map(|c| c.rule).collect();
        rules.sort_unstable();
        rules.dedup();
        for &k in &rules {
            let e: Vec<String> = r.checks.iter().filter(|c| c.rule == k && c.then == CheckThen::Escape).map(|c| format!("({})", holds(c))).collect();
            if !e.is_empty() {
                let _ = writeln!(out, "        {k} => {},", e.join(" || "));
            }
        }
        out.push_str("        _ => false,\n    }\n}\n\n");
        out.push_str("/// The marks' tests' hits at token `s..=l` of rule `rule`: the `at` marks' bits, the `after`\n/// marks' at bit 16 on.\n#[allow(unused_variables)]\nfn hits(m: &Masks, rule: u8, s: usize, l: usize) -> u32 {\n    let mut h = 0u32;\n    match rule {\n");
        for &k in &rules {
            let hs: Vec<String> = r
                .checks
                .iter()
                .filter(|c| c.rule == k)
                .filter_map(|c| {
                    let bit = match c.then {
                        CheckThen::Escape | CheckThen::Cut => return None,
                        CheckThen::At(m) => 1u32 << m,
                        CheckThen::After(m) => 1u32 << (crate::records::AFTER_SHIFT + u32::from(m)),
                    };
                    Some(format!("            if {} {{\n                h |= 0x{bit:x};\n            }}", holds(c)))
                })
                .collect();
            if !hs.is_empty() {
                let _ = writeln!(out, "        {k} => {{\n{}\n        }}", hs.join("\n"));
            }
        }
        out.push_str("        _ => {}\n    }\n    h\n}\n\n");
        // A rule's `Cut` tests (0413, `run R once D`): its token ends before the test's mask's
        // `least`th bit in it, before its other tests are made (`records::cut`).
        let cuts: Vec<&crate::records::TokenCheck> = r.checks.iter().filter(|c| c.then == CheckThen::Cut).collect();
        if !cuts.is_empty() {
            out.push_str("/// Token `s..=l` of rule `rule` cut by its rule's `Cut` tests (`run R once D`): it ends\n/// before the test's mask's `least`th bit in it, as a lexer stops at a number's second `.`.\nfn cut(m: &Masks, rule: u8, s: usize, l: usize) -> usize {\n    match rule {\n");
            for &k in &rules {
                let cs: Vec<String> = cuts.iter().filter(|c| c.rule == k).map(|c| format!("            let l = nth_in(&{}, s, l, {}).map_or(l, |i| i - 1);", self.mask(c.o), c.least)).collect();
                if !cs.is_empty() {
                    let _ = writeln!(out, "        {k} => {{\n{}\n            l\n        }}", cs.join("\n"));
                }
            }
            out.push_str("        _ => l,\n    }\n}\n\n");
        }
    }

    fn helpers(&self, out: &mut String) {
        out.push_str(HELPERS);
    }

    /// The harness's entry: each row's file lexed, its records, counts and tier 2's masks written
    /// where the CPU's tier 1 writes them (`native_cpu_block::Batch`).
    fn entry(&self, out: &mut String) {
        let p = self.p;
        let r = self.r;
        let extracts = p.extract_outputs();
        let outputs = p.outputs.len();
        let cols = [u64::from(r.starts), u64::from(r.lasts), extracts.len() as u64];
        let index = |field: usize, k: &str| -> String {
            match self.inputs.layout {
                crate::records::RecordLayout::Columns => format!("{} * cap + {k}", cols[field]),
                crate::records::RecordLayout::Rows => format!("{} * cap + 3 * {k} + {field}", cols.iter().min().copied().unwrap_or(0)),
            }
        };
        out.push_str("/// A file of the harness's batch (`native_cpu_block::Row`).\n#[repr(C)]\npub struct Row {\n    pub bytes: u64,\n    pub blocks: u64,\n    pub len: u64,\n    pub out: u64,\n    pub positions: u64,\n    pub capacity: u64,\n    pub counts: u64,\n    pub scratch: u64,\n}\n\n");
        let _ = writeln!(out, "/// The harness's entry (`native_cpu_block::Entry`): each row's file lexed, and what tier 2 reads\n/// written into the batch's buffers.\n///\n/// # Safety\n/// The pointers are a `Batch`'s, laid out for this program: {outputs} outputs a block.\npub unsafe extern \"C\" fn tier1_entry(bytes: *const u8, rows: *const Row, n: u64, out: *mut u64, positions: *mut u32, counts: *mut u64, _scratch: *mut u64) {{");
        out.push_str("    for f in 0..n as usize {\n        // SAFETY: the batch's rows, files, and buffers (the harness's contract).\n        unsafe {\n            let row = &*rows.add(f);\n            let src = std::slice::from_raw_parts(bytes.add(row.bytes as usize), row.len as usize);\n            let lexed = lex(src);\n            let cap = row.capacity as usize;\n            let at = positions.add(row.positions as usize);\n            for (k, ((&s, &l), &c)) in lexed.starts.iter().zip(&lexed.lasts).zip(&lexed.codes).enumerate() {\n");
        let _ = writeln!(out, "                *at.add({}) = s;\n                *at.add({}) = l;\n                *at.add({}) = c;\n            }}", index(0, "k"), index(1, "k"), index(2, "k"));
        for (e, &o) in extracts.iter().enumerate() {
            if e == usize::from(r.starts) || e == usize::from(r.lasts) {
                let _ = writeln!(out, "            *counts.add(row.counts as usize + {e}) = lexed.starts.len() as u64;");
            } else {
                let n = self.name_of(o);
                let _ = writeln!(out, "            let mut j = 0usize;\n            for (w, &x) in lexed.masks.{n}.iter().enumerate() {{\n                let mut x = x;\n                while x != 0 {{\n                    *at.add({e} * cap + j) = (64 * w + x.trailing_zeros() as usize) as u32;\n                    j += 1;\n                    x &= x - 1;\n                }}\n            }}\n            *counts.add(row.counts as usize + {e}) = j as u64;");
            }
        }
        if !p.totals.is_empty() {
            let _ = writeln!(out, "            *counts.add(row.counts as usize + {}) = u64::from(lexed.refused);", extracts.len());
        }
        let _ = writeln!(out, "            let words = out.add(row.out as usize);");
        for o in self.tier2_masks() {
            let n = self.name_of(o);
            let _ = writeln!(out, "            for (w, &x) in lexed.masks.{n}.iter().enumerate() {{\n                *words.add(w * {outputs} + {o}) = x;\n            }}");
        }
        out.push_str("        }\n    }\n}\n\n");
    }
}

fn indent(s: &str, n: usize) -> String {
    let pad = "    ".repeat(n);
    s.lines().map(|l| if l.is_empty() { "\n".to_owned() } else { format!("{pad}{l}\n") }).collect()
}

fn byte_name(b: u8) -> String {
    match b {
        b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' => format!("{}", b as char).to_ascii_lowercase() + if b.is_ascii_uppercase() { "_upper" } else { "" },
        b'_' => "underscore".into(),
        b'\n' => "lf".into(),
        b'\r' => "cr".into(),
        b'\t' => "tab".into(),
        b' ' => "space".into(),
        0 => "nul".into(),
        b if b.is_ascii_punctuation() => rust_name(&(b as char).to_string()),
        b => format!("x{b:02x}"),
    }
}

fn byte_lit(b: u8) -> String {
    match b {
        b'\'' => "b'\\''".into(),
        b'\\' => "b'\\\\'".into(),
        b'\n' => "b'\\n'".into(),
        b'\r' => "b'\\r'".into(),
        b'\t' => "b'\\t'".into(),
        0x20..=0x7e => format!("b'{}'", b as char),
        b => format!("0x{b:02x}"),
    }
}

/// The generated file's helpers: the block loop's carried operations and the token loop's
/// scans, as `block::carry` and `records` define them.
const HELPERS: &str = r#"/// Block `i` of `src`, zeros past its end.
#[inline(always)]
fn block_at(src: &[u8], i: usize) -> [u8; 64] {
    match src.get(64 * i..64 * i + 64) {
        Some(b) => b.try_into().expect("64 bytes"),
        None => {
            let mut b = [0u8; 64];
            let from = (64 * i).min(src.len());
            b[..src.len() - from].copy_from_slice(&src[from..]);
            b
        }
    }
}

/// `within`'s word at delay `d` for block `i` of `len` bytes: bits `d <= p < len + d`.
#[inline(always)]
fn within(i: usize, len: usize, d: usize) -> u64 {
    let below = |n: usize| {
        let t = n.saturating_sub(i * 64).min(64) as u32;
        ((1u64 << (t >> 1)) << (t - (t >> 1))).wrapping_sub(1)
    };
    below(len + d) & !below(d)
}

/// `fill(s, e)` (`block::carry::fill`): from each `s` up to the next `e`, one add.
#[inline(always)]
fn fill(s: u64, e: u64, cin: u64) -> (u64, u64) {
    let (a, b) = (s, !e | s);
    let into = a.wrapping_add(b).wrapping_add(cin) ^ a ^ b;
    let f = s | (!e & into);
    (f, f >> 63)
}

/// simdjson's escaped bytes after runs of `run` (`block::carry::escaped`).
#[inline(always)]
fn escaped(run: u64, cin: u64) -> (u64, u64) {
    const EVEN: u64 = 0x5555_5555_5555_5555;
    let b = run & !cin;
    let follows = (b << 1) | cin;
    let odd_starts = b & !EVEN & !follows;
    let (sequences, overflow) = odd_starts.overflowing_add(b);
    ((EVEN ^ (sequences << 1)) & follows, u64::from(overflow))
}

/// The prefix XOR (`block::carry::toggle`), inverted where `carry` is set.
#[allow(dead_code)]
#[inline(always)]
fn toggle(word: u64, carry: u64) -> (u64, u64) {
    let mut y = word;
    for k in [1, 2, 4, 8, 16, 32] {
        y ^= y << k;
    }
    let y = y ^ carry;
    (y, ((y as i64) >> 63) as u64)
}

/// The output's words as the stream's bytes: shifted down by its delay, cut at `len`.
fn undelay(words: &[u64], d: u32, len: usize) -> Vec<u64> {
    let mut out: Vec<u64> = (0..len.div_ceil(64))
        .map(|j| {
            let w = words.get(j).copied().unwrap_or(0);
            if d == 0 {
                return w;
            }
            (w >> d) | (words.get(j + 1).copied().unwrap_or(0) << (64 - d))
        })
        .collect();
    if let Some(last) = out.last_mut()
        && len % 64 != 0
    {
        *last &= (1u64 << (len % 64)) - 1;
    }
    out
}

/// Whether bit `i` of a mask holds.
#[inline(always)]
fn bit(w: &[u64], i: usize) -> bool {
    w.get(i / 64).is_some_and(|x| x >> (i % 64) & 1 == 1)
}

/// The first bit of a mask at or after `from`.
#[inline(always)]
fn first_bit(w: &[u64], from: usize) -> Option<usize> {
    let mut q = from / 64;
    let mut x = w.get(q)? & (!0u64 << (from % 64));
    while x == 0 {
        q += 1;
        x = *w.get(q)?;
    }
    Some(64 * q + x.trailing_zeros() as usize)
}

/// The first bit of a mask from `from` up to `end`, a word at a time (the regions' walk).
#[inline(always)]
fn next_bit(w: &[u64], from: usize, end: usize) -> Option<usize> {
    let mut t = from;
    while t < end {
        let x = w[t / 64] >> (t % 64);
        if x != 0 {
            return Some(t + x.trailing_zeros() as usize).filter(|&b| b < end);
        }
        t = (t / 64 + 1) * 64;
    }
    None
}

/// Bits `from..to` of a mask set.
fn set(w: &mut [u64], from: usize, to: usize) {
    let mut t = from;
    while t < to {
        let q = t / 64;
        let hi = (to - 64 * q).min(64);
        let lo = t - 64 * q;
        let ones = if hi == 64 { !0u64 } else { (1u64 << hi) - 1 };
        w[q] |= ones & (!0u64 << lo);
        t = 64 * q + hi;
    }
}

/// The first byte at or after `at`, below `n`, that the blanks' mask does not hold.
#[inline(always)]
fn next_clear(blank: &[u64], at: usize, n: usize) -> Option<usize> {
    let mut q = at / 64;
    let mut x = !blank.get(q).copied().unwrap_or(0) & (!0u64 << (at % 64));
    while x == 0 {
        q += 1;
        if 64 * q >= n {
            return None;
        }
        x = !blank.get(q).copied().unwrap_or(0);
    }
    Some(64 * q + x.trailing_zeros() as usize).filter(|&s| s < n)
}

/// A run's last byte: from `s + 1`, the mask's run, a word at a time.
#[inline(always)]
fn run_end(w: &[u64], s: usize) -> usize {
    let mut i = s + 1;
    loop {
        let Some(&x) = w.get(i / 64) else { return i - 1 };
        let ones = (!(x >> (i % 64))).trailing_zeros() as usize;
        let left = 64 - i % 64;
        if ones < left {
            return i + ones - 1;
        }
        i += left;
    }
}

/// Whether a mask holds anywhere in `a..=b`.
fn any_in(w: &[u64], a: usize, b: usize) -> bool {
    if a > b {
        return false;
    }
    (a / 64..=b / 64).any(|q| {
        let mut x = w.get(q).copied().unwrap_or(0);
        if q == a / 64 {
            x &= !0u64 << (a % 64);
        }
        if q == b / 64 && b % 64 != 63 {
            x &= (1u64 << (b % 64 + 1)) - 1;
        }
        x != 0
    })
}

/// How many bits of a mask are in `s..=l`.
#[allow(dead_code)]
fn count_in(w: &[u64], s: usize, l: usize) -> usize {
    (s..=l).filter(|&i| bit(w, i)).count()
}

/// The `n`th bit (from 1) of a mask in `s..=l`, a word at a time.
#[allow(dead_code)]
fn nth_in(w: &[u64], s: usize, l: usize, n: u32) -> Option<usize> {
    let mut left = n;
    for q in s / 64..=l / 64 {
        let mut x = w.get(q).copied().unwrap_or(0);
        if q == s / 64 {
            x &= !0u64 << (s % 64);
        }
        if q == l / 64 && l % 64 != 63 {
            x &= (1u64 << (l % 64 + 1)) - 1;
        }
        let c = x.count_ones();
        if c < left {
            left -= c;
            continue;
        }
        for _ in 1..left {
            x &= x - 1;
        }
        return Some(64 * q + x.trailing_zeros() as usize);
    }
    None
}

/// Whether a mask holds at each of `s..s + n`.
#[allow(dead_code)]
fn all_in(w: &[u64], s: usize, n: usize) -> bool {
    (s..s + n).all(|i| bit(w, i))
}

/// The word of the `len` bytes at `at` (`records::WordTable::word`): its code, or 0.
#[inline(always)]
fn word(src: &[u8], at: usize, len: usize) -> u32 {
    let load = |o: usize| -> u64 {
        let mut w = [0u8; 8];
        let have = src.len().saturating_sub(o).min(8);
        w[..have].copy_from_slice(&src[o..o + have]);
        u64::from_le_bytes(w)
    };
    let cut = |w: u64, n: usize| if n >= 8 { w } else { w & ((1u64 << (8 * n)) - 1) };
    let key = cut(load(at), len);
    let s = ((key ^ len as u64).wrapping_mul(WORD_MULT) >> (64 - WORD_BITS)) as usize;
    let (k, meta) = (WORD_KEYS[s], WORD_META[s]);
    if meta == 0 || k != key || (meta & 0xff) as usize != len {
        return 0;
    }
    let code = meta >> 8;
    if len > 8 {
        let tail = &WORD_TAILS[code as usize - 1];
        for (j, &t) in tail.iter().enumerate() {
            let from = 8 * (j + 1);
            if from < len && cut(load(at + from), len - from) != t {
                return 0;
            }
        }
    }
    code
}

"#;
