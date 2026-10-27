//! Phase-one production contracts. Semantic conformance belongs to later phases.
use super::*;
use crate::code_rule_dsl::tape::{Grammar, TapeImage, TapeSource};
use ast_grep_core::tree_sitter::StrDoc;
use crate::code_rule_dsl::language::GrammarDialect as SupportLang;

fn input() -> (Program, TapeSource, Vec<Vec<Arc<[u32]>>>) {
    let source = Arc::new("name\n".to_owned());
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let native = StrDoc::new(source.as_str(), SupportLang::Python);
    let image =
        Arc::new(TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap());
    let lines = Arc::new(crate::code_rule_dsl::lines::LineTable::new(&source, crate::code_rule_dsl::lines::Terminators::Newlines));
    let program =
        Program::compile_tree_sitter_query_pattern("(identifier) @name", &grammar.language())
            .unwrap();
    let anchors = vec![vec![Arc::from(
        (0..image.primary.node_count() as u32).collect::<Vec<_>>(),
    )]];
    (
        program,
        TapeSource {
            source,
            grammar,
            image,
            lines,
        },
        anchors,
    )
}

fn dynamic_input() -> (Program, TapeSource, Vec<Vec<Arc<[u32]>>>) {
    let (mut program, file, mut anchors) = input();
    program.words.insert(0, word(DESCEND, 0, 0));
    program.validate().unwrap();
    anchors[0][0] = Arc::from([0_u32]);
    (program, file, anchors)
}

#[test]
fn phase_three_production_pages_publish_complete_dynamic_results() {
    let (program, file, anchors) = dynamic_input();
    let doc = file.restore(&|| false).unwrap();
    let cpu = execute_batch(
        std::slice::from_ref(&program),
        &[doc],
        Some(&[vec![anchors[0][0].to_vec()]]),
    )
    .unwrap();
    assert_eq!(
        cpu[0][0].len(),
        1,
        "fixture must produce one complete result"
    );
    let mut publications = Vec::new();
    let result = execute_production_corpus_pages(
        &[program],
        &[file],
        &anchors,
        &[100],
        &mut |page, complete| {
            publications.push((page.into_outcome()?.into_result()?, complete));
            Ok(())
        },
    );
    assert_eq!(result, Ok(()));
    assert_eq!(publications, vec![(cpu, 1)]);
}

#[test]
fn phase_three_batch_entry_publishes_complete_dynamic_results() {
    let (program, file, anchors) = dynamic_input();
    let doc = file.restore(&|| false).unwrap();
    let expected = execute_batch(
        std::slice::from_ref(&program),
        std::slice::from_ref(&doc),
        Some(&[vec![anchors[0][0].to_vec()]]),
    )
    .unwrap();
    let result = execute_production_corpus(&[program], &[doc], &anchors, &[100])
        .unwrap()
        .into_outcome()
        .unwrap()
        .into_result();
    assert_eq!(result, Ok(expected));
}

#[test]
fn phase_one_invalid_input_is_rejected_before_the_stage_error() {
    let (program, file, anchors) = input();
    for (invalid_anchors, limits) in [
        (Vec::new(), vec![100]),
        (vec![Vec::new()], vec![100]),
        (vec![vec![Arc::from([u32::MAX])]], vec![100]),
        (anchors, Vec::new()),
    ] {
        let mut publications = 0;
        let result = execute_production_corpus_pages(
            std::slice::from_ref(&program),
            std::slice::from_ref(&file),
            &invalid_anchors,
            &limits,
            &mut |_, _| {
                publications += 1;
                Ok(())
            },
        );
        assert_eq!(result, Err(Error::InvalidAnchor));
        assert_eq!(publications, 0);
    }
}

#[test]
fn phase_one_shared_input_keeps_distinct_roots_and_exact_signature_blocks() {
    let (program, file, anchors) = input();
    let programs = vec![program.clone(), program];
    let anchors = vec![vec![anchors[0][0].clone(), anchors[0][0].clone()]];
    let doc = file.restore(&|| false).unwrap();
    let input = metal_grid::MetalGridInput::build_shared(&programs, &[doc], &anchors).unwrap();
    let shapes = metal_grid::compile_program_shapes(&programs);
    assert_eq!(input.grid.header.root_count, anchors[0][0].len() as u32);
    assert_eq!(
        input.grid.header.semantic_tuple_count,
        2 * input.grid.header.root_count
    );
    assert_eq!(input.grid.header.selector_count, 1);
    assert!(input.packed.tasks.is_empty());
    assert_eq!(input.packed.header.task_count, 0);
    let mut covered = 0;
    for block in &input.grid.blocks {
        assert!((1..=256).contains(&block.logical_count));
        for span in &input.grid.block_spans[block.block_span_start as usize
            ..(block.block_span_start + block.block_span_count) as usize]
        {
            let evaluation = input.grid.evaluation_spans[span.evaluation_span as usize];
            assert_eq!(
                shapes[evaluation.program as usize].dispatch_signature,
                block.dispatch_signature
            );
            assert_eq!(evaluation.route, block.route);
            covered += span.root_count;
        }
    }
    assert_eq!(covered, input.grid.header.semantic_tuple_count);
}

#[test]
fn phase_one_plan_keeps_broad_stages_as_blocks_increase() {
    use super::metal_grid_plan::{ExecutionPlan, ResidentInput, StageKind, WorkDomain};
    let (program, file, _) = input();
    let doc = file.restore(&|| false).unwrap();
    let plans = [1, 2048].map(|count| {
        let programs = vec![program.clone(); count];
        let anchors = vec![vec![Arc::<[u32]>::from([1]); count]];
        let resident = ResidentInput::new(
            metal_grid::MetalGridInput::build_shared(
                &programs,
                std::slice::from_ref(&doc),
                &anchors,
            )
            .unwrap(),
            None,
        )
        .unwrap();
        ExecutionPlan::new(&resident, &programs).unwrap()
    });
    assert_eq!(plans[0].stages.len(), plans[1].stages.len());
    for plan in &plans {
        assert_eq!(plan.stages.first().unwrap().kind, StageKind::ClassifyRoots);
        assert_eq!(
            plan.stages.last().unwrap().kind,
            StageKind::ScatterTypedPage
        );
        let counts = plan
            .stages
            .iter()
            .filter(|stage| stage.kind == StageKind::CountDirectBlocks)
            .collect::<Vec<_>>();
        assert_eq!(counts.len(), 1);
        let WorkDomain::Blocks(groups) = &counts[0].work else {
            panic!("direct work must use signature blocks")
        };
        assert_eq!(
            groups.len(),
            1,
            "identical signatures must share a broad stage"
        );
        for (index, stage) in plan.stages.iter().enumerate() {
            assert_eq!(stage.after, index.checked_sub(1));
        }
        let scan = plan
            .stages
            .iter()
            .position(|stage| stage.kind == StageKind::ScanSemanticCounts)
            .unwrap();
        let boundary = plan
            .stages
            .iter()
            .position(|stage| stage.kind == StageKind::SelectOutputPage)
            .unwrap();
        assert!(scan < boundary && boundary < plan.stages.len() - 1);
    }
    let WorkDomain::Blocks(groups) = &plans[1].stages[1].work else {
        panic!("missing broad direct stage")
    };
    assert_eq!(groups[0].blocks.len(), 8);
}

#[test]
fn phase_one_plan_rejects_conflicting_identity_and_coverage() {
    use super::metal_grid::MetalGridInput;
    use super::metal_grid_plan::{ExecutionPlan, ResidentInput};
    let (program, file, anchors) = input();
    let doc = file.restore(&|| false).unwrap();
    let corruptions: [fn(&mut MetalGridInput); 7] = [
        |input| input.grid.blocks[0].dispatch_signature ^= 1,
        |input| input.grid.evaluation_spans[0].class = 255,
        |input| input.grid.evaluation_spans[0].program = u32::MAX,
        |input| input.grid.block_spans[0].semantic_start += 1,
        |input| input.grid.blocks[0].logical_count = 257,
        |input| input.grid.header.format_version = u32::MAX,
        |input| input.grid.header.root_count += 1,
    ];
    for corrupt in corruptions {
        let mut input = MetalGridInput::build_shared(
            std::slice::from_ref(&program),
            std::slice::from_ref(&doc),
            &anchors,
        )
        .unwrap();
        corrupt(&mut input);
        let result = ExecutionPlan::new(
            &ResidentInput::new(input, None).unwrap(),
            std::slice::from_ref(&program),
        );
        assert_eq!(result.err(), Some(Error::InvalidAnchor));
    }
}

#[test]
fn phase_one_dynamic_work_uses_a_bounded_compact_frontier_stage() {
    use super::metal_grid_plan::{ExecutionPlan, ResidentInput, StageKind, WorkDomain};
    let (_, file, anchors) = input();
    let program = Program::new(
        vec![word(DESCEND, 0, 0), word(EMIT, 0, 0)],
        vec![b"match".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let doc = file.restore(&|| false).unwrap();
    let resident = ResidentInput::new(
        metal_grid::MetalGridInput::build_shared(std::slice::from_ref(&program), &[doc], &anchors)
            .unwrap(),
        None,
    )
    .unwrap();
    let plan = ExecutionPlan::new(&resident, &[program]).unwrap();
    assert_eq!(
        plan.stages
            .iter()
            .filter(|stage| stage.kind == StageKind::CountDirectBlocks)
            .count(),
        0
    );
    let seed = plan
        .stages
        .iter()
        .position(|stage| stage.kind == StageKind::SeedDynamicFrontier)
        .unwrap();
    let wave = plan
        .stages
        .iter()
        .position(|stage| stage.kind == StageKind::AdvanceDynamicFrontier)
        .unwrap();
    assert!(seed < wave);
    assert_eq!(
        plan.stages[wave].work,
        WorkDomain::CompactFrontier { checked_waves: 16 }
    );
}

#[test]
fn phase_one_prepared_input_preserves_limits_without_coercion_or_aliasing() {
    use super::metal_grid_plan::{ExecutionPlan, ResidentInput};
    let (program, file, anchors) = input();
    let programs = [program.clone(), program];
    let doc = file.restore(&|| false).unwrap();
    let anchors = [vec![anchors[0][0].clone(), anchors[0][0].clone()]];
    let prepare = || {
        metal_grid::MetalGridInput::build_shared(&programs, std::slice::from_ref(&doc), &anchors)
            .unwrap()
    };
    let mut limits = [0, 7];
    let mut resident = ResidentInput::new(prepare(), Some(&limits)).unwrap();
    limits[1] = u32::MAX;
    assert_eq!(resident.witness_limits, [0, 7]);
    assert_eq!(limits, [0, u32::MAX]);
    assert!(ExecutionPlan::new(&resident, &programs).is_ok());
    resident.witness_limits.push(9);
    assert_eq!(
        ExecutionPlan::new(&resident, &programs).err(),
        Some(Error::InvalidAnchor)
    );
    assert_eq!(
        ResidentInput::new(prepare(), Some(&[1])).err(),
        Some(Error::InvalidAnchor)
    );
    assert_eq!(
        ResidentInput::new(prepare(), None).unwrap().witness_limits,
        [u32::MAX; 2]
    );
}

#[test]
fn phase_one_executor_rejects_inconsistent_stage_domains_and_dependencies() {
    use super::metal_grid_executor::GuardedExecutor;
    use super::metal_grid_plan::{ExecutionPlan, ResidentInput, WorkDomain};
    let (program, file, anchors) = input();
    let doc = file.restore(&|| false).unwrap();
    let corruptions: [fn(&mut ExecutionPlan); 5] = [
        |plan| plan.stages[0].work = WorkDomain::TypedPages,
        |plan| plan.stages[1].after = Some(1),
        |plan| {
            plan.stages.remove(0);
        },
        |plan| {
            plan.stages.pop();
        },
        |plan| {
            plan.stages.swap(2, 3);
        },
    ];
    for (index, corrupt) in corruptions.into_iter().enumerate() {
        let resident = ResidentInput::new(
            metal_grid::MetalGridInput::build_shared(
                std::slice::from_ref(&program),
                std::slice::from_ref(&doc),
                &anchors,
            )
            .unwrap(),
            None,
        )
        .unwrap();
        let mut plan = ExecutionPlan::new(&resident, std::slice::from_ref(&program)).unwrap();
        corrupt(&mut plan);
        assert_eq!(
            GuardedExecutor::new(resident, plan).err(),
            Some(Error::InvalidAnchor),
            "malformed stage plan {index} must fail before an executor can own it"
        );
    }
}

#[test]
fn phase_one_executor_rejects_invalid_frontier_wave_bounds() {
    use super::metal_grid_executor::GuardedExecutor;
    use super::metal_grid_plan::{ExecutionPlan, ResidentInput, StageKind, WorkDomain};
    let (_, file, anchors) = input();
    let program = Program::new(
        vec![word(DESCEND, 0, 0), word(EMIT, 0, 0)],
        vec![b"match".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let doc = file.restore(&|| false).unwrap();
    for checked_waves in [0, 17, u32::MAX] {
        let resident = ResidentInput::new(
            metal_grid::MetalGridInput::build_shared(
                std::slice::from_ref(&program),
                std::slice::from_ref(&doc),
                &anchors,
            )
            .unwrap(),
            None,
        )
        .unwrap();
        let mut plan = ExecutionPlan::new(&resident, std::slice::from_ref(&program)).unwrap();
        plan.stages
            .iter_mut()
            .find(|stage| stage.kind == StageKind::AdvanceDynamicFrontier)
            .unwrap()
            .work = WorkDomain::CompactFrontier { checked_waves };
        assert_eq!(
            GuardedExecutor::new(resident, plan).err(),
            Some(Error::InvalidAnchor),
            "invalid wave bound {checked_waves} must not enter the executor"
        );
    }
}

#[test]
fn phase_one_distinct_roots_scale_work_without_adding_broad_stages() {
    use super::metal_grid_plan::{ExecutionPlan, ResidentInput, StageKind, WorkDomain};
    let source = Arc::new(
        (0..800)
            .map(|index| format!("name_{index}\n"))
            .collect::<String>(),
    );
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let native = StrDoc::new(source.as_str(), SupportLang::Python);
    let image =
        Arc::new(TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap());
    let file = TapeSource {
        lines: Arc::new(crate::code_rule_dsl::lines::LineTable::new(&source, crate::code_rule_dsl::lines::Terminators::Newlines)),
        source,
        grammar,
        image,
    };
    let program =
        Program::compile_tree_sitter_query_pattern("(identifier) @name", &file.grammar.language())
            .unwrap();
    let doc = file.restore(&|| false).unwrap();
    let plans = [8_u32, 800].map(|count| {
        let anchors = [vec![Arc::<[u32]>::from((1..=count).collect::<Vec<_>>())]];
        let resident = ResidentInput::new(
            metal_grid::MetalGridInput::build_shared(
                std::slice::from_ref(&program),
                std::slice::from_ref(&doc),
                &anchors,
            )
            .unwrap(),
            None,
        )
        .unwrap();
        assert_eq!(resident.input.grid.header.root_count, count);
        assert_eq!(resident.input.grid.header.semantic_tuple_count, count);
        assert!(resident.input.packed.tasks.is_empty());
        let covered: u32 = resident
            .input
            .grid
            .blocks
            .iter()
            .map(|block| block.logical_count)
            .sum();
        assert_eq!(covered, count);
        ExecutionPlan::new(&resident, std::slice::from_ref(&program)).unwrap()
    });
    assert_eq!(
        plans[0]
            .stages
            .iter()
            .map(|stage| stage.kind)
            .collect::<Vec<_>>(),
        plans[1]
            .stages
            .iter()
            .map(|stage| stage.kind)
            .collect::<Vec<_>>()
    );
    for (plan, expected_blocks) in plans.iter().zip([1, 4]) {
        let stage = plan
            .stages
            .iter()
            .find(|stage| stage.kind == StageKind::CountDirectBlocks)
            .unwrap();
        let WorkDomain::Blocks(groups) = &stage.work else {
            panic!("expected signature blocks")
        };
        assert_eq!(
            groups.iter().map(|group| group.blocks.len()).sum::<usize>(),
            expected_blocks
        );
    }
}

#[test]
fn phase_one_resource_handoffs_reject_wrong_missing_and_future_producers() {
    use super::metal_grid_executor::GuardedExecutor;
    use super::metal_grid_plan::{ExecutionPlan, ProgressBoundary, ResidentInput, ResourceKind};
    let (program, file, anchors) = input();
    let doc = file.restore(&|| false).unwrap();
    let corruptions: [fn(&mut ExecutionPlan); 8] = [
        |plan| plan.stages.last_mut().unwrap().contract.continuation = None,
        |plan| {
            plan.stages
                .last_mut()
                .unwrap()
                .contract
                .continuation
                .as_mut()
                .unwrap()
                .resume_at = super::metal_grid_plan::StageKind::ClassifyRoots
        },
        |plan| {
            let index = plan.stages.len() - 2;
            plan.stages[index].contract.resume_input = None;
        },
        |plan| plan.stages[1].contract.inputs[0].producer = Some(1),
        |plan| plan.stages[1].contract.inputs.clear(),
        |plan| plan.stages[0].contract.outputs = vec![ResourceKind::TypedPage],
        |plan| plan.stages.last_mut().unwrap().contract.progress = ProgressBoundary::None,
        |plan| plan.stages.last_mut().unwrap().contract.inputs[1].producer = Some(0),
    ];
    for corrupt in corruptions {
        let resident = ResidentInput::new(
            metal_grid::MetalGridInput::build_shared(
                std::slice::from_ref(&program),
                std::slice::from_ref(&doc),
                &anchors,
            )
            .unwrap(),
            None,
        )
        .unwrap();
        let mut plan = ExecutionPlan::new(&resident, std::slice::from_ref(&program)).unwrap();
        corrupt(&mut plan);
        assert_eq!(
            GuardedExecutor::new(resident, plan).err(),
            Some(Error::InvalidAnchor)
        );
    }
}

#[test]
fn phase_one_counts_failures_and_pages_use_their_actual_producers() {
    use super::metal_grid_plan::{
        ExecutionPlan, ProgressBoundary, ResidentInput, ResourceKind, StageKind,
    };
    let (program, file, anchors) = input();
    let predicate_program = program
        .clone()
        .with_capture_predicates(vec![CapturePredicate::SameLine {
            left: 0,
            right: 0,
            negated: false,
        }])
        .unwrap();
    let dynamic = Program::new(
        vec![word(DESCEND, 0, 0), word(EMIT, 0, 0)],
        vec![b"match".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let bounded = Program::new(
        vec![word(PARENT, 0, 0), word(EMIT, 0, 0)],
        vec![b"match".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let programs = [program, predicate_program, bounded, dynamic];
    let doc = file.restore(&|| false).unwrap();
    let anchors = [vec![anchors[0][0].clone(); programs.len()]];
    let resident = ResidentInput::new(
        metal_grid::MetalGridInput::build_shared(&programs, &[doc], &anchors).unwrap(),
        None,
    )
    .unwrap();
    let plan = ExecutionPlan::new(&resident, &programs).unwrap();
    for (index, stage) in plan.stages.iter().enumerate() {
        assert!(!stage.contract.outputs.is_empty());
        if let Some(continuation) = stage.contract.continuation {
            assert!(stage.contract.outputs.contains(&continuation.checkpoint));
            let target = plan
                .stages
                .iter()
                .find(|target| target.kind == continuation.resume_at)
                .unwrap();
            assert_eq!(target.contract.resume_input, Some(continuation.checkpoint));
        }
        for input in &stage.contract.inputs {
            if let Some(producer) = input.producer {
                assert!(producer < index);
                assert!(plan.stages[producer].contract.outputs.contains(&input.kind));
            } else {
                assert_eq!(input.kind, ResourceKind::ResidentInput);
            }
        }
    }
    let failures = plan
        .stages
        .iter()
        .position(|stage| stage.kind == StageKind::ReduceFailures)
        .unwrap();
    let producers = plan.stages[..failures]
        .iter()
        .enumerate()
        .filter(|(_, stage)| stage.contract.outputs.contains(&ResourceKind::Failures))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    assert!(
        producers.len() == 6,
        "direct, bounded, seed, readiness, wave, and predicate stages must all report failures"
    );
    assert_eq!(
        plan.stages[failures]
            .contract
            .inputs
            .iter()
            .map(|input| input.producer.unwrap())
            .collect::<Vec<_>>(),
        producers
    );
    let scan = plan
        .stages
        .iter()
        .position(|stage| stage.kind == StageKind::ScanSemanticCounts)
        .unwrap();
    let predicate = plan
        .stages
        .iter()
        .position(|stage| stage.kind == StageKind::CompactPredicates)
        .unwrap();
    assert_eq!(
        plan.stages[scan].contract.inputs[0].kind,
        ResourceKind::PredicateCounts
    );
    assert_eq!(
        plan.stages[scan].contract.inputs[0].producer,
        Some(predicate)
    );
    assert!(
        plan.stages[scan]
            .contract
            .inputs
            .iter()
            .any(|input| input.producer == Some(failures))
    );
    let boundary = plan
        .stages
        .iter()
        .position(|stage| stage.kind == StageKind::SelectOutputPage)
        .unwrap();
    let scatter = plan.stages.last().unwrap();
    assert!(
        scatter
            .contract
            .inputs
            .iter()
            .any(|input| input.producer == Some(boundary))
    );
    assert!(
        scatter
            .contract
            .inputs
            .iter()
            .any(|input| input.producer == Some(scan))
    );
    let match_producers = scatter
        .contract
        .inputs
        .iter()
        .filter(|input| input.kind == ResourceKind::AcceptedMatches)
        .map(|input| plan.stages[input.producer.unwrap()].kind)
        .collect::<Vec<_>>();
    assert_eq!(
        match_producers,
        [
            StageKind::AdvanceDynamicFrontier,
            StageKind::CompactPredicates
        ]
    );
    assert!(
        !scatter
            .contract
            .inputs
            .iter()
            .any(|input| input.kind == ResourceKind::Candidates)
    );
    assert_eq!(scatter.contract.progress, ProgressBoundary::Output);
    assert_eq!(
        plan.stages
            .iter()
            .map(|stage| stage.kind)
            .collect::<Vec<_>>(),
        [
            StageKind::ClassifyRoots,
            StageKind::CountDirectBlocks,
            StageKind::CountBoundedBlocks,
            StageKind::SeedDynamicFrontier,
            StageKind::ScheduleDynamicFrontier,
            StageKind::AdvanceDynamicFrontier,
            StageKind::CompactPredicates,
            StageKind::ReduceFailures,
            StageKind::ScanSemanticCounts,
            StageKind::SelectOutputPage,
            StageKind::ScatterTypedPage
        ],
        "mixed work must retain the design's complete broad stage sequence"
    );
    let executor = super::metal_grid_executor::GuardedExecutor::new(resident, plan).unwrap();
    use super::metal_grid_plan::{ContinuationPoint, FrontierKind};
    let point = |kind, generation| ContinuationPoint::Frontier { kind, generation };
    let before = point(FrontierKind::Predicate, 2);
    assert_eq!(
        executor.check_continuation(predicate, before, point(FrontierKind::Predicate, 3)),
        Ok(StageKind::CompactPredicates)
    );
    for after in [
        before,
        point(FrontierKind::Predicate, 1),
        point(FrontierKind::Structural, 3),
    ] {
        assert_eq!(
            executor.check_continuation(predicate, before, after),
            Err(Error::BackendFailure)
        );
    }
}

#[test]
fn phase_one_continuations_require_semantic_advance_and_exact_output_prefixes() {
    use super::metal_grid_executor::GuardedExecutor;
    use super::metal_grid_plan::{
        ContinuationPoint, ExecutionPlan, FrontierKind, OutputBoundary, ResidentInput, StageKind,
    };
    let (_, file, anchors) = input();
    let program = Program::new(
        vec![word(DESCEND, 0, 0), word(EMIT, 0, 0)],
        vec![b"match".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let doc = file.restore(&|| false).unwrap();
    let resident = ResidentInput::new(
        metal_grid::MetalGridInput::build_shared(std::slice::from_ref(&program), &[doc], &anchors)
            .unwrap(),
        None,
    )
    .unwrap();
    let plan = ExecutionPlan::new(&resident, &[program]).unwrap();
    let wave = plan
        .stages
        .iter()
        .position(|stage| stage.kind == StageKind::AdvanceDynamicFrontier)
        .unwrap();
    let scatter = plan.stages.len() - 1;
    let executor = GuardedExecutor::new(resident, plan).unwrap();
    let frontier = |kind, generation| ContinuationPoint::Frontier { kind, generation };
    let before = frontier(FrontierKind::Structural, 7);
    assert_eq!(
        executor.check_continuation(wave, before, frontier(FrontierKind::Structural, 8)),
        Ok(StageKind::ScheduleDynamicFrontier)
    );
    for after in [
        before,
        frontier(FrontierKind::Structural, 6),
        frontier(FrontierKind::Predicate, 8),
    ] {
        assert_eq!(
            executor.check_continuation(wave, before, after),
            Err(Error::BackendFailure)
        );
    }
    let output = |span, lane, ordinal, generation| {
        ContinuationPoint::Output(OutputBoundary {
            semantic_span: span,
            block_lane: lane,
            local_output_ordinal: ordinal,
            frontier_generation: generation,
        })
    };
    let before = output(0, 0, 3, 8);
    assert_eq!(
        executor.check_continuation(scatter, before, output(0, 0, 4, 8)),
        Ok(StageKind::SelectOutputPage)
    );
    assert_eq!(
        executor.check_continuation(scatter, before, output(0, 1, 0, 8)),
        Ok(StageKind::SelectOutputPage)
    );
    for after in [
        before,
        output(0, 0, 2, 8),
        output(0, 0, 4, 9),
        frontier(FrontierKind::Structural, 9),
    ] {
        assert_eq!(
            executor.check_continuation(scatter, before, after),
            Err(Error::BackendFailure)
        );
    }
    assert_eq!(
        executor.check_continuation(scatter, before, output(1, 0, 0, 8)),
        Ok(StageKind::SelectOutputPage)
    );
    for after in [
        output(u32::MAX, 0, 0, 8),
        output(0, 256, 0, 8),
        output(1, 1, 0, 8),
    ] {
        assert_eq!(
            executor.check_continuation(scatter, before, after),
            Err(Error::InvalidAnchor)
        );
    }
    assert_eq!(
        executor.check_continuation(0, before, output(0, 0, 4, 8)),
        Err(Error::BackendFailure)
    );
}

#[test]
fn phase_one_direct_scatter_does_not_require_candidate_materialization() {
    use super::metal_grid_plan::{ExecutionPlan, ResidentInput, ResourceKind, StageKind};
    let (program, file, anchors) = input();
    let doc = file.restore(&|| false).unwrap();
    let resident = ResidentInput::new(
        metal_grid::MetalGridInput::build_shared(std::slice::from_ref(&program), &[doc], &anchors)
            .unwrap(),
        None,
    )
    .unwrap();
    let plan = ExecutionPlan::new(&resident, &[program]).unwrap();
    assert!(!plan.stages.iter().any(|stage| matches!(
        stage.kind,
        StageKind::AdvanceDynamicFrontier | StageKind::CompactPredicates
    )));
    let scatter = plan.stages.last().unwrap();
    assert_eq!(
        scatter
            .contract
            .inputs
            .iter()
            .map(|input| input.kind)
            .collect::<Vec<_>>(),
        [
            ResourceKind::ResidentInput,
            ResourceKind::ReducedFailures,
            ResourceKind::SemanticOffsets,
            ResourceKind::ExactPageBoundary
        ],
        "direct scatter must rerun register semantics without materialized candidates"
    );
}
