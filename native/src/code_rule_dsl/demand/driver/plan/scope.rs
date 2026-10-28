use super::identity::{DocumentId, ScopeInstanceId, ScopeRecipeId, SnapshotId};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct CandidateRoot {
    document: DocumentId,
    node_ordinal: u32,
}

impl CandidateRoot {
    pub(super) fn new(document: DocumentId, node_ordinal: u32) -> Self {
        Self {
            document,
            node_ordinal,
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum CompletionStatus {
    Complete,
    Failed(String),
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct DocumentCompletion {
    document: DocumentId,
    status: CompletionStatus,
}

impl DocumentCompletion {
    pub(super) fn new(document: DocumentId, status: CompletionStatus) -> Self {
        Self { document, status }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ScopeError {
    DuplicateDocument(DocumentId),
    CompletionDocumentsDiffer,
    EmptyFailureCode(DocumentId),
    UnknownCandidateDocument(DocumentId),
    CandidateOrder,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct CandidateScope {
    id: ScopeInstanceId,
    snapshot: SnapshotId,
    recipe: ScopeRecipeId,
    node_order_version: u16,
    documents: Vec<DocumentId>,
    roots: Vec<CandidateRoot>,
    completions: Vec<DocumentCompletion>,
}

impl CandidateScope {
    pub(super) fn new(
        snapshot: SnapshotId,
        recipe: ScopeRecipeId,
        node_order_version: u16,
        documents: Vec<DocumentId>,
        roots: Vec<CandidateRoot>,
        completions: Vec<DocumentCompletion>,
    ) -> Result<Self, ScopeError> {
        let mut positions = BTreeMap::new();
        let mut unique = BTreeSet::new();
        for (position, document) in documents.iter().copied().enumerate() {
            if !unique.insert(document) {
                return Err(ScopeError::DuplicateDocument(document));
            }
            positions.insert(document, position);
        }

        if completions.len() != documents.len()
            || completions
                .iter()
                .zip(&documents)
                .any(|(completion, document)| completion.document != *document)
        {
            return Err(ScopeError::CompletionDocumentsDiffer);
        }
        if let Some(completion) = completions.iter().find(|completion| {
            matches!(&completion.status, CompletionStatus::Failed(code) if code.is_empty())
        }) {
            return Err(ScopeError::EmptyFailureCode(completion.document));
        }

        let mut previous = None;
        for root in &roots {
            let Some(&position) = positions.get(&root.document) else {
                return Err(ScopeError::UnknownCandidateDocument(root.document));
            };
            let key = (position, root.node_ordinal);
            if previous.is_some_and(|previous| previous >= key) {
                return Err(ScopeError::CandidateOrder);
            }
            previous = Some(key);
        }

        let mut identity = ScopeInstanceId::builder()
            .identity(snapshot)
            .identity(recipe)
            .u16(node_order_version)
            .u64(documents.len() as u64);
        for document in &documents {
            identity = identity.identity(*document);
        }
        identity = identity.u64(completions.len() as u64);
        for completion in &completions {
            identity = identity.identity(completion.document);
            identity = match &completion.status {
                CompletionStatus::Complete => identity.u16(1),
                CompletionStatus::Failed(code) => identity.u16(2).text(code),
            };
        }
        identity = identity.u64(roots.len() as u64);
        for root in &roots {
            identity = identity.identity(root.document).u32(root.node_ordinal);
        }

        Ok(Self {
            id: identity.finish(),
            snapshot,
            recipe,
            node_order_version,
            documents,
            roots,
            completions,
        })
    }

    pub(super) fn id(&self) -> ScopeInstanceId {
        self.id
    }

    #[cfg(test)]
    pub(super) fn same_exact_scope(&self, other: &Self) -> bool {
        self == other
    }
}
