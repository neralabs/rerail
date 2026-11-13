mod execution;
mod grouping;
mod identity;
mod physical;
mod render;
mod scope;
#[cfg(test)]
mod semantics;
#[cfg(test)]
mod utility;

pub(super) use execution::{ClauseLogicalPlan, exact_scope_groups};
pub(super) use identity::{UtilityDefinitionId, utility_definition_id};
pub(super) use physical::{FactFamilyContract, StatementRelations};
#[cfg(test)]
pub(super) use render::legacy_test_scan_sql;
pub(super) use render::{ClauseMergeAst, TypedBoundaryAst, workspace_execution_sql};

#[cfg(test)]
mod tests {
    use super::identity::{RelationId, ScopeRecipeId};
    use super::render::SqlAstNodeKind;

    #[test]
    fn sql_ast_has_every_planned_relational_node() {
        assert_eq!(
            SqlAstNodeKind::ALL,
            [
                SqlAstNodeKind::RelationScan,
                SqlAstNodeKind::DemandScan,
                SqlAstNodeKind::Select,
                SqlAstNodeKind::Join,
                SqlAstNodeKind::SemiJoin,
                SqlAstNodeKind::AntiJoin,
                SqlAstNodeKind::OrderedReduce,
                SqlAstNodeKind::Aggregate,
                SqlAstNodeKind::Project,
                SqlAstNodeKind::Union,
                SqlAstNodeKind::Materialize,
                SqlAstNodeKind::OutputMerge,
                SqlAstNodeKind::Group,
                SqlAstNodeKind::Window,
                SqlAstNodeKind::Recursive,
                SqlAstNodeKind::Reference,
            ]
        );
    }

    #[test]
    fn semantic_identity_is_framed_and_domain_separated() {
        let split_after = RelationId::builder().text("ab").text("c").finish();
        let split_before = RelationId::builder().text("a").text("bc").finish();
        let other_domain = ScopeRecipeId::builder().text("ab").text("c").finish();

        assert_ne!(split_after, split_before);
        assert_ne!(split_after.as_bytes(), other_domain.as_bytes());
    }

    #[test]
    fn semantic_identity_distinguishes_field_types() {
        let text = RelationId::builder().text("\0\u{1}").finish();
        let number = RelationId::builder().u16(1).finish();
        let bytes = RelationId::builder().bytes(&[0, 1]).finish();

        assert_ne!(text, number);
        assert_ne!(text, bytes);
        assert_ne!(number, bytes);
    }

    #[test]
    fn semantic_identity_encoding_has_a_fixed_golden() {
        let identity = RelationId::builder()
            .u16(5)
            .u32(89)
            .u64(13_421)
            .boolean(true)
            .text("candidate roots")
            .bytes(&[0, 1, 2, 255])
            .finish();

        assert_eq!(
            identity.to_string(),
            "relation:0fff066dbf2d31cff4062e2cd918bb65d6eb107cb5b19531ff6863ec068188e4"
        );
    }
}

#[cfg(test)]
mod scope_tests {
    use super::identity::{DocumentId, ScopeRecipeId, SnapshotId};
    use super::scope::{
        CandidateRoot, CandidateScope, CompletionStatus, DocumentCompletion, ScopeError,
    };

    fn document(name: &str) -> DocumentId {
        DocumentId::builder().text(name).finish()
    }

    fn scope(
        documents: Vec<DocumentId>,
        roots: Vec<CandidateRoot>,
        completions: Vec<DocumentCompletion>,
    ) -> Result<CandidateScope, ScopeError> {
        CandidateScope::new(
            SnapshotId::builder().text("snapshot").finish(),
            ScopeRecipeId::builder().text("python roots").finish(),
            1,
            documents,
            roots,
            completions,
        )
    }

    #[test]
    fn exact_scope_identity_covers_roots_order_and_completion() {
        let a = document("a.py");
        let b = document("b.py");
        let complete = vec![
            DocumentCompletion::new(a, CompletionStatus::Complete),
            DocumentCompletion::new(b, CompletionStatus::Complete),
        ];
        let first = scope(
            vec![a, b],
            vec![CandidateRoot::new(a, 3), CandidateRoot::new(b, 1)],
            complete.clone(),
        )
        .expect("valid scope");
        let same = scope(
            vec![a, b],
            vec![CandidateRoot::new(a, 3), CandidateRoot::new(b, 1)],
            complete,
        )
        .expect("same valid scope");
        let different_root = scope(
            vec![a, b],
            vec![CandidateRoot::new(a, 4), CandidateRoot::new(b, 1)],
            vec![
                DocumentCompletion::new(a, CompletionStatus::Complete),
                DocumentCompletion::new(b, CompletionStatus::Complete),
            ],
        )
        .expect("valid different scope");

        assert!(first.same_exact_scope(&same));
        assert_eq!(first.id(), same.id());
        assert!(!first.same_exact_scope(&different_root));
        assert_ne!(first.id(), different_root.id());
    }

    #[test]
    fn successful_and_failed_empty_scopes_have_distinct_identities() {
        let file = document("empty.py");
        let successful = scope(
            vec![file],
            Vec::new(),
            vec![DocumentCompletion::new(file, CompletionStatus::Complete)],
        )
        .expect("complete empty scope");
        let failed = scope(
            vec![file],
            Vec::new(),
            vec![DocumentCompletion::new(
                file,
                CompletionStatus::Failed("anchor-capacity".to_owned()),
            )],
        )
        .expect("failed empty scope");

        assert!(!successful.same_exact_scope(&failed));
        assert_ne!(successful.id(), failed.id());
    }

    #[test]
    fn candidate_scope_rejects_invalid_document_maps() {
        let a = document("a.py");
        let b = document("b.py");
        let complete_a = DocumentCompletion::new(a, CompletionStatus::Complete);

        assert_eq!(
            scope(
                vec![a, a],
                Vec::new(),
                vec![complete_a.clone(), complete_a.clone()]
            ),
            Err(ScopeError::DuplicateDocument(a))
        );
        assert_eq!(
            scope(vec![a, b], Vec::new(), vec![complete_a.clone()]),
            Err(ScopeError::CompletionDocumentsDiffer)
        );
        assert_eq!(
            scope(
                vec![a],
                vec![CandidateRoot::new(b, 1)],
                vec![complete_a.clone()]
            ),
            Err(ScopeError::UnknownCandidateDocument(b))
        );
        assert_eq!(
            scope(
                vec![a, b],
                vec![CandidateRoot::new(b, 1), CandidateRoot::new(a, 2)],
                vec![
                    complete_a,
                    DocumentCompletion::new(b, CompletionStatus::Complete)
                ]
            ),
            Err(ScopeError::CandidateOrder)
        );
    }
}

#[cfg(test)]
mod grouping_tests {
    use super::grouping::{ClauseScope, FailureIsolationMode, group_exact_scopes};
    use super::identity::{
        ClausePlanId, DocumentId, DocumentMapGenerationId, EffectProfileId, ScopeRecipeId,
        SnapshotId,
    };
    use super::scope::{CandidateRoot, CandidateScope, CompletionStatus, DocumentCompletion};

    fn candidate_scope(root: u32) -> CandidateScope {
        let document = DocumentId::builder().text("a.py").finish();
        CandidateScope::new(
            SnapshotId::builder().text("snapshot").finish(),
            ScopeRecipeId::builder().text("python roots").finish(),
            1,
            vec![document],
            vec![CandidateRoot::new(document, root)],
            vec![DocumentCompletion::new(
                document,
                CompletionStatus::Complete,
            )],
        )
        .expect("valid candidate scope")
    }

    fn assignment(name: &str, scope: CandidateScope) -> ClauseScope {
        ClauseScope::new(
            ClausePlanId::builder().text(name).finish(),
            scope,
            EffectProfileId::builder().text("declarative").finish(),
            DocumentMapGenerationId::builder()
                .text("document-map-v1")
                .finish(),
            FailureIsolationMode::PerFile,
        )
    }

    #[test]
    fn grouping_uses_exact_scope_and_sorts_clauses() {
        let scope = candidate_scope(7);
        let a = assignment("a", scope.clone());
        let b = assignment("b", scope);
        let expected = [a.clause(), b.clause()];

        let groups = group_exact_scopes(vec![b, a]);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].clauses(), &expected);
    }

    #[test]
    fn overlapping_scopes_never_share_a_group() {
        let groups = group_exact_scopes(vec![
            assignment("a", candidate_scope(7)),
            assignment("b", candidate_scope(8)),
        ]);

        assert_eq!(groups.len(), 2);
        assert_ne!(groups[0].scope().id(), groups[1].scope().id());
    }

    #[test]
    fn effect_map_and_failure_profiles_are_group_boundaries() {
        let scope = candidate_scope(7);
        let base = assignment("a", scope.clone());
        let other_effect = ClauseScope::new(
            ClausePlanId::builder().text("b").finish(),
            scope.clone(),
            EffectProfileId::builder().text("ordered").finish(),
            base.document_map_generation(),
            FailureIsolationMode::PerFile,
        );
        let other_map = ClauseScope::new(
            ClausePlanId::builder().text("c").finish(),
            scope.clone(),
            base.effect_profile(),
            DocumentMapGenerationId::builder()
                .text("document-map-v2")
                .finish(),
            FailureIsolationMode::PerFile,
        );
        let other_failure = ClauseScope::new(
            ClausePlanId::builder().text("d").finish(),
            scope,
            base.effect_profile(),
            base.document_map_generation(),
            FailureIsolationMode::AtomicGroup,
        );

        let groups = group_exact_scopes(vec![base, other_effect, other_map, other_failure]);

        assert_eq!(groups.len(), 4);
        assert!(groups.iter().all(|group| group.clauses().len() == 1));
    }
}

#[cfg(test)]
mod utility_tests {
    use super::identity::{
        DocumentId, ExclusionId, FrameId, LanguageContractId, RelationId, SemanticContextId,
    };
    use super::utility::{
        FrameFieldKind, UtilityCatalog, UtilityDefinition, UtilityDemand, UtilityEffects,
    };

    fn definition(effects: UtilityEffects, parameters: Vec<FrameFieldKind>) -> UtilityDefinition {
        UtilityDefinition::new(
            RelationId::builder()
                .text("canonical utility body")
                .finish(),
            LanguageContractId::builder()
                .text("python:default")
                .finish(),
            parameters,
            effects,
        )
    }

    #[test]
    fn utility_catalog_deduplicates_definitions_but_keeps_provenance() {
        let definition = definition(UtilityEffects::pure(), vec![FrameFieldKind::Node]);
        let mut catalog = UtilityCatalog::new();

        let first = catalog
            .intern("policy-a/utils/find", definition.clone())
            .expect("first definition");
        let second = catalog
            .intern("policy-b/utils/locate", definition)
            .expect("same semantic definition");

        assert_eq!(first, second);
        assert_eq!(catalog.definition_count(), 1);
        assert_eq!(
            catalog.provenance(first),
            &["policy-a/utils/find", "policy-b/utils/locate"]
        );
    }

    #[test]
    fn utility_definition_identity_covers_frames_and_effects() {
        let pure_node = definition(UtilityEffects::pure(), vec![FrameFieldKind::Node]);
        let pure_text = definition(UtilityEffects::pure(), vec![FrameFieldKind::Text]);
        let pure_sequence = definition(UtilityEffects::pure(), vec![FrameFieldKind::Sequence]);
        let clause_state = definition(
            UtilityEffects::pure().with_clause_state(),
            vec![FrameFieldKind::Node],
        );

        assert_ne!(pure_node.id(), pure_text.id());
        assert_ne!(pure_node.id(), pure_sequence.id());
        assert_ne!(pure_node.id(), clause_state.id());
    }

    #[test]
    fn utility_demand_identity_covers_the_projected_semantic_key() {
        let definition = definition(UtilityEffects::pure(), vec![FrameFieldKind::Node]);
        let document = DocumentId::builder().text("a.py").finish();
        let frame = FrameId::builder().text("A=node:7").finish();
        let context = SemanticContextId::builder().text("file-state:1").finish();
        let base = UtilityDemand::new(definition.id(), document, 7, frame, None, context, false);
        let excluded = UtilityDemand::new(
            definition.id(),
            document,
            7,
            frame,
            Some(ExclusionId::builder().text("each:1").finish()),
            context,
            false,
        );
        let three_valued =
            UtilityDemand::new(definition.id(), document, 7, frame, None, context, true);

        assert_ne!(base.id(), excluded.id());
        assert_ne!(base.id(), three_valued.id());
    }
}

#[cfg(test)]
mod semantics_tests {
    use super::identity::LogicalFillId;
    use super::semantics::{
        DeclarativeContract, DslBehavior, EffectFailure, EffectLedger, EffectSummary,
        FileEffectState, FillOwner, LogicalFill, OperationalOwner, PushedOperator,
    };

    #[test]
    fn each_order_and_dependent_bindings_have_explicit_owners() {
        assert_eq!(
            DslBehavior::AtomicPredicate.owner(),
            OperationalOwner::Declarative(DeclarativeContract::ClosedRelation)
        );
        assert_eq!(
            DslBehavior::CaptureFreeRelation.owner(),
            OperationalOwner::Declarative(DeclarativeContract::SemiJoin)
        );
        assert_eq!(
            DslBehavior::FirstWitnessRelation.owner(),
            OperationalOwner::Pushed(PushedOperator::OrderedNavigation)
        );
        assert_eq!(
            DslBehavior::IndependentAll.owner(),
            OperationalOwner::Declarative(DeclarativeContract::RelationalJoin)
        );
        assert_eq!(
            DslBehavior::IndependentAny.owner(),
            OperationalOwner::Declarative(DeclarativeContract::OrderedChoice)
        );
        assert_eq!(
            DslBehavior::DependentAll.owner(),
            OperationalOwner::Pushed(PushedOperator::OrderedRegion)
        );
        assert_eq!(
            DslBehavior::EffectfulAny.owner(),
            OperationalOwner::Pushed(PushedOperator::OrderedRegion)
        );
        assert_eq!(
            DslBehavior::Each.owner(),
            OperationalOwner::Pushed(PushedOperator::OrderedEnumeration)
        );
        assert_eq!(
            DslBehavior::AcyclicUtility.owner(),
            OperationalOwner::Pushed(PushedOperator::UtilityAnswers)
        );
        assert_eq!(
            DslBehavior::RecursiveUtility.owner(),
            OperationalOwner::Pushed(PushedOperator::RecursiveUtilityAnswers)
        );
        assert_eq!(
            DslBehavior::LogicalBudget.owner(),
            OperationalOwner::Pushed(PushedOperator::EffectLedger)
        );
    }

    fn fill(name: &str, work: u64, owner: FillOwner) -> LogicalFill {
        LogicalFill::new(LogicalFillId::builder().text(name).finish(), work, owner)
    }

    #[test]
    fn memo_replay_costs_are_logical_and_fill_ownership_is_explicit() {
        let summary = EffectSummary::new(
            2,
            3,
            vec![
                fill("file scopes", 5, FillOwner::File),
                fill("clause facts", 7, FillOwner::Clause),
            ],
        )
        .expect("canonical effects");
        let mut file = FileEffectState::new();
        let mut first_clause = EffectLedger::new(100, 8);
        let mut second_clause = EffectLedger::new(100, 8);

        assert_eq!(
            first_clause
                .apply(&summary, &mut file)
                .unwrap()
                .work_charged(),
            14
        );
        assert_eq!(
            first_clause
                .apply(&summary, &mut file)
                .unwrap()
                .work_charged(),
            2
        );
        assert_eq!(
            second_clause
                .apply(&summary, &mut file)
                .unwrap()
                .work_charged(),
            9
        );
    }

    #[test]
    fn failed_effect_application_is_transactional_at_exact_boundaries() {
        let summary = EffectSummary::new(
            2,
            3,
            vec![
                fill("file scopes", 5, FillOwner::File),
                fill("clause facts", 7, FillOwner::Clause),
            ],
        )
        .expect("canonical effects");
        let mut file = FileEffectState::new();
        let mut too_small = EffectLedger::new(13, 8);

        assert_eq!(
            too_small.apply(&summary, &mut file),
            Err(EffectFailure::WorkLimit {
                required: 14,
                remaining: 13,
            })
        );
        assert_eq!(too_small.used_work(), 0);
        assert!(file.is_empty());

        let mut exact = EffectLedger::new(14, 8);
        assert_eq!(exact.apply(&summary, &mut file).unwrap().work_charged(), 14);
        assert_eq!(exact.used_work(), 14);
    }

    #[test]
    fn depth_failure_precedes_work_and_publishes_no_fills() {
        let summary = EffectSummary::new(1, 9, vec![fill("file scopes", 1, FillOwner::File)])
            .expect("canonical effects");
        let mut file = FileEffectState::new();
        let mut ledger = EffectLedger::new(100, 8);

        assert_eq!(
            ledger.apply(&summary, &mut file),
            Err(EffectFailure::DepthLimit {
                required: 9,
                maximum: 8,
            })
        );
        assert_eq!(ledger.used_work(), 0);
        assert!(file.is_empty());
    }
}
