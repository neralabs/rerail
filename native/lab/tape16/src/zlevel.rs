//! Decision 0399 §6: what 16z's block sink needs of its compressor. The engine's format 16 tape of each file
//! (`f16::write_from`, the shipped tables), cut into 16z's pieces as its writer cuts them (a block's column at its
//! rows, then that block's side-list part), each piece stored as 16z stores it (LZ4 when that shrinks it by a
//! tenth, else raw; repeats left out, the same for every compressor), by each compressor: LZ4 HC at a level, or
//! LZ4's fast compressor.

use crate::capture::File;
use rerail_tape_format::f16::{self, Written};
use rerail_tape_format::tape::{F15, Tape, f15_columns};
use std::os::raw::c_char;

const Z: usize = 1024;
const HEAD16: usize = 120;
const LZ4_MIN: usize = 16;

/// The compressors measured: an HC level, or 0 for LZ4's fast one.
pub const LEVELS: [i32; 7] = [0, 1, 2, 3, 4, 6, 9];

fn compress(src: &[u8], level: i32) -> usize {
    let n = src.len() as i32;
    // SAFETY: LZ4's own bound for `n` bytes; both buffers as long as the lengths given.
    unsafe {
        let cap = lz4_sys::LZ4_compressBound(n);
        let mut out = vec![0u8; cap as usize];
        let k = if level == 0 {
            lz4_sys::LZ4_compress_default(src.as_ptr() as *const c_char, out.as_mut_ptr() as *mut c_char, n, cap)
        } else {
            lz4_sys::LZ4_compress_HC(src.as_ptr() as *const c_char, out.as_mut_ptr() as *mut c_char, n, cap, level)
        };
        assert!(k > 0);
        k as usize
    }
}

fn u32_at(t: &[u8], at: usize) -> usize {
    u32::from_le_bytes(t[at..at + 4].try_into().unwrap()) as usize
}

/// A format 16 tape's pieces, each a block's column `q` at its rows and its side-list part `q`.
fn pieces(t: &[u8]) -> Vec<Vec<u8>> {
    let n = u64::from_le_bytes(t[24..32].try_into().unwrap()) as usize;
    let w = [t[36] as usize, t[37] as usize, t[38] as usize];
    let stride = u32_at(t, 40);
    let side: [usize; 4] = std::array::from_fn(|q| u32_at(t, 48 + 4 * q));
    let parts: [usize; 4] = std::array::from_fn(|q| u32_at(t, 64 + 4 * q));
    let cell = [1, w[0], w[1], w[2]];
    let before = |r: usize| cell[..r].iter().sum::<usize>();
    let count = n.div_ceil(Z);
    let mut out = Vec::new();
    for b in 0..count {
        let rows = (n - b * Z).min(Z);
        let room = HEAD16 + b * stride;
        for q in 0..4 {
            let col = room + rows * before(q);
            let mut piece = t[col..col + rows * cell[q]].to_vec();
            let (s, e) = (u32_at(t, parts[q] + 4 * b), u32_at(t, parts[q] + 4 * (b + 1)));
            piece.extend_from_slice(&t[side[q] + s..side[q] + e]);
            out.push(piece);
        }
    }
    out
}

#[derive(Default)]
pub struct Report {
    pub files: usize,
    pub nodes: u64,
    pub tape16: u64,
    pub pieces: u64,
    pub raw: u64,
    /// By `LEVELS`: the pieces' bytes as stored, the pieces stored as LZ4, the compressor's time (ns).
    pub stored: [u64; LEVELS.len()],
    pub lz4: [u64; LEVELS.len()],
    pub ns: [u64; LEVELS.len()],
}

pub fn run(files: &[&File]) -> Result<Report, String> {
    let mut r = Report::default();
    for f in files {
        let (columns, table) = f15_columns(&f.image)?;
        let t = F15::new(&f.image, columns, &table)?;
        let Some(lang) = f16::lang(f.tag) else { return Err(format!("no format 16 table for tag {}", f.tag)) };
        let number = f16::numbers(lang, &table)?;
        let Written::F16(t16) = f16::write_from(lang, &t, &number, &f.source, f.tag, false)? else { unreachable!() };
        let bytes = t16.as_bytes();
        r.files += 1;
        r.nodes += u64::from_le_bytes(bytes[24..32].try_into().unwrap());
        r.tape16 += bytes.len() as u64;
        for p in pieces(bytes) {
            r.pieces += 1;
            r.raw += p.len() as u64;
            for (k, &level) in LEVELS.iter().enumerate() {
                let t0 = std::time::Instant::now();
                let z = if p.len() >= LZ4_MIN { compress(&p, level) } else { usize::MAX };
                r.ns[k] += t0.elapsed().as_nanos() as u64;
                if z != usize::MAX && 10 * z <= 9 * p.len() {
                    r.stored[k] += z as u64;
                    r.lz4[k] += 1;
                } else {
                    r.stored[k] += p.len() as u64;
                }
            }
        }
    }
    Ok(r)
}

pub fn print(title: &str, r: &Report) {
    println!("### {title}\n");
    println!("{} files, {} nodes, format 16 {:.2} B/node, {} pieces, {:.2} B/node raw\n", r.files, r.nodes, r.tape16 as f64 / r.nodes as f64, r.pieces, r.raw as f64 / r.nodes as f64);
    println!("| compressor | pieces B/node | vs HC 9 | LZ4 pieces | ns/node |");
    println!("|---|---|---|---|---|");
    let hc9 = r.stored[LEVELS.len() - 1] as f64;
    for (k, &level) in LEVELS.iter().enumerate() {
        let name = if level == 0 { "LZ4 fast".to_owned() } else { format!("HC {level}") };
        println!("| {name} | {:.3} | {:+.1}% | {} | {:.1} |", r.stored[k] as f64 / r.nodes as f64, 100.0 * (r.stored[k] as f64 / hc9 - 1.0), r.lz4[k], r.ns[k] as f64 / r.nodes as f64);
    }
    println!();
}

/// Decision 0399 §6, R13 at scale: each file's format 16 and 16z tapes by the writer's ops in CLIF (their Rust)
/// and by the reference writer, byte for byte; each writer's time.
#[derive(Default)]
pub struct Ops {
    pub files: usize,
    pub nodes: u64,
    pub same: [usize; 2],
    pub first_differs: Option<String>,
    pub ns_ref: [u64; 2],
    pub ns_ops: [u64; 2],
}

pub fn ops(files: &[&File]) -> Result<Ops, String> {
    let mut r = Ops::default();
    for f in files {
        let (columns, table) = f15_columns(&f.image)?;
        let t = F15::new(&f.image, columns, &table)?;
        let Some(lang) = f16::lang(f.tag) else { return Err(format!("no format 16 table for tag {}", f.tag)) };
        let number = f16::numbers(lang, &table)?;
        r.files += 1;
        r.nodes += t.len() as u64;
        for (k, z) in [false, true].into_iter().enumerate() {
            let t0 = std::time::Instant::now();
            let want = f16::write_from(lang, &t, &number, &f.source, f.tag, z)?;
            r.ns_ref[k] += t0.elapsed().as_nanos() as u64;
            let t0 = std::time::Instant::now();
            let got = f16::ops::write_from(lang, &t, &number, &f.source, f.tag, z)?;
            r.ns_ops[k] += t0.elapsed().as_nanos() as u64;
            let bytes = |w: &Written| match w {
                Written::F16(t) => t.as_bytes().to_vec(),
                Written::F16Z(t) => t.as_bytes().to_vec(),
            };
            if bytes(&want) == bytes(&got) {
                r.same[k] += 1;
            } else if r.first_differs.is_none() {
                let (a, b) = (bytes(&want), bytes(&got));
                let at = (0..a.len().min(b.len())).find(|&k| a[k] != b[k]);
                let u32_at = |t: &[u8], k: usize| u32::from_le_bytes(t[k..k + 4].try_into().unwrap());
                let tail = |t: &[u8]| (0..10).map(|k| u32_at(t, 48 + 4 * k)).collect::<Vec<_>>();
                r.first_differs = Some(format!("{} (16z {z}): lengths {} and {}, first at {at:?}, tails {:?} and {:?}", f.path, a.len(), b.len(), tail(&a), tail(&b)));
            }
        }
    }
    Ok(r)
}

pub fn print_ops(title: &str, r: &Ops) {
    println!("### {title}\n");
    println!("{} files, {} nodes: format 16 the same in {}, 16z in {}; first differing: {}", r.files, r.nodes, r.same[0], r.same[1], r.first_differs.as_deref().unwrap_or("none"));
    let per = |ns: u64| ns as f64 / r.nodes as f64;
    println!("ns/node: 16 reference {:.1}, ops {:.1}; 16z reference {:.1}, ops {:.1}\n", per(r.ns_ref[0]), per(r.ns_ops[0]), per(r.ns_ref[1]), per(r.ns_ops[1]));
}
