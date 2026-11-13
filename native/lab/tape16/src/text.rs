//! Text as pieces (0156 §6, T6): each event's delta covers one piece of the source, a leaf's text or a gap
//! (whitespace and the tokens the grammar implies). A text stream holds a piece id per event.

use crate::events::{BACK, Events};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Piece {
    Gap,
    Leaf,
}

/// The pieces of `source` in event order, with what each is. A backward event makes no piece; a forward one
/// after it covers only bytes not yet covered, so the pieces always concatenate to the source.
pub fn pieces<'a>(ev: &Events, source: &'a [u8]) -> Vec<(Piece, &'a [u8])> {
    let mut out = Vec::with_capacity(ev.opens.len() * 2 + 1);
    let (mut pos, mut covered) = (0u64, 0u64);
    let mut t = 0usize;
    let mut leaf_open = false;
    let mut step = |d: u32, what: Piece, out: &mut Vec<(Piece, &'a [u8])>| {
        pos = if d & BACK != 0 { pos - u64::from(d & !BACK) } else { pos + u64::from(d) };
        if pos > covered {
            out.push((what, &source[covered as usize..pos as usize]));
            covered = pos;
        } else if d & BACK == 0 {
            out.push((what, &source[0..0]));
        }
    };
    for (i, o) in ev.opens.iter().enumerate() {
        for _ in 0..o.pops {
            let what = if leaf_open { Piece::Leaf } else { Piece::Gap };
            step(ev.trails[t], what, &mut out);
            t += 1;
            leaf_open = false;
        }
        step(o.lead, Piece::Gap, &mut out);
        // A node is a leaf when the next event is its own close.
        leaf_open = ev.opens.get(i + 1).is_none_or(|n| n.pops > 0) || i + 1 == ev.opens.len();
    }
    for _ in 0..ev.end_pops {
        let what = if leaf_open { Piece::Leaf } else { Piece::Gap };
        step(ev.trails[t], what, &mut out);
        t += 1;
        leaf_open = false;
    }
    step(ev.final_gap, Piece::Gap, &mut out);
    out
}

/// A language's shared piece table: the most frequent pieces of the training half (the text's dictionary).
#[derive(Default)]
pub struct Table {
    pub ids: HashMap<Vec<u8>, u32>,
    pub bytes: usize,
}

impl Table {
    pub fn train(counts: &HashMap<Vec<u8>, u64>, entries: usize) -> Self {
        let mut v: Vec<_> = counts.iter().filter(|(_, c)| **c >= 2).collect();
        v.sort_by(|a, b| (b.1 * b.0.len().max(1) as u64).cmp(&(a.1 * a.0.len().max(1) as u64)).then(a.0.cmp(b.0)));
        let mut t = Table::default();
        for (p, _) in v.into_iter().take(entries) {
            t.bytes += p.len() + 1;
            let id = t.ids.len() as u32;
            t.ids.insert(p.clone(), id);
        }
        t
    }
}

/// One file's text stream at a level: the id stream and the file's own pool of pieces not in the table.
pub struct Stream {
    pub ids: Vec<u32>,
    pub pool: Vec<u8>,
    pub gaps: usize,
    pub gaps_in_table: usize,
    pub leaves: usize,
    pub leaves_in_table: usize,
}

/// `leaves_only`: the names level (leaf pieces); else the source level (every piece).
pub fn stream(pieces: &[(Piece, &[u8])], table: &Table, leaves_only: bool) -> Stream {
    let mut local: HashMap<&[u8], u32> = HashMap::new();
    let mut s = Stream { ids: Vec::new(), pool: Vec::new(), gaps: 0, gaps_in_table: 0, leaves: 0, leaves_in_table: 0 };
    let base = table.ids.len() as u32;
    for &(what, text) in pieces {
        if leaves_only && what == Piece::Gap {
            continue;
        }
        let hit = table.ids.get(text).copied();
        match what {
            Piece::Gap => {
                s.gaps += 1;
                s.gaps_in_table += hit.is_some() as usize;
            }
            Piece::Leaf => {
                s.leaves += 1;
                s.leaves_in_table += hit.is_some() as usize;
            }
        }
        let id = match hit {
            Some(id) => id,
            None => {
                let next = base + local.len() as u32;
                *local.entry(text).or_insert_with(|| {
                    let mut n = text.len();
                    while n >= 0x80 {
                        s.pool.push(n as u8 | 0x80);
                        n >>= 7;
                    }
                    s.pool.push(n as u8);
                    s.pool.extend_from_slice(text);
                    next
                })
            }
        };
        s.ids.push(id);
    }
    s
}

/// The source again from the source-level stream, through the table and the pool (exactness check).
pub fn rebuild(s: &Stream, table: &Table) -> Vec<u8> {
    let mut by_id: Vec<&[u8]> = vec![&[]; table.ids.len()];
    for (p, &id) in &table.ids {
        by_id[id as usize] = p;
    }
    let mut at = 0;
    while at < s.pool.len() {
        let (mut n, mut sh) = (0usize, 0);
        loop {
            let c = s.pool[at];
            at += 1;
            n |= usize::from(c & 0x7f) << sh;
            if c < 0x80 {
                break;
            }
            sh += 7;
        }
        by_id.push(&s.pool[at..at + n]);
        at += n;
    }
    let mut out = Vec::new();
    for &id in &s.ids {
        out.extend_from_slice(by_id[id as usize]);
    }
    out
}

/// The id stream as bytes for a codec: u32 byte planes (format 16's fixed widths).
pub fn id_planes(ids: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(ids.len() * 4);
    for b in 0..4 {
        out.extend(ids.iter().map(|v| (v >> (8 * b)) as u8));
    }
    out
}
