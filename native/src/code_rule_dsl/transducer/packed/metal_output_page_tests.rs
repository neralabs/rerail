//! Publication keeps complete file intervals while bounded GPU pages retain typed rows.
use super::*;

#[test]
fn finite_output_larger_than_the_raw_arena_limit_publishes_every_occurrence() {
    use metal_grid_plan::{ExecutionPlan, ResidentInput};
    let file_count = 1024_usize;
    let program_count = 128_usize;
    let root_count = 256_usize;
    let file = tape(&"# node\n".repeat(root_count));
    assert_eq!(file.transducer_len(), root_count + 1);
    let files = vec![file; file_count];
    let program = Program::new(
        vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"item".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let programs = vec![program; program_count];
    let selected: Arc<[u32]> = (1..=root_count as u32).collect::<Vec<_>>().into();
    let anchors = vec![vec![selected; program_count]; file_count];
    let resident = ResidentInput::new(
        metal_grid::MetalGridInput::build_shared(&programs, &files, &anchors).unwrap(),
        None,
    )
    .unwrap();
    let plan = ExecutionPlan::new(&resident, &programs).unwrap();
    let scope = metal_backend::launch::LaunchScope::enter();
    let buffers = metal_backend::BufferObservation::enter();
    let mut completed = 0;
    let mut matches_seen = 0_usize;
    let actual = metal_grid_executor::GuardedExecutor::new(resident, plan)
        .unwrap()
        .execute_pages(&mut |page, end| {
            assert_eq!(end, (completed + 64).min(file_count));
            assert_eq!(
                page.host_arena_counts(),
                [
                    (end - completed) * program_count * root_count,
                    (end - completed) * program_count * root_count,
                    0,
                    0
                ]
            );
            for file in completed..end {
                for program in 0..program_count {
                    let (matches, error) = page.program_results(file, program)?;
                    assert_eq!(error, None);
                    assert_eq!(matches.len(), root_count);
                    for (root, found) in matches.into_iter().enumerate() {
                        assert_eq!(found.task_ordinal(), (program * root_count + root) as u32);
                        assert_eq!(found.local_ordinal(), 0);
                        assert_eq!(found.start_node(), root as u32 + 1);
                        assert_eq!(found.label(), 0);
                        assert!(found.properties().next().is_none());
                        assert!(found.dynamic_properties().next().is_none());
                        let mut captures = found.captures();
                        let capture = captures.next().expect("one capture for every finite root");
                        assert!(captures.next().is_none());
                        assert_eq!(capture.label(), 0);
                        assert_eq!(capture.node(), root as u32 + 1);
                        assert_eq!(capture.span(), root * 7..root * 7 + 6);
                        assert_eq!(capture.transformed_text(), None);
                        assert!(capture.properties().next().is_none());
                        matches_seen += 1;
                    }
                }
            }
            completed = end;
            Ok(())
        });
    assert_eq!(
        actual,
        Ok(()),
        "all finite results must fit without retaining the complete raw arena"
    );
    assert_eq!(completed, file_count);
    assert_eq!(matches_seen, 33_554_432);
    let trace = scope.snapshot();
    assert!(
        trace
            .iter()
            .all(|record| record.completed && record.submitted)
    );
    assert!(trace.len() <= 600);
    let peak = trace
        .iter()
        .map(|record| record.peak_application_buffer_bytes)
        .max()
        .unwrap();
    assert!(peak <= 2_000_000_000);
    assert_eq!(buffers.wait_for_live_bytes(0), 0);
    eprintln!(
        "FINITE_OUTPUT_STORAGE rows={matches_seen} launches={} peak={peak}",
        trace.len()
    );
}

#[test]
fn compact_constant_spans_merge_shared_transforms_and_preserve_witness_limits() {
    use metal_grid_plan::{ExecutionPlan, ResidentInput};
    let file_count = 128_usize;
    let program_count = 64_usize;
    let root_count = 512_usize;
    let file = tape(&"# node\n".repeat(root_count));
    let base = Program::new(
        vec![word(CAPTURE, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![
            b"item".to_vec(),
            b"unused".to_vec(),
            b"role".to_vec(),
            b"flag".to_vec(),
            vec![],
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
            key: 3,
            value: Some(4),
        },
    ])
    .unwrap();
    let mut populated = base.directives.clone();
    populated.push(Directive::Downcase { capture: 0 });
    let mut empty = base.directives.clone();
    empty.extend([
        Directive::Offset {
            capture: 0,
            offsets: [0, 0, 0, -6],
        },
        Directive::Downcase { capture: 0 },
    ]);
    let mut rejected = base.directives.clone();
    rejected.push(Directive::Offset {
        capture: 0,
        offsets: [0, 0, 0, -7],
    });
    let templates = [
        base.clone(),
        base.clone().with_directives(populated).unwrap(),
        base.clone().with_directives(empty).unwrap(),
        base.with_directives(rejected).unwrap(),
    ];
    let roots = (1..=root_count as u32).collect::<Vec<_>>();
    let expected = execute_batch(
        &templates,
        std::slice::from_ref(&file),
        Some(&[vec![roots.clone(); templates.len()]]),
    )
    .unwrap();
    assert_eq!(
        expected[0].iter().map(Vec::len).collect::<Vec<_>>(),
        [root_count, root_count, root_count, 0]
    );
    for (kind, matches) in expected[0].iter().enumerate().take(3) {
        for (root, found) in matches.iter().enumerate() {
            assert_eq!(found.properties, [(2, None)]);
            assert_eq!(found.captures.len(), 2);
            for capture in &found.captures {
                assert_eq!(capture.label, 0);
                assert_eq!(capture.node, root as u32 + 1);
                assert_eq!(capture.properties, [(3, Some(4))]);
                assert_eq!(
                    capture.span,
                    root * 7..root * 7 + if kind == 2 { 0 } else { 6 }
                );
                assert_eq!(
                    capture.text.as_deref(),
                    match kind {
                        0 => None,
                        1 => Some(b"# node".as_slice()),
                        2 => Some(b"".as_slice()),
                        _ => unreachable!(),
                    }
                );
            }
        }
    }
    let template_for = |program: usize| match program {
        31 => 1,
        32 => 2,
        33 => 3,
        _ => 0,
    };
    let programs = (0..program_count)
        .map(|i| templates[template_for(i)].clone())
        .collect::<Vec<_>>();
    let files = vec![file; file_count];
    let roots: Arc<[u32]> = roots.into();
    let anchors = vec![vec![roots; program_count]; file_count];
    for limit in [0, 1, 257_u32] {
        let resident = ResidentInput::new(
            metal_grid::MetalGridInput::build_shared(&programs, &files, &anchors).unwrap(),
            Some(&vec![limit; program_count]),
        )
        .unwrap();
        let plan = ExecutionPlan::new(&resident, &programs).unwrap();
        let scope = metal_backend::launch::LaunchScope::enter();
        let mut completed = 0;
        let mut seen = 0_usize;
        metal_grid_executor::GuardedExecutor::new(resident, plan)
            .unwrap()
            .execute_pages(&mut |page, end| {
                assert_eq!(end, (completed + 64).min(file_count));
                for file in completed..end {
                    for program in 0..program_count {
                        let kind = template_for(program);
                        let expected = &expected[0][kind];
                        let count = expected.len().min(limit as usize);
                        let (actual, error) = page.program_results(file, program)?;
                        assert_eq!(error, None);
                        assert_eq!(
                            actual.len(),
                            count,
                            "file {file}, Program {program}, limit {limit}"
                        );
                        for (actual, expected) in
                            actual.into_iter().zip(expected.iter().take(count))
                        {
                            assert_eq!(
                                actual.task_ordinal(),
                                expected.task_ordinal - kind as u32 * root_count as u32
                                    + program as u32 * root_count as u32
                            );
                            assert_eq!(actual.local_ordinal(), expected.local_ordinal);
                            assert_eq!(actual.label(), expected.label);
                            assert_eq!(actual.start_node(), expected.start);
                            assert_eq!(
                                actual.properties().collect::<Vec<_>>(),
                                expected.properties
                            );
                            assert!(actual.dynamic_properties().next().is_none());
                            let captures = actual.captures().collect::<Vec<_>>();
                            assert_eq!(captures.len(), expected.captures.len());
                            for (actual, expected) in captures.into_iter().zip(&expected.captures) {
                                assert_eq!(actual.label(), expected.label);
                                assert_eq!(actual.node(), expected.node);
                                assert_eq!(actual.span(), expected.span);
                                assert_eq!(actual.transformed_text(), expected.text.as_deref());
                                assert_eq!(
                                    actual.properties().collect::<Vec<_>>(),
                                    expected.properties
                                );
                            }
                            seen += 1;
                        }
                    }
                }
                completed = end;
                Ok(())
            })
            .unwrap();
        assert_eq!(completed, file_count);
        assert_eq!(
            seen,
            file_count * (program_count - 1) * (limit as usize).min(root_count)
        );
        let trace = scope.snapshot();
        assert_eq!(
            trace
                .iter()
                .filter(|r| r.kernel == "constant_output_scatter")
                .count(),
            1,
            "all eligible Programs share one physical producer"
        );
        assert!(trace.len() <= 600 && trace.iter().all(|r| r.completed));
        let peak = trace
            .iter()
            .map(|r| r.peak_application_buffer_bytes)
            .max()
            .unwrap();
        eprintln!(
            "CONSTANT_MIXED_OUTPUT limit={limit} rows={seen} launches={} peak={peak}",
            trace.len()
        );
    }
}

#[test]
fn saved_output_pages_retain_only_the_current_complete_files_in_host_arenas() {
    use metal_grid_plan::{ExecutionPlan, ResidentInput};
    let programs = [Program::new(
        vec![word(CAPTURE, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"text".to_vec(), b"^# ".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap()
    .with_directives(vec![
        Directive::Set {
            key: 0,
            value: Some(1),
        },
        Directive::SetCapture {
            capture: 0,
            key: 0,
            value: Some(1),
        },
        Directive::Strip {
            capture: 0,
            source: 1,
            compiled: regex::Regex::new("^# ").unwrap(),
        },
        Directive::Downcase { capture: 0 },
    ])
    .unwrap()];
    let files = (0..65)
        .map(|i| tape(&format!("# F{i:03} UPPERé\n")))
        .collect::<Vec<_>>();
    let anchors = vec![vec![vec![0]]; files.len()];
    let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    assert!(
        expected
            .iter()
            .all(|file| file[0].len() == 1 && file[0][0].captures.len() == 2)
    );
    let resident = ResidentInput::new(
        metal_grid::MetalGridInput::build(&programs, &files, &anchors).unwrap(),
        None,
    )
    .unwrap();
    let plan = ExecutionPlan::new(&resident, &programs).unwrap();
    let buffers = metal_backend::BufferObservation::enter();
    metal_register_execution::take_payload_reads();
    let mut observations = Vec::new();
    let scope = metal_backend::launch::LaunchScope::enter();
    let actual = metal_grid_executor::GuardedExecutor::new(resident, plan)
        .unwrap()
        .execute_pages(&mut |page, end| {
            let counts = page.host_arena_counts();
            let trace = scope.snapshot();
            let count = trace
                .iter()
                .rev()
                .find(|r| r.kernel == "output_page_count")
                .unwrap();
            let scatter = trace
                .iter()
                .rev()
                .find(|r| r.kernel == "output_page_scatter")
                .unwrap();
            let durable = std::iter::once(count.buffers[0])
                .chain(scatter.buffers[..5].iter().copied())
                .collect::<std::collections::BTreeMap<_, _>>()
                .values()
                .sum::<u64>() as usize;
            assert_eq!(
                buffers.wait_for_live_bytes(durable),
                durable,
                "semantic and page scratch must retire before publication"
            );
            assert_eq!(
                metal_register_execution::take_payload_reads(),
                counts,
                "only the published payload crosses to host memory"
            );
            let physical_files = page.physical_file_count();
            observations.push((end, counts, physical_files, page.into_outcome()?));
            Ok(())
        });
    let trace = scope.snapshot();
    drop(scope);
    assert_eq!(
        buffers.wait_for_live_bytes(0),
        0,
        "all GPU output owners retire after the final callback"
    );
    assert_eq!(actual, Ok(()));
    assert_eq!(
        observations.iter().map(|o| o.0).collect::<Vec<_>>(),
        vec![64, 65]
    );
    let mut first = 0;
    for (end, counts, physical_files, page) in observations {
        assert_eq!(
            physical_files, 65,
            "immutable input remains shared across output pages"
        );
        assert!(page.errors.iter().flatten().all(Option::is_none));
        for (file, outputs) in page.outputs.iter().enumerate() {
            if (first..end).contains(&file) {
                assert_eq!(outputs, &expected[file]);
            } else {
                assert!(outputs.iter().all(Vec::is_empty));
            }
        }
        let expected_bytes = expected[first..end]
            .iter()
            .flat_map(|f| &f[0])
            .flat_map(|r| &r.captures)
            .map(|c| c.text.as_ref().unwrap().len())
            .sum::<usize>();
        assert_eq!(
            counts,
            [
                end - first,
                (end - first) * 2,
                (end - first) * 3,
                expected_bytes
            ],
            "publication must retain only its current file interval's output arenas"
        );
        first = end;
    }
    assert_eq!(
        trace
            .iter()
            .filter(|r| r.kernel == "register_roots")
            .count(),
        1
    );
    assert_eq!(
        trace
            .iter()
            .filter(|r| r.kernel == "regex_frontier_advance")
            .count(),
        1
    );
    assert!(
        trace.len() <= 600
            && trace
                .iter()
                .all(|r| r.encoded && r.submitted && r.completed)
    );
}

fn execute_bounded(
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
    bounds: metal_output_pages::PageBounds,
) -> (
    Result<BatchOutcome, Error>,
    Vec<metal_backend::launch::DispatchRecord>,
) {
    let resident = metal_grid_plan::ResidentInput::new(
        metal_grid::MetalGridInput::build(programs, files, anchors).unwrap(),
        None,
    )
    .unwrap();
    let plan = metal_grid_plan::ExecutionPlan::new(&resident, programs).unwrap();
    let scope = metal_backend::launch::LaunchScope::enter();
    let output = metal_grid_executor::GuardedExecutor::new(resident, plan)
        .unwrap()
        .with_output_bounds(bounds)
        .execute()
        .and_then(PackedExecution::into_outcome);
    (output, scope.snapshot())
}

#[test]
fn tiny_gpu_pages_preserve_duplicate_captures_transforms_mime_and_raw_ordinals() {
    let source = "# APPLICATION/JSON\n# TEXT/HTMLé\n# IMAGE/PNG\n";
    let files = [tape(source), tape("")];
    let query = "(module (comment)? @first (comment)* @rest)";
    let query =
        Program::compile_tree_sitter_query_pattern(query, &SupportLang::Python.get_ts_language())
            .unwrap();
    let ordinary = Program::new(
        vec![word(CAPTURE, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"text".to_vec(), b"^# ".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap()
    .with_directives(vec![
        Directive::SetCapture {
            capture: 0,
            key: 0,
            value: Some(1),
        },
        Directive::Strip {
            capture: 0,
            source: 1,
            compiled: regex::Regex::new("^# ").unwrap(),
        },
        Directive::Downcase { capture: 0 },
        Directive::SetLanguageFromMimeType { capture: 0 },
    ])
    .unwrap();
    let programs = [ordinary, query];
    let anchors = files
        .iter()
        .map(|doc| vec![(0..doc.transducer_len() as u32).collect(), vec![0]])
        .collect::<Vec<_>>();
    let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    assert!(expected[0][0].len() > 3);
    assert!(
        expected[0][0]
            .iter()
            .any(|r| !r.dynamic_properties.is_empty())
    );
    assert!(expected[0][1].len() > 1);
    let mut semantic_traces = Vec::new();
    for rows in [1, 2, 4096] {
        let (actual, trace) = execute_bounded(
            &programs,
            &files,
            &anchors,
            metal_output_pages::PageBounds {
                rows,
                ..Default::default()
            },
        );
        let actual = actual.unwrap().into_result().unwrap();
        assert_eq!(actual, expected, "row capacity {rows}");
        let page_counts = trace
            .iter()
            .filter(|r| r.kernel == "output_page_count")
            .count();
        let page_copies = trace
            .iter()
            .filter(|r| r.kernel == "output_page_scatter")
            .count();
        let total = expected.iter().flatten().map(Vec::len).sum::<usize>();
        assert_eq!(page_counts, total.div_ceil(rows as usize));
        assert_eq!(page_copies, page_counts);
        semantic_traces.push(
            trace
                .iter()
                .filter(|r| !r.kernel.starts_with("output_page_"))
                .map(|r| r.kernel.clone())
                .collect::<Vec<_>>(),
        );
        assert!(
            trace.len() <= 600
                && trace
                    .iter()
                    .all(|r| r.encoded && r.submitted && r.completed)
        );
        eprintln!(
            "SAVED_PAGES_EVIDENCE {}",
            serde_json::json!({"rows":rows,"outputs":total,"launches":trace.len(),"pages":page_counts,"peakMetalBytes":trace.iter().map(|r|r.peak_application_buffer_bytes).max()})
        );
    }
    assert!(
        semantic_traces.windows(2).all(|t| t[0] == t[1]),
        "page exhaustion must not replay any semantic stage"
    );
}

#[test]
fn a_row_larger_than_an_empty_page_refuses_without_completing_the_file() {
    let programs = [Program::new(
        vec![word(CAPTURE, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"x".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap()
    .with_directives(vec![
        Directive::Downcase { capture: 0 },
        Directive::SetCapture {
            capture: 0,
            key: 0,
            value: None,
        },
    ])
    .unwrap()];
    let files = [tape("# UPPERé\n")];
    let anchors = [vec![vec![0]]];
    let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    assert_eq!(expected[0][0][0].captures.len(), 2);
    for bounds in [
        metal_output_pages::PageBounds {
            captures: 1,
            ..Default::default()
        },
        metal_output_pages::PageBounds {
            properties: 1,
            ..Default::default()
        },
        metal_output_pages::PageBounds {
            bytes: 1,
            ..Default::default()
        },
    ] {
        let resident = metal_grid_plan::ResidentInput::new(
            metal_grid::MetalGridInput::build(&programs, &files, &anchors).unwrap(),
            None,
        )
        .unwrap();
        let plan = metal_grid_plan::ExecutionPlan::new(&resident, &programs).unwrap();
        let mut completed = Vec::new();
        let actual = metal_grid_executor::GuardedExecutor::new(resident, plan)
            .unwrap()
            .with_output_bounds(bounds)
            .execute_pages(&mut |_, end| {
                completed.push(end);
                Ok(())
            });
        assert_eq!(actual, Err(Error::BackendCapacity));
        assert!(completed.is_empty());
    }
}

#[test]
fn page_exhaustion_uses_the_same_outer_budget_and_never_completes_a_refused_file() {
    let programs = [Program::new(
        vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"x".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap()];
    let files = [tape(&"# row\n".repeat(320))];
    let anchors = [vec![
        (0..files[0].transducer_len() as u32).collect::<Vec<_>>(),
    ]];
    assert_eq!(anchors[0][0].len(), 321);
    let resident = metal_grid_plan::ResidentInput::new(
        metal_grid::MetalGridInput::build(&programs, &files, &anchors).unwrap(),
        None,
    )
    .unwrap();
    let plan = metal_grid_plan::ExecutionPlan::new(&resident, &programs).unwrap();
    let scope = metal_backend::launch::LaunchScope::enter();
    let mut complete = Vec::new();
    let actual = metal_grid_executor::GuardedExecutor::new(resident, plan)
        .unwrap()
        .with_output_bounds(metal_output_pages::PageBounds {
            rows: 1,
            ..Default::default()
        })
        .execute_pages(&mut |_, end| {
            complete.push(end);
            Ok(())
        });
    let trace = scope.snapshot();
    drop(scope);
    assert_eq!(actual, Err(Error::BackendLaunchLimit));
    assert!(complete.is_empty());
    assert_eq!(trace.iter().filter(|r| r.encoded).count(), 600);
    assert_eq!(
        trace
            .iter()
            .filter(|r| r.kernel == "register_roots")
            .count(),
        1
    );
    let refused = trace.iter().position(|r| !r.encoded).unwrap();
    assert_eq!(refused, 600);
    let command = trace[refused].command;
    assert!(
        trace[refused..]
            .iter()
            .all(|r| !r.encoded && !r.submitted && !r.completed)
    );
    assert!(
        trace
            .iter()
            .filter(|r| r.command != command)
            .all(|r| r.encoded && r.submitted && r.completed)
    );
    assert!(
        trace
            .iter()
            .filter(|r| r.command == command)
            .all(|r| !r.submitted && !r.completed)
    );
    assert!(trace[refused].kernel.starts_with("output_page_"));
}

#[test]
fn late_operand_and_work_failures_are_reduced_before_any_file_page_is_published() {
    let good = Program::new(
        vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"x".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let bad = Program::new(
        vec![
            word(FORK, 0, 3),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 0),
            word(REPEAT_END, 0, 0),
            word(EMIT, 0, 0),
        ],
        vec![b"x".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let pattern = "^# (?:a|aa)*b$";
    let work = Program::new_with_capture_predicates(
        vec![word(CAPTURE, 0, 0), word(ASSERT, 0, 0), word(EMIT, 0, 0)],
        vec![pattern.as_bytes().to_vec(), b"x".to_vec()],
        vec![TextPredicate::Regex {
            source: 0,
            compiled: regex::Regex::new(pattern).unwrap(),
        }],
        vec![CapturePredicate::Text {
            capture: 0,
            predicate: 0,
            negated: false,
            any: false,
        }],
        vec![1],
        vec![],
    )
    .unwrap()
    .with_tree_sitter_query_semantics()
    .unwrap();
    let programs = [good, bad, work];
    let files = [tape(&format!("# ab\n# {}", "a".repeat(200_000)))];
    let anchors = [vec![vec![0, 1, 2], vec![0], vec![1, 2]]];
    let expected = execute_with_backend_outcomes(
        ExecutionBackend::CpuInterpreter,
        &programs,
        &files,
        &anchors,
    )
    .unwrap();
    assert_eq!(
        expected.errors,
        [vec![
            None,
            Some(Error::InvalidOperand(3)),
            Some(Error::WorkLimit)
        ]]
    );
    assert_eq!(expected.outputs[0][0].len(), 3);
    for limit in [0, 1] {
        let resident = metal_grid_plan::ResidentInput::new(
            metal_grid::MetalGridInput::build(&programs, &files, &anchors).unwrap(),
            Some(&[limit; 3]),
        )
        .unwrap();
        let plan = metal_grid_plan::ExecutionPlan::new(&resident, &programs).unwrap();
        let scope = metal_backend::launch::LaunchScope::enter();
        let mut actual = Vec::new();
        let result = metal_grid_executor::GuardedExecutor::new(resident, plan)
            .unwrap()
            .with_output_bounds(metal_output_pages::PageBounds {
                rows: 1,
                ..Default::default()
            })
            .execute_pages(&mut |page, end| {
                actual.push((end, page.into_outcome()?));
                Ok(())
            });
        drop(scope);
        assert_eq!(result, Ok(()));
        assert_eq!(actual.len(), 1);
        assert_eq!(actual[0].0, 1);
        assert_eq!(actual[0].1.errors, expected.errors);
        let mut outputs = expected.outputs.clone();
        for rows in outputs.iter_mut().flatten() {
            rows.truncate(limit as usize);
        }
        assert_eq!(actual[0].1.outputs, outputs);
    }
}

#[test]
fn broad_output_pages_preserve_complete_rows_without_semantic_replay() {
    let programs = [Program::new(
        vec![word(CAPTURE, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"text".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap()
    .with_directives(vec![Directive::Downcase { capture: 0 }])
    .unwrap()];
    for (comments, capacity) in [(5120, 8192), (16_640, 32_768)] {
        let files = [tape(&"# ROWé\n".repeat(comments))];
        let anchors = [vec![
            (0..files[0].transducer_len() as u32).collect::<Vec<_>>(),
        ]];
        let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        assert_eq!(expected[0][0].len(), comments + 1);
        let (actual, trace) = execute_bounded(
            &programs,
            &files,
            &anchors,
            metal_output_pages::PageBounds {
                rows: capacity,
                ..Default::default()
            },
        );
        let actual = actual
            .expect("a broad GPU page must accept all complete rows")
            .into_result()
            .unwrap();
        assert_eq!(actual, expected);
        let counts = trace
            .iter()
            .filter(|r| r.kernel == "output_page_count")
            .collect::<Vec<_>>();
        assert_eq!(counts.len(), 1);
        assert_eq!(
            counts[0].grid,
            [(comments + 1).div_ceil(16384) as u64, 1, 1]
        );
        assert_eq!(
            trace
                .iter()
                .filter(|r| r.kernel == "output_page_scatter")
                .count(),
            1
        );
        assert_eq!(
            trace
                .iter()
                .filter(|r| r.kernel == "register_roots")
                .count(),
            1
        );
        assert!(
            trace.len() <= 600
                && trace
                    .iter()
                    .all(|r| r.encoded && r.submitted && r.completed)
        );
    }
}

#[test]
fn a_single_typed_batch_reuses_its_gpu_candidate_arenas_without_a_gather_copy() {
    let programs = [Program::new(
        vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"x".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap()
    .with_directives(vec![Directive::Downcase { capture: 0 }])
    .unwrap()];
    let files = [tape(&"# UPPERé\n".repeat(80))];
    let anchors = [vec![
        (0..files[0].transducer_len() as u32).collect::<Vec<_>>(),
    ]];
    let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    assert_eq!(expected[0][0].len(), 81);
    let (actual, trace) = execute_bounded(&programs, &files, &anchors, Default::default());
    assert_eq!(actual.unwrap().into_result().unwrap(), expected);
    let producer = trace
        .iter()
        .find(|r| r.kernel == "register_scatter")
        .unwrap();
    let rows = trace
        .iter()
        .find(|r| r.kernel == "output_initialize_results")
        .unwrap();
    let captures = trace
        .iter()
        .find(|r| r.kernel == "output_initialize_captures")
        .unwrap();
    assert_eq!(
        rows.buffers[1], producer.buffers[6],
        "one owned batch must not copy its complete result arena before transformation"
    );
    assert_eq!(
        captures.buffers[2], producer.buffers[7],
        "one owned batch must not copy its complete capture arena before transformation"
    );
}

#[test]
fn saved_boundaries_publish_proved_empty_file_intervals_without_replaying_gpu_work() {
    let programs = [Program::new(
        vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"x".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap()];
    let files = vec![tape("# one\n"); 129];
    let mut anchors = vec![vec![vec![]]; 129];
    anchors[128][0].push(0);
    let resident = metal_grid_plan::ResidentInput::new(
        metal_grid::MetalGridInput::build(&programs, &files, &anchors).unwrap(),
        None,
    )
    .unwrap();
    let plan = metal_grid_plan::ExecutionPlan::new(&resident, &programs).unwrap();
    metal_register_execution::take_payload_reads();
    let scope = metal_backend::launch::LaunchScope::enter();
    let mut pages = Vec::new();
    let actual = metal_grid_executor::GuardedExecutor::new(resident, plan)
        .unwrap()
        .execute_pages(&mut |page, end| {
            pages.push((
                end,
                metal_register_execution::take_payload_reads(),
                page.into_outcome()?,
            ));
            Ok(())
        });
    let trace = scope.snapshot();
    drop(scope);
    assert_eq!(actual, Ok(()));
    assert_eq!(
        pages.iter().map(|p| p.0).collect::<Vec<_>>(),
        [64, 128, 129]
    );
    assert_eq!(pages[0].1, [0; 4]);
    assert_eq!(pages[1].1, [0; 4]);
    assert_eq!(pages[2].1, [1, 1, 0, 0]);
    for (i, (_, _, page)) in pages.iter().enumerate() {
        assert!(page.errors.iter().flatten().all(Option::is_none));
        assert_eq!(
            page.outputs.iter().flatten().map(Vec::len).sum::<usize>(),
            usize::from(i == 2)
        );
    }
    let row = &pages[2].2.outputs[128][0][0];
    assert_eq!(row.start, 0);
    assert_eq!(row.local_ordinal, 0);
    assert_eq!(row.captures.len(), 1);
    assert_eq!(
        trace
            .iter()
            .filter(|r| r.kernel == "register_roots")
            .count(),
        1
    );
    assert_eq!(
        trace
            .iter()
            .filter(|r| r.kernel == "output_page_count")
            .count(),
        2
    );
    assert_eq!(
        trace
            .iter()
            .filter(|r| r.kernel == "output_page_scatter")
            .count(),
        1
    );
    assert!(
        trace
            .iter()
            .all(|r| r.encoded && r.submitted && r.completed)
    );
}
