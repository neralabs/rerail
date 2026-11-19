//! The floor of a simdjson-shaped lexer for C: stage 1 as simdjson's structural indexer, over the
//! same files the cost example reads. `stage1_floor <list> <passes>`.
//!
//! Per 64-byte block: the byte classes by simdjson's nibble tables (`rerail_simd8`), a token-start
//! mask (a name's or number's first byte, and every other byte that is neither blank nor a line
//! end), and the starts flattened into an index of positions, four at a time and written behind,
//! as simdjson's `bit_indexer::write`. It does not hide strings or comments, nor join two-byte
//! punctuators: a measure of the shape's cost, not a lexer. Its start count is printed beside the
//! lexer's token count for that reason.

use rerail_simd8::Simd8x64;

const C_WORD: u8 = 1;
const C_GAP: u8 = 2;

/// Low nibbles: word bytes [0-9A-Za-z_] and gaps (' ', '\t', '\n', '\v', '\f', '\r').
static LO: [u8; 16] = {
    let mut t = [0u8; 16];
    let mut c = 0usize;
    while c < 128 {
        let b = c as u8;
        let lo = c & 15;
        if b.is_ascii_alphanumeric() || b == b'_' {
            t[lo] |= C_WORD;
        }
        if matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r') {
            t[lo] |= C_GAP;
        }
        c += 1;
    }
    t
};
/// High nibbles, the same classes: a byte is in a class where both nibbles' tables have its bit.
/// (Word's nibble product over-approximates: `@`, `[`, `` ` `` and the like have word nibbles; for a
/// floor that is a start more or less, not a cost.)
static HI: [u8; 16] = {
    let mut t = [0u8; 16];
    t[0] = C_GAP;
    t[2] = C_GAP;
    t[3] = C_WORD;
    t[4] = C_WORD;
    t[5] = C_WORD;
    t[6] = C_WORD;
    t[7] = C_WORD;
    t
};

#[inline(always)]
unsafe fn block<S: Simd8x64>(b: &[u8; 64], prev_word: &mut u64) -> u64 {
    unsafe {
        let v = S::load(b);
        let c = S::classify(v, &LO, &HI);
        let word = S::to_bitmask(S::test(c, S::splat(C_WORD)));
        let gap = S::to_bitmask(S::test(c, S::splat(C_GAP)));
        // A word byte after a word byte starts nothing (simdjson's `follows`).
        let follows = (word << 1) | *prev_word;
        *prev_word = word >> 63;
        !gap & !(word & follows)
    }
}

/// simdjson's `bit_indexer::write`: the positions of `bits`' set bits, four at a time, written
/// behind (the buffer has room for 64 past its end), up to 24 before a loop.
#[inline(always)]
fn write(tail: &mut *mut u32, idx: u32, mut bits: u64) {
    if bits == 0 {
        return;
    }
    let cnt = bits.count_ones() as usize;
    let t = *tail;
    // SAFETY: the index has room for every byte's position and 64 more.
    unsafe {
        let mut i = 0;
        while i < 24 {
            for k in 0..4 {
                *t.add(i + k) = idx + bits.trailing_zeros();
                bits &= bits.wrapping_sub(1);
            }
            i += 4;
            if i >= cnt {
                break;
            }
        }
        while i < cnt {
            *t.add(i) = idx + bits.trailing_zeros();
            bits &= bits.wrapping_sub(1);
            i += 1;
        }
        *tail = t.add(cnt);
    }
}

unsafe fn stage1<S: Simd8x64>(src: &[u8], index: &mut Vec<u32>) -> usize {
    index.clear();
    index.reserve(src.len() + 64);
    let mut tail = index.as_mut_ptr();
    let mut prev_word = 0;
    let mut at = 0;
    while let Some(b) = src.get(at..at + 64) {
        let starts = unsafe { block::<S>(b.try_into().expect("64 bytes"), &mut prev_word) };
        write(&mut tail, at as u32, starts);
        at += 64;
    }
    if at < src.len() {
        let mut b = [b' '; 64];
        b[..src.len() - at].copy_from_slice(&src[at..]);
        let starts = unsafe { block::<S>(&b, &mut prev_word) };
        write(&mut tail, at as u32, starts);
    }
    // SAFETY: `tail` is within the reserved buffer, every position before it written.
    let n = unsafe { tail.offset_from(index.as_ptr()) as usize };
    unsafe { index.set_len(n) };
    n
}

rerail_simd8::dispatch! {
    fn stage1_best(src: &[u8], index: &mut Vec<u32>) -> usize = stage1, on stage1_on;
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let list = std::fs::read_to_string(&args[1]).unwrap();
    let passes: usize = args.get(2).map_or(1, |p| p.parse().unwrap());
    let files: Vec<Vec<u8>> = list.lines().filter(|l| !l.is_empty()).filter_map(|p| std::fs::read(p).ok()).collect();
    let files: Vec<Vec<u8>> = files.into_iter().filter(|f| rerail_c_lean::tape(f, rerail_c_lean::Dialect::C).is_ok()).collect();
    let (mut starts, mut bytes, mut sum) = (0usize, 0usize, 0u64);
    let mut index = Vec::new();
    for _ in 0..passes {
        for f in &files {
            starts += stage1_best(f, &mut index);
            bytes += f.len();
            sum = sum.wrapping_add(index.last().copied().unwrap_or(0).into());
        }
    }
    eprintln!("{} files, {passes} passes, {bytes} bytes, {starts} starts (checksum {sum})", files.len());
}
