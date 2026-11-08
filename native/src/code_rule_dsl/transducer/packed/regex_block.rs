//! A span regex as a block program (decision 0081 §3): straight-line word operations over the file's bytes that
//! mark every byte after which a match of the pattern can end. Every executor of block programs runs it: the CPU
//! interpreter, Cranelift, and Metal (each lane a chunk, the carries of `prev` and `fill` across chunks). It is
//! chunk-at-a-time matching (decision 0041): no per-byte loop, no per-node scan.
//!
//! The marks are a superset of the ends of the matches `regex` finds: the host runs the pattern, captures and all,
//! only where a mark says a match can be (`core_rr::CoreRows::matches`). Exact where the translation is exact;
//! wider where it is not, never narrower.
//!
//! The translation is Parabix's (Cameron et al., "Bitwise data parallelism in regular expression matching",
//! PACT 2014). A marker stream `m` has bit `i` set when a match of the pattern so far can end just before byte `i`:
//!
//! ```text
//! start                 m = within                       every position can start a match
//! a byte set C          m' = prev(m & C, 1)              consume one byte of C
//! x y                   m' = y(x(m))
//! x | y                 m' = x(m) | y(m)
//! C*  (one-byte C)      m' = fill(m, !prev(C, 1))        through a run of C bytes (an add with carry)
//! x*  (any other x)     m' = fill(m, 0)                  every later position: wider, still a superset
//! ^ $ \b (?m)^ (?m)$    m' = m & the positions where they hold, from `within`, `prev` and `eq '\n'`
//! ```
//!
//! A Unicode class is its UTF-8 sequences, taken per length as one byte set per position (wider than the class
//! when its sequences differ in more than one byte, as large classes do). The result is delayed one byte, so that
//! a match ending at the end of the file has a bit: output bit `i` is "a match can end after byte `i`".

use super::block::{BlockOp, BlockProgram, ClassTable};
use regex_syntax::hir::{Class, Hir, HirKind, Look};
use std::collections::HashMap;

/// More operations than this and the pattern is left to the host.
const MAX_OPS: usize = 2048;

/// The ends a block program marks, for a pattern over `source` spans (`lines` false) or over each line (`lines`
/// true: the text anchors `^`, `$`, `\A`, `\z` hold at every line's edges, so they are left unchecked, as is `(?m)$`
/// before a line's `\r`).
pub(in crate::code_rule_dsl) struct SpanPrefilter {
    program: BlockProgram,
}

impl SpanPrefilter {
    /// `pattern` as a block program, or `None` when it can match the empty string (every position would be an end)
    /// or grows past `MAX_OPS`.
    pub(in crate::code_rule_dsl) fn compile(pattern: &str, lines: bool) -> Option<Self> {
        let hir = regex_syntax::ParserBuilder::new().build().parse(pattern).ok()?;
        if hir.properties().minimum_len().is_none_or(|n| n == 0) {
            return None;
        }
        let mut b = Builder { lines, ..Builder::default() };
        let start = b.push(BlockOp::Within(0));
        let end = b.hir(&hir, start)?;
        // A match ends at a character's edge: not before a continuation byte.
        let cont = b.bytes(&|x| (0x80..=0xbf).contains(&x));
        let end = b.push(BlockOp::AndNot(end, cont));
        if b.ops.len() > MAX_OPS {
            return None;
        }
        let program = BlockProgram { tables: b.tables, ops: b.ops, outputs: vec![end], delay: 1, ..BlockProgram::default() };
        program.verify().ok()?;
        Some(SpanPrefilter { program })
    }

    /// Bit `i` of the words: a match can end after byte `i` of `bytes`.
    pub(in crate::code_rule_dsl) fn ends(&self, bytes: &[u8]) -> Vec<u64> {
        self.program.interpret(bytes).swap_remove(0)
    }

    /// Whether a match can end in `(a, b]`, a span's exclusive end, given `ends`.
    pub(in crate::code_rule_dsl) fn any_in(ends: &[u64], a: usize, b: usize) -> bool {
        // Bits a..b (a match ending at e is bit e - 1).
        let (mut i, end) = (a, b);
        while i < end {
            let (w, bit) = (i / 64, i % 64);
            let take = (64 - bit).min(end - i);
            let mask = if take == 64 { u64::MAX } else { ((1u64 << take) - 1) << bit };
            if ends.get(w).is_some_and(|x| x & mask != 0) {
                return true;
            }
            i += take;
        }
        false
    }

    #[cfg(test)]
    pub(super) fn program(&self) -> &BlockProgram {
        &self.program
    }
}

#[derive(Default)]
struct Builder {
    lines: bool,
    ops: Vec<BlockOp>,
    tables: Vec<ClassTable>,
    /// Class bits given out so far: table `n / 8`, bit `n % 8`.
    bits: usize,
    memo: HashMap<BlockOp, u16>,
    sets: HashMap<[u64; 4], u16>,
}

impl Builder {
    fn push(&mut self, op: BlockOp) -> u16 {
        if let Some(&v) = self.memo.get(&op) {
            return v;
        }
        let v = self.ops.len() as u16;
        self.ops.push(op);
        self.memo.insert(op, v);
        v
    }

    /// The stream's bytes in a set, as one `class` per table (or `eq` for one byte), inside the stream.
    fn set(&mut self, set: [u64; 4]) -> u16 {
        if let Some(&v) = self.sets.get(&set) {
            return v;
        }
        let has = |b: usize| set[b / 64] >> (b % 64) & 1 == 1;
        let within = self.push(BlockOp::Within(0));
        let members: Vec<usize> = (0..256).filter(|&b| has(b)).collect();
        let raw = match members.as_slice() {
            [] => self.push(BlockOp::Const(0)),
            [b] => self.push(BlockOp::Eq(*b as u8)),
            _ => {
                // A class bit is a product of a low-nibble set and a high-nibble set: one per distinct low set.
                let mut by_lo: Vec<(u16, u16)> = Vec::new();
                for hi in 0..16 {
                    let lo: u16 = (0..16).filter(|&lo| has(hi << 4 | lo)).map(|lo| 1u16 << lo).sum();
                    if lo == 0 {
                        continue;
                    }
                    match by_lo.iter_mut().find(|(l, _)| *l == lo) {
                        Some((_, h)) => *h |= 1 << hi,
                        None => by_lo.push((lo, 1 << hi)),
                    }
                }
                let mut per_table: Vec<(u16, u8)> = Vec::new();
                for (lo, hi) in by_lo {
                    let (t, bit) = (self.bits / 8, 1u8 << (self.bits % 8));
                    self.bits += 1;
                    if t == self.tables.len() {
                        self.tables.push(ClassTable { lo: [0; 16], hi: [0; 16] });
                    }
                    for l in 0..16 {
                        if lo >> l & 1 == 1 {
                            self.tables[t].lo[l] |= bit;
                        }
                        if hi >> l & 1 == 1 {
                            self.tables[t].hi[l] |= bit;
                        }
                    }
                    match per_table.iter_mut().find(|(tt, _)| usize::from(*tt) == t) {
                        Some((_, bits)) => *bits |= bit,
                        None => per_table.push((t as u16, bit)),
                    }
                }
                let parts: Vec<u16> = per_table.into_iter().map(|(table, bits)| self.push(BlockOp::Class { table, bits })).collect();
                parts.into_iter().reduce(|a, b| self.push(BlockOp::Or(a, b))).expect("a class bit")
            }
        };
        let v = self.push(BlockOp::And(raw, within));
        self.sets.insert(set, v);
        v
    }

    fn bytes(&mut self, f: &dyn Fn(u8) -> bool) -> u16 {
        let mut set = [0u64; 4];
        for b in 0..=255u8 {
            if f(b) {
                set[usize::from(b) / 64] |= 1 << (b % 64);
            }
        }
        self.set(set)
    }

    fn consume(&mut self, m: u16, set: u16) -> u16 {
        let a = self.push(BlockOp::And(m, set));
        self.push(BlockOp::Prev(a, 1))
    }

    /// One byte set per position of each length of a class's UTF-8 sequences.
    fn class_steps(class: &Class) -> Vec<Vec<[u64; 4]>> {
        let mut by_len: Vec<Vec<[u64; 4]>> = Vec::new();
        let mut add = |seq: &[(u8, u8)]| {
            while by_len.len() < seq.len() {
                by_len.push(Vec::new());
            }
            let steps = &mut by_len[seq.len() - 1];
            if steps.is_empty() {
                steps.resize(seq.len(), [0; 4]);
            }
            for (k, &(lo, hi)) in seq.iter().enumerate() {
                for b in lo..=hi {
                    steps[k][usize::from(b) / 64] |= 1 << (b % 64);
                }
            }
        };
        match class {
            Class::Unicode(c) => {
                for r in c.iter() {
                    for seq in regex_syntax::utf8::Utf8Sequences::new(r.start(), r.end()) {
                        let s: Vec<(u8, u8)> = seq.as_slice().iter().map(|r| (r.start, r.end)).collect();
                        add(&s);
                    }
                }
            }
            Class::Bytes(c) => {
                for r in c.iter() {
                    add(&[(r.start(), r.end())]);
                }
            }
        }
        by_len.into_iter().filter(|s| !s.is_empty()).collect()
    }

    fn class(&mut self, class: &Class, m: u16) -> u16 {
        let mut out: Option<u16> = None;
        for steps in Self::class_steps(class) {
            let mut x = m;
            for set in steps {
                let s = self.set(set);
                x = self.consume(x, s);
            }
            out = Some(match out {
                Some(o) => self.push(BlockOp::Or(o, x)),
                None => x,
            });
        }
        out.unwrap_or_else(|| self.push(BlockOp::Const(0)))
    }

    /// The bytes a class's characters can hold, in any position.
    fn class_bytes(class: &Class) -> [u64; 4] {
        let mut all = [0u64; 4];
        for steps in Self::class_steps(class) {
            for s in steps {
                for (a, b) in all.iter_mut().zip(s) {
                    *a |= b;
                }
            }
        }
        all
    }

    /// The bytes a run of `hir` holds, when it is one character or a class of them.
    fn run_bytes(hir: &Hir) -> Option<[u64; 4]> {
        match hir.kind() {
            HirKind::Class(c) => Some(Self::class_bytes(c)),
            HirKind::Literal(lit) if std::str::from_utf8(&lit.0).is_ok_and(|t| t.chars().count() == 1) => {
                let mut set = [0u64; 4];
                for &b in lit.0.iter() {
                    set[usize::from(b) / 64] |= 1 << (b % 64);
                }
                Some(set)
            }
            HirKind::Capture(c) => Self::run_bytes(&c.sub),
            _ => None,
        }
    }

    fn look(&mut self, look: Look, m: u16) -> u16 {
        let within = self.push(BlockOp::Within(0));
        let before = self.push(BlockOp::Prev(within, 1));
        let first = self.push(BlockOp::AndNot(within, before));
        let last = self.push(BlockOp::AndNot(before, within));
        let nl = self.push(BlockOp::Eq(b'\n'));
        let held = match look {
            Look::Start if !self.lines => first,
            Look::End if !self.lines => last,
            Look::StartLF => {
                let after = self.push(BlockOp::Prev(nl, 1));
                self.push(BlockOp::Or(first, after))
            }
            Look::EndLF if !self.lines => {
                let at = self.push(BlockOp::And(nl, within));
                self.push(BlockOp::Or(at, last))
            }
            Look::WordAscii | Look::WordAsciiNegate => {
                let w = self.bytes(&|b| b.is_ascii_alphanumeric() || b == b'_');
                let pw = self.push(BlockOp::Prev(w, 1));
                let edge = self.push(BlockOp::Xor(pw, w));
                if look == Look::WordAscii { edge } else { self.push(BlockOp::Not(edge)) }
            }
            // The others (Unicode word edges, CRLF lines, the text's edges within a line) are not checked: wider.
            _ => return m,
        };
        self.push(BlockOp::And(m, held))
    }

    /// The markers after `hir` from markers `m`; `None` when it grows past the budget.
    fn hir(&mut self, hir: &Hir, m: u16) -> Option<u16> {
        if self.ops.len() > MAX_OPS {
            return None;
        }
        Some(match hir.kind() {
            HirKind::Empty => m,
            HirKind::Literal(lit) => {
                let mut x = m;
                for &b in lit.0.iter() {
                    let s = self.bytes(&|y| y == b);
                    x = self.consume(x, s);
                }
                x
            }
            HirKind::Class(c) => self.class(c, m),
            HirKind::Look(l) => self.look(*l, m),
            HirKind::Capture(c) => self.hir(&c.sub, m)?,
            HirKind::Concat(xs) => {
                let mut x = m;
                for h in xs {
                    x = self.hir(h, x)?;
                }
                x
            }
            HirKind::Alternation(xs) => {
                let mut out: Option<u16> = None;
                for h in xs {
                    let x = self.hir(h, m)?;
                    out = Some(match out {
                        Some(o) => self.push(BlockOp::Or(o, x)),
                        None => x,
                    });
                }
                out.unwrap_or(m)
            }
            HirKind::Repetition(r) => {
                let mut x = m;
                for _ in 0..r.min {
                    x = self.hir(&r.sub, x)?;
                }
                match r.max {
                    Some(max) if max - r.min <= 16 => {
                        for _ in r.min..max {
                            let y = self.hir(&r.sub, x)?;
                            x = self.push(BlockOp::Or(x, y));
                        }
                        x
                    }
                    _ => match Self::run_bytes(&r.sub) {
                        // A run of a class's bytes (or one character's): one fill. Exact for one-byte characters.
                        Some(set) => {
                            let s = self.set(set);
                            let p = self.push(BlockOp::Prev(s, 1));
                            let stop = self.push(BlockOp::Not(p));
                            self.push(BlockOp::Fill(x, stop))
                        }
                        // Anything later: every position after a marker.
                        None => {
                            let zero = self.push(BlockOp::Const(0));
                            self.push(BlockOp::Fill(x, zero))
                        }
                    },
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every byte after which some substring ending there matches the pattern whole: the exact set Parabix marks.
    fn brute(re: &str, text: &str) -> Vec<usize> {
        let whole = regex::Regex::new(&format!("^(?:{re})$")).unwrap();
        let b = text.as_bytes();
        let mut out = Vec::new();
        for e in 1..=b.len() {
            if !text.is_char_boundary(e) {
                continue;
            }
            if (0..e).filter(|&s| text.is_char_boundary(s)).any(|s| whole.is_match(&text[s..e])) {
                out.push(e - 1);
            }
        }
        out
    }

    /// The ends of the matches the host finds (`find_iter`), as marks.
    fn found(re: &str, text: &str) -> Vec<usize> {
        regex::Regex::new(re).unwrap().find_iter(text).filter(|m| m.end() > m.start()).map(|m| m.end() - 1).collect()
    }

    fn marks(p: &SpanPrefilter, text: &str) -> Vec<usize> {
        let ends = p.ends(text.as_bytes());
        (0..text.len()).filter(|&i| ends.get(i / 64).is_some_and(|w| w >> (i % 64) & 1 == 1)).collect()
    }

    const TEXTS: &[&str] = &[
        "x = 1  # SEC1 SEC22\nclass Foo:\n    pass\nclass Bar: pass\n",
        "aaa ab abab b\nba\n\nabba",
        "Grεek Α, Bαd Р, Русский; p）q",
        "",
        "a",
    ];

    /// The marks are a superset of the ends of the matches the host finds; for a pattern without look-arounds that
    /// the translation renders exactly, they are every end of every substring that matches.
    #[test]
    fn a_patterns_block_program_marks_every_end_and_exact_ones_only_those() {
        let long = "ab ".repeat(40) + "SEC123 tail";
        let cases: &[(&str, bool)] = &[
            ("SEC\\d+", true),
            ("ab|ba", true),
            ("a+b", true),
            ("(?<w>[a-z]+)", true),
            ("x = (?<v>\\d+).*", false),
            ("(?m)^class\\s+(?<c>\\w+)", false),
            ("\\bab\\b", false),
            ("(?-u:\\b)ab", false),
            ("[\\p{Alphabetic}\\p{N}]+", false),
            ("(?<ch>[\\x{a0}´¸×])", true),
            ("(ab)*c", false),
            ("a{2,3}", true),
        ];
        for &(re, exact) in cases {
            let p = SpanPrefilter::compile(re, false).unwrap_or_else(|| panic!("{re} compiles"));
            for text in TEXTS.iter().copied().chain([long.as_str()]) {
                let (got, want) = (marks(&p, text), found(re, text));
                assert!(want.iter().all(|e| got.contains(e)), "{re} over {text:?}: misses {:?}", want.iter().filter(|e| !got.contains(e)).collect::<Vec<_>>());
                if exact {
                    assert_eq!(got, brute(re, text), "{re} over {text:?}");
                }
            }
        }
        assert!(SpanPrefilter::compile("a*", false).is_none(), "a pattern matching the empty string is the host's");
    }

    /// The program is the same on every executor of block programs: the CPU interpreter, Cranelift and, on a Mac,
    /// Metal's interpreter and generated MSL.
    #[test]
    fn a_patterns_block_program_runs_alike_on_every_executor() {
        for re in ["SEC\\d+", "(?m)^class\\s+(?<c>\\w+)", "[\\p{Alphabetic}\\p{N}]+"] {
            let p = SpanPrefilter::compile(re, false).unwrap();
            let program = p.program();
            let mut rows = String::new();
            for (i, t) in program.tables.iter().enumerate() {
                let hex = |x: &[u8; 16]| x.iter().map(|b| format!("{b:02x}")).collect::<String>();
                rows.push_str(&format!("  block 0 table {i} lo={} hi={}\n", hex(&t.lo), hex(&t.hi)));
            }
            for (i, op) in program.ops.iter().enumerate() {
                rows.push_str(&format!("  block 0 %{i} = {}\n", op.text()));
            }
            rows.push_str(&format!("  block 0 output %{}\n  block 0 delay 1\n", program.outputs[0]));
            let assembled = super::super::rir::assemble(&format!("program -\n  mode set=0 query=0\n{rows}  0000 STOP 0\n")).unwrap();
            let executors = super::super::block_tests::Executors::new(&assembled);
            for text in TEXTS {
                let (runs, _) = executors.run(text.as_bytes(), true);
                let want = p.ends(text.as_bytes());
                for (name, got) in runs {
                    assert_eq!(got[0], want, "{re} on {name} over {text:?}");
                }
            }
        }
    }
}
