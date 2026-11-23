//! The content store as the facts reach it: `FactStore` (rerail-store-base), each call this store's method of the
//! same name (crate split step 10b-2).
use super::{
    Binding, ContentStore, Derivation, DerivationRecord, FactStore, GitTree, HashKey, ObjectBytes, QueuedDocument,
    SourceId, StoreResult, StoredDerivation, Sym, TapeBytes, TapeFrame, TapeId, TapePrefilter, TapeSearchIndex,
    TreeDiff, working_set::WorkingSet, write_back::Key,
};
use super::positional_search;
use super::{AtomTable, CachedBytes};
use std::collections::HashMap;
use std::sync::Arc;

impl ContentStore {
    /// The store a `FactStore` is, for the crates above the store that read the rest of it (boot): the only
    /// `FactStore` is a `ContentStore`.
    pub fn of(store: &dyn FactStore) -> &ContentStore {
        (store as &dyn std::any::Any).downcast_ref::<ContentStore>().expect("every FactStore is a ContentStore")
    }
}

impl FactStore for ContentStore {
    fn bind_names(&self, bindings: &[Binding]) -> StoreResult<Vec<String>> {
        ContentStore::bind_names(self, bindings)
    }
    fn record_derivations(&self, records: &[DerivationRecord]) -> StoreResult<Vec<String>> {
        ContentStore::record_derivations(self, records)
    }
    fn binding(&self, name: &str) -> StoreResult<Option<[u8; 32]>> {
        ContentStore::binding(self, name)
    }
    fn binding_of(&self, name: Sym) -> StoreResult<Option<[u8; 32]>> {
        ContentStore::binding_of(self, name)
    }
    fn derivations_reading(&self, names: &[String]) -> StoreResult<Vec<Derivation>> {
        ContentStore::derivations_reading(self, names)
    }
    fn derivation_valid(&self, output: &str) -> StoreResult<bool> {
        ContentStore::derivation_valid(self, output)
    }
    fn derivation_of(&self, output: Sym) -> StoreResult<Option<StoredDerivation>> {
        ContentStore::derivation_of(self, output)
    }
    fn readers_of(&self, name: Sym) -> StoreResult<Vec<Sym>> {
        ContentStore::readers_of(self, name)
    }
    fn invalid_derivations(&self) -> StoreResult<Vec<Derivation>> {
        ContentStore::invalid_derivations(self)
    }
    fn write_derivation_layer(&self, bindings: &[(Sym, [u8; 32])], derivations: &[(Sym, StoredDerivation)]) -> StoreResult<()> {
        ContentStore::write_derivation_layer(self, bindings, derivations)
    }
    fn begin_push(&self) -> StoreResult<()> {
        ContentStore::begin_push(self)
    }
    fn end_push(&self) -> StoreResult<()> {
        ContentStore::end_push(self)
    }
    fn push_clean(&self) -> StoreResult<bool> {
        ContentStore::push_clean(self)
    }

    fn database_path(&self) -> &std::path::Path {
        ContentStore::database_path(self)
    }
    fn open_connection(&self) -> StoreResult<rusqlite::Connection> {
        ContentStore::open_connection(self)
    }
    fn put_source(&self, source: &[u8], aliases: &[HashKey]) -> StoreResult<SourceId> {
        ContentStore::put_source(self, source, aliases)
    }
    fn get_source(&self, source_id: SourceId) -> StoreResult<Vec<u8>> {
        ContentStore::get_source(self, source_id)
    }
    fn source_by_hash(&self, hash: HashKey) -> StoreResult<Option<SourceId>> {
        ContentStore::source_by_hash(self, hash)
    }
    fn source_by_alias(&self, alias: HashKey) -> StoreResult<Option<SourceId>> {
        ContentStore::source_by_alias(self, alias)
    }
    fn put_tape(&self, source_id: SourceId, grammar_id: i64, grammar_version: i64, tape_format_version: i64, tape: &[u8]) -> StoreResult<TapeId> {
        ContentStore::put_tape(self, source_id, grammar_id, grammar_version, tape_format_version, tape)
    }
    fn get_tapes_bulk(&self, tape_ids: &[TapeId]) -> StoreResult<HashMap<TapeId, Arc<CachedBytes>>> {
        ContentStore::get_tapes_bulk(self, tape_ids)
    }
    fn tape_by_source(&self, source_id: SourceId, grammar_id: i64, grammar_version: i64, tape_format_version: i64) -> StoreResult<Option<TapeId>> {
        ContentStore::tape_by_source(self, source_id, grammar_id, grammar_version, tape_format_version)
    }
    fn tape_frames_for(&self, sources: &[SourceId], grammar_id: i64, grammar_version: i64, tape_format_version: i64) -> StoreResult<Vec<Option<TapeFrame>>> {
        ContentStore::tape_frames_for(self, sources, grammar_id, grammar_version, tape_format_version)
    }
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
    ) -> StoreResult<()> {
        ContentStore::queue_source_and_tape(self, key, canonical, grammar_id, grammar_version, tape_format_version, source, tape, named, documents)
    }
    fn queued_tape(&self, key: &Key) -> Option<StoreResult<ObjectBytes>> {
        ContentStore::queued_tape(self, key)
    }
    fn working_set(&self) -> &WorkingSet {
        ContentStore::working_set(self)
    }
    fn hold_collection(&self) -> StoreResult<Box<dyn std::any::Any + Send>> {
        Ok(Box::new(ContentStore::hold_collection(self)?))
    }

    fn put_prefilters_bulk(&self, sets: &[TapePrefilter]) -> StoreResult<()> {
        ContentStore::put_prefilters_bulk(self, sets)
    }
    fn hold_prefilters(&self, sets: &[TapePrefilter]) {
        ContentStore::hold_prefilters(self, sets)
    }
    fn candidate_transaction_with(&self, writes: bool, operation: &mut dyn FnMut(&rusqlite::Connection) -> rusqlite::Result<()>) -> StoreResult<()> {
        ContentStore::candidate_transaction(self, writes, operation)
    }
    fn put_search_indexes_bulk(&self, indexes: &[TapeSearchIndex]) -> StoreResult<()> {
        ContentStore::put_search_indexes_bulk(self, indexes)
    }
    fn put_positional_search_indexes_bulk(&self, indexes: &[positional_search::TapeSearchIndex]) -> StoreResult<()> {
        ContentStore::put_positional_search_indexes_bulk(self, indexes)
    }
    fn get_search_anchors_bulk(&self, lookups: &[positional_search::SearchAnchorLookup]) -> StoreResult<Vec<positional_search::StoredAnchorSelection>> {
        ContentStore::get_search_anchors_bulk(self, lookups)
    }
    fn get_exact_value_candidates_bulk(&self, tape_ids: &[TapeId], index_version: i64, value_tokens: &[String]) -> StoreResult<Vec<Vec<Option<bool>>>> {
        ContentStore::get_exact_value_candidates_bulk(self, tape_ids, index_version, value_tokens)
    }
    fn get_text_candidates_bulk(&self, tape_ids: &[TapeId], required_words: &[String]) -> StoreResult<Vec<Option<bool>>> {
        ContentStore::get_text_candidates_bulk(self, tape_ids, required_words)
    }
    fn put_git_trees_bulk(&self, trees: &[GitTree]) -> StoreResult<()> {
        ContentStore::put_git_trees_bulk(self, trees)
    }
    fn tree_changes_since(&self, old: Option<[u8; 32]>, new: HashKey) -> StoreResult<TreeDiff> {
        ContentStore::tree_changes_since(self, old, new)
    }

    fn intern_atoms(&self, values: &[&str]) -> StoreResult<Vec<Sym>> {
        ContentStore::intern_atoms(self, values)
    }
    fn atoms_handle(&self) -> Arc<dyn AtomTable> {
        ContentStore::atoms_handle(self)
    }
    fn atom(&self, value: &str) -> StoreResult<Option<Sym>> {
        ContentStore::atom(self, value)
    }
    fn atom_value(&self, atom: Sym) -> StoreResult<Option<String>> {
        ContentStore::atom_value(self, atom)
    }
}
