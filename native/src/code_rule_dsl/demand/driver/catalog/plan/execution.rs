use super::grouping::{ClauseScope, FailureIsolationMode, group_exact_scopes};
use super::identity::{
    ClausePlanId, DocumentId, DocumentMapGenerationId, EffectProfileId, ScopeRecipeId, SnapshotId,
    UtilityDefinitionId,
};
use super::scope::{CandidateRoot, CandidateScope, CompletionStatus, DocumentCompletion};
use crate::code_rule_dsl::language::label;
use crate::code_rule_dsl::query::{ClauseId, PackedFileInput};
use std::collections::BTreeMap;

/// A declarative recipe for the candidates admitted to a clause.
///
/// The recipe identifies the selection operation, not a clause's allocation or source position.
/// Two clauses evaluated by this recipe can share only after their evaluated scopes compare equal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl::query) enum ScopeRecipe {
    AdmittedClauseCandidates,
}

impl ScopeRecipe {
    fn identity(self) -> ScopeRecipeId {
        match self {
            Self::AdmittedClauseCandidates => ScopeRecipeId::builder()
                .text("admitted-clause-candidates")
                .u16(1)
                .finish(),
        }
    }

    fn evaluate(
        self,
        files: &[PackedFileInput],
        documents: &[DocumentId],
        source: ClauseId,
        snapshot: SnapshotId,
    ) -> Result<CandidateScope, String> {
        let mut eligible = Vec::new();
        let mut roots = Vec::new();
        let mut completions = Vec::new();
        for (file, document) in files.iter().zip(documents) {
            let Some(candidates) = file.tasks.get(&source) else {
                continue;
            };
            eligible.push(*document);
            completions.push(DocumentCompletion::new(
                *document,
                match &file.syntax_failure {
                    Some(error) => CompletionStatus::Failed(error.clone()),
                    None => CompletionStatus::Complete,
                },
            ));
            let mut candidates = candidates.clone();
            candidates.sort_unstable();
            candidates.dedup();
            roots.extend(
                candidates
                    .into_iter()
                    .map(|node| CandidateRoot::new(*document, node)),
            );
        }
        CandidateScope::new(snapshot, self.identity(), 1, eligible, roots, completions)
            .map_err(|error| format!("invalid exact candidate scope: {error:?}"))
    }
}

/// The typed logical information needed before physical statement planning.
pub(in crate::code_rule_dsl::query) struct ClauseLogicalPlan {
    ordinal: usize,
    source: ClauseId,
    identity: ClausePlanId,
    scope_recipe: ScopeRecipe,
    effect_profile: EffectProfileId,
    utility_definitions: Vec<UtilityDefinitionId>,
}

impl ClauseLogicalPlan {
    pub(in crate::code_rule_dsl::query) fn new(
        ordinal: usize,
        source: ClauseId,
        rule_id: &str,
        clause_id: &str,
        effect_profile: u16,
        lowering_variant: u64,
        utility_definitions: impl IntoIterator<Item = UtilityDefinitionId>,
    ) -> Self {
        Self {
            ordinal,
            source,
            identity: ClausePlanId::builder()
                .text(rule_id)
                .text(clause_id)
                .u64(lowering_variant)
                .finish(),
            scope_recipe: ScopeRecipe::AdmittedClauseCandidates,
            effect_profile: EffectProfileId::builder().u16(effect_profile).finish(),
            utility_definitions: utility_definitions.into_iter().collect(),
        }
    }

    pub(in crate::code_rule_dsl::query) fn utility_definitions(&self) -> &[UtilityDefinitionId] {
        &self.utility_definitions
    }
}

pub(in crate::code_rule_dsl::query) fn exact_scope_groups(
    files: &[PackedFileInput],
    clauses: &[ClauseLogicalPlan],
) -> Result<Vec<Vec<usize>>, String> {
    let documents = files
        .iter()
        .map(|file| {
            let language = label(file.language).unwrap_or("unknown");
            DocumentId::builder()
                .text(&file.path)
                .text(language)
                .bytes(file.tape.transducer_source())
                .finish()
        })
        .collect::<Vec<_>>();
    let mut snapshot = SnapshotId::builder()
        .u64(u64::try_from(documents.len()).map_err(|_| "workspace document count exceeds u64")?);
    for document in &documents {
        snapshot = snapshot.identity(*document);
    }
    let snapshot = snapshot.finish();

    let mut document_map = DocumentMapGenerationId::builder();
    for file in files {
        document_map = document_map.u64(
            u64::try_from(file.document_maps.len())
                .map_err(|_| "workspace document-map count exceeds u64")?,
        );
        for span in &file.document_maps {
            for value in [
                span.source_document,
                span.generated_document,
                span.source_start,
                span.source_end,
                span.generated_start,
                span.generated_end,
            ] {
                document_map = document_map
                    .u64(u64::try_from(value).map_err(|_| "document-map coordinate exceeds u64")?);
            }
        }
    }
    let document_map = document_map.finish();

    let mut ordinals = BTreeMap::new();
    let mut assignments = Vec::with_capacity(clauses.len());
    for clause in clauses {
        if ordinals.insert(clause.identity, clause.ordinal).is_some() {
            return Err(format!(
                "duplicate semantic clause plan identity {} at ordinal {}",
                clause.identity, clause.ordinal
            ));
        }
        assignments.push(ClauseScope::new(
            clause.identity,
            clause
                .scope_recipe
                .evaluate(files, &documents, clause.source, snapshot)?,
            clause.effect_profile,
            document_map,
            FailureIsolationMode::PerFile,
        ));
    }

    let mut groups = group_exact_scopes(assignments)
        .into_iter()
        .map(|group| {
            group
                .clauses()
                .iter()
                .map(|clause| {
                    ordinals
                        .get(clause)
                        .copied()
                        .ok_or_else(|| "exact-scope group contains an unknown clause".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    for group in &mut groups {
        group.sort_unstable();
    }
    groups.sort_by_key(|group| group.first().copied().unwrap_or(usize::MAX));
    Ok(groups)
}
