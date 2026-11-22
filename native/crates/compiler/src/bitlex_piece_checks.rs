//! A grammar's errors at an escape, tested where tier 2 reads the escape (decision 0599).
//!
//! JSON's grammar states `\u`'s checks as masks anchored at an escape's second byte: `unicode = esc_last & u`, its four
//! hex digits `next(hex, 1)` to `next(hex, 4)`, the surrogates `high` and `low` after it, and a low one's high one six
//! bytes before (`prev(high, 6)`). Tier 1 wrote them as masks at every block. The lean parser checks them in stage 2,
//! where it reads an escape. Tier 2's region reader stops at each escape piece too (decision 0586), so these errors are
//! tested there, from the bytes. Tier 1 leaves them out, and with them the classes and masks only they read.
//!
//! A statement is one of these where:
//!
//! - the region's piece starts at `B & R & ~E` (in any order), with `E` `escaped(B)` and `B` a class: the escape's
//!   backslash;
//! - a mask whose conjuncts are `E` and `R` is the anchor, the piece's second byte, true at each piece;
//! - a mask (or the error) is anchored where one of its conjuncts is an anchored mask, at its own byte, and the others
//!   are read from the bytes: a class at an offset (`next`, `prev`), a mask of classes only, an anchored mask at an
//!   earlier byte (`prev`: the pieces before, a bit each, in one word), and `~`, `&`, `|`, `^` of these.
//!
//! An anchored mask holds only at a piece's second byte, so it is read at each piece and nowhere else. What falls
//! outside this is left as it was: tier 1 keeps it.
//!
//! As the lean parser's `escaped_string` reads them:
//!
//! - the digits the first error requires (one to four hex digits at consecutive bytes) are one number, `cp`, its error
//!   `cp` past them. Each run of digit classes (sets of hex digits, both cases) at consecutive bytes is then a range:
//!   of `cp` within its digits, else of a number of its own. Only where the classes spell one interval;
//! - a piece before read at one distance `k` alone, where no piece can start in the `k - 1` bytes from it, is one
//!   position: the second byte `k` after the last one (`low_checked`). Otherwise it is a word of the pieces before.
//!
//! The pieces before are the string's: a `prev` that reaches past the string's opening quote reads no piece there. For
//! the `prev` of JSON's grammar (6, from a `u` to the one before) that is never so: from a `u` back to a string before,
//! there are its four bytes after its `u`, a closer, at least one byte between, an opener and a backslash.

use ::bitlex::syntax::{Arg, Expr, Stmt};
use std::collections::HashMap;
use std::fmt::Write;

/// A byte the padding after the file holds (`PADDING` spaces): no class read past the file may hold it.
const PAD: u8 = b' ';

/// A read of the bytes at a piece, relative to its second byte `y`.
#[derive(Clone, Debug)]
enum R {
    True,
    Class(Box<[bool; 256]>, i64),
    /// Anchored mask `id` at `y + off`: `off` 0, this piece's; below, a piece before.
    Anch(usize, i64),
    Not(Box<R>),
    And(Box<R>, Box<R>),
    Or(Box<R>, Box<R>),
    Xor(Box<R>, Box<R>),
    /// Number `var` (digits read from the bytes) in `lo..=hi`; `full`, the most its digits hold.
    Num(String, u32, u32, u32),
}

impl R {
    fn shift(&self, k: i64) -> Option<R> {
        Some(match self {
            R::True => R::True,
            R::Class(s, o) => R::Class(s.clone(), o + k),
            // An anchored mask's operand is read at its own byte only: a mask of classes holds none.
            R::Anch(..) | R::Num(..) => return None,
            R::Not(a) => R::Not(Box::new(a.shift(k)?)),
            R::And(a, b) => R::And(Box::new(a.shift(k)?), Box::new(b.shift(k)?)),
            R::Or(a, b) => R::Or(Box::new(a.shift(k)?), Box::new(b.shift(k)?)),
            R::Xor(a, b) => R::Xor(Box::new(a.shift(k)?), Box::new(b.shift(k)?)),
        })
    }
}

/// What a name is, where a statement reads it.
#[derive(Clone)]
enum Def {
    Class(Box<[bool; 256]>),
    /// A mask of classes alone, read at any offset.
    Pure(R),
    Anchored(usize),
    /// A mask by its expression, for the anchor's match.
    Other(Expr),
}

/// The errors tested at each piece, and their code.
pub struct PieceChecks {
    /// The statements it covers: tier 1 leaves them out.
    pub covers: Vec<usize>,
    /// The string's state, before its first piece: each word of the pieces before.
    pub state: String,
    /// The test at a piece whose first byte is at `p`: an error refuses at its byte.
    pub at_piece: String,
    /// The module's tables the test reads.
    pub statics: String,
}

fn conjuncts<'e>(e: &'e Expr, out: &mut Vec<&'e Expr>) {
    match e {
        Expr::And(a, b) => {
            conjuncts(a, out);
            conjuncts(b, out);
        }
        e => out.push(e),
    }
}

/// `e` with its names that are masks of one name or call unfolded, for the anchor's match.
fn unfold(e: &Expr, scope: &HashMap<String, Def>) -> String {
    match e {
        Expr::Name(n) => match scope.get(n) {
            Some(Def::Other(d @ (Expr::Name(_) | Expr::Call(..)))) => unfold(d, scope),
            _ => n.clone(),
        },
        Expr::Call(f, args) => {
            let a: Vec<String> = args
                .iter()
                .map(|a| match a {
                    Arg::E(e) => unfold(e, scope),
                    Arg::N(n) => n.to_string(),
                    Arg::S(s) => format!("{s:?}"),
                })
                .collect();
            format!("{f}({})", a.join(","))
        }
        Expr::Not(a) => format!("~{}", unfold(a, scope)),
        e => format!("{e:?}"),
    }
}

/// `e`'s read at the piece, at `off` from its second byte; `None` where it is not one.
fn read(e: &Expr, off: i64, scope: &HashMap<String, Def>) -> Option<R> {
    Some(match e {
        Expr::Name(n) => match scope.get(n)? {
            Def::Class(s) => R::Class(s.clone(), off),
            Def::Pure(r) => r.shift(off)?,
            Def::Anchored(id) if off <= 0 => R::Anch(*id, off),
            _ => return None,
        },
        Expr::Call(f, args) if f == "next" || f == "prev" => {
            let [Arg::E(x), Arg::N(k)] = args.as_slice() else { return None };
            let k = i64::try_from(*k).ok()?;
            read(x, if f == "next" { off + k } else { off - k }, scope)?
        }
        Expr::Not(a) => R::Not(Box::new(read(a, off, scope)?)),
        Expr::And(a, b) => R::And(Box::new(read(a, off, scope)?), Box::new(read(b, off, scope)?)),
        Expr::Or(a, b) => R::Or(Box::new(read(a, off, scope)?), Box::new(read(b, off, scope)?)),
        Expr::Xor(a, b) => R::Xor(Box::new(read(a, off, scope)?), Box::new(read(b, off, scope)?)),
        _ => return None,
    })
}

/// Whether `r` reads only what it may: no anchored mask past its byte, no class holding the padding's byte, offsets in
/// the padding and a word's pieces before.
fn fits(r: &R, anchored: bool) -> bool {
    match r {
        R::True | R::Num(..) => true,
        R::Class(s, o) => !s[usize::from(PAD)] && (-16..=16).contains(o),
        R::Anch(_, o) => *o == 0 || (anchored && (-63..0).contains(o)),
        R::Not(a) => fits(a, anchored),
        R::And(a, b) | R::Or(a, b) | R::Xor(a, b) => fits(a, anchored) && fits(b, anchored),
    }
}

/// A conjunct list's read, where one conjunct is an anchored mask at its own byte: the mask's (or error's) read.
fn anchored(e: &Expr, scope: &HashMap<String, Def>, anchor: usize) -> Option<R> {
    let mut cs = Vec::new();
    conjuncts(e, &mut cs);
    let rs: Vec<R> = cs.iter().map(|c| read(c, 0, scope)).collect::<Option<_>>()?;
    if !rs.iter().any(|r| matches!(r, R::Anch(_, 0))) || !rs.iter().all(|r| fits(r, true)) {
        return None;
    }
    // The anchor holds at every piece.
    Some(rs.into_iter().map(|r| if matches!(r, R::Anch(a, 0) if a == anchor) { R::True } else { r }).reduce(|a, b| R::And(Box::new(a), Box::new(b))).expect("a conjunct"))
}

fn pattern(set: &[bool; 256]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut b = 0usize;
    while b < 256 {
        if !set[b] {
            b += 1;
            continue;
        }
        let mut j = b;
        while j + 1 < 256 && set[j + 1] {
            j += 1;
        }
        out.push(if j == b { format!("{b:#04x}") } else { format!("{b:#04x}..={j:#04x}") });
        b = j + 1;
    }
    if out.is_empty() { "_ if false".to_owned() } else { out.join(" | ") }
}

/// `r` as Rust, the masks `a` true (the anchor, and those the gate is).
fn code(r: &R, a: &[usize]) -> String {
    match r {
        R::True => "true".to_owned(),
        R::Class(s, 0) => format!("matches!(src[y], {})", pattern(s)),
        R::Class(s, o) if *o > 0 => format!("matches!(src[y + {o}], {})", pattern(s)),
        R::Class(s, o) => format!("(y >= {k} && matches!(src[y - {k}], {}))", pattern(s), k = -o),
        R::Anch(id, 0) if a.contains(id) => "true".to_owned(),
        R::Anch(id, 0) => format!("m{id}"),
        R::Anch(id, o) => format!("h{id} >> {} & 1 != 0", -o),
        R::Num(v, 0, hi, full) if hi == full => format!("{v} <= {full:#x}"),
        R::Num(v, lo, hi, _) => format!("({lo:#x}..={hi:#x}).contains(&{v})"),
        R::Not(x) if matches!(&**x, R::Num(_, 0, hi, full) if hi == full) => {
            let R::Num(v, _, _, full) = &**x else { unreachable!() };
            format!("{v} > {full:#x}")
        }
        R::Not(x) => format!("!({})", code(x, a)),
        R::And(x, z) if matches!(**x, R::True) || matches!(**x, R::Anch(id, 0) if a.contains(&id)) => code(z, a),
        R::And(x, z) if matches!(**z, R::True) || matches!(**z, R::Anch(id, 0) if a.contains(&id)) => code(x, a),
        R::And(x, z) => format!("{} && {}", code(x, a), code(z, a)),
        R::Or(x, z) => format!("({} || {})", code(x, a), code(z, a)),
        R::Xor(x, z) => format!("({} ^ {})", code(x, a), code(z, a)),
    }
}

/// `r` where the byte at `y` is of `g`: its test of that, true.
fn within(r: &R, g: &[bool; 256]) -> R {
    match r {
        R::Class(s, 0) if **s == *g => R::True,
        R::Not(x) => R::Not(Box::new(within(x, g))),
        R::And(x, z) => match (within(x, g), within(z, g)) {
            (R::True, z) => z,
            (x, R::True) => x,
            (x, z) => R::And(Box::new(x), Box::new(z)),
        },
        R::Or(x, z) => R::Or(Box::new(within(x, g)), Box::new(within(z, g))),
        R::Xor(x, z) => R::Xor(Box::new(within(x, g)), Box::new(within(z, g))),
        r => r.clone(),
    }
}

/// A hex digit's value, or none.
fn digit(c: u8) -> Option<u32> {
    char::from(c).to_digit(16)
}

/// The digit values a class is, where it is a set of hex digits that holds both cases of each (a bit a value).
fn digit_values(s: &[bool; 256]) -> Option<u16> {
    let mut vs = 0u16;
    for c in 0..=255u8 {
        match (s[usize::from(c)], digit(c)) {
            (true, None) => return None,
            (true, Some(v)) => vs |= 1 << v,
            _ => {}
        }
    }
    let closed = (b'a'..=b'f').all(|c| s[usize::from(c)] == s[usize::from(c.to_ascii_uppercase())]);
    closed.then_some(vs)
}

/// The numbers the digits `d` (most significant first, each a set of values) spell: `lo..=hi` where they are all of it.
fn interval(d: &[u16]) -> Option<(u32, u32)> {
    let n = 1u32 << (4 * d.len());
    let has = |v: u32| d.iter().enumerate().all(|(j, set)| set >> ((v >> (4 * (d.len() - 1 - j))) & 15) & 1 == 1);
    let lo = (0..n).find(|&v| has(v))?;
    let hi = (0..n).rev().find(|&v| has(v))?;
    (lo..=hi).all(has).then_some((lo, hi))
}

fn and_chain(r: &R, out: &mut Vec<R>) {
    match r {
        R::And(a, b) => {
            and_chain(a, out);
            and_chain(b, out);
        }
        R::True => {}
        r => out.push(r.clone()),
    }
}

/// `r` with each run of hex-digit classes at consecutive offsets after the piece's byte read as one number (as
/// simdjson's `hex_to_u32_nocheck`): a range of `cp` where the run is within its digits (`cp`, the four from `at`), else
/// a range of a number of the run's own, named into `own`.
fn numbers(r: &R, cp: Option<(i64, usize)>, own: &mut Vec<(i64, usize)>) -> R {
    match r {
        R::And(..) => {
            let mut cs = Vec::new();
            and_chain(r, &mut cs);
            let mut digits: Vec<(i64, u16)> = Vec::new();
            let mut rest = Vec::new();
            for c in cs {
                match &c {
                    R::Class(s, o) if *o > 0 && let Some(v) = digit_values(s) => match digits.iter_mut().find(|d| d.0 == *o) {
                        Some(d) => d.1 &= v,
                        None => digits.push((*o, v)),
                    },
                    R::Class(..) => rest.push(c),
                    _ => rest.push(numbers(&c, cp, own)),
                }
            }
            digits.sort();
            let mut i = 0;
            while i < digits.len() {
                let mut j = i;
                while j + 1 < digits.len() && digits[j + 1].0 == digits[j].0 + 1 {
                    j += 1;
                }
                let run = &digits[i..=j];
                let (a, b) = (run[0].0, run[run.len() - 1].0);
                let within = cp.filter(|&(at, k)| a >= at && b < at + k as i64);
                let (at, k) = within.unwrap_or((a, run.len()));
                let set: Vec<u16> = (0..k as i64).map(|o| run.iter().find(|d| d.0 == at + o).map_or(0xffff, |d| d.1)).collect();
                match (k <= 4).then(|| interval(&set)).flatten() {
                    Some((lo, hi)) => {
                        let var = if within.is_some() { "cp".to_owned() } else {
                            if !own.contains(&(at, k)) {
                                own.push((at, k));
                            }
                            format!("n{at}")
                        };
                        rest.push(R::Num(var, lo, hi, (1 << (4 * k)) - 1));
                    }
                    None => rest.extend(run.iter().map(|d| R::Class(Box::new(std::array::from_fn(|c| digit(c as u8).is_some_and(|v| d.1 >> v & 1 == 1))), d.0))),
                }
                i = j + 1;
            }
            rest.into_iter().reduce(|a, b| R::And(Box::new(a), Box::new(b))).unwrap_or(R::True)
        }
        R::Class(..) => numbers(&R::And(Box::new(r.clone()), Box::new(R::True)), cp, own),
        R::Not(x) => R::Not(Box::new(numbers(x, cp, own))),
        R::Or(x, z) => R::Or(Box::new(numbers(x, cp, own)), Box::new(numbers(z, cp, own))),
        R::Xor(x, z) => R::Xor(Box::new(numbers(x, cp, own)), Box::new(numbers(z, cp, own))),
        r => r.clone(),
    }
}

/// The digits code for `k` digits from `y + at`: each byte's value placed, one not hex putting bits past them.
fn digits_code(at: i64, k: usize) -> String {
    (0..k).map(|j| format!("HEX_VALUE[usize::from(src[y + {}])] << {}", at + j as i64, 4 * (k - 1 - j))).collect::<Vec<_>>().join(" | ").replace(" << 0", "")
}

/// The classes `r` holds of at offsets, where it holds (its top `&`s), the masks it reads at its byte unfolded.
fn holds(r: &R, masks: &[(String, R)], out: &mut Vec<(i64, [bool; 256])>) {
    match r {
        R::And(a, b) => {
            holds(a, masks, out);
            holds(b, masks, out);
        }
        R::Class(s, o) => out.push((*o, **s)),
        R::Anch(id, 0) => holds(&masks[*id].1, masks, out),
        _ => {}
    }
}

fn reads_back(r: &R, out: &mut Vec<usize>) {
    match r {
        R::Anch(id, o) if *o < 0 => {
            if !out.contains(id) {
                out.push(*id);
            }
        }
        R::Not(a) => reads_back(a, out),
        R::And(a, b) | R::Or(a, b) | R::Xor(a, b) => {
            reads_back(a, out);
            reads_back(b, out);
        }
        _ => {}
    }
}

/// The errors at the pieces that start at the mask `part_start` (the region's one part): `Err` says why there are
/// none.
pub fn piece_checks(stmts: &[Stmt], part_start: &str) -> Result<PieceChecks, String> {
    let mut scope: HashMap<String, Def> = HashMap::new();
    let mut anchor: Option<(String, usize)> = None;
    // The pieces' first byte's class (`B`).
    let mut lead: Option<[bool; 256]> = None;
    // The anchored masks: each one's read, in order.
    let mut masks: Vec<(String, R)> = Vec::new();
    let mut errors: Vec<(usize, String, R)> = Vec::new();
    for (i, s) in stmts.iter().enumerate() {
        match s {
            Stmt::Class(name, items) => {
                let mut set = Box::new([false; 256]);
                for &(lo, hi) in items {
                    for c in lo..=hi.min(255) {
                        set[c as usize] = true;
                    }
                }
                scope.insert(name.clone(), Def::Class(set));
            }
            Stmt::Mask(name, e) => {
                // The piece's start: its conjuncts `B`, `R`, `~E`, with `E` `escaped(B)`.
                if name == part_start {
                    let mut cs = Vec::new();
                    conjuncts(e, &mut cs);
                    let un: Vec<String> = cs.iter().map(|c| unfold(c, &scope)).collect();
                    if let [_, _, _] = un.as_slice() {
                        let b = un.iter().find(|u| matches!(scope.get(u.as_str()), Some(Def::Class(_))));
                        let esc = un.iter().find(|u| u.starts_with('~'));
                        if let (Some(b), Some(esc)) = (b, esc) {
                            let r = un.iter().find(|u| *u != b && *u != esc);
                            if esc[1..] == format!("escaped({b})")
                                && let Some(r) = r
                            {
                                if let Some(Def::Class(set)) = scope.get(b.as_str()) {
                                    lead = Some(**set);
                                }
                                anchor = Some((format!("{}&{r}", &esc[1..]), usize::MAX));
                            }
                        }
                    }
                }
                let def = match &anchor {
                    // The anchor: `E & R`.
                    Some((key, usize::MAX)) => {
                        let mut cs = Vec::new();
                        conjuncts(e, &mut cs);
                        let mut un: Vec<String> = cs.iter().map(|c| unfold(c, &scope)).collect();
                        un.sort();
                        let mut want: Vec<String> = key.split('&').map(str::to_owned).collect();
                        want.sort();
                        if un == want {
                            let id = masks.len();
                            masks.push((name.clone(), R::True));
                            anchor = Some((key.clone(), id));
                            Some(Def::Anchored(id))
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                let def = match (def, &anchor) {
                    (Some(d), _) => d,
                    (None, Some((_, a))) if *a != usize::MAX => match anchored(e, &scope, *a) {
                        Some(r) => {
                            let id = masks.len();
                            masks.push((name.clone(), r));
                            Def::Anchored(id)
                        }
                        None => match read(e, 0, &scope) {
                            Some(r) if fits(&r, false) => Def::Pure(r),
                            _ => Def::Other(e.clone()),
                        },
                    },
                    _ => match read(e, 0, &scope) {
                        Some(r) if fits(&r, false) => Def::Pure(r),
                        _ => Def::Other(e.clone()),
                    },
                };
                scope.insert(name.clone(), def);
            }
            Stmt::Error(why, e) => {
                if let Some((_, a)) = &anchor
                    && *a != usize::MAX
                    && let Some(r) = anchored(e, &scope, *a)
                {
                    errors.push((i, why.clone(), r));
                }
            }
            _ => {}
        }
    }
    let Some((_, a)) = anchor else { return Err(format!("the piece's start `{part_start}` is no escape's `B & R & ~escaped(B)`")) };
    if a == usize::MAX {
        return Err("no mask is the escape's second byte (`escaped(B) & R`)".into());
    }
    if errors.is_empty() {
        return Err("no error is anchored at an escape".into());
    }
    // The masks the errors read, and those they read, in order.
    let mut need = vec![false; masks.len()];
    let mut back = Vec::new();
    fn mark(r: &R, need: &mut [bool]) {
        match r {
            R::Anch(id, _) => need[*id] = true,
            R::Not(a) => mark(a, need),
            R::And(a, b) | R::Or(a, b) | R::Xor(a, b) => {
                mark(a, need);
                mark(b, need);
            }
            _ => {}
        }
    }
    for (_, _, r) in &errors {
        mark(r, &mut need);
        reads_back(r, &mut back);
    }
    for id in (0..masks.len()).rev() {
        if need[id] {
            let r = masks[id].1.clone();
            mark(&r, &mut need);
            reads_back(&r, &mut back);
        }
    }
    back.sort();
    // The byte every error holds at only (and every word of pieces before): the test runs at those escapes alone, as
    // the lean parser's `\u` case. A word skipped at another escape is the same word shifted later.
    fn gates(r: &R, masks: &[(String, R)], out: &mut Vec<[bool; 256]>) {
        match r {
            R::Class(set, 0) => out.push(**set),
            R::Anch(id, 0) => gates(&masks[*id].1, masks, out),
            R::And(x, z) => {
                gates(x, masks, out);
                gates(z, masks, out);
            }
            _ => {}
        }
    }
    let of = |r: &R| {
        let mut g = Vec::new();
        gates(r, &masks, &mut g);
        g
    };
    let gate = of(&errors[0].2).into_iter().find(|g| errors.iter().map(|e| &e.2).chain(back.iter().map(|id| &masks[*id].1)).all(|r| of(r).contains(g)));
    // Inside the gate, its test is true.
    if let Some(g) = &gate {
        for m in &mut masks {
            m.1 = within(&m.1, g);
        }
        for e in &mut errors {
            e.2 = within(&e.2, g);
        }
    }
    // The digits the first error requires, read as one number (`cp`, simdjson's `hex_to_u32_nocheck`): its test
    // first, so each range of `cp` after it reads valid digits.
    // An error's conjuncts but those true at every piece in the gate.
    let bare = |r: &R, masks: &[(String, R)]| {
        let mut cs = Vec::new();
        and_chain(r, &mut cs);
        cs.retain(|c| !matches!(c, R::Anch(id, 0) if *id == a || matches!(masks[*id].1, R::True)));
        cs
    };
    let cp = match bare(&errors[0].2, &masks).as_slice() {
        [R::Not(x)] => {
            let mut cs = Vec::new();
            and_chain(x, &mut cs);
            let mut os: Vec<i64> = cs.iter().filter_map(|c| match c {
                R::Class(s, o) if *o > 0 && digit_values(s) == Some(0xffff) => Some(*o),
                _ => None,
            }).collect();
            os.sort();
            (os.len() == cs.len() && (1..=4).contains(&os.len()) && os.windows(2).all(|w| w[1] == w[0] + 1)).then(|| (os[0], os.len()))
        }
        _ => None,
    };
    let mut own = Vec::new();
    if cp.is_some() {
        for m in &mut masks {
            m.1 = numbers(&m.1, cp, &mut own);
        }
        for e in &mut errors {
            e.2 = numbers(&e.2, cp, &mut own);
        }
    }
    // A piece before read at one distance `k` only, where no piece starts in the `k - 1` bytes from it (the classes
    // it holds of there, its own and those the errors before require, are not the pieces' first byte): the last one's
    // position is the only one that can be `k` back, so it is kept as the second byte `k` on (the lean parser's
    // `low_checked`).
    let mut after: Vec<(usize, i64)> = Vec::new();
    for &id in &back {
        let mut ks = Vec::new();
        fn dists(r: &R, id: usize, out: &mut Vec<i64>) {
            match r {
                R::Anch(i, o) if *i == id && *o < 0 => out.push(-o),
                R::Not(a) => dists(a, id, out),
                R::And(a, b) | R::Or(a, b) | R::Xor(a, b) => {
                    dists(a, id, out);
                    dists(b, id, out);
                }
                _ => {}
            }
        }
        for r in errors.iter().map(|e| &e.2).chain(masks.iter().map(|m| &m.1)) {
            dists(r, id, &mut ks);
        }
        ks.sort();
        ks.dedup();
        let [k] = ks.as_slice() else { continue };
        let mut held = Vec::new();
        holds(&R::Anch(id, 0), &masks, &mut held);
        if let Some(g) = &gate {
            held.push((0, *g));
        }
        // An error of one `~x` alone, tested before: `x` holds.
        for (_, _, r) in &errors {
            if let [R::Not(x)] = bare(r, &masks).as_slice() {
                holds(x, &masks, &mut held);
            }
        }
        // The digits' error before: its bytes hex.
        if let Some((at, n)) = cp {
            held.extend((at..at + n as i64).map(|o| (o, std::array::from_fn(|c| digit(c as u8).is_some()))));
        }
        let Some(b) = &lead else { continue };
        if (0..k - 1).all(|o| held.iter().any(|(ho, set)| *ho == o && (0..256).all(|c| !(set[c] && b[c])))) {
            after.push((id, *k));
        }
    }
    let mut state = String::new();
    for id in &back {
        match after.iter().find(|a| a.0 == *id) {
            Some((_, k)) => {
                let _ = writeln!(state, "// The second byte {k} after the string's last `{}` piece: the only one a piece reads it at (0: none).\nlet mut after{id} = 0usize;", masks[*id].0);
            }
            None => {
                let _ = writeln!(state, "// `{}` at the string's pieces before, a bit a byte back from the last one's second byte (`y{id}`).\nlet (mut w{id}, mut y{id}) = (0u64, 0usize);", masks[*id].0);
            }
        }
    }
    let mut at = String::from("// The grammar's errors at an escape, from its bytes: its second byte `y`.\nlet y = p + 1;\n");
    // Inside the gate, a mask that is it is true too.
    let mut truth = vec![a];
    if let Some(g) = &gate {
        let _ = writeln!(at, "if matches!(src[y], {}) {{", pattern(g));
        for (id, m) in masks.iter().enumerate() {
            if matches!(m.1, R::True) {
                truth.push(id);
            }
        }
    }
    if let Some((o, k)) = cp {
        let _ = writeln!(at, "// The {k} digits as a number (simdjson's `hex_to_u32_nocheck`): a byte not hex puts bits past them.\nlet cp = {};", digits_code(o, k));
    }
    for (o, k) in &own {
        let _ = writeln!(at, "let n{o} = {};", digits_code(*o, *k));
    }
    for id in &back {
        if !after.iter().any(|a| a.0 == *id) {
            let _ = writeln!(at, "let h{id} = if y - y{id} < 64 {{ w{id} << (y - y{id}) }} else {{ 0 }};");
        }
    }
    let back_code = |r: &R| {
        let mut c = code(r, &truth);
        for (id, k) in &after {
            c = c.replace(&format!("!(h{id} >> {k} & 1 != 0)"), &format!("after{id} != y")).replace(&format!("h{id} >> {k} & 1 != 0"), &format!("after{id} == y"));
        }
        c
    };
    for (id, (name, r)) in masks.iter().enumerate() {
        if need[id] && !truth.contains(&id) {
            let _ = writeln!(at, "// mask {name}\nlet m{id} = {};", back_code(r));
        }
    }
    for (_, why, r) in &errors {
        let _ = write!(at, "// {why}\nif {} {{\nreturn Err(y as u32);\n}}\n", back_code(r));
    }
    for id in &back {
        let m = code(&R::Anch(*id, 0), &truth);
        match after.iter().find(|a| a.0 == *id) {
            Some((_, k)) => {
                let _ = writeln!(at, "if {m} {{\nafter{id} = y + {k};\n}}");
            }
            None => {
                let _ = writeln!(at, "(w{id}, y{id}) = (h{id} | u64::from({m}), y);");
            }
        }
    }
    if gate.is_some() {
        at.push_str("}\n");
    }
    let statics = if cp.is_some() || !own.is_empty() {
        format!("/// Each byte's hex digit value, or bits past four digits' where it is none (simdjson's `digit_to_val32`).\nstatic HEX_VALUE: [u32; 256] = [{}];\n\n", (0..=255u8).map(|c| digit(c).map_or("0xffff_0000".to_owned(), |v| format!("{v}"))).collect::<Vec<_>>().join(", "))
    } else {
        String::new()
    };
    Ok(PieceChecks { covers: errors.iter().map(|(i, _, _)| *i).collect(), state, at_piece: at, statics })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(bytes: &[u8]) -> [bool; 256] {
        std::array::from_fn(|c| bytes.contains(&(c as u8)))
    }

    #[test]
    fn digit_classes_are_ranges_where_they_spell_one() {
        assert_eq!(digit_values(&class(b"dD")), Some(1 << 13));
        assert_eq!(digit_values(&class(b"d")), None, "one case only");
        assert_eq!(digit_values(&class(b"dg")), None, "not a digit");
        let high2 = digit_values(&class(b"89abAB")).expect("digits");
        let low2 = digit_values(&class(b"cdefCDEF")).expect("digits");
        // `high` and `low`: D, then 8..B or C..F, then any two.
        assert_eq!(interval(&[1 << 13, high2, 0xffff, 0xffff]), Some((0xd800, 0xdbff)));
        assert_eq!(interval(&[1 << 13, low2, 0xffff, 0xffff]), Some((0xdc00, 0xdfff)));
        assert_eq!(interval(&[1 << 13, low2]), Some((0xdc, 0xdf)));
        // Not one interval: any first digit, then 0.
        assert_eq!(interval(&[0xffff, 1]), None);
    }
}
