//! Format 15 images (v9np, decision 0025) read into a `Tree`, and their stored size as the store keeps
//! them (`content_store::compression::compress_as`: zstd level 3, and below 64 KiB the smaller of a plain
//! frame and one with the language's dictionary).

use crate::tree::{Node, Tree};
use rerail_tape_format::{CHECKPOINT_STRIDE, HEADER_BYTES, TAIL_FROM_START, header, padded};

pub fn read(bytes: &[u8]) -> Result<Tree, String> {
    let u64_at = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize;
    if bytes.len() < HEADER_BYTES || &bytes[..8] != rerail_tape_format::MAGIC {
        return Err("not a format-15 image".into());
    }
    let w = [bytes[header::WIDTHS] as usize, bytes[header::WIDTHS + 1] as usize, (bytes[header::WIDTHS + 2] & !TAIL_FROM_START) as usize];
    let from_start = bytes[header::WIDTHS + 2] & TAIL_FROM_START != 0;
    let (n, codes) = (u64_at(header::NODES), u64_at(header::CODES));
    let source_len = u64_at(header::SOURCE_LEN) as u64;
    let mut at = HEADER_BYTES;
    let mut column = |width: usize, count: usize| {
        let c = &bytes[at..at + width * count];
        at = padded(at + width * count);
        c
    };
    let code = column(2, n);
    let size = column(w[0], n);
    let base = column(4, n.div_ceil(CHECKPOINT_STRIDE));
    let off = column(w[1], n);
    let tail = column(w[2], n);
    let mut table = Vec::with_capacity(codes);
    let mut c = at;
    for _ in 0..codes {
        let kf = u16::from_le_bytes([bytes[c], bytes[c + 1]]);
        let flags = bytes[c + 2];
        let alias = u16::from_le_bytes([bytes[c + 3], bytes[c + 4]]);
        let count = bytes[c + 5] as usize;
        let classes: Vec<u16> = (0..count).map(|k| u16::from_le_bytes([bytes[c + 6 + 2 * k], bytes[c + 7 + 2 * k]])).collect();
        // An alias is its own kind here (format 16: the grammar's alias range), so it shows in the kind.
        let kind = if alias != u16::MAX { 0x8000 | alias } else { kf & 1023 };
        table.push((kind, (kf >> 10) as u8, flags, classes));
        c += 6 + 2 * count;
    }
    let get = |col: &[u8], width: usize, i: usize| -> u64 {
        let mut v = [0u8; 8];
        v[..width].copy_from_slice(&col[i * width..i * width + width]);
        u64::from_le_bytes(v)
    };
    let start = |i: usize| get(base, 4, i / CHECKPOINT_STRIDE) + get(off, w[1], i);
    let mut nodes = Vec::with_capacity(n);
    let mut sizes = Vec::with_capacity(n);
    for i in 0..n {
        let s = get(size, w[0], i) as usize;
        let id = u16::from_le_bytes([code[2 * i], code[2 * i + 1]]) as usize;
        let (kind, field, flags, classes) = table.get(id).ok_or("a code not in the table")?.clone();
        let last = i + s.max(1) - 1;
        let from = if from_start { start(i) } else { start(last) };
        let end = from.wrapping_add(get(tail, w[2], i)) & 0xffff_ffff;
        nodes.push(Node { kind, field, flags, classes, start: start(i), end });
        sizes.push(s as u32);
    }
    let parent = Tree::parents_from_sizes(&sizes);
    Ok(Tree { nodes, size: sizes, parent, source_len })
}

/// The store's dictionary for a tape of tape tag `tag` (`native/dictionaries`), if it has one.
pub fn store_tape_dictionary(tag: u8) -> Option<Vec<u8>> {
    let name = match tag {
        1 => "python",
        2 | 3 => "typescript",
        6 => "javascript",
        7 => "php",
        9 => "rust",
        10 => "sql",
        _ => return None,
    };
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dictionaries");
    std::fs::read(dir.join(format!("{name}-tape.zdict"))).ok()
}

/// The store's source dictionary for tape tag `tag`, if it has one.
pub fn store_source_dictionary(tag: u8) -> Option<Vec<u8>> {
    let name = match tag {
        1 => "python",
        2 | 3 => "typescript",
        6 => "javascript",
        7 => "php",
        9 => "rust",
        _ => return None,
    };
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dictionaries");
    std::fs::read(dir.join(format!("{name}-source.zdict"))).ok()
}

/// Bytes as the store keeps them: zstd 3; below 64 KiB the smaller of plain and with `dictionary`.
pub fn stored_size(bytes: &[u8], dictionary: Option<&[u8]>) -> usize {
    let plain = zstd::bulk::compress(bytes, 3).expect("zstd").len();
    match dictionary {
        Some(d) if bytes.len() < 64 * 1024 => {
            let mut c = zstd::bulk::Compressor::with_dictionary(3, d).expect("zstd dictionary");
            plain.min(c.compress(bytes).expect("zstd").len())
        }
        _ => plain,
    }
}
