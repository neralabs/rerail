//! simdjson's arm64 stage 1 and our stage 2 (`rerail-simdjson-native`'s `kernel.inc`), translated
//! to Rust: the reference the generated tier 1 and tier 2 are read against. Not a parse path: the
//! engine reads JSON through `rerail-simdjson` (the C++ kernel) and the Bitlex parser.
//!
//! The translation is line for line, with simdjson's names:
//! - `stage1`: `json_structural_indexer::index<64>` and what it calls (simdjson 5.0.5's
//!   `simd.h`, `json_character_block::classify`, the escape, string and JSON scanners, the UTF-8
//!   checker, arm64's own `bit_indexer::write`);
//! - `stage2`: `kernel.inc`'s `walk`, which writes the v9np tape;
//! - `number`: `kernel.inc`'s `number_end`, simdjson's number check;
//! - `tables`: simdjson's own tables, copied by `scripts/json_lean/tables.py`.
//!
//! `Parser::tape` is `kernel.inc`'s `rerail_v9np::tape` without the source's hash, and
//! `Parser::stage1` its `stage1_only` (simdjson's stage 1): each copies the source into its padded
//! buffer first, as the kernel does. JSONC is left out.
//!
//! arm64 only (the implementation translated), and not on MSVC, where its references are empty
//! (`rerail-simdjson`); elsewhere the crate is empty.

#![cfg(all(target_arch = "aarch64", not(all(windows, target_env = "msvc"))))]

mod number;
mod stage1;
mod stage2;
mod tables;

pub use rerail_simdjson::{Error, Kinds, Refusal, Tape};

use stage2::{DEFAULT_MAX_DEPTH, SLOTS, Sink, Stacks, walk};

/// simdjson's `error_code`, the values this reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Code {
    Success = 0,
    Capacity = 1,
    Memalloc = 2,
    TapeError = 3,
    DepthError = 4,
    StringError = 5,
    TAtomError = 6,
    FAtomError = 7,
    NAtomError = 8,
    NumberError = 9,
    BigintError = 10,
    Utf8Error = 11,
    Uninitialized = 12,
    Empty = 13,
    UnescapedChars = 14,
    UnclosedString = 15,
    UnexpectedError = 24,
}

/// simdjson's `SIMDJSON_PADDING`.
const PADDING: usize = 64;

/// A thread's parser (`kernel.inc`'s `Thread`): the padded source, stage 1's index and the depth
/// stacks, kept from file to file.
pub struct Parser {
    buffer: Vec<u8>,
    indexes: Vec<u32>,
    capacity: usize,
    container: Box<[u32]>,
    pair: Box<[u32]>,
    objects: Box<[bool]>,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    pub fn new() -> Self {
        let depth = DEFAULT_MAX_DEPTH as usize + 1;
        Parser {
            buffer: Vec::new(),
            indexes: Vec::new(),
            capacity: 0,
            container: vec![0; depth].into_boxed_slice(),
            pair: vec![0; depth].into_boxed_slice(),
            objects: vec![false; depth].into_boxed_slice(),
        }
    }

    /// `padded`: the source, copied with simdjson's padding (zeros) after it.
    #[inline(always)]
    fn padded(&mut self, src: &[u8]) {
        if self.buffer.len() < src.len() + PADDING {
            self.buffer = vec![0; src.len() + PADDING];
        }
        self.buffer[..src.len()].copy_from_slice(src);
        self.buffer[src.len()..src.len() + PADDING].fill(0);
    }

    /// `ready`: stage 1's index made to hold `body` bytes' structurals (simdjson's `set_capacity`:
    /// the capacity rounded up to 64, and 2 + 7 more).
    #[inline(always)]
    fn ready(&mut self, body: usize) {
        if self.capacity < body {
            self.capacity = body.max(64);
            self.indexes = vec![0; self.capacity.div_ceil(64) * 64 + 2 + 7];
        }
    }

    /// Stage 1 alone, as `kernel.inc`'s `stage1_only` (unhashed: simdjson's own): the structural
    /// index's length, or simdjson's error code negated.
    pub fn stage1(&mut self, src: &[u8]) -> i64 {
        self.padded(src);
        self.ready(src.len());
        // SAFETY: the buffer holds `src` and its padding; the index, the room `index` needs.
        match unsafe { stage1::index::<{ stage1::ROUNDS }>(self.buffer.as_ptr(), src.len(), self.indexes.as_mut_ptr()) } {
            Ok(n) => i64::from(n),
            Err(e) => -(e as i64),
        }
    }

    /// `stage1` with the string's toggle in another form (`form`: 1, the carry-less multiply
    /// simdjson left out, `vmull_p64`; 2, that multiply of the quotes as the vector registers hold
    /// them): the measure of the forms. The caller checks the host has `pmull`.
    pub fn stage1_form(&mut self, src: &[u8], form: u8) -> i64 {
        self.padded(src);
        self.ready(src.len());
        let (buf, len, out) = (self.buffer.as_ptr(), src.len(), self.indexes.as_mut_ptr());
        // SAFETY: as `stage1`.
        let r = unsafe {
            match form {
                stage1::PMULL => stage1::index::<{ stage1::PMULL }>(buf, len, out),
                stage1::PMULL_RAW => stage1::index::<{ stage1::PMULL_RAW }>(buf, len, out),
                _ => stage1::index::<{ stage1::ROUNDS }>(buf, len, out),
            }
        };
        match r {
            Ok(n) => i64::from(n),
            Err(e) => -(e as i64),
        }
    }

    /// The structural index stage 1 found last (after `stage1`).
    pub fn indexes(&self, count: usize) -> &[u32] {
        &self.indexes[..count]
    }

    /// `src`'s tape, as `kernel.inc`'s `tape` writes it (no digest): stage 1, then the walk at the
    /// widths predicted, again at the widths a walk shows where it was wrong.
    pub fn tape(&mut self, src: &[u8], kinds: &Kinds) -> Result<Tape, Refusal> {
        if src.len() >= u32::MAX as usize / 2 {
            return Err(Refusal::TooLarge);
        }
        let k = kinds;
        let mut sink = Sink {
            col: Default::default(),
            width: [2; 3],
            kind: [k.document, k.pair, k.string_content, k.escape_sequence, k.object, k.array, k.string, k.number, k.true_, k.false_, k.null, k.comment],
            key_field: k.key_field,
            value_field: k.value_field,
            codes: 0,
            code_kind: [0; 36],
            code_field: [0; 36],
            code_value: [0; 36],
        };
        let code = self.tape_into(src, &mut sink);
        if code != 0 {
            return Err(Refusal::Simdjson(Error(code)));
        }
        for v in &mut sink.col {
            // The buffers were made for the most nodes a file of its length can have: what the tape
            // does not use is given back (as `rerail_simdjson::tape_with`).
            if v.capacity() > v.len() + v.len() / 2 + 64 {
                v.shrink_to_fit();
            }
        }
        let codes = (0..sink.codes as usize).map(|i| (sink.code_kind[i], sink.code_field[i], sink.code_value[i] != 0)).collect();
        let [code, size, off, tail, base] = sink.col;
        Ok(Tape { code, codes, size, base, off, tail, widths: sink.width, digest: None })
    }

    /// `tape`'s body: 0, or simdjson's error code.
    #[inline(never)]
    fn tape_into(&mut self, src: &[u8], sink: &mut Sink) -> i32 {
        let len = src.len();
        self.padded(src);
        let shift: u32 = if len >= 3 && src[..3] == [0xEF, 0xBB, 0xBF] { 3 } else { 0 };
        let body = len - shift as usize;
        self.ready(body);
        // SAFETY: the buffer holds `src` and its padding; the index, the room `index` needs.
        let buf = unsafe { self.buffer.as_ptr().add(shift as usize) };
        let count = match unsafe { stage1::index::<{ stage1::ROUNDS }>(buf, body, self.indexes.as_mut_ptr()) } {
            Ok(n) => n,
            Err(e) => return e as i32,
        };
        // `size` 4 bytes wide where the nodes, about one and a quarter a structural, pass 2 bytes;
        // `off` and `tail` 2. A prediction a walk shows wrong is walked again at the widths it showed.
        let mut wide: u32 = if count > 52000 { 1 } else { 0 };
        for _ in 0..3 {
            let r = self.walk_at(wide, buf, count, shift, len as u32, sink);
            if r < 0 {
                return -r;
            }
            if r == 0 {
                return 0;
            }
            wide = (wide | (r as u32 & 7)) & !(if r & 8 != 0 { 1 } else { 0 });
        }
        Code::UnexpectedError as i32
    }

    /// `walk_at`: a walk at the widths `wide` names (bit 1 size, 2 off, 4 tail: 4 bytes).
    fn walk_at(&mut self, wide: u32, buf: *const u8, count: u32, shift: u32, len: u32, sink: &mut Sink) -> i32 {
        let index = self.indexes.as_ptr();
        let st = Stacks { container: self.container.as_mut_ptr(), pair: self.pair.as_mut_ptr(), object: self.objects.as_mut_ptr() };
        // SAFETY: `buf` and the index are stage 1's; the stacks hold `DEFAULT_MAX_DEPTH + 1`.
        unsafe {
            match wide {
                0 => walk::<u16, u16, u16>(buf, index, count, shift, len, sink, st),
                1 => walk::<u32, u16, u16>(buf, index, count, shift, len, sink, st),
                2 => walk::<u16, u32, u16>(buf, index, count, shift, len, sink, st),
                3 => walk::<u32, u32, u16>(buf, index, count, shift, len, sink, st),
                4 => walk::<u16, u16, u32>(buf, index, count, shift, len, sink, st),
                5 => walk::<u32, u16, u32>(buf, index, count, shift, len, sink, st),
                6 => walk::<u16, u32, u32>(buf, index, count, shift, len, sink, st),
                _ => walk::<u32, u32, u32>(buf, index, count, shift, len, sink, st),
            }
        }
    }
}

const _: () = assert!(SLOTS == 12);

#[cfg(test)]
mod tests;
