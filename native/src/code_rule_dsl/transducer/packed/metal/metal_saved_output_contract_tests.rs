//! Negative controls exercise saved owner identity and checked GPU resume positions.
use super::*;
use crate::policy_dsl::transducer::{CAPTURE, EMIT, Program, word};

fn resident() -> ResidentInput {
    resident_with_query(false)
}

fn resident_with_query(query: bool) -> ResidentInput {
    use crate::policy_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use ast_grep_core::tree_sitter::StrDoc;
    use ast_grep_language::SupportLang;
    let source = "# FIRST\n# SECOND\n# THIRD\n";
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let native = StrDoc::new(source, SupportLang::Python);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
    let file = TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap();
    let program = Program::new(
        vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"x".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let programs = [if query {
        program.with_tree_sitter_query_semantics().unwrap()
    } else {
        program
    }];
    let anchors = [vec![(0..file.transducer_len() as u32).collect::<Vec<_>>()]];
    ResidentInput::new(
        super::super::metal_grid::MetalGridInput::build(&programs, &[file], &anchors).unwrap(),
        None,
    )
    .unwrap()
}
fn save(resident: &ResidentInput) -> Result<SavedOutput, Error> {
    let device = Device::system_default().ok_or(Error::UnsupportedBackend)?;
    let queue = device.new_command_queue();
    let input = super::super::metal_register_execution::upload(&device, resident)?;
    let count = resident.input.grid.header.root_count;
    let rows = (0..count)
        .map(|i| ResultRecord {
            task_ordinal: i,
            start: i,
            capture_start: i,
            capture_count: 1,
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let caps = resident
        .input
        .packed
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| FinalCaptureRecord {
            node: i as u32,
            start: n.source_start,
            end: n.source_end,
            text_start: u32::MAX,
            retained: 1,
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let arenas = OutputArenas {
        rows: MappedBuffer::from_slice(&device, &rows)?,
        captures: MappedBuffer::from_slice(&device, &caps)?,
        properties: MappedBuffer::zeroed::<PropertyRecord>(&device, 0)?,
        bytes: MappedBuffer::zeroed::<u8>(&device, 0)?,
        layout: None,
        counts: [count, count, 0, 0],
    };
    SavedOutput::new(resident, device, queue, input, arenas, vec![None])?.with_bounds(PageBounds {
        rows: 1,
        ..Default::default()
    })
}

#[test]
fn large_saved_output_preserves_every_occurrence_with_bounded_gpu_storage() {
    use crate::policy_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use ast_grep_core::tree_sitter::StrDoc;
    use ast_grep_language::SupportLang;
    let roots = 1024_u32;
    let program_count = 1024_usize;
    let source = "# node\n".repeat(roots as usize);
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let native = StrDoc::new(&source, SupportLang::Python);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
    let file = TapeDoc::restore(Arc::new(source), grammar, &image, &|| false).unwrap();
    assert_eq!(file.transducer_len(), roots as usize + 1);
    let program = Program::new(
        vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"x".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let programs = vec![program; program_count];
    let anchors = [vec![(1..=roots).collect::<Vec<_>>(); program_count]];
    let resident = ResidentInput::new(
        super::super::metal_grid::MetalGridInput::build(&programs, &[file], &anchors).unwrap(),
        None,
    )
    .unwrap();
    let _guard = super::super::metal_backend::metal_execution_guard().unwrap();
    let _scope = super::super::metal_backend::launch::LaunchScope::enter();
    let device = Device::system_default().unwrap();
    let queue = device.new_command_queue();
    let input = super::super::metal_register_execution::upload(&device, &resident).unwrap();
    let count = roots * program_count as u32;
    let rows = (0..count)
        .map(|i| ResultRecord {
            task_ordinal: i,
            start: i % roots + 1,
            capture_start: i,
            capture_count: 1,
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let caps = (0..count)
        .map(|i| FinalCaptureRecord {
            node: i % roots + 1,
            start: (i % roots) * 7,
            end: (i % roots) * 7 + 6,
            text_start: u32::MAX,
            retained: 1,
            ..Default::default()
        })
        .collect::<Vec<_>>();
    // Observe the original arenas too: starting after upload would hide retained
    // producer storage and incorrectly give the old implementation zero bytes.
    let observation = super::super::metal_backend::BufferObservation::enter();
    let arenas = OutputArenas {
        rows: MappedBuffer::from_slice(&device, &rows).unwrap(),
        captures: MappedBuffer::from_slice(&device, &caps).unwrap(),
        properties: MappedBuffer::zeroed::<PropertyRecord>(&device, 0).unwrap(),
        bytes: MappedBuffer::zeroed::<u8>(&device, 0).unwrap(),
        layout: None,
        counts: [count, count, 0, 0],
    };
    drop((rows, caps));
    let mut saved = SavedOutput::new(
        &resident,
        device,
        queue,
        input,
        arenas,
        vec![None; program_count],
    )
    .unwrap();
    let retained = observation.live_bytes();
    if let columns::SavedArenas::Encoded { headers, .. } = &saved.gpu.as_ref().unwrap().arenas {
        let headers = headers.buffer.clone();
        // Both count-time row reads and scatter-time capture reads must reject.
        // A refused page cannot consume the saved continuation.
        for index in [1_usize, (count as usize).div_ceil(256) * 10 + 1] {
            let pointer = unsafe { headers.contents().cast::<[u32; 2]>().add(index) };
            let original = unsafe { pointer.read() };
            unsafe {
                pointer.write([original[0], (original[1] & 0x7fff_ffe0) | 0x8000_0001]);
            }
            let boundary = saved.boundary;
            assert_eq!(saved.page(boundary, 1).err(), Some(Error::BackendFailure));
            assert_eq!(saved.cursor, 0);
            assert_eq!(saved.boundary, boundary);
            unsafe {
                pointer.write(original);
            }
        }
    }
    let actual = saved.execute().unwrap();
    assert_eq!(
        actual.host_arena_counts(),
        [count as usize, count as usize, 0, 0]
    );
    for program in 0..program_count {
        let (matches, error) = actual.program_results(0, program).unwrap();
        assert_eq!(error, None);
        assert_eq!(matches.len(), roots as usize);
        for (root, found) in matches.into_iter().enumerate() {
            assert_eq!(found.task_ordinal(), program as u32 * roots + root as u32);
            assert_eq!(found.local_ordinal(), 0);
            assert_eq!(found.start_node(), root as u32 + 1);
            assert_eq!(found.label(), 0);
            assert_eq!(found.properties().collect::<Vec<_>>(), []);
            assert_eq!(found.dynamic_properties().collect::<Vec<_>>(), []);
            let captures = found.captures().collect::<Vec<_>>();
            assert_eq!(captures.len(), 1);
            let capture = captures[0];
            assert_eq!(capture.label(), 0);
            assert_eq!(capture.node(), root as u32 + 1);
            assert_eq!(capture.span(), root * 7..root * 7 + 6);
            assert_eq!(capture.transformed_text(), None);
            assert_eq!(capture.properties().collect::<Vec<_>>(), []);
        }
    }
    eprintln!("TEMPO_SAVED_OUTPUT_STORAGE rows={count} retained={retained}");
    assert!(
        retained <= 24 * 1024 * 1024,
        "saved output retains literal-width records: {retained}"
    );
    assert_eq!(observation.wait_for_live_bytes(0), 0);
}

#[test]
fn large_typed_publication_preserves_properties_absent_text_and_empty_text() {
    use crate::policy_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use crate::policy_dsl::transducer::Directive;
    use ast_grep_core::tree_sitter::StrDoc;
    use ast_grep_language::SupportLang;
    let roots = 512_u32;
    let source = "# node\n".repeat(roots as usize);
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let native = StrDoc::new(&source, SupportLang::Python);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
    let file = TapeDoc::restore(Arc::new(source), grammar, &image, &|| false).unwrap();
    let base = Program::new(
        vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"x".to_vec(), b"flag".to_vec(), vec![]],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let programs = (0..512)
        .map(|i| {
            let value = (i % 2 != 0).then_some(2);
            let mut directives = vec![
                Directive::Set { key: 1, value },
                Directive::SetCapture {
                    capture: 0,
                    key: 1,
                    value,
                },
            ];
            if i % 4 >= 2 {
                directives.push(Directive::Offset {
                    capture: 0,
                    offsets: [0, 0, 0, -6],
                });
            }
            if i % 2 != 0 {
                directives.push(Directive::Downcase { capture: 0 });
            }
            base.clone().with_directives(directives).unwrap()
        })
        .collect::<Vec<_>>();
    let anchors = [vec![(1..=roots).collect::<Vec<_>>(); programs.len()]];
    let expected = crate::policy_dsl::transducer::execute_batch(
        &programs,
        std::slice::from_ref(&file),
        Some(&anchors),
    )
    .unwrap();
    for (program, matches) in expected[0].iter().enumerate() {
        assert_eq!(matches.len(), roots as usize);
        for (root, found) in matches.iter().enumerate() {
            assert_eq!(found.captures.len(), 1);
            let capture = &found.captures[0];
            let start = root * 7;
            assert_eq!(capture.node, root as u32 + 1);
            assert_eq!(
                capture.span,
                start..start + if program % 4 >= 2 { 0 } else { 6 }
            );
            let text = match program % 4 {
                0 | 2 => None,
                1 => Some(b"# node".as_slice()),
                3 => Some(b"".as_slice()),
                _ => unreachable!(),
            };
            assert_eq!(capture.text.as_deref(), text);
            let properties = vec![(1, (program % 2 != 0).then_some(2))];
            assert_eq!(found.properties, properties);
            assert_eq!(capture.properties, properties);
            assert!(found.dynamic_properties.is_empty());
        }
    }
    let actual =
        crate::policy_dsl::transducer::metal_program_backend::execute(&programs, &[file], &anchors)
            .unwrap();
    assert_eq!(actual.len(), expected.len());
    for (file, (actual, expected)) in actual.into_iter().zip(expected).enumerate() {
        assert_eq!(actual.len(), expected.len());
        for (program, (actual, expected)) in actual.into_iter().zip(expected).enumerate() {
            assert_eq!(actual.len(), expected.len());
            for (root, (actual, expected)) in actual.into_iter().zip(expected).enumerate() {
                assert_eq!(
                    actual, expected,
                    "file {file}, Program {program}, root {root}"
                );
            }
        }
    }
}

#[test]
fn semantic_columns_reject_wrong_roots_owners_and_segments_without_consuming_output() {
    use crate::policy_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use crate::policy_dsl::transducer::{
        metal_constant_output, metal_grid, metal_grid_plan::ExecutionPlan, metal_register_codegen,
    };
    use ast_grep_core::tree_sitter::StrDoc;
    use ast_grep_language::SupportLang;
    let source = "# node\n".repeat(512);
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let native = StrDoc::new(&source, SupportLang::Python);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
    let file = TapeDoc::restore(Arc::new(source), grammar, &image, &|| false).unwrap();
    let program = Program::new(
        vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"item".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let programs = vec![program; 128];
    let files = vec![file; 64];
    let roots: Arc<[u32]> = (1..=512).collect::<Vec<_>>().into();
    let anchors = vec![vec![roots; programs.len()]; files.len()];
    let limits = vec![1; programs.len()];
    let resident = ResidentInput::new(
        metal_grid::MetalGridInput::build_shared(&programs, &files, &anchors).unwrap(),
        Some(&limits),
    )
    .unwrap();
    let other = ResidentInput::new(
        metal_grid::MetalGridInput::build_shared(&programs, &files, &anchors).unwrap(),
        Some(&limits),
    )
    .unwrap();
    ExecutionPlan::new(&other, &programs).unwrap();
    let plan = ExecutionPlan::new(&resident, &programs).unwrap();
    let code = metal_register_codegen::compile(&resident).unwrap();
    let proof = metal_constant_output::ConstantOutput::new(&resident, &code)
        .unwrap()
        .expect("the original finite domain requires compact storage");
    let device = Device::system_default().unwrap();
    let queue = device.new_command_queue();
    let expected_roots = resident
        .input
        .grid
        .roots
        .iter()
        .map(|root| resident.input.packed.files[root.file as usize].node_start + root.node)
        .collect::<Vec<_>>();
    // These are valid nodes in the correct file, but different original roots.
    // The late case lies after the accepted witness limit and must still reject.
    for index in [0, expected_roots.len() - 1] {
        let _guard = super::super::metal_backend::metal_execution_guard().unwrap();
        let data = super::super::metal_register_execution::upload(&device, &resident).unwrap();
        let mut roots = expected_roots.clone();
        roots[index] = expected_roots[if index == 0 { 1 } else { index - 1 }];
        let roots = MappedBuffer::from_slice(&device, &roots).unwrap();
        let progress =
            MappedBuffer::zeroed::<u32>(&device, resident.input.grid.evaluation_spans.len())
                .unwrap();
        assert_eq!(
            proof
                .execute(
                    &device,
                    &queue,
                    &data,
                    &roots,
                    &progress,
                    OutputArenas::empty(&device).unwrap()
                )
                .err(),
            Some(Error::BackendFailure)
        );
    }
    let _guard = super::super::metal_backend::metal_execution_guard().unwrap();
    let mut saved =
        super::super::metal_register_execution::execute(&resident, &plan, u32::MAX).unwrap();
    let metadata = {
        let gpu = saved.gpu.as_ref().unwrap();
        let columns::SavedArenas::Semantic(arenas) = &gpu.arenas else {
            panic!("this fixture requires semantic columns")
        };
        assert_eq!(
            arenas.validate(&other, &gpu.input),
            Err(Error::BackendFailure)
        );
        let foreign = MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
        assert_eq!(
            arenas.validate(&resident, &foreign),
            Err(Error::BackendFailure)
        );
        arenas.segments.buffer.clone()
    };
    let pointer = metadata.contents().cast::<[u32; 12]>();
    let original = unsafe { pointer.read() };
    for malformed in 0..5 {
        let mut modified = original;
        match malformed {
            0 => modified[0] = 0,
            1 => modified[1] = u32::MAX,
            2 => modified[2] += 1,
            3 => {
                modified[0] = 2;
                modified[1] = u32::MAX;
            }
            4 => modified[8] = 1,
            _ => unreachable!(),
        }
        unsafe {
            pointer.write(modified);
        }
        let boundary = saved.boundary;
        assert_eq!(
            saved.page(boundary, 64).err(),
            Some(Error::BackendFailure),
            "malformed segment {malformed}"
        );
        assert_eq!(saved.cursor, 0);
        assert_eq!(saved.next_file, 0);
        assert_eq!(saved.boundary, boundary);
        unsafe {
            pointer.write(original);
        }
    }
    let actual = saved.execute().unwrap();
    assert_eq!(actual.host_arena_counts(), [8192, 8192, 0, 0]);
    for file in 0..64 {
        for program in 0..128 {
            let (matches, error) = actual.program_results(file, program).unwrap();
            assert_eq!(error, None);
            assert_eq!(matches.len(), 1);
            let found = matches[0];
            assert_eq!(found.task_ordinal(), program as u32 * 512);
            assert_eq!(found.start_node(), 1);
            assert_eq!(found.local_ordinal(), 0);
            assert_eq!(found.label(), 0);
            assert!(found.properties().next().is_none());
            assert!(found.dynamic_properties().next().is_none());
            let captures = found.captures().collect::<Vec<_>>();
            assert_eq!(captures.len(), 1);
            assert_eq!(captures[0].node(), 1);
            assert_eq!(captures[0].label(), 0);
            assert_eq!(captures[0].span(), 0..6);
            assert_eq!(captures[0].transformed_text(), None);
            assert!(captures[0].properties().next().is_none());
        }
    }
}

#[test]
fn stale_generation_rank_and_cursor_reject_without_consuming_saved_output() {
    let resident = resident();
    super::super::metal_register_execution::take_payload_reads();
    let observations = {
        let _guard = super::super::metal_backend::metal_execution_guard().unwrap();
        let _scope = super::super::metal_backend::launch::LaunchScope::enter();
        let mut saved = save(&resident).unwrap();
        let first = saved.boundary;
        let before = super::super::metal_register_execution::take_payload_reads();
        let mut invalid = first;
        invalid.frontier_generation += 1;
        let wrong_generation = saved.page(invalid, 1).err();
        invalid = first;
        invalid.local_output_ordinal += 1;
        let wrong_rank = saved.page(invalid, 1).err();
        let unchanged = saved.cursor == 0 && saved.boundary == first;
        let initial = saved.page(first, 1).unwrap().is_some();
        let next = saved.boundary;
        let stale = saved.page(first, 1).err();
        saved.cursor += 1;
        let contradictory_cursor = saved.page(next, 1).err();
        saved.cursor -= 1;
        let recovered = saved.page(next, 1).unwrap().is_some();
        (
            before,
            wrong_generation,
            wrong_rank,
            unchanged,
            initial,
            first,
            next,
            stale,
            contradictory_cursor,
            recovered,
        )
    };
    let (before, generation, rank, unchanged, initial, first, next, stale, cursor, recovered) =
        observations;
    assert_eq!(before, [0; 4]);
    assert_eq!(generation, Some(Error::InvalidAnchor));
    assert_eq!(rank, Some(Error::InvalidAnchor));
    assert!(unchanged && initial && recovered);
    assert_eq!(first.frontier_generation, next.frontier_generation);
    assert!(
        next.semantic_span > first.semantic_span
            || next.block_lane > first.block_lane
            || next.local_output_ordinal > first.local_output_ordinal
    );
    assert_eq!(stale, Some(Error::InvalidAnchor));
    assert_eq!(cursor, Some(Error::BackendFailure));
}

#[test]
fn a_different_saved_owner_cannot_resume_a_matching_logical_boundary() {
    let resident = resident();
    let (different, actual) = {
        let _guard = super::super::metal_backend::metal_execution_guard().unwrap();
        let _scope = super::super::metal_backend::launch::LaunchScope::enter();
        let first = save(&resident).unwrap();
        let mut second = save(&resident).unwrap();
        (
            first.boundary.frontier_generation != second.boundary.frontier_generation,
            second.page(first.boundary, 1).err(),
        )
    };
    assert!(different);
    assert_eq!(actual, Some(Error::InvalidAnchor));
}

#[test]
fn a_saved_row_cannot_publish_pooled_bytes_that_no_typed_value_owns() {
    let resident = resident();
    let (actual, complete) = {
        let _guard = super::super::metal_backend::metal_execution_guard().unwrap();
        let _scope = super::super::metal_backend::launch::LaunchScope::enter();
        let device = Device::system_default().unwrap();
        let queue = device.new_command_queue();
        let input = super::super::metal_register_execution::upload(&device, &resident).unwrap();
        let arenas = OutputArenas {
            rows: MappedBuffer::from_slice(&device, &[ResultRecord::default()]).unwrap(),
            captures: MappedBuffer::zeroed::<FinalCaptureRecord>(&device, 0).unwrap(),
            properties: MappedBuffer::zeroed::<PropertyRecord>(&device, 0).unwrap(),
            bytes: MappedBuffer::from_slice(&device, b"X").unwrap(),
            layout: Some(MappedBuffer::from_slice(&device, &[[0u32, 1, 0, 0]]).unwrap()),
            counts: [1, 0, 0, 1],
        };
        let mut complete = Vec::new();
        let actual = SavedOutput::new(&resident, device, queue, input, arenas, vec![None])
            .unwrap()
            .execute_pages(&mut |_, end| {
                complete.push(end);
                Ok(())
            });
        (actual, complete)
    };
    assert_eq!(actual, Err(Error::BackendFailure));
    assert!(complete.is_empty());
}

#[test]
fn saved_ranks_preserve_query_order_with_raw_ordinal_gaps_and_permutations() {
    let resident = resident_with_query(true);
    let (boundaries, actual) = {
        let _guard = super::super::metal_backend::metal_execution_guard().unwrap();
        let _scope = super::super::metal_backend::launch::LaunchScope::enter();
        let device = Device::system_default().unwrap();
        let queue = device.new_command_queue();
        let input = super::super::metal_register_execution::upload(&device, &resident).unwrap();
        let rows = [2, 0, 4]
            .into_iter()
            .enumerate()
            .map(|(i, ordinal)| ResultRecord {
                local_ordinal: ordinal,
                capture_start: i as u32,
                capture_count: 1,
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let caps = (1..4)
            .map(|i| {
                let n = resident.input.packed.nodes[i];
                FinalCaptureRecord {
                    node: i as u32,
                    start: n.source_start,
                    end: n.source_end,
                    text_start: u32::MAX,
                    retained: 1,
                    ..Default::default()
                }
            })
            .collect::<Vec<_>>();
        let arenas = OutputArenas {
            rows: MappedBuffer::from_slice(&device, &rows).unwrap(),
            captures: MappedBuffer::from_slice(&device, &caps).unwrap(),
            properties: MappedBuffer::zeroed::<PropertyRecord>(&device, 0).unwrap(),
            bytes: MappedBuffer::zeroed::<u8>(&device, 0).unwrap(),
            layout: None,
            counts: [3, 3, 0, 0],
        };
        let mut saved = SavedOutput::new(&resident, device, queue, input, arenas, vec![None])
            .unwrap()
            .with_bounds(PageBounds {
                rows: 1,
                ..Default::default()
            })
            .unwrap();
        let mut boundaries = vec![saved.boundary];
        let mut fragments = Vec::new();
        for _ in 0..3 {
            fragments.push(saved.page(saved.boundary, 1).unwrap().unwrap());
            boundaries.push(saved.boundary);
        }
        let actual = saved
            .publication
            .execution(fragments, 0..1)
            .unwrap()
            .into_outcome()
            .unwrap();
        (boundaries, actual)
    };
    assert!(actual.errors.iter().flatten().all(Option::is_none));
    let rows = &actual.outputs[0][0];
    assert_eq!(
        rows.iter().map(|r| r.local_ordinal).collect::<Vec<_>>(),
        [2, 0, 4]
    );
    assert_eq!(
        rows.iter().map(|r| r.captures[0].node).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert!(
        rows.iter()
            .all(|r| r.task_ordinal == 0 && r.start == 0 && r.captures.len() == 1)
    );
    assert_eq!(
        boundaries[..3]
            .iter()
            .map(|b| b.local_output_ordinal)
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert!(
        boundaries
            .iter()
            .all(|b| b.frontier_generation == boundaries[0].frontier_generation)
    );
    assert_eq!(
        boundaries[3].semantic_span,
        resident.input.grid.semantic_span_order.len() as u32
    );
    assert_eq!(
        (boundaries[3].block_lane, boundaries[3].local_output_ordinal),
        (0, 0)
    );
}
