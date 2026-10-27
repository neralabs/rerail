//! Initial semantic states must come from the resident selectors on the GPU.
use super::metal_backend::launch::{LaunchScope, commit_checked, wait_checked};
use super::metal_backend::{MappedBuffer, metal_execution_guard, pipeline_named};
use super::metal_frontier_seed::{self, SeedInput};
use super::metal_grid::MetalGridInput;
use super::metal_grid_plan::{ExecutionPlan, ResidentInput};
use super::metal_parallel::encode;
use super::metal_state_values::{self, COLUMNS, StateValues, ValueFrontier};
use super::*;
use crate::policy_dsl::tape::{Grammar, TapeImage};
use ast_grep_core::tree_sitter::StrDoc;
use ast_grep_language::SupportLang;
use metal::Device;

fn fixture(lines: usize, empty: bool) -> (ResidentInput, ExecutionPlan, Vec<[u32; COLUMNS]>) {
    let make =
        |words| Program::new(words, vec![b"match".to_vec()], vec![], vec![0], vec![]).unwrap();
    let programs = [
        make(vec![word(EMIT, 0, 0)]),
        make(vec![
            word(NEXT, NAMED, 0),
            word(DESCEND, 0, 0),
            word(EMIT, 0, 0),
        ]),
        make(vec![
            word(FIRST, NAMED, 0),
            word(CHILD, NAMED, 0),
            word(EMIT, 0, 0),
        ]),
        make(vec![
            word(LAST, NAMED, 0),
            word(FOLLOWING, NAMED, 0),
            word(EMIT, 0, 0),
        ]),
    ];
    let mut docs = Vec::new();
    let mut roots = Vec::new();
    let mut expected = Vec::new();
    let mut ordinal = 0;
    let mut node_base = 0;
    for _ in 0..2 {
        let source = Arc::new("f(a)\ng(b)\n".repeat(lines));
        let grammar = Grammar::new(SupportLang::Python).unwrap();
        let native = StrDoc::new(source.as_str(), SupportLang::Python);
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
        let identifier = grammar.language().id_for_node_kind("identifier", true);
        let doc = TapeDoc::restore(source, grammar, &image, &|| false).unwrap();
        let selected = if empty {
            Vec::new()
        } else {
            (0..doc.transducer_len() as u32)
                .filter(|&node| doc.transducer_kind(node) == identifier)
                .collect::<Vec<_>>()
        };
        for program in 0..programs.len() {
            for &node in &selected {
                if program != 0 {
                    // Task, PC, node, sibling requirement, Program, continuation kind,
                    // empty typed spans and initial logical order.
                    expected.push([
                        ordinal,
                        0,
                        node_base + node,
                        program as u32,
                        program as u32,
                        2,
                        0,
                        0,
                        0,
                        0,
                        0,
                        0,
                        0,
                    ]);
                }
                ordinal += 1;
            }
        }
        node_base += doc.transducer_len() as u32;
        roots.push(vec![selected; programs.len()]);
        docs.push(doc);
    }
    let resident = ResidentInput::new(
        MetalGridInput::build(&programs, &docs, &roots).unwrap(),
        None,
    )
    .unwrap();
    let plan = ExecutionPlan::new(&resident, &programs).unwrap();
    (resident, plan, expected)
}

fn frontier(device: &Device, capacity: usize) -> ValueFrontier {
    ValueFrontier {
        columns: MappedBuffer::from_slice(device, &vec![u32::MAX; capacity * COLUMNS]).unwrap(),
        counts: MappedBuffer::from_slice(device, &[u32::MAX; 5]).unwrap(),
        capacity,
        generation: 0,
    }
}

#[test]
fn structural_seeds_preserve_exact_tasks_across_shared_selectors_and_files() {
    let device = Device::system_default().unwrap();
    let mut stage_counts = Vec::new();
    for (lines, empty) in [(1, true), (1, false), (100, false)] {
        let _guard = metal_execution_guard().unwrap();
        let (resident, plan, expected) = fixture(lines, empty);
        assert_eq!(expected.len(), if empty { 0 } else { lines * 24 });
        let data = super::metal_register_execution::upload(&device, &resident).unwrap();
        let roots =
            MappedBuffer::from_slice(&device, &vec![u32::MAX; resident.input.grid.roots.len()])
                .unwrap();
        let output = frontier(&device, expected.len() + 7);
        let reduced = frontier(&device, expected.len() + 7);
        let values = StateValues {
            cursors: MappedBuffer::zeroed::<u32>(&device, 0).unwrap(),
            repeats: MappedBuffer::zeroed::<u32>(&device, 0).unwrap(),
            captures: MappedBuffer::zeroed::<u32>(&device, 0).unwrap(),
            counts: MappedBuffer::from_slice(&device, &[0u32, 0]).unwrap(),
            cursor_capacity: 0,
            repeat_capacity: 0,
            capture_capacity: 0,
            generation: 0,
        };
        let consumed =
            MappedBuffer::from_slice(&device, &vec![u32::MAX; (expected.len() + 7) * COLUMNS])
                .unwrap();
        let root_pipeline =
            pipeline_named(include_str!("metal_resident.metal"), "register_roots").unwrap();
        let consumer_source = r#"
#include <metal_stdlib>
using namespace metal;
kernel void consume_seed_states(device const uint* states [[buffer(0)]],device const uint* counts [[buffer(1)]],device uint* output [[buffer(2)]],uint tid [[thread_position_in_grid]]) {
 if(tid>=counts[0]) return;
 for(uint c=0;c<13;c++) output[tid*13+c]=states[c*counts[0]+tid];
}
"#;
        let consumer = pipeline_named(consumer_source, "consume_seed_states").unwrap();
        let scope = LaunchScope::enter();
        let queue = device.new_command_queue();
        let command = queue.new_command_buffer();
        encode(
            command,
            &root_pipeline,
            "register_roots",
            &[&data.buffer, &roots.buffer],
            &[],
            resident.input.grid.roots.len().div_ceil(256).max(1),
            None,
        );
        let pending = metal_frontier_seed::encode(
            &device,
            command,
            SeedInput {
                resident: &resident,
                plan: &plan,
                data: &data,
                roots: &roots,
                spans: 0..resident.input.grid.semantic_span_order.len(),
            },
            &output,
        )
        .expect("the GPU must generate compact initial states for dynamic tuples");
        let reduction =
            metal_state_values::encode(&device, command, &output, &reduced, &values, 8).unwrap();
        encode(
            command,
            &consumer,
            "consume_seed_states",
            &[
                &reduced.columns.buffer,
                &reduced.counts.buffer,
                &consumed.buffer,
            ],
            &[],
            0,
            Some((&reduced.counts.buffer, 8)),
        );
        commit_checked(command).unwrap();
        wait_checked(command).unwrap();
        pending.check_completed().unwrap();
        reduction.check_completed().unwrap();
        for result in [&output, &reduced] {
            let count = unsafe {
                std::slice::from_raw_parts(result.counts.buffer.contents().cast::<u32>(), 5)
            };
            assert_eq!(
                count,
                &[
                    expected.len() as u32,
                    0,
                    expected.len().div_ceil(256) as u32,
                    1,
                    1
                ]
            );
            let columns = unsafe {
                std::slice::from_raw_parts(
                    result.columns.buffer.contents().cast::<u32>(),
                    result.capacity * COLUMNS,
                )
            };
            for (row, wanted) in expected.iter().enumerate() {
                for col in 0..COLUMNS {
                    assert_eq!(
                        columns[col * expected.len() + row],
                        wanted[col],
                        "lines={lines}, row={row}, column={col}"
                    );
                }
            }
            assert!(
                columns[expected.len() * COLUMNS..]
                    .iter()
                    .all(|&v| v == u32::MAX),
                "capacity tail is not active work"
            );
        }
        let actual = unsafe {
            std::slice::from_raw_parts(
                consumed.buffer.contents().cast::<[u32; COLUMNS]>(),
                expected.len() + 7,
            )
        };
        assert_eq!(&actual[..expected.len()], expected);
        assert!(
            actual[expected.len()..]
                .iter()
                .all(|row| *row == [u32::MAX; COLUMNS])
        );
        let trace = scope.snapshot();
        assert_eq!(
            trace
                .iter()
                .map(|r| r.command)
                .collect::<BTreeSet<_>>()
                .len(),
            1
        );
        assert!(
            trace
                .iter()
                .all(|r| r.encoded && r.submitted && r.completed)
        );
        assert!(
            trace.len() <= 25,
            "seed, reduction, and consumer must stay broad: {} launches",
            trace.len()
        );
        if !empty {
            stage_counts.push(
                trace
                    .iter()
                    .filter(|r| r.kernel.starts_with("seed_"))
                    .count(),
            );
        }
        eprintln!(
            "STRUCTURAL_SEED_EVIDENCE {}",
            serde_json::json!({"lines":lines,"empty":empty,"states":expected.len(),"dispatches":trace})
        );
    }
    assert_eq!(
        stage_counts[0], stage_counts[1],
        "100 times more roots must not schedule individual tuples"
    );
}

fn encode_roots(
    command: &metal::CommandBufferRef,
    data: &MappedBuffer,
    roots: &MappedBuffer,
    count: usize,
) {
    let pipeline = pipeline_named(include_str!("metal_resident.metal"), "register_roots").unwrap();
    encode(
        command,
        &pipeline,
        "register_roots",
        &[&data.buffer, &roots.buffer],
        &[],
        count.div_ceil(256).max(1),
        None,
    );
}

fn read_states(frontier: &ValueFrontier) -> Vec<[u32; COLUMNS]> {
    let counts =
        unsafe { std::slice::from_raw_parts(frontier.counts.buffer.contents().cast::<u32>(), 5) };
    let count = counts[0] as usize;
    assert!(count <= frontier.capacity);
    assert_eq!(counts, &[count as u32, 0, count.div_ceil(256) as u32, 1, 1]);
    let columns = unsafe {
        std::slice::from_raw_parts(
            frontier.columns.buffer.contents().cast::<u32>(),
            frontier.capacity * COLUMNS,
        )
    };
    assert!(
        columns[count * COLUMNS..]
            .iter()
            .all(|&value| value == u32::MAX)
    );
    (0..count)
        .map(|i| std::array::from_fn(|c| columns[c * count + i]))
        .collect()
}

#[test]
fn structural_seeds_preserve_disjoint_input_boundaries_and_shared_residency() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let (resident, plan, expected) = fixture(1, false);
    let data = super::metal_register_execution::upload(&device, &resident).unwrap();
    let roots = MappedBuffer::from_slice(&device, &vec![u32::MAX; resident.input.grid.roots.len()])
        .unwrap();
    let count = resident.input.grid.semantic_span_order.len();
    assert!(count >= 6);
    let boundaries = [0, 2, count - 1, count];
    let queue = device.new_command_queue();
    let scope = LaunchScope::enter();
    let mut actual = Vec::new();
    for (index, boundary) in boundaries.windows(2).enumerate() {
        let output = frontier(&device, expected.len() + 7);
        let command = queue.new_command_buffer();
        if index == 0 {
            encode_roots(command, &data, &roots, resident.input.grid.roots.len());
        }
        let pending = metal_frontier_seed::encode(
            &device,
            command,
            SeedInput {
                resident: &resident,
                plan: &plan,
                data: &data,
                roots: &roots,
                spans: boundary[0]..boundary[1],
            },
            &output,
        )
        .unwrap();
        commit_checked(command).unwrap();
        wait_checked(command).unwrap();
        pending.check_completed().unwrap();
        actual.extend(read_states(&output));
    }
    assert_eq!(actual, expected);
    let trace = scope.snapshot();
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
            .map(|r| r.command)
            .collect::<BTreeSet<_>>()
            .len(),
        3
    );
    let shared = trace[0].buffers[0];
    assert!(
        trace
            .iter()
            .filter(|r| matches!(r.kernel.as_str(), "seed_span_counts" | "seed_scatter"))
            .all(|r| r.buffers[0] == shared)
    );
    assert!(
        trace
            .iter()
            .all(|r| r.completed && r.submitted && r.encoded)
    );
}

#[test]
fn structural_seeds_refuse_capacity_without_publishing_usable_states() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let (resident, plan, expected) = fixture(1, false);
    let data = super::metal_register_execution::upload(&device, &resident).unwrap();
    let roots = MappedBuffer::zeroed::<u32>(&device, resident.input.grid.roots.len()).unwrap();
    let output = frontier(&device, expected.len() - 1);
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    encode_roots(command, &data, &roots, resident.input.grid.roots.len());
    let pending = metal_frontier_seed::encode(
        &device,
        command,
        SeedInput {
            resident: &resident,
            plan: &plan,
            data: &data,
            roots: &roots,
            spans: 0..resident.input.grid.semantic_span_order.len(),
        },
        &output,
    )
    .unwrap();
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    assert_eq!(pending.check_completed(), Err(Error::BackendCapacity));
    assert_eq!(read_states(&output), Vec::<[u32; COLUMNS]>::new());
}

#[test]
fn structural_seeds_reject_invalid_generation_aliases_and_input_boundaries() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let (resident, plan, expected) = fixture(1, false);
    let data = super::metal_register_execution::upload(&device, &resident).unwrap();
    let roots = MappedBuffer::zeroed::<u32>(&device, resident.input.grid.roots.len()).unwrap();
    let end = resident.input.grid.semantic_span_order.len();
    let queue = device.new_command_queue();
    let scope = LaunchScope::enter();
    for case in 0..4 {
        let mut output = frontier(&device, expected.len() + 7);
        if case == 1 {
            output.generation = 1;
        }
        if case == 3 {
            output.capacity = u32::MAX as usize;
        }
        let command = queue.new_command_buffer();
        let result = metal_frontier_seed::encode(
            &device,
            command,
            SeedInput {
                resident: &resident,
                plan: &plan,
                data: &data,
                roots: if case == 2 { &output.columns } else { &roots },
                spans: if case == 0 { end + 1..end + 1 } else { 0..end },
            },
            &output,
        );
        assert_eq!(
            result.err(),
            Some(if case == 0 {
                Error::InvalidAnchor
            } else {
                Error::BackendFailure
            }),
            "case {case}"
        );
    }
    assert!(scope.snapshot().is_empty());
}

#[test]
fn structural_seeds_require_caller_owned_command_completion() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let (resident, plan, expected) = fixture(1, false);
    let data = super::metal_register_execution::upload(&device, &resident).unwrap();
    let roots = MappedBuffer::zeroed::<u32>(&device, resident.input.grid.roots.len()).unwrap();
    let output = frontier(&device, expected.len());
    let queue = device.new_command_queue();
    let scope = LaunchScope::enter();
    let command = queue.new_command_buffer();
    let pending = metal_frontier_seed::encode(
        &device,
        command,
        SeedInput {
            resident: &resident,
            plan: &plan,
            data: &data,
            roots: &roots,
            spans: 0..resident.input.grid.semantic_span_order.len(),
        },
        &output,
    )
    .unwrap();
    assert_eq!(pending.check_completed(), Err(Error::BackendFailure));
    let trace = scope.snapshot();
    assert!(!trace.is_empty());
    assert!(
        trace
            .iter()
            .all(|r| r.encoded && !r.submitted && !r.completed)
    );
}

#[test]
fn seeded_admission_preserves_ordinary_and_nonrepeating_query_occurrences() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let source = Arc::new("# one\n".to_owned());
    let native = StrDoc::new(source.as_str(), SupportLang::Python);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
    let doc = TapeDoc::restore(source, grammar, &image, &|| false).unwrap();
    let mut programs = Vec::new();
    for (query, repeat) in [(false, false), (false, true), (true, false), (true, true)] {
        let mut words = Vec::new();
        if repeat {
            words.push(word(REPEAT_BEGIN, 0, 0));
        }
        words.extend([word(CHILD, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)]);
        let mut program =
            Program::new(words, vec![b"match".to_vec()], vec![], vec![0], vec![]).unwrap();
        program.tree_sitter_query = query;
        program.validate().unwrap();
        programs.push(program);
    }
    let resident = ResidentInput::new(
        MetalGridInput::build(&programs, &[doc], &[vec![vec![0]; 4]]).unwrap(),
        None,
    )
    .unwrap();
    let plan = ExecutionPlan::new(&resident, &programs).unwrap();
    let data = super::metal_register_execution::upload(&device, &resident).unwrap();
    let roots = MappedBuffer::from_slice(&device, &vec![u32::MAX; resident.input.grid.roots.len()])
        .unwrap();
    let seeds = frontier(&device, 11);
    let duplicated = frontier(&device, 15);
    let reduced = frontier(&device, 15);
    let values = StateValues {
        cursors: MappedBuffer::zeroed::<u32>(&device, 0).unwrap(),
        repeats: MappedBuffer::zeroed::<u32>(&device, 0).unwrap(),
        captures: MappedBuffer::zeroed::<u32>(&device, 0).unwrap(),
        counts: MappedBuffer::from_slice(&device, &[0u32, 0]).unwrap(),
        cursor_capacity: 0,
        repeat_capacity: 0,
        capture_capacity: 0,
        generation: 0,
    };
    // Two ordered successors with equal values, using the actual GPU seed kinds.
    let duplicate=pipeline_named(r#"
#include <metal_stdlib>
using namespace metal;
kernel void duplicate_seeds(device const uint* src [[buffer(0)]],device const uint* counts [[buffer(1)]],device uint* dst [[buffer(2)]],device uint* out [[buffer(3)]],uint tid [[thread_position_in_grid]]) {
 uint n=counts[0];
 if(tid==0u) { out[0]=2u*n;out[1]=0u;out[2]=(2u*n+255u)/256u;out[3]=out[4]=1u; }
 if(tid>=n) return;
 for(uint c=0u;c<13u;c++) for(uint k=0u;k<2u;k++) dst[c*2u*n+2u*tid+k]=c==12u?k:src[c*n+tid];
}
"#,"duplicate_seeds").unwrap();
    let root_pipeline =
        pipeline_named(include_str!("metal_resident.metal"), "register_roots").unwrap();
    let scope = LaunchScope::enter();
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    encode(
        command,
        &root_pipeline,
        "register_roots",
        &[&data.buffer, &roots.buffer],
        &[],
        resident.input.grid.roots.len().div_ceil(256).max(1),
        None,
    );
    let seed = metal_frontier_seed::encode(
        &device,
        command,
        SeedInput {
            resident: &resident,
            plan: &plan,
            data: &data,
            roots: &roots,
            spans: 0..resident.input.grid.semantic_span_order.len(),
        },
        &seeds,
    )
    .unwrap();
    encode(
        command,
        &duplicate,
        "duplicate_seeds",
        &[
            &seeds.columns.buffer,
            &seeds.counts.buffer,
            &duplicated.columns.buffer,
            &duplicated.counts.buffer,
        ],
        &[],
        0,
        Some((&seeds.counts.buffer, 8)),
    );
    let reduction =
        metal_state_values::encode(&device, command, &duplicated, &reduced, &values, 0).unwrap();
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    seed.check_completed().unwrap();
    reduction.check_completed().unwrap();
    let counts =
        unsafe { std::slice::from_raw_parts(reduced.counts.buffer.contents().cast::<u32>(), 5) };
    assert_eq!(counts, [7, 0, 1, 1, 1]);
    let columns = unsafe {
        std::slice::from_raw_parts(
            reduced.columns.buffer.contents().cast::<u32>(),
            15 * COLUMNS,
        )
    };
    let expected = [
        (0, 2, 0),
        (0, 2, 1),
        (1, 2, 0),
        (1, 2, 1),
        (2, 2, 0),
        (2, 2, 1),
        (3, 0, 0),
    ];
    for (index, (task, kind, ordinal)) in expected.into_iter().enumerate() {
        let row = [task, 0, 0, 0, task, kind, 0, 0, 0, 0, 0, 0, ordinal];
        for column in 0..COLUMNS {
            assert_eq!(
                columns[column * 7 + index],
                row[column],
                "row={index} column={column}"
            );
        }
    }
    assert!(columns[7 * COLUMNS..].iter().all(|&v| v == u32::MAX));
    assert!(
        scope
            .snapshot()
            .iter()
            .all(|d| d.encoded && d.submitted && d.completed)
    );
}

#[test]
fn gpu_seed_counts_drive_ready_lists_and_an_indirect_successor_consumer() {
    use super::metal_ready_batch::{self, ReadyBatch, ReadyInput};
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let (resident, plan, expected) = fixture(100, false);
    let data = super::metal_register_execution::upload(&device, &resident).unwrap();
    let roots = MappedBuffer::zeroed::<u32>(&device, resident.input.grid.roots.len()).unwrap();
    let states = frontier(&device, expected.len() + 7);
    let values = StateValues {
        cursors: MappedBuffer::zeroed::<u32>(&device, 0).unwrap(),
        repeats: MappedBuffer::zeroed::<u32>(&device, 0).unwrap(),
        captures: MappedBuffer::zeroed::<u32>(&device, 0).unwrap(),
        counts: MappedBuffer::zeroed::<u32>(&device, 2).unwrap(),
        cursor_capacity: 0,
        repeat_capacity: 0,
        capture_capacity: 0,
        generation: 0,
    };
    let batch = ReadyBatch {
        indices: MappedBuffer::from_slice(&device, &vec![u32::MAX; states.capacity * 4]).unwrap(),
        counts: MappedBuffer::from_slice(&device, &[u32::MAX; 17]).unwrap(),
        capacity: states.capacity,
        generation: 0,
    };
    let consumed =
        MappedBuffer::from_slice(&device, &vec![[u32::MAX; COLUMNS]; states.capacity]).unwrap();
    let pipeline=pipeline_named(r#"
#include <metal_stdlib>
using namespace metal;
kernel void consume_ready(device const uint* states [[buffer(0)]],device const uint* header [[buffer(1)]],device const uint* ids [[buffer(2)]],device const uint* counts [[buffer(3)]],device uint* output [[buffer(4)]],uint i [[thread_position_in_grid]]) {
 if(i>=counts[0]) return;
 for(uint c=0;c<13;c++) output[i*13+c]=states[c*header[0]+ids[i]];
}
"#,"consume_ready").unwrap();
    let scope = LaunchScope::enter();
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    encode_roots(command, &data, &roots, resident.input.grid.roots.len());
    let seeds = metal_frontier_seed::encode(
        &device,
        command,
        SeedInput {
            resident: &resident,
            plan: &plan,
            data: &data,
            roots: &roots,
            spans: 0..resident.input.grid.semantic_span_order.len(),
        },
        &states,
    )
    .unwrap();
    let ready = metal_ready_batch::encode(
        &device,
        command,
        ReadyInput {
            resident: &resident,
            plan: &plan,
            data: &data,
            states: &states,
            values: &values,
        },
        &batch,
    )
    .unwrap();
    encode(
        command,
        &pipeline,
        "consume_ready",
        &[
            &states.columns.buffer,
            &states.counts.buffer,
            &batch.indices.buffer,
            &batch.counts.buffer,
            &consumed.buffer,
        ],
        &[],
        0,
        Some((&batch.counts.buffer, 16)),
    );
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    seeds.check_completed().unwrap();
    ready.check_completed().unwrap();
    let actual = unsafe {
        std::slice::from_raw_parts(
            consumed.buffer.contents().cast::<[u32; COLUMNS]>(),
            states.capacity,
        )
    };
    assert_eq!(&actual[..expected.len()], &expected);
    assert!(
        actual[expected.len()..]
            .iter()
            .all(|r| *r == [u32::MAX; COLUMNS])
    );
    let counts =
        unsafe { std::slice::from_raw_parts(batch.counts.buffer.contents().cast::<u32>(), 16) };
    assert_eq!(&counts[..4], &[expected.len() as u32, 0, 0, 0]);
    let trace = scope.snapshot();
    assert!(trace.iter().all(|r| r.completed));
    assert_eq!(
        trace
            .iter()
            .map(|r| r.command)
            .collect::<BTreeSet<_>>()
            .len(),
        1
    );
    assert!(trace.last().unwrap().indirect);
    eprintln!(
        "SEED_READY_EVIDENCE {}",
        serde_json::json!({"states":expected.len(),"launches":trace.len(),"peakMetalBytes":trace.iter().map(|r|r.peak_application_buffer_bytes).max().unwrap()})
    );
}
