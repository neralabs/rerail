//! A tape's positional search index and its lookups, as their builders and readers hold them.

use crate::content_store::TapeId;
use std::sync::Arc;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TapeSearchIndex {
    pub tape_id: TapeId,
    pub index_version: i64,
    pub node_count: u32,
    pub kind_tokens: Arc<String>,
    pub value_tokens: Arc<String>,
    pub source_text: Arc<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchAnchorLookup {
    pub tape_id: TapeId,
    pub index_version: i64,
    pub kind_token: String,
    pub value_token: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoredAnchorSelection {
    Unknown,
    Candidate,
    Roots(Vec<u32>),
}
