//! A cheap necessary condition must preserve complete shared predicate output.
use super::metal_backend::launch::LaunchScope;
use super::metal_grid::MetalGridInput;
use super::metal_grid_executor::GuardedExecutor;
use super::metal_grid_plan::{ExecutionPlan, ResidentInput};
use super::*;

#[test]
fn value_prefixes_preserve_mixed_capture_groups_and_every_negated_any_all_result() {
    let file = super::metal_register_conformance::source_file(
        "lower(Alpha)\nlower(lower)\nAlpha(Alpha)\n".to_owned(),
    );
    let doc = file.restore(&|| false).unwrap();
    let base = Program::compile_tree_sitter_query_pattern(
        r#"((call function: (identifier) @v arguments: (argument_list (identifier) @v)) (#match? @v "^[A-Z]"))"#,
        &file.grammar.language(),
    ).unwrap();
    let kind = file.grammar.language().id_for_node_kind("call", true);
    let roots = (0..doc.transducer_len() as u32)
        .filter(|&n| doc.transducer_kind(n) == kind)
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), 3);
    let programs = (0..4)
        .map(|mode| {
            let mut predicates = base.capture_predicates.clone();
            let CapturePredicate::Text { negated, any, .. } = &mut predicates[0] else {
                panic!("one text predicate")
            };
            *negated = mode & 1 != 0;
            *any = mode & 2 != 0;
            base.clone().with_capture_predicates(predicates).unwrap()
        })
        .collect::<Vec<_>>();
    let anchors = vec![vec![roots; programs.len()]];
    let wanted = execute_batch(&programs, std::slice::from_ref(&doc), Some(&anchors)).unwrap();
    assert_eq!(
        wanted[0].iter().map(Vec::len).collect::<Vec<_>>(),
        [1, 1, 2, 2]
    );
    assert_eq!(
        metal_program_backend::execute(&programs, &[doc], &anchors).unwrap(),
        wanted
    );
}

#[test]
fn bounded_prefix_operator_keeps_a_real_aggregate_work_failure_on_the_value_frontier() {
    use super::metal_backend::launch::{commit_checked, wait_checked};
    use super::metal_backend::{MappedBuffer, pipeline_named};
    use metal::Device;
    let file = super::metal_register_conformance::source_file("lower\n".to_owned());
    let doc = file.restore(&|| false).unwrap();
    assert_eq!(doc.transducer_text(2), b"lower");
    let pattern = "^(?:A?){4096}Z";
    let mut words = vec![word(
        KIND,
        0,
        file.grammar.language().id_for_node_kind("identifier", true),
    )];
    words.extend(std::iter::repeat_n(word(CAPTURE, 0, 0), 200));
    words.push(word(EMIT, 0, 1));
    let large = Program::new(
        words,
        vec![
            b"v".to_vec(),
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
        negated: false,
        any: false,
    }])
    .unwrap();
    let mut small = large.clone();
    small.words = vec![word(CAPTURE, 0, 0), word(EMIT, 0, 1)];
    small.validate().unwrap();
    let value::ValueProgram::Regex(nfa) = &large.value_programs[0] else {
        panic!("one NFA value")
    };
    let (member_result, member_work) = nfa.test_work_count(b"lower");
    assert_eq!(member_result, Ok(false));
    assert!(member_work * 200 > MAX_STEPS_PER_PATH);
    eprintln!(
        "BOUNDED_PREFIX_BUDGET_CONTROL memberWork={member_work} captures=200 limit={MAX_STEPS_PER_PATH}"
    );
    assert_eq!(large.execute_roots(&doc, [2]), Err(Error::WorkLimit));
    assert_eq!(small.execute_roots(&doc, [2]), Ok(vec![]));
    let programs = [large, small];
    let resident = ResidentInput::new(
        MetalGridInput::build(&programs, &[doc], &[vec![vec![2]; 2]]).unwrap(),
        None,
    )
    .unwrap();
    let _guard = metal_backend::metal_execution_guard().unwrap();
    let scope = LaunchScope::enter();
    let device = Device::system_default().unwrap();
    let queue = device.new_command_queue();
    let input = metal_register_execution::upload(&device, &resident).unwrap();
    let output = MappedBuffer::zeroed::<u32>(&device, 4).unwrap();
    let source = concat!(
        include_str!("metal_resident.metal"),
        include_str!("metal_value_prefix.metal"),
        r#"
kernel void prefix_budget_probe(device const uchar* data [[buffer(0)]],device uint* output [[buffer(1)]],uint i [[thread_position_in_grid]]){
 if(i>=2u)return;View v=view(data);Program pg=v.programs[i];uint captures=0u;
 for(uint pc=0u;pc<pg.structural_count;pc++)captures+=uint((v.words[pg.structural_start+pc]&255u)==0x20u);
 output[2u*i]=register_prefix_label(v,pg,captures);output[2u*i+1u]=uint(register_prefix_possible(v,pg,2u));
}"#
    );
    metal::objc::rc::autoreleasepool(|| {
        let command = queue.new_command_buffer();
        metal_parallel::encode(
            command,
            &pipeline_named(source, "prefix_budget_probe").unwrap(),
            "prefix_budget_probe",
            &[&input.buffer, &output.buffer],
            &[],
            1,
            None,
        );
        commit_checked(command).unwrap();
        wait_checked(command).unwrap();
    });
    assert_eq!(
        metal_register_execution::read::<u32>(&output, 4).unwrap(),
        [u32::MAX, 0, 0, 0],
        "the same losing prefix can reject one capture but cannot hide the real200-capture work failure"
    );
    assert_eq!(scope.snapshot().len(), 1);
    assert!(scope.snapshot()[0].completed);
}

#[test]
fn bounded_value_prefixes_keep_complete_regex_and_lua_output_without_retaining_losing_candidates() {
    complete_prefix_evaluation(64, false);
}

#[test]
fn counted_finite_register_work_does_not_partition_by_unrealized_capture_bounds() {
    complete_prefix_evaluation(128, true);
}

fn complete_prefix_evaluation(program_count: usize, require_single_count: bool) {
    let file = super::metal_register_conformance::source_file(format!(
        "{}Alpha\nUPPER\n__magic__\n",
        "lower\n".repeat(509)
    ));
    let doc = file.restore(&|| false).unwrap();
    let kind = file.grammar.language().id_for_node_kind("identifier", true);
    let roots = (0..doc.transducer_len() as u32)
        .filter(|&node| doc.transducer_kind(node) == kind)
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), 512);
    let queries = [
        r#"((identifier) @v @v (#match? @v "^[A-Z]") (#set! role "kept"))"#,
        r#"((identifier) @v @v (#match? @v "^_*[A-Z][A-Z\\d_]*$") (#set! role "kept"))"#,
        r#"((identifier) @v @v (#lua-match? @v "^[A-Z].*[a-z]") (#set! role "kept"))"#,
        r#"((identifier) @v @v (#lua-match? @v "^__[a-zA-Z0-9_]*__$") (#set! role "kept"))"#,
    ];
    let templates = queries
        .iter()
        .map(|q| Program::compile_tree_sitter_query_pattern(q, &file.grammar.language()).unwrap())
        .collect::<Vec<_>>();
    let expected = execute_batch(
        &templates,
        std::slice::from_ref(&doc),
        Some(&[vec![roots.clone(); templates.len()]]),
    )
    .unwrap()
    .pop()
    .unwrap();
    let expected_texts: [&[&[u8]]; 4] = [
        &[b"Alpha", b"UPPER"],
        &[b"UPPER"],
        &[b"Alpha"],
        &[b"__magic__"],
    ];
    for (index, outputs) in expected.iter().enumerate() {
        assert_eq!(outputs.len(), expected_texts[index].len());
        for (output, text) in outputs.iter().zip(expected_texts[index]) {
            assert_eq!(doc.transducer_text(output.start), *text);
            assert_eq!(output.captures.len(), 2);
            assert_eq!(output.local_ordinal, 0);
            assert_eq!(output.properties.len(), 1);
            assert!(output.dynamic_properties.is_empty());
            for capture in &output.captures {
                assert_eq!(capture.node, output.start);
                assert_eq!(capture.span, doc.transducer_span(output.start));
                assert!(capture.text.is_none() && capture.properties.is_empty());
            }
        }
    }
    let programs = (0..program_count)
        .map(|i| templates[i % templates.len()].clone())
        .collect::<Vec<_>>();
    let files = vec![doc; 16];
    let roots: Arc<[u32]> = roots.into();
    let anchors = vec![vec![Arc::clone(&roots); programs.len()]; files.len()];
    let resident = ResidentInput::new(
        MetalGridInput::build_shared(&programs, &files, &anchors).unwrap(),
        None,
    )
    .unwrap();
    let plan = ExecutionPlan::new(&resident, &programs).unwrap();
    let scope = LaunchScope::enter();
    let started = std::time::Instant::now();
    let result = GuardedExecutor::new(resident, plan).unwrap().execute();
    let trace = scope.snapshot();
    let peak = trace
        .iter()
        .map(|r| r.peak_application_buffer_bytes)
        .max()
        .unwrap();
    eprintln!(
        "BOUNDED_VALUE_PREFIX_EVIDENCE {}",
        serde_json::json!({"logicalRoots":files.len()*programs.len()*roots.len(),"dispatches":trace.len(),"peakMetalBytes":peak,"seconds":started.elapsed().as_secs_f64(),"error":format!("{:?}",result.as_ref().err())})
    );
    let actual = result.unwrap();
    let mut observed = 0;
    for file in 0..files.len() {
        for program in 0..programs.len() {
            let (rows, error) = actual.program_results(file, program).unwrap();
            assert_eq!(error, None);
            let expected = &expected[program % templates.len()];
            assert_eq!(rows.len(), expected.len());
            for (row, wanted) in rows.into_iter().zip(expected) {
                assert_eq!(
                    row.task_ordinal(),
                    (program * roots.len()) as u32 + wanted.task_ordinal % roots.len() as u32
                );
                assert_eq!(
                    (row.local_ordinal(), row.start_node(), row.label()),
                    (wanted.local_ordinal, wanted.start, wanted.label)
                );
                assert_eq!(
                    row.captures()
                        .map(|c| Capture {
                            label: c.label(),
                            node: c.node(),
                            span: c.span(),
                            text: c.transformed_text().map(<[u8]>::to_vec),
                            properties: c.properties().collect()
                        })
                        .collect::<Vec<_>>(),
                    wanted.captures
                );
                assert_eq!(row.properties().collect::<Vec<_>>(), wanted.properties);
                assert_eq!(
                    row.dynamic_properties()
                        .map(|(key, value)| (key.to_vec(), value.map(<[u8]>::to_vec)))
                        .collect::<Vec<_>>(),
                    wanted.dynamic_properties
                );
                observed += 1;
            }
        }
    }
    assert_eq!(observed, program_count * 20);
    assert!(
        trace
            .iter()
            .all(|r| r.encoded && r.submitted && r.completed)
    );
    assert!(
        peak <= 128 * 1024 * 1024,
        "a bounded rejection cannot retain the complete losing capture frontier: peak={peak}"
    );
    if require_single_count {
        assert_eq!(
            trace
                .iter()
                .filter(|r| r.kernel == "register_boundary")
                .count(),
            1,
            "one counted finite class must consume its actual output totals before storage allocation"
        );
    }
}
