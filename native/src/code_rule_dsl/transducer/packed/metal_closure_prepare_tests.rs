//! Resident navigation and serial first-visit oracle, independent of GPU packing.
use super::metal_backend::launch::{LaunchScope, commit_checked, wait_checked};
use super::metal_backend::{MappedBuffer, metal_execution_guard, pipeline_named};
use super::metal_bulk_closure::{self, ClosureExits, ClosureRanges};
use super::metal_closure_prepare::{self, ClosureInput, PreparedRanges};
use super::metal_grid::MetalGridInput;
use super::metal_grid_plan::ResidentInput;
use super::metal_parallel;
use super::metal_state_values::{COLUMNS, StateValues, ValueFrontier};
use super::*;
use crate::code_rule_dsl::tape::{Grammar, TapeImage};
use ast_grep_core::tree_sitter::StrDoc;
use crate::code_rule_dsl::language::GrammarDialect as SupportLang;
use metal::Device;

fn program(guards: &[u32], longs: Vec<u64>) -> Program {
    let mut words = vec![
        word(REPEAT_BEGIN, 0, 0),
        word(REPEAT_BUMP, 0, 0),
        word(FORK, GREEDY, (6 + guards.len()) as u16),
        word(NEXT, 0, 0),
    ];
    words.extend_from_slice(guards);
    words.extend([
        word(REPEAT_BUMP, 0, 0),
        word(GOTO, 0, 2),
        word(REPEAT_END, 0, 0),
        word(EMIT, 0, 0),
    ]);
    Program::new(words, vec![b"match".to_vec()], vec![], vec![0], longs)
        .unwrap()
        .with_tree_sitter_query_semantics()
        .unwrap()
}
fn document(text: &str) -> TapeDoc {
    let source = Arc::new(text.to_owned());
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let native = StrDoc::new(text, SupportLang::Python);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
    TapeDoc::restore(source, grammar, &image, &|| false).unwrap()
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct State {
    task: u32,
    program: u32,
    node: u32,
    requirement: u32,
    cursors: Vec<[u32; 4]>,
    repeats: Vec<[u32; 4]>,
    captures: Vec<[u32; 4]>,
    ordinal: u32,
}
fn input(device: &Device, states: &[State]) -> (ValueFrontier, StateValues) {
    let n = states.len();
    let mut columns = vec![u32::MAX; (n + 7) * COLUMNS];
    let mut cursors = Vec::new();
    let mut repeats = Vec::new();
    let mut captures = Vec::new();
    for (i, s) in states.iter().enumerate() {
        for (c, v) in [s.task, 2, s.node, s.requirement, s.program, 0]
            .into_iter()
            .enumerate()
        {
            columns[c * n + i] = v;
        }
        for (c, arena, values) in [
            (6, &mut cursors, &s.cursors),
            (8, &mut repeats, &s.repeats),
            (10, &mut captures, &s.captures),
        ] {
            columns[c * n + i] = arena.len() as u32;
            columns[(c + 1) * n + i] = values.len() as u32;
            arena.extend_from_slice(values);
        }
        columns[12 * n + i] = s.ordinal;
    }
    let lengths = [cursors.len() as u32, repeats.len() as u32];
    let cap = captures.len() as u32;
    for arena in [&mut cursors, &mut repeats, &mut captures] {
        arena.extend([[u32::MAX; 4]; 7]);
    }
    (
        ValueFrontier {
            columns: MappedBuffer::from_slice(device, &columns).unwrap(),
            counts: MappedBuffer::from_slice(
                device,
                &[n as u32, cap, n.div_ceil(256) as u32, 1, 1],
            )
            .unwrap(),
            capacity: n + 7,
            generation: 11,
        },
        StateValues {
            cursors: MappedBuffer::from_slice(device, &cursors).unwrap(),
            repeats: MappedBuffer::from_slice(device, &repeats).unwrap(),
            captures: MappedBuffer::from_slice(device, &captures).unwrap(),
            counts: MappedBuffer::from_slice(device, &lengths).unwrap(),
            cursor_capacity: cursors.len(),
            repeat_capacity: repeats.len(),
            capture_capacity: captures.len(),
            generation: 11,
        },
    )
}
fn output(device: &Device, entries: usize, cells: usize) -> PreparedRanges {
    PreparedRanges {
        ranges: ClosureRanges {
            domains: MappedBuffer::from_slice(device, &vec![[u32::MAX; 2]; entries]).unwrap(),
            entries: MappedBuffer::from_slice(device, &vec![[u32::MAX; 2]; entries]).unwrap(),
            allowed: MappedBuffer::from_slice(device, &vec![u32::MAX; cells]).unwrap(),
            counts: MappedBuffer::from_slice(device, &[u32::MAX; 3]).unwrap(),
            domain_capacity: entries,
            entry_capacity: entries,
            cell_capacity: cells,
            generation: 11,
        },
        nodes: MappedBuffer::from_slice(device, &vec![u32::MAX; cells]).unwrap(),
    }
}
fn permits(resident: &ResidentInput, programs: &[Program], program: u32, node: u32) -> bool {
    let n = resident.input.packed.nodes[node as usize];
    let pg = &programs[program as usize];
    let proof = closure_proof::regions(pg).unwrap()[0];
    proof.guards.iter().all(|&w| {
        let (op, mode, arg) = split(w);
        match op {
            KIND => n.kind == u32::from(arg),
            FIELD => n.field == u32::from(arg),
            FLAGS => n.flags & u32::from(mode) == u32::from(mode) && n.flags & u32::from(arg) == 0,
            SIZE => {
                let size = u64::from(n.source_end - n.source_start);
                let expected = if mode & SIZE_LONG == 0 {
                    u64::from(arg)
                } else {
                    pg.longs[arg as usize]
                };
                match mode & !SIZE_LONG {
                    0 => size == expected,
                    1 => size >= expected,
                    2 => size <= expected,
                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        }
    })
}
fn oracle(resident: &ResidentInput, programs: &[Program], states: &[State]) -> Vec<[u32; 3]> {
    let batch = &resident.input.packed;
    let mut visited = BTreeSet::new();
    let mut out = Vec::new();
    for (i, s) in states.iter().enumerate() {
        let mut state = s.clone();
        state.ordinal = 0;
        if visited.contains(&state) {
            continue;
        }
        loop {
            visited.insert(state.clone());
            let next = batch.nodes[state.node as usize].next_any;
            if next == u32::MAX {
                break;
            }
            let allowed = permits(resident, programs, s.program, next);
            let mut successor = state.clone();
            successor.node = next;
            if !allowed || visited.contains(&successor) {
                break;
            }
            state = successor;
        }
        out.push([i as u32, state.node, s.ordinal]);
    }
    out
}
const IO: &str = r#"
#include <metal_stdlib>
using namespace metal;
kernel void prepare_test_produce(device uint* output [[buffer(0)]],constant uint4& counts [[buffer(1)]],uint tid [[thread_position_in_grid]]) { if(tid==0u) {output[0]=counts.x;output[1]=counts.y;output[2]=(counts.x+255u)/256u;output[3]=1u;output[4]=1u;} }
kernel void prepare_test_consume(device const uint2* exits [[buffer(0)]],device const uint* counts [[buffer(1)]],device const uint* nodes [[buffer(2)]],device const uint* states [[buffer(3)]],device const uint* state_counts [[buffer(4)]],device uint* out [[buffer(5)]],uint tid [[thread_position_in_grid]]) {
 if(tid<counts[0]) {uint2 e=exits[tid];out[tid*3]=e.x;out[tid*3+1]=nodes[e.y];out[tid*3+2]=states[12u*state_counts[0]+e.x];}
}
"#;
fn check(resident: &ResidentInput, programs: &[Program], states: &[State], cells: usize) -> usize {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let scope = LaunchScope::enter();
    let data = super::metal_register_execution::upload(&device, resident).unwrap();
    let (mut frontier, values) = input(&device, states);
    let cap = states.iter().map(|s| s.captures.len() as u32).sum::<u32>();
    frontier.counts = MappedBuffer::from_slice(&device, &[u32::MAX; 5]).unwrap();
    let prepared = output(&device, frontier.capacity, cells + 7);
    let exits = ClosureExits {
        exits: MappedBuffer::from_slice(&device, &vec![[u32::MAX; 2]; frontier.capacity]).unwrap(),
        counts: MappedBuffer::from_slice(&device, &[u32::MAX; 4]).unwrap(),
        capacity: frontier.capacity,
        generation: 11,
    };
    let consumed =
        MappedBuffer::from_slice(&device, &vec![[u32::MAX; 3]; frontier.capacity]).unwrap();
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    metal_parallel::encode(
        command,
        &pipeline_named(IO, "prepare_test_produce").unwrap(),
        "prepare_test_produce",
        &[&frontier.counts.buffer],
        &[states.len() as u32, cap, 0, 0],
        1,
        None,
    );
    let pending = metal_closure_prepare::encode(
        &device,
        command,
        ClosureInput {
            resident,
            programs,
            data: &data,
            states: &frontier,
            values: &values,
        },
        &prepared,
    )
    .expect("ready header states must produce exact resident closure domains on GPU");
    let closure = metal_bulk_closure::encode(&device, command, &prepared.ranges, &exits).unwrap();
    metal_parallel::encode(
        command,
        &pipeline_named(IO, "prepare_test_consume").unwrap(),
        "prepare_test_consume",
        &[
            &exits.exits.buffer,
            &exits.counts.buffer,
            &prepared.nodes.buffer,
            &frontier.columns.buffer,
            &frontier.counts.buffer,
            &consumed.buffer,
        ],
        &[],
        0,
        Some((&exits.counts.buffer, 4)),
    );
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    pending.check_completed().unwrap();
    closure.check_completed().unwrap();
    let expected = oracle(resident, programs, states);
    let header =
        unsafe { std::slice::from_raw_parts(exits.counts.buffer.contents().cast::<u32>(), 4) };
    assert_eq!(
        header,
        [
            expected.len() as u32,
            expected.len().div_ceil(256) as u32,
            1,
            1
        ]
    );
    let actual = unsafe {
        std::slice::from_raw_parts(
            consumed.buffer.contents().cast::<[u32; 3]>(),
            frontier.capacity,
        )
    };
    assert_eq!(&actual[..expected.len()], expected);
    assert!(actual[expected.len()..].iter().all(|v| *v == [u32::MAX; 3]));
    let lengths = unsafe {
        std::slice::from_raw_parts(prepared.ranges.counts.buffer.contents().cast::<u32>(), 3)
    };
    assert_eq!(lengths[1], states.len() as u32);
    assert_eq!(lengths[2], cells as u32);
    // Verify the complete semantic ranges, not only the exits. Domain IDs may
    // be assigned differently; equality and membership must still be exact.
    let domains = unsafe {
        std::slice::from_raw_parts(
            prepared.ranges.domains.buffer.contents().cast::<[u32; 2]>(),
            prepared.ranges.domain_capacity,
        )
    };
    let entries = unsafe {
        std::slice::from_raw_parts(
            prepared.ranges.entries.buffer.contents().cast::<[u32; 2]>(),
            prepared.ranges.entry_capacity,
        )
    };
    let nodes = unsafe {
        std::slice::from_raw_parts(
            prepared.nodes.buffer.contents().cast::<u32>(),
            prepared.ranges.cell_capacity,
        )
    };
    let allowed = unsafe {
        std::slice::from_raw_parts(
            prepared.ranges.allowed.buffer.contents().cast::<u32>(),
            prepared.ranges.cell_capacity,
        )
    };
    let batch = &resident.input.packed;
    let mut memberships = BTreeMap::new();
    let mut used = BTreeSet::new();
    for (index, state) in states.iter().enumerate() {
        let parent = batch.nodes[state.node as usize].parent;
        let siblings = if parent == u32::MAX {
            vec![state.node]
        } else {
            let p = batch.nodes[parent as usize];
            batch.children[p.child_start as usize..(p.child_start + p.child_count) as usize]
                .to_vec()
        };
        let [domain, offset] = entries[index];
        assert!(domain < lengths[0]);
        let [first, count] = domains[domain as usize];
        assert_eq!(count as usize, siblings.len());
        assert!(offset < count);
        assert_eq!(nodes[(first + offset) as usize], state.node);
        let mut key = state.clone();
        key.node = siblings[0];
        key.ordinal = 0;
        if let Some(previous) = memberships.insert(key, domain) {
            assert_eq!(previous, domain);
        } else {
            assert!(used.insert(domain));
        }
        assert_eq!(&nodes[first as usize..(first + count) as usize], siblings);
        assert_eq!(
            &allowed[first as usize..(first + count) as usize],
            siblings
                .iter()
                .map(|&node| u32::from(permits(resident, programs, state.program, node)))
                .collect::<Vec<_>>()
        );
    }
    assert_eq!(memberships.len(), lengths[0] as usize);
    assert!(
        domains[lengths[0] as usize..]
            .iter()
            .all(|&v| v == [u32::MAX; 2])
    );
    assert!(entries[states.len()..].iter().all(|&v| v == [u32::MAX; 2]));
    assert!(nodes[cells..].iter().all(|&v| v == u32::MAX));
    assert!(allowed[cells..].iter().all(|&v| v == u32::MAX));
    let trace = scope.snapshot();
    assert!(
        trace
            .iter()
            .all(|d| d.encoded && d.submitted && d.completed)
    );
    assert!(trace.len() < 100);
    eprintln!(
        "CLOSURE_PREPARE {}",
        serde_json::json!({"states":states.len(),"domains":lengths[0],"cells":cells,"exits":expected.len(),"launches":trace.len(),"peakMetalBytes":trace.iter().map(|d|d.peak_application_buffer_bytes).max().unwrap()})
    );
    trace.len()
}
fn fixture(groups: usize) -> (ResidentInput, Vec<Program>, Vec<State>, usize) {
    let docs = vec![document("x\n# a\n# bb\n# ccc\n"), document("# other\n")];
    let programs = vec![program(&[], vec![])];
    let roots = docs.iter().map(|_| vec![vec![0]]).collect::<Vec<_>>();
    let resident = ResidentInput::new(
        MetalGridInput::build(&programs, &docs, &roots).unwrap(),
        None,
    )
    .unwrap();
    let siblings = docs[0].transducer_children(0).collect::<Vec<_>>();
    let mut states = Vec::new();
    for group in 0..groups {
        for offset in [2, 0, 2] {
            states.push(State {
                task: 0,
                program: 0,
                node: siblings[offset],
                requirement: 0,
                cursors: vec![[group as u32, 0, 0, 0]],
                repeats: vec![[0, 0, 0, 0]],
                captures: vec![],
                ordinal: (states.len() * 10 + 7) as u32,
            });
        }
    }
    (resident, programs, states, groups * 4)
}
#[test]
fn closure_preparation_preserves_resident_sibling_domains_and_logical_exit_gaps() {
    let mut launches = Vec::new();
    for groups in [1, 128, 4096] {
        let (resident, programs, states, cells) = fixture(groups);
        launches.push(check(&resident, &programs, &states, cells));
    }
    assert!(
        launches[2] - launches[1] <= 2,
        "independent domains must not create serial commands: {launches:?}"
    );
}

#[test]
fn closure_preparation_keeps_roots_frames_captures_and_program_domains_distinct() {
    let (resident, programs, mut states, _) = fixture(2);
    let base = states[1].clone();
    states.truncate(3);
    let expression = base.node;
    let identifier = resident.input.packed.children
        [resident.input.packed.nodes[expression as usize].child_start as usize];
    for case in 0..7 {
        let mut s = base.clone();
        s.captures = vec![[expression, 0, 0, 1], [expression, 1, 0, 1]];
        match case {
            0 => (),
            1 => s.cursors[0][1] = expression,
            2 => s.repeats = vec![[0, 1, 0, 0], [1, 0, 0, 0]],
            3 => s.captures.reverse(),
            4 => s.captures.push([expression, 1, 0, 1]),
            5 => s.captures[0][0] = identifier,
            6 => {
                s.cursors = (0..256).map(|i| [i, 0, 0, 0]).collect();
                s.repeats = (0..512).map(|i| [i, 0, 0, 0]).collect();
                s.captures = vec![[expression, 0, 0, 1]; 768];
            }
            _ => unreachable!(),
        }
        s.ordinal = states.len() as u32 * 10 + 7;
        states.push(s);
    }
    // A root and its children have different singleton/chain domains even with
    // identical complete control values. Other file roots also stay distinct.
    for node in [0, resident.input.packed.files[1].node_start] {
        let mut s = base.clone();
        s.node = node;
        s.ordinal = states.len() as u32 * 10 + 7;
        states.push(s);
    }
    check(&resident, &programs, &states, 8 * 4 + 2);
    check(&resident, &programs, &[], 0);
}

#[test]
fn closure_preparation_evaluates_mixed_generated_guards_and_long_operands() {
    let doc = document("x\n# a\n# bb\n# ccc\n");
    let siblings = doc.transducer_children(0).collect::<Vec<_>>();
    let comment = doc.transducer_kind(siblings[1]);
    let bodies = vec![
        vec![],
        vec![word(KIND, 0, comment)],
        vec![word(FIELD, 0, 0)],
        vec![word(FLAGS, 1, 0)],
        vec![word(FLAGS, 0, 1)],
        vec![word(SIZE, 0, 4)],
        vec![word(SIZE, 1, 4)],
        vec![word(SIZE, 2, 4)],
        vec![word(SIZE, SIZE_LONG, 0)],
        vec![word(SIZE, SIZE_LONG | 1, 0)],
        vec![word(SIZE, SIZE_LONG | 2, 0)],
        vec![word(SIZE, SIZE_LONG | 2, 1)],
        vec![word(FLAGS, 0, 0); 64],
    ];
    let programs = bodies
        .iter()
        .map(|body| program(body, vec![4, u64::MAX]))
        .collect::<Vec<_>>();
    let roots = vec![vec![vec![0]; programs.len()]];
    let resident = ResidentInput::new(
        MetalGridInput::build(&programs, &[doc], &roots).unwrap(),
        None,
    )
    .unwrap();
    let mut states = Vec::new();
    for pg in 0..programs.len() {
        for offset in [2, 0, 2, 1] {
            states.push(State {
                task: pg as u32,
                program: pg as u32,
                node: siblings[offset],
                requirement: 0,
                cursors: vec![],
                repeats: vec![[0, 0, 0, 0]],
                captures: vec![],
                ordinal: states.len() as u32 * 3 + 1,
            });
        }
    }
    check(&resident, &programs, &states, programs.len() * 4);
}

#[test]
fn closure_preparation_rejects_invalid_headers_and_capacity_without_partial_ranges() {
    for case in 0..17 {
        let _guard = metal_execution_guard().unwrap();
        let device = Device::system_default().unwrap();
        let (resident, programs, states, _) = fixture(1);
        let data = super::metal_register_execution::upload(&device, &resident).unwrap();
        let (frontier, values) = input(&device, &states);
        let mut prepared = output(&device, frontier.capacity, 11);
        unsafe {
            let c = frontier.columns.buffer.contents().cast::<u32>();
            let h = frontier.counts.buffer.contents().cast::<u32>();
            match case {
                0 => h.write(frontier.capacity as u32 + 1),
                1 => h.add(2).write(2),
                2 => c.add(2 * states.len()).write(u32::MAX),
                3 => c.add(states.len()).write(0),
                4 => c.add(4 * states.len()).write(99),
                5 => c.add(5 * states.len()).write(2),
                6 => c.add(9 * states.len()).write(0),
                7 => values
                    .repeats
                    .buffer
                    .contents()
                    .cast::<u32>()
                    .add(1)
                    .write(1),
                8 => c.add(8 * states.len()).write(u32::MAX),
                9 => values
                    .cursors
                    .buffer
                    .contents()
                    .cast::<u32>()
                    .add(2)
                    .write(1),
                10 => c.add(3 * states.len()).write(4),
                11 => h.add(1).write(u32::MAX),
                12 => prepared.ranges.cell_capacity = 3,
                13 => prepared.ranges.domain_capacity = 0,
                14 => prepared.ranges.entry_capacity = 2,
                15 => values
                    .counts
                    .buffer
                    .contents()
                    .cast::<u32>()
                    .add(1)
                    .write(0),
                16 => h.add(3).write(0),
                _ => unreachable!(),
            }
        }
        let exits = ClosureExits {
            exits: MappedBuffer::from_slice(&device, &[[u32::MAX; 2]; 10]).unwrap(),
            counts: MappedBuffer::from_slice(&device, &[u32::MAX; 4]).unwrap(),
            capacity: 10,
            generation: 11,
        };
        let consumed = MappedBuffer::from_slice(&device, &[[u32::MAX; 3]; 10]).unwrap();
        let queue = device.new_command_queue();
        let command = queue.new_command_buffer();
        let pending = metal_closure_prepare::encode(
            &device,
            command,
            ClosureInput {
                resident: &resident,
                programs: &programs,
                data: &data,
                states: &frontier,
                values: &values,
            },
            &prepared,
        )
        .unwrap();
        let closure =
            metal_bulk_closure::encode(&device, command, &prepared.ranges, &exits).unwrap();
        metal_parallel::encode(
            command,
            &pipeline_named(IO, "prepare_test_consume").unwrap(),
            "prepare_test_consume",
            &[
                &exits.exits.buffer,
                &exits.counts.buffer,
                &prepared.nodes.buffer,
                &frontier.columns.buffer,
                &frontier.counts.buffer,
                &consumed.buffer,
            ],
            &[],
            0,
            Some((&exits.counts.buffer, 4)),
        );
        commit_checked(command).unwrap();
        wait_checked(command).unwrap();
        assert_eq!(
            pending.check_completed(),
            Err(if (12..=14).contains(&case) {
                Error::BackendCapacity
            } else {
                Error::BackendFailure
            }),
            "case {case}"
        );
        closure.check_completed().unwrap();
        assert_eq!(
            unsafe {
                std::slice::from_raw_parts(
                    prepared.ranges.counts.buffer.contents().cast::<u32>(),
                    3,
                )
            },
            [0, 0, 0]
        );
        assert_eq!(
            unsafe { std::slice::from_raw_parts(exits.counts.buffer.contents().cast::<u32>(), 4) },
            [0, 0, 1, 1]
        );
        for buffer in [
            &prepared.ranges.domains,
            &prepared.ranges.entries,
            &prepared.ranges.allowed,
            &prepared.nodes,
            &exits.exits,
            &consumed,
        ] {
            let len = if std::ptr::eq(buffer, &prepared.ranges.domains)
                || std::ptr::eq(buffer, &prepared.ranges.entries)
            {
                frontier.capacity * 2
            } else if std::ptr::eq(buffer, &prepared.ranges.allowed)
                || std::ptr::eq(buffer, &prepared.nodes)
            {
                11
            } else if std::ptr::eq(buffer, &exits.exits) {
                20
            } else {
                30
            };
            assert!(
                unsafe { std::slice::from_raw_parts(buffer.buffer.contents().cast::<u32>(), len) }
                    .iter()
                    .all(|&v| v == u32::MAX),
                "case {case}"
            );
        }
    }
}

#[test]
fn closure_preparation_rejects_host_aliases_generations_and_unproved_programs() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let (resident, programs, states, _) = fixture(1);
    let data = super::metal_register_execution::upload(&device, &resident).unwrap();
    let (frontier, mut values) = input(&device, &states);
    let mut prepared = output(&device, frontier.capacity, 11);
    let queue = device.new_command_queue();
    let scope = LaunchScope::enter();
    let command = queue.new_command_buffer();
    prepared.ranges.generation += 1;
    assert!(matches!(
        metal_closure_prepare::encode(
            &device,
            command,
            ClosureInput {
                resident: &resident,
                programs: &programs,
                data: &data,
                states: &frontier,
                values: &values
            },
            &prepared
        ),
        Err(Error::BackendFailure)
    ));
    prepared.ranges.generation -= 1;
    values.generation += 1;
    assert!(matches!(
        metal_closure_prepare::encode(
            &device,
            command,
            ClosureInput {
                resident: &resident,
                programs: &programs,
                data: &data,
                states: &frontier,
                values: &values
            },
            &prepared
        ),
        Err(Error::BackendFailure)
    ));
    values.generation -= 1;
    let nodes = prepared.nodes;
    prepared.nodes = MappedBuffer {
        buffer: prepared.ranges.allowed.buffer.clone(),
    };
    assert!(matches!(
        metal_closure_prepare::encode(
            &device,
            command,
            ClosureInput {
                resident: &resident,
                programs: &programs,
                data: &data,
                states: &frontier,
                values: &values
            },
            &prepared
        ),
        Err(Error::BackendFailure)
    ));
    prepared.nodes = nodes;
    prepared.ranges.cell_capacity = u32::MAX as usize + 1;
    assert!(matches!(
        metal_closure_prepare::encode(
            &device,
            command,
            ClosureInput {
                resident: &resident,
                programs: &programs,
                data: &data,
                states: &frontier,
                values: &values
            },
            &prepared
        ),
        Err(Error::BackendCapacity)
    ));
    prepared.ranges.cell_capacity = 11;
    let short = MappedBuffer::zeroed::<u32>(&device, 0).unwrap();
    // Mapped buffers are page-sized; an allocation shorter than its declaration
    // must fail even when the byte count still fits the host integer type.
    prepared.ranges.cell_capacity = short.buffer.length() as usize / 4 + 1;
    prepared.nodes = short;
    assert!(matches!(
        metal_closure_prepare::encode(
            &device,
            command,
            ClosureInput {
                resident: &resident,
                programs: &programs,
                data: &data,
                states: &frontier,
                values: &values
            },
            &prepared
        ),
        Err(Error::BackendFailure)
    ));
    let prepared = output(&device, frontier.capacity, 11);
    let different = vec![program(&[word(SIZE, 2, 4)], vec![])];
    assert!(matches!(
        metal_closure_prepare::encode(
            &device,
            command,
            ClosureInput {
                resident: &resident,
                programs: &different,
                data: &data,
                states: &frontier,
                values: &values
            },
            &prepared
        ),
        Err(Error::InvalidAnchor)
    ));
    assert!(scope.snapshot().is_empty());
    let pending = metal_closure_prepare::encode(
        &device,
        command,
        ClosureInput {
            resident: &resident,
            programs: &programs,
            data: &data,
            states: &frontier,
            values: &values,
        },
        &prepared,
    )
    .unwrap();
    assert_eq!(pending.check_completed(), Err(Error::BackendFailure));
    assert!(scope.snapshot().iter().all(|d| d.encoded && !d.submitted));
}

#[test]
fn closure_preparation_shares_the_evaluation_budget_across_commands() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let (resident, programs, states, _) = fixture(1);
    let data = super::metal_register_execution::upload(&device, &resident).unwrap();
    let (frontier, values) = input(&device, &states);
    let prepared = output(&device, frontier.capacity, 11);
    let queue = device.new_command_queue();
    let scope = LaunchScope::enter();
    let mut completed = 0;
    let mut refusal = None;
    for _ in 0..=600 {
        let _nested = LaunchScope::enter();
        let command = queue.new_command_buffer();
        let pending = metal_closure_prepare::encode(
            &device,
            command,
            ClosureInput {
                resident: &resident,
                programs: &programs,
                data: &data,
                states: &frontier,
                values: &values,
            },
            &prepared,
        )
        .unwrap();
        match commit_checked(command) {
            Ok(()) => {
                wait_checked(command).unwrap();
                pending.check_completed().unwrap();
                completed = scope.snapshot().len();
            }
            Err(error) => {
                refusal = Some((error, pending.check_completed()));
                break;
            }
        }
    }
    let trace = scope.snapshot();
    drop(_guard);
    assert_eq!(
        refusal,
        Some((Error::BackendLaunchLimit, Err(Error::BackendLaunchLimit)))
    );
    assert!(completed > 0 && completed <= 600);
    assert_eq!(trace.iter().filter(|d| d.encoded).count(), 600);
    assert_eq!(trace.iter().filter(|d| d.submitted).count(), completed);
    assert_eq!(trace.iter().filter(|d| d.completed).count(), completed);
    assert!(
        trace[..completed]
            .iter()
            .all(|d| d.encoded && d.submitted && d.completed)
    );
    assert!(
        trace[completed..]
            .iter()
            .all(|d| !d.submitted && !d.completed)
    );
    assert!(trace.iter().skip(600).all(|d| !d.encoded));
}

#[test]
fn closure_preparation_covers_guard_blocks_and_singleton_roots() {
    for count in [1, 255, 256, 257, 511, 1024] {
        let source = (0..count)
            .map(|i| if i % 127 == 1 { "x\n" } else { "# a\n" })
            .collect::<String>();
        let doc = document(&source);
        let siblings = doc.transducer_children(0).collect::<Vec<_>>();
        assert_eq!(siblings.len(), count);
        let kind = doc.transducer_kind(siblings[0]);
        let programs = vec![program(&[word(KIND, 0, kind)], vec![])];
        let resident = ResidentInput::new(
            MetalGridInput::build(&programs, &[doc], &[vec![vec![0]]]).unwrap(),
            None,
        )
        .unwrap();
        let mut states = Vec::new();
        for node in [siblings[count - 1], siblings[0], siblings[count - 1], 0] {
            states.push(State {
                task: 0,
                program: 0,
                node,
                requirement: 0,
                cursors: vec![],
                repeats: vec![[0, 0, 0, 0]],
                captures: vec![],
                ordinal: states.len() as u32 * 5 + 1,
            });
        }
        check(&resident, &programs, &states, count + 1);
    }
}

#[test]
fn closure_preparation_refuses_valid_but_ineligible_header_batches() {
    for guards in [vec![word(FLAGS, 0, 0); 65], vec![word(SAVE_CURSOR, 0, 0)]] {
        let _guard = metal_execution_guard().unwrap();
        let device = Device::system_default().unwrap();
        let (_, _, states, _) = fixture(1);
        let programs = vec![program(&guards, vec![])];
        let resident = ResidentInput::new(
            MetalGridInput::build(
                &programs,
                &[document("x\n# a\n# bb\n# ccc\n")],
                &[vec![vec![0]]],
            )
            .unwrap(),
            None,
        )
        .unwrap();
        assert!(closure_proof::regions(&programs[0]).unwrap().is_empty());
        let data = super::metal_register_execution::upload(&device, &resident).unwrap();
        let (frontier, values) = input(&device, &states);
        let prepared = output(&device, frontier.capacity, 11);
        let queue = device.new_command_queue();
        let command = queue.new_command_buffer();
        let pending = metal_closure_prepare::encode(
            &device,
            command,
            ClosureInput {
                resident: &resident,
                programs: &programs,
                data: &data,
                states: &frontier,
                values: &values,
            },
            &prepared,
        )
        .unwrap();
        commit_checked(command).unwrap();
        wait_checked(command).unwrap();
        assert_eq!(pending.check_completed(), Err(Error::BackendFailure));
        assert_eq!(
            unsafe {
                std::slice::from_raw_parts(
                    prepared.ranges.counts.buffer.contents().cast::<u32>(),
                    3,
                )
            },
            [0, 0, 0]
        );
    }
}
