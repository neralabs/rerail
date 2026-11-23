//! A documents' table's places (decision 0494 step 4 part 2): beside each document's content, its sha256, its
//! source's place in its segment and its tape's hash and place. The writer finds them as it writes the table (its
//! transaction reads the objects' rows), once the run's sources and tapes are placed. A run inside another's waves
//! then reads a changed document's old source and tape where they lie, with no SQL (decision 0104), each checked by
//! the hash kept: a segment's number is the largest's plus one, so a removed last segment's number is used again,
//! and a place gone, moved, archived or reused is a miss.
//!
//! A row as given (`DOCUMENT_WORDS`): the path's atom, the document's key, the content's tag (0 a stored source, by
//! its id; 1 one not stored when the run gave the table, by its sha256) and four words of it. As written
//! (`PLACED_WORDS`): that, then the sha256's four words, the source's place (segment, offset, length), the tape's
//! hash (its algorithm and four lanes), its place and its length. A place not found is segment 0.

use super::object_segments::{object_dir, row_frame};
use super::segments::SegmentMaps;
use super::{CachedBytes, ContentStore, HashKey, StoreResult, TapeFrame};
use rusqlite::{Connection, OptionalExtension as _};
use std::sync::Arc;

/// The words of a row as the run gives it.
pub const DOCUMENT_WORDS: usize = 7;
/// The words of a row as the table keeps it.
pub const PLACED_WORDS: usize = DOCUMENT_WORDS + 4 + 3 + 5 + 3 + 1;

/// A document's places as its table's row keeps them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocumentPlaces {
    /// Its content's sha256.
    pub sha256: [u8; 32],
    /// Its source's segment, offset and length; segment 0 where it was not found.
    pub source: [i64; 3],
    /// Its tape's hash (`HashKey::tape_identity`: its algorithm and lanes), place and length; segment 0 where none.
    pub tape_hash: [i64; 5],
    pub tape: [i64; 3],
    pub tape_length: i64,
}

impl DocumentPlaces {
    /// The places of a row as the table keeps it (`PLACED_WORDS`), past its first `DOCUMENT_WORDS`; `None` where none
    /// was found (no sha256).
    pub fn of_row(row: &[i64]) -> Option<Self> {
        let w = row.get(DOCUMENT_WORDS..PLACED_WORDS)?;
        if w[..4].iter().all(|&x| x == 0) {
            return None;
        }
        Some(Self { sha256: digest_of(&w[..4]), source: [w[4], w[5], w[6]], tape_hash: [w[7], w[8], w[9], w[10], w[11]], tape: [w[12], w[13], w[14]], tape_length: w[15] })
    }
}

fn digest_of(words: &[i64]) -> [u8; 32] {
    let mut d = [0u8; 32];
    for (chunk, w) in d.as_chunks_mut::<8>().0.iter_mut().zip(words) {
        *chunk = w.to_le_bytes();
    }
    d
}

fn words_of(d: &[u8; 32]) -> [i64; 4] {
    std::array::from_fn(|i| i64::from_le_bytes(d[i * 8..i * 8 + 8].try_into().expect("eight bytes")))
}

fn sha256_of(key: HashKey) -> Option<[u8; 32]> {
    (key.algorithm == super::HASH_SHA256).then(|| {
        let mut d = [0u8; 32];
        for (chunk, lane) in d.as_chunks_mut::<8>().0.iter_mut().zip(key.lanes) {
            *chunk = lane.to_be_bytes();
        }
        d
    })
}

/// `rows` (`n` rows as given, `DOCUMENT_WORDS` each) with each one's places found, in transaction `t`: the source
/// its content names (by id, else by sha256), and its tape in the grammar `tape` (grammar id, grammar version, tape
/// format version) names, where one is stored. A content given by its sha256 and stored now is kept by its id (tag
/// 0), as a reader of the table resolves it before the first wave: a run inside another's waves, which reads no SQL,
/// finds its files by their ids, and compares them with the table's as they are.
pub(super) fn resolve(t: &Connection, rows: &super::kept_facts::BlockFlat, tape: Option<[i64; 3]>) -> StoreResult<super::kept_facts::BlockFlat> {
    // Each row's words written into the placed table's block as they are found, none gathered on the heap.
    let mut placed = super::kept_facts::BlockFlat::new(rows.len(), PLACED_WORDS, true);
    let mut source_st = t.prepare_cached(
        "SELECT h.algorithm_id, h.lane0, h.lane1, h.lane2, h.lane3, s.seg, s.seg_at, s.seg_len FROM source_objects s \
         JOIN hashes h ON h.hash_id = s.canonical_hash_id WHERE s.source_id = ?1",
    )?;
    let mut tape_st = t.prepare_cached(
        "SELECT h.algorithm_id, h.lane0, h.lane1, h.lane2, h.lane3, t.seg, t.seg_at, t.seg_len, t.raw_length FROM tape_objects t \
         JOIN hashes h ON h.hash_id = t.canonical_hash_id \
         WHERE t.source_id = ?1 AND t.grammar_id = ?2 AND t.grammar_version = ?3 AND t.tape_format_version = ?4",
    )?;
    let hash = |r: &rusqlite::Row<'_>| -> rusqlite::Result<HashKey> { Ok(HashKey { algorithm: r.get(0)?, lanes: [r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?] }) };
    for r in 0..rows.len() {
        let row: [i64; DOCUMENT_WORDS] = std::array::from_fn(|c| rows.get(r, c));
        let row = &row;
        let mut c = 0;
        let mut out = |words: &[i64]| {
            for &w in words {
                placed.put(r, c, w);
                c += 1;
            }
        };
        let id = match row[2] {
            0 => Some(row[3]),
            1 => super::objects::source_id_by_hash(t, HashKey::from_sha256_digest(digest_of(&row[3..7])))?.map(|s| s.0),
            _ => None,
        };
        match id {
            Some(id) if row[2] == 1 => out(&[row[0], row[1], 0, id, 0, 0, 0]),
            _ => out(row),
        }
        let source: Option<(HashKey, [i64; 3])> = match id {
            Some(id) => source_st.query_row([id], |r| Ok((hash(r)?, [r.get(5)?, r.get(6)?, r.get(7)?]))).optional()?,
            None => None,
        };
        let Some((key, place)) = source.and_then(|(k, p)| Some((sha256_of(k)?, p))) else {
            out(&[0i64; PLACED_WORDS - DOCUMENT_WORDS]);
            continue;
        };
        out(&words_of(&key));
        // An archived object's place is no segment's: a miss, read through the store.
        out(&if place[0] > 0 { place } else { [0; 3] });
        let tape_row: Option<(HashKey, [i64; 3], i64)> = match (id, tape) {
            (Some(id), Some([grammar, version, format])) => tape_st.query_row([id, grammar, version, format], |r| Ok((hash(r)?, [r.get(5)?, r.get(6)?, r.get(7)?], r.get(8)?))).optional()?,
            _ => None,
        };
        match tape_row.filter(|(_, p, _)| p[0] > 0) {
            Some((h, p, length)) => {
                out(&[h.algorithm]);
                out(&h.lanes);
                out(&p);
                out(&[length]);
            }
            None => out(&[0i64; 9]),
        }
    }
    Ok(placed)
}

impl ContentStore {
    /// The source `places` names, read where it lies with no SQL: `None` unless its segment holds bytes of its sha256
    /// there.
    pub fn placed_source(&self, places: &DocumentPlaces) -> Option<Arc<CachedBytes>> {
        let [seg, at, len] = places.source;
        let mut maps = SegmentMaps::new(object_dir(&self.database_path));
        let frame = row_frame(&mut maps, seg, at, len)?;
        (HashKey::sha256(frame.segment().bytes()) == HashKey::from_sha256_digest(places.sha256)).then(|| Arc::new(CachedBytes::mapped(frame.segment().clone())))
    }

    /// The tape `places` names, read where it lies with no SQL: `None` unless its segment holds a tape of its hash
    /// there (`HashKey::tape_identity` of its source's sha256 and its bytes).
    pub fn placed_tape(&self, places: &DocumentPlaces) -> Option<TapeFrame> {
        let [seg, at, len] = places.tape;
        let mut maps = SegmentMaps::new(object_dir(&self.database_path));
        let frame = row_frame(&mut maps, seg, at, len)?;
        let [algorithm, l0, l1, l2, l3] = places.tape_hash;
        let kept = HashKey { algorithm, lanes: [l0, l1, l2, l3] };
        (HashKey::tape_identity(HashKey::from_sha256_digest(places.sha256), frame.segment().bytes()) == kept).then(|| TapeFrame::new(places.tape_length, frame))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rows_places_read_back_as_written() {
        let places = DocumentPlaces { sha256: [7; 32], source: [3, 40, 12], tape_hash: [1, -2, 3, -4, 5], tape: [4, 0, 99], tape_length: 99 };
        let mut row = vec![11, 12, 0, 13, 0, 0, 0];
        row.extend(words_of(&places.sha256));
        row.extend(places.source);
        row.extend(places.tape_hash);
        row.extend(places.tape);
        row.push(places.tape_length);
        assert_eq!(row.len(), PLACED_WORDS);
        assert_eq!(DocumentPlaces::of_row(&row), Some(places));
        assert_eq!(DocumentPlaces::of_row(&row[..DOCUMENT_WORDS]), None, "a row as given has no places");
    }
}
