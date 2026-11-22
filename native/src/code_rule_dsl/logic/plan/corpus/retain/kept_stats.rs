//! A kept relation's statistics across processes (decision 0120 §3).
//!
//! A run keeps a kept relation's statistics measured once (`Measured`, decision 0094 §4), while it is kept as it
//! is. (Format 1 also held statistics kept by delta, decision 0096 step 4, of a relation a form maintained as
//! `r__old`; the forms are gone (0364), and format 2 holds measured ones only.)
//!
//! They were keyed by the engine relation's address, so only a run in the same process found them. A run read from
//! the frames started with none: it measured each kept one again (208 counts and 241 measures, 0.87 s, of a
//! one-line edit of `dl-sub40`).
//!
//! Here they are stored beside the frames, by the name a run binds the relation as and the frame's digest of its
//! rows. Statistics are a function of the rows, and the digest names the rows, so a relation admitted at that
//! digest has those statistics. An entry whose digest no frame has any more is stale and is never matched. They are
//! a part of the relation's frame (`PART`), admitted with its rows, so reading them is no read of its own.
use super::Measured;
use std::collections::{BTreeSet, HashMap};

/// Each relation's statistics by the name it is bound as and its rows' digest.
pub(in crate::code_rule_dsl) type ByDigest = HashMap<(String, u64), Measured>;

/// The frame part a relation's statistics are stored in (its rows are part 0, an input's granules 1 and up, its
/// deltas -1 down to -16: decision 0117 step 4, whose chain reads every part from -k to -1).
pub(super) const PART: i64 = i64::MIN;

/// Whether statistics are kept in the frames: `RERAIL_KEPT_STATS=off` keeps none, as before 0120.
pub(super) fn on() -> bool {
    !std::env::var("RERAIL_KEPT_STATS").is_ok_and(|v| v == "off")
}

/// `bytes` as a frame part's one column of words: the length, then the bytes eight to a word.
pub(super) fn to_words(bytes: &[u8]) -> Vec<i64> {
    let mut out = vec![bytes.len() as i64];
    out.extend(bytes.chunks(8).map(|c| {
        let mut w = [0u8; 8];
        w[..c.len()].copy_from_slice(c);
        i64::from_le_bytes(w)
    }));
    out
}

/// `to_words`' entries back (`decode`) from a part of `n` words, `word(i)` its `i`th read where it lies (decision
/// 0353 step 8: the entry's words are not read into a vector first); `None` if the words are not such a part.
pub(super) fn from_words(n: usize, word: &dyn Fn(usize) -> i64) -> Option<ByDigest> {
    let rest = n.checked_sub(1)?;
    let len = usize::try_from(word(0)).ok().filter(|len| len.div_ceil(8) == rest)?;
    let byte = |i: usize| (word(1 + i / 8) >> (8 * (i % 8))) as u8;
    decode_from(&byte, len)
}

fn int(out: &mut Vec<u8>, v: i64) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn text(out: &mut Vec<u8>, s: &str) {
    int(out, s.len() as i64);
    out.extend_from_slice(s.as_bytes());
}

fn counts(out: &mut Vec<u8>, c: &[(String, i64, i64)]) {
    int(out, c.len() as i64);
    for (s, a, b) in c {
        text(out, s);
        int(out, *a);
        int(out, *b);
    }
}

fn tested(out: &mut Vec<u8>, t: &BTreeSet<(usize, i64)>) {
    int(out, t.len() as i64);
    for (c, v) in t {
        int(out, *c as i64);
        int(out, *v);
    }
}

/// Format 2: a version byte, the entries' count, then each entry's name, digest and measured statistics.
pub(super) fn encode(all: &ByDigest) -> Vec<u8> {
    let mut out = vec![2u8];
    int(&mut out, all.len() as i64);
    for ((name, digest), (cols, c, t)) in all {
        text(&mut out, name);
        out.extend_from_slice(&digest.to_le_bytes());
        int(&mut out, cols.len() as i64);
        for (s, a, b, x, y) in cols {
            text(&mut out, s);
            for v in [a, b, x, y] {
                int(&mut out, *v);
            }
        }
        counts(&mut out, c);
        tested(&mut out, t);
    }
    out
}

/// A blob's bytes read by position (`byte`, `n` of them), where they lie.
struct Reader<'a> {
    byte: &'a dyn Fn(usize) -> u8,
    n: usize,
    at: usize,
}

impl Reader<'_> {
    /// The next `n` bytes' place, past them.
    fn take(&mut self, n: usize) -> Option<std::ops::Range<usize>> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.n)?;
        let r = self.at..end;
        self.at = end;
        Some(r)
    }
    fn u64(&mut self) -> Option<u64> {
        Some(self.take(8)?.rev().fold(0u64, |w, i| w << 8 | (self.byte)(i) as u64))
    }
    fn int(&mut self) -> Option<i64> {
        Some(self.u64()? as i64)
    }
    fn len(&mut self) -> Option<usize> {
        usize::try_from(self.int()?).ok().filter(|n| *n <= self.n)
    }
    fn text(&mut self) -> Option<String> {
        let n = self.len()?;
        String::from_utf8(self.take(n)?.map(|i| (self.byte)(i)).collect()).ok()
    }
    fn counts(&mut self) -> Option<Vec<(String, i64, i64)>> {
        (0..self.len()?).map(|_| Some((self.text()?, self.int()?, self.int()?))).collect()
    }
    fn tested(&mut self) -> Option<BTreeSet<(usize, i64)>> {
        (0..self.len()?).map(|_| Some((usize::try_from(self.int()?).ok()?, self.int()?))).collect()
    }
}

/// `encode`'s entries back; `None` if the blob is not of this format (the caller keeps none, so a format 1 blob is
/// measured again once).
pub(super) fn decode(b: &[u8]) -> Option<ByDigest> {
    decode_from(&|i| b[i], b.len())
}

/// `decode`, of `n` bytes read by position.
fn decode_from(byte: &dyn Fn(usize) -> u8, bytes: usize) -> Option<ByDigest> {
    let mut r = Reader { byte, n: bytes, at: 0 };
    if byte(r.take(1)?.start) != 2 {
        return None;
    }
    let n = r.len()?;
    let mut all = ByDigest::with_capacity(n);
    for _ in 0..n {
        let name = r.text()?;
        let digest = r.u64()?;
        let cols = (0..r.len()?).map(|_| Some((r.text()?, r.int()?, r.int()?, r.int()?, r.int()?))).collect::<Option<Vec<_>>>()?;
        all.insert((name, digest), (cols, r.counts()?, r.tested()?));
    }
    (r.at == bytes).then_some(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Entries come back as stored, and a blob of another format (format 1's included) or cut short is none.
    #[test]
    fn kept_statistics_come_back_by_name_and_digest() {
        let mut all = ByDigest::new();
        all.insert(("k".to_owned(), 7), (vec![("k_c0".to_owned(), 4, 2, 8, 1), ("k_c1".to_owned(), 4, 4, 4, 4)], vec![("k_c0".to_owned(), 1, 2)], [(0, 1)].into()));
        all.insert(("m".to_owned(), u64::MAX), (vec![("m_c0".to_owned(), 4, 3, 2, 1)], vec![], [(0, -5)].into()));
        let blob = encode(&all);
        let back = decode(&blob).expect("decodes");
        assert_eq!(back, all);
        assert!(decode(&blob[..blob.len() - 1]).is_none());
        assert!(decode(&[1]).is_none());
        assert!(decode(&[3]).is_none());
        let words = to_words(&blob);
        assert_eq!(from_words(words.len(), &|i| words[i]).map(|b| b.len()), Some(2));
        assert!(from_words(words.len() - 1, &|i| words[i + 1]).is_none());
        assert!(from_words(0, &|i| words[i]).is_none());
    }
}
