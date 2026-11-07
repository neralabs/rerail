//! Phase 3 row "Vocabulary + regex" (05), at its planned path. P's `nera_terms_matching` and
//! `nera_literal_set` and its `vocabulary.sql`/`vocabulary-fill.sql` were placed where their
//! closure lives: the store's functions in `content_store::vocabulary` (installed on every
//! connection), the fill in `prefilter/vocabulary-fill.sql`, P's originals in
//! `matcher::p_anchors::sql::functions`. This module names them here (provenance, O19).

#[allow(unused_imports)]
pub(crate) use crate::content_store::vocabulary::install;
