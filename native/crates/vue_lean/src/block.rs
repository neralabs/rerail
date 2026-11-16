//! The text's byte classes, 64 bytes a step (stage 1, as in `rerail-html-lean`): a mask a class,
//! one bit a byte, from simdjson's nibble tables (`LO`, `HI`) and compares over `rerail_simd8`'s
//! operations, compiled for each backend. The parser's scans (stage 2) take a run's end, or the
//! next byte of a class, as the lowest bit of a block's mask.
//!
//! Past the text's end the runs (blanks, names) stop and the stops are all set, so a scan never
//! reads past the last block; its result is at most the text's length.

use rerail_simd8::Simd8x64;

/// A block's masks, by index.
const TEXT: usize = 0; // a text's stops (the scanner's cases): '<', '{', '}', '\n', '\r'
const BLANK: usize = 1; // ASCII white space: ' ', '\t', '\n', '\v', '\f', '\r'
const NAME: usize = 2; // a tag name's bytes: [A-Za-z0-9:-]
const ANAME: usize = 3; // an attribute name's stops: '<', '>', '"', '\'', '/', '=', white space
const AVALUE: usize = 4; // an unquoted attribute value's stops: the same but '/'
const DVALUE: usize = 5; // a directive name's or value's stops: an attribute name's, ':' and '.'
const DINNER: usize = 6; // a dynamic argument's stops: an attribute name's and ']'
const MODIFIER: usize = 7; // a modifier's stops: an attribute name's and '.'
const LT: usize = 8; // '<'
const GT: usize = 9; // '>'
const DQUOTE: usize = 10; // '"'
const QUOTE: usize = 11; // '\''
const HIGH: usize = 12; // a byte past ASCII
const MASKS: usize = 13;

/// The masks set past the text's end (the stops); the runs are clear there.
const STOPS: [usize; 11] = [TEXT, ANAME, AVALUE, DVALUE, DINNER, MODIFIER, LT, GT, DQUOTE, QUOTE, HIGH];

// The nibble tables' bits: each is the product of a set of high nibbles and a set of low ones.
const C_AO: u8 = 1; // high 4 or 6, low 1..=15: A-O, a-o
const C_PZ: u8 = 2; // high 5 or 7, low 0..=10: P-Z, p-z
const C_DIGIT: u8 = 4; // high 3, low 0..=9
const C_DASH: u8 = 8; // high 2, low 13
const C_COLON: u8 = 16; // high 3, low 10
const C_SPACE: u8 = 32; // high 2, low 0
const C_CONTROL: u8 = 64; // high 0, low 9..=13
const C_NAME: u8 = C_AO | C_PZ | C_DIGIT | C_DASH | C_COLON;
const C_BLANK: u8 = C_SPACE | C_CONTROL;

static LO: [u8; 16] = {
    let mut t = [0u8; 16];
    let mut n = 0;
    while n < 16 {
        let mut b = 0;
        if n >= 1 {
            b |= C_AO;
        }
        if n <= 10 {
            b |= C_PZ;
        }
        if n <= 9 {
            b |= C_DIGIT;
        }
        if n == 13 {
            b |= C_DASH;
        }
        if n == 10 {
            b |= C_COLON;
        }
        if n == 0 {
            b |= C_SPACE;
        }
        if n >= 9 && n <= 13 {
            b |= C_CONTROL;
        }
        t[n] = b;
        n += 1;
    }
    t
};

static HI: [u8; 16] = {
    let mut t = [0u8; 16];
    t[0] = C_CONTROL;
    t[2] = C_SPACE | C_DASH;
    t[3] = C_DIGIT | C_COLON;
    t[4] = C_AO;
    t[5] = C_PZ;
    t[6] = C_AO;
    t[7] = C_PZ;
    t
};

type Masks = [u64; MASKS];

/// One block's masks.
#[inline(always)]
unsafe fn masks<S: Simd8x64>(b: &[u8; 64]) -> Masks {
    unsafe {
        let v = S::load(b);
        let c = S::classify(v, &LO, &HI);
        let is = |x: u8| S::eq(v, S::splat(x));
        let blank_v = S::test(c, S::splat(C_BLANK));
        let blank = S::to_bitmask(blank_v);
        let name = S::to_bitmask(S::test(c, S::splat(C_NAME)));
        let (lt, gt) = (is(b'<'), is(b'>'));
        let (dq, q, eq) = (is(b'"'), is(b'\''), is(b'='));
        let (nl, cr) = (S::eq_mask(v, b'\n'), S::eq_mask(v, b'\r'));
        let text = S::to_bitmask(S::or(S::or(lt, is(b'{')), is(b'}'))) | nl | cr;
        let avalue = S::to_bitmask(S::or(S::or(S::or(lt, gt), S::or(dq, q)), S::or(eq, blank_v)));
        let aname = avalue | S::eq_mask(v, b'/');
        let dot = S::eq_mask(v, b'.');
        let dvalue = aname | dot | S::eq_mask(v, b':');
        let dinner = aname | S::eq_mask(v, b']');
        [
            text,
            blank,
            name,
            aname,
            avalue,
            dvalue,
            dinner,
            aname | dot,
            S::to_bitmask(lt),
            S::to_bitmask(gt),
            S::to_bitmask(dq),
            S::to_bitmask(q),
            S::to_bitmask(S::ge(v, 0x80)),
        ]
    }
}

/// Stage 1 over `src`: each block's masks, the last block's padded.
#[inline(always)]
unsafe fn stage1<S: Simd8x64>(src: &[u8]) -> Vec<Masks> {
    let mut out = Vec::with_capacity(src.len().div_ceil(64));
    let mut at = 0;
    while let Some(b) = src.get(at..at + 64) {
        out.push(unsafe { masks::<S>(b.try_into().expect("64 bytes")) });
        at += 64;
    }
    if at < src.len() {
        let mut b = [0u8; 64];
        let n = src.len() - at;
        b[..n].copy_from_slice(&src[at..]);
        let mut m = unsafe { masks::<S>(&b) };
        let past = !0u64 << n;
        m[BLANK] &= !past;
        m[NAME] &= !past;
        for k in STOPS {
            m[k] |= past;
        }
        out.push(m);
    }
    out
}

rerail_simd8::dispatch! {
    fn stage1_best(src: &[u8]) -> Vec<Masks> = stage1, on stage1_on;
}

/// The text's masks (stage 1), for the parser's scans (stage 2).
pub struct Blocks<'s> {
    src: &'s [u8],
    m: Vec<Masks>,
}

impl<'s> Blocks<'s> {
    /// Stage 1 over `src`, on the best backend the CPU runs.
    pub fn of(src: &'s [u8]) -> Self {
        Blocks { src, m: stage1_best(src) }
    }

    /// The first byte from `at` on whose bit in mask `k` is `set`.
    #[inline(always)]
    fn next(&self, at: usize, k: usize, set: bool) -> usize {
        if at >= self.src.len() {
            return self.src.len();
        }
        let flip = if set { 0 } else { !0 };
        let mut b = at / 64;
        let mut m = (self.m[b][k] ^ flip) & (!0u64 << (at % 64));
        loop {
            if m != 0 {
                return (b * 64 + m.trailing_zeros() as usize).min(self.src.len());
            }
            b += 1;
            if b >= self.m.len() {
                return self.src.len();
            }
            m = self.m[b][k] ^ flip;
        }
    }

    /// The first byte from `at` the text scan stops at: `<`, `{`, `}`, a line's end.
    #[inline]
    pub fn text_stop(&self, at: usize) -> usize {
        self.next(at, TEXT, true)
    }

    /// The end of a run of ASCII white space from `at`.
    #[inline]
    pub fn blank_end(&self, at: usize) -> usize {
        self.next(at, BLANK, false)
    }

    /// The end of a run of a tag name's bytes from `at`.
    #[inline]
    pub fn name_end(&self, at: usize) -> usize {
        self.next(at, NAME, false)
    }

    /// The first byte from `at` that ends an attribute name.
    #[inline]
    pub fn attribute_name_end(&self, at: usize) -> usize {
        self.next(at, ANAME, true)
    }

    /// The first byte from `at` that ends an unquoted attribute value.
    #[inline]
    pub fn attribute_value_end(&self, at: usize) -> usize {
        self.next(at, AVALUE, true)
    }

    /// The first byte from `at` that ends a directive's name or value.
    #[inline]
    pub fn directive_end(&self, at: usize) -> usize {
        self.next(at, DVALUE, true)
    }

    /// The first byte from `at` that ends a dynamic argument's inside.
    #[inline]
    pub fn inner_end(&self, at: usize) -> usize {
        self.next(at, DINNER, true)
    }

    /// The first byte from `at` that ends a modifier.
    #[inline]
    pub fn modifier_end(&self, at: usize) -> usize {
        self.next(at, MODIFIER, true)
    }

    /// The first `<` from `at`.
    #[inline]
    pub fn lt(&self, at: usize) -> usize {
        self.next(at, LT, true)
    }

    /// The first `>` from `at`.
    #[inline]
    pub fn gt(&self, at: usize) -> usize {
        self.next(at, GT, true)
    }

    /// The first `quote` (`"` or `'`) from `at`.
    #[inline]
    pub fn quote(&self, at: usize, quote: u8) -> usize {
        self.next(at, if quote == b'"' { DQUOTE } else { QUOTE }, true)
    }

    /// The first byte past ASCII from `at`.
    #[inline]
    pub fn high(&self, at: usize) -> usize {
        self.next(at, HIGH, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The masks one byte at a time.
    fn scalar(src: &[u8]) -> Vec<Masks> {
        let mut out = vec![[0u64; MASKS]; src.len().div_ceil(64)];
        for (b, o) in out.iter_mut().enumerate() {
            for i in 0..64 {
                let at = b * 64 + i;
                let Some(&c) = src.get(at) else {
                    for k in STOPS {
                        o[k] |= 1 << i;
                    }
                    continue;
                };
                let bit = |x: bool| u64::from(x) << i;
                let blank = matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r');
                let aname = blank || matches!(c, b'<' | b'>' | b'"' | b'\'' | b'/' | b'=');
                o[TEXT] |= bit(matches!(c, b'<' | b'{' | b'}' | b'\n' | b'\r'));
                o[BLANK] |= bit(blank);
                o[NAME] |= bit(c.is_ascii_alphanumeric() || c == b'-' || c == b':');
                o[ANAME] |= bit(aname);
                o[AVALUE] |= bit(blank || matches!(c, b'<' | b'>' | b'"' | b'\'' | b'='));
                o[DVALUE] |= bit(aname || c == b':' || c == b'.');
                o[DINNER] |= bit(aname || c == b']');
                o[MODIFIER] |= bit(aname || c == b'.');
                o[LT] |= bit(c == b'<');
                o[GT] |= bit(c == b'>');
                o[DQUOTE] |= bit(c == b'"');
                o[QUOTE] |= bit(c == b'\'');
                o[HIGH] |= bit(c >= 0x80);
            }
        }
        out
    }

    #[test]
    fn every_backend_gives_the_byte_tests_masks() {
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let alphabet = b"aZ_9:-. \t\x0b\n\r\x0c'\"=/<>&!#;{}[]@\x7f";
        for len in [0usize, 1, 63, 64, 65, 200, 1000] {
            for _ in 0..20 {
                let src: Vec<u8> = (0..len)
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        if x.is_multiple_of(3) { (x >> 16) as u8 } else { alphabet[(x >> 8) as usize % alphabet.len()] }
                    })
                    .collect();
                let want = scalar(&src);
                for be in rerail_simd8::Backend::ALL {
                    if let Some(got) = stage1_on(be, &src) {
                        assert_eq!(got, want, "{be:?}, {len} bytes");
                    }
                }
            }
        }
    }

    #[test]
    fn scans_stop_where_they_should() {
        let b = Blocks::of(b"ab c<d");
        assert_eq!(b.text_stop(0), 4);
        assert_eq!(b.name_end(4 + 1), 6);
        assert_eq!(Blocks::of(b"x  \n y").blank_end(1), 5);
        assert_eq!(Blocks::of(b"href=a/b>").attribute_name_end(0), 4);
        assert_eq!(Blocks::of(b"a/b>").attribute_value_end(0), 3);
        assert_eq!(Blocks::of(b"v-on:click.stop").directive_end(2), 4);
        assert_eq!(Blocks::of(b"a.b]").inner_end(0), 3);
        assert_eq!(Blocks::of(b"stop.x").modifier_end(0), 4);
        assert_eq!(Blocks::of("aé".as_bytes()).high(0), 1);
    }
}
