//! simdjson's arm64 stage 1 (`json_structural_indexer::index<64>`), translated: each struct and
//! function is simdjson's, in its order, with its name. The source is simdjson 5.0.5's
//! amalgamation (`native/vendor/simdjson-5.0.5/simdjson.cpp`); each item names the file it is from.
//!
//! Every `simdjson_inline` is `#[inline(always)]`, and `index` is one function, as simdjson's
//! `dom_parser_implementation::stage1` (`simdjson_flatten`) is.

use core::arch::aarch64::*;

use crate::Code;

// ---------------------------------------------------------------------------------------------
// simdjson/arm64/simd.h: `simd8x64<uint8_t>`, four vectors, and its bitmasks.

/// `simd8x64<T>`: a 64-byte block as four vectors.
#[derive(Clone, Copy)]
pub(crate) struct Simd8x64 {
    pub(crate) chunks: [uint8x16_t; 4],
}

const BIT_MASK: [u8; 16] = [0x01, 0x02, 0x4, 0x8, 0x10, 0x20, 0x40, 0x80, 0x01, 0x02, 0x4, 0x8, 0x10, 0x20, 0x40, 0x80];

impl Simd8x64 {
    /// `simd8x64(const T ptr[64])`.
    #[inline(always)]
    pub(crate) unsafe fn load(ptr: *const u8) -> Self {
        // SAFETY: the caller's block has 64 bytes.
        unsafe { Simd8x64 { chunks: [vld1q_u8(ptr), vld1q_u8(ptr.add(16)), vld1q_u8(ptr.add(32)), vld1q_u8(ptr.add(48))] } }
    }

    /// `simd8x64<bool>::to_bitmask`.
    #[inline(always)]
    pub(crate) fn to_bitmask(c: [uint8x16_t; 4]) -> u64 {
        // SAFETY: NEON, a feature of every arm64 target.
        unsafe {
            let bit_mask = vld1q_u8(BIT_MASK.as_ptr());
            let mut sum0 = vpaddq_u8(vandq_u8(c[0], bit_mask), vandq_u8(c[1], bit_mask));
            let sum1 = vpaddq_u8(vandq_u8(c[2], bit_mask), vandq_u8(c[3], bit_mask));
            sum0 = vpaddq_u8(sum0, sum1);
            sum0 = vpaddq_u8(sum0, sum0);
            vgetq_lane_u64::<0>(vreinterpretq_u64_u8(sum0))
        }
    }

    /// `eq`.
    #[inline(always)]
    pub(crate) fn eq(&self, m: u8) -> u64 {
        // SAFETY: NEON.
        unsafe {
            let mask = vmovq_n_u8(m);
            Self::to_bitmask([vceqq_u8(self.chunks[0], mask), vceqq_u8(self.chunks[1], mask), vceqq_u8(self.chunks[2], mask), vceqq_u8(self.chunks[3], mask)])
        }
    }

    /// `lteq`.
    #[inline(always)]
    pub(crate) fn lteq(&self, m: u8) -> u64 {
        // SAFETY: NEON.
        unsafe {
            let mask = vmovq_n_u8(m);
            Self::to_bitmask([vcleq_u8(self.chunks[0], mask), vcleq_u8(self.chunks[1], mask), vcleq_u8(self.chunks[2], mask), vcleq_u8(self.chunks[3], mask)])
        }
    }

    /// `reduce_or`.
    #[inline(always)]
    fn reduce_or(&self) -> uint8x16_t {
        // SAFETY: NEON.
        unsafe { vorrq_u8(vorrq_u8(self.chunks[0], self.chunks[1]), vorrq_u8(self.chunks[2], self.chunks[3])) }
    }
}

// ---------------------------------------------------------------------------------------------
// simdjson/arm64/bitmanipulation.h and bitmask.h.

/// `prefix_xor`: arm64's is six shift-XOR rounds (simdjson leaves `vmull_p64` out: "We could do
/// this with PMULL, but it is apparently slow"). `PMULL` makes it the multiply simdjson left out,
/// `vmull_p64(-1, bitmask)`, to measure the two against each other (`tests::prefix_xor_forms`).
/// Stage 1's forms of the string's toggle (`index::<FORM>`): simdjson's rounds, the multiply of
/// the unescaped quotes (made in the general registers), and the multiply of the quotes as the
/// vector registers hold them, the escaped ones taken back out.
pub(crate) const ROUNDS: u8 = 0;
pub(crate) const PMULL: u8 = 1;
pub(crate) const PMULL_RAW: u8 = 2;

#[inline(always)]
pub(crate) fn prefix_xor<const PMULL: bool>(mut bitmask: u64) -> u64 {
    if PMULL {
        // SAFETY: the caller checked `pmull` (`stage1_form`); every Apple core has it.
        return unsafe { vmull_p64(u64::MAX, bitmask) } as u64;
    }
    bitmask ^= bitmask << 1;
    bitmask ^= bitmask << 2;
    bitmask ^= bitmask << 4;
    bitmask ^= bitmask << 8;
    bitmask ^= bitmask << 16;
    bitmask ^= bitmask << 32;
    bitmask
}

/// `reverse_bits`: `rbit`.
#[inline(always)]
fn reverse_bits(input_num: u64) -> u64 {
    input_num.reverse_bits()
}

/// `leading_zeroes`: `clz`.
#[inline(always)]
fn leading_zeroes(input_num: u64) -> u32 {
    input_num.leading_zeros()
}

/// `zero_leading_bit`. Where `leading_zeroes` is 64 the result is never used (simdjson's comment);
/// `wrapping_shr` is the shift arm64's `lsr` makes.
#[inline(always)]
fn zero_leading_bit(rev_bits: u64, leading_zeroes: u32) -> u64 {
    rev_bits ^ 0x8000000000000000u64.wrapping_shr(leading_zeroes)
}

// ---------------------------------------------------------------------------------------------
// The arm64 implementation's `json_character_block::classify`.

/// `json_character_block`: white space and operators (`[]{},:`).
#[derive(Clone, Copy)]
struct JsonCharacterBlock {
    whitespace: u64,
    op: u64,
}

impl JsonCharacterBlock {
    #[inline(always)]
    fn scalar(&self) -> u64 {
        !(self.op | self.whitespace)
    }

    /// `classify`: an operator is a byte equal to its entry in `op_table`, indexed by
    /// `(byte + 3) >> 4`; white space is a space or a byte whose `ws_table` entry (`tbx`, so a byte
    /// past 15 keeps the space test) is set.
    #[inline(always)]
    fn classify(input: &Simd8x64) -> Self {
        const OP_TABLE: [u8; 16] = [0xff, 0, b',', b':', 0, b'[', b']', b'{', b'}', 0, 0, 0, 0, 0, 0, 0];
        const WS_TABLE: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0, 0, 0xff, 0, 0];
        // SAFETY: NEON.
        unsafe {
            let op_table = vld1q_u8(OP_TABLE.as_ptr());
            let ws_table = vld1q_u8(WS_TABLE.as_ptr());
            let [d0_0, d0_1, d0_2, d0_3] = input.chunks;
            let three = vdupq_n_u8(3);
            let match_op_0 = vceqq_u8(vqtbl1q_u8(op_table, vshrq_n_u8::<4>(vaddq_u8(d0_0, three))), d0_0);
            let match_op_1 = vceqq_u8(vqtbl1q_u8(op_table, vshrq_n_u8::<4>(vaddq_u8(d0_1, three))), d0_1);
            let match_op_2 = vceqq_u8(vqtbl1q_u8(op_table, vshrq_n_u8::<4>(vaddq_u8(d0_2, three))), d0_2);
            let match_op_3 = vceqq_u8(vqtbl1q_u8(op_table, vshrq_n_u8::<4>(vaddq_u8(d0_3, three))), d0_3);
            let space = vdupq_n_u8(b' ');
            let match_ws_0 = vqtbx1q_u8(vceqq_u8(d0_0, space), ws_table, d0_0);
            let match_ws_1 = vqtbx1q_u8(vceqq_u8(d0_1, space), ws_table, d0_1);
            let match_ws_2 = vqtbx1q_u8(vceqq_u8(d0_2, space), ws_table, d0_2);
            let match_ws_3 = vqtbx1q_u8(vceqq_u8(d0_3, space), ws_table, d0_3);
            let bit_mask = vld1q_u8(BIT_MASK.as_ptr());
            let mut op_sum0 = vpaddq_u8(vandq_u8(match_op_0, bit_mask), vandq_u8(match_op_1, bit_mask));
            let mut ws_sum0 = vpaddq_u8(vandq_u8(match_ws_0, bit_mask), vandq_u8(match_ws_1, bit_mask));
            let op_sum1 = vpaddq_u8(vandq_u8(match_op_2, bit_mask), vandq_u8(match_op_3, bit_mask));
            let ws_sum1 = vpaddq_u8(vandq_u8(match_ws_2, bit_mask), vandq_u8(match_ws_3, bit_mask));
            op_sum0 = vpaddq_u8(op_sum0, op_sum1);
            ws_sum0 = vpaddq_u8(ws_sum0, ws_sum1);
            op_sum0 = vpaddq_u8(op_sum0, op_sum0);
            ws_sum0 = vpaddq_u8(ws_sum0, ws_sum0);
            let op = vgetq_lane_u64::<0>(vreinterpretq_u64_u8(op_sum0));
            let whitespace = vgetq_lane_u64::<0>(vreinterpretq_u64_u8(ws_sum0));
            JsonCharacterBlock { whitespace, op }
        }
    }
}

/// `is_ascii`.
#[inline(always)]
fn is_ascii(input: &Simd8x64) -> bool {
    // SAFETY: NEON.
    unsafe { vmaxvq_u8(input.reduce_or()) < 0x80 }
}

/// `must_be_2_3_continuation`.
#[inline(always)]
fn must_be_2_3_continuation(prev2: uint8x16_t, prev3: uint8x16_t) -> uint8x16_t {
    // SAFETY: NEON.
    unsafe {
        let is_third_byte = vqsubq_u8(prev2, vmovq_n_u8(0xe0 - 0x80));
        let is_fourth_byte = vqsubq_u8(prev3, vmovq_n_u8(0xf0 - 0x80));
        vorrq_u8(is_third_byte, is_fourth_byte)
    }
}

// ---------------------------------------------------------------------------------------------
// generic/stage1/utf8_lookup4_algorithm.h.

const TOO_SHORT: u8 = 1 << 0;
const TOO_LONG: u8 = 1 << 1;
const OVERLONG_3: u8 = 1 << 2;
const SURROGATE: u8 = 1 << 4;
const OVERLONG_2: u8 = 1 << 5;
const TWO_CONTS: u8 = 1 << 7;
const TOO_LARGE: u8 = 1 << 3;
const TOO_LARGE_1000: u8 = 1 << 6;
const OVERLONG_4: u8 = 1 << 6;
const CARRY: u8 = TOO_SHORT | TOO_LONG | TWO_CONTS;

const BYTE_1_HIGH: [u8; 16] = [
    TOO_LONG,
    TOO_LONG,
    TOO_LONG,
    TOO_LONG,
    TOO_LONG,
    TOO_LONG,
    TOO_LONG,
    TOO_LONG,
    TWO_CONTS,
    TWO_CONTS,
    TWO_CONTS,
    TWO_CONTS,
    TOO_SHORT | OVERLONG_2,
    TOO_SHORT,
    TOO_SHORT | OVERLONG_3 | SURROGATE,
    TOO_SHORT | TOO_LARGE | TOO_LARGE_1000 | OVERLONG_4,
];
const BYTE_1_LOW: [u8; 16] = [
    CARRY | OVERLONG_3 | OVERLONG_2 | OVERLONG_4,
    CARRY | OVERLONG_2,
    CARRY,
    CARRY,
    CARRY | TOO_LARGE,
    CARRY | TOO_LARGE | TOO_LARGE_1000,
    CARRY | TOO_LARGE | TOO_LARGE_1000,
    CARRY | TOO_LARGE | TOO_LARGE_1000,
    CARRY | TOO_LARGE | TOO_LARGE_1000,
    CARRY | TOO_LARGE | TOO_LARGE_1000,
    CARRY | TOO_LARGE | TOO_LARGE_1000,
    CARRY | TOO_LARGE | TOO_LARGE_1000,
    CARRY | TOO_LARGE | TOO_LARGE_1000,
    CARRY | TOO_LARGE | TOO_LARGE_1000 | SURROGATE,
    CARRY | TOO_LARGE | TOO_LARGE_1000,
    CARRY | TOO_LARGE | TOO_LARGE_1000,
];
const BYTE_2_HIGH: [u8; 16] = [
    TOO_SHORT,
    TOO_SHORT,
    TOO_SHORT,
    TOO_SHORT,
    TOO_SHORT,
    TOO_SHORT,
    TOO_SHORT,
    TOO_SHORT,
    TOO_LONG | OVERLONG_2 | TWO_CONTS | OVERLONG_3 | TOO_LARGE_1000 | OVERLONG_4,
    TOO_LONG | OVERLONG_2 | TWO_CONTS | OVERLONG_3 | TOO_LARGE,
    TOO_LONG | OVERLONG_2 | TWO_CONTS | SURROGATE | TOO_LARGE,
    TOO_LONG | OVERLONG_2 | TWO_CONTS | SURROGATE | TOO_LARGE,
    TOO_SHORT,
    TOO_SHORT,
    TOO_SHORT,
    TOO_SHORT,
];
/// `is_incomplete`'s `max_value`: the last 16 of `max_array`.
const MAX_VALUE: [u8; 16] = [255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0xf0 - 1, 0xe0 - 1, 0xc0 - 1];

/// `check_special_cases`.
#[inline(always)]
fn check_special_cases(input: uint8x16_t, prev1: uint8x16_t) -> uint8x16_t {
    // SAFETY: NEON.
    unsafe {
        let byte_1_high = vqtbl1q_u8(vld1q_u8(BYTE_1_HIGH.as_ptr()), vshrq_n_u8::<4>(prev1));
        let byte_1_low = vqtbl1q_u8(vld1q_u8(BYTE_1_LOW.as_ptr()), vandq_u8(prev1, vmovq_n_u8(0x0f)));
        let byte_2_high = vqtbl1q_u8(vld1q_u8(BYTE_2_HIGH.as_ptr()), vshrq_n_u8::<4>(input));
        vandq_u8(vandq_u8(byte_1_high, byte_1_low), byte_2_high)
    }
}

/// `check_multibyte_lengths`.
#[inline(always)]
fn check_multibyte_lengths(input: uint8x16_t, prev_input: uint8x16_t, sc: uint8x16_t) -> uint8x16_t {
    // SAFETY: NEON.
    unsafe {
        let prev2 = vextq_u8::<14>(prev_input, input);
        let prev3 = vextq_u8::<13>(prev_input, input);
        let must23 = must_be_2_3_continuation(prev2, prev3);
        let must23_80 = vandq_u8(must23, vmovq_n_u8(0x80));
        veorq_u8(must23_80, sc)
    }
}

/// `is_incomplete`.
#[inline(always)]
fn is_incomplete(input: uint8x16_t) -> uint8x16_t {
    // SAFETY: NEON.
    unsafe { vcgtq_u8(input, vld1q_u8(MAX_VALUE.as_ptr())) }
}

/// `utf8_checker`.
pub(crate) struct Utf8Checker {
    error: uint8x16_t,
    prev_input_block: uint8x16_t,
    prev_incomplete: uint8x16_t,
}

impl Utf8Checker {
    #[inline(always)]
    pub(crate) fn new() -> Self {
        // SAFETY: NEON.
        unsafe { Utf8Checker { error: vdupq_n_u8(0), prev_input_block: vdupq_n_u8(0), prev_incomplete: vdupq_n_u8(0) } }
    }

    #[inline(always)]
    fn check_utf8_bytes(&mut self, input: uint8x16_t, prev_input: uint8x16_t) {
        // SAFETY: NEON.
        unsafe {
            let prev1 = vextq_u8::<15>(prev_input, input);
            let sc = check_special_cases(input, prev1);
            self.error = vorrq_u8(self.error, check_multibyte_lengths(input, prev_input, sc));
        }
    }

    #[inline(always)]
    pub(crate) fn check_eof(&mut self) {
        // SAFETY: NEON.
        unsafe { self.error = vorrq_u8(self.error, self.prev_incomplete) }
    }

    #[inline(always)]
    pub(crate) fn check_next_input(&mut self, input: &Simd8x64) {
        if is_ascii(input) {
            // SAFETY: NEON.
            unsafe { self.error = vorrq_u8(self.error, self.prev_incomplete) }
        } else {
            let [c0, c1, c2, c3] = input.chunks;
            self.check_utf8_bytes(c0, self.prev_input_block);
            self.check_utf8_bytes(c1, c0);
            self.check_utf8_bytes(c2, c1);
            self.check_utf8_bytes(c3, c2);
            self.prev_incomplete = is_incomplete(c3);
            self.prev_input_block = c3;
        }
    }

    #[inline(always)]
    pub(crate) fn errors(&self) -> Code {
        // SAFETY: NEON.
        if unsafe { vmaxvq_u32(vreinterpretq_u32_u8(self.error)) } != 0 { Code::Utf8Error } else { Code::Success }
    }
}

// ---------------------------------------------------------------------------------------------
// generic/stage1/json_escape_scanner.h (arm64: `SIMDJSON_SKIP_BACKSLASH_SHORT_CIRCUIT`, so no
// branch on a block with no backslash).

const ODD_BITS: u64 = 0xAAAAAAAAAAAAAAAA;

/// `json_escape_scanner`.
struct JsonEscapeScanner {
    next_is_escaped: u64,
}

impl JsonEscapeScanner {
    /// `next`: the escaped characters (the escape characters are not used by stage 1).
    #[inline(always)]
    fn next(&mut self, backslash: u64) -> u64 {
        let escape_and_terminal_code = Self::next_escape_and_terminal_code(backslash & !self.next_is_escaped);
        let escaped = escape_and_terminal_code ^ (backslash | self.next_is_escaped);
        let escape = escape_and_terminal_code & backslash;
        self.next_is_escaped = escape >> 63;
        escaped
    }

    #[inline(always)]
    fn next_escape_and_terminal_code(potential_escape: u64) -> u64 {
        let maybe_escaped = potential_escape << 1;
        let maybe_escaped_and_odd_bits = maybe_escaped | ODD_BITS;
        let even_series_codes_and_odd_bits = maybe_escaped_and_odd_bits.wrapping_sub(potential_escape);
        even_series_codes_and_odd_bits ^ ODD_BITS
    }
}

// ---------------------------------------------------------------------------------------------
// generic/stage1/json_string_scanner.h.

/// `json_string_block`.
#[derive(Clone, Copy)]
pub(crate) struct JsonStringBlock {
    quote: u64,
    in_string: u64,
}

impl JsonStringBlock {
    #[inline(always)]
    pub(crate) fn non_quote_inside_string(&self, mask: u64) -> u64 {
        mask & self.in_string
    }
    #[inline(always)]
    fn string_tail(&self) -> u64 {
        self.in_string ^ self.quote
    }
}

/// `json_string_scanner`.
struct JsonStringScanner {
    escape_scanner: JsonEscapeScanner,
    prev_in_string: u64,
}

impl JsonStringScanner {
    #[inline(always)]
    fn next<const FORM: u8>(&mut self, input: &Simd8x64) -> JsonStringBlock {
        let backslash = input.eq(b'\\');
        let escaped = self.escape_scanner.next(backslash);
        let raw = input.eq(b'"');
        let quote = raw & !escaped;
        let toggle = match FORM {
            ROUNDS => prefix_xor::<false>(quote),
            PMULL => prefix_xor::<true>(quote),
            _ => {
                // The multiply of the quotes as they come from the vector registers, escaped or
                // not: prefix XOR is linear, so the escaped ones' part is taken back out, where
                // there are any.
                let mut t = prefix_xor::<true>(raw);
                let wrong = raw & escaped;
                if wrong != 0 {
                    core::hint::cold_path();
                    t ^= prefix_xor::<false>(wrong);
                }
                t
            }
        };
        let in_string = toggle ^ self.prev_in_string;
        self.prev_in_string = ((in_string as i64) >> 63) as u64;
        JsonStringBlock { quote, in_string }
    }

    #[inline(always)]
    fn finish(&self) -> Code {
        if self.prev_in_string != 0 { Code::UnclosedString } else { Code::Success }
    }
}

// ---------------------------------------------------------------------------------------------
// generic/stage1/json_scanner.h.

/// `json_block`.
#[derive(Clone, Copy)]
pub(crate) struct JsonBlock {
    pub(crate) string: JsonStringBlock,
    characters: JsonCharacterBlock,
    follows_potential_nonquote_scalar: u64,
}

impl JsonBlock {
    /// `structural_start`.
    #[inline(always)]
    pub(crate) fn structural_start(&self) -> u64 {
        self.potential_structural_start() & !self.string.string_tail()
    }
    #[inline(always)]
    fn potential_structural_start(&self) -> u64 {
        self.characters.op | self.potential_scalar_start()
    }
    #[inline(always)]
    fn potential_scalar_start(&self) -> u64 {
        self.characters.scalar() & !self.follows_potential_nonquote_scalar
    }
}

/// `json_scanner`.
pub(crate) struct JsonScanner {
    prev_scalar: u64,
    string_scanner: JsonStringScanner,
}

/// `follows`.
#[inline(always)]
fn follows(m: u64, overflow: &mut u64) -> u64 {
    let result = (m << 1) | *overflow;
    *overflow = m >> 63;
    result
}

impl JsonScanner {
    pub(crate) fn new() -> Self {
        JsonScanner { prev_scalar: 0, string_scanner: JsonStringScanner { escape_scanner: JsonEscapeScanner { next_is_escaped: 0 }, prev_in_string: 0 } }
    }

    /// `next`.
    #[inline(always)]
    pub(crate) fn next<const FORM: u8>(&mut self, input: &Simd8x64) -> JsonBlock {
        let strings = self.string_scanner.next::<FORM>(input);
        let characters = JsonCharacterBlock::classify(input);
        let nonquote_scalar = characters.scalar() & !strings.quote;
        let follows_nonquote_scalar = follows(nonquote_scalar, &mut self.prev_scalar);
        JsonBlock { string: strings, characters, follows_potential_nonquote_scalar: follows_nonquote_scalar }
    }

    /// `finish`.
    #[inline(always)]
    pub(crate) fn finish(&self) -> Code {
        self.string_scanner.finish()
    }
}

// ---------------------------------------------------------------------------------------------
// generic/stage1/json_structural_indexer.h's `bit_indexer`, with the arm64 implementation's own
// `write` (`SIMDJSON_GENERIC_JSON_STRUCTURAL_INDEXER_CUSTOM_BIT_INDEXER`): four indexes
// unconditionally, then by twos up to 24, then one at a time. `SIMDJSON_PREFER_REVERSE_BITS`: the
// bits reversed once, each index a `clz`.

/// `bit_indexer`.
pub(crate) struct BitIndexer {
    pub(crate) tail: *mut u32,
}

impl BitIndexer {
    /// `write_index`.
    #[inline(always)]
    fn write_index(&self, idx: u32, rev_bits: &mut u64, i: usize) {
        let lz = leading_zeroes(*rev_bits);
        // SAFETY: the index buffer has room for every block's 64 and the slack after them
        // (`Parser::ready`).
        unsafe { *self.tail.add(i) = idx.wrapping_add(lz) };
        *rev_bits = zero_leading_bit(*rev_bits, lz);
    }

    /// `write`.
    #[inline(always)]
    pub(crate) fn write(&mut self, idx: u32, bits: u64) {
        if bits == 0 {
            return;
        }
        let cnt = bits.count_ones() as usize;
        let mut bits = reverse_bits(bits);
        // `write_indexes<0, 4>`.
        self.write_index(idx, &mut bits, 0);
        self.write_index(idx, &mut bits, 1);
        self.write_index(idx, &mut bits, 2);
        self.write_index(idx, &mut bits, 3);
        if 4 < cnt {
            // `write_indexes_stepped<4, 24, 2>`: two, then two more while the count goes past them,
            // each test nested in the last, as the template expands.
            macro_rules! stepped {
                ($i:expr) => {
                    self.write_index(idx, &mut bits, $i);
                    self.write_index(idx, &mut bits, $i + 1);
                };
                ($i:expr, $($rest:expr),+) => {
                    self.write_index(idx, &mut bits, $i);
                    self.write_index(idx, &mut bits, $i + 1);
                    if $i + 2 < cnt {
                        stepped!($($rest),+);
                    }
                };
            }
            stepped!(4, 6, 8, 10, 12, 14, 16, 18, 20, 22);
        }
        if 24 < cnt {
            for i in 24..cnt {
                self.write_index(idx, &mut bits, i);
            }
        }
        // SAFETY: within the buffer, as each write.
        self.tail = unsafe { self.tail.add(cnt) };
    }
}

// ---------------------------------------------------------------------------------------------
// generic/stage1/json_structural_indexer.h: `index<64>`, `step<64>`, `next` and `finish`, for the
// whole-document mode (`stage1_mode::regular`).

/// The structural index of `buf[..len]` (with simdjson's 64 bytes of padding after it) into
/// `indexes`, which has room for `len` rounded up to 64, and 2 + 7 more; its count, or simdjson's
/// error. As simdjson, the three entries after the count are `len`, `len` and 0.
///
/// # Safety
/// `buf` has `len + 64` bytes; `indexes` has the room above.
#[inline(never)]
pub(crate) unsafe fn index<const FORM: u8>(buf: *const u8, len: usize, indexes: *mut u32) -> Result<u32, Code> {
    if len == 0 {
        return Err(Code::Empty);
    }
    // `buf_block_reader<64>`.
    let lenminusstep = if len < 64 { 0 } else { len - 64 };
    let mut idx = 0usize;
    // `json_structural_indexer`.
    let mut scanner = JsonScanner::new();
    let mut checker = Utf8Checker::new();
    let mut indexer = BitIndexer { tail: indexes };
    let mut prev_structurals = 0u64;
    let mut unescaped_chars_error = 0u64;
    // `step<64>` and `next`.
    macro_rules! step {
        ($block:expr) => {{
            // SAFETY: a full block, or the remainder's copy: 64 bytes.
            let in_1 = unsafe { Simd8x64::load($block) };
            let block_1 = scanner.next::<FORM>(&in_1);
            let unescaped = in_1.lteq(0x1F);
            checker.check_next_input(&in_1);
            // Output *last* iteration's structurals to the parser.
            indexer.write((idx as u32).wrapping_sub(64), prev_structurals);
            prev_structurals = block_1.structural_start();
            unescaped_chars_error |= block_1.string.non_quote_inside_string(unescaped);
            idx += 64;
        }};
    }
    while idx < lenminusstep {
        // SAFETY: `idx + 64 <= len`.
        let block = unsafe { buf.add(idx) };
        step!(block);
    }
    // `get_remainder`: the last block, padded with spaces.
    let mut block = [0x20u8; 64];
    if len == idx {
        return Err(Code::UnexpectedError);
    }
    // SAFETY: `len - idx` bytes of `buf` from `idx`, at most 64.
    unsafe { core::ptr::copy_nonoverlapping(buf.add(idx), block.as_mut_ptr(), len - idx) };
    step!(block.as_ptr());
    // `finish`.
    indexer.write((idx as u32).wrapping_sub(64), prev_structurals);
    let error = scanner.finish();
    // SAFETY: within the buffer.
    let n = unsafe { indexer.tail.offset_from(indexes) } as u32;
    unsafe {
        *indexes.add(n as usize) = len as u32;
        *indexes.add(n as usize + 1) = len as u32;
        *indexes.add(n as usize + 2) = 0;
    }
    if error != Code::Success {
        return Err(error);
    }
    if unescaped_chars_error != 0 {
        return Err(Code::UnescapedChars);
    }
    if n == 0 {
        return Err(Code::Empty);
    }
    // SAFETY: `n > 0`.
    if unsafe { *indexes.add(n as usize - 1) } as usize > len {
        return Err(Code::UnexpectedError);
    }
    checker.check_eof();
    match checker.errors() {
        Code::Success => Ok(n),
        e => Err(e),
    }
}
