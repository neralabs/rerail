//! The one pass's masks read from the bytes where the token loop reads them (decision 0623 step 1), as the lean
//! lexer reads its window's: no stage 1 and no mask of the whole file.
//!
//! - A test at a byte (`bit(&M, i)`) is a test of the bytes there, as lean's `c == b'/' && d == b'/'`.
//! - A search or a count over bytes (`first_bit`, `count_in`, `first_in`, `first_clear`, `any_in`, a run's end) reads
//!   the mask 64 bytes at a time from its first byte, each window made of the bytes loaded where they are
//!   (`rerail_simd8`): a class by its byte ranges, `prev` and `next` as the window a byte back or on, `escaped` word by
//!   word with its carry from the search's first byte.
//! - A block's token starts (`!skip[b]`) are the block's window.
//!
//! Written where every mask the loop reads has that form; elsewhere the pass keeps stage 1 (`Gen::source`).

use std::collections::HashMap;
use std::fmt::Write as _;

use ::bitlex::syntax::{Arg, Expr, Stmt};

/// The masks' and the classes' definitions, the last of each name, and the window functions written so far.
pub(super) struct Bytes<'a> {
    classes: HashMap<&'a str, &'a [(u32, u32)]>,
    defs: HashMap<String, Expr>,
    /// Each mask's expression, by the local the token loop names it by (`m_NAME`, a temporary `tK`).
    locals: HashMap<String, Expr>,
    /// The window functions written, by the local.
    written: Vec<String>,
    /// Each class's window function, by name.
    cls: Vec<String>,
    code: String,
    /// The masks the loop reads from its block's word (`k_NAME`, made once a block, as the lean lexer's `Window`).
    block: Vec<String>,
    /// Each class's bits as stage 1 makes them of the byte vector `v` (`cP`: its nibble tables' pair `P`, simdjson's
    /// `classify`), by name.
    class_code: HashMap<String, String>,
    /// Whether the line read is in a region's row: its reads go from the opener, as the lean lexer's micro-lexers.
    in_region: bool,
    /// The operators' pairs masks the loop reads (`k_pair_K`): each row's pairs of bytes, a pair's first byte's bit where
    /// the second is the byte after (lean's `pair`), made of the block's classes too.
    pub(super) pairs: Vec<(String, Vec<(u8, u8)>)>,
    /// The window functions that read one window's escapes (`escaped_step` of a mask), by name, and that mask: each has
    /// a twin (`e_NAME`) given the window's escaped bytes, so tests read in one window loop find them once (`fuse_tests`).
    pub(super) escapes: HashMap<String, String>,
    /// The bytes each such mask holds, where it is a class (`escapes`' masks): a row whose first byte none of them may be
    /// starts no escape, so its region's end and tests read in one loop from its opener (`fuse_read`).
    pub(super) escape_bytes: HashMap<String, [bool; 256]>,
}

/// The calls of the loop that read a mask: their name, and how many of their first arguments are masks.
const READS: [(&str, usize); 8] =
    [("run_once_last", 2), ("first_clear", 1), ("first_bit", 1), ("count_in", 1), ("first_in", 1), ("run_last", 1), ("any_in", 1), ("bit", 1)];

impl<'a> Bytes<'a> {
    pub(super) fn new(stmts: &'a [Stmt], locals: HashMap<String, Expr>, class_code: HashMap<String, String>) -> Self {
        let mut classes = HashMap::new();
        let mut defs = HashMap::new();
        for s in stmts {
            match s {
                Stmt::Class(n, rs) => {
                    classes.insert(n.as_str(), rs.as_slice());
                    defs.remove(n);
                }
                Stmt::Mask(n, e) => {
                    defs.insert(n.clone(), e.clone());
                    classes.remove(n.as_str());
                }
                _ => {}
            }
        }
        Bytes { classes, defs, locals, written: Vec::new(), cls: Vec::new(), code: String::new(), block: Vec::new(), class_code, in_region: false, pairs: Vec::new(), escapes: HashMap::new(), escape_bytes: HashMap::new() }
    }

    /// `prev(E, k)` and `next(E, k)`: the call's mask and its distance, back (negative) or on.
    fn shift(f: &str, args: &[Arg]) -> Option<(Expr, i64)> {
        let e = match args.first() {
            Some(Arg::E(e)) => e.clone(),
            _ => return None,
        };
        let k = match args.get(1) {
            None => 1,
            Some(Arg::N(k)) => i64::try_from(*k).ok()?,
            _ => return None,
        };
        match f {
            "prev" => Some((e, -k)),
            "next" => Some((e, k)),
            _ => None,
        }
    }

    /// `e` at byte `at` (Rust, a `usize`), a test of the bytes there: none past the file or before it. At a token's
    /// first byte (`s`) the byte is in the file.
    fn at(&self, e: &Expr, at: &str, depth: usize) -> Result<String, String> {
        if depth > 64 {
            return Err("a mask's definition reads itself".into());
        }
        Ok(match e {
            Expr::Empty => "false".to_owned(),
            Expr::Name(n) => {
                if let Some(rs) = self.classes.get(n.as_str()) {
                    if at == "s" { pattern_test("src[s]", rs) } else { format!("({at} < n && {})", pattern_test(&format!("src[{at}]"), rs)) }
                } else if let Some(d) = self.defs.get(n) {
                    self.at(d, at, depth + 1)?
                } else {
                    return Err(format!("`{n}` is no class or mask of the bytes"));
                }
            }
            Expr::Not(a) if at == "s" => format!("!{}", self.at(a, at, depth + 1)?),
            Expr::Not(a) => format!("({at} < n && !{})", self.at(a, at, depth + 1)?),
            Expr::And(a, b) => format!("({} && {})", self.at(a, at, depth + 1)?, self.at(b, at, depth + 1)?),
            Expr::Or(a, b) => format!("({} || {})", self.at(a, at, depth + 1)?, self.at(b, at, depth + 1)?),
            Expr::Xor(a, b) => format!("({} != {})", self.at(a, at, depth + 1)?, self.at(b, at, depth + 1)?),
            Expr::Call(f, args) => match Self::shift(f, args) {
                Some((a, k)) if k < 0 => format!("({at} >= {} && {})", -k, self.at(&a, &format!("({at} - {})", -k), depth + 1)?),
                Some((a, k)) => self.at(&a, &format!("({at} + {k})"), depth + 1)?,
                None => return Err(format!("`{f}` is no test of the bytes at one")),
            },
            Expr::Test { .. } => return Err("a machine's test is no test of the bytes".into()),
        })
    }

    /// `e`'s bits in the 64 bytes from byte `p + off`: none past the file or before it. `escaped`: the search's
    /// carried escapes (`esc`), from its first byte.
    fn win(&mut self, e: &Expr, off: i64, depth: usize) -> Result<String, String> {
        if depth > 64 {
            return Err("a mask's definition reads itself".into());
        }
        let p = match off {
            0 => "p".to_owned(),
            o if o < 0 => format!("(p - {})", -o),
            o => format!("(p + {o})"),
        };
        Ok(match e {
            Expr::Empty => "0u64".to_owned(),
            Expr::Name(n) => {
                if self.classes.contains_key(n.as_str()) {
                    let f = format!("cls_{}", ident(n));
                    // A class without the space: the padding after the file (spaces, `PADDING`) holds none of it, so its
                    // bits need no end, as the lean lexer reads its padded bytes.
                    let held = if self.classes[n.as_str()].iter().any(|&(lo, hi)| lo <= 0x20 && 0x20 <= hi) { "held(b, q, n, p)" } else { "padded(b, q, p)" };
                    if !self.cls.contains(&f)
                        && let Some(code) = self.class_code.get(n.as_str())
                    {
                        self.cls.push(f.clone());
                        // Stage 1's bits of the class: its pairs of nibble tables, then one test (simdjson's `classify`).
                        let code = code.trim_end_matches(" & lv").to_owned();
                        let mut lines = String::new();
                        for k in 0..8 {
                            if code.contains(&format!("c{k},")) || code.contains(&format!("c{k})")) {
                                let _ = writeln!(lines, "    let c{k} = unsafe {{ S::classify(v, &LO_{k}, &HI_{k}) }};");
                            }
                        }
                        let _ = write!(
                            self.code,
                            "/// Class `{n}`'s bits in the 64 bytes from `p`: none past the file or before it.\n///\n/// # Safety\n/// The CPU runs `S`'s backend.\n#[inline(always)]\nunsafe fn {f}<S: rerail_simd8::Simd8x64>(src: &[u8], n: usize, p: isize) -> u64 {{\n    let Some((v, q)) = (unsafe {{ window::<S>(src, n, p) }}) else {{ return 0 }};\n{lines}    // SAFETY: the caller's.\n    let b = unsafe {{ {code} }};\n    {held}\n}}\n\n"
                        );
                    }
                    if !self.cls.contains(&f) {
                        self.cls.push(f.clone());
                        let rs = self.classes[n.as_str()];
                        let bits: Vec<String> = rs
                            .iter()
                            .map(|&(lo, hi)| match (lo, hi) {
                                (lo, hi) if lo == hi => format!("S::eq_mask(v, {lo:#04x})"),
                                (lo, 255) => format!("S::to_bitmask(S::ge(v, {lo:#04x}))"),
                                (0, hi) => format!("!S::to_bitmask(S::ge(v, {:#04x}))", hi + 1),
                                (lo, hi) => format!("(S::to_bitmask(S::ge(v, {lo:#04x})) & !S::to_bitmask(S::ge(v, {:#04x})))", hi + 1),
                            })
                            .collect();
                        let _ = write!(
                            self.code,
                            "/// Class `{n}`'s bits in the 64 bytes from `p`: none past the file or before it.\n///\n/// # Safety\n/// The CPU runs `S`'s backend.\n#[inline(always)]\nunsafe fn {f}<S: rerail_simd8::Simd8x64>(src: &[u8], n: usize, p: isize) -> u64 {{\n    let Some((v, q)) = (unsafe {{ window::<S>(src, n, p) }}) else {{ return 0 }};\n    // SAFETY: the caller's.\n    let b = unsafe {{ {} }};\n    {held}\n}}\n\n",
                            if bits.is_empty() { "0".to_owned() } else { bits.join(" | ") }
                        );
                    }
                    format!("unsafe {{ {f}::<S>(src, n, {p}) }}")
                } else if let Some(d) = self.defs.get(n).cloned() {
                    self.win(&d, off, depth + 1)?
                } else {
                    return Err(format!("`{n}` is no class or mask of the bytes"));
                }
            }
            Expr::Not(a) => format!("(!{} & valid(n, {p}))", self.win(a, off, depth + 1)?),
            Expr::And(a, b) => {
                let (a, b) = (self.win(a, off, depth + 1)?, self.win(b, off, depth + 1)?);
                // A test of several windows after one that rarely holds (a `\u` escape's digits after its `\u`): read
                // only where the first holds, as the lean lexer unescapes a string only when it saw a `\u`. A side
                // that carries escapes is always read, its carry the next window's.
                if b.matches("cls_").count() >= 3 && !b.contains("esc)") {
                    format!("{{ let a = {a}; if a == 0 {{ 0 }} else {{ a & {b} }} }}")
                } else {
                    format!("({a} & {b})")
                }
            }
            Expr::Or(a, b) => format!("({} | {})", self.win(a, off, depth + 1)?, self.win(b, off, depth + 1)?),
            Expr::Xor(a, b) => format!("({} ^ {})", self.win(a, off, depth + 1)?, self.win(b, off, depth + 1)?),
            Expr::Call(f, args) if f == "escaped" && off == 0 => {
                let Some(Arg::E(b)) = args.first() else { return Err("`escaped` of no mask".into()) };
                let b = self.win(b, 0, depth + 1)?;
                format!("escaped_step({b}, esc)")
            }
            Expr::Call(f, args) => match Self::shift(f, args) {
                Some((a, k)) => self.win(&a, off + k, depth + 1)?,
                None => return Err(format!("`{f}` is no mask of the bytes 64 at a time")),
            },
            Expr::Test { .. } => return Err("a machine's test is no mask of the bytes".into()),
        })
    }

    /// The local's expression.
    fn expr(&self, local: &str) -> Result<Expr, String> {
        self.locals.get(local).cloned().ok_or_else(|| format!("`{local}` is no mask of the bytes"))
    }

    /// The window function of the local `m` (`w_m`), written once; whether it reads escapes (its windows must then
    /// start where the carry is none: a token's first byte, or the one after).
    fn window_fn(&mut self, m: &str) -> Result<(String, bool), String> {
        let f = format!("w_{m}");
        let e = self.expr(m)?;
        let bits = self.win(&e, 0, 0)?;
        let esc = bits.contains("esc)");
        if !self.written.contains(&f) {
            self.written.push(f.clone());
            let _ = write!(
                self.code,
                "/// `{m}`'s bits in the 64 bytes from `p`, from the bytes: none past the file or before it.\n///\n/// # Safety\n/// The CPU runs `S`'s backend.\n#[inline(always)]\n#[allow(unused_variables)]\nunsafe fn {f}<S: rerail_simd8::Simd8x64>(src: &[u8], n: usize, p: isize, esc: &mut u64) -> u64 {{\n    {bits}\n}}\n\n"
            );
            // Its twin given the window's escaped bytes, where it reads them of one mask.
            if bits.matches("escaped_step(").count() == 1
                && let Some(at) = bits.find("escaped_step(")
                && let Some(len) = bits[at..].find(", esc)")
            {
                let of = bits[at + "escaped_step(".len()..at + len].to_owned();
                let twin = bits.replacen(&format!("escaped_step({of}, esc)"), "escaped", 1);
                let _ = write!(
                    self.code,
                    "/// `{f}` given its window's escaped bytes (`escaped`).\n///\n/// # Safety\n/// The CPU runs `S`'s backend.\n#[inline(always)]\n#[allow(unused_variables)]\nunsafe fn e_{m}<S: rerail_simd8::Simd8x64>(src: &[u8], n: usize, p: isize, escaped: u64) -> u64 {{\n    {twin}\n}}\n\n"
                );
                // The class's bytes, where the escapes are of one class.
                if let Some(c) = of.strip_prefix("unsafe { cls_").and_then(|c| c.strip_suffix("::<S>(src, n, p) }"))
                    && let Some((name, rs)) = self.classes.iter().find(|(name, _)| ident(name) == c)
                {
                    let _ = name;
                    let held: [bool; 256] = std::array::from_fn(|b| rs.iter().any(|&(lo, hi)| lo as usize <= b && b <= hi as usize));
                    self.escape_bytes.insert(of.clone(), held);
                }
                self.escapes.insert(f.clone(), of);
            }
        }
        Ok((format!("|p: isize, esc: &mut u64| unsafe {{ {f}::<S>(src, n, p, esc) }}"), esc))
    }

    /// One read (`name(args)`) of the loop as the bytes' form.
    fn read(&mut self, name: &str, args: &[String], line: &str) -> Result<Option<String>, String> {
        let mask = |a: &str| a.trim().strip_prefix('&').map(|m| m.strip_suffix("[..made]").unwrap_or(m).to_owned());
        let Some(m) = mask(&args[0]) else { return Ok(None) };
        // The test build's checks read the walk's masks, which stay the walk's.
        if !self.locals.contains_key(&m) {
            return if line.contains("assert") { Ok(None) } else { Err(format!("`{m}` is no mask of the bytes")) };
        }
        let from_start = |a: &str| matches!(a.trim(), "s" | "s + 1");
        let windowed = |me: &mut Self, from: &str| -> Result<String, String> {
            let (f, esc) = me.window_fn(&m)?;
            if esc && !from_start(from) {
                return Err(format!("`{m}` reads escapes from no token's start"));
            }
            Ok(f)
        };
        // A read of one byte (`first_clear(&M, a, a + 1)`): a test of the bytes there.
        if name == "first_clear" && args.len() == 3 && args[2].trim() == format!("{} + 1", args[1].trim()) {
            let e = self.expr(&m)?;
            let a = args[1].trim();
            return Ok(Some(format!("(!{}).then_some({a})", self.at(&e, a, 0)?)));
        }
        // A mask with no escapes: its block's word first, made once a block (`k_M`), then windows past the block.
        if name != "bit" && !self.in_region && !self.window_fn(&m)?.1 {
            let k = self.block_word(&m);
            let (f, _) = self.window_fn(&m)?;
            return Ok(Some(match (name, args.len()) {
                ("first_bit", 2) => format!("bfirst_bit(n, base, {k}, {}, {f})", args[1].trim()),
                ("count_in", 3) => format!("bcount_in(n, base, {k}, {}, {}, {f})", args[1].trim(), args[2].trim()),
                ("first_in", 3) => format!("bfirst_in(n, base, {k}, {}, {}, {f})", args[1].trim(), args[2].trim()),
                ("any_in", 3) => format!("bfirst_in(n, base, {k}, {}, {}, {f}).is_some()", args[1].trim(), args[2].trim()),
                ("first_clear", 3) => format!("bfirst_clear(n, base, {k}, {}, {}, {f})", args[1].trim(), args[2].trim()),
                ("run_last", 2) => format!("brun_last(n, base, {k}, {}, {f})", args[1].trim()),
                ("run_once_last", 3) => {
                    let Some(d) = mask(&args[1]) else { return Ok(None) };
                    let (fd, esc) = self.window_fn(&d)?;
                    if esc {
                        return Err(format!("`{d}` reads escapes"));
                    }
                    let kd = self.block_word(&d);
                    format!("brun_once_last(n, base, {k}, {kd}, {}, {f}, {fd})", args[2].trim())
                }
                _ => return Err(format!("`{name}` with {} arguments has no form of the bytes", args.len())),
            }));
        }
        Ok(Some(match (name, args.len()) {
            ("bit", 2) => {
                let e = self.expr(&m)?;
                format!("({})", self.at(&e, args[1].trim(), 0)?)
            }
            ("first_bit", 2) => format!("wfirst_bit(n, {}, {})", args[1].trim(), windowed(self, &args[1])?),
            ("count_in", 3) => format!("wcount_in(n, {}, {}, {})", args[1].trim(), args[2].trim(), windowed(self, &args[1])?),
            ("first_in", 3) => format!("wfirst_in(n, {}, {}, {})", args[1].trim(), args[2].trim(), windowed(self, &args[1])?),
            ("any_in", 3) => format!("wfirst_in(n, {}, {}, {}).is_some()", args[1].trim(), args[2].trim(), windowed(self, &args[1])?),
            ("first_clear", 3) => format!("wfirst_clear(n, {}, {}, {})", args[1].trim(), args[2].trim(), windowed(self, &args[1])?),
            ("run_last", 2) => format!("wrun_last(n, {}, {})", args[1].trim(), windowed(self, "s")?),
            ("run_once_last", 3) => {
                let Some(d) = mask(&args[1]) else { return Ok(None) };
                let r = windowed(self, "s")?;
                let (fd, esc) = self.window_fn(&d)?;
                if esc {
                    return Err(format!("`{d}` reads escapes"));
                }
                format!("wrun_once_last(n, {}, {r}, {fd})", args[2].trim())
            }
            _ => return Err(format!("`{name}` with {} arguments has no form of the bytes", args.len())),
        }))
    }

    /// `code` with each read of a mask in the bytes' form.
    pub(super) fn rewrite(&mut self, code: &str) -> Result<String, String> {
        let mut out = String::new();
        self.in_region = false;
        for line in code.split_inclusive('\n') {
            // A row's comment names its rule (`// `kind`: rule`): a region's is `read …`.
            if let Some(rule) = line.trim_start().strip_prefix("// `").and_then(|r| r.split_once("`: ")).map(|(_, r)| r) {
                self.in_region = rule.starts_with("read ");
            }
            out.push_str(&self.rewrite_line(line, line)?);
        }
        Ok(out)
    }

    fn rewrite_line(&mut self, text: &str, line: &str) -> Result<String, String> {
        let mut out = String::new();
        let mut rest = text;
        'scan: while !rest.is_empty() {
            for (name, _) in READS {
                let pat = format!("{name}(&");
                if rest.starts_with(&pat) && !out.chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    let inner = &rest[name.len() + 1..];
                    let Some(close) = closing(inner) else { return Err(format!("an unclosed call in `{}`", line.trim())) };
                    let args: Vec<String> = split_args(&inner[..close]).into_iter().map(|a| self.rewrite_line(&a, line)).collect::<Result<_, _>>()?;
                    match self.read(name, &args, line)? {
                        Some(r) => out.push_str(&r),
                        None => {
                            let _ = write!(out, "{name}({})", args.join(","));
                        }
                    }
                    rest = &inner[close + 1..];
                    continue 'scan;
                }
            }
            let c = rest.chars().next().expect("not empty");
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
        Ok(out)
    }

    /// The block's word of mask `m` (`k_m`), made once a block.
    fn block_word(&mut self, m: &str) -> String {
        if !self.block.iter().any(|b| b == m) {
            self.block.push(m.to_owned());
        }
        format!("k_{m}")
    }

    /// The block's token starts: the bytes `skip` does not hold, from block `b`'s word.
    pub(super) fn starts(&mut self, skip: &str) -> Result<String, String> {
        let (_, esc) = self.window_fn(skip)?;
        if esc {
            return Err(format!("`{skip}` reads escapes"));
        }
        Ok(format!("!{}", self.block_word(skip)))
    }

    /// The block's words, made once a block where the loop comes to it, as the lean lexer's `Window`: each class the
    /// words read classified once a block (`block_classes`, one load and one classify a pair of nibble tables), a class
    /// a few bytes on or back its bits shifted, the neighbour block's shifted in. Each block's classes are made once: the
    /// next block's are its `ahead`, and the last block's its `behind`. With the declarations the loop's head needs
    /// before it. A word of escapes, or of a class 64 bytes away, reads its windows as a window past the block.
    pub(super) fn block_words(&mut self) -> (String, String) {
        match self.block_from_classes() {
            Some(r) => r,
            None => {
                let mut out = String::from("        // The block's masks, from its bytes (the lean lexer's `Window`): what its tokens read of it.\n");
                for m in &self.block {
                    let _ = writeln!(out, "        let k_{m} = unsafe {{ w_{m}::<S>(src, n, base as isize, &mut 0) }};");
                }
                for (f, _) in &self.pairs {
                    let _ = writeln!(out, "        let k_{f} = unsafe {{ w_{f}::<S>(src, base) }};");
                }
                (String::new(), out)
            }
        }
    }

    /// `block_words` from the block's classes: none where a word reads escapes or a class 64 bytes away.
    fn block_from_classes(&mut self) -> Option<(String, String)> {
        let mut used: Vec<String> = Vec::new();
        let (mut back, mut on) = (false, false);
        let mut words = Vec::new();
        for m in self.block.clone() {
            let e = self.expr(&m).ok()?;
            words.push((m, self.blk(&e, 0, 0, &mut used, &mut back, &mut on)?));
        }
        // Each pairs mask of the block's one load too, as lean's `pair`: the byte's nibble tables name the pairs it
        // starts, the next byte's (`shift_in`) the pairs it ends; a pair is a bit of both, packed once. Eight pairs a
        // pair of tables.
        let mut statics = String::new();
        let mut shifted = false;
        for (f, two) in self.pairs.clone() {
            let mut groups = Vec::new();
            for (g, chunk) in two.chunks(8).enumerate() {
                let (mut flo, mut fhi, mut slo, mut shi) = ([0u8; 16], [0u8; 16], [0u8; 16], [0u8; 16]);
                for (bit, &(a, b)) in chunk.iter().enumerate() {
                    flo[usize::from(a & 15)] |= 1 << bit;
                    fhi[usize::from(a >> 4)] |= 1 << bit;
                    slo[usize::from(b & 15)] |= 1 << bit;
                    shi[usize::from(b >> 4)] |= 1 << bit;
                }
                let t = format!("{}_{g}", f.to_uppercase());
                let row = |t: &[u8; 16]| t.iter().map(|b| format!("{b:#04x}")).collect::<Vec<_>>().join(", ");
                let _ = write!(statics, "/// `{f}`'s pairs ({}): a first byte's bits the pairs it starts, a second's those it ends.\nstatic {t}_FIRST: ([u8; 16], [u8; 16]) = ([{}], [{}]);\nstatic {t}_SECOND: ([u8; 16], [u8; 16]) = ([{}], [{}]);\n\n", chunk.iter().map(|(a, b)| format!("`{}{}`", *a as char, *b as char)).collect::<Vec<_>>().join(", "), row(&flo), row(&fhi), row(&slo), row(&shi));
                let c = format!("{f}_{g}");
                self.class_code.insert(c.clone(), format!("S::to_bitmask(S::test(S::classify(v, &{t}_FIRST.0, &{t}_FIRST.1), S::classify(d, &{t}_SECOND.0, &{t}_SECOND.1)))"));
                used.push(c);
                groups.push(format!("cur[{}]", used.len() - 1));
                shifted = true;
            }
            words.push((f, if groups.is_empty() { "0u64".to_owned() } else { groups.join(" | ") }));
        }
        if used.is_empty() {
            return None;
        }
        // The classes' bits of the block at `base` (in the file), each its class's test of its nibble tables' pair.
        let mut tables = String::new();
        for k in 0..8 {
            if used.iter().any(|c| self.class_code.get(c.as_str()).is_some_and(|code| code.contains(&format!("c{k},")) || code.contains(&format!("c{k})")))) {
                let _ = writeln!(tables, "        let c{k} = S::classify(v, &LO_{k}, &HI_{k});");
            }
        }
        let bits: Vec<String> = used.iter().map(|c| self.class_bits(c)).collect::<Option<_>>()?;
        self.code.push_str(&statics);
        if shifted {
            // The block a byte on: its last byte the next block's first, in the padded source.
            tables.insert_str(0, "        let d = S::shift_in(v, src[base + 64]);\n");
        }
        let k = used.len();
        let _ = write!(
            self.code,
            "/// The classes the block's words read ({}), each's bits in the block at `base`, a byte of the file: one load, and one\n/// classify a pair of nibble tables, as the lean lexer's `Window`. None past the file.\n///\n/// # Safety\n/// The CPU runs `S`'s backend.\n#[inline(always)]\nunsafe fn block_classes<S: rerail_simd8::Simd8x64>(src: &[u8], n: usize, base: usize) -> [u64; {k}] {{\n    let block: &[u8; 64] = src[base..base + 64].try_into().expect(\"the padded source\");\n    let lv = valid(n, base as isize);\n    // SAFETY: the caller's.\n    unsafe {{\n        let v = S::load(block);\n{tables}        [{}]\n    }}\n}}\n\n",
            used.iter().map(|c| format!("`{c}`")).collect::<Vec<_>>().join(", "),
            bits.iter().map(|b| format!("{b} & lv")).collect::<Vec<_>>().join(", ")
        );
        let mut decl = String::new();
        let mut head = String::from("        // The block's masks, from its classes (the lean lexer's `Window`): what its tokens read of it.\n");
        if on {
            let _ = writeln!(decl, "    // The next block's classes, made with the block before it: the block's own, where the loop goes on to it.\n    let (mut ahead_b, mut ahead) = (usize::MAX, [0u64; {k}]);");
            let _ = writeln!(head, "        // SAFETY: `base` is in the file.\n        let cur = if ahead_b == b {{ ahead }} else {{ unsafe {{ block_classes::<S>(src, n, base) }} }};");
            let _ = writeln!(head, "        // SAFETY: `base + 64` is in the file.\n        let next = if base + 64 < n {{ unsafe {{ block_classes::<S>(src, n, base + 64) }} }} else {{ [0u64; {k}] }};\n        (ahead_b, ahead) = (b + 1, next);");
        } else {
            let _ = writeln!(head, "        // SAFETY: `base` is in the file.\n        let cur = unsafe {{ block_classes::<S>(src, n, base) }};");
        }
        if back {
            let _ = writeln!(decl, "    // The last block's classes: the block before's, where the loop came to it from there.\n    let (mut behind_b, mut behind) = (usize::MAX, [0u64; {k}]);");
            let _ = writeln!(head, "        // SAFETY: `base - 64` is in the file.\n        let prev = if behind_b.wrapping_add(1) == b {{ behind }} else if b > 0 {{ unsafe {{ block_classes::<S>(src, n, base - 64) }} }} else {{ [0u64; {k}] }};\n        (behind_b, behind) = (b, cur);");
        }
        if words.iter().any(|(_, w)| w.contains("lv")) {
            head.push_str("        let lv = valid(n, base as isize);\n");
        }
        for (m, w) in words {
            let _ = writeln!(head, "        let k_{m} = {w};");
        }
        Some((decl, head))
    }

    /// Class `c`'s bits of the byte vector `v` (its nibble tables' `cP`, or its ranges' tests).
    fn class_bits(&self, c: &str) -> Option<String> {
        if let Some(code) = self.class_code.get(c) {
            return Some(code.trim_end_matches(" & lv").to_owned());
        }
        let rs = self.classes.get(c)?;
        let bits: Vec<String> = rs
            .iter()
            .map(|&(lo, hi)| match (lo, hi) {
                (lo, hi) if lo == hi => format!("S::eq_mask(v, {lo:#04x})"),
                (lo, 255) => format!("S::to_bitmask(S::ge(v, {lo:#04x}))"),
                (0, hi) => format!("!S::to_bitmask(S::ge(v, {:#04x}))", hi + 1),
                (lo, hi) => format!("(S::to_bitmask(S::ge(v, {lo:#04x})) & !S::to_bitmask(S::ge(v, {:#04x})))", hi + 1),
            })
            .collect();
        Some(if bits.is_empty() { "0".to_owned() } else { format!("({})", bits.join(" | ")) })
    }

    /// `e`'s bits in the block, `off` bytes on (back, negative), from the block's classes (`cur`, and the neighbours'
    /// `prev` and `next`): none where it reads escapes or a class 64 bytes away.
    #[allow(clippy::too_many_arguments)]
    fn blk(&self, e: &Expr, off: i64, depth: usize, used: &mut Vec<String>, back: &mut bool, on: &mut bool) -> Option<String> {
        if depth > 64 || off.abs() >= 64 {
            return None;
        }
        Some(match e {
            Expr::Empty => "0u64".to_owned(),
            Expr::Name(n) if self.classes.contains_key(n.as_str()) => {
                let i = match used.iter().position(|c| c == n) {
                    Some(i) => i,
                    None => {
                        used.push(n.clone());
                        used.len() - 1
                    }
                };
                let rs = self.classes[n.as_str()];
                match off {
                    0 => format!("cur[{i}]"),
                    // A byte on or back: the neighbour block's one byte, tested where it is (lean's `shift_in`), not
                    // the block classified.
                    1 => format!("(cur[{i}] >> 1 | u64::from(base + 64 < n && {}) << 63)", pattern_test("src[base + 64]", rs)),
                    -1 => format!("(cur[{i}] << 1 | u64::from(base > 0 && {}))", pattern_test("src[base - 1]", rs)),
                    o if o > 0 => {
                        *on = true;
                        format!("(cur[{i}] >> {o} | next[{i}] << {})", 64 - o)
                    }
                    o => {
                        *back = true;
                        format!("(cur[{i}] << {} | prev[{i}] >> {})", -o, 64 + o)
                    }
                }
            }
            Expr::Name(n) => self.blk(self.defs.get(n)?, off, depth + 1, used, back, on)?,
            Expr::Not(a) => format!("(!{} & lv)", self.blk(a, off, depth + 1, used, back, on)?),
            Expr::And(a, b) => format!("({} & {})", self.blk(a, off, depth + 1, used, back, on)?, self.blk(b, off, depth + 1, used, back, on)?),
            Expr::Or(a, b) => format!("({} | {})", self.blk(a, off, depth + 1, used, back, on)?, self.blk(b, off, depth + 1, used, back, on)?),
            Expr::Xor(a, b) => format!("({} ^ {})", self.blk(a, off, depth + 1, used, back, on)?, self.blk(b, off, depth + 1, used, back, on)?),
            Expr::Call(f, args) => {
                let (a, k) = Self::shift(f, args)?;
                self.blk(&a, off + k, depth + 1, used, back, on)?
            }
            Expr::Test { .. } => return None,
        })
    }

    /// The window functions and helpers the rewritten reads call.
    pub(super) fn code(&self) -> String {
        format!("{HELPERS}{}", self.code)
    }
}

/// The index of the `)` that closes the call whose arguments start `s`.
fn closing(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth > 0 => depth -= 1,
            ')' => return Some(i),
            _ => {}
        }
    }
    None
}

/// A call's arguments, split at its own commas.
fn split_args(s: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                out.push(String::new());
                continue;
            }
            _ => {}
        }
        out.last_mut().expect("one").push(c);
    }
    out
}

/// A byte's test of a class's ranges.
fn pattern_test(b: &str, rs: &[(u32, u32)]) -> String {
    if rs.is_empty() {
        return "false".to_owned();
    }
    let arms: Vec<String> = rs.iter().map(|&(lo, hi)| if lo == hi { format!("{lo:#04x}") } else { format!("{lo:#04x}..={hi:#04x}") }).collect();
    format!("matches!({b}, {})", arms.join(" | "))
}

fn ident(n: &str) -> String {
    n.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

/// `code` with each run of a token's tests over the same bytes (`if wcount_in(n, s, l, w_A) >= 1 { … }`, one after
/// another, comments between) read in one window loop (`wany_in`), as the lean lexer reads a string once: the tests'
/// classes are then loaded once a window, not once a test.
/// `code` (the rows' match) with each row's region end and its tests over the region (`wfirst_bit` from the opener's
/// next byte, then `wany_in` of its tests from the opener) read in one window loop from the opener (`wfirst_any`), as the
/// lean lexer reads a string once (`string_end`). Where the row's first bytes (`firsts`, by row) start no escape of the
/// end's mask, so the escapes from the opener are those from the byte after it.
pub(super) fn fuse_read(code: &str, firsts: &[[bool; 256]], escapes: &HashMap<String, String>, escape_bytes: &HashMap<String, [bool; 256]>) -> String {
    let lines: Vec<&str> = code.lines().collect();
    let mut out = String::new();
    let mut row: Option<usize> = None;
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        if let Some(k) = l.strip_prefix("                ").and_then(|t| t.strip_suffix(" => {")).and_then(|t| t.parse::<usize>().ok()) {
            row = Some(k);
        }
        let fused = (|| -> Option<(String, usize)> {
            let first = firsts.get(row?)?;
            let ind = l.len() - l.trim_start().len();
            let pad = " ".repeat(ind);
            let f = l.trim_start().strip_prefix("let l = match wfirst_bit(n, s + 1, |p: isize, esc: &mut u64| unsafe { ")?.strip_suffix("::<S>(src, n, p, esc) }) {")?;
            (lines.get(i + 1)? == &format!("{pad}    Some(c) => c + 0,")).then_some(())?;
            // The `None` arm, to the match's end.
            let close = (i + 2..lines.len()).find(|&j| lines[j] == format!("{pad}}};"))?;
            let none: Vec<&str> = lines[i + 2..close].to_vec();
            (lines.get(close + 1)? == &format!("{pad}let l = l.max(s);")).then_some(())?;
            let mut j = close + 2;
            while lines.get(j)?.trim_start().starts_with("//") {
                j += 1;
            }
            let rest = lines[j].trim_start().strip_prefix("let hit = wany_in(n, s, l, |p: isize, esc: &mut [u64; ")?;
            let (_, rest) = rest.split_once("]| unsafe { let escaped = escaped_step(")?;
            let (b, calls) = rest.split_once(", &mut esc[0]); [")?;
            let calls = calls.strip_suffix("] });")?;
            (escapes.get(f)? == b).then_some(())?;
            let bytes = escape_bytes.get(b)?;
            (!(0..256).any(|c| first[c] && bytes[c])).then_some(())?;
            let e = f.strip_prefix("w_")?;
            let mut t = String::new();
            let _ = writeln!(t, "{pad}// The region's end and its tests in one window loop from its opener, the escapes found once a window, as the");
            let _ = writeln!(t, "{pad}// lean lexer reads a string once: no first byte of the row starts an escape.");
            let _ = writeln!(t, "{pad}let (l, hit) = match wfirst_any(n, s, |p: isize, esc: &mut u64| unsafe {{ let escaped = escaped_step({b}, esc); (e_{e}::<S>(src, n, p, escaped), [{calls}]) }}) {{");
            let _ = writeln!(t, "{pad}    Some((c, hit)) => (c, hit),");
            for x in &none {
                let _ = writeln!(t, "{x}");
            }
            let _ = writeln!(t, "{pad}}};");
            let _ = writeln!(t, "{pad}let l = l.max(s);");
            Some((t, j + 1))
        })();
        match fused {
            Some((t, next)) => {
                out.push_str(&t);
                i = next;
            }
            None => {
                out.push_str(l);
                out.push('\n');
                i += 1;
            }
        }
    }
    out
}

pub(super) fn fuse_tests(code: &str, escapes: &HashMap<String, String>) -> String {
    let lines: Vec<&str> = code.lines().collect();
    // A test's head: its indent, its span and its window function.
    let head = |l: &str| -> Option<(usize, String, String)> {
        let ind = l.len() - l.trim_start().len();
        let rest = l.trim_start().strip_prefix("if wcount_in(n, ")?.strip_suffix("::<S>(src, n, p, esc) }) >= 1 {")?;
        let (span, f) = rest.split_once(", |p: isize, esc: &mut u64| unsafe { ")?;
        (f.starts_with("w_") && !f.contains(' ')).then(|| (ind, span.to_owned(), f.to_owned()))
    };
    // The line after a test's block, from its head at `i`.
    let end = |i: usize, ind: usize| -> Option<usize> {
        let close = format!("{}}}", " ".repeat(ind));
        (i + 1..lines.len()).find(|&j| lines[j] == close).map(|j| j + 1)
    };
    let mut out = String::new();
    let mut i = 0;
    while i < lines.len() {
        let Some((ind, span, _)) = head(lines[i]) else {
            out.push_str(lines[i]);
            out.push('\n');
            i += 1;
            continue;
        };
        // The run: tests of the same span and indent, only comments between.
        let mut tests = Vec::new();
        let (mut j, mut last) = (i, i);
        while j < lines.len() {
            match head(lines[j]) {
                Some((d, sp, f)) if d == ind && sp == span => {
                    let Some(e) = end(j, ind) else { break };
                    tests.push((j, f));
                    j = e;
                    last = e;
                }
                _ if lines[j].trim_start().starts_with("//") && lines[j].len() - lines[j].trim_start().len() == ind => j += 1,
                _ => break,
            }
        }
        if tests.len() < 2 {
            out.push_str(lines[i]);
            out.push('\n');
            i += 1;
            continue;
        }
        let pad = " ".repeat(ind);
        // Tests that read the escapes of one mask find them once a window, with one carry, as the lean lexer's string
        // reader passes each escape once.
        let of: Vec<Option<&String>> = tests.iter().map(|(_, f)| escapes.get(f)).collect();
        let shared = of.iter().all(|o| o.is_some() && *o == of[0]);
        let _ = writeln!(out, "{pad}// The token's tests of its bytes, in one window loop: their classes loaded once a window.");
        if let (true, Some(Some(b))) = (shared, of.first()) {
            let calls: Vec<String> = tests.iter().map(|(_, f)| format!("e_{}::<S>(src, n, p, escaped)", f.trim_start_matches("w_"))).collect();
            let _ = writeln!(out, "{pad}let hit = wany_in(n, {span}, |p: isize, esc: &mut [u64; {}]| unsafe {{ let escaped = escaped_step({b}, &mut esc[0]); [{}] }});", tests.len(), calls.join(", "));
        } else {
            let calls: Vec<String> = tests.iter().enumerate().map(|(k, (_, f))| format!("{f}::<S>(src, n, p, &mut esc[{k}])")).collect();
            let _ = writeln!(out, "{pad}let hit = wany_in(n, {span}, |p: isize, esc: &mut [u64; {}]| unsafe {{ [{}] }});", tests.len(), calls.join(", "));
        }
        for (k, line) in lines.iter().enumerate().take(last).skip(i) {
            match tests.iter().position(|(h, _)| *h == k) {
                Some(t) => {
                    let _ = writeln!(out, "{pad}if hit[{t}] {{");
                }
                None => {
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
        i = last;
    }
    if !code.ends_with('\n') {
        out.pop();
    }
    out
}

/// `code` without the counts' scans (`scan … max N`, decision 0623): where tier 1 reads the bytes, tier 2 is one loop
/// over its own stack, which refuses a level past its `DEPTH`. So a file nested too deep still ends in a refusal, and
/// no token counts its brackets.
pub(super) fn strip_scans(code: &str) -> String {
    let mut out = String::new();
    let mut lines = code.lines();
    while let Some(line) = lines.next() {
        if line.trim_start().starts_with("// scan ") {
            // To the scan loop's end: its `q = i + 1;`, then its closing brace.
            for l in lines.by_ref() {
                if l.trim() == "q = i + 1;" {
                    lines.next();
                    break;
                }
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !code.ends_with('\n') {
        out.pop();
    }
    out
}

/// The loop's own pauses (decision 0619 step 2), where stage 1 made the blocks as the loop came to need them: none
/// where the loop reads the bytes.
pub(super) fn strip_pauses(code: &str) -> String {
    let lines: Vec<&str> = code.split_inclusive('\n').collect();
    let t = |i: usize| lines.get(i).map(|l| l.trim()).unwrap_or("");
    let mut out = String::new();
    let mut i = 0;
    while i < lines.len() {
        if t(i) == "// The byte after the token in a block stage 1 has not made: it makes more, and the token is read again." && t(i + 1) == "if l + 1 >= 64 * made && made < nw {" {
            i += 5;
            continue;
        }
        if t(i) == "if made < nw {" && t(i + 1) == "wait = (2 * made - b).min(nw);" && t(i + 2) == "continue 'pass;" && t(i + 3) == "}" {
            i += 4;
            continue;
        }
        out.push_str(lines[i]);
        i += 1;
    }
    out
}

/// The helpers every window reads: the 64 bytes loaded where they are, the bits of the file's bytes, the escapes
/// carried from window to window, and the searches 64 bytes at a time.
const HELPERS: &str = r#"/// The 64 bytes from `p` as loaded, and the byte they start at: at the file's first byte for a window before it. None
/// for a window past the file's end.
///
/// # Safety
/// The CPU runs `S`'s backend.
#[inline(always)]
unsafe fn window<S: rerail_simd8::Simd8x64>(src: &[u8], n: usize, p: isize) -> Option<(S::V, isize)> {
    if p >= n as isize || p <= -64 {
        return None;
    }
    let q = p.max(0);
    let block: &[u8; 64] = src[q as usize..q as usize + 64].try_into().expect("the padded source");
    // SAFETY: the caller's.
    Some((unsafe { S::load(block) }, q))
}

/// The bits `b` of the window loaded at `q`, as the window from `p`'s: none past the file or before it.
#[inline(always)]
fn held(b: u64, q: isize, n: usize, p: isize) -> u64 {
    let b = if p < q { b << (q - p) } else { b };
    b & valid(n, p)
}

/// `held` of a class the padding holds none of: the bits before the file shifted out, those past it none.
#[inline(always)]
fn padded(b: u64, q: isize, p: isize) -> u64 {
    if p < q { b << (q - p) } else { b }
}

/// The bits of the window from `p` that are the file's bytes.
#[inline(always)]
fn valid(n: usize, p: isize) -> u64 {
    let left = n as isize - p;
    let hi = if left >= 64 { !0 } else if left <= 0 { 0 } else { (1u64 << left) - 1 };
    if p < 0 { hi & (!0u64 << (-p).min(63)) } else { hi }
}

/// One window's escaped bytes (`escaped`), each after an odd run of `bs`'s; `carry`, the run's escape over the window
/// before.
#[inline(always)]
fn escaped_step(bs: u64, carry: &mut u64) -> u64 {
    const ODD: u64 = 0xaaaa_aaaa_aaaa_aaaa;
    if bs == 0 {
        return std::mem::take(carry);
    }
    let potential = bs & !*carry;
    let series = ((potential << 1) | ODD).wrapping_sub(potential);
    let escape_and_terminal = series ^ ODD;
    let escaped = escape_and_terminal ^ (bs | *carry);
    *carry = (escape_and_terminal & bs) >> 63;
    escaped
}

/// A region's end and its tests in one window loop from its opener `a` (`fuse_read`): the first byte after `a` the end
/// holds, and whether each test holds in `a..=` that byte. `w` gives a window's end and tests, its escapes found once.
#[inline(always)]
fn wfirst_any<const K: usize>(n: usize, a: usize, mut w: impl FnMut(isize, &mut u64) -> (u64, [u64; K])) -> Option<(usize, [bool; K])> {
    let mut esc = 0u64;
    let mut any = [0u64; K];
    let mut p = a as isize;
    // The opener's byte is no end.
    let mut opener = 1u64;
    while p < n as isize {
        let (end, t) = w(p, &mut esc);
        let end = end & !opener;
        opener = 0;
        if end != 0 {
            let k = end.trailing_zeros();
            let keep = u64::MAX >> (63 - k);
            for i in 0..K {
                any[i] |= t[i] & keep;
            }
            return Some((p as usize + k as usize, any.map(|x| x != 0)));
        }
        for i in 0..K {
            any[i] |= t[i];
        }
        p += 64;
    }
    None
}

/// The first byte at or after `from` a mask holds, its windows `w` from `from` on.
#[inline(always)]
fn wfirst_bit(n: usize, from: usize, mut w: impl FnMut(isize, &mut u64) -> u64) -> Option<usize> {
    let mut esc = 0u64;
    let mut p = from as isize;
    while p < n as isize {
        let bits = w(p, &mut esc);
        if bits != 0 {
            return Some(p as usize + bits.trailing_zeros() as usize);
        }
        p += 64;
    }
    None
}

/// The first byte in `a..=b` a mask holds, its windows `w` from `a` on.
#[inline(always)]
fn wfirst_in(n: usize, a: usize, b: usize, mut w: impl FnMut(isize, &mut u64) -> u64) -> Option<usize> {
    let mut esc = 0u64;
    let mut p = a as isize;
    while p <= b as isize && p < n as isize {
        let mut bits = w(p, &mut esc);
        let left = b as isize - p + 1;
        if left < 64 {
            bits &= (1u64 << left) - 1;
        }
        if bits != 0 {
            return Some(p as usize + bits.trailing_zeros() as usize);
        }
        p += 64;
    }
    None
}

/// A mask's bits in bytes `a..=b`, its windows `w` from `a` on.
#[inline(always)]
fn wcount_in(n: usize, a: usize, b: usize, mut w: impl FnMut(isize, &mut u64) -> u64) -> u32 {
    let mut esc = 0u64;
    let mut c = 0;
    let mut p = a as isize;
    while p <= b as isize && p < n as isize {
        let mut bits = w(p, &mut esc);
        let left = b as isize - p + 1;
        if left < 64 {
            bits &= (1u64 << left) - 1;
        }
        c += bits.count_ones();
        p += 64;
    }
    c
}

/// Whether each of `K` masks has a bit in bytes `a..=b`, their windows `w` from `a` on: one loop, so each window's
/// bytes are loaded and classified once for them all (each mask its own escape carry).
#[inline(always)]
fn wany_in<const K: usize>(n: usize, a: usize, b: usize, mut w: impl FnMut(isize, &mut [u64; K]) -> [u64; K]) -> [bool; K] {
    let mut esc = [0u64; K];
    let mut any = [0u64; K];
    let mut p = a as isize;
    while p <= b as isize && p < n as isize {
        let bits = w(p, &mut esc);
        let left = b as isize - p + 1;
        let keep = if left < 64 { (1u64 << left) - 1 } else { !0 };
        for k in 0..K {
            any[k] |= bits[k] & keep;
        }
        p += 64;
    }
    any.map(|x| x != 0)
}

/// The first byte at or after `from`, before `to`, that a mask does not hold, its windows `w` from `from` on.
#[inline(always)]
fn wfirst_clear(n: usize, from: usize, to: usize, mut w: impl FnMut(isize, &mut u64) -> u64) -> Option<usize> {
    let mut esc = 0u64;
    let mut p = from as isize;
    while p < to as isize {
        let mut bits = !w(p, &mut esc);
        let left = to as isize - p;
        if left < 64 {
            bits &= (1u64 << left) - 1;
        }
        if bits != 0 {
            let i = p as usize + bits.trailing_zeros() as usize;
            return (i < n).then_some(i);
        }
        p += 64;
    }
    None
}

/// The last byte of a mask's run after byte `s` (`s` where none), its windows `w` from `s + 1` on.
#[inline(always)]
fn wrun_last(n: usize, s: usize, mut w: impl FnMut(isize, &mut u64) -> u64) -> usize {
    let _ = n;
    let mut esc = 0u64;
    let mut p = s as isize + 1;
    loop {
        let ones = (!w(p, &mut esc)).trailing_zeros() as isize;
        if ones < 64 {
            return (p + ones - 1) as usize;
        }
        p += 64;
    }
}

/// `wfirst_bit` from block `base`'s word `k` where `from` is in the block, then windows past it.
#[inline(always)]
fn bfirst_bit(n: usize, base: usize, k: u64, from: usize, w: impl FnMut(isize, &mut u64) -> u64) -> Option<usize> {
    let q = from.wrapping_sub(base);
    if q >= 64 {
        return wfirst_bit(n, from, w);
    }
    let bits = k & (!0u64 << q);
    if bits != 0 {
        return Some(base + bits.trailing_zeros() as usize);
    }
    wfirst_bit(n, base + 64, w)
}

/// Block `base`'s word `k` cut to bytes `a..=b` (`a` in the block), and whether `b` is past the block.
#[inline(always)]
fn in_block(base: usize, k: u64, a: usize, b: usize) -> (u64, bool) {
    let mut bits = k & (!0u64 << (a - base));
    let past = b >= base + 64;
    if !past {
        bits &= !0u64 >> (63 - (b - base));
    }
    (bits, past)
}

/// `wfirst_in` from block `base`'s word `k` where `a` is in the block, then windows past it.
#[inline(always)]
fn bfirst_in(n: usize, base: usize, k: u64, a: usize, b: usize, w: impl FnMut(isize, &mut u64) -> u64) -> Option<usize> {
    if a > b {
        return None;
    }
    if a.wrapping_sub(base) >= 64 {
        return wfirst_in(n, a, b, w);
    }
    let (bits, past) = in_block(base, k, a, b);
    if bits != 0 {
        return Some(base + bits.trailing_zeros() as usize);
    }
    if past { wfirst_in(n, base + 64, b, w) } else { None }
}

/// `wcount_in` from block `base`'s word `k` where `a` is in the block, then windows past it.
#[inline(always)]
fn bcount_in(n: usize, base: usize, k: u64, a: usize, b: usize, w: impl FnMut(isize, &mut u64) -> u64) -> u32 {
    if a > b {
        return 0;
    }
    if a.wrapping_sub(base) >= 64 {
        return wcount_in(n, a, b, w);
    }
    let (bits, past) = in_block(base, k, a, b);
    bits.count_ones() + if past { wcount_in(n, base + 64, b, w) } else { 0 }
}

/// `wfirst_clear` from block `base`'s word `k` where `from` is in the block, then windows past it.
#[inline(always)]
fn bfirst_clear(n: usize, base: usize, k: u64, from: usize, to: usize, w: impl FnMut(isize, &mut u64) -> u64) -> Option<usize> {
    if from >= to {
        return None;
    }
    if from.wrapping_sub(base) >= 64 {
        return wfirst_clear(n, from, to, w);
    }
    let (bits, past) = in_block(base, !k, from, to - 1);
    if bits != 0 {
        let i = base + bits.trailing_zeros() as usize;
        return (i < n).then_some(i);
    }
    if past { wfirst_clear(n, base + 64, to, w) } else { None }
}

/// `wrun_last` from block `base`'s word `k` where the run's first byte is in the block (the lean lexer's
/// `micro::run`), then windows past it (its `word_end`).
#[inline(always)]
fn brun_last(n: usize, base: usize, k: u64, s: usize, w: impl FnMut(isize, &mut u64) -> u64) -> usize {
    let q = (s + 1).wrapping_sub(base);
    if q >= 64 {
        return wrun_last(n, s, w);
    }
    let ones = (!(k >> q)).trailing_zeros() as usize;
    if q + ones < 64 {
        return s + ones;
    }
    wrun_last(n, base + 63, w)
}

/// `wrun_once_last` from block `base`'s words `kr` and `kd`, then windows past it.
#[inline(always)]
fn brun_once_last(n: usize, base: usize, kr: u64, kd: u64, s: usize, r: impl FnMut(isize, &mut u64) -> u64, mut d: impl FnMut(isize, &mut u64) -> u64) -> usize {
    let l = brun_last(n, base, kr, s, r);
    match bfirst_in(n, base, kd, s + 1, l, &mut d).and_then(|p| bfirst_in(n, base, kd, p + 1, l, &mut d)) {
        Some(q) => q - 1,
        None => l,
    }
}

/// The last byte of the run of `r` after byte `s` with one bit of `d` in it at most: the second ends the run before
/// it, as a lexer reads a number's one `.` and stops at the next.
#[inline(always)]
fn wrun_once_last(n: usize, s: usize, r: impl FnMut(isize, &mut u64) -> u64, mut d: impl FnMut(isize, &mut u64) -> u64) -> usize {
    let l = wrun_last(n, s, r);
    match wfirst_in(n, s + 1, l, &mut d).and_then(|p| wfirst_in(n, p + 1, l, &mut d)) {
        Some(q) => q - 1,
        None => l,
    }
}

"#;

/// A word's first 8 bytes as one word of memory, little-endian, zeros past its end.
fn key8(w: &[u8]) -> u64 {
    w.iter().take(8).enumerate().fold(0, |k, (i, &c)| k | u64::from(c) << (8 * i))
}

/// A token's word from one load (decision 0623 step 4, the lean lexer's `word_code`): its first 8 bytes masked to its
/// length, and the length, hashed by one multiply into a table where each of the grammar's words (`words`, code
/// `i + 1`) has a slot of its own. `None` where no multiplier the search tries gives each its own slot.
pub(super) fn word_table(words: &[Vec<u8>]) -> Option<String> {
    let keys: Vec<u64> = words.iter().map(|w| key8(w) ^ w.len() as u64).collect();
    let least = usize::BITS - words.len().max(1).leading_zeros();
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    for bits in least + 1..=least + 4 {
        for _ in 0..100_000 {
            // splitmix64's steps: a fixed sequence, so the table is the same at every build.
            x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            let mul = (z ^ (z >> 31)) | 1;
            let mut slots = vec![usize::MAX; 1 << bits];
            let ok = keys.iter().enumerate().all(|(i, &k)| {
                let slot = (k.wrapping_mul(mul) >> (64 - bits)) as usize;
                let free = slots[slot] == usize::MAX;
                slots[slot] = i;
                free
            });
            if !ok {
                continue;
            }
            let mut table = String::new();
            for slot in &slots {
                match words.get(*slot) {
                    Some(w) => {
                        let _ = writeln!(table, "    ({:#018x}, {}, {}),", key8(w), w.len(), slot + 1);
                    }
                    None => table.push_str("    (0, 0, 0),\n"),
                }
            }
            // The bytes after the 8th of each word longer, by its code: compared only for a word that long.
            let long: Vec<String> = words.iter().enumerate().filter(|(_, w)| w.len() > 8).map(|(i, w)| format!("        {} => b\"{}\",", i + 1, w[8..].iter().map(|c| std::ascii::escape_default(*c).to_string()).collect::<String>())).collect();
            return Some(format!(
                "/// Each slot's word (`word_at`): its first 8 bytes, its length, its code (0: none), as the lean lexer's `WORDS`.\nstatic WORDS: [(u64, u32, u32); {}] = [\n{table}];\n\n/// A token's word: the code of the word its `len` bytes from `s` are, 0 for none, as the lean lexer's `word_code`. Its\n/// first 8 bytes, masked to its length, and the length, hashed by one multiply into `WORDS`, compared without a branch;\n/// a word of more than 8 compares the rest.\n#[inline(always)]\nfn word_at(src: &[u8], s: usize, len: usize) -> u32 {{\n    let raw = u64::from_le_bytes(src[s..s + 8].try_into().expect(\"the padded source\"));\n    let k = raw & (u64::MAX >> (64 - 8 * len.min(8)));\n    let (key, l, code) = WORDS[((k ^ len as u64).wrapping_mul({mul:#018x}) >> {}) as usize];\n    let code = code & 0u32.wrapping_sub(u32::from((key == k) & (l as usize == len)));\n    if len > 8 && code != 0 && src[s + 8..s + len] != *long_tail(code) {{ 0 }} else {{ code }}\n}}\n\n/// The bytes after the 8th of the word of `code`, of those longer than 8.\nfn long_tail(code: u32) -> &'static [u8] {{\n    match code {{\n{}\n        _ => b\"\",\n    }}\n}}\n\n",
                1usize << bits,
                64 - bits,
                long.join("\n")
            ));
        }
    }
    None
}
