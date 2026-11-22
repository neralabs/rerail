//! A combined run's fact spaces (decision 0091): the library's relations are canonical, made once over every
//! document the run reads, and each policy's own rules see only the documents of its scope.
//!
//! When the libraries are held once (decision 0089), the library is a part of its own, under its own names, and no
//! policy's: canonical by construction. Otherwise (`RERAIL_LIBRARIES=copied`, or one program naming its libraries)
//! `combine` names each policy's relations `p<i>_…`, an included library's among them. Here:
//!
//! - A relation the policies mark global (`.global`, or `.global library` for an included file's, as
//!   `rr::read_file` writes it) is *canonical* when every policy's copy is congruent to the others
//!   (`merge::congruence`: equal rules, relations named by class, variables by order). The simplifier then
//!   merges the copies into one (`s_…`). A `.global` the policies define differently is an error; a library's that a
//!   policy changed (it adds rules, or the library reads a relation the policy defines) stays each policy's own.
//! - Canonical relations, and every relation of a policy they read, are *shared*: computed over every document.
//! - Each other rule of a policy whose scope reads fewer documents than the run has a *guard* on each read of a base or
//!   shared relation whose location no read of its own relations already gives: `!rr_scope_not(d)` of the read's
//!   document (its document column, or `@doc(n)` of its node, a shift of the node id: no rows). A policy's own
//!   relations are then made from its documents' rows only, by induction, so their locations need no guard.
//!
//! Without guards, a policy's cross-file rule would join rows of documents only another policy's scope selects.

use std::collections::{BTreeMap, BTreeSet};


/// What `combine` knows of one policy: its prefix, its own relations (prefixed), its globals (unprefixed), its scope.
#[derive(Clone, Debug)]
pub struct Policy {
    pub prefix: String,
    pub own: BTreeSet<String>,
    pub named: BTreeSet<String>,
    pub library: BTreeSet<String>,
    pub scope: Option<Vec<(Vec<String>, Vec<String>)>>,
}

/// What the fact spaces did (`preparation::combining::FactSpaces`), for the profile.
#[derive(Default, Debug)]
pub struct Report {
    /// Global relations with one copy for the run (by name), and how many policies' copies each merged.
    pub canonical: BTreeMap<String, usize>,
    /// Copies of the engine's relations made canonical (alike in several policies).
    pub engine: usize,
    /// Library relations a policy changed: each policy's own.
    pub demoted: Vec<String>,
    /// Policies whose rules got guards, and the guards added.
    pub guarded: Vec<(String, usize)>,
}

