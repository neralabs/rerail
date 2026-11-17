//! Our stage 2, `kernel.inc`'s `walk`, translated: simdjson's `json_iterator::walk_document` with
//! the tape builder's checks, writing the v9np tape at each node's preorder slot. Each lambda of
//! `walk` is a method of `Walk`, inlined; each `goto` label a state of one loop, whose every
//! transition is a constant (LLVM threads them into the branches `goto` makes).
//!
//! JSONC (`kernel.inc`'s `C`) is left out: its `gap` and comment cases are no code where `C` is
//! false, the case this reads.

use crate::Code;
use crate::number::{is_not_structural_or_whitespace, number_end};
use crate::stage1::Simd8x64;
use crate::tables::{DIGIT_TO_VAL32, ESCAPE_MAP};

// A node's kind, by slot, and its field: as `rerail_simdjson`'s writer numbers them.
pub(crate) const DOCUMENT: u32 = 0;
pub(crate) const PAIR: u32 = 1;
pub(crate) const CONTENT: u32 = 2;
pub(crate) const ESCAPE: u32 = 3;
pub(crate) const OBJECT: u32 = 4;
pub(crate) const ARRAY: u32 = 5;
pub(crate) const STRING: u32 = 6;
pub(crate) const NUMBER: u32 = 7;
pub(crate) const TRUE_: u32 = 8;
pub(crate) const FALSE_: u32 = 9;
pub(crate) const NULL_: u32 = 10;
pub(crate) const SLOTS: usize = 12;
pub(crate) const NO_FIELD: u32 = 0;
pub(crate) const KEY: u32 = 1;
pub(crate) const VALUE: u32 = 2;
// The tape's buffers, in the sink's order.
pub(crate) const CODE: usize = 0;
pub(crate) const SIZE: usize = 1;
pub(crate) const OFF: usize = 2;
pub(crate) const TAIL: usize = 3;
pub(crate) const BASE: usize = 4;
const STRIDE: u32 = 16;
pub(crate) const DEFAULT_MAX_DEPTH: u32 = 1024;

/// A column's cell: 2 or 4 bytes (`kernel.inc`'s `SZ`, `OF` and `TL`).
pub(crate) trait Cell: Copy {
    const WIDTH: u32;
    fn of(v: u32) -> Self;
}
impl Cell for u16 {
    const WIDTH: u32 = 2;
    #[inline(always)]
    fn of(v: u32) -> Self {
        v as u16
    }
}
impl Cell for u32 {
    const WIDTH: u32 = 4;
    #[inline(always)]
    fn of(v: u32) -> Self {
        v
    }
}

/// The tape's buffers and codes (`kernel.inc`'s `rerail_v9np_sink`): the columns are the caller's
/// vectors, grown once for the most nodes a file can have, then written through raw pointers.
pub(crate) struct Sink {
    pub(crate) col: [Vec<u8>; 5],
    pub(crate) width: [u8; 3],
    pub(crate) kind: [u16; SLOTS],
    pub(crate) key_field: u16,
    pub(crate) value_field: u16,
    pub(crate) codes: u32,
    pub(crate) code_kind: [u16; 36],
    pub(crate) code_field: [u16; 36],
    pub(crate) code_value: [u8; 36],
}

/// `column_bytes`: the bytes of `c` for `nodes` nodes, `size`, `off` and `tail` at `width`.
#[inline(always)]
fn column_bytes(c: usize, nodes: u32, width: &[u32; 3]) -> usize {
    if c == CODE {
        nodes as usize * 2
    } else if c == BASE {
        nodes.div_ceil(STRIDE) as usize * 4
    } else {
        nodes as usize * width[c - 1] as usize
    }
}

/// `reserve_columns`: the buffers made to hold `nodes` nodes at `width`, empty.
#[inline(never)]
fn reserve_columns(sink: &mut Sink, nodes: u32, width: &[u32; 3]) {
    for c in 0..5 {
        sink.col[c].clear();
        sink.col[c].reserve(column_bytes(c, nodes, width));
    }
}

/// `hex_to_u32_nocheck`.
#[inline(always)]
unsafe fn hex_to_u32_nocheck(src: *const u8) -> u32 {
    // SAFETY: four bytes of the padded source.
    unsafe {
        let v1 = DIGIT_TO_VAL32[630 + *src as usize];
        let v2 = DIGIT_TO_VAL32[420 + *src.add(1) as usize];
        let v3 = DIGIT_TO_VAL32[210 + *src.add(2) as usize];
        let v4 = DIGIT_TO_VAL32[*src.add(3) as usize];
        v1 | v2 | v3 | v4
    }
}

/// `atomparsing::str4ncmp`.
#[inline(always)]
unsafe fn str4ncmp(src: *const u8, atom: &[u8; 4]) -> u32 {
    // SAFETY: four bytes of the padded source.
    unsafe { (src as *const u32).read_unaligned() ^ u32::from_le_bytes(*atom) }
}

#[inline(always)]
unsafe fn is_valid_true_atom(src: *const u8) -> bool {
    // SAFETY: five bytes of the padded source.
    unsafe { (str4ncmp(src, b"true") | is_not_structural_or_whitespace(*src.add(4))) == 0 }
}
#[inline(always)]
unsafe fn is_valid_false_atom(src: *const u8) -> bool {
    // SAFETY: six bytes of the padded source.
    unsafe { (str4ncmp(src.add(1), b"alse") | is_not_structural_or_whitespace(*src.add(5))) == 0 }
}
#[inline(always)]
unsafe fn is_valid_null_atom(src: *const u8) -> bool {
    // SAFETY: five bytes of the padded source.
    unsafe { (str4ncmp(src, b"null") | is_not_structural_or_whitespace(*src.add(4))) == 0 }
}
#[inline(always)]
unsafe fn is_valid_true_atom_len(src: *const u8, len: usize) -> bool {
    // SAFETY: as `is_valid_true_atom`.
    unsafe {
        if len > 4 {
            is_valid_true_atom(src)
        } else if len == 4 {
            str4ncmp(src, b"true") == 0
        } else {
            false
        }
    }
}
#[inline(always)]
unsafe fn is_valid_false_atom_len(src: *const u8, len: usize) -> bool {
    // SAFETY: as `is_valid_false_atom`.
    unsafe {
        if len > 5 {
            is_valid_false_atom(src)
        } else if len == 5 {
            str4ncmp(src.add(1), b"alse") == 0
        } else {
            false
        }
    }
}
#[inline(always)]
unsafe fn is_valid_null_atom_len(src: *const u8, len: usize) -> bool {
    // SAFETY: as `is_valid_null_atom`.
    unsafe {
        if len > 4 {
            is_valid_null_atom(src)
        } else if len == 4 {
            str4ncmp(src, b"null") == 0
        } else {
            false
        }
    }
}

/// `backslash_quote<simd8<uint8_t>>`: the 32 bytes at `p`, a bit for each `\` (low half) and each
/// `"` (high half), in two 16-byte vectors.
#[inline(always)]
unsafe fn backslash_quote(p: *const u8) -> u64 {
    use core::arch::aarch64::*;
    // SAFETY: 32 bytes of the padded source.
    unsafe {
        let v0 = vld1q_u8(p);
        let v1 = vld1q_u8(p.add(16));
        let bs = vmovq_n_u8(b'\\');
        let q = vmovq_n_u8(b'"');
        Simd8x64::to_bitmask([vceqq_u8(v0, bs), vceqq_u8(v1, bs), vceqq_u8(v0, q), vceqq_u8(v1, q)])
    }
}

/// `walk`'s locals that its lambdas share.
struct Walk<SZ, OF, TL> {
    buf: *const u8,
    shift: u32,
    code: *mut u16,
    base: *mut u32,
    size: *mut SZ,
    off: *mut OF,
    tail: *mut TL,
    n: u32,
    block_base: u32,
    last_start: u32,
    high_size: u32,
    high_off: u32,
    high_tail: u32,
    /// `ids`, a local of `walk` of its own (an array the walk indexes, kept apart from the
    /// scalars, which stay in registers).
    ids: *mut u16,
}

impl<SZ: Cell, OF: Cell, TL: Cell> Walk<SZ, OF, TL> {
    /// `code_of`: a node's code, numbered by first use. The first use is `simdjson_unlikely`, in
    /// place.
    #[inline(always)]
    fn code_of(&mut self, sink: &mut Sink, slot: u32, field: u32) -> u16 {
        let key = (slot * 3 + field) as usize;
        // SAFETY: `key` is below `SLOTS * 3`, `ids`' length.
        let mut id = unsafe { *self.ids.add(key) };
        if id == 0xffff {
            core::hint::cold_path();
            id = sink.codes as u16;
            sink.codes += 1;
            unsafe { *self.ids.add(key) = id };
            sink.code_kind[id as usize] = sink.kind[slot as usize];
            sink.code_field[id as usize] = if field == KEY {
                sink.key_field
            } else if field == VALUE {
                sink.value_field
            } else {
                0
            };
            // A value's place: the kinds a value can be, unless keyed.
            sink.code_value[id as usize] = u8::from((OBJECT..=NULL_).contains(&slot) && field != KEY);
        }
        id
    }

    /// `open`: a node's start: its code, its offset from its block's base, the base of each block.
    #[inline(always)]
    fn open(&mut self, sink: &mut Sink, slot: u32, field: u32, start: u32) -> u32 {
        let c = self.code_of(sink, slot, field);
        // SAFETY: the columns hold `bound` nodes, more than any file has.
        unsafe {
            self.code.add(self.n as usize).write_unaligned(c);
            if self.n % STRIDE == 0 {
                self.block_base = start;
                self.base.add((self.n / STRIDE) as usize).write_unaligned(start);
            }
            let o = start - self.block_base;
            self.high_off |= o;
            self.off.add(self.n as usize).write_unaligned(OF::of(o));
        }
        self.last_start = start;
        let ord = self.n;
        self.n += 1;
        ord
    }

    /// `close`: a node's end: its size and its tail (its end less its last descendant's start).
    #[inline(always)]
    fn close(&mut self, ord: u32, end: u32) {
        let t = end.wrapping_sub(self.last_start);
        let z = self.n - ord;
        self.high_tail |= t;
        self.high_size |= z;
        // SAFETY: `ord` was opened.
        unsafe {
            self.tail.add(ord as usize).write_unaligned(TL::of(t));
            self.size.add(ord as usize).write_unaligned(SZ::of(z));
        }
    }

    /// `leaf`.
    #[inline(always)]
    fn leaf(&mut self, sink: &mut Sink, slot: u32, field: u32, start: u32, end: u32) {
        let ord = self.open(sink, slot, field, start);
        let t = end.wrapping_sub(start);
        self.high_tail |= t;
        // SAFETY: `ord` was opened.
        unsafe {
            self.tail.add(ord as usize).write_unaligned(TL::of(t));
            self.size.add(ord as usize).write_unaligned(SZ::of(1));
        }
    }

    /// `at`.
    #[inline(always)]
    fn at(&self, p: *const u8) -> u32 {
        // SAFETY: `p` is in the source.
        (unsafe { p.offset_from(self.buf) } as u32).wrapping_add(self.shift)
    }

    /// `escaped_string`: a string with escapes: its contents' runs and its escapes, each escape
    /// checked as simdjson's `parse_string` checks it. Its end, or 0.
    #[inline(always)]
    unsafe fn escaped_string(&mut self, sink: &mut Sink, v: *const u8, field: u32) -> u32 {
        let ord = self.open(sink, STRING, field, self.at(v));
        // SAFETY: within the padded source: a string ends before the padding.
        unsafe {
            let mut p = v.add(1);
            let mut from = p;
            let mut low_checked: *const u8 = core::ptr::null();
            loop {
                let bits = backslash_quote(p);
                let bs = bits as u32;
                let quote = (bits >> 32) as u32;
                if (bs.wrapping_sub(1) & quote) != 0 {
                    let q = p.add(quote.trailing_zeros() as usize);
                    if q > from {
                        self.leaf(sink, CONTENT, NO_FIELD, self.at(from), self.at(q));
                    }
                    let end = self.at(q) + 1;
                    self.close(ord, end);
                    return end;
                }
                if bs == 0 {
                    p = p.add(32);
                    continue;
                }
                let e = p.add(bs.trailing_zeros() as usize);
                let c = *e.add(1);
                if c == b'u' {
                    if e == low_checked {
                        low_checked = core::ptr::null();
                    } else {
                        let cp = hex_to_u32_nocheck(e.add(2));
                        if (0xd800..0xdc00).contains(&cp) {
                            if *e.add(6) != b'\\' || *e.add(7) != b'u' || (hex_to_u32_nocheck(e.add(8)).wrapping_sub(0xdc00) >> 10) != 0 {
                                return 0;
                            }
                            low_checked = e.add(6);
                        } else if cp > 0xffff || (0xdc00..=0xdfff).contains(&cp) {
                            // Not four hex digits, or a lone low surrogate.
                            return 0;
                        }
                    }
                } else if ESCAPE_MAP[c as usize] == 0 {
                    return 0;
                }
                if e > from {
                    self.leaf(sink, CONTENT, NO_FIELD, self.at(from), self.at(e));
                }
                self.leaf(sink, ESCAPE, NO_FIELD, self.at(e), self.at(e) + 2);
                p = e.add(2);
                from = p;
            }
        }
    }

    /// `string`: a string: its end, or 0. One with no escape in its first 32 bytes, the most, is
    /// written as its two nodes at once (the string and its content), or one where it is empty.
    #[inline(always)]
    unsafe fn string(&mut self, sink: &mut Sink, v: *const u8, field: u32) -> u32 {
        let s = self.at(v);
        // SAFETY: 32 bytes of the padded source.
        let bits = unsafe { backslash_quote(v.add(1)) };
        let bs = bits as u32;
        let quote = (bits >> 32) as u32;
        if (bs.wrapping_sub(1) & quote) != 0 {
            let q = quote.trailing_zeros();
            if q == 0 {
                self.leaf(sink, STRING, field, s, s + 2);
                return s + 2;
            }
            let ord = self.open(sink, STRING, field, s);
            let content = self.open(sink, CONTENT, NO_FIELD, s + 1);
            self.high_tail |= q + 1;
            // SAFETY: both opened.
            unsafe {
                self.tail.add(content as usize).write_unaligned(TL::of(q));
                self.size.add(content as usize).write_unaligned(SZ::of(1));
                self.tail.add(ord as usize).write_unaligned(TL::of(q + 1));
                self.size.add(ord as usize).write_unaligned(SZ::of(2));
            }
            return s + q + 2;
        }
        // `simdjson_likely` above: the escaped string is the cold path.
        core::hint::cold_path();
        // SAFETY: as above.
        unsafe { self.escaped_string(sink, v, field) }
    }

    /// `primitive`: a scalar in an array or an object: its end, or 0 and `error`.
    #[inline(always)]
    unsafe fn primitive(&mut self, sink: &mut Sink, v: *const u8, field: u32, error: &mut Code) -> u32 {
        let s = self.at(v);
        // SAFETY: within the padded source.
        unsafe {
            if *v == b'"' {
                let end = self.string(sink, v, field);
                if end == 0 {
                    core::hint::cold_path();
                    *error = Code::StringError;
                }
                return end;
            }
            // As simdjson's `visit_primitive`: a byte below `0` is a number's too, refused by its check.
            // `*v - '0'` is an `int` in C++: below `0` is negative, so below 10.
            if i32::from(*v) - i32::from(b'0') < 10 || *v == b'-' {
                let mut e = v;
                *error = number_end(v, &mut e);
                if *error != Code::Success {
                    core::hint::cold_path();
                    return 0;
                }
                self.leaf(sink, NUMBER, field, s, self.at(e));
                return self.at(e);
            }
            let (slot, length) = match *v {
                b't' => {
                    if !is_valid_true_atom(v) {
                        *error = Code::TAtomError;
                        return 0;
                    }
                    (TRUE_, 4)
                }
                b'f' => {
                    if !is_valid_false_atom(v) {
                        *error = Code::FAtomError;
                        return 0;
                    }
                    (FALSE_, 5)
                }
                b'n' => {
                    if !is_valid_null_atom(v) {
                        *error = Code::NAtomError;
                        return 0;
                    }
                    (NULL_, 4)
                }
                _ => {
                    *error = Code::TapeError;
                    return 0;
                }
            };
            self.leaf(sink, slot, field, s, s + length);
            s + length
        }
    }

    /// `root_primitive`: the document a scalar alone: an atom checked against the bytes left, a
    /// number from a copy followed by spaces (as simdjson's tape builder does: the padding is zeros).
    #[inline(always)]
    unsafe fn root_primitive(&mut self, sink: &mut Sink, v: *const u8, len: u32) -> Code {
        // SAFETY: `v` is in the source.
        let rest = (len - self.shift) as usize - unsafe { v.offset_from(self.buf) } as usize;
        let s = self.at(v);
        // SAFETY: within the padded source.
        unsafe {
            match *v {
                b'"' => {
                    if self.string(sink, v, NO_FIELD) != 0 {
                        Code::Success
                    } else {
                        Code::StringError
                    }
                }
                b't' => {
                    if !is_valid_true_atom_len(v, rest) {
                        return Code::TAtomError;
                    }
                    self.leaf(sink, TRUE_, NO_FIELD, s, s + 4);
                    Code::Success
                }
                b'f' => {
                    if !is_valid_false_atom_len(v, rest) {
                        return Code::FAtomError;
                    }
                    self.leaf(sink, FALSE_, NO_FIELD, s, s + 5);
                    Code::Success
                }
                b'n' => {
                    if !is_valid_null_atom_len(v, rest) {
                        return Code::NAtomError;
                    }
                    self.leaf(sink, NULL_, NO_FIELD, s, s + 4);
                    Code::Success
                }
                _ => {
                    let mut copy = Vec::<u8>::with_capacity(rest + 64);
                    core::ptr::copy_nonoverlapping(v, copy.as_mut_ptr(), rest);
                    core::ptr::write_bytes(copy.as_mut_ptr().add(rest), b' ', 64);
                    copy.set_len(rest + 64);
                    let mut e = copy.as_ptr();
                    let r = number_end(copy.as_ptr(), &mut e);
                    if r != Code::Success {
                        return r;
                    }
                    self.leaf(sink, NUMBER, NO_FIELD, s, s + e.offset_from(copy.as_ptr()) as u32);
                    Code::Success
                }
            }
        }
    }
}

/// `walk`'s labels where control passes between containers. `kernel.inc`'s `goto`s are direct
/// branches; Rust has none (`become` is unstable), and a `loop { match label }` holding every
/// label compiles to a switch each transition passes (LLVM threads no jump into a loop header).
/// So each container's cycle (`object_key` to `object_continue`, `array_value` to
/// `array_continue`) is a loop of its own, its transitions direct, and only a container's opening
/// and closing pass this switch, where `kernel.inc` reads `object[d]`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Label {
    ObjectBegin,
    ObjectKey,
    ArrayBegin,
    ArrayValue,
    ScopeEnd,
    DocumentEnd,
}

/// `walk`: what a walk leaves: 0 and each 2-byte column's largest value fitting, or a column's bit
/// (1 size, 2 off, 4 tail) where one did not, 8 where `size` is 4 bytes and its values fit 2; or
/// simdjson's error code negated.
///
/// # Safety
/// `buf` is the padded source after any byte order mark, `index` its `count` structurals and the
/// three entries after them; `container`, `pair` and `object` hold `DEFAULT_MAX_DEPTH + 1`.
#[inline(never)]
pub(crate) unsafe fn walk<SZ: Cell, OF: Cell, TL: Cell>(
    buf: *const u8,
    index: *const u32,
    count: u32,
    shift: u32,
    len: u32,
    sink: &mut Sink,
    container: *mut u32,
    pair: *mut u32,
    object: *mut bool,
) -> i32 {
    let widths = [SZ::WIDTH, OF::WIDTH, TL::WIDTH];
    // A node starts at a byte of its own, but a pair (its key's quote) and the document (its value's
    // first byte); a pair takes four bytes or more. So the tape has at most this many nodes, and the
    // buffers are made for them once: no node checks for room.
    let bound = len + len / 4 + 2;
    reserve_columns(sink, bound, &widths);
    let mut ids = [0xffffu16; SLOTS * 3];
    let mut w = Walk::<SZ, OF, TL> {
        buf,
        shift,
        code: sink.col[CODE].as_mut_ptr().cast(),
        base: sink.col[BASE].as_mut_ptr().cast(),
        size: sink.col[SIZE].as_mut_ptr().cast(),
        off: sink.col[OFF].as_mut_ptr().cast(),
        tail: sink.col[TAIL].as_mut_ptr().cast(),
        n: 0,
        block_base: 0,
        last_start: 0,
        high_size: 0,
        high_off: 0,
        high_tail: 0,
        ids: ids.as_mut_ptr(),
    };
    sink.codes = 0;
    let neg = |e: Code| -(e as i32);

    // SAFETY: every read is of the padded source or the index and its three entries after it, as
    // `kernel.inc`'s; every stack is within `DEFAULT_MAX_DEPTH`.
    unsafe {
        let mut next = index;
        let mut d: u32 = 0; // the open containers
        let mut vf = NO_FIELD; // a value's field here: `value` in an object
        let mut end: u32;
        let mut error = Code::Success;
        let mut v: *const u8;
        *object = false;

        macro_rules! take {
            () => {{
                let x = *next;
                next = next.add(1);
                x
            }};
        }
        // `empty`: an empty object or array at `v`, its closer the next token: a leaf.
        macro_rules! empty {
            ($slot:expr, $field:expr) => {{
                let start = w.at(v);
                end = *next + shift + 1;
                w.leaf(sink, $slot, $field, start, end);
                next = next.add(1);
            }};
        }

        // The document starts at its first token.
        w.open(sink, DOCUMENT, NO_FIELD, *index + shift);
        v = buf.add(take!() as usize);
        // The outer object or array closed at the document's end (simdjson's check).
        let last = *buf.add(*index.add(count as usize - 1) as usize);
        if (*v == b'{' && last != b'}') || (*v == b'[' && last != b']') {
            return neg(Code::TapeError);
        }
        let mut label = match *v {
            b'{' => {
                if *buf.add(*next as usize) == b'}' {
                    empty!(OBJECT, NO_FIELD);
                    Label::DocumentEnd
                } else {
                    Label::ObjectBegin
                }
            }
            b'[' => {
                if *buf.add(*next as usize) == b']' {
                    empty!(ARRAY, NO_FIELD);
                    Label::DocumentEnd
                } else {
                    Label::ArrayBegin
                }
            }
            _ => {
                error = w.root_primitive(sink, v, len);
                if error != Code::Success {
                    return neg(error);
                }
                Label::DocumentEnd
            }
        };

        loop {
            label = match label {
                Label::ObjectBegin | Label::ObjectKey => {
                    if label == Label::ObjectBegin {
                        // object_begin
                        d += 1;
                        if d >= DEFAULT_MAX_DEPTH {
                            return neg(Code::DepthError);
                        }
                        *container.add(d as usize) = w.open(sink, OBJECT, vf, w.at(v));
                        *object.add(d as usize) = true;
                        vf = VALUE;
                        v = buf.add(take!() as usize);
                        if *v != b'"' {
                            return neg(Code::TapeError);
                        }
                    }
                    loop {
                        // object_key
                        *pair.add(d as usize) = w.open(sink, PAIR, NO_FIELD, w.at(v));
                        if w.string(sink, v, KEY) == 0 {
                            core::hint::cold_path();
                            return neg(Code::StringError);
                        }
                        if *buf.add(take!() as usize) != b':' {
                            core::hint::cold_path();
                            return neg(Code::TapeError);
                        }
                        v = buf.add(take!() as usize);
                        match *v {
                            b'{' => {
                                if *buf.add(*next as usize) != b'}' {
                                    break Label::ObjectBegin;
                                }
                                empty!(OBJECT, VALUE);
                                w.close(*pair.add(d as usize), end);
                            }
                            b'[' => {
                                if *buf.add(*next as usize) != b']' {
                                    break Label::ArrayBegin;
                                }
                                empty!(ARRAY, VALUE);
                                w.close(*pair.add(d as usize), end);
                            }
                            _ => {
                                end = w.primitive(sink, v, VALUE, &mut error);
                                if end == 0 {
                                    core::hint::cold_path();
                                    return neg(error);
                                }
                                w.close(*pair.add(d as usize), end);
                            }
                        }
                        // object_continue
                        match *buf.add(take!() as usize) {
                            b',' => {
                                v = buf.add(take!() as usize);
                                if *v != b'"' {
                                    core::hint::cold_path();
                                    return neg(Code::TapeError);
                                }
                            }
                            b'}' => break Label::ScopeEnd,
                            _ => return neg(Code::TapeError),
                        }
                    }
                }
                Label::ArrayBegin | Label::ArrayValue => {
                    if label == Label::ArrayBegin {
                        // array_begin
                        d += 1;
                        if d >= DEFAULT_MAX_DEPTH {
                            return neg(Code::DepthError);
                        }
                        *container.add(d as usize) = w.open(sink, ARRAY, vf, w.at(v));
                        *object.add(d as usize) = false;
                        vf = NO_FIELD;
                    }
                    loop {
                        // array_value
                        v = buf.add(take!() as usize);
                        match *v {
                            b'{' => {
                                if *buf.add(*next as usize) != b'}' {
                                    break Label::ObjectBegin;
                                }
                                empty!(OBJECT, NO_FIELD);
                            }
                            b'[' => {
                                if *buf.add(*next as usize) != b']' {
                                    break Label::ArrayBegin;
                                }
                                empty!(ARRAY, NO_FIELD);
                            }
                            _ => {
                                if w.primitive(sink, v, NO_FIELD, &mut error) == 0 {
                                    core::hint::cold_path();
                                    return neg(error);
                                }
                            }
                        }
                        // array_continue
                        match *buf.add(take!() as usize) {
                            b',' => {}
                            b']' => break Label::ScopeEnd,
                            _ => return neg(Code::TapeError),
                        }
                    }
                }
                Label::ScopeEnd => loop {
                    // scope_end: the container's closer is the structural just read; the pair it is
                    // the value of ends with it.
                    end = *next.sub(1) + shift + 1;
                    w.close(*container.add(d as usize), end);
                    d -= 1;
                    if d == 0 {
                        break Label::DocumentEnd;
                    }
                    if *object.add(d as usize) {
                        vf = VALUE;
                        w.close(*pair.add(d as usize), end);
                        // object_continue
                        match *buf.add(take!() as usize) {
                            b',' => {
                                v = buf.add(take!() as usize);
                                if *v != b'"' {
                                    core::hint::cold_path();
                                    return neg(Code::TapeError);
                                }
                                break Label::ObjectKey;
                            }
                            b'}' => {}
                            _ => return neg(Code::TapeError),
                        }
                    } else {
                        vf = NO_FIELD;
                        // array_continue
                        match *buf.add(take!() as usize) {
                            b',' => break Label::ArrayValue,
                            b']' => {}
                            _ => return neg(Code::TapeError),
                        }
                    }
                },
                Label::DocumentEnd => break,
            };
        }

        // More than one value, or bytes after it.
        if next != index.add(count as usize) {
            return neg(Code::TapeError);
        }
        w.close(0, len);
        for c in 0..5 {
            sink.col[c].set_len(column_bytes(c, w.n, &widths));
        }
        for c in 0..3 {
            sink.width[c] = widths[c] as u8;
        }
        (if SZ::WIDTH == 2 && w.high_size > 0xffff { 1 } else { 0 })
            | (if OF::WIDTH == 2 && w.high_off > 0xffff { 2 } else { 0 })
            | (if TL::WIDTH == 2 && w.high_tail > 0xffff { 4 } else { 0 })
            | (if SZ::WIDTH == 4 && w.high_size <= 0xffff { 8 } else { 0 })
    }
}
