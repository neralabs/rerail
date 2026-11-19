//! Byte-at-a-time oracles for the mask versions of Python's strings and f-strings, ported from
//! docs/parser-generators/prototypes (nofsa.rs, fstr.rs).
//!
//! - `machine` is the 13-state `strings` automaton the old lexer used. Its classes are what the
//!   mask version (`reflect`, `hypotheses`, `rounds`) must reproduce on files without Python 3.12
//!   quote reuse.
//! - `nofsa` is the whole-file speculate-and-verify version of the same.
//! - `fstring` finds the interpolations, format expressions and format specs of one f-string body,
//!   by rounds over a context, the way 08-without-automata.md §5 describes. It handles format specs
//!   at every depth.

use std::collections::HashSet;

pub fn escaped(src: &[u8]) -> Vec<bool> {
    let mut e = vec![false; src.len() + 1];
    let mut run = 0usize;
    for (i, &c) in src.iter().enumerate() {
        if c == b'\\' {
            run += 1
        } else {
            if run % 2 == 1 {
                e[i] = true
            }
            run = 0
        }
    }
    e
}
fn is_q(src: &[u8], esc: &[bool], i: usize) -> bool {
    i < src.len() && (src[i] == b'\'' || src[i] == b'"') && !esc[i]
}
fn is_nl(src: &[u8], esc: &[bool], i: usize) -> bool {
    src[i] == b'\n' && !esc[i] && !(i > 0 && src[i - 1] == b'\r' && esc[i - 1])
}

/// Classes per byte: 0 code, 1 string, 2 comment. None when a string is left open.
pub fn machine(src: &[u8]) -> Option<Vec<u8>> {
    let esc = escaped(src);
    // states: 0 code 1 comment 2 s1 3 d1 4 s3 5 d3 6 os2 7 os1 8 od2 9 od1 10 cs2 11 cs1
    let mut cls = vec![0u8; src.len()];
    let mut st = 0u8;
    for i in 0..src.len() {
        let c = src[i];
        let ev: Option<u8> = if is_q(src, &esc, i) {
            let tri = is_q(src, &esc, i + 1) && is_q(src, &esc, i + 2) && src[i + 1] == c && src[i + 2] == c;
            Some(match (c, tri) {
                (b'\'', true) => 2,
                (b'"', true) => 3,
                (b'\'', false) => 0,
                _ => 1,
            })
        } else if c == b'#' {
            Some(4)
        } else if is_nl(src, &esc, i) {
            Some(5)
        } else {
            None
        };
        if let Some(e) = ev {
            st = match (st, e) {
                (0, 4) => 1,
                (0, 0) => 2,
                (0, 1) => 3,
                (0, 2) => 6,
                (0, 3) => 8,
                (1, 5) => 0,
                (2, 0) | (2, 2) => 0,
                (3, 1) | (3, 3) => 0,
                (2, 5) | (3, 5) => return None,
                (6, _) => 7,
                (7, _) => 4,
                (8, _) => 9,
                (9, _) => 5,
                (4, 2) => 10,
                (5, 3) => 10,
                (10, _) => 11,
                (11, _) => 0,
                (s, _) => s,
            };
        }
        cls[i] = match st {
            1 => 2,
            2..=9 => 1,
            _ => 0,
        };
    }
    if st != 0 && st != 1 {
        return None;
    }
    Some(cls)
}

/// Whole-file speculate and verify; classes as `machine`, and the passes it took.
pub fn nofsa(src: &[u8], kmax: usize) -> Option<(Vec<u8>, usize)> {
    let esc = escaped(src);
    let esc = &esc[..];
    let mut tri: Vec<(usize, u8)> = vec![];
    let mut i = 0;
    while i < src.len() {
        if is_q(src, esc, i) {
            let c = src[i];
            let mut j = i;
            while j < src.len() && src[j] == c && is_q(src, esc, j) {
                j += 1
            }
            let mut k = i;
            while k + 3 <= j {
                tri.push((k, if c == b'\'' { 1 } else { 2 }));
                k += 3
            }
            i = j
        } else {
            i += 1
        }
    }
    let cand: HashSet<usize> = tri.iter().map(|t| t.0).collect();
    let mut hidden: HashSet<usize> = HashSet::new();
    for iter in 0..kmax {
        let mut x = 0i32;
        let mut tstr = vec![false; src.len()];
        let mut tbyte = vec![false; src.len()];
        let mut openers: HashSet<usize> = HashSet::new();
        let mut open_from: Option<usize> = None;
        for &(p, b) in &tri {
            if hidden.contains(&p) {
                continue;
            }
            let before = x;
            x = (b as i32 - x).rem_euclid(3);
            tbyte[p] = true;
            tbyte[p + 1] = true;
            tbyte[p + 2] = true;
            if before == 0 && x != 0 {
                open_from = Some(p);
                openers.insert(p);
            }
            if before != 0 && x == 0 {
                for q in open_from.take().unwrap()..p {
                    tstr[q] = true
                }
            }
        }
        let t_open = open_from.is_some();
        if let Some(f) = open_from {
            for q in f..src.len() {
                tstr[q] = true
            }
        }
        let mut cls = vec![0u8; src.len()];
        let (mut s, mut cmt) = (0i32, false);
        let mut bad = false;
        let mut next_hidden: HashSet<usize> = HashSet::new();
        for i in 0..src.len() {
            if cand.contains(&i) && (openers.contains(&i) || !tstr[i]) && (s != 0 || cmt) {
                next_hidden.insert(i);
            }
            if tstr[i] {
                cls[i] = 1;
                s = 0;
                cmt = false;
                continue;
            }
            let c = src[i];
            if cmt {
                if is_nl(src, esc, i) {
                    cmt = false;
                    s = 0;
                    cls[i] = 0
                } else {
                    cls[i] = 2
                }
                continue;
            }
            if is_q(src, esc, i) && !tbyte[i] {
                let b = if c == b'\'' { 1 } else { 2 };
                s = (b - s).rem_euclid(3)
            } else if c == b'#' && s == 0 {
                cmt = true;
                cls[i] = 2;
                continue;
            } else if is_nl(src, esc, i) {
                if s != 0 {
                    bad = true
                }
                s = 0
            }
            cls[i] = if s != 0 { 1 } else { 0 };
        }
        if next_hidden != hidden {
            hidden = next_hidden;
            continue;
        }
        if t_open || bad || (s != 0 && !cmt) {
            return None;
        }
        return Some((cls, iter + 1));
    }
    None
}

/// Spans of one f-string's fields, with absolute offsets.
#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub struct Fields {
    pub interp: Vec<(usize, usize)>,
    pub fexpr: Vec<(usize, usize)>,
    pub spec: Vec<(usize, usize)>,
}

#[derive(Debug)]
pub enum FOut {
    Ok(Fields, usize),
    Unsupported(&'static str),
    Unsettled,
}

#[derive(Clone, PartialEq)]
struct Ctx {
    d: Vec<i32>,  // depth before each byte
    fd: Vec<i32>, // depth of the innermost replacement field
    sp: Vec<bool>, // that field is in its format spec
}

/// The body of one f-string is `src[a..b]`. Rounds over the context before each byte; the first
/// round guesses without depth (08-without-automata.md §5.1).
pub fn fstring(src: &[u8], esc: &[bool], a: usize, b: usize, triple_outer: bool, raw: bool, kmax: usize) -> FOut {
    let n = b - a;
    let at = |i: usize| src[a + i];
    let isq = |i: usize| (at(i) == b'\'' || at(i) == b'"') && !esc[a + i];
    let mut ctx = Ctx { d: vec![0; n + 1], fd: vec![0; n + 1], sp: vec![false; n + 1] };
    for round in 0..kmax {
        let naive = round == 0;
        let mut delta = vec![0i32; n];
        let mut fopen = vec![false; n];
        let mut fclose = vec![false; n];
        let mut colon = vec![false; n];
        let mut err: Option<&'static str> = None;
        let (mut cur_inner, mut cur_cmt) = (0u8, false);
        let mut i = 0;
        while i < n {
            let c = at(i);
            if ctx.d[i] == 0 {
                cur_inner = 0;
                if !triple_outer {
                    cur_cmt = false
                }
            }
            let (d, inn, cm) = (ctx.d[i], cur_inner, cur_cmt);
            let stext = d >= 1 && d == ctx.fd[i] && ctx.sp[i];
            if cm && c == b'\n' {
                cur_cmt = false
            }
            if c == b'{' && d == 0 && !raw && i >= 1 && at(i - 1) == b'N' && esc[a + i - 1] {
                let mut j = i;
                while j < n && at(j) != b'}' {
                    j += 1
                }
                i = j + 1;
                continue;
            }
            if (c == b'{' || c == b'}') && !cm && inn == 0 {
                let mut j = i;
                while j < n && at(j) == c {
                    j += 1
                }
                let len = j - i;
                if naive {
                    if len % 2 == 1 {
                        if c == b'{' {
                            delta[j - 1] = 1;
                            fopen[j - 1] = true
                        } else {
                            delta[j - 1] = -1;
                            fclose[j - 1] = true
                        }
                    }
                } else if c == b'{' {
                    if d == 0 {
                        if len % 2 == 1 {
                            delta[j - 1] = 1;
                            fopen[j - 1] = true
                        }
                    } else {
                        for k in i..j {
                            delta[k] = 1
                        }
                        if stext {
                            fopen[i] = true
                        }
                    }
                } else {
                    let closes = (d as usize).min(len);
                    for k in i..i + closes {
                        delta[k] = -1;
                        if ctx.d[k] == ctx.fd[k] {
                            fclose[k] = true
                        }
                    }
                    if (len - closes) % 2 == 1 {
                        err.get_or_insert("a lone '}' in f-string text");
                    }
                }
                i = j;
                continue;
            }
            if cm {
                i += 1;
                continue;
            }
            if inn != 0 {
                if isq(i) && (if at(i) == b'\'' { 1 } else { 2 }) == inn {
                    cur_inner = 0
                }
                i += 1;
                continue;
            }
            if d >= 1 && !stext {
                match c {
                    b'(' | b'[' => delta[i] = 1,
                    b')' | b']' => delta[i] = -1,
                    b'\'' | b'"' if !esc[a + i] => {
                        if i + 2 < n && at(i + 1) == c && at(i + 2) == c {
                            err.get_or_insert("a triple-quoted string inside an interpolation");
                        }
                        cur_inner = if c == b'\'' { 1 } else { 2 }
                    }
                    b'#' => cur_cmt = true,
                    b':' if d == ctx.fd[i] && !ctx.sp[i] => colon[i] = true,
                    b'\n' if !triple_outer => {
                        err.get_or_insert("a newline in a one-line f-string's interpolation");
                    }
                    _ => {}
                }
            }
            i += 1;
        }
        let mut nc = Ctx { d: vec![0; n + 1], fd: vec![0; n + 1], sp: vec![false; n + 1] };
        let (mut d, mut fd, mut sp) = (0i32, 0i32, false);
        for i in 0..n {
            nc.d[i] = d;
            nc.fd[i] = fd;
            nc.sp[i] = sp;
            if delta[i] < 0 && d == 0 {
            } else {
                d += delta[i]
            }
            if fopen[i] {
                fd += 1;
                sp = false
            }
            if fclose[i] && fd > 0 {
                fd -= 1;
                sp = fd >= 1
            }
            if colon[i] {
                sp = true
            }
        }
        nc.d[n] = d;
        nc.fd[n] = fd;
        nc.sp[n] = sp;
        if nc == ctx {
            if let Some(e) = err {
                return FOut::Unsupported(e);
            }
            if d != 0 || cur_inner != 0 {
                return FOut::Unsupported("unbalanced");
            }
            let mut f = Fields::default();
            let mut stack: Vec<(usize, bool, Option<usize>)> = vec![]; // (open, is interpolation, colon)
            for i in 0..n {
                if fopen[i] {
                    stack.push((i, ctx.d[i] == 0, None))
                }
                if colon[i] {
                    if let Some(t) = stack.last_mut() {
                        t.2 = Some(i)
                    }
                }
                if fclose[i] {
                    if let Some((s, top, c)) = stack.pop() {
                        if top { f.interp.push((a + s, a + i + 1)) } else { f.fexpr.push((a + s, a + i + 1)) }
                        if let Some(c) = c {
                            f.spec.push((a + c, a + i))
                        }
                    }
                }
            }
            f.interp.sort();
            f.fexpr.sort();
            f.spec.sort();
            return FOut::Ok(f, round + 1);
        }
        ctx = nc;
    }
    FOut::Unsettled
}
