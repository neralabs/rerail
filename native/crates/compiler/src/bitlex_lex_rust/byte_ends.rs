//! Each token's last byte, read from the bytes by tier 2 (decision 0619 step 4, as 0586's): tier 1's records then
//! hold no last byte. The token's rule is the first whose start holds at its first byte, as tier 1's dispatch takes
//! it, the grammar's assumptions at a start read in; its end is its rule's, as tier 1's token loop finds it, each mask
//! it reads made of the bytes of 64 at a time (`rerail_simd8`) in place of stage 1's.
//!
//! Written where every mask a rule reads is local (classes, `prev`, `next` and the operators) or a closer's
//! `C & ~escaped(B)`; a region read in parts, a second closer (`or`) or one `after` another is not written yet, and
//! tier 1 then keeps its last bytes.

use std::collections::HashMap;
use std::fmt::Write as _;

use ::bitlex::syntax::{Arg, Expr, Region, Stmt, TokenRule};

/// The masks' and the classes' definitions, the last of each name, and the window functions written so far.
struct Ends<'a> {
    classes: HashMap<&'a str, &'a [(u32, u32)]>,
    defs: HashMap<String, Expr>,
    /// Each class's window function, by name: the class's bits in the 64 bytes from a byte.
    cls: Vec<String>,
    code: String,
}

impl<'a> Ends<'a> {
    fn new(stmts: &'a [Stmt]) -> Self {
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
        Ends { classes, defs, cls: Vec::new(), code: String::new() }
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

    /// `e` at byte `at` (Rust, a `usize`), a test of the bytes there: none past the file or before it.
    fn at(&self, e: &Expr, at: &str, depth: usize) -> Result<String, String> {
        if depth > 64 {
            return Err("a mask's definition reads itself".into());
        }
        Ok(match e {
            Expr::Empty => "false".to_owned(),
            Expr::Name(n) => {
                if let Some(rs) = self.classes.get(n.as_str()) {
                    // A token's first byte is in the file.
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

    /// `e`'s bits in the 64 bytes from byte `p + off` (Rust, an `isize` expression `p`): none past the file or before
    /// it. `escaped`: the closer search's carried escapes (`esc`, this window's).
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
                format!("escaped_step({b}, &mut esc)")
            }
            Expr::Call(f, args) => match Self::shift(f, args) {
                Some((a, k)) => self.win(&a, off + k, depth + 1)?,
                None => return Err(format!("`{f}` is no mask of the bytes 64 at a time")),
            },
            Expr::Test { .. } => return Err("a machine's test is no mask of the bytes".into()),
        })
    }

    /// The first byte from `from` (a `usize` expression) that `e` holds, or `None`: the windows of 64 in turn.
    fn first(&mut self, e: &Expr, from: &str) -> Result<String, String> {
        let bits = self.win(e, 0, 0)?;
        let esc = if bits.contains("esc") { "let mut esc = 0u64;\n    " } else { "" };
        Ok(format!(
            "{{\n    {esc}let mut p = ({from}) as isize;\n    loop {{\n        if p >= n as isize {{\n            break None;\n        }}\n        let w = {bits};\n        if w != 0 {{\n            break Some(p as usize + w.trailing_zeros() as usize);\n        }}\n        p += 64;\n    }}\n}}"
        ))
    }

    /// The last byte of the run of `e` after byte `s`: `s` where none.
    fn run(&mut self, e: &Expr) -> Result<String, String> {
        let bits = self.win(e, 0, 0)?;
        if bits.contains("esc") {
            return Err("a run of escapes".into());
        }
        Ok(format!(
            "{{\n    let mut p = s as isize + 1;\n    loop {{\n        let ones = (!{bits}).trailing_zeros() as isize;\n        if ones < 64 {{\n            break (p + ones - 1) as usize;\n        }}\n        p += 64;\n    }}\n}}"
        ))
    }

    /// The token's last byte by its rule (`l`), as tier 1's token loop finds it; `assumed`, what its rule reads in
    /// its tokens (`in KIND`).
    fn end(&mut self, rule: &Option<TokenRule>, last: &Option<Expr>, starts: &[(&str, &Expr)], assumed: &[(&str, &Expr)]) -> Result<String, String> {
        let read = |e: &Expr, defs: &HashMap<String, Expr>| if assumed.is_empty() { e.clone() } else { crate::bitlex_lex_rust::substitute(e, assumed, defs) };
        Ok(match rule {
            Some(TokenRule::Run(r)) => {
                let r = read(r, &self.defs);
                self.run(&r)?
            }
            Some(TokenRule::RunOnce(r, d)) => {
                let l = self.run(r)?;
                let d1 = self.first(d, "s + 1")?;
                let d2 = self.first(d, "p + 1")?;
                format!("{{\n    let l = {l};\n    match ({d1}).filter(|&p| p <= l).and_then(|p| {d2}).filter(|&q| q <= l) {{\n        Some(q) => q - 1,\n        None => l,\n    }}\n}}")
            }
            Some(TokenRule::Pair(p)) => format!("s + usize::from({})", self.at(p, "s", 0)?),
            Some(TokenRule::Pairs { pairs, .. }) => {
                let mut two = Vec::new();
                let mut three = Vec::new();
                for p in pairs {
                    match p.as_slice() {
                        [a, b] => two.push(format!("[{a:#04x}, {b:#04x}]")),
                        [a, b, c] => three.push(format!("[{a:#04x}, {b:#04x}, {c:#04x}]")),
                        _ => return Err(format!("a pair of {} bytes", p.len())),
                    }
                }
                let third = if three.is_empty() { "0".to_owned() } else { format!("usize::from(matches!([c(s), c(s + 1), c(s + 2)], {}))", three.join(" | ")) };
                format!("match [c(s), c(s + 1)] {{\n    {} => s + 1 + {third},\n    _ => s,\n}}", if two.is_empty() { "[_, _] if false".to_owned() } else { two.join(" | ") })
            }
            Some(TokenRule::Read(rs)) => {
                let mut code = String::new();
                for (i, r) in rs.iter().enumerate() {
                    let one = self.region(r)?;
                    if rs.len() == 1 {
                        return Ok(one);
                    }
                    let o = crate::bitlex_lex_rust::substitute(&r.opener, starts, &self.defs);
                    let _ = write!(code, "{}if {} {{\n    {}\n}} ", if i == 0 { "" } else { "else " }, self.at(&o, "s", 0)?, one.replace('\n', "\n    "));
                }
                code.push_str("else {\n    s\n}");
                code
            }
            Some(TokenRule::ReadParts(..)) => return Err("a region read in parts".into()),
            Some(TokenRule::Inside(..)) => return Err("a part's token".into()),
            None => match last {
                Some(l) => format!("({}).unwrap_or(s)", self.first(l, "s")?),
                None => "s".to_owned(),
            },
        })
    }

    /// A region's token's last byte: its closer's last after its opener, or before the closer (`outside`, or the
    /// file's end); with no closer, its `else` form's bytes, else the file's last byte (tier 1 refuses the file).
    fn region(&mut self, r: &Region) -> Result<String, String> {
        if r.or.is_some() || !r.after.is_empty() || r.resumes {
            return Err(format!("region `{}`: a second closer or one after another", r.kind));
        }
        let c = self.first(&r.closer, "s + 1")?;
        Ok(if r.outside {
            format!("({c}).unwrap_or(n) - 1")
        } else {
            let none = match &r.empty {
                Some((_, m)) => format!("s + {}", m.saturating_sub(1)),
                None => "n - 1".to_owned(),
            };
            format!("match {c} {{\n    Some(c) => c + {},\n    None => {none},\n}}", r.bytes.saturating_sub(1))
        })
    }
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

/// The helpers every window reads: the 64 bytes loaded where they are, the bits of the file's bytes, and the escapes
/// carried from window to window (`escaped`'s step, word by word as tier 1's).
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

"#;

/// Tier 2's `token_last(src, n, s)`, the last byte of the token tier 1 starts at `s`, read from the bytes; or why it
/// is not written.
pub fn byte_ends(stmts: &[Stmt]) -> Result<String, String> {
    let mut e = Ends::new(stmts);
    let starts: Vec<(&str, &Expr)> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::Assume { scope: None, name, expr, .. } => Some((name.as_str(), expr)),
            _ => None,
        })
        .collect();
    let mut rows = String::new();
    for s in stmts {
        let Stmt::Token { kind, start, rule, last, .. } = s else { continue };
        if matches!(rule, Some(TokenRule::Inside(..))) {
            continue;
        }
        let assumed: Vec<(&str, &Expr)> = stmts
            .iter()
            .filter_map(|s| match s {
                Stmt::Assume { scope: Some(k), name, expr, .. } if k == kind => Some((name.as_str(), expr)),
                _ => None,
            })
            .collect();
        let start = crate::bitlex_lex_rust::substitute(start, &starts, &e.defs);
        let test = e.at(&start, "s", 0).map_err(|w| format!("`{kind}`'s start: {w}"))?;
        let end = e.end(rule, last, &starts, &assumed).map_err(|w| format!("`{kind}`'s end: {w}"))?;
        let _ = write!(rows, "    // `{kind}`\n    if {test} {{\n        return {};\n    }}\n", end.replace('\n', "\n        "));
    }
    if rows.is_empty() {
        return Err("no token".into());
    }
    Ok(format!(
        "{HELPERS}{}/// The last byte of the token tier 1 starts at `s`, from the bytes (decision 0619 step 4, as 0586's): the first\n/// rule whose start holds there, as tier 1's dispatch takes it, and its end as tier 1's token loop finds it.\n///\n/// # Safety\n/// The CPU runs `S`'s backend.\n#[allow(unused_unsafe, clippy::all)]\nunsafe fn token_last_on<S: rerail_simd8::Simd8x64>(src: &[u8], n: usize, s: usize) -> usize {{\n    let c = |i: usize| if i < n {{ src[i] }} else {{ 0 }};\n    let _ = c;\n{rows}    s\n}}\n\nrerail_simd8::dispatch! {{\n    /// `token_last_on` on the CPU's best backend.\n    pub fn token_last(src: &[u8], n: usize, s: usize) -> usize = token_last_on, on token_last_on_best;\n}}\n",
        e.code
    ))
}

/// Whether tier 2 reads each token's last byte from the bytes, and tier 1's records hold none: where no region is
/// read in parts (tier 2 then reads no mask, decision 0619 step 3) and every rule's end has its byte form.
pub fn ends_from_bytes(stmts: &[Stmt]) -> bool {
    crate::bitlex_lex_rust::readers(stmts).is_empty() && byte_ends(stmts).is_ok()
}
