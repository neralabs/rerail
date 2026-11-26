//! A tape's prefilter rows as their builder hands them to the store (`prefilter::TapePrefilter`).

use crate::content_store::TapeId;
use std::collections::BTreeMap;

/// The image a tape's rows were built by. 2: ERROR nodes are kind code 0 in both postings.
/// 3: term postings carry their node count, which the planner statistics sum. 4: kind sizes and
/// edges (`kind_size`, `edge_freq`). 5: terms are atoms. 6: kind postings are container segments
/// (`postings::segment_positions`, decision 0045). 9: kept facts (`kept`), with no edges.
pub const PREFILTER_VERSION: i64 = 9;

/// A vocabulary's kind (the store's `prefilter::vocabulary`, kept facts): a regex's terms, the prefilter's.
pub const VOCABULARY_REGEX: i64 = 0;
/// A vocabulary's kind: a text plan's terms, the text layer's (`text::vocabulary::fill`, through `FactStore`).
pub const VOCABULARY_TEXT: i64 = 1;
/// A vocabulary's test of a group of keys' plans over terms (`FactStore::vocabulary_terms`): for each term, the
/// positions in the group of the plans it matches.
pub type VocabularyTest<'a> = &'a (dyn Fn(&[usize], &[&str]) -> Vec<Vec<u32>> + Sync);

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
    /// Clean leaf terms: (kind atom, term) → preorders, ascending.
    pub postings: BTreeMap<(u32, String), Vec<u32>>,
    /// Per kind atom: the coverage flags and the number of unclean leaves.
    pub cover: BTreeMap<u32, (u8, usize)>,
    /// Per kind atom: the nodes no term covers, ascending.
    pub missing: BTreeMap<u32, Vec<u32>>,
    /// Per kind atom: unclean leaves, which become terms unless the store-wide cap refuses them.
    pub residue: BTreeMap<u32, Vec<(u32, String)>>,
}
