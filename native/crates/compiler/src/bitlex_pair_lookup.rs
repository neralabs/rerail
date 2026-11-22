//! A grammar's byte-pair errors in simdjson's `utf8_checker` form, for tier 1's Rust (`bitlex_lex_rust`).
//!
//! A grammar states UTF-8's check as simdjson's rules (decision 0404). There are two kinds of rule:
//!
//! - **The two-byte rules**, `prev(L, 1) & C` or `C & ~prev(L, 1)`.
//! - **One XOR rule**, `(prev(A, 1) & B) ^ (prev(X, 2) | prev(Y, 3))`, with `X` and `Y` ranges up to 0xff.
//!
//! simdjson checks them in three steps:
//!
//! - It looks up three nibble tables, `prev1`'s high and low nibbles and this byte's high nibble (`check_special_cases`),
//!   and ANDs them. Each rule is a bit of the tables.
//! - It tests lags 2 and 3 with two compares (`must_be_2_3_continuation`), and XORs the result with the XOR rule's bit.
//! - It skips a block of ASCII. Such a block holds an error only where the block before left a sequence open
//!   (`prev_incomplete`, a test of that block's last three bytes). After the file's last block, the block of zeros
//!   past its end is such a block, so the same test is the end's check.
//!
//! Tier 1's Rust takes this form when it can prove the result equal to the grammar's statements. These are its error
//! statements over classes alone, read at lags 1 to 3, together with those that hold only within three bytes of the
//! file's end. The tables come from the rules (`native_cpu_pairs::merged`, as the JIT's pairs, decision 0404). The
//! proof is by enumeration, over each byte class the statements tell apart and the bytes the check reads past the
//! file (0x00, the padding's spaces, the `prefix`):
//!
//! - every window of four bytes, at the file's start too;
//! - every end of five bytes, with the errors those bytes already show;
//! - the test of a block's last three bytes.
//!
//! Where any of these differs, tier 1 keeps the grammar's masks.

use ::bitlex::syntax::{Arg, Expr, Stmt};
use std::collections::HashMap;

/// A mask of the grammar made of classes alone: its value at a byte is its bytes' classes there.
#[derive(Clone, Debug)]
enum P {
    Class(std::boxed::Box<[bool; 256]>),
    Empty,
    Not(std::boxed::Box<P>),
    And(std::boxed::Box<P>, std::boxed::Box<P>),
    Or(std::boxed::Box<P>, std::boxed::Box<P>),
    Xor(std::boxed::Box<P>, std::boxed::Box<P>),
    Prev(std::boxed::Box<P>, usize),
    Next(std::boxed::Box<P>, usize),
}

/// The check, as tier 1 writes it: its tables, its compares, and the statements it is.
#[derive(Clone, Debug)]
pub struct Lookup {
    /// `prev1`'s high nibble's bits, its low nibble's, and this byte's high nibble's (simdjson's `byte_1_high`,
    /// `byte_1_low`, `byte_2_high`).
    pub byte_1_high: [u8; 16],
    pub byte_1_low: [u8; 16],
    pub byte_2_high: [u8; 16],
    /// The XOR rule: the least bytes 2 back and 3 back where its pair must hold (simdjson's 0xe0 and 0xf0), its bit
    /// 0x80.
    pub xor: Option<(u8, u8)>,
    /// The least bytes 1, 2 and 3 back from a block's end that leave a sequence open (`is_incomplete`): bytes 63, 62
    /// and 61. `None` where no byte there does.
    pub open: [Option<u8>; 3],
    /// The statements it is, by index, and their reasons.
    pub covers: Vec<(usize, String)>,
}

/// The check of `stmts`' errors over classes, if it is simdjson's lookup form, or why not; `prefix` is the bytes a file
/// may start with that no mask reads (`prefix`, decision 0335).
pub fn lookup(stmts: &[Stmt], prefix: Option<&[u8]>) -> Result<Lookup, String> {
    // Each name's mask as it is at each statement: a class, a mask of classes, or (`None`) anything else.
    let mut env: HashMap<String, Option<P>> = HashMap::new();
    let mut window: Vec<(usize, String, P)> = Vec::new();
    let mut ends: Vec<(usize, String, P)> = Vec::new();
    for (i, s) in stmts.iter().enumerate() {
        match s {
            Stmt::Class(name, items) => {
                let mut set = [false; 256];
                for &(lo, hi) in items {
                    for c in lo..=hi.min(255) {
                        set[c as usize] = true;
                    }
                }
                env.insert(name.clone(), Some(P::Class(std::boxed::Box::new(set))));
            }
            Stmt::Mask(name, e) => {
                let p = resolve(e, &env);
                env.insert(name.clone(), p);
            }
            Stmt::Error(why, e) | Stmt::Escape(why, e) => {
                if let Some(p) = resolve(e, &env) {
                    if lags(&p).is_some_and(|(back, ahead)| back <= 3 && ahead == 0) {
                        window.push((i, why.clone(), p));
                    } else if at_end(&p) {
                        ends.push((i, why.clone(), p));
                    }
                }
            }
            // Any other name it defines (a region's masks, a count, a number): not of classes.
            other => {
                let mut names = Vec::new();
                defined(other, &mut names);
                for n in names {
                    env.insert(n, None);
                }
            }
        }
    }
    if window.is_empty() {
        return Err("no error of classes at lags 1 to 3".into());
    }
    // The rules: the window statements' terms.
    let mut pairs = Vec::new();
    let mut xor: Option<(crate::native_cpu_pairs::Box, u8, u8)> = None;
    for (_, _, p) in &window {
        let mut terms = Vec::new();
        ors(p, &mut terms);
        for t in terms {
            let no_box = || format!("a rule's sets are not products of nibbles: {t:?}");
            match rule(t) {
                Some(Rule::Pair(before, here)) => pairs.push(box_of(&before, &here).ok_or_else(no_box)?),
                Some(Rule::Xor(before, here, x, y)) if xor.is_none() => xor = Some((box_of(&before, &here).ok_or_else(no_box)?, x, y)),
                _ => return Err(format!("a term is no rule: {t:?}")),
            }
        }
    }
    let boxes = crate::native_cpu_pairs::merged(pairs);
    let room = if xor.is_some() { 7 } else { 8 };
    // simdjson's three tables: this byte's set a set of high nibbles in each rule.
    if boxes.len() > room {
        return Err(format!("{} rules' boxes, room for {room}", boxes.len()));
    }
    if boxes.iter().chain(xor.iter().map(|(b, _, _)| b)).any(|b| b[3] != 0xffff) {
        return Err("a rule's byte is not a set of high nibbles".into());
    }
    let (mut byte_1_high, mut byte_1_low, mut byte_2_high) = ([0u8; 16], [0u8; 16], [0u8; 16]);
    let bits = boxes.iter().enumerate().map(|(k, b)| (1u8 << k, b)).chain(xor.iter().map(|(b, _, _)| (0x80u8, b)));
    for (bit, b) in bits {
        for n in 0..16 {
            if b[0] >> n & 1 == 1 {
                byte_1_high[n] |= bit;
            }
            if b[1] >> n & 1 == 1 {
                byte_1_low[n] |= bit;
            }
            if b[2] >> n & 1 == 1 {
                byte_2_high[n] |= bit;
            }
        }
    }
    let mut l = Lookup { byte_1_high, byte_1_low, byte_2_high, xor: xor.map(|(_, x, y)| (x, y)), open: [None; 3], covers: Vec::new() };
    // A block's last three bytes that leave a sequence open: each the least byte there where the next block, of ASCII,
    // holds an error.
    for k in 0..3 {
        let mut w = [0u8; 3];
        l.open[k] = (0..=255u8).find(|&b| {
            w = [0; 3];
            w[2 - k] = b;
            open(&l, w)
        });
    }
    // The bytes the statements tell apart, and those the check reads past the file.
    let mut sets: Vec<Vec<bool>> = Vec::new();
    for (_, _, p) in window.iter().chain(&ends) {
        classes(p, &mut sets);
    }
    sets.push((0..256).map(|b| b < 0x80).collect());
    for k in 0..3 {
        if let Some(t) = l.open[k] {
            sets.push((0..256).map(|b| b >= usize::from(t)).collect());
        }
    }
    if let Some((x, y)) = l.xor {
        sets.push((0..256).map(|b| b >= usize::from(x)).collect());
        sets.push((0..256).map(|b| b >= usize::from(y)).collect());
    }
    let mut reps: Vec<u8> = Vec::new();
    let mut seen: Vec<Vec<bool>> = Vec::new();
    for b in 0..=255u8 {
        let sig: Vec<bool> = sets.iter().map(|s| s[usize::from(b)]).collect();
        if !seen.contains(&sig) {
            seen.push(sig);
            reps.push(b);
        }
    }
    for &b in [0x00u8, b' '].iter().chain(prefix.unwrap_or(&[])) {
        if !reps.contains(&b) {
            reps.push(b);
        }
    }
    let befores: Vec<Vec<u8>> = std::iter::once(Vec::new()).chain(prefix.map(<[u8]>::to_vec)).collect();
    let syms: Vec<Option<u8>> = std::iter::once(None).chain(reps.iter().map(|&b| Some(b))).collect();
    // The statements' value at each window of four bytes, and at each end of three, tabulated.
    let n = syms.len();
    let at4 = |q: [usize; 4]| ((q[0] * n + q[1]) * n + q[2]) * n + q[3];
    let mut w4 = vec![false; n * n * n * n];
    for a in 0..n {
        for b in 0..n {
            for c in 0..n {
                for d in 1..n {
                    let s = [syms[a], syms[b], syms[c], syms[d]];
                    w4[at4([a, b, c, d])] = window.iter().any(|(_, _, p)| eval(p, &s, 3, 4));
                }
            }
        }
    }
    let mut e3 = vec![false; n * n * n];
    for x in 0..n {
        for y in 0..n {
            for z in 1..n {
                let s = [syms[x], syms[y], syms[z]];
                e3[(x * n + y) * n + z] = ends.iter().any(|(_, _, p)| (0..3).any(|i| eval(p, &s, i, 3)));
            }
        }
    }
    // Every window of four bytes: `None` a byte before the file (or of its `prefix`, which no mask reads).
    let valid = |t: &[usize]| t.windows(2).all(|p| p[0] == 0 || p[1] != 0);
    for a in 0..n {
        for b in 0..n {
            for c in 0..n {
                for d in 1..n {
                    if !valid(&[a, b, c, d]) {
                        continue;
                    }
                    let s = [syms[a], syms[b], syms[c], syms[d]];
                    for before in &befores {
                        if bad(&l, bytes(&s, before)) != w4[at4([a, b, c, d])] {
                            return Err(format!("the window {s:x?} (before it {before:x?}): the statements {}, the check not", w4[at4([a, b, c, d])]));
                        }
                    }
                }
            }
        }
    }
    // A `prefix`'s own bytes: no error there.
    if let Some(pre) = prefix {
        for i in 0..pre.len() {
            let w: [u8; 4] = std::array::from_fn(|j| (i + j).checked_sub(3).map_or(0, |k| pre[k]));
            if bad(&l, w) {
                return Err(format!("an error in the prefix: {w:x?}"));
            }
        }
    }
    // Every end of five bytes: the end statements there are the windows that read past the file, each side with the
    // errors already in it at the windows the five bytes decide (the last two bytes', or all at a file's start). The
    // end statements read only the last three bytes (`at_end`), the windows four; in a longer file, the last two
    // bytes' windows are its own, so the two refuse the same files.
    // Where the check's end and the statements' differ, each byte before the last four must leave an error inside.
    for code in 0..n.pow(4) {
        let mut t = [0usize; 5];
        let mut c = code;
        for k in (1..5).rev() {
            t[k] = c % n;
            c /= n;
        }
        if t[4] == 0 || !valid(&t[1..]) {
            continue;
        }
        let want = e3[(t[2] * n + t[3]) * n + t[4]];
        let s: [Option<u8>; 4] = std::array::from_fn(|j| syms[t[j + 1]]);
        for before in &befores {
            let [_, x, y, z] = bytes(&s, before);
            for pad in [0x00u8, b' '] {
                let got = bad(&l, [x, y, z, pad]) || bad(&l, [y, z, pad, pad]) || bad(&l, [z, pad, pad, pad]);
                if got == want {
                    continue;
                }
                for v in 0..n {
                    t[0] = v;
                    if !valid(&t) {
                        continue;
                    }
                    let quad = |i: usize| -> [usize; 4] { std::array::from_fn(|j| (i + j).checked_sub(3).map_or(0, |k| t[k])) };
                    if !(0..5).filter(|&i| t[i] != 0 && (i >= 3 || t[0] == 0)).any(|i| w4[at4(quad(i))]) {
                        let s: Vec<Option<u8>> = t.iter().map(|&i| syms[i]).collect();
                        return Err(format!("the end {s:x?} (padded {pad:#04x}, before it {before:x?}): the statements {want}, the check {got}"));
                    }
                }
            }
        }
    }
    // A block of ASCII: an error only at its first three bytes, where the block before left a sequence open.
    let ascii: Vec<u8> = reps.iter().copied().filter(|&b| b < 0x80).collect();
    for &a in &ascii {
        for &b in &ascii {
            for &c in &ascii {
                for &d in &ascii {
                    if bad(&l, [a, b, c, d]) {
                        return Err(format!("an error in ASCII: {:x?}", [a, b, c, d]));
                    }
                }
            }
        }
    }
    for &x in &reps {
        for &y in &reps {
            for &z in &reps {
                let by_test = [z, y, x].iter().zip(l.open).any(|(&b, t)| t.is_some_and(|t| b >= t));
                if ascii.iter().any(|&a| open_after(&l, [x, y, z], a) != by_test) {
                    return Err(format!("a block's end {:x?}: its test is not the next block's errors", [x, y, z]));
                }
            }
        }
    }
    l.covers = window.iter().chain(&ends).map(|(i, why, _)| (*i, why.clone())).collect();
    l.covers.sort();
    Ok(l)
}

/// `e` as a mask of classes, its names as `env` has them.
fn resolve(e: &Expr, env: &HashMap<String, Option<P>>) -> Option<P> {
    let b = |e: &Expr| resolve(e, env).map(std::boxed::Box::new);
    Some(match e {
        Expr::Name(n) => env.get(n)?.clone()?,
        Expr::Empty => P::Empty,
        Expr::Not(a) => P::Not(b(a)?),
        Expr::And(x, y) => P::And(b(x)?, b(y)?),
        Expr::Or(x, y) => P::Or(b(x)?, b(y)?),
        Expr::Xor(x, y) => P::Xor(b(x)?, b(y)?),
        Expr::Call(f, args) if f == "prev" || f == "next" => {
            let (a, k) = match args.as_slice() {
                [Arg::E(a)] => (a, 1),
                [Arg::E(a), Arg::N(k)] => (a, usize::try_from(*k).ok()?),
                _ => return None,
            };
            if f == "prev" { P::Prev(b(a)?, k) } else { P::Next(b(a)?, k) }
        }
        _ => return None,
    })
}

/// The names a statement defines: its own (`Stmt::defines`), and a `regions` statement's masks (its hidden bytes, each
/// kind's openers `o_K`, fill `f_K` and later openers).
fn defined(s: &Stmt, out: &mut Vec<String>) {
    s.defines(out);
    if let Stmt::Rounds { regions: Some(rg), .. } = s {
        out.push(rg.hidden.clone());
        for r in &rg.kinds {
            out.extend([r.kind.clone(), format!("o_{}", r.kind), format!("f_{}", r.kind), r.later()]);
        }
    }
}

/// How far back and ahead `p` reads, if it reads no further than its lags.
fn lags(p: &P) -> Option<(usize, usize)> {
    Some(match p {
        P::Class(_) | P::Empty => (0, 0),
        P::Not(a) => lags(a)?,
        P::And(a, b) | P::Or(a, b) | P::Xor(a, b) => {
            let (x, y) = (lags(a)?, lags(b)?);
            (x.0.max(y.0), x.1.max(y.1))
        }
        P::Prev(a, k) => {
            let (b, n) = lags(a)?;
            (b + k, n.saturating_sub(*k))
        }
        P::Next(a, k) => {
            let (b, n) = lags(a)?;
            (b.saturating_sub(*k), n + k)
        }
    })
}

/// `k` where `p` is the byte `k` before the file's last (`last_byte` is `~next(~empty)`: 0).
fn end_of(p: &P) -> Option<usize> {
    match p {
        P::Not(a) => match &**a {
            P::Next(b, 1) if matches!(&**b, P::Not(c) if matches!(**c, P::Empty)) => Some(0),
            _ => None,
        },
        P::Next(a, k) => end_of(a).map(|j| j + k),
        _ => None,
    }
}

/// Whether `p` holds only within three bytes of the file's end: each of its terms a class there.
fn at_end(p: &P) -> bool {
    let mut terms = Vec::new();
    ors(p, &mut terms);
    terms.iter().all(|t| {
        let mut fs = Vec::new();
        ands(t, &mut fs);
        let ends = fs.iter().filter_map(|f| end_of(f)).filter(|&k| k <= 2).count();
        ends == 1 && fs.iter().all(|f| end_of(f).is_some() || lags(f) == Some((0, 0)))
    })
}

/// `p`'s terms: its operands where it is an `or`.
fn ors<'p>(p: &'p P, out: &mut Vec<&'p P>) {
    match p {
        P::Or(a, b) => {
            ors(a, out);
            ors(b, out);
        }
        _ => out.push(p),
    }
}

/// `p`'s factors: its operands where it is an `and`.
fn ands<'p>(p: &'p P, out: &mut Vec<&'p P>) {
    match p {
        P::And(a, b) => {
            ands(a, out);
            ands(b, out);
        }
        _ => out.push(p),
    }
}

/// The classes `p` reads, each a set of bytes.
fn classes(p: &P, out: &mut Vec<Vec<bool>>) {
    match p {
        P::Class(s) => out.push(s.to_vec()),
        P::Empty => {}
        P::Not(a) | P::Prev(a, _) | P::Next(a, _) => classes(a, out),
        P::And(a, b) | P::Or(a, b) | P::Xor(a, b) => {
            classes(a, out);
            classes(b, out);
        }
    }
}

/// A term of the window statements as a rule.
enum Rule {
    /// The byte before in the first set, this byte in the second.
    Pair([bool; 256], [bool; 256]),
    /// The XOR rule: its pair, and its least bytes 2 and 3 back.
    Xor([bool; 256], [bool; 256], u8, u8),
}

/// The class `p` is, at no lag.
fn set(p: &P) -> Option<[bool; 256]> {
    match p {
        P::Class(s) => Some(**s),
        P::Or(a, b) => {
            let (a, b) = (set(a)?, set(b)?);
            Some(std::array::from_fn(|i| a[i] || b[i]))
        }
        _ => None,
    }
}

/// The class `p` is at lag `k`.
fn set_at(p: &P, k: usize) -> Option<[bool; 256]> {
    match p {
        P::Prev(a, j) if *j == k => set(a),
        _ => None,
    }
}

/// `t` as a rule: `prev(L, 1) & C`, `C & ~prev(L, 1)` (the byte before not in `L`), or the XOR rule.
fn rule(t: &P) -> Option<Rule> {
    let pair = |a: &P, b: &P| -> Option<([bool; 256], [bool; 256])> {
        if let (Some(l), Some(c)) = (set_at(a, 1), set(b)) {
            return Some((l, c));
        }
        if let (Some(l), Some(c)) = (set_at(b, 1), set(a)) {
            return Some((l, c));
        }
        let outside = |n: &P, c: &P| match n {
            P::Not(x) => Some((set_at(x, 1)?, set(c)?)),
            _ => None,
        };
        let (l, c) = outside(b, a).or_else(|| outside(a, b))?;
        Some((std::array::from_fn(|i| !l[i]), c))
    };
    match t {
        P::And(a, b) => pair(a, b).map(|(l, c)| Rule::Pair(l, c)),
        P::Xor(a, b) => {
            let (P::And(x, y), P::Or(m, n)) = (&**a, &**b) else { return None };
            let (l, c) = pair(x, y)?;
            let from = |s: [bool; 256]| -> Option<u8> {
                let t = (0..256).find(|&i| s[i])?;
                (t..256).all(|i| s[i]).then_some(t as u8).filter(|_| (0..t).all(|i| !s[i]))
            };
            let (x, y) = match (set_at(m, 2), set_at(n, 3)) {
                (Some(x), Some(y)) => (x, y),
                _ => (set_at(n, 2)?, set_at(m, 3)?),
            };
            Some(Rule::Xor(l, c, from(x)?, from(y)?))
        }
        _ => None,
    }
}

/// A rule's box, each of its sets one product of nibbles.
fn box_of(before: &[bool; 256], here: &[bool; 256]) -> Option<crate::native_cpu_pairs::Box> {
    let (bh, bl) = crate::native_cpu_pairs::product(before)?;
    let (hh, hl) = crate::native_cpu_pairs::product(here)?;
    Some([bh, bl, hh, hl])
}

/// `p` at byte `i` of the file `s` (its first `len` bytes; `None` a byte before it, or of its `prefix`).
fn eval(p: &P, s: &[Option<u8>], i: usize, len: usize) -> bool {
    let at = |i: usize| i < len && i < s.len() && s[i].is_some();
    match p {
        P::Class(set) => at(i) && set[usize::from(s[i].expect("a byte"))],
        P::Empty => false,
        P::Not(a) => at(i) && !eval(a, s, i, len),
        P::And(a, b) => eval(a, s, i, len) && eval(b, s, i, len),
        P::Or(a, b) => eval(a, s, i, len) || eval(b, s, i, len),
        P::Xor(a, b) => eval(a, s, i, len) != eval(b, s, i, len),
        P::Prev(a, k) => i >= *k && eval(a, s, i - k, len),
        P::Next(a, k) => eval(a, s, i + k, len),
    }
}

/// The bytes the check reads for the window `s`: a byte before the file is the `prefix`'s (`before`), from its end, or
/// 0x00 (simdjson's first `prev_input_block`).
fn bytes(s: &[Option<u8>; 4], before: &[u8]) -> [u8; 4] {
    let first = s.iter().position(Option::is_some).unwrap_or(4);
    std::array::from_fn(|j| match s[j] {
        Some(b) => b,
        None => (before.len() + j).checked_sub(first).map_or(0, |k| before[k]),
    })
}

/// The check's error at the last byte of `w` (the bytes 3, 2 and 1 back, then this one).
fn bad(l: &Lookup, w: [u8; 4]) -> bool {
    let (p1, here) = (w[2], w[3]);
    let sc = l.byte_1_high[usize::from(p1 >> 4)] & l.byte_1_low[usize::from(p1 & 15)] & l.byte_2_high[usize::from(here >> 4)];
    let must = l.xor.is_some_and(|(x, y)| w[1] >= x || w[0] >= y);
    (sc ^ if must { 0x80 } else { 0 }) != 0
}

/// Whether a block ending in `w` holds an error at the next block's first three bytes, all 0x00.
fn open(l: &Lookup, w: [u8; 3]) -> bool {
    open_after(l, w, 0)
}

/// Whether a block ending in `w` holds an error at the next block's first three bytes, all `a`.
fn open_after(l: &Lookup, w: [u8; 3], a: u8) -> bool {
    let [x, y, z] = w;
    bad(l, [x, y, z, a]) || bad(l, [y, z, a, a]) || bad(l, [z, a, a, a])
}
