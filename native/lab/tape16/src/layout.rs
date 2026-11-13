//! What the codec sees (T2): a block's records as byte planes, field planes, whole records, or varints
//! (the last a reference for the entropy floor; it is not fixed-width).

use crate::blocks::Block;
use crate::events::Open;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Layout {
    BytePlanes,
    FieldPlanes,
    Records,
    Varint,
}

impl Layout {
    pub fn name(self) -> &'static str {
        match self {
            Layout::BytePlanes => "bytes",
            Layout::FieldPlanes => "fields",
            Layout::Records => "records",
            Layout::Varint => "varint",
        }
    }
    pub const ALL: [Layout; 4] = [Layout::BytePlanes, Layout::FieldPlanes, Layout::Records, Layout::Varint];
}

/// Fields stored beyond format 16's own (T5): a parent delta, a subtree size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub struct Extras {
    pub parent: bool,
    pub size: bool,
}

/// An open record's fields as (width, value) in order, with any extras (`up`: i − parent, `size`).
fn fields(o: &Open, up: u32, size: u32, x: Extras) -> ([(u8, u32); 8], usize) {
    let mut f = [(0u8, 0u32); 8];
    let mut k = 0;
    for v in [(2, o.kind as u32), (1, o.field as u32), (1, o.flags as u32), (2, o.memb as u32), (4, o.lead), (4, o.pops)] {
        f[k] = v;
        k += 1;
    }
    if x.parent {
        f[k] = (4, up);
        k += 1;
    }
    if x.size {
        f[k] = (4, size);
        k += 1;
    }
    (f, k)
}

fn varint(out: &mut Vec<u8>, mut v: u32) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn read_varint(b: &[u8], at: &mut usize) -> u32 {
    let mut v = 0u32;
    let mut s = 0;
    loop {
        let c = b[*at];
        *at += 1;
        v |= u32::from(c & 0x7f) << s;
        if c < 0x80 {
            return v;
        }
        s += 7;
    }
}

fn widths(x: Extras) -> Vec<usize> {
    let mut w = vec![2, 1, 1, 2, 4, 4];
    if x.parent {
        w.push(4);
    }
    if x.size {
        w.push(4);
    }
    w
}

/// The block's bytes. `up` and `size` are per open (needed only with extras).
pub fn lay(b: &Block, layout: Layout, x: Extras, up: &[u32], size: &[u32]) -> Vec<u8> {
    let (n, m) = (b.opens.len(), b.trails.len());
    let mut out = Vec::with_capacity(16 + n * 22 + m * 4);
    for v in [n as u32, m as u32, b.end_pops, b.final_gap] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    let rec = |i: usize| fields(&b.opens[i], if x.parent { up[i] } else { 0 }, if x.size { size[i] } else { 0 }, x);
    match layout {
        Layout::Records => {
            for i in 0..n {
                let (f, k) = rec(i);
                for &(w, v) in &f[..k] {
                    out.extend_from_slice(&v.to_le_bytes()[..w as usize]);
                }
            }
            for &t in &b.trails {
                out.extend_from_slice(&t.to_le_bytes());
            }
        }
        Layout::FieldPlanes | Layout::BytePlanes => {
            let widths = widths(x);
            for (f, &w) in widths.iter().enumerate() {
                if layout == Layout::FieldPlanes {
                    for i in 0..n {
                        out.extend_from_slice(&rec(i).0[f].1.to_le_bytes()[..w]);
                    }
                } else {
                    for byte in 0..w {
                        for i in 0..n {
                            out.push((rec(i).0[f].1 >> (8 * byte)) as u8);
                        }
                    }
                }
            }
            if layout == Layout::FieldPlanes {
                for &t in &b.trails {
                    out.extend_from_slice(&t.to_le_bytes());
                }
            } else {
                for byte in 0..4 {
                    out.extend(b.trails.iter().map(|t| (t >> (8 * byte)) as u8));
                }
            }
        }
        Layout::Varint => {
            for f in 0..widths(x).len() {
                for i in 0..n {
                    varint(&mut out, rec(i).0[f].1);
                }
            }
            for &t in &b.trails {
                varint(&mut out, t);
            }
        }
    }
    out
}

/// The block back from its bytes (extras are read and dropped: the decoder derives them, or a reader uses them).
pub fn unlay(bytes: &[u8], layout: Layout, x: Extras) -> Block {
    let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let (n, m) = (u32_at(0) as usize, u32_at(4) as usize);
    let (end_pops, final_gap) = (u32_at(8), u32_at(12));
    let widths = widths(x);
    let k = widths.len();
    let mut vals = vec![vec![0u32; n]; k];
    let mut trails = vec![0u32; m];
    let mut at = 16;
    match layout {
        Layout::Records => {
            for i in 0..n {
                for f in 0..k {
                    let w = widths[f];
                    let mut v = [0u8; 4];
                    v[..w].copy_from_slice(&bytes[at..at + w]);
                    vals[f][i] = u32::from_le_bytes(v);
                    at += w;
                }
            }
            for t in &mut trails {
                *t = u32_at(at);
                at += 4;
            }
        }
        Layout::FieldPlanes => {
            for f in 0..k {
                let w = widths[f];
                for i in 0..n {
                    let mut v = [0u8; 4];
                    v[..w].copy_from_slice(&bytes[at..at + w]);
                    vals[f][i] = u32::from_le_bytes(v);
                    at += w;
                }
            }
            for t in &mut trails {
                *t = u32_at(at);
                at += 4;
            }
        }
        Layout::BytePlanes => {
            for f in 0..k {
                for byte in 0..widths[f] {
                    for i in 0..n {
                        vals[f][i] |= u32::from(bytes[at]) << (8 * byte);
                        at += 1;
                    }
                }
            }
            for byte in 0..4 {
                for t in trails.iter_mut() {
                    *t |= u32::from(bytes[at]) << (8 * byte);
                    at += 1;
                }
            }
        }
        Layout::Varint => {
            for f in 0..k {
                for i in 0..n {
                    vals[f][i] = read_varint(bytes, &mut at);
                }
            }
            for t in &mut trails {
                *t = read_varint(bytes, &mut at);
            }
        }
    }
    let opens = (0..n)
        .map(|i| Open {
            kind: vals[0][i] as u16,
            field: vals[1][i] as u8,
            flags: vals[2][i] as u8,
            memb: vals[3][i] as u16,
            lead: vals[4][i],
            pops: vals[5][i],
        })
        .collect();
    Block { opens, trails, end_pops, final_gap }
}
