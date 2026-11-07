//! Completed relations retain every value across physical formats and late refusals.
use super::super::*;
use super::*;

const ROOTS: u32 = 8193;
const CAPTURES: u32 = 128;

fn fixture() -> (ResidentInput, Vec<Vec<Output>>) {
    let file =
        metal_register_conformance::source_file("\"text/JavaScript\"\n".repeat(ROOTS as usize));
    let doc = file.restore(&|| false).unwrap();
    let mut words = vec![word(CAPTURE, 0, 0); CAPTURES as usize];
    words.push(word(EMIT, 0, 1));
    let program = Program::new(
        words,
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
        Directive::SetLanguageFromMimeType { capture: 0 },
    ])
    .unwrap();
    let programs = [program.clone(), program];
    let roots = (0..doc.transducer_len() as u32)
        .filter(|&node| doc.transducer_text(node) == b"text/JavaScript")
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), ROOTS as usize);
    let anchors = [vec![roots.clone(), vec![roots[0]]]];
    let mut expected =
        execute_batch(&programs, std::slice::from_ref(&doc), Some(&anchors)).unwrap();
    assert_eq!(
        expected[0].iter().map(Vec::len).collect::<Vec<_>>(),
        [ROOTS as usize, 1]
    );
    assert_eq!(expected[0][1][0].task_ordinal, ROOTS);
    assert_eq!(
        expected[0][0][0].captures[0].text.as_deref(),
        Some(b"text/javascript".as_slice())
    );
    assert_eq!(
        expected[0][0][0].dynamic_properties,
        vec![(b"language".to_vec(), Some(b"JavaScript".to_vec()))]
    );
    let resident = ResidentInput::new(
        metal_grid::MetalGridInput::build(&programs, std::slice::from_ref(&doc), &anchors).unwrap(),
        Some(&[2, 1]),
    )
    .unwrap();
    (resident, expected.remove(0))
}

// Serialize the independent interpreter's complete values into the typed ABI.
// No production ordering, publication, or column encoder supplies the fixture.
fn arenas(device: &Device, outputs: &[Output]) -> OutputArenas {
    let mut rows = Vec::new();
    let mut captures = Vec::new();
    let mut properties = Vec::new();
    let mut bytes = Vec::new();
    let mut layout = Vec::new();
    for (owner, output) in outputs.iter().enumerate() {
        let prop_start = properties.len() as u32;
        let byte_start = bytes.len() as u32;
        let capture_start = captures.len() as u32;
        for &(key, value) in &output.properties {
            properties.push(PropertyRecord {
                kind: 0,
                owner: owner as u32,
                key: u32::from(key),
                value: value.map_or(u32::MAX, u32::from),
                ..Default::default()
            });
        }
        for capture in &output.captures {
            let cap_owner = captures.len() as u32;
            let cap_properties = properties.len() as u32;
            for &(key, value) in &capture.properties {
                properties.push(PropertyRecord {
                    kind: 1,
                    owner: cap_owner,
                    key: u32::from(key),
                    value: value.map_or(u32::MAX, u32::from),
                    ..Default::default()
                });
            }
            let (text_start, text_length) = if let Some(text) = &capture.text {
                let first = bytes.len() as u32;
                bytes.extend_from_slice(text);
                (first, text.len() as u32)
            } else {
                (u32::MAX, 0)
            };
            captures.push(FinalCaptureRecord {
                node: capture.node,
                label: u32::from(capture.label),
                start: capture.span.start as u32,
                end: capture.span.end as u32,
                text_start,
                text_length,
                property_start: if capture.properties.is_empty() {
                    0
                } else {
                    cap_properties
                },
                property_count: capture.properties.len() as u32,
                retained: 1,
            });
        }
        let dynamic_start = properties.len() as u32;
        for (key, value) in &output.dynamic_properties {
            let key_start = bytes.len() as u32;
            bytes.extend_from_slice(key);
            let value_start = bytes.len() as u32;
            if let Some(value) = value {
                bytes.extend_from_slice(value);
            }
            properties.push(PropertyRecord {
                kind: 2,
                owner: owner as u32,
                key: 0,
                value: u32::from(value.is_some()),
                key_start,
                key_length: key.len() as u32,
                value_start: if value.is_some() { value_start } else { 0 },
                value_length: value.as_ref().map_or(0, |value| value.len() as u32),
            });
        }
        rows.push(ResultRecord {
            task_ordinal: output.task_ordinal,
            local_ordinal: output.local_ordinal,
            start: output.start,
            label: u32::from(output.label),
            capture_start,
            capture_count: output.captures.len() as u32,
            property_start: if output.properties.is_empty() {
                0
            } else {
                prop_start
            },
            property_count: output.properties.len() as u32,
            dynamic_start: if output.dynamic_properties.is_empty() {
                0
            } else {
                dynamic_start
            },
            dynamic_count: output.dynamic_properties.len() as u32,
        });
        layout.push([
            properties.len() as u32 - prop_start,
            bytes.len() as u32 - byte_start,
            prop_start,
            byte_start,
        ]);
    }
    OutputArenas {
        rows: MappedBuffer::from_slice(device, &rows).unwrap(),
        captures: MappedBuffer::from_slice(device, &captures).unwrap(),
        properties: MappedBuffer::from_slice(device, &properties).unwrap(),
        bytes: MappedBuffer::from_slice(device, &bytes).unwrap(),
        layout: Some(MappedBuffer::from_slice(device, &layout).unwrap()),
        counts: [
            rows.len() as u32,
            captures.len() as u32,
            properties.len() as u32,
            bytes.len() as u32,
        ],
    }
}

fn inputs(
    device: &Device,
    queue: &metal::CommandQueueRef,
    expected: &[Vec<Output>],
) -> [TypedCheckpoint; 2] {
    let inputs = [
        TypedCheckpoint::new(device, queue, arenas(device, &expected[0])).unwrap(),
        TypedCheckpoint::new(device, queue, arenas(device, &expected[1])).unwrap(),
    ];
    assert!(matches!(
        &inputs[0].arenas,
        columns::SavedArenas::Encoded { .. }
    ));
    assert!(matches!(
        &inputs[1].arenas,
        columns::SavedArenas::Literal(_)
    ));
    inputs
}

#[test]
fn mixed_completed_column_formats_preserve_every_value_global_limits_and_late_failures() {
    let (resident, expected) = fixture();
    let _guard = metal_backend::metal_execution_guard().unwrap();
    let scope = metal_backend::launch::LaunchScope::enter();
    let device = Device::system_default().unwrap();
    let queue = device.new_command_queue();
    let data = metal_register_execution::upload(&device, &resident).unwrap();
    for (reverse, failed) in [(false, false), (true, false), (false, true)] {
        let mut inputs = inputs(&device, &queue, &expected);
        if reverse {
            inputs.swap(0, 1);
        }
        let errors = vec![failed.then_some(Error::WorkLimit), None];
        let merged =
            TypedCheckpoint::merge(&device, &queue, &resident, &data, inputs, &errors, true)
                .unwrap();
        let actual = SavedOutput::new(
            &resident,
            device.clone(),
            queue.clone(),
            MappedBuffer {
                buffer: data.buffer.clone(),
            },
            merged,
            errors.clone(),
        )
        .unwrap()
        .execute()
        .unwrap()
        .into_outcome()
        .unwrap();
        assert_eq!(actual.errors, vec![errors]);
        assert_eq!(actual.outputs.len(), 1);
        assert_eq!(actual.outputs[0].len(), 2);
        for (program, expected) in expected.iter().enumerate() {
            let count = if program == 0 {
                if failed { 0 } else { 2 }
            } else {
                1
            };
            assert_eq!(actual.outputs[0][program], expected[..count]);
        }
    }
    assert!(
        scope
            .snapshot()
            .iter()
            .all(|r| r.encoded && r.submitted && r.completed)
    );
}

#[test]
fn inconsistent_completed_column_metadata_and_corrupted_blocks_reject_without_format_fallback() {
    let (resident, expected) = fixture();
    let _guard = metal_backend::metal_execution_guard().unwrap();
    let scope = metal_backend::launch::LaunchScope::enter();
    let device = Device::system_default().unwrap();
    let queue = device.new_command_queue();
    let data = metal_register_execution::upload(&device, &resident).unwrap();
    for case in 0..5 {
        let mut inputs = inputs(&device, &queue, &expected);
        let columns::SavedArenas::Encoded {
            headers,
            payload,
            typed,
            words,
            groups,
            ..
        } = &mut inputs[0].arenas
        else {
            unreachable!()
        };
        match case {
            0 => *groups += 1,
            1 => *typed = false,
            2 => {
                let short = MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
                assert!(u64::from(*words) * 4 > short.buffer.length());
                *payload = short;
            }
            3 | 4 => {
                let group = if case == 3 {
                    0
                } else {
                    ROOTS.div_ceil(256) as usize * 10
                };
                assert!(group < *groups as usize);
                unsafe {
                    headers
                        .buffer
                        .contents()
                        .cast::<u32>()
                        .add(group * 2 + 1)
                        .write(0x8000_001f);
                }
            }
            _ => unreachable!(),
        }
        let before = scope.snapshot().len();
        assert_eq!(
            TypedCheckpoint::merge(
                &device,
                &queue,
                &resident,
                &data,
                inputs,
                &[None, None],
                true
            )
            .err(),
            Some(Error::BackendFailure),
            "case={case}"
        );
        let trace = scope.snapshot();
        let trace = &trace[before..];
        if case < 3 {
            assert!(
                trace.is_empty(),
                "metadata must reject before device access"
            );
        } else {
            assert!(
                !trace.is_empty(),
                "corrupted block must reach the checked device reader"
            );
        }
        assert!(
            trace
                .iter()
                .all(|r| r.encoded && r.submitted && r.completed)
        );
    }
}
