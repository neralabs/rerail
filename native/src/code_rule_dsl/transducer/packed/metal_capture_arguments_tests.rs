//! Contradictory typed predicate arguments cannot become a match or a clean rejection.
use super::*;
use metal_backend::MappedBuffer;
use metal_capture_arguments::{ArgumentInput, CaptureParams};

#[test]
fn shared_predicate_arguments_reject_contradictory_specs_owners_and_source_spans() {
    let program = Program::new_with_capture_predicates(
        vec![word(CAPTURE, 0, 0), word(ASSERT, 0, 0), word(EMIT, 0, 0)],
        vec![b"item".to_vec(), b"# X".to_vec()],
        vec![TextPredicate::Equal(1)],
        vec![CapturePredicate::Text {
            capture: 0,
            predicate: 0,
            negated: false,
            any: false,
        }],
        vec![0],
        vec![],
    )
    .unwrap();
    let files = [tape("# X")];
    let anchors = [vec![vec![0]]];
    let expected = execute_batch(std::slice::from_ref(&program), &files, Some(&anchors)).unwrap();
    assert_eq!(expected[0][0].len(), 1);
    assert_eq!(expected[0][0][0].captures[0].span, 0..3);
    let _guard = metal_backend::metal_execution_guard().unwrap();
    let _scope = metal_backend::launch::LaunchScope::enter();
    let device = metal::Device::system_default().unwrap();
    let queue = device.new_command_queue();
    let resident = metal_grid_plan::ResidentInput::new(
        metal_grid::MetalGridInput::build(std::slice::from_ref(&program), &files, &anchors)
            .unwrap(),
        None,
    )
    .unwrap();
    let data = metal_register_execution::upload(&device, &resident).unwrap();
    let programs =
        metal_capture_predicates::PredicatePrograms::new(&device, &queue, &resident, &data)
            .unwrap();
    let mut observed = Vec::new();
    for defect in 0..15 {
        let mut spec = [1u32, 0, 0, 0, 0, 0, 0, 0];
        let mut meta = [0u32, 0, 0, 1];
        let mut capture = [0u32, 0, 0, 3];
        let mut span = [0u32, 1];
        let mut owner = 0u32;
        match defect {
            0 => {}
            1 => spec = [u32::MAX, 0, 0, 0, 0, 0, u32::MAX, 0],
            2 => spec[0] = 7,
            3 => spec[6] = 1,
            4 => meta[0] = 1,
            5 => meta[2] = 1,
            6 => capture[2] = 1,
            7 => span[0] = 1,
            8 => capture[1] = 1,
            9 => spec[5] = 2,
            10 => spec[7] = 1,
            11 => owner = 1,
            12..=14 => {}
            _ => unreachable!(),
        }
        let initial = meta;
        let specs = MappedBuffer::from_slice(&device, &[spec]).unwrap();
        let metadata = MappedBuffer::from_slice(&device, &[meta]).unwrap();
        let captures = MappedBuffer::from_slice(&device, &[capture]).unwrap();
        let spans = MappedBuffer::from_slice(&device, &[span]).unwrap();
        let owners = MappedBuffer::from_slice(&device, &[owner]).unwrap();
        let mut status = MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
        let work = MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
        if defect == 12 {
            status = MappedBuffer {
                buffer: work.buffer.clone(),
            };
        }
        let failures = if defect == 13 {
            MappedBuffer {
                buffer: status.buffer.clone(),
            }
        } else {
            MappedBuffer::zeroed::<u32>(&device, 1).unwrap()
        };
        let engine =
            metal_capture_predicates::CapturePredicates::new_inline(&device, &programs, 1, work)
                .unwrap();
        if defect == 14 {
            status = MappedBuffer {
                buffer: engine.outcome_counts().buffer.clone(),
            };
        }
        let input = ArgumentInput {
            data: &data,
            spans: &spans,
            captures: &captures,
            metadata: &metadata,
            owners: &owners,
            specs: &specs,
            status: &status,
            parameters: CaptureParams {
                layout: metal_capture_arguments::ArgumentLayout::Compact,
                results: 1,
                captures: 1,
                step: 0,
                nodes: resident.input.packed.nodes.len() as u32,
                pairs: 0,
                windows: 0,
                units: 0,
                abort_evaluation: 0,
                merge_width: 0,
            },
        };
        let result = engine.execute_arguments(&device, &queue, &resident, &input, &failures, false);
        let actual = metal_register_execution::read::<[u32; 4]>(&metadata, 1).unwrap()[0];
        observed.push((result.err(), actual, initial));
    }
    assert_eq!(observed[0], (None, [0, 0, 0, 1], [0, 0, 0, 1]));
    assert_eq!(
        observed[1..]
            .iter()
            .map(|(error, _, _)| error.clone())
            .collect::<Vec<_>>(),
        vec![Some(Error::BackendFailure); 14]
    );
    for (index, (_, actual, initial)) in observed.iter().enumerate().skip(1) {
        assert_eq!(
            actual, initial,
            "defect{index} must not publish an acceptance decision"
        );
    }
}

struct QueryFixture {
    device: metal::Device,
    queue: metal::CommandQueue,
    resident: metal_grid_plan::ResidentInput,
    data: MappedBuffer,
    rows: MappedBuffer,
    captures: MappedBuffer,
    metadata: MappedBuffer,
    owners: MappedBuffer,
    status: MappedBuffer,
    roots: usize,
}
impl QueryFixture {
    fn new(roots: usize) -> Self {
        use metal_batch_abi::{FinalCaptureRecord, ResultRecord};
        let source = (0..roots)
            .map(|i| if i % 2 == 0 { "# KEEP\n" } else { "# DROP\n" })
            .collect::<String>();
        let file = super::super::metal_register_conformance::source_file(source);
        let doc = file.restore(&|| false).unwrap();
        assert_eq!(doc.transducer_len(), roots + 1);
        let program = Program::new(
            vec![word(CAPTURE, 0, 0), word(CAPTURE, 0, 1), word(EMIT, 0, 2)],
            vec![
                b"value".to_vec(),
                b"unused".to_vec(),
                b"match".to_vec(),
                b"# KEEP".to_vec(),
            ],
            vec![TextPredicate::Equal(3)],
            vec![0, 1, 2],
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
        let sample = execute_batch(
            std::slice::from_ref(&program),
            std::slice::from_ref(&doc),
            Some(&[vec![(1..=roots.min(2) as u32).collect()]]),
        )
        .unwrap();
        assert_eq!(sample[0][0].len(), 1);
        assert_eq!(sample[0][0][0].captures.len(), 2);
        let anchors = [vec![(1..=roots as u32).collect()]];
        let resident = metal_grid_plan::ResidentInput::new(
            metal_grid::MetalGridInput::build(&[program], std::slice::from_ref(&doc), &anchors)
                .unwrap(),
            None,
        )
        .unwrap();
        let device = metal::Device::system_default().unwrap();
        let queue = device.new_command_queue();
        let data = metal_register_execution::upload(&device, &resident).unwrap();
        let row_values = (0..roots)
            .map(|i| ResultRecord {
                task_ordinal: i as u32,
                start: i as u32 + 1,
                label: 2,
                capture_start: i as u32 * 2,
                capture_count: 2,
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let cap_values = (1..=roots as u32)
            .flat_map(|node| (0..2).map(move |label| (node, label)))
            .map(|(node, label)| FinalCaptureRecord {
                node,
                label,
                start: doc.transducer_span(node).start as u32,
                end: doc.transducer_span(node).end as u32,
                text_start: u32::MAX,
                retained: 1,
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let rows = MappedBuffer::from_slice(&device, &row_values).unwrap();
        let captures = MappedBuffer::from_slice(&device, &cap_values).unwrap();
        let metadata = MappedBuffer::from_slice(&device, &vec![[0u32, 0, 0, 1]; roots]).unwrap();
        let owners = MappedBuffer::from_slice(
            &device,
            &(0..roots as u32)
                .flat_map(|row| [row; 2])
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let status = MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
        Self {
            device,
            queue,
            resident,
            data,
            rows,
            captures,
            metadata,
            owners,
            status,
            roots,
        }
    }
    fn input(&self) -> metal_output_dispatch::OperatorInput<'_> {
        metal_output_dispatch::OperatorInput {
            data: &self.data,
            rows: &self.rows,
            captures: &self.captures,
            metadata: &self.metadata,
            owners: &self.owners,
            status: &self.status,
            parameters: metal_output_dispatch::OutputParams {
                results: self.roots as u32,
                captures: self.roots as u32 * 2,
                step: 0,
                max_operators: 0,
                merge_width: 0,
                source_results: 0,
                source_captures: 0,
                result_base: 0,
                capture_base: 0,
                dynamic_slots: 0,
                transform_bytes: 0,
            },
        }
    }
    fn evaluate(
        &self,
        arguments: &metal_capture_arguments::QueryArguments<'_>,
    ) -> Result<(), Error> {
        let programs = metal_capture_predicates::PredicatePrograms::new(
            &self.device,
            &self.queue,
            &self.resident,
            &self.data,
        )?;
        let predicates =
            metal_capture_predicates::CapturePredicates::new(&self.device, &programs, self.roots)?
                .unwrap();
        let failures = MappedBuffer::zeroed::<u32>(&self.device, 1)?;
        predicates.execute_arguments(
            &self.device,
            &self.queue,
            &self.resident,
            &arguments.input(),
            &failures,
            true,
        )
    }
}

#[test]
fn query_argument_preparation_keeps_source_records_instead_of_a_second_capture_relation() {
    let _guard = metal_backend::metal_execution_guard().unwrap();
    let scope = metal_backend::launch::LaunchScope::enter();
    let fixture = QueryFixture::new(131_072);
    let observation = metal_backend::BufferObservation::enter();
    let arguments = metal_capture_arguments::QueryArguments::new(
        &fixture.input(),
        fixture.resident.input.packed.nodes.len(),
    )
    .unwrap();
    let prepared_bytes = observation.live_bytes();
    fixture.evaluate(&arguments).unwrap();
    let flags =
        metal_register_execution::read::<[u32; 4]>(&fixture.metadata, fixture.roots).unwrap();
    for (i, flag) in flags.iter().enumerate() {
        assert_eq!(*flag, [0, 0, 0, u32::from(i % 2 == 0)], "query root {i}");
    }
    assert!(
        scope
            .snapshot()
            .iter()
            .all(|d| d.encoded && d.submitted && d.completed)
    );
    eprintln!(
        "QUERY_ARGUMENT_VIEW roots={} captures={} preparationMetalBytes={prepared_bytes}",
        fixture.roots,
        fixture.roots * 2
    );
    assert!(
        prepared_bytes <= 64 * 1024,
        "query argument preparation copies the candidate relation: {prepared_bytes}"
    );
}

#[test]
fn query_arguments_cannot_hide_a_corrupted_source_capture_behind_a_prepared_snapshot() {
    let _guard = metal_backend::metal_execution_guard().unwrap();
    let _scope = metal_backend::launch::LaunchScope::enter();
    let fixture = QueryFixture::new(1);
    let arguments = metal_capture_arguments::QueryArguments::new(
        &fixture.input(),
        fixture.resident.input.packed.nodes.len(),
    )
    .unwrap();
    // Simulate a contradictory GPU producer after preparation, before validation.
    unsafe {
        (*fixture
            .captures
            .buffer
            .contents()
            .cast::<metal_batch_abi::FinalCaptureRecord>())
        .node = u32::MAX;
    }
    assert_eq!(
        fixture.evaluate(&arguments).err(),
        Some(Error::BackendFailure)
    );
    assert_eq!(
        metal_register_execution::read::<[u32; 4]>(&fixture.metadata, 1).unwrap(),
        vec![[0, 0, 0, 1]]
    );
}

#[test]
fn query_argument_views_reject_foreign_code_bindings_short_records_and_wrong_layouts() {
    let _guard = metal_backend::metal_execution_guard().unwrap();
    let scope = metal_backend::launch::LaunchScope::enter();
    let fixture = QueryFixture::new(2048);
    let arguments = metal_capture_arguments::QueryArguments::new(
        &fixture.input(),
        fixture.resident.input.packed.nodes.len(),
    )
    .unwrap();
    let counts = MappedBuffer::zeroed::<[u32; 4]>(&fixture.device, fixture.roots).unwrap();
    let work = MappedBuffer::zeroed::<u32>(&fixture.device, fixture.roots).unwrap();
    let failures = MappedBuffer::zeroed::<u32>(&fixture.device, 1).unwrap();
    let short_rows = MappedBuffer::zeroed::<[u32; 2]>(&fixture.device, fixture.roots).unwrap();
    let short_captures =
        MappedBuffer::zeroed::<[u32; 4]>(&fixture.device, fixture.roots * 2).unwrap();
    let auxiliary = [
        (&counts, fixture.roots, 16),
        (&work, fixture.roots, 4),
        (&failures, 1, 4),
    ];
    assert_eq!(
        arguments.input().validate_storage(
            fixture.resident.input.packed.nodes.len(),
            true,
            auxiliary
        ),
        Ok(())
    );
    for defect in 0..6 {
        let mut input = arguments.input();
        match defect {
            0 => input.specs = &fixture.owners,
            1 => input.spans = &short_rows,
            2 => input.captures = &short_captures,
            3 => input.metadata = &fixture.rows,
            4 => input.parameters.layout = metal_capture_arguments::ArgumentLayout::Compact,
            5 => input.parameters.abort_evaluation = 0,
            _ => unreachable!(),
        }
        assert_eq!(
            input.validate_storage(fixture.resident.input.packed.nodes.len(), true, auxiliary),
            Err(Error::BackendFailure),
            "query view defect {defect}"
        );
    }
    assert!(scope.snapshot().is_empty());
    assert_eq!(
        metal_register_execution::read::<[u32; 4]>(&fixture.metadata, fixture.roots).unwrap(),
        vec![[0, 0, 0, 1]; fixture.roots]
    );
}
