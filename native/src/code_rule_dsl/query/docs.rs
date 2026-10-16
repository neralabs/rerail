//! The documents a batch's statements read: its files, and derived documents, the literals a
//! sub-grammar parsed (`parse`, `formatArgs`), registered on first use. A derived document is
//! addressed like a file, by an index past the batch's files, so the tables and functions walk and
//! test its nodes as they do a file's.

use super::BatchState;
use crate::code_rule_dsl::literal::{SubDoc, SubTree, sub_ordinal};
use crate::code_rule_dsl::scope::ScopeTable;
use crate::code_rule_dsl::transducer::sql_backend::{BatchFile, BatchView};
use ast_grep_core::{AstGrep, Node};
use std::sync::Arc;

/// A parsed literal as a document of the batch.
pub(super) struct Derived {
    root: AstGrep<SubDoc>,
    /// The document the literal was parsed from.
    pub(super) host: i64,
    /// A string type definition's file table, which its names are read in.
    pub(super) table: Option<Arc<ScopeTable>>,
}

impl Derived {
    pub(super) fn tree(&self) -> &Arc<SubTree> {
        &self.root.root().get_doc().tree
    }

    pub(super) fn node(&self, ordinal: u32) -> Node<'_, SubDoc> {
        self.root.adopt(self.root.root().get_doc().node_at(ordinal))
    }

    pub(super) fn len(&self) -> usize {
        self.tree().len()
    }
}

/// A document of a batch, by index.
pub(super) enum Document<'a> {
    File(&'a BatchFile),
    Derived(Arc<Derived>),
}

impl Document<'_> {
    pub(super) fn len(&self) -> usize {
        match self {
            Self::File(file) => file.len(),
            Self::Derived(derived) => derived.len(),
        }
    }
}

/// The document at `index`: a file of the batch, or a derived document past them.
pub(super) fn document<'a>(
    view: &'a BatchView,
    state: Option<&BatchState>,
    index: i64,
) -> Option<Document<'a>> {
    let index = usize::try_from(index).ok()?;
    let files = view.files();
    if let Some(file) = files.get(index) {
        return Some(Document::File(file));
    }
    state?
        .derived
        .lock()
        .expect("derived documents mutex poisoned")
        .get(index - files.len())
        .cloned()
        .map(Document::Derived)
}

/// The batch state a view carries.
pub(super) fn batch_state(view: &BatchView) -> Option<&BatchState> {
    view.extension()
        .and_then(|extension| extension.downcast_ref::<BatchState>())
}

/// The index of `tree` parsed from document `host`, registering it on first use.
pub(super) fn derive(
    view: &BatchView,
    state: &BatchState,
    tree: &Arc<SubTree>,
    host: i64,
    table: Option<Arc<ScopeTable>>,
) -> i64 {
    let mut derived = state
        .derived
        .lock()
        .expect("derived documents mutex poisoned");
    let base = view.files().len() as i64;
    if let Some(index) = derived
        .iter()
        .position(|known| Arc::ptr_eq(known.tree(), tree) && known.host == host)
    {
        return base + index as i64;
    }
    derived.push(Arc::new(Derived {
        root: AstGrep::doc(SubDoc {
            tree: Arc::clone(tree),
        }),
        host,
        table,
    }));
    base + derived.len() as i64 - 1
}

/// The chain of derived documents from `index` out to its file, innermost first.
pub(super) fn derived_chain(
    view: &BatchView,
    state: &BatchState,
    mut index: i64,
) -> Vec<Arc<Derived>> {
    let mut chain = Vec::new();
    while let Some(Document::Derived(derived)) = document(view, Some(state), index) {
        index = derived.host;
        chain.push(derived);
    }
    chain
}

/// A derived document's node ordinal, from its node id.
pub(super) fn derived_ordinal(node_id: usize) -> u32 {
    sub_ordinal(node_id)
}
