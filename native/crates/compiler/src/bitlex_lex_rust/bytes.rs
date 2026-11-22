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
}

/// The calls of the loop that read a mask: their name, and how many of their first arguments are masks.
const READS: [(&str, usize); 8] =
    [("run_once_last", 2), ("first_clear", 1), ("first_bit", 1), ("count_in", 1), ("first_in", 1), ("run_last", 1), ("any_in", 1), ("bit", 1)];

impl<'a> Bytes<'a> {
    pub(super) fn new(stmts: &'a [Stmt], locals: HashMap<String, Expr>) -> Self {
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
        Bytes { classes, defs, locals, written: Vec::new(), cls: Vec::new(), code: String::new() }
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
                            "/// Class `{n}`'s bits in the 64 bytes from `p`: none past the file or before it.\n///\n/// # Safety\n/// The CPU runs `S`'s backend.\n#[inline(always)]\nunsafe fn {f}<S: rerail_simd8::Simd8x64>(src: &[u8], n: usize, p: isize) -> u64 {{\n    let Some((v, q)) = (unsafe {{ window::<S>(src, n, p) }}) else {{ return 0 }};\n    // SAFETY: the caller's.\n    let b = unsafe {{ {} }};\n    held(b, q, n, p)\n}}\n\n",
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
            Expr::And(a, b) => format!("({} & {})", self.win(a, off, depth + 1)?, self.win(b, off, depth + 1)?),
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
        for line in code.split_inclusive('\n') {
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

    /// The block's token starts: the bytes `skip` does not hold, from block `b`'s window.
    pub(super) fn starts(&mut self, skip: &str) -> Result<String, String> {
        let (f, esc) = self.window_fn(skip)?;
        if esc {
            return Err(format!("`{skip}` reads escapes"));
        }
        Ok(format!("!({f})(base as isize, &mut 0)"))
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
                        let tail: Vec<String> = w.iter().skip(8).map(|c| format!("{c:#04x}")).collect();
                        let _ = writeln!(table, "    ({:#018x}, {}, {}, &[{}]),", key8(w), w.len(), slot + 1, tail.join(", "));
                    }
                    None => table.push_str("    (0, 0, 0, &[]),\n"),
                }
            }
            return Some(format!(
                "/// Each slot's word (`word_at`): its first 8 bytes, its length, its code, its bytes after the 8th.\nstatic WORDS: [(u64, usize, u32, &[u8]); {}] = [\n{table}];\n\n/// A token's word: the code of the word its `len` bytes from `s` are, 0 for none. Its first 8 bytes, masked to its length,\n/// and the length, hashed by one multiply into `WORDS` (the lean lexer's `word_code`); a word of more than 8 compares the rest.\n#[inline(always)]\nfn word_at(src: &[u8], s: usize, len: usize) -> u32 {{\n    let raw = u64::from_le_bytes(src[s..s + 8].try_into().expect(\"the padded source\"));\n    let k = raw & (u64::MAX >> (64 - 8 * len.min(8)));\n    let (key, l, code, tail) = WORDS[((k ^ len as u64).wrapping_mul({mul:#018x}) >> {}) as usize];\n    if key == k && l == len && (len <= 8 || src[s + 8..s + len] == *tail) {{ code }} else {{ 0 }}\n}}\n\n",
                1usize << bits,
                64 - bits
            ));
        }
    }
    None
}
