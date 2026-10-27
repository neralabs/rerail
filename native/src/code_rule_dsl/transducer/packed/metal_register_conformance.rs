//! Behavioral phase-two gates. The interpreter is an independent test oracle only.
use super::metal_backend::launch::{DispatchRecord, LaunchScope};
use super::metal_grid::MetalGridInput;
use super::metal_grid_executor::GuardedExecutor;
use super::metal_grid_plan::{ExecutionPlan, ResidentInput};
use super::*;
use crate::code_rule_dsl::tape::{Grammar, TapeImage, TapeSource};
use ast_grep_core::tree_sitter::StrDoc;
use crate::code_rule_dsl::language::GrammarDialect as SupportLang;

fn file(lines: usize) -> TapeSource {
    source_file("name\n".repeat(lines))
}

pub(super) fn source_file(text: String) -> TapeSource {
    let source = Arc::new(text);
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let native = StrDoc::new(source.as_str(), SupportLang::Python);
    let image =
        Arc::new(TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap());
    let lines = Arc::new(crate::code_rule_dsl::lines::LineTable::new(&source, crate::code_rule_dsl::lines::Terminators::Newlines));
    TapeSource {
        source,
        grammar,
        image,
        lines,
    }
}

#[test]
fn broad_register_counts_preserve_every_task_with_one_scan_before_each_boundary() {
    let file = file(1);
    let doc = file.restore(&|| false).unwrap();
    let program =
        Program::compile_tree_sitter_query_pattern("(identifier) @name", &file.grammar.language())
            .unwrap();
    let programs = vec![program; 1025];
    let roots = vec![vec![vec![2]; programs.len()]];
    assert_eq!(doc.transducer_text(2), b"name");
    let expected = execute_batch(&programs, std::slice::from_ref(&doc), Some(&roots)).unwrap();
    let resident = ResidentInput::new(
        MetalGridInput::build(&programs, &[doc], &roots).unwrap(),
        None,
    )
    .unwrap();
    let plan = ExecutionPlan::new(&resident, &programs).unwrap();
    let scope = LaunchScope::enter();
    let actual = GuardedExecutor::new(resident, plan)
        .unwrap()
        .execute()
        .unwrap()
        .into_outcome()
        .unwrap()
        .into_result()
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual[0].len(), programs.len());
    for (program, rows) in actual[0].iter().enumerate() {
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].task_ordinal, program as u32);
        assert_eq!(rows[0].captures.len(), 1);
        assert_eq!(rows[0].captures[0].span, 0..4);
    }
    let trace = scope.snapshot();
    let mut scans = 0;
    let mut boundaries = 0;
    for record in &trace {
        assert!(record.encoded && record.submitted && record.completed);
        if record.kernel == "register_scan" || record.kernel == "register_add_parent" {
            scans += 1;
        }
        if record.kernel == "register_boundary" {
            assert_eq!(
                scans, 1,
                "one complete broad count scan must precede each boundary"
            );
            scans = 0;
            boundaries += 1;
        }
    }
    assert_eq!(boundaries, 1);
    assert_eq!(scans, 0);
}

#[test]
fn register_proofs_preserve_cursor_frame_truncation() {
    let file = file(1);
    let docs = [file.restore(&|| false).unwrap()];
    let program = Program::new(
        vec![
            word(SAVE_CURSOR, 0, 500),
            word(SAVE_CURSOR, 0, 0),
            word(RESTORE_CURSOR, 0, 500),
            word(EMIT, 0, 0),
        ],
        vec![b"node".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    // The interpreter resizes the frame on every save, including a smaller save.
    assert_eq!(
        program.execute_roots(&docs[0], [0]),
        Err(Error::InvalidOperand(2))
    );
    assert!(
        super::execution_shape::register_paths(&program.words).is_none(),
        "an invalidated cursor must not read a stale register and fabricate a match"
    );
    let resident = ResidentInput::new(
        MetalGridInput::build(std::slice::from_ref(&program), &docs, &[vec![vec![0]]]).unwrap(),
        None,
    )
    .unwrap();
    assert!(
        ExecutionPlan::new(&resident, std::slice::from_ref(&program))
            .unwrap()
            .stages
            .iter()
            .any(|stage| stage.kind == super::metal_grid_plan::StageKind::SeedDynamicFrontier)
    );

    let mut restored = program.clone();
    restored.words.insert(2, word(SAVE_CURSOR, 0, 500));
    assert!(super::execution_shape::register_paths(&restored.words).is_some());
    assert_eq!(restored.execute_roots(&docs[0], [0]).unwrap().len(), 1);
    restored.words = vec![
        word(SAVE_CURSOR, 0, 0),
        word(SAVE_CURSOR, 0, 500),
        word(RESTORE_CURSOR, 0, 0),
        word(EMIT, 0, 0),
    ];
    assert!(super::execution_shape::register_paths(&restored.words).is_some());
    assert_eq!(restored.execute_roots(&docs[0], [0]).unwrap().len(), 1);
}

fn programs(grammar: &Grammar) -> Vec<Program> {
    let direct =
        Program::compile_tree_sitter_query_pattern("(identifier) @name", &grammar.language())
            .unwrap();
    let mut bounded = direct.clone();
    // Fixed navigation keeps capture identity, root identity, and semantic ordinals distinct.
    bounded.words.splice(
        1..1,
        [
            word(SAVE_CURSOR, 0, 500),
            word(PARENT, 0, 0),
            word(RESTORE_CURSOR, 0, 500),
        ],
    );
    bounded.validate().unwrap();
    vec![direct, bounded]
}

fn anchors(programs: &[Program], docs: &[TapeDoc]) -> Vec<Vec<Arc<[u32]>>> {
    docs.iter()
        .map(|doc| {
            vec![Arc::from((0..doc.transducer_len() as u32).collect::<Vec<_>>()); programs.len()]
        })
        .collect()
}

fn run(lines: usize, partition_spans: u32) -> (Vec<Vec<Vec<Output>>>, Vec<DispatchRecord>) {
    let file = file(lines);
    let programs = programs(&file.grammar);
    let docs = [file.restore(&|| false).unwrap()];
    let kind = file.grammar.language().id_for_node_kind("identifier", true);
    let roots = Arc::<[u32]>::from(
        (0..docs[0].transducer_len() as u32)
            .filter(|&node| docs[0].transducer_kind(node) == kind)
            .collect::<Vec<_>>(),
    );
    assert_eq!(
        roots.len(),
        lines,
        "root scaling must be exactly 100-fold, not an approximation from source line counts"
    );
    let anchors = vec![vec![roots; programs.len()]];
    let plain = anchors
        .iter()
        .map(|file| file.iter().map(|roots| roots.to_vec()).collect())
        .collect::<Vec<Vec<Vec<u32>>>>();
    let expected = execute_batch(&programs, &docs, Some(&plain)).unwrap();
    let resident = ResidentInput::new(
        MetalGridInput::build_shared(&programs, &docs, &anchors).unwrap(),
        None,
    )
    .unwrap();
    let count_capacity = resident
        .input
        .grid
        .block_spans
        .len()
        .min(partition_spans as usize);
    let direct_blocks = resident
        .input
        .grid
        .blocks
        .iter()
        .filter(|block| {
            let span = resident.input.grid.block_spans[block.block_span_start as usize];
            resident.input.grid.evaluation_spans[span.evaluation_span as usize].class < 2
        })
        .count() as u64;
    let bounded_blocks = resident.input.grid.blocks.len() as u64 - direct_blocks;
    assert_eq!(resident.input.grid.header.root_count as usize, lines);
    let plan = ExecutionPlan::new(&resident, &programs).unwrap();
    let scope = LaunchScope::enter();
    let actual = GuardedExecutor::new(resident, plan)
        .unwrap()
        .with_partition_span_limit(partition_spans)
        .unwrap()
        .execute()
        .expect("phase two must execute real direct and fixed navigation GPU stages")
        .into_outcome()
        .unwrap()
        .into_result()
        .unwrap();
    assert_eq!(
        actual, expected,
        "GPU output must preserve every CPU tuple, capture, and ordinal"
    );
    let trace = scope.snapshot();
    assert_eq!(
        trace
            .iter()
            .filter(|record| record.kernel == "register_direct_count")
            .map(|record| record.grid[0])
            .sum::<u64>(),
        direct_blocks
    );
    assert_eq!(
        trace
            .iter()
            .filter(|record| record.kernel == "register_bounded_count")
            .map(|record| record.grid[0])
            .sum::<u64>(),
        bounded_blocks
    );
    assert!(!trace.is_empty());
    assert!(
        trace
            .iter()
            .all(|record| record.encoded && record.submitted && record.completed)
    );
    assert!(
        trace
            .iter()
            .filter(|record| record.kernel == "register_scan")
            .all(|record| if record.buffers.len() == 2 {
                record.buffers[0] != record.buffers[1]
            } else {
                record.buffers[0] == record.buffers[1]
            }),
        "cooperative totals need a distinct owner; hierarchy counts become offsets in place"
    );
    assert!(trace.iter().all(|record| record.application_buffer_bytes
        <= super::metal_backend::METAL_BUFFER_LIMIT
        && record.private_buffer_bytes <= record.application_buffer_bytes
        && record.application_buffer_bytes <= record.peak_application_buffer_bytes
        && record.peak_application_buffer_bytes <= super::metal_backend::METAL_BUFFER_LIMIT));
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as usize;
    let expected_count_bytes = (count_capacity * 16).max(1).div_ceil(page_size) * page_size;
    assert!(
        trace
            .iter()
            .filter(|record| record.kernel.starts_with("register_direct")
                || record.kernel.starts_with("register_bounded"))
            .all(|record| record.buffers[3].1 as usize <= expected_count_bytes),
        "global counts must scale with block spans, not semantic tuples"
    );
    assert!(
        trace
            .iter()
            .all(|record| record.group == [256, 1, 1] && !record.indirect)
    );
    assert!(
        trace
            .iter()
            .all(|record| !record.kernel.contains("frontier"))
    );
    let input = trace
        .iter()
        .find(|record| record.kernel == "register_roots")
        .unwrap()
        .buffers[0];
    assert!(
        trace
            .iter()
            .filter(|record| record.kernel.starts_with("register_direct")
                || record.kernel.starts_with("register_bounded")
                || record.kernel == "register_scatter")
            .all(|record| record.buffers[0] == input),
        "actual bindings must share the same immutable input across every partition"
    );
    eprintln!(
        "TEMPO_REGISTER_TRACE {}",
        serde_json::json!({"lines":lines,"partitionSpans":partition_spans,"dispatches":&trace})
    );
    (actual, trace)
}

#[test]
fn phase_two_invalid_resident_records_fail_before_gpu_access() {
    let file = file(3);
    let programs = programs(&file.grammar);
    let docs = [file.restore(&|| false).unwrap()];
    let anchors = anchors(&programs, &docs);
    let corruptions: [fn(&mut MetalGridInput); 9] = [
        |input| input.grid.selectors[0].data_start = u32::MAX,
        |input| input.grid.selectors[0].kind = 0,
        |input| input.grid.selectors[0].data_count = 0,
        |input| input.grid.evaluation_spans[0].selector = u32::MAX,
        |input| input.grid.block_spans[0].selector_offset = u32::MAX,
        |input| input.grid.roots[0].node = u32::MAX,
        |input| input.grid.roots[0].file = u32::MAX,
        |input| Arc::make_mut(&mut input.packed).programs[0].structural_start = u32::MAX,
        |input| Arc::make_mut(&mut input.packed).words[0] = word(STOP, 0, 0),
    ];
    for (index, corrupt) in corruptions.into_iter().enumerate() {
        let mut input = MetalGridInput::build_shared(&programs, &docs, &anchors).unwrap();
        corrupt(&mut input);
        let resident = ResidentInput::new(input, None).unwrap();
        let result = std::panic::catch_unwind(|| ExecutionPlan::new(&resident, &programs));
        assert!(
            result.is_ok(),
            "invalid resident input {index} must return an error, never panic"
        );
        assert_eq!(
            result.unwrap().err(),
            Some(Error::InvalidAnchor),
            "invalid resident input {index} must fail before GPU access"
        );
    }
}

#[test]
fn phase_two_packed_literals_cannot_diverge_from_the_program_contract() {
    let file = file(1);
    let doc = file.restore(&|| false).unwrap();
    let mut program = Program::new(
        vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"name".to_vec(), b"other".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    program.longs = vec![u64::MAX];
    program.words.insert(0, word(SIZE, SIZE_LONG | 2, 0));
    program.validate().unwrap();
    let programs = [program];
    let roots = vec![vec![(0..doc.transducer_len() as u32).collect::<Vec<_>>()]];
    let cpu = execute_batch(&programs, std::slice::from_ref(&doc), Some(&roots)).unwrap();
    assert_eq!(cpu[0][0].len(), doc.transducer_len());
    let corruptions: [fn(&mut super::packed_batch::PackedBatch); 4] = [
        |batch| batch.longs[0] = 0,
        |batch| batch.labels[0] = 1,
        |batch| batch.string_bytes[0] = b'x',
        |batch| batch.programs[0].flags ^= 1,
    ];
    for corrupt in corruptions {
        let mut input =
            MetalGridInput::build(&programs, std::slice::from_ref(&doc), &roots).unwrap();
        corrupt(Arc::make_mut(&mut input.packed));
        let resident = ResidentInput::new(input, None).unwrap();
        assert_eq!(
            ExecutionPlan::new(&resident, &programs).err(),
            Some(Error::InvalidAnchor)
        );
    }
}

#[test]
fn phase_two_hierarchical_scan_preserves_semantic_order_across_shared_signature_blocks() {
    let file = file(1);
    let doc = file.restore(&|| false).unwrap();
    let root = (0..doc.transducer_len() as u32)
        .find(|&node| {
            doc.transducer_kind(node)
                == file.grammar.language().id_for_node_kind("identifier", true)
        })
        .unwrap();
    let programs = (0..1025)
        .map(|index| {
            Program::new(
                vec![word(CAPTURE, 0, index % 2), word(EMIT, 0, index % 2)],
                vec![b"first".to_vec(), b"second".to_vec()],
                vec![],
                vec![0, 1],
                vec![],
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let anchors = vec![vec![vec![root]; programs.len()]];
    let expected = execute_batch(&programs, std::slice::from_ref(&doc), Some(&anchors)).unwrap();
    let scope = LaunchScope::enter();
    let actual = metal_grid_backend::execute(&programs, &[doc], &anchors).unwrap();
    assert_eq!(actual, expected);
    let trace = scope.snapshot();
    assert_eq!(
        trace
            .iter()
            .filter(|record| record.kernel == "register_direct_count")
            .count(),
        1
    );
    assert!(
        !trace
            .iter()
            .any(|record| record.kernel == "register_add_parent")
    );
    let scans = trace
        .iter()
        .filter(|record| record.kernel == "register_scan")
        .collect::<Vec<_>>();
    assert_eq!(scans.len(), 1);
    assert_eq!(scans[0].buffers.len(), 2);
    assert_ne!(scans[0].buffers[0], scans[0].buffers[1]);
    assert_eq!(
        trace
            .iter()
            .find(|record| record.kernel == "register_direct_count")
            .unwrap()
            .buffers[3],
        scans[0].buffers[0]
    );
    assert_eq!(
        trace
            .iter()
            .find(|record| record.kernel == "register_boundary")
            .unwrap()
            .buffers[0],
        scans[0].buffers[1]
    );
}

#[test]
fn phase_two_field_indexes_and_utf8_spans_match_cpu() {
    let file = source_file("def café(a):\n    return a\n".to_owned());
    let doc = file.restore(&|| false).unwrap();
    let field = (0..doc.transducer_len() as u32)
        .map(|node| doc.transducer_field(node))
        .find(|&field| field != 0)
        .expect("fixture must have named fields");
    let mut programs = [FIELD, ABSENT_FIELD]
        .map(|op| {
            Program::new(
                vec![word(op, 0, field), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
                vec![b"node".to_vec()],
                vec![],
                vec![0],
                vec![],
            )
            .unwrap()
        })
        .to_vec();
    programs.push(
        Program::compile_tree_sitter_query_pattern("(identifier) @name", &file.grammar.language())
            .unwrap(),
    );
    let roots = vec![vec![
        (0..doc.transducer_len() as u32).collect::<Vec<_>>();
        programs.len()
    ]];
    let expected = execute_batch(&programs, std::slice::from_ref(&doc), Some(&roots)).unwrap();
    assert!(
        expected[0][..2]
            .iter()
            .all(|outputs| !outputs.is_empty() && outputs.len() < doc.transducer_len())
    );
    assert!(
        expected[0][2]
            .iter()
            .any(|output| output.captures[0].span.len() == 5)
    );
    let actual = metal_grid_backend::execute(&programs, &[doc], &roots).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn phase_two_completed_memory_partitions_cannot_reset_the_launch_budget() {
    // Keep one-span partitions, with enough real work to exceed the temporary cap.
    let file = file(9600);
    let programs = programs(&file.grammar);
    let docs = [file.restore(&|| false).unwrap()];
    let anchors = anchors(&programs, &docs);
    let resident = ResidentInput::new(
        MetalGridInput::build_shared(&programs, &docs, &anchors).unwrap(),
        None,
    )
    .unwrap();
    let plan = ExecutionPlan::new(&resident, &programs).unwrap();
    let scope = LaunchScope::enter();
    let mut publications = 0;
    let result = GuardedExecutor::new(resident, plan)
        .unwrap()
        .with_partition_span_limit(1)
        .unwrap()
        .execute_pages(&mut |_, _| {
            publications += 1;
            Ok(())
        });
    assert_eq!(result, Err(Error::BackendLaunchLimit));
    assert_eq!(publications, 0);
    let trace = scope.snapshot();
    assert_eq!(trace.iter().filter(|record| record.encoded).count(), 600);
    assert!(trace.len() > 600);
    let submitted = trace.iter().filter(|record| record.submitted).count();
    assert_eq!(
        submitted,
        trace.iter().filter(|record| record.completed).count()
    );
    assert!(submitted > 0 && submitted <= 600);
    assert!(
        trace
            .iter()
            .filter(|record| record.completed)
            .map(|record| record.command)
            .collect::<BTreeSet<_>>()
            .len()
            > 1
    );
    let refused_commands = trace
        .iter()
        .filter(|record| !record.encoded)
        .map(|record| record.command)
        .collect::<BTreeSet<_>>();
    assert!(!refused_commands.is_empty());
    assert!(trace.iter().all(|record| {
        !refused_commands.contains(&record.command) || (!record.submitted && !record.completed)
    }));
    assert!(trace.iter().all(|record| {
        !record.encoded || record.submitted || refused_commands.contains(&record.command)
    }));
}

#[test]
fn phase_two_finite_branches_keep_capture_multiplicity_and_output_order() {
    let file = file(3);
    let doc = file.restore(&|| false).unwrap();
    let words = vec![
        word(FORK, 0, 3),
        word(CAPTURE, 0, 0),
        word(EMIT, 0, 0),
        word(PARENT, 0, 0),
        word(CAPTURE, 0, 1),
        word(EMIT, 0, 1),
    ];
    let plain = Program::new(
        words,
        vec![b"first".to_vec(), b"second".to_vec()],
        vec![],
        vec![0, 1],
        vec![],
    )
    .unwrap();
    let mut ordered = plain.clone();
    ordered.tree_sitter_query = true;
    let repeated = Program::new(
        vec![word(CAPTURE, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"twice".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let programs = [plain, ordered, repeated];
    let anchors = vec![vec![
        (0..doc.transducer_len() as u32).collect::<Vec<_>>();
        3
    ]];
    let expected = execute_batch(&programs, std::slice::from_ref(&doc), Some(&anchors)).unwrap();
    assert!(
        expected[0][1]
            .windows(2)
            .any(|pair| pair[0].task_ordinal == pair[1].task_ordinal
                && pair[0].local_ordinal > pair[1].local_ordinal)
    );
    assert!(
        expected[0][2]
            .iter()
            .all(|output| output.captures.len() == 2 && output.captures[0] == output.captures[1])
    );
    let actual = metal_grid_backend::execute(&programs, &[doc], &anchors).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn phase_two_optional_capture_order_includes_absence_at_node_zero() {
    let file = file(1);
    let doc = file.restore(&|| false).unwrap();
    let mut program = Program::new(
        vec![
            word(FORK, 0, 3),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 0),
            word(EMIT, 0, 1),
        ],
        vec![b"node".to_vec(), b"empty".to_vec()],
        vec![],
        vec![0, 1],
        vec![],
    )
    .unwrap();
    program.tree_sitter_query = true;
    let programs = [program];
    let anchors = vec![vec![vec![0_u32]]];
    let expected = execute_batch(&programs, std::slice::from_ref(&doc), Some(&anchors)).unwrap();
    assert_eq!(expected[0][0].len(), 2);
    assert!(expected[0][0][0].captures.is_empty());
    assert_eq!(expected[0][0][1].captures[0].node, 0);
    let actual = metal_grid_backend::execute(&programs, &[doc], &anchors)
        .expect("absence must remain distinct from the real node zero in semantic ordering");
    assert_eq!(actual, expected);
}

#[test]
fn phase_two_empty_selection_does_not_invoke_unsupported_stages() {
    let file = file(1);
    let doc = file.restore(&|| false).unwrap();
    let program = Program::compile_tree_sitter_query_pattern(
        "((identifier) @name (#match? @name \"^name$\"))",
        &file.grammar.language(),
    )
    .unwrap();
    let programs = [program];
    let anchors = vec![vec![Vec::new()]];
    let expected = execute_batch(&programs, std::slice::from_ref(&doc), Some(&anchors)).unwrap();
    let scope = LaunchScope::enter();
    let actual = metal_grid_backend::execute(&programs, &[doc], &anchors)
        .expect("an empty selection must need no semantic stage");
    assert_eq!(actual, expected);
    assert!(scope.snapshot().is_empty());
}

#[test]
fn phase_two_fixed_register_operators_preserve_tree_and_byte_semantics() {
    let file = file(4);
    let doc = file.restore(&|| false).unwrap();
    let (_, supertype) = doc
        .transducer_supertypes()
        .next()
        .expect("fixture must include stored supertype membership");
    let mut programs = Vec::new();
    for operation in [
        word(FLAGS, NAMED, 0),
        word(FIRST, NAMED, 0),
        word(LAST, NAMED, 0),
        word(NEXT, NAMED, 0),
        word(SUPERTYPE, 0, supertype),
        word(SIZE, 0, 4),
        word(SIZE, 1, 4),
        word(SIZE, 2, 4),
    ] {
        programs.push(
            Program::new(
                vec![operation, word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
                vec![b"node".to_vec()],
                vec![],
                vec![0],
                vec![],
            )
            .unwrap(),
        );
    }
    let mut long = programs.last().unwrap().clone();
    long.longs = vec![4];
    long.words[0] = word(SIZE, SIZE_LONG | 2, 0);
    long.validate().unwrap();
    programs.push(long);
    let roots = vec![vec![
        (0..doc.transducer_len() as u32).collect::<Vec<_>>();
        programs.len()
    ]];
    let expected = execute_batch(&programs, std::slice::from_ref(&doc), Some(&roots)).unwrap();
    assert!(
        expected[0].iter().all(|outputs| !outputs.is_empty()),
        "every operator fixture must exercise a successful path"
    );
    let actual = metal_grid_backend::execute(&programs, &[doc], &roots).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn phase_four_properties_publish_complete_cpu_equal_results() {
    let file = file(1);
    let program = Program::compile_tree_sitter_query_pattern(
        "((identifier) @name (#set! role \"test\"))",
        &file.grammar.language(),
    )
    .unwrap();
    let docs = [file.restore(&|| false).unwrap()];
    let programs = [program];
    let anchors = anchors(&programs, &docs);
    let cpu = execute_batch(&programs, &docs, Some(&[vec![anchors[0][0].to_vec()]])).unwrap();
    assert_eq!(cpu[0][0].len(), 1);
    assert!(!cpu[0][0][0].properties.is_empty());
    let mut publications = Vec::new();
    let result = execute_production_corpus_pages(
        &programs,
        &[file],
        &anchors,
        &[u32::MAX],
        &mut |page, complete| {
            publications.push((page.into_outcome()?.into_result()?, complete));
            Ok(())
        },
    );
    assert_eq!(result, Ok(()));
    assert_eq!(publications, vec![(cpu, 1)]);
}

#[test]
fn phase_two_gpu_register_execution_matches_cpu_and_uses_broad_class_dispatches() {
    let (outputs, trace) = run(8, u32::MAX);
    assert_eq!(outputs[0][0].len(), 8);
    assert_eq!(outputs[0][1].len(), 8);
    for name in ["register_direct_count", "register_bounded_count"] {
        assert_eq!(
            trace.iter().filter(|record| record.kernel == name).count(),
            1
        );
    }
}

#[test]
fn phase_two_hundredfold_root_growth_changes_groups_instead_of_launches() {
    let (_, small) = run(80, u32::MAX);
    let (_, large) = run(8000, u32::MAX);
    assert_eq!(
        small
            .iter()
            .map(|record| &record.kernel)
            .collect::<Vec<_>>(),
        large
            .iter()
            .map(|record| &record.kernel)
            .collect::<Vec<_>>()
    );
    assert!(
        large
            .iter()
            .zip(&small)
            .any(|(large, small)| large.grid[0] > small.grid[0])
    );
}

#[test]
fn phase_two_all_selector_encodings_preserve_every_tuple_and_program_operands() {
    let file = file(64);
    let doc = file.restore(&|| false).unwrap();
    let count = doc.transducer_len() as u32;
    let programs = (0..4)
        .map(|index| {
            Program::new(
                vec![word(CAPTURE, 0, index % 2), word(EMIT, 0, index % 2)],
                vec![b"first".to_vec(), b"second".to_vec()],
                vec![],
                vec![0, 1],
                vec![],
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let roots = vec![
        (0..count).collect::<Vec<_>>(),
        (0..24).chain(count - 24..count).collect(),
        (0..count).step_by(2).collect(),
        vec![0, count / 2, count - 1],
    ];
    let anchors = vec![roots];
    let expected = execute_batch(&programs, std::slice::from_ref(&doc), Some(&anchors)).unwrap();
    let input = MetalGridInput::build(&programs, &[doc], &anchors).unwrap();
    assert_eq!(
        input
            .grid
            .selectors
            .iter()
            .map(|selector| selector.kind)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([1, 2, 3, 4])
    );
    let resident = ResidentInput::new(input, None).unwrap();
    let plan = ExecutionPlan::new(&resident, &programs).unwrap();
    let actual = GuardedExecutor::new(resident, plan)
        .unwrap()
        .execute()
        .expect("every selector must execute on GPU")
        .into_outcome()
        .unwrap()
        .into_result()
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual[0][0].len(), count as usize);
    assert_eq!(
        actual[0].iter().map(Vec::len).sum::<usize>(),
        anchors[0].iter().map(Vec::len).sum::<usize>()
    );
}

#[test]
fn phase_two_variable_child_expansion_requires_compact_continuations() {
    use super::metal_grid_plan::StageKind;
    let file = file(3);
    let doc = file.restore(&|| false).unwrap();
    let program = Program::new(
        vec![word(CHILD, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"child".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let programs = [program];
    let roots = vec![vec![Arc::from([0_u32])]];
    let resident = ResidentInput::new(
        MetalGridInput::build_shared(&programs, &[doc], &roots).unwrap(),
        None,
    )
    .unwrap();
    let plan = ExecutionPlan::new(&resident, &programs).unwrap();
    assert!(
        plan.stages
            .iter()
            .any(|stage| stage.kind == StageKind::SeedDynamicFrontier),
        "finite words do not bound child fan-out; expansion must cross a compact continuation boundary"
    );
    assert!(
        !plan
            .stages
            .iter()
            .any(|stage| stage.kind == StageKind::CountBoundedBlocks)
    );
}

#[test]
fn phase_two_scratch_partitions_preserve_disjoint_semantics_and_one_budget() {
    let (whole, whole_trace) = run(300, u32::MAX);
    let (partitioned, partition_trace) = run(300, 2);
    assert_eq!(partitioned, whole);
    assert!(partition_trace.len() > whole_trace.len());
    assert!(partition_trace.len() <= 100);
    assert_eq!(
        partition_trace
            .iter()
            .filter(|record| record.kernel == "register_roots")
            .count(),
        1,
        "root input must upload and resolve once, outside scratch partitions"
    );
}

#[test]
fn phase_two_witness_limits_apply_across_scratch_partitions() {
    for limit in [0, 1, 257, u32::MAX] {
        let file = file(300);
        let programs = programs(&file.grammar);
        let docs = [file.restore(&|| false).unwrap()];
        let anchors = anchors(&programs, &docs);
        let plain = vec![anchors[0].iter().map(|roots| roots.to_vec()).collect()];
        let mut expected = execute_batch(&programs, &docs, Some(&plain)).unwrap();
        for outputs in &mut expected[0] {
            outputs.truncate(limit as usize);
        }
        let resident = ResidentInput::new(
            MetalGridInput::build_shared(&programs, &docs, &anchors).unwrap(),
            Some(&[limit; 2]),
        )
        .unwrap();
        let plan = ExecutionPlan::new(&resident, &programs).unwrap();
        let actual = GuardedExecutor::new(resident, plan)
            .unwrap()
            .with_partition_span_limit(2)
            .unwrap()
            .execute()
            .expect("witness limits must remain exact across execution partitions")
            .into_outcome()
            .unwrap()
            .into_result()
            .unwrap();
        assert_eq!(actual, expected);
    }
}

#[test]
fn phase_two_publication_uses_one_resident_execution_across_65_files() {
    for count in [1, 65] {
        let files = vec![file(1); count];
        let programs = programs(&files[0].grammar);
        let docs = files
            .iter()
            .map(|file| file.restore(&|| false).unwrap())
            .collect::<Vec<_>>();
        let anchors = anchors(&programs, &docs);
        let scope = LaunchScope::enter();
        let mut previous_batch = None;
        let mut completed = 0;
        let mut publications = 0;
        execute_production_corpus_pages(
            &programs,
            &files,
            &anchors,
            &[u32::MAX; 2],
            &mut |page, end| {
                assert_eq!(page.physical_file_count(), count);
                let identity = Arc::as_ptr(&page.batch) as usize;
                assert_eq!(*previous_batch.get_or_insert(identity), identity);
                assert!(end > completed && end - completed <= 64);
                for file in completed..end {
                    for program in 0..programs.len() {
                        let (results, error) = page.program_results(file, program)?;
                        assert_eq!(error, None);
                        assert_eq!(results.len(), 1);
                    }
                }
            if end < count {
                for program in 0..programs.len() {
                    let (lookahead, error) = page.program_results(end, program)?;
                    assert_eq!(error, None);
                    assert!(lookahead.is_empty(), "a publication page must not expose the next file twice to the existing consumer");
                }
            }
                completed = end;
                publications += 1;
                Ok(())
            },
        )
        .expect("publication must consume completed GPU execution");
        assert_eq!(completed, count);
        assert_eq!(publications, count.div_ceil(64));
        assert_eq!(
            scope
                .snapshot()
                .iter()
                .filter(|record| record.kernel == "register_roots")
                .count(),
            1
        );
    }
}

#[test]
fn phase_two_over_budget_partition_plan_never_publishes_success() {
    let file = file(1);
    let programs = programs(&file.grammar);
    let docs = [file.restore(&|| false).unwrap()];
    let anchors = anchors(&programs, &docs);
    let scope = LaunchScope::enter();
    // Consume a real evaluation prefix, then require the production entry to share its budget.
    let device = metal::Device::system_default().unwrap();
    let queue = device.new_command_queue();
    let pipeline = super::metal_backend::pipeline_named("#include <metal_stdlib>\nusing namespace metal; kernel void prefix(device uint* value [[buffer(0)]]) { value[0] += 1u; }", "prefix").unwrap();
    let buffer = super::metal_backend::MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
    {
        let _guard = super::metal_backend::metal_execution_guard().unwrap();
        let command = queue.new_command_buffer();
        let encoder = command.new_compute_command_encoder();
        encoder.set_label("evaluation_prefix");
        encoder.set_compute_pipeline_state(&pipeline);
        encoder.set_buffer(0, Some(&buffer.buffer), 0);
        use super::metal_backend::launch::CheckedComputeEncoder;
        for _ in 0..599 {
            encoder.checked_dispatch_thread_groups(
                command,
                metal::MTLSize::new(1, 1, 1),
                metal::MTLSize::new(1, 1, 1),
            );
        }
        encoder.end_encoding();
        super::metal_backend::launch::commit_checked(command).unwrap();
        super::metal_backend::launch::wait_checked(command).unwrap();
    }
    let mut publications = 0;
    let result = execute_production_corpus_pages(
        &programs,
        &[file],
        &anchors,
        &[u32::MAX; 2],
        &mut |_, _| {
            publications += 1;
            Ok(())
        },
    );
    assert_eq!(result, Err(Error::BackendLaunchLimit));
    assert_eq!(publications, 0);
    let trace = scope.snapshot();
    assert_eq!(trace.iter().filter(|record| record.encoded).count(), 600);
    assert!(trace.iter().any(|record| !record.encoded));
    assert!(
        trace
            .iter()
            .filter(|record| record.kernel != "evaluation_prefix")
            .all(|record| !record.submitted && !record.completed)
    );
    assert_eq!(
        unsafe { buffer.buffer.contents().cast::<u32>().read() },
        599
    );
}

#[test]
fn direct_query_capture_ties_preserve_insertion_order_and_non_query_label_order() {
    let file = source_file("# node\n".to_owned());
    let docs = [file.restore(&|| false).unwrap()];
    let mut program = Program::new(
        vec![
            word(FORK, 0, 4),
            word(CAPTURE, 0, 0),
            word(CAPTURE, 0, 1),
            word(EMIT, 0, 0),
            word(CAPTURE, 0, 1),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 0),
        ],
        vec![b"x".to_vec(), b"y".to_vec()],
        vec![],
        vec![0, 1],
        vec![],
    )
    .unwrap();
    for query in [true, false] {
        program.tree_sitter_query = query;
        let expected = [vec![0, 1], if query { vec![1, 0] } else { vec![0, 1] }]
            .into_iter()
            .enumerate()
            .map(|(index, labels)| Output {
                task_ordinal: 0,
                local_ordinal: index as u32,
                start: 1,
                label: 0,
                captures: labels
                    .into_iter()
                    .map(|label| Capture {
                        label,
                        node: 1,
                        span: 0..6,
                        text: None,
                        properties: vec![],
                    })
                    .collect(),
                properties: vec![],
                dynamic_properties: vec![],
            })
            .collect::<Vec<_>>();
        let actual = execute_with_backend(
            ExecutionBackend::Metal,
            std::slice::from_ref(&program),
            &docs,
            &[vec![vec![1]]],
        );
        assert_eq!(
            actual,
            Ok(vec![vec![expected.clone()]]),
            "tree_sitter_query={query}"
        );
        assert_eq!(program.execute_roots(&docs[0], [1]), Ok(expected));
    }
}

#[test]
fn resident_navigation_rejects_conflicting_tree_representations() {
    use super::packed_batch::PackedBatch;
    let file = source_file("f(a, b)\ng(c)\nh(d)\n".to_owned());
    let programs = programs(&file.grammar);
    let docs = [
        file.restore(&|| false).unwrap(),
        file.restore(&|| false).unwrap(),
    ];
    let roots = anchors(&programs, &docs);
    // Establish the unmodified producer and exact multi-file output first.
    let owned_roots = roots
        .iter()
        .map(|file| file.iter().map(|roots| roots.to_vec()).collect())
        .collect::<Vec<Vec<Vec<u32>>>>();
    let expected = execute_batch(&programs, &docs, Some(&owned_roots)).unwrap();
    let actual = metal_grid_backend::execute(&programs, &docs, &owned_roots).unwrap();
    assert_eq!(actual, expected);
    type Corruption = (&'static str, fn(&mut PackedBatch));
    let corruptions: [Corruption; 12] = [
        ("root descendant range omits its children", |b| {
            b.nodes[0].descendant_end = 1
        }),
        ("leaf descendant range includes its sibling", |b| {
            let i = b
                .nodes
                .iter()
                .position(|n| n.child_count == 0 && n.next_any != u32::MAX)
                .unwrap();
            b.nodes[i].descendant_end = b.nodes[i].next_any + 1;
        }),
        ("sibling link skips an immediate sibling", |b| {
            let first = b.children[b.nodes[0].child_start as usize] as usize;
            let second = b.nodes[first].next_any as usize;
            assert_ne!(b.nodes[second].next_any, u32::MAX);
            b.nodes[first].next_any = b.nodes[second].next_any;
        }),
        ("sibling link crosses a parent", |b| b.nodes[1].next_any = 2),
        ("named link points to punctuation", |b| {
            let i = b
                .nodes
                .iter()
                .position(|n| n.next_any != u32::MAX && b.nodes[n.next_any as usize].flags & 1 == 0)
                .unwrap();
            b.nodes[i].next_named = b.nodes[i].next_any;
        }),
        ("missing child remains parented", |b| {
            b.nodes[0].child_count -= 1
        }),
        ("file root has a sibling", |b| b.nodes[0].next_any = 1),
        ("file root has a named sibling", |b| {
            b.nodes[0].next_named = 1
        }),
        ("child geometry escapes its parent", |b| {
            b.nodes[2].source_end = b.nodes[0].source_end
        }),
        ("unowned trailing node", |b| {
            b.files.last_mut().unwrap().node_count -= 1
        }),
        ("unowned child arena entry", |b| b.children.push(1)),
        ("child slice leaves an unused prefix", |b| {
            b.children.insert(0, 1);
            for n in &mut b.nodes {
                n.child_start += 1;
            }
        }),
    ];
    let mut accepted = Vec::new();
    for (name, corrupt) in corruptions {
        let mut input = MetalGridInput::build_shared(&programs, &docs, &roots).unwrap();
        corrupt(Arc::make_mut(&mut input.packed));
        let resident = ResidentInput::new(input, None).unwrap();
        let scope = LaunchScope::enter();
        match ExecutionPlan::new(&resident, &programs) {
            Err(Error::InvalidAnchor) => {}
            Ok(_) => accepted.push(name),
            Err(other) => panic!("{name}: expected InvalidAnchor, got {other:?}"),
        }
        assert!(
            scope.snapshot().is_empty(),
            "{name}: invalid data must not reach a GPU dispatch"
        );
    }
    assert!(
        accepted.is_empty(),
        "accepted inconsistent navigation: {accepted:?}"
    );
}
