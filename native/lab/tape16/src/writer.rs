//! E6: what format 16's writer costs, against format 15's, on the lean `.rr` parser's path.
//!
//! The lean parser pushes and closes nodes into `tape_format::Writer`, and the writer finishes the image.
//! A format-16 writer would take the same pushes and closes, and finish events instead. So per file:
//!
//! - **A**: the lean parse and its format-15 write (`lean::tape`), the engine as it is;
//! - **B**: format 15's writer alone from the node list (`preorder_image`: its pushes, closes and finish);
//! - **C**: format 16's writer alone from the same node list: events, then byte planes.
//!
//! Format 16's writer is then A − B + C. B and C need no parser, so they run on any captured tapes.

use crate::blocks::Block;
use crate::events::{BACK, Events, Open};
use crate::layout::{self, Extras, Layout};

/// A captured image's nodes as `preorder_image` takes them: key, start, end, subtree size.
pub fn records(image: &[u8]) -> Result<Vec<(u32, u32, u32, u32)>, String> {
    let d = rerail_tape_format::decode(image)?;
    let mut size = vec![1u32; d.len()];
    for i in (0..d.len()).rev() {
        if let Some(p) = d[i].parent {
            size[p as usize] += size[i];
        }
    }
    Ok(d.iter().zip(&size).map(|(n, &s)| (n.key, n.start, n.end, s)).collect())
}

fn delta(to: u64, from: u64) -> u32 {
    if to >= from { (to - from) as u32 } else { (from - to) as u32 | BACK }
}

/// Format 16's writer from the node list, in one pass straight into byte planes: every node has one open
/// and one close, so each plane's offset is known before the first node (`layout::lay`'s byte planes, as
/// `tests` checks).
pub fn events_writer(src_len: u64, nodes: &[(u32, u32, u32, u32)]) -> Vec<u8> {
    let n = nodes.len();
    // Planes: kind 2, field 1, flags 1, membership 2, lead 4, pops 4 bytes of each open; 4 of each close.
    let mut out = vec![0u8; 16 + n * 14 + n * 4];
    let (kind, field, flags, memb, lead, pops, trail) = (16, 16 + 2 * n, 16 + 3 * n, 16 + 4 * n, 16 + 6 * n, 16 + 10 * n, 16 + 14 * n);
    let mut last = 0u64;
    let mut open: Vec<u32> = Vec::with_capacity(64);
    let mut t = 0usize;
    let mut close = |out: &mut Vec<u8>, last: &mut u64, e: u32| {
        let d = delta(u64::from(e), *last);
        *last = u64::from(e);
        for b in 0..4 {
            out[trail + b * n + t] = (d >> (8 * b)) as u8;
        }
        t += 1;
    };
    for (i, &(key, start, _, _)) in nodes.iter().enumerate() {
        let mut p = 0u32;
        while let Some(&j) = open.last() {
            let j = j as usize;
            if j + nodes[j].3 as usize > i {
                break;
            }
            open.pop();
            close(&mut out, &mut last, nodes[j].2);
            p += 1;
        }
        let d = delta(u64::from(start), last);
        last = u64::from(start);
        let k = (key & 1023) as u16;
        out[kind + i] = k as u8;
        out[kind + n + i] = (k >> 8) as u8;
        out[field + i] = ((key >> 10) & 63) as u8;
        out[flags + i] = (key >> 16) as u8;
        let _ = memb;
        for b in 0..4 {
            out[lead + b * n + i] = (d >> (8 * b)) as u8;
            out[pops + b * n + i] = (p >> (8 * b)) as u8;
        }
        open.push(i as u32);
    }
    let mut end_pops = 0u32;
    while let Some(j) = open.pop() {
        close(&mut out, &mut last, nodes[j as usize].2);
        end_pops += 1;
    }
    let final_gap = delta(src_len, last);
    for (k, v) in [n as u32, n as u32, end_pops, final_gap].into_iter().enumerate() {
        out[4 * k..4 * k + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

/// The same bytes by the lab's general path (events, then `layout::lay`): what `events_writer` must equal.
pub fn events_writer_reference(src_len: u64, nodes: &[(u32, u32, u32, u32)]) -> Vec<u8> {
    let mut ev = Events { opens: Vec::with_capacity(nodes.len()), trails: Vec::with_capacity(nodes.len()), ..Default::default() };
    let mut last = 0u64;
    let mut open: Vec<(usize, u32)> = Vec::new();
    for (i, &(key, start, _end, _)) in nodes.iter().enumerate() {
        let mut pops = 0;
        while let Some(&(j, e)) = open.last() {
            if j + nodes[j].3 as usize > i {
                break;
            }
            open.pop();
            ev.trails.push(delta(u64::from(e), last));
            last = u64::from(e);
            pops += 1;
        }
        ev.opens.push(Open { kind: (key & 1023) as u16, field: ((key >> 10) & 63) as u8, flags: (key >> 16) as u8, memb: 0, lead: delta(u64::from(start), last), pops });
        last = u64::from(start);
        open.push((i, nodes[i].2));
    }
    while let Some((_, e)) = open.pop() {
        ev.trails.push(delta(u64::from(e), last));
        last = u64::from(e);
        ev.end_pops += 1;
    }
    ev.final_gap = delta(src_len, last);
    let b = Block { opens: ev.opens, trails: ev.trails, end_pops: ev.end_pops, final_gap: ev.final_gap };
    layout::lay(&b, Layout::BytePlanes, Extras::default(), &[], &[])
}
