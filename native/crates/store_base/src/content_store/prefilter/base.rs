//! A tape's prefilter rows as their builder hands them to the store (`prefilter::TapePrefilter`).

use crate::content_store::TapeId;
use std::collections::BTreeMap;

/// One tape's prefilter rows, built from the tape alone (`code_rule_dsl::prefilter::image`), so
/// publication holds the write lock only to write.
#[derive(Clone, Debug)]
pub struct TapePrefilter {
    pub tape_id: TapeId,
    pub lang: u32,
    /// The tape's node count: candidate SQL enumerates every node of a tape from it.
    pub nodes: u32,
    /// (kind atom, count, sum of descendants, delta-varint postings), sorted by atom.
    pub kinds: Vec<(u32, u32, u64, Vec<u8>)>,
    /// Direct edges: (parent kind atom, child kind atom, field, count), sorted.
    pub edges: Vec<(u32, u32, u16, u32)>,
    /// Clean leaf terms: (kind atom, term) → preorders, ascending.
    pub postings: BTreeMap<(u32, String), Vec<u32>>,
    /// Per kind atom: the coverage flags and the number of unclean leaves.
    pub cover: BTreeMap<u32, (u8, usize)>,
    /// Per kind atom: the nodes no term covers, ascending.
    pub missing: BTreeMap<u32, Vec<u32>>,
    /// Per kind atom: unclean leaves, which become terms unless the store-wide cap refuses them.
    pub residue: BTreeMap<u32, Vec<(u32, String)>>,
}
