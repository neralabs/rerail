//! A start's other value, derived from the base run's words without running it.
//!
//! A 1-bit carried operation's words from carry in 1 follow from its words from carry in 0 and
//! its inputs, with word operations (the design page, §3):
//!
//! - a toggle: every word flipped (`t₁ = t₀ ^ ~0`), simdjson's `in_string` read from inside;
//! - a fill `fill(s, e)`: every bit before the chunk's first end bit set (`f₁ = f₀ | before(e)`);
//!   from the first end bit on, the two are one;
//! - an escape: the chunk's leading run of backslashes read from an odd offset, the rest as
//!   from 0.
//!
//! Only the operation's own words: what reads them is run again in the operation's cone.

use rerail_compiler::block::carry;

/// A toggle's words from the other carry: each word flipped.
pub fn toggle_other(words: &[u64]) -> Vec<u64> {
    words.iter().map(|&w| !w).collect()
}

/// A fill's words from carry in 1, given its words from carry in 0 and its end words `e`:
/// every bit before the first bit of `e` in the chunk set (`before(e)`, across words).
pub fn fill_other(words: &[u64], e: &[u64]) -> Vec<u64> {
    let mut out = words.to_vec();
    for (i, &end) in e.iter().enumerate() {
        if end == 0 {
            out[i] = u64::MAX;
            continue;
        }
        // The bits below the first end bit: `blsmsk(e) ^ lowest` (`(e ^ (e - 1)) >> 1`).
        out[i] |= (end ^ end.wrapping_sub(1)) >> 1;
        break;
    }
    out
}

/// An escape's words from carry in 1, given the run words: the first word's escapes from carry
/// 1, carried on until a word's carry out agrees with the base's.
pub fn escaped_other(words: &[u64], runs: &[u64]) -> Vec<u64> {
    let mut out = words.to_vec();
    let (mut c1, mut c0) = (1u64, 0u64);
    for (i, &r) in runs.iter().enumerate() {
        if c1 == c0 {
            break;
        }
        let (w1, n1) = carry::escaped(r, c1);
        let (_, n0) = carry::escaped(r, c0);
        out[i] = w1;
        (c1, c0) = (n1, n0);
    }
    out
}
