//! Complete typed outputs must not require a second complete raw candidate arena.
use super::*;
use crate::policy_dsl::transducer::*;

#[test]
fn shared_outputs_reduce_producer_batches_before_retaining_combined_raw_capture_storage() {
    let roots = 131_072u32;
    let source = (0..roots)
        .map(|i| {
            if i % 256 == 255 {
                "# KEEP\n"
            } else {
                "# DROP\n"
            }
        })
        .collect::<String>();
    let file = metal_register_conformance::source_file(source);
    let doc = file.restore(&|| false).unwrap();
    assert_eq!(doc.transducer_len(), roots as usize + 1);
    let mut words = vec![word(CAPTURE, 0, 0); 4];
    words.push(word(EMIT, 0, 1));
    let base = Program::new(
        words,
        vec![
            b"value".to_vec(),
            b"match".to_vec(),
            b"role".to_vec(),
            b"# KEEP".to_vec(),
        ],
        vec![TextPredicate::Equal(3)],
        vec![0, 1],
        vec![],
    )
    .unwrap()
    .with_tree_sitter_query_semantics()
    .unwrap()
    .with_capture_predicates(vec![CapturePredicate::Text {
        capture: 0,
        predicate: 0,
        negated: false,
        any: false,
    }])
    .unwrap();
    let programs = [
        base.clone()
            .with_directives(vec![
                Directive::Set {
                    key: 2,
                    value: None,
                },
                Directive::Downcase { capture: 0 },
            ])
            .unwrap(),
        base.with_directives(vec![
            Directive::SetCapture {
                capture: 0,
                key: 2,
                value: Some(3),
            },
            Directive::Downcase { capture: 0 },
        ])
        .unwrap(),
    ];
    let anchors = [vec![(1..=roots).collect::<Vec<_>>(); programs.len()]];
    let expected = execute_batch(&programs, std::slice::from_ref(&doc), Some(&anchors)).unwrap();
    assert_eq!(
        expected[0].iter().map(Vec::len).collect::<Vec<_>>(),
        [512, 512]
    );
    for outputs in &expected[0] {
        for output in outputs {
            assert_eq!(output.captures.len(), 4);
            assert_eq!(output.local_ordinal, 0);
        }
    }
    let resident = ResidentInput::new(
        metal_grid::MetalGridInput::build(&programs, std::slice::from_ref(&doc), &anchors).unwrap(),
        None,
    )
    .unwrap();
    let _guard = metal_backend::metal_execution_guard().unwrap();
    let scope = metal_backend::launch::LaunchScope::enter();
    let observation = metal_backend::BufferObservation::enter();
    let device = Device::system_default().unwrap();
    let queue = device.new_command_queue();
    let input = metal_register_execution::upload(&device, &resident).unwrap();
    let predicates =
        metal_capture_predicates::PredicatePrograms::new(&device, &queue, &resident, &input)
            .unwrap();
    let mut batches = vec![];
    for program in 0..programs.len() {
        let rows = (0..roots)
            .map(|i| ResultRecord {
                task_ordinal: program as u32 * roots + i,
                start: i + 1,
                label: 1,
                capture_start: i * 4,
                capture_count: 4,
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let captures = (1..=roots)
            .flat_map(|node| {
                std::iter::repeat_n(
                    FinalCaptureRecord {
                        node,
                        start: doc.transducer_span(node).start as u32,
                        end: doc.transducer_span(node).end as u32,
                        text_start: u32::MAX,
                        retained: 1,
                        ..Default::default()
                    },
                    4,
                )
            })
            .collect::<Vec<_>>();
        batches.push(CandidateBatch {
            results: MappedBuffer::from_slice(&device, &rows).unwrap(),
            captures: MappedBuffer::from_slice(&device, &captures).unwrap(),
            result_count: rows.len(),
            capture_count: captures.len(),
        });
    }
    let started = std::time::Instant::now();
    let (arenas, errors) = execute(
        &device,
        &queue,
        &resident,
        &input,
        batches,
        vec![None; 2],
        &predicates,
    )
    .unwrap();
    drop(predicates);
    let actual =
        metal_output_pages::SavedOutput::new(&resident, device, queue, input, arenas, errors)
            .unwrap()
            .execute()
            .unwrap();
    assert_eq!(
        actual.into_outcome().unwrap().into_result().unwrap(),
        expected
    );
    let trace = scope.snapshot();
    let peak = trace
        .iter()
        .map(|r| r.peak_application_buffer_bytes)
        .max()
        .unwrap();
    eprintln!(
        "TYPED_BATCH_STORAGE {}",
        serde_json::json!({"roots": roots*2, "outputs":1024,"peakMetalBytes":peak,"dispatches":trace.len(),"seconds":started.elapsed().as_secs_f64()})
    );
    assert!(
        trace
            .iter()
            .all(|r| r.encoded && r.submitted && r.completed)
    );
    assert!(
        peak <= 128 * 1024 * 1024,
        "shared operators retain a complete combined raw capture arena: {peak}"
    );
    assert_eq!(observation.wait_for_live_bytes(0), 0);
}

fn raw_batch(device: &Device, doc: &TapeDoc, roots: &[(u32, u32)], count: u32) -> CandidateBatch {
    let rows = roots
        .iter()
        .enumerate()
        .map(|(i, &(task, node))| ResultRecord {
            task_ordinal: task,
            start: node,
            label: 1,
            capture_start: i as u32 * count,
            capture_count: count,
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let captures = roots
        .iter()
        .flat_map(|&(_, node)| {
            std::iter::repeat_n(
                FinalCaptureRecord {
                    node,
                    start: doc.transducer_span(node).start as u32,
                    end: doc.transducer_span(node).end as u32,
                    text_start: u32::MAX,
                    retained: 1,
                    ..Default::default()
                },
                count as usize,
            )
        })
        .collect::<Vec<_>>();
    CandidateBatch {
        results: MappedBuffer::from_slice(device, &rows).unwrap(),
        captures: MappedBuffer::from_slice(device, &captures).unwrap(),
        result_count: rows.len(),
        capture_count: captures.len(),
    }
}

#[test]
fn typed_producer_merge_keeps_global_witness_limits_and_complete_dynamic_values() {
    let file = metal_register_conformance::source_file("# FIRST\n# SECOND\n# THIRD\n".to_owned());
    let doc = file.restore(&|| false).unwrap();
    let program = Program::new(
        vec![word(CAPTURE, 0, 0), word(EMIT, 0, 1)],
        vec![b"value".to_vec(), b"match".to_vec(), b"role".to_vec()],
        vec![],
        vec![0, 1],
        vec![],
    )
    .unwrap()
    .with_tree_sitter_query_semantics()
    .unwrap()
    .with_directives(vec![
        Directive::SetCapture {
            capture: 0,
            key: 2,
            value: None,
        },
        Directive::SetLanguageFromMimeType { capture: 0 },
        Directive::Downcase { capture: 0 },
    ])
    .unwrap();
    let anchors = [vec![vec![1, 2, 3]]];
    let expected = execute_batch(
        std::slice::from_ref(&program),
        std::slice::from_ref(&doc),
        Some(&anchors),
    )
    .unwrap();
    assert_eq!(expected[0][0].len(), 3);
    assert!(
        expected[0][0]
            .iter()
            .all(|row| !row.dynamic_properties.is_empty())
    );
    let _guard = metal_backend::metal_execution_guard().unwrap();
    let _scope = metal_backend::launch::LaunchScope::enter();
    for limit in [0u32, 1, 2, 3, u32::MAX] {
        let resident = ResidentInput::new(
            metal_grid::MetalGridInput::build(
                std::slice::from_ref(&program),
                std::slice::from_ref(&doc),
                &anchors,
            )
            .unwrap(),
            Some(&[limit]),
        )
        .unwrap();
        let device = Device::system_default().unwrap();
        let queue = device.new_command_queue();
        let input = metal_register_execution::upload(&device, &resident).unwrap();
        let predicates =
            metal_capture_predicates::PredicatePrograms::new(&device, &queue, &resident, &input)
                .unwrap();
        // Producer order differs from semantic order, including an interleaved pair.
        let batches = vec![
            raw_batch(&device, &doc, &[(0, 1), (2, 3)], 1),
            raw_batch(&device, &doc, &[(1, 2)], 1),
        ];
        let (arenas, errors) = execute(
            &device,
            &queue,
            &resident,
            &input,
            batches,
            vec![None],
            &predicates,
        )
        .unwrap();
        drop(predicates);
        let actual =
            metal_output_pages::SavedOutput::new(&resident, device, queue, input, arenas, errors)
                .unwrap()
                .execute()
                .unwrap()
                .into_outcome()
                .unwrap()
                .into_result()
                .unwrap();
        let mut wanted = expected.clone();
        wanted[0][0].truncate(limit as usize);
        assert_eq!(actual, wanted, "global witness limit {limit}");
    }
}

#[test]
fn a_late_value_budget_failure_discards_earlier_accepted_producer_output() {
    let pattern = "^# A(?:A?){1024}Z";
    let file = metal_register_conformance::source_file("# SHORT\n# AB\n".to_owned());
    let doc = file.restore(&|| false).unwrap();
    assert_eq!(doc.transducer_len(), 3);
    let captures = 512usize;
    let mut words = vec![word(CAPTURE, 0, 0); captures];
    words.push(word(EMIT, 0, 1));
    let program = Program::new(
        words,
        vec![
            b"value".to_vec(),
            b"match".to_vec(),
            pattern.as_bytes().to_vec(),
        ],
        vec![TextPredicate::Regex {
            source: 2,
            compiled: Regex::new(pattern).unwrap(),
        }],
        vec![0, 1],
        vec![],
    )
    .unwrap()
    .with_tree_sitter_query_semantics()
    .unwrap()
    .with_capture_predicates(vec![CapturePredicate::Text {
        capture: 0,
        predicate: 0,
        negated: true,
        any: false,
    }])
    .unwrap();
    let value::ValueProgram::Regex(nfa) = &program.value_programs[0] else {
        panic!("one NFA value")
    };
    let (early_result, early_work) = nfa.test_work_count(doc.transducer_text(1));
    let (late_result, late_work) = nfa.test_work_count(doc.transducer_text(2));
    assert_eq!((early_result, late_result), (Ok(false), Ok(false)));
    assert!(early_work * captures < MAX_STEPS_PER_PATH);
    assert!(late_work * captures > MAX_STEPS_PER_PATH);
    eprintln!("TYPED_BATCH_LATE_WORK early={early_work} late={late_work} captures={captures}");
    let early = program.execute_roots(&doc, [1]).unwrap();
    assert_eq!(early.len(), 1);
    assert_eq!(early[0].captures.len(), captures);
    assert_eq!(
        program.execute_roots(&doc, [2]).err(),
        Some(Error::WorkLimit)
    );
    let anchors = [vec![vec![1, 2]]];
    let resident = ResidentInput::new(
        metal_grid::MetalGridInput::build(&[program], std::slice::from_ref(&doc), &anchors)
            .unwrap(),
        None,
    )
    .unwrap();
    let _guard = metal_backend::metal_execution_guard().unwrap();
    let _scope = metal_backend::launch::LaunchScope::enter();
    let device = Device::system_default().unwrap();
    let queue = device.new_command_queue();
    let input = metal_register_execution::upload(&device, &resident).unwrap();
    let predicates =
        metal_capture_predicates::PredicatePrograms::new(&device, &queue, &resident, &input)
            .unwrap();
    let batches = vec![
        raw_batch(&device, &doc, &[(0, 1)], captures as u32),
        raw_batch(&device, &doc, &[(1, 2)], captures as u32),
    ];
    let (output, errors) = execute(
        &device,
        &queue,
        &resident,
        &input,
        batches,
        vec![None],
        &predicates,
    )
    .unwrap();
    assert_eq!(errors, vec![Some(Error::WorkLimit)]);
    assert_eq!(output.counts, [0; 4]);
}

#[test]
fn typed_merge_rejects_two_producers_claiming_the_same_task() {
    let file = metal_register_conformance::source_file("# VALUE\n".to_owned());
    let doc = file.restore(&|| false).unwrap();
    let program = Program::new(
        vec![word(CAPTURE, 0, 0), word(EMIT, 0, 1)],
        vec![b"value".to_vec(), b"match".to_vec()],
        vec![],
        vec![0, 1],
        vec![],
    )
    .unwrap();
    let resident = ResidentInput::new(
        metal_grid::MetalGridInput::build(&[program], std::slice::from_ref(&doc), &[vec![vec![1]]])
            .unwrap(),
        None,
    )
    .unwrap();
    let _guard = metal_backend::metal_execution_guard().unwrap();
    let _scope = metal_backend::launch::LaunchScope::enter();
    let device = Device::system_default().unwrap();
    let queue = device.new_command_queue();
    let input = metal_register_execution::upload(&device, &resident).unwrap();
    let predicates =
        metal_capture_predicates::PredicatePrograms::new(&device, &queue, &resident, &input)
            .unwrap();
    let batches = vec![
        raw_batch(&device, &doc, &[(0, 1)], 1),
        raw_batch(&device, &doc, &[(0, 1)], 1),
    ];
    assert_eq!(
        execute(
            &device,
            &queue,
            &resident,
            &input,
            batches,
            vec![None],
            &predicates
        )
        .err(),
        Some(Error::BackendFailure)
    );
}

#[test]
fn unrelated_program_operators_do_not_expand_a_producers_workspace_or_launch_floor() {
    let roots = 131_072u32;
    let file = metal_register_conformance::source_file("# KEEP\n".repeat(roots as usize));
    let doc = file.restore(&|| false).unwrap();
    let base = Program::new(
        vec![word(CAPTURE, 0, 0), word(EMIT, 0, 1)],
        vec![b"value".to_vec(), b"match".to_vec(), b"^# KEEP$".to_vec()],
        vec![],
        vec![0, 1],
        vec![],
    )
    .unwrap();
    let mut strings = base.strings.clone();
    strings.extend((0..64).map(|i| format!("key-{i}").into_bytes()));
    let rare = Program::new(
        base.words.clone(),
        strings,
        vec![TextPredicate::Regex {
            source: 2,
            compiled: Regex::new("^# KEEP$").unwrap(),
        }],
        vec![0, 1],
        vec![],
    )
    .unwrap()
    .with_tree_sitter_query_semantics()
    .unwrap()
    .with_capture_predicates(vec![CapturePredicate::Text {
        capture: 0,
        predicate: 0,
        negated: false,
        any: false,
    }])
    .unwrap()
    .with_directives(
        (3..67)
            .map(|key| Directive::SetCapture {
                capture: 0,
                key,
                value: None,
            })
            .collect(),
    )
    .unwrap();
    let programs = [base, rare];
    let anchors = [vec![(1..=roots).collect(), vec![roots]]];
    let expected = execute_batch(&programs, std::slice::from_ref(&doc), Some(&anchors)).unwrap();
    assert_eq!(
        expected[0].iter().map(Vec::len).collect::<Vec<_>>(),
        [roots as usize, 1]
    );
    assert_eq!(expected[0][1][0].captures[0].properties.len(), 64);
    let resident = ResidentInput::new(
        metal_grid::MetalGridInput::build(&programs, std::slice::from_ref(&doc), &anchors).unwrap(),
        None,
    )
    .unwrap();
    let _guard = metal_backend::metal_execution_guard().unwrap();
    let scope = metal_backend::launch::LaunchScope::enter();
    let device = Device::system_default().unwrap();
    let queue = device.new_command_queue();
    let input = metal_register_execution::upload(&device, &resident).unwrap();
    let predicates =
        metal_capture_predicates::PredicatePrograms::new(&device, &queue, &resident, &input)
            .unwrap();
    let task_roots = (0..roots).map(|i| (i, i + 1)).collect::<Vec<_>>();
    let batches = vec![
        raw_batch(&device, &doc, &task_roots, 1),
        raw_batch(&device, &doc, &[(roots, roots)], 1),
    ];
    let (arenas, errors) = execute(
        &device,
        &queue,
        &resident,
        &input,
        batches,
        vec![None; 2],
        &predicates,
    )
    .unwrap();
    drop(predicates);
    let actual =
        metal_output_pages::SavedOutput::new(&resident, device, queue, input, arenas, errors)
            .unwrap()
            .execute()
            .unwrap()
            .into_outcome()
            .unwrap()
            .into_result()
            .unwrap();
    assert_eq!(actual, expected);
    let trace = scope.snapshot();
    let peak = trace
        .iter()
        .map(|r| r.peak_application_buffer_bytes)
        .max()
        .unwrap();
    eprintln!(
        "TYPED_PROGRAM_SCOPE roots={roots} outputs={} launches={} peakMetalBytes={peak}",
        roots + 1,
        trace.len()
    );
    assert!(
        trace
            .iter()
            .all(|r| r.encoded && r.submitted && r.completed)
    );
    assert!(
        peak <= 64 * 1024 * 1024,
        "unrelated operators inflate producer storage: {peak}"
    );
    assert!(
        trace.len() <= 200,
        "unrelated operators inflate the launch floor: {}",
        trace.len()
    );
}

#[test]
fn predicate_execution_retires_before_transform_workspace_allocation() {
    let roots = 16_384u32;
    let captures = 64u32;
    let mut source = "# DROP\n".repeat(roots as usize - 1);
    source.push_str("# KEEP\n");
    let file = metal_register_conformance::source_file(source);
    let doc = file.restore(&|| false).unwrap();
    assert_eq!(doc.transducer_len(), roots as usize + 1);
    let mut words = vec![word(CAPTURE, 0, 0); captures as usize];
    words.push(word(EMIT, 0, 1));
    let mut strings = vec![b"value".to_vec(), b"match".to_vec(), b"# KEEP".to_vec()];
    strings.extend((0..64).map(|i| format!("key-{i}").into_bytes()));
    let mut directives = (3..67)
        .map(|key| Directive::SetCapture {
            capture: 0,
            key,
            value: None,
        })
        .collect::<Vec<_>>();
    directives.push(Directive::Downcase { capture: 0 });
    let program = Program::new(words, strings, vec![], vec![0, 1], vec![])
        .unwrap()
        .with_tree_sitter_query_semantics()
        .unwrap()
        .with_capture_predicates(vec![CapturePredicate::AnyOf {
            capture: 0,
            strings: vec![2],
            negated: false,
            any: false,
        }])
        .unwrap()
        .with_directives(directives)
        .unwrap();
    let anchors = [vec![(1..=roots).collect::<Vec<_>>()]];
    let expected = execute_batch(
        std::slice::from_ref(&program),
        std::slice::from_ref(&doc),
        Some(&anchors),
    )
    .unwrap();
    assert_eq!(expected[0][0].len(), 1);
    assert_eq!(expected[0][0][0].task_ordinal, roots - 1);
    assert_eq!(expected[0][0][0].captures.len(), captures as usize);
    for capture in &expected[0][0][0].captures {
        assert_eq!(capture.properties.len(), 64);
        assert_eq!(capture.text.as_deref(), Some(b"# keep".as_slice()));
    }
    let resident = ResidentInput::new(
        metal_grid::MetalGridInput::build(&[program], std::slice::from_ref(&doc), &anchors)
            .unwrap(),
        None,
    )
    .unwrap();
    let _guard = metal_backend::metal_execution_guard().unwrap();
    let scope = metal_backend::launch::LaunchScope::enter();
    let device = Device::system_default().unwrap();
    let queue = device.new_command_queue();
    let input = metal_register_execution::upload(&device, &resident).unwrap();
    let programs =
        metal_capture_predicates::PredicatePrograms::new(&device, &queue, &resident, &input)
            .unwrap();
    let task_roots = (0..roots).map(|i| (i, i + 1)).collect::<Vec<_>>();
    let batches = vec![raw_batch(&device, &doc, &task_roots, captures)];
    let started = std::time::Instant::now();
    let (arenas, errors) = execute(
        &device,
        &queue,
        &resident,
        &input,
        batches,
        vec![None],
        &programs,
    )
    .unwrap();
    drop(programs);
    let actual =
        metal_output_pages::SavedOutput::new(&resident, device, queue, input, arenas, errors)
            .unwrap()
            .execute()
            .unwrap()
            .into_outcome()
            .unwrap()
            .into_result()
            .unwrap();
    assert_eq!(actual, expected);
    let trace = scope.snapshot();
    let peak = trace
        .iter()
        .map(|r| r.peak_application_buffer_bytes)
        .max()
        .unwrap();
    eprintln!(
        "PREDICATE_TRANSFORM_PHASE roots={roots} captures={} outputs=1 peakMetalBytes={peak} launches={} seconds={}",
        roots * captures,
        trace.len(),
        started.elapsed().as_secs_f64()
    );
    assert!(
        trace
            .iter()
            .all(|r| r.encoded && r.submitted && r.completed)
    );
    assert!(
        peak <= 384 * 1024 * 1024,
        "query and transform workspaces overlap: {peak}"
    );
}

#[test]
fn typed_outputs_preserve_transform_order_and_complete_values_within_twenty_eight_dispatches() {
    let file = metal_register_conformance::source_file("# FIRST\n# SECOND\n# TARGET\n".to_owned());
    let doc = file.restore(&|| false).unwrap();
    assert_eq!(doc.transducer_len(), 4);
    let _guard = metal_backend::metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let queue = device.new_command_queue();
    for count in [1025usize, 0, 1, 2, 255, 256, 257, 513] {
        let scope = metal_backend::launch::LaunchScope::enter();
        let start = scope.snapshot().len();
        let mut words = Vec::new();
        let mut branches = Vec::new();
        for i in 0..count {
            let branch = words.len();
            if i + 1 < count {
                words.push(word(FORK, 0, 0));
            }
            let captures = match i % 4 {
                0 => vec![],
                1 => {
                    words.extend([
                        word(CAPTURE, 0, 0),
                        word(NEXT, NAMED, 0),
                        word(NEXT, NAMED, 0),
                        word(CAPTURE, 0, 1),
                    ]);
                    vec![(0, 1), (1, 3)]
                }
                2 => {
                    words.extend([
                        word(NEXT, NAMED, 0),
                        word(CAPTURE, 0, 0),
                        word(NEXT, NAMED, 0),
                        word(CAPTURE, 0, 1),
                    ]);
                    vec![(0, 2), (1, 3)]
                }
                _ => {
                    words.push(word(CAPTURE, 0, 0));
                    vec![(0, 1)]
                }
            };
            words.push(word(EMIT, 0, i as u16 + 2));
            if i + 1 < count {
                words[branch] = word(FORK, 0, words.len() as u16);
            }
            branches.push(captures);
        }
        if count == 0 {
            words.push(word(STOP, 0, 0));
        }
        let mut strings = vec![b"value".to_vec(), b"target".to_vec()];
        strings.extend((0..count).map(|i| format!("branch-{i}").into_bytes()));
        let key = strings.len() as u16;
        strings.extend([b"role".to_vec(), b"kept".to_vec()]);
        let program = Program::new(words, strings, vec![], (0..key).collect(), vec![])
            .unwrap()
            .with_tree_sitter_query_semantics()
            .unwrap()
            .with_directives(vec![
                Directive::SetCapture {
                    capture: 0,
                    key,
                    value: Some(key + 1),
                },
                Directive::Downcase { capture: 0 },
                Directive::SelectAdjacent {
                    capture: 0,
                    target: 1,
                },
                Directive::Set { key, value: None },
            ])
            .unwrap();
        let anchors = [vec![vec![1]]];
        let expected = execute_batch(
            std::slice::from_ref(&program),
            std::slice::from_ref(&doc),
            Some(&anchors),
        )
        .unwrap();
        assert_eq!(expected[0][0].len(), count);
        for output in &expected[0][0] {
            assert_eq!(output.properties.len(), 1);
            for capture in &output.captures {
                if capture.label == 0 {
                    assert_eq!(capture.properties.len(), 1);
                    assert_eq!(
                        capture.text.as_deref(),
                        Some(if capture.node == 1 {
                            b"# first".as_slice()
                        } else {
                            b"# second".as_slice()
                        })
                    );
                }
            }
        }
        let resident = ResidentInput::new(
            metal_grid::MetalGridInput::build(&[program], std::slice::from_ref(&doc), &anchors)
                .unwrap(),
            None,
        )
        .unwrap();
        let input = metal_register_execution::upload(&device, &resident).unwrap();
        let programs =
            metal_capture_predicates::PredicatePrograms::new(&device, &queue, &resident, &input)
                .unwrap();
        let mut rows = Vec::new();
        let mut captures = Vec::new();
        // Reverse producer order. Raw ordinals, empty rows, directive removal,
        // and the first retained capture must determine the final order.
        for i in (0..count).rev() {
            rows.push(ResultRecord {
                start: 1,
                local_ordinal: i as u32,
                label: i as u32 + 2,
                capture_start: captures.len() as u32,
                capture_count: branches[i].len() as u32,
                ..Default::default()
            });
            for &(label, node) in &branches[i] {
                let span = doc.transducer_span(node);
                captures.push(FinalCaptureRecord {
                    label,
                    node,
                    start: span.start as u32,
                    end: span.end as u32,
                    text_start: u32::MAX,
                    retained: 1,
                    ..Default::default()
                });
            }
        }
        let batches = vec![CandidateBatch {
            results: MappedBuffer::from_slice(&device, &rows).unwrap(),
            captures: MappedBuffer::from_slice(&device, &captures).unwrap(),
            result_count: rows.len(),
            capture_count: captures.len(),
        }];
        let started = std::time::Instant::now();
        let (arenas, errors) = execute(
            &device,
            &queue,
            &resident,
            &input,
            batches,
            vec![None],
            &programs,
        )
        .unwrap();
        drop(programs);
        let actual = metal_output_pages::SavedOutput::new(
            &resident,
            device.clone(),
            queue.clone(),
            input,
            arenas,
            errors,
        )
        .unwrap()
        .execute()
        .unwrap()
        .into_outcome()
        .unwrap()
        .into_result()
        .unwrap();
        assert_eq!(actual, expected, "count={count}");
        let trace = scope.snapshot();
        let trace = &trace[start..];
        eprintln!(
            "TYPED_OUTPUT_DISPATCH rows={count} dispatches={} seconds={}",
            trace.len(),
            started.elapsed().as_secs_f64()
        );
        assert!(
            trace
                .iter()
                .all(|r| r.encoded && r.submitted && r.completed)
        );
        assert!(
            trace.len() <= 28,
            "complete typed output exceeds its dispatch allowance: {}",
            trace.len()
        );
    }
}

#[test]
fn completed_typed_producers_seal_before_later_producers_need_output_storage() {
    let count = 65_536u32;
    let captures = 32u32;
    let _guard = metal_backend::metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let queue = device.new_command_queue();
    for producers in [2usize, 3] {
        let file = metal_register_conformance::source_file("# VALUE\n".repeat(count as usize));
        let doc = file.restore(&|| false).unwrap();
        let mut words = vec![word(CAPTURE, 0, 0); captures as usize];
        words.push(word(EMIT, 0, 1));
        let program = Program::new(
            words,
            vec![b"value".to_vec(), b"match".to_vec(), b"role".to_vec()],
            vec![],
            vec![0, 1],
            vec![],
        )
        .unwrap()
        .with_directives(vec![Directive::Set {
            key: 2,
            value: None,
        }])
        .unwrap();
        let programs = vec![program; producers];
        let anchors = [vec![(1..=count).collect::<Vec<_>>(); producers]];
        let expected =
            execute_batch(&programs, std::slice::from_ref(&doc), Some(&anchors)).unwrap();
        assert_eq!(
            expected[0].iter().map(Vec::len).collect::<Vec<_>>(),
            vec![count as usize; producers]
        );
        for outputs in &expected[0] {
            for output in outputs {
                assert_eq!(output.captures.len(), captures as usize);
                assert_eq!(output.properties.len(), 1);
            }
        }
        let resident = ResidentInput::new(
            metal_grid::MetalGridInput::build(&programs, std::slice::from_ref(&doc), &anchors)
                .unwrap(),
            None,
        )
        .unwrap();
        let scope = metal_backend::launch::LaunchScope::enter();
        let start = scope.snapshot().len();
        let input = metal_register_execution::upload(&device, &resident).unwrap();
        let predicates =
            metal_capture_predicates::PredicatePrograms::new(&device, &queue, &resident, &input)
                .unwrap();
        let mut collector = OutputCollector::new(&device, &queue, &resident, &input, &predicates);
        let errors = vec![None; producers];
        let started = std::time::Instant::now();
        for program in 0..producers {
            let roots = (0..count)
                .map(|i| (program as u32 * count + i, i + 1))
                .collect::<Vec<_>>();
            collector
                .push(raw_batch(&device, &doc, &roots, captures), &errors)
                .unwrap();
        }
        let (arenas, errors) = collector.finish(errors).unwrap();
        drop(predicates);
        let actual = metal_output_pages::SavedOutput::new(
            &resident,
            device.clone(),
            queue.clone(),
            input,
            arenas,
            errors,
        )
        .unwrap()
        .execute()
        .unwrap()
        .into_outcome()
        .unwrap()
        .into_result()
        .unwrap();
        assert_eq!(actual.len(), 1);
        assert_eq!(actual[0].len(), producers);
        for (program, (actual, expected)) in actual[0].iter().zip(&expected[0]).enumerate() {
            assert_eq!(actual.len(), expected.len());
            for (row, (actual, expected)) in actual.iter().zip(expected).enumerate() {
                assert_eq!(actual, expected, "program={program} row={row}");
            }
        }
        let trace = scope.snapshot();
        let trace = &trace[start..];
        let peak = trace
            .iter()
            .map(|r| r.peak_application_buffer_bytes)
            .max()
            .unwrap();
        eprintln!(
            "SEALED_TYPED_PRODUCERS rows={} captures={} peakMetalBytes={peak} dispatches={} seconds={}",
            producers as u32 * count,
            producers as u32 * count * captures,
            trace.len(),
            started.elapsed().as_secs_f64()
        );
        assert!(
            trace
                .iter()
                .all(|r| r.encoded && r.submitted && r.completed)
        );
        assert!(
            peak <= 288 * 1024 * 1024,
            "completed raw typed producers overlap later output storage: {peak}"
        );
    }
}

#[test]
fn saved_pages_preserve_every_value_with_three_dispatches_after_large_count_groups() {
    let program = Program::new(
        vec![word(CAPTURE, 0, 0), word(EMIT, 0, 1)],
        vec![
            b"value".to_vec(),
            b"match".to_vec(),
            b"role".to_vec(),
            b"kept".to_vec(),
        ],
        vec![],
        vec![0, 1],
        vec![],
    )
    .unwrap()
    .with_directives(vec![
        Directive::Set {
            key: 2,
            value: None,
        },
        Directive::SetCapture {
            capture: 0,
            key: 2,
            value: Some(3),
        },
        Directive::Downcase { capture: 0 },
    ])
    .unwrap();
    let _guard = metal_backend::metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let queue = device.new_command_queue();
    for count in [65537usize, 16383, 16384, 16385] {
        let file = metal_register_conformance::source_file("# VALUE\n".repeat(count));
        let doc = file.restore(&|| false).unwrap();
        let anchors = [vec![(1..=count as u32).collect::<Vec<_>>()]];
        let expected = execute_batch(
            std::slice::from_ref(&program),
            std::slice::from_ref(&doc),
            Some(&anchors),
        )
        .unwrap();
        assert_eq!(expected[0][0].len(), count);
        let resident = ResidentInput::new(
            metal_grid::MetalGridInput::build(
                std::slice::from_ref(&program),
                std::slice::from_ref(&doc),
                &anchors,
            )
            .unwrap(),
            None,
        )
        .unwrap();
        let input = metal_register_execution::upload(&device, &resident).unwrap();
        let programs =
            metal_capture_predicates::PredicatePrograms::new(&device, &queue, &resident, &input)
                .unwrap();
        let roots = (0..count)
            .map(|task| (task as u32, task as u32 + 1))
            .collect::<Vec<_>>();
        let (arenas, errors) = execute(
            &device,
            &queue,
            &resident,
            &input,
            vec![raw_batch(&device, &doc, &roots, 1)],
            vec![None],
            &programs,
        )
        .unwrap();
        drop(programs);
        let saved = metal_output_pages::SavedOutput::new(
            &resident,
            device.clone(),
            queue.clone(),
            input,
            arenas,
            errors,
        )
        .unwrap();
        let scope = metal_backend::launch::LaunchScope::enter();
        let start = scope.snapshot().len();
        let started = std::time::Instant::now();
        let actual = saved
            .execute()
            .unwrap()
            .into_outcome()
            .unwrap()
            .into_result()
            .unwrap();
        assert_eq!(actual.len(), 1);
        assert_eq!(actual[0].len(), 1);
        assert_eq!(actual[0][0].len(), count);
        for (row, (actual, expected)) in actual[0][0].iter().zip(&expected[0][0]).enumerate() {
            assert_eq!(actual, expected, "count={count} row={row}");
        }
        let trace = scope.snapshot();
        let trace = &trace[start..];
        eprintln!(
            "SAVED_PAGE_DISPATCH rows={count} dispatches={} seconds={}",
            trace.len(),
            started.elapsed().as_secs_f64()
        );
        assert!(
            trace
                .iter()
                .all(|r| r.encoded && r.submitted && r.completed)
        );
        assert!(
            trace.len() <= 3,
            "one complete saved page exceeds its dispatch allowance: {}",
            trace.len()
        );
    }
}
