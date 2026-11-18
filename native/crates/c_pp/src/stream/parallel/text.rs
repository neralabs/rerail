//! P2 to P4: the active text, its invocations, and the output.
//!
//! The active text is stretches: an instance's tokens between two directives, in an active
//! group. A call never crosses a stretch's end in the serial pass but by reading a directive
//! inside its arguments, which the shape sends to the serial pass; so each stretch is read
//! alone.
//!
//! - **P2.** A candidate is an identifier that names a macro at its position (a search in the
//!   positional table). An object-like one is an invocation of its own token; a function-like
//!   one followed by `(` runs to its matching `)` (parentheses matched by depth). The outermost
//!   invocations are those no earlier one's reach covers (a prefix maximum).
//! - **P3.** Each outermost invocation is expanded alone, under the definitions of its
//!   position. One that asks past its end is merged with the tokens after it and expanded
//!   again: a function-like name ending it before a `(` takes that `(`'s call, and a call left
//!   open takes the next token.
//! - **P4.** Each piece's count (a token copied, an expansion's tokens), a prefix sum of the
//!   counts, and each piece's tokens written at its offset. `__COUNTER__` is a prefix sum too:
//!   each invocation's uses, in the unit's order, give the next one's first value.

use super::files::Tab;
use super::rounds::{Layout, State, Tu};
use super::{Isolated, Stats, in_context};
use super::super::{Pp, Src};
use crate::macros::*;
use rerail_c_lean::T;
use rerail_c_lean::pp::P;
use std::collections::HashSet;

const NONE: u32 = u32::MAX;

/// A stretch of active text: its position, instance, and tokens' range in its file's text.
struct Stretch {
    pos: u64,
    inst: u32,
    start: u32,
    end: u32,
}

/// A piece of the output: text copied (a range of a stretch's tokens), or an invocation's
/// expansion with its count of `__COUNTER__`s.
enum Piece {
    Copy(u32, u32),
    Expanded { at: u32, end: u32, toks: Vec<Tk>, counted: u64 },
}

pub(super) fn output(pp: &mut Pp, tu: &Tu, state: &State, lay: &Layout, stats: &mut Stats) -> Result<Vec<Vec<u8>>, &'static str> {
    // The stretches of every instance in an active group, sorted into the unit's order.
    let mut stretches = Vec::new();
    for &i in &lay.order {
        let tab = tu.tab_of(i);
        let n = tab.dirs.len();
        for k in 0..=n {
            let group = if k == 0 { 0 } else { tab.dirs[k - 1].after };
            let start = if k == 0 { 0 } else { tab.dirs[k - 1].text_before };
            let end = if k < n { tab.dirs[k].text_before } else { tab.text.len() as u32 };
            if start < end && state.groups[i as usize][group as usize] {
                stretches.push(Stretch { pos: lay.text_pos(tab, i, k), inst: i, start, end });
            }
        }
    }
    stretches.sort_unstable_by_key(|s| s.pos);
    // P2 and P3, a stretch at a time (each alone).
    let mut pieces: Vec<Vec<Piece>> = Vec::with_capacity(stretches.len());
    let mut distinct: HashSet<Vec<u8>> = HashSet::new();
    for s in &stretches {
        let tab = tu.tab_of(s.inst);
        stats.active_tokens += (s.end - s.start) as u64;
        pieces.push(stretch(pp, tu, tab, s, stats, &mut distinct)?);
    }
    stats.distinct = distinct.len() as u64;
    // `__COUNTER__`: each invocation that counts, read again from the count of those before it.
    let mut base = 0u64;
    for (s, ps) in stretches.iter().zip(pieces.iter_mut()) {
        let tab = tu.tab_of(s.inst);
        for p in ps.iter_mut() {
            if let Piece::Expanded { at, end, toks, counted } = p {
                if *counted == 0 {
                    continue;
                }
                if base > 0 {
                    let chain = &tu.insts[s.inst as usize].chain;
                    let list = tab.text[(s.start + *at) as usize..=(s.start + *end) as usize].to_vec();
                    let r = in_context(pp, chain, s.pos + *at as u64, |pp| isolated(pp, list, base));
                    *toks = r.out;
                }
                base += *counted;
            }
        }
    }
    // P4: counts, their prefix sum, and the tokens written at their offsets.
    let counts: Vec<u64> = pieces
        .iter()
        .flatten()
        .map(|p| match p {
            Piece::Copy(a, b) => (b - a) as u64,
            Piece::Expanded { toks, .. } => toks.len() as u64,
        })
        .collect();
    let mut offsets = Vec::with_capacity(counts.len() + 1);
    let mut acc = 0u64;
    for &c in &counts {
        offsets.push(acc);
        acc += c;
    }
    stats.output = acc;
    let mut out: Vec<Vec<u8>> = vec![Vec::new(); acc as usize];
    let mut k = 0;
    for (s, ps) in stretches.iter().zip(&pieces) {
        let tab = tu.tab_of(s.inst);
        for p in ps {
            let at = offsets[k] as usize;
            match p {
                Piece::Copy(a, b) => {
                    for (j, t) in tab.text[(s.start + a) as usize..(s.start + b) as usize].iter().enumerate() {
                        out[at + j] = pp.text(t).to_vec();
                    }
                }
                Piece::Expanded { toks, .. } => {
                    for (j, t) in toks.iter().enumerate() {
                        out[at + j] = pp.text(t).to_vec();
                    }
                }
            }
            k += 1;
        }
    }
    Ok(out)
}

/// A stretch's pieces: its candidates, its outermost invocations, each expanded.
fn stretch(pp: &mut Pp, tu: &Tu, tab: &Tab, s: &Stretch, stats: &mut Stats, distinct: &mut HashSet<Vec<u8>>) -> Result<Vec<Piece>, &'static str> {
    let toks = &tab.text[s.start as usize..s.end as usize];
    let n = toks.len();
    let chain = &tu.insts[s.inst as usize].chain;
    // Each `(`'s matching `)` in the stretch, by depth.
    let mut close = vec![NONE; n];
    let mut open = Vec::new();
    for (j, t) in toks.iter().enumerate() {
        if t.is(T::LParen) {
            open.push(j);
        } else if t.is(T::RParen) {
            if let Some(o) = open.pop() {
                close[o] = j as u32;
            }
        }
    }
    // The candidates, and each invocation's last token.
    let vis = pp.vis.as_ref().expect("the positional table");
    let mut reach = vec![NONE; n];
    for (j, t) in toks.iter().enumerate() {
        if t.kind != P::Ident {
            continue;
        }
        let Some(d) = vis.at(t.sym, s.pos + j as u64) else { continue };
        stats.candidates += 1;
        if !pp.defs[d as usize].fnlike {
            reach[j] = j as u32;
        } else if toks.get(j + 1).is_some_and(|t| t.is(T::LParen)) {
            if close[j + 1] == NONE {
                return Err("a call past its stretch of text");
            }
            reach[j] = close[j + 1];
        }
    }
    // The outermost: those past every earlier one's reach.
    let mut outer = Vec::new();
    let mut far: i64 = -1;
    for j in 0..n {
        if reach[j] != NONE {
            if j as i64 > far {
                outer.push((j as u32, reach[j]));
            }
            far = far.max(reach[j] as i64);
        }
    }
    // Each expanded alone, and merged with what follows while it asks for it.
    let mut pieces = Vec::new();
    let mut copied = 0u32;
    let mut g = 0;
    while g < outer.len() {
        let (at, mut end) = outer[g];
        if copied < at {
            pieces.push(Piece::Copy(copied, at));
        }
        let r = loop {
            let list = toks[at as usize..=end as usize].to_vec();
            let r = in_context(pp, chain, s.pos + at as u64, |pp| isolated(pp, list, 0));
            if let Some(why) = r.unsupported {
                return Err(why);
            }
            let next = end as usize + 1;
            if r.cut {
                // A call left open: the next token, and every invocation it starts.
                if next >= n {
                    return Err("a call past its stretch of text");
                }
                end = next as u32;
                while let Some(&(_, e)) = outer.get(g + 1).filter(|o| o.0 <= end) {
                    end = end.max(e);
                    g += 1;
                }
                stats.merges += 1;
                continue;
            }
            if r.peeked && toks.get(next).is_some_and(|t| t.is(T::LParen)) {
                // A function-like name at its end: the call after it.
                end = close[next];
                if end == NONE {
                    return Err("a call past its stretch of text");
                }
                stats.merges += 1;
                continue;
            }
            break r;
        };
        stats.invocations += 1;
        stats.expanded += r.out.len() as u64;
        stats.longest_in = stats.longest_in.max((end - at + 1) as u64);
        stats.longest_out = stats.longest_out.max(r.out.len() as u64);
        let mut key = Vec::new();
        for t in &toks[at as usize..=end as usize] {
            key.extend_from_slice(pp.text(t));
            key.push(b' ');
        }
        distinct.insert(key);
        pieces.push(Piece::Expanded { at, end, toks: r.out, counted: r.counted });
        copied = end + 1;
        while outer.get(g).is_some_and(|o| o.0 <= end) {
            g += 1;
        }
    }
    if (copied as usize) < n {
        pieces.push(Piece::Copy(copied, n as u32));
    }
    Ok(pieces)
}

/// An isolated reading's result.
struct Read {
    out: Vec<Tk>,
    peeked: bool,
    cut: bool,
    unsupported: Option<&'static str>,
    counted: u64,
}

/// `list` read alone and expanded, `__COUNTER__` from `counter`.
fn isolated(pp: &mut Pp, list: Vec<Tk>, counter: u64) -> Read {
    let outer = pp.stack.len();
    pp.stack.push(Src::Toks { toks: list, pos: 0, def: None, stop: true });
    pp.par = Some(Isolated { outer, peeked: false, cut: false, unsupported: None });
    pp.counter = counter;
    let mut out = Vec::new();
    loop {
        let t = pp.next();
        if t.kind == P::Eof {
            break;
        }
        out.push(t);
    }
    let par = pp.par.take().expect("the reading's state");
    Read { out, peeked: par.peeked, cut: par.cut, unsupported: par.unsupported, counted: pp.counter - counter }
}
