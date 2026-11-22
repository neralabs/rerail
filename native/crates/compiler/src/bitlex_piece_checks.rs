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
}

impl R {
    fn shift(&self, k: i64) -> Option<R> {
        Some(match self {
            R::True => R::True,
            R::Class(s, o) => R::Class(s.clone(), o + k),
            // An anchored mask's operand is read at its own byte only: a mask of classes holds none.
            R::Anch(..) => return None,
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
        R::True => true,
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
    let mut state = String::new();
    for id in &back {
        let _ = writeln!(state, "// `{}` at the string's pieces before, a bit a byte back from the last one's second byte (`y{id}`).\nlet (mut w{id}, mut y{id}) = (0u64, 0usize);", masks[*id].0);
    }
    let mut at = String::from("// The grammar's errors at an escape, from its bytes: its second byte `y`.\nlet y = p + 1;\n");
    // Inside the gate, its test is true: a mask that is it, too.
    let mut truth = vec![a];
    if let Some(g) = &gate {
        let _ = writeln!(at, "if matches!(src[y], {}) {{", pattern(g));
        for (id, m) in masks.iter_mut().enumerate() {
            m.1 = within(&m.1, g);
            if matches!(m.1, R::True) {
                truth.push(id);
            }
        }
        for e in &mut errors {
            e.2 = within(&e.2, g);
        }
    }
    for id in &back {
        let _ = writeln!(at, "let h{id} = if y - y{id} < 64 {{ w{id} << (y - y{id}) }} else {{ 0 }};");
    }
    for (id, (name, r)) in masks.iter().enumerate() {
        if need[id] && !truth.contains(&id) {
            let _ = writeln!(at, "// mask {name}\nlet m{id} = {};", code(r, &truth));
        }
    }
    for (_, why, r) in &errors {
        let _ = write!(at, "// {why}\nif {} {{\nreturn Err(y as u32);\n}}\n", code(r, &truth));
    }
    for id in &back {
        let _ = writeln!(at, "(w{id}, y{id}) = (h{id} | u64::from({}), y);", code(&R::Anch(*id, 0), &truth));
    }
    if gate.is_some() {
        at.push_str("}\n");
    }
    Ok(PieceChecks { covers: errors.iter().map(|(i, _, _)| *i).collect(), state, at_piece: at })
}
