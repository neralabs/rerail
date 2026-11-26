//! Phase 3 row "Vocabulary + regex" (05), at its planned path. A regex's terms are kept facts of the store
//! (`content_store::prefilter::vocabulary`, brought up to date by `ContentStore::regex_terms`, t33 step 7); the SQL
//! functions P's `vocabulary.sql` used are the store's (`content_store::vocabulary`, installed on every connection),
//! P's originals in `matcher::p_anchors::sql::functions`. This module names them here (provenance, O19).

#[allow(unused_imports)]
pub(crate) use crate::content_store::vocabulary::install;
