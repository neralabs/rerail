//! Content-defined blocks (0156 §3, T3): cut after an open where a rolling hash over the open records meets
//! a condition, between a minimum and a maximum count. A block's body is its opens and the closes that
//! fall in it; its placement says where it sits.

use crate::events::{Events, Open};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cut {
    /// One block per file.
    Whole,
    /// Content-defined, aiming at this many nodes (min a quarter, max four times).
    Target(usize),
}

impl Cut {
    pub fn name(self) -> String {
        match self {
            Cut::Whole => "whole".into(),
            Cut::Target(t) => format!("cdc{t}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub opens: Vec<Open>,
    pub trails: Vec<u32>,
    /// The last block's closes after its last open, and the gap to the source's end.
    pub end_pops: u32,
    pub final_gap: u32,
}

/// Where a block sits: what is not in its body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub first_node: u64,
    /// The position of the event before the block's first.
    pub base: u64,
    pub entry_depth: u32,
    pub min_depth: u32,
}

/// Bytes a placement entry takes, with the body's 32-byte content id.
pub const PLACEMENT_BYTES: usize = 32 + 8 + 8 + 4 + 4;

fn gear() -> &'static [u64; 256] {
    static G: std::sync::OnceLock<[u64; 256]> = std::sync::OnceLock::new();
    G.get_or_init(|| {
        let mut s = 0x9e3779b97f4a7c15u64;
        let mut g = [0u64; 256];
        for v in &mut g {
            s = s.wrapping_add(0x9e3779b97f4a7c15);
            let mut z = s;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
            *v = z ^ (z >> 31);
        }
        g
    })
}

/// The open indexes each block starts at.
pub fn cuts(ev: &Events, cut: Cut) -> Vec<usize> {
    let n = ev.opens.len();
    let Cut::Target(target) = cut else { return vec![0] };
    let (min, max) = (target / 4, target * 4);
    let mask = (target - min).next_power_of_two() as u64 - 1;
    let g = gear();
    let mut starts = vec![0];
    let mut h = 0u64;
    let mut count = 0usize;
    for (i, o) in ev.opens.iter().enumerate() {
        // The record's content, not its place: a repeat cuts the same.
        for b in [o.kind as u8, (o.kind >> 8) as u8, o.field, o.lead as u8, o.pops as u8] {
            h = (h << 1).wrapping_add(g[b as usize]);
        }
        count += 1;
        if i + 1 < n && ((count >= min && h & mask == 0) || count >= max) {
            starts.push(i + 1);
            count = 0;
            h = 0;
        }
    }
    starts
}

pub fn split(ev: &Events, cut: Cut) -> (Vec<Block>, Vec<Placement>) {
    let starts = cuts(ev, cut);
    let n = ev.opens.len();
    let mut blocks = Vec::with_capacity(starts.len());
    let mut places = Vec::with_capacity(starts.len());
    let (mut t, mut depth, mut pos, mut node) = (0usize, 0u64, 0u64, 0u64);
    for (k, &a) in starts.iter().enumerate() {
        let b = starts.get(k + 1).copied().unwrap_or(n);
        let last = k + 1 == starts.len();
        let closes: usize = ev.opens[a..b].iter().map(|o| o.pops as usize).sum::<usize>() + if last { ev.end_pops as usize } else { 0 };
        let entry_depth = depth;
        let mut min_depth = depth;
        let base = pos;
        // Walk the block to carry depth and position to the next one.
        let mut tt = t;
        for o in &ev.opens[a..b] {
            for _ in 0..o.pops {
                pos = step(pos, ev.trails[tt]);
                tt += 1;
                depth -= 1;
                min_depth = min_depth.min(depth);
            }
            pos = step(pos, o.lead);
            depth += 1;
        }
        blocks.push(Block {
            opens: ev.opens[a..b].to_vec(),
            trails: ev.trails[t..t + closes].to_vec(),
            end_pops: if last { ev.end_pops } else { 0 },
            final_gap: if last { ev.final_gap } else { 0 },
        });
        places.push(Placement { first_node: node, base, entry_depth: entry_depth as u32, min_depth: min_depth as u32 });
        t += closes;
        node += (b - a) as u64;
    }
    (blocks, places)
}

fn step(pos: u64, d: u32) -> u64 {
    if d & crate::events::BACK != 0 { pos - u64::from(d & !crate::events::BACK) } else { pos + u64::from(d) }
}

/// The blocks joined back into one stream.
pub fn join(blocks: &[Block]) -> Events {
    let mut ev = Events::default();
    for b in blocks {
        ev.opens.extend_from_slice(&b.opens);
        ev.trails.extend_from_slice(&b.trails);
        ev.end_pops += b.end_pops;
        ev.final_gap += b.final_gap;
    }
    ev
}
