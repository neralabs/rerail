//! The content store as the facts reach it (crate split step 10b-2): the calls the crates below the store make of an
//! open store (rerail-facts-base, rerail-facts, rerail-transducer-base) (`FactStore`), which `ContentStore` implements (rerail-store, `content_store/fact_store.rs`).
//! A crate that holds a store through it needs no store: the store is built above it, and an edit to the store
//! rebuilds only the crates that open one (`tools/cratify/fanout.py`).
//!
//! Each call is `ContentStore`'s method of the same name; its documentation is there.
use super::positional_search;
use super::working_set::WorkingSet;
use super::{
    Binding, CachedBytes, Derivation, DerivationRecord, GitTree, HashKey, ObjectBytes, QueuedDocument, SourceId, StoreResult,
    StoredDerivation, Sym, TapeBytes, TapeFrame, TapeId, TapePrefilter, TapeSearchIndex, TreeDiff, write_back::Key,
};
use std::collections::HashMap;
use std::sync::Arc;

/// A store's atoms (`atoms::Atoms`, rerail-store), as a batch interns and reads them.
pub trait AtomTable: Send + Sync {
    /// The atom of `value`, made where there is none (`Atoms::intern`).
    fn intern(&self, value: &str) -> StoreResult<Sym>;
    /// The atoms of `values`, in order (`Atoms::intern_all`).
    fn intern_all(&self, values: &[&str]) -> StoreResult<Vec<Sym>>;
    /// An atom's text (`Atoms::value`).
    fn value(&self, atom: Sym) -> StoreResult<Option<String>>;
}

/// An open content store, as the facts read and write it.
pub trait FactStore: Send + Sync + std::any::Any {
    // The derivations (`derivation.rs`).
    fn bind_names(&self, bindings: &[Binding]) -> StoreResult<Vec<String>>;
    fn record_derivations(&self, records: &[DerivationRecord]) -> StoreResult<Vec<String>>;
    fn binding(&self, name: &str) -> StoreResult<Option<[u8; 32]>>;
    fn binding_of(&self, name: Sym) -> StoreResult<Option<[u8; 32]>>;
    fn derivations_reading(&self, names: &[String]) -> StoreResult<Vec<Derivation>>;
    fn derivation_valid(&self, output: &str) -> StoreResult<bool>;
    fn derivation_of(&self, output: Sym) -> StoreResult<Option<StoredDerivation>>;
    fn readers_of(&self, name: Sym) -> StoreResult<Vec<Sym>>;
    fn invalid_derivations(&self) -> StoreResult<Vec<Derivation>>;
    fn write_derivation_layer(&self, bindings: &[(Sym, [u8; 32])], derivations: &[(Sym, StoredDerivation)]) -> StoreResult<()>;
    fn begin_push(&self) -> StoreResult<()>;
    fn end_push(&self) -> StoreResult<()>;
    fn push_clean(&self) -> StoreResult<bool>;

    // Sources and tapes (`objects.rs`, `object_segments.rs`).
    fn put_source(&self, source: &[u8], aliases: &[HashKey]) -> StoreResult<SourceId>;
    fn get_source(&self, source_id: SourceId) -> StoreResult<Vec<u8>>;
    fn source_by_hash(&self, hash: HashKey) -> StoreResult<Option<SourceId>>;
    fn source_by_alias(&self, alias: HashKey) -> StoreResult<Option<SourceId>>;
    fn put_tape(&self, source_id: SourceId, grammar_id: i64, grammar_version: i64, tape_format_version: i64, tape: &[u8]) -> StoreResult<TapeId>;
    fn get_tapes_bulk(&self, tape_ids: &[TapeId]) -> StoreResult<HashMap<TapeId, Arc<CachedBytes>>>;
    fn tape_by_source(&self, source_id: SourceId, grammar_id: i64, grammar_version: i64, tape_format_version: i64) -> StoreResult<Option<TapeId>>;
    fn tape_frames_for(&self, sources: &[SourceId], grammar_id: i64, grammar_version: i64, tape_format_version: i64) -> StoreResult<Vec<Option<TapeFrame>>>;
    #[allow(clippy::too_many_arguments)]
    fn queue_source_and_tape(
        &self,
        key: Key,
        canonical: HashKey,
        grammar_id: i64,
        grammar_version: i64,
        tape_format_version: i64,
        source: &Arc<String>,
        tape: TapeBytes,
        named: bool,
        documents: Option<Arc<[QueuedDocument]>>,
    ) -> StoreResult<()>;
    fn queued_tape(&self, key: &Key) -> Option<StoreResult<ObjectBytes>>;
    /// The store's RAM working set (decision 0024 §5).
    fn working_set(&self) -> &WorkingSet;
    /// A reader's collection lease, held while it is (`ContentStore::hold_collection`).
    fn hold_collection(&self) -> StoreResult<Box<dyn std::any::Any + Send>>;

    // Prefilters, search indexes and Git trees.
    fn put_prefilters_bulk(&self, sets: &[TapePrefilter]) -> StoreResult<()>;
    fn hold_prefilters(&self, sets: &[TapePrefilter]);
    /// `operation` in the candidate statement's transaction (`ContentStore::candidate_transaction`; callers use
    /// `dyn FactStore`'s `candidate_transaction`, which returns its value).
    fn candidate_transaction_with(&self, writes: bool, operation: &mut dyn FnMut(&rusqlite::Connection) -> rusqlite::Result<()>) -> StoreResult<()>;
    fn put_search_indexes_bulk(&self, indexes: &[TapeSearchIndex]) -> StoreResult<()>;
    // The positional search index (`positional_search.rs`).
    fn put_positional_search_indexes_bulk(&self, indexes: &[positional_search::TapeSearchIndex]) -> StoreResult<()>;
    fn get_search_anchors_bulk(&self, lookups: &[positional_search::SearchAnchorLookup]) -> StoreResult<Vec<positional_search::StoredAnchorSelection>>;
    fn get_exact_value_candidates_bulk(&self, tape_ids: &[TapeId], index_version: i64, value_tokens: &[String]) -> StoreResult<Vec<Vec<Option<bool>>>>;
    fn get_text_candidates_bulk(&self, tape_ids: &[TapeId], required_words: &[String]) -> StoreResult<Vec<Option<bool>>>;
    fn put_git_trees_bulk(&self, trees: &[GitTree]) -> StoreResult<()>;
    fn tree_changes_since(&self, old: Option<[u8; 32]>, new: HashKey) -> StoreResult<TreeDiff>;

    // The atoms.
    fn intern_atoms(&self, values: &[&str]) -> StoreResult<Vec<Sym>>;
    /// The store's atoms, as a batch interns and reads them (`ContentStore::atoms_handle`).
    fn atoms_handle(&self) -> Arc<dyn AtomTable>;
    fn atom(&self, value: &str) -> StoreResult<Option<Sym>>;
    fn atom_value(&self, atom: Sym) -> StoreResult<Option<String>>;
}

impl dyn FactStore {
    /// `operation` in the candidate statement's transaction, its value returned (`ContentStore::candidate_transaction`).
    pub fn candidate_transaction<T>(&self, writes: bool, mut operation: impl FnMut(&rusqlite::Connection) -> rusqlite::Result<T>) -> StoreResult<T> {
        let mut out = None;
        self.candidate_transaction_with(writes, &mut |connection| {
            out = Some(operation(connection)?);
            Ok(())
        })?;
        Ok(out.expect("the candidate transaction ran its operation"))
    }
}
