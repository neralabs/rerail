//! Observe complete successor values, not generated code or private dispatch choices.
use super::metal_backend::launch::{commit_checked, wait_checked};
use super::metal_backend::{MappedBuffer, metal_execution_guard};
use super::metal_grid::MetalGridInput;
use super::metal_grid_plan::{ExecutionPlan, ResidentInput};
use super::metal_ready_batch::ReadyBatch;
use super::metal_state_values::{POSITIONED_COLUMNS as COLUMNS, PositionedFrontier, StateValues};
use super::metal_successor::{self, ControlValues, SuccessorInput, SuccessorOutput};
use super::*;
use crate::code_rule_dsl::tape::{Grammar, TapeImage};
use ast_grep_core::tree_sitter::StrDoc;
use crate::code_rule_dsl::language::GrammarDialect as SupportLang;
use metal::Device;

#[path = "metal_closure_continuation_tests.rs"]
mod closure_tests;
#[path = "metal_terminal_tests.rs"]
mod terminal_tests;

#[derive(Clone, Debug, PartialEq, Eq)]
struct State {
    task: u32,
    pc: u32,
    node: u32,
    kind: u32,
    cursors: Vec<[u32; 4]>,
    repeats: Vec<[u32; 4]>,
    captures: Vec<[u32; 4]>,
    position: Vec<u32>,
    body: Vec<u32>,
    offer: Vec<u32>,
}
fn state(pc: u32, kind: u32) -> State {
    State {
        task: 0,
        pc,
        node: 1,
        kind,
        cursors: vec![],
        repeats: vec![],
        captures: vec![],
        position: vec![],
        body: vec![],
        offer: vec![],
    }
}

#[test]
fn semantic_operand_failures_retain_only_the_executed_branch_prefix() {
    let p = program(vec![
        word(REPEAT_BEGIN, 0, 0),
        word(FORK, 0, 6),
        word(RESTORE_CURSOR, 0, 0),
        word(FORK, 0, 5),
        word(EMIT, 0, 0),
        word(EMIT, 0, 0),
        word(EMIT, 0, 0),
    ])
    .with_tree_sitter_query_semantics()
    .unwrap();
    let mut input = state(0, 0);
    input.position = vec![9];
    let mut failed = state(2, 8);
    failed.repeats = vec![[0, 0, 0, 0]];
    failed.position = vec![9, 0];
    let mut completed = state(6, 1);
    completed.repeats = vec![[0, 0, 0, 0]];
    completed.position = vec![9, 1];
    assert_eq!(run(&p, &[input], 1), vec![failed, completed]);
}

#[test]
fn finished_operand_failures_require_a_real_invalid_operand_in_their_saved_frame() {
    let restore = program(vec![word(RESTORE_CURSOR, 0, 7), word(EMIT, 0, 0)]);
    let failed = state(0, 9);
    assert_eq!(
        run(&restore, std::slice::from_ref(&failed), 1),
        vec![failed.clone()]
    );
    let mut restored = failed.clone();
    restored.cursors = vec![[7, 1, 0, 0]];
    assert_eq!(
        attempt(&restore, &[restored], 1, |_, _, _| {}),
        Err(Error::BackendFailure)
    );
    for opcode in [REPEAT_BUMP, REPEAT_END] {
        let repeat = program(vec![word(opcode, 0, 7), word(EMIT, 0, 0)]);
        assert_eq!(
            run(&repeat, std::slice::from_ref(&failed), 1),
            vec![failed.clone()]
        );
        let mut matched = failed.clone();
        matched.repeats = vec![[0, 7, 0, 0]];
        assert_eq!(
            attempt(&repeat, &[matched], 1, |_, _, _| {}),
            Err(Error::BackendFailure)
        );
    }
    for opcode in [CAPTURE, EMIT] {
        let normal = program(vec![word(opcode, 0, 0), word(EMIT, 0, 0)]);
        assert_eq!(
            attempt(&normal, std::slice::from_ref(&failed), 1, |_, _, _| {}),
            Err(Error::BackendFailure)
        );
    }
}
fn program(words: Vec<u32>) -> Program {
    Program::new(words, vec![b"match".to_vec()], vec![], vec![0], vec![]).unwrap()
}

#[test]
fn pending_query_exits_require_a_declared_capture_free_greedy_region() {
    let p = program(vec![
        word(REPEAT_BEGIN, 0, 0),
        word(REPEAT_BUMP, 0, 0),
        word(FORK, GREEDY, 6),
        word(NEXT, 0, 0),
        word(REPEAT_BUMP, 0, 0),
        word(GOTO, 0, 2),
        word(REPEAT_END, 0, 0),
        word(EMIT, 0, 0),
    ])
    .with_tree_sitter_query_semantics()
    .unwrap();
    let mut pending = state(6, 6);
    pending.repeats = vec![[0, 0, 0, 0]];
    pending.position = vec![9, 1];
    pending.body = vec![9, 0];
    pending.offer = vec![9];
    let mut expected = pending.clone();
    expected.kind = 0;
    expected.body.clear();
    expected.offer.clear();
    assert_eq!(run(&p, std::slice::from_ref(&pending), 1), vec![expected]);
    pending.pc = 7;
    assert_eq!(
        attempt(&p, &[pending], 1, |_, _, _| {}),
        Err(Error::BackendFailure)
    );
}
fn fixture(program: &Program, tasks: usize) -> (ResidentInput, ExecutionPlan) {
    fixture_sources(
        program,
        tasks,
        &["# first\n# second\n# third\n".repeat(tasks.max(1))],
    )
}
fn fixture_sources(
    program: &Program,
    tasks: usize,
    sources: &[String],
) -> (ResidentInput, ExecutionPlan) {
    let docs = sources
        .iter()
        .map(|text| {
            let source = Arc::new(text.clone());
            let grammar = Grammar::new(SupportLang::Python).unwrap();
            let native = StrDoc::new(source.as_str(), SupportLang::Python);
            let image =
                TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
            TapeDoc::restore(source, grammar, &image, &|| false).unwrap()
        })
        .collect::<Vec<_>>();
    let roots = (1..=tasks as u32).collect::<Vec<_>>();
    let roots = (0..sources.len())
        .map(|i| vec![if i == 0 { roots.clone() } else { vec![] }])
        .collect::<Vec<_>>();
    let resident = ResidentInput::new(
        MetalGridInput::build(std::slice::from_ref(program), &docs, &roots).unwrap(),
        None,
    )
    .unwrap();
    let plan = ExecutionPlan::new(&resident, std::slice::from_ref(program)).unwrap();
    (resident, plan)
}
struct Storage {
    states: PositionedFrontier,
    values: StateValues,
    control: ControlValues,
}
fn storage(device: &Device, capacity: usize, cells: usize, generation: u64) -> Storage {
    Storage {
        states: PositionedFrontier {
            columns: MappedBuffer::from_slice(device, &vec![u32::MAX; capacity * COLUMNS]).unwrap(),
            counts: MappedBuffer::from_slice(device, &[u32::MAX; 5]).unwrap(),
            capacity,
            generation,
        },
        values: StateValues {
            cursors: MappedBuffer::from_slice(device, &vec![[u32::MAX; 4]; cells]).unwrap(),
            repeats: MappedBuffer::from_slice(device, &vec![[u32::MAX; 4]; cells]).unwrap(),
            captures: MappedBuffer::from_slice(device, &vec![[u32::MAX; 4]; cells]).unwrap(),
            counts: MappedBuffer::from_slice(device, &[u32::MAX; 2]).unwrap(),
            cursor_capacity: cells,
            repeat_capacity: cells,
            capture_capacity: cells,
            generation,
        },
        control: ControlValues {
            path_work: MappedBuffer::from_slice(device, &vec![u32::MAX; capacity]).unwrap(),
            spans: MappedBuffer::from_slice(device, &vec![u32::MAX; capacity * 6]).unwrap(),
            digits: MappedBuffer::from_slice(device, &vec![u32::MAX; cells]).unwrap(),
            count: MappedBuffer::from_slice(device, &[u32::MAX]).unwrap(),
            state_capacity: capacity,
            digit_capacity: cells,
            generation,
        },
    }
}
fn write<T: Copy>(buffer: &MappedBuffer, values: &[T]) {
    assert!(std::mem::size_of_val(values) as u64 <= buffer.buffer.length());
    unsafe {
        std::ptr::copy_nonoverlapping(
            values.as_ptr(),
            buffer.buffer.contents().cast(),
            values.len(),
        )
    }
}
fn read<T: Copy>(buffer: &MappedBuffer, count: usize) -> Vec<T> {
    assert!((std::mem::size_of::<T>() * count) as u64 <= buffer.buffer.length());
    unsafe { std::slice::from_raw_parts(buffer.buffer.contents().cast(), count).to_vec() }
}
fn upload(device: &Device, rows: &[State], program: &Program) -> Storage {
    let cells = rows
        .iter()
        .map(|s| {
            s.cursors.len()
                + s.repeats.len()
                + s.captures.len()
                + s.position.len()
                + s.body.len()
                + s.offer.len()
        })
        .sum::<usize>()
        + 32;
    let input = storage(device, rows.len() + 7, cells, 8);
    let n = rows.len();
    let mut columns = vec![0; n * COLUMNS];
    let mut spans = vec![0; n * 6];
    let mut arenas: [Vec<[u32; 4]>; 3] = std::array::from_fn(|_| vec![]);
    let mut digits: Vec<u32> = vec![];
    for (i, s) in rows.iter().enumerate() {
        let requirement = program.sibling_requirement(s.pc as u16) as u32;
        for (c, v) in [s.task, s.pc, s.node, requirement, 0, s.kind]
            .iter()
            .enumerate()
        {
            columns[c * n + i] = *v;
        }
        for (k, values) in [&s.cursors, &s.repeats, &s.captures]
            .into_iter()
            .enumerate()
        {
            columns[(6 + k * 2) * n + i] = arenas[k].len() as u32;
            columns[(7 + k * 2) * n + i] = values.len() as u32;
            arenas[k].extend(values);
        }
        for (k, values) in [&s.position, &s.body, &s.offer].into_iter().enumerate() {
            spans[(k * 2) * n + i] = digits.len() as u32;
            spans[(k * 2 + 1) * n + i] = values.len() as u32;
            digits.extend(values);
        }
    }
    write(&input.states.columns, &columns);
    write(
        &input.states.counts,
        &[
            n as u32,
            arenas[2].len() as u32,
            n.div_ceil(256) as u32,
            1,
            1,
        ],
    );
    write(&input.values.cursors, &arenas[0]);
    write(&input.values.repeats, &arenas[1]);
    write(&input.values.captures, &arenas[2]);
    write(
        &input.values.counts,
        &[arenas[0].len() as u32, arenas[1].len() as u32],
    );
    write(&input.control.path_work, &vec![0u32; n]);
    write(&input.control.spans, &spans);
    write(&input.control.digits, &digits);
    write(&input.control.count, &[digits.len() as u32]);
    input
}
fn observe(output: &Storage, program: &Program) -> Vec<State> {
    observe_programs(output, program, |_| 0)
}
fn observe_programs(
    output: &Storage,
    program: &Program,
    program_for: impl Fn(u32) -> u32,
) -> Vec<State> {
    let h = read::<u32>(&output.states.counts, 5);
    let n = h[0] as usize;
    assert!(n <= output.states.capacity);
    assert_eq!(&h[2..], &[n.div_ceil(256) as u32, 1, 1]);
    let c = read::<u32>(&output.states.columns, output.states.capacity * COLUMNS);
    assert!(c[n * COLUMNS..].iter().all(|&x| x == u32::MAX));
    let counts = read::<u32>(&output.values.counts, 2);
    let lengths = [counts[0], counts[1], h[1]];
    let arrays = [
        (&output.values.cursors, output.values.cursor_capacity),
        (&output.values.repeats, output.values.repeat_capacity),
        (&output.values.captures, output.values.capture_capacity),
    ]
    .map(|(buffer, capacity)| read::<[u32; 4]>(buffer, capacity));
    for (k, a) in arrays.iter().enumerate() {
        assert!(a[lengths[k] as usize..].iter().all(|&x| x == [u32::MAX; 4]));
    }
    let spans = read::<u32>(&output.control.spans, output.control.state_capacity * 6);
    assert!(spans[n * 6..].iter().all(|&x| x == u32::MAX));
    let length = read::<u32>(&output.control.count, 1)[0] as usize;
    let digits = read::<u32>(&output.control.digits, output.control.digit_capacity);
    assert!(digits[length..].iter().all(|&x| x == u32::MAX));
    (0..n)
        .map(|i| {
            let cells = |k: usize| {
                let start = c[(6 + k * 2) * n + i] as usize;
                let count = c[(7 + k * 2) * n + i] as usize;
                assert!(start + count <= lengths[k] as usize);
                arrays[k][start..start + count].to_vec()
            };
            let path = |k: usize| {
                let start = spans[k * 2 * n + i] as usize;
                let count = spans[(k * 2 + 1) * n + i] as usize;
                assert!(start + count <= length);
                digits[start..start + count].to_vec()
            };
            assert_eq!(c[4 * n + i], program_for(c[i]));
            assert_eq!(
                c[3 * n + i],
                program.sibling_requirement(c[n + i] as u16) as u32
            );
            State {
                task: c[i],
                pc: c[n + i],
                node: c[2 * n + i],
                kind: c[5 * n + i],
                cursors: cells(0),
                repeats: cells(1),
                captures: cells(2),
                position: path(0),
                body: path(1),
                offer: path(2),
            }
        })
        .collect()
}
fn run(program: &Program, rows: &[State], tasks: usize) -> Vec<State> {
    attempt(program, rows, tasks, |_, _, _| {})
        .expect("bounded successors must preserve the complete behavioral result")
}
fn attempt(
    program: &Program,
    rows: &[State],
    tasks: usize,
    configure: impl FnOnce(&mut Storage, &mut Storage, &mut ReadyBatch),
) -> Result<Vec<State>, Error> {
    attempt_resident(program, rows, tasks, fixture(program, tasks), configure)
}
fn attempt_resident(
    program: &Program,
    rows: &[State],
    tasks: usize,
    fixture: (ResidentInput, ExecutionPlan),
    configure: impl FnOnce(&mut Storage, &mut Storage, &mut ReadyBatch),
) -> Result<Vec<State>, Error> {
    let _guard = metal_execution_guard().unwrap();
    let scope = super::metal_backend::launch::LaunchScope::enter();
    let device = Device::system_default().unwrap();
    let (resident, plan) = fixture;
    let data = super::metal_register_execution::upload(&device, &resident).unwrap();
    let mut input = upload(&device, rows, program);
    let cells = rows
        .iter()
        .map(|s| {
            s.cursors.len()
                + s.repeats.len()
                + s.captures.len()
                + s.position.len()
                + s.body.len()
                + s.offer.len()
        })
        .sum::<usize>()
        * 4
        + rows.len() * 128
        + 64;
    let mut output = storage(&device, rows.len() * 4 + 7, cells, 9);
    let mut ids = vec![u32::MAX; input.states.capacity * 4];
    let mut counts = [0u32; 17];
    for (i, s) in rows.iter().enumerate() {
        let kind = match s.kind {
            1 | 3 | 8 | 9 | 13 => 3,
            6 | 7 | 11 => 2,
            _ => 0,
        };
        ids[kind * input.states.capacity + counts[kind] as usize] = i as u32;
        counts[kind] += 1;
        counts[16] += u32::from(s.kind == 11);
    }
    for k in 0..4 {
        counts[4 + k * 3] = counts[k].div_ceil(256);
        counts[5 + k * 3] = 1;
        counts[6 + k * 3] = 1;
    }
    let mut ready = ReadyBatch {
        indices: MappedBuffer::from_slice(&device, &ids).unwrap(),
        counts: MappedBuffer::from_slice(&device, &counts).unwrap(),
        capacity: input.states.capacity,
        generation: 8,
    };
    configure(&mut input, &mut output, &mut ready);
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    let pending = metal_successor::encode(
        &device,
        &queue,
        command,
        SuccessorInput {
            resident: &resident,
            plan: &plan,
            data: &data,
            states: &input.states,
            values: &input.values,
            control: &input.control,
            ready: &ready,
        },
        SuccessorOutput {
            states: &output.states,
            values: &output.values,
            control: &output.control,
        },
    )?;
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    let result = pending.check_completed();
    let trace = scope.snapshot();
    assert!(trace.iter().all(|r| r.completed));
    eprintln!(
        "SUCCESSOR_EVIDENCE {}",
        serde_json::json!({"tasks":tasks,"states":rows.len(),"launches":trace.len(),"peakMetalBytes":trace.iter().map(|r|r.peak_application_buffer_bytes).max().unwrap_or(0)})
    );
    if let Err(error) = result {
        assert_eq!(read::<u32>(&output.states.counts, 5), vec![0, 0, 0, 1, 1]);
        for (buffer, count) in [
            (&output.states.columns, output.states.capacity * COLUMNS),
            (&output.values.cursors, output.values.cursor_capacity * 4),
            (&output.values.repeats, output.values.repeat_capacity * 4),
            (&output.values.captures, output.values.capture_capacity * 4),
            (&output.control.spans, output.control.state_capacity * 6),
            (&output.control.path_work, output.control.state_capacity),
            (&output.control.digits, output.control.digit_capacity),
        ] {
            assert!(read::<u32>(buffer, count).iter().all(|&x| x == u32::MAX));
        }
        assert_eq!(read::<u32>(&output.values.counts, 2), vec![0, 0]);
        assert_eq!(read::<u32>(&output.control.count, 1), vec![0]);
        return Err(error);
    }
    Ok(observe(&output, program))
}
#[test]
fn bounded_prefix_preserves_exact_cursor_repeat_capture_and_branch_values() {
    let p = program(vec![
        word(SAVE_CURSOR, 0, 5),
        word(NEXT, 0, 0),
        word(SAVE_CURSOR, 0, 2),
        word(REPEAT_BEGIN, 0, 9),
        word(REPEAT_BUMP, 0, 9),
        word(CAPTURE, 0, 0),
        word(FORK, 0, 9),
        word(EMIT, 0, 0),
        word(STOP, 0, 0),
        word(EMIT, 0, 0),
    ]);
    let mut s = state(0, 2);
    s.cursors = vec![[0, 3, 0, 0], [7, 1, 0, 0]];
    s.repeats = vec![[0, 4, 0, 0]];
    s.captures = vec![[1, 0, 0, 7]];
    s.position = vec![4, 7];
    let mut first = s.clone();
    first.pc = 7;
    first.node = 2;
    first.cursors = vec![[0, 3, 0, 0], [2, 2, 0, 0]];
    first.repeats.push([1, 9, 0, 0]);
    first.captures.push([2, 0, 8, 16]);
    first.position.push(0);
    let mut second = first.clone();
    second.pc = 9;
    *second.position.last_mut().unwrap() = 1;
    assert_eq!(run(&p, &[s], 1), vec![first, second]);
}
#[test]
fn greedy_fork_retains_a_pending_exit_with_its_exact_body_and_offer_positions() {
    let p = program(vec![
        word(FORK, GREEDY, 2),
        word(EMIT, 0, 0),
        word(EMIT, 0, 0),
    ]);
    let mut input = state(0, 2);
    input.position = vec![3, 5];
    input.captures = vec![[1, 0, 0, 7]];
    let mut body = input.clone();
    body.pc = 1;
    body.position.push(0);
    let mut exit = input.clone();
    exit.pc = 2;
    exit.kind = 7;
    exit.position.push(1);
    exit.body = body.position.clone();
    exit.offer = input.position.clone();
    assert_eq!(run(&p, &[input], 1), vec![body, exit]);
}

#[test]
fn bounded_prefix_limits_resume_at_the_exact_unexecuted_instruction() {
    for (instruction, count, boundary) in
        [(word(FLAGS, 0, 0), 65, 64), (word(CAPTURE, 0, 0), 33, 32)]
    {
        let mut words = vec![instruction; count];
        words.push(word(EMIT, 0, 0));
        let p = program(words);
        let mut input = state(0, 2);
        input.position = vec![3, 1];
        let mut expected = input.clone();
        expected.pc = boundary;
        if instruction == word(CAPTURE, 0, 0) {
            expected.captures = vec![[1, 0, 0, 7]; boundary as usize];
        }
        assert_eq!(run(&p, &[input], 1), vec![expected.clone()]);
        let mut terminal = expected.clone();
        terminal.pc = count as u32;
        terminal.kind = 3;
        if instruction == word(CAPTURE, 0, 0) {
            terminal.captures.push([1, 0, 0, 7]);
        }
        assert_eq!(run(&p, &[expected], 1), vec![terminal]);
    }
    let p = program(vec![
        word(FLAGS, 0, 0),
        word(REPEAT_END, 0, 9),
        word(EMIT, 0, 0),
    ]);
    assert_eq!(run(&p, &[state(0, 2)], 1), vec![state(1, 9)]);
}

#[test]
fn region_suffix_entries_preserve_existing_cursor_and_capture_values_without_replay() {
    let p = program(vec![
        word(SAVE_CURSOR, 0, 0),
        word(CAPTURE, 0, 0),
        word(CAPTURE, 0, 0),
        word(EMIT, 0, 0),
    ]);
    for pc in [1, 2, 3] {
        let mut input = state(pc, 2);
        input.cursors = vec![[0, 2, 0, 0], [7, 3, 0, 0]];
        input.captures = vec![[1, 0, 0, 7]];
        let mut expected = input.clone();
        expected.pc = 3;
        expected.kind = 3;
        expected.captures = vec![[1, 0, 0, 7]; (4 - pc) as usize];
        assert_eq!(run(&p, &[input], 1), vec![expected]);
    }
}

#[test]
fn greedy_merges_only_descendant_offers_and_keeps_the_latest_logical_offer() {
    let p = program(vec![
        word(FORK, GREEDY, 3),
        word(STOP, 0, 0),
        word(STOP, 0, 0),
        word(EMIT, 0, 0),
    ]);
    let mut old = state(3, 7);
    old.position = vec![1];
    old.body = vec![0];
    old.offer = vec![0, 4];
    old.node = 3;
    let mut early = state(0, 2);
    early.position = vec![0, 2];
    early.node = 1;
    let mut late = state(0, 2);
    late.position = vec![0, 6];
    late.node = 2;
    let mut unrelated = state(0, 2);
    unrelated.position = vec![2];
    // Deliberately reverse physical arrival order. It must not select the offer.
    let mut expected_old = old.clone();
    expected_old.offer = late.position.clone();
    expected_old.node = 2;
    let mut body = late.clone();
    body.pc = 1;
    body.position.push(0);
    let mut early_body = early.clone();
    early_body.pc = 1;
    early_body.position.push(0);
    let mut unrelated_body = unrelated.clone();
    unrelated_body.pc = 1;
    unrelated_body.position.push(0);
    let mut unrelated_exit = unrelated.clone();
    unrelated_exit.pc = 3;
    unrelated_exit.kind = 7;
    unrelated_exit.position.push(1);
    unrelated_exit.body = unrelated_body.position.clone();
    unrelated_exit.offer = unrelated.position.clone();
    assert_eq!(
        run(&p, &[late, early, unrelated, old], 1),
        vec![
            body,
            early_body,
            unrelated_body,
            unrelated_exit,
            expected_old
        ]
    );
}

#[test]
fn pending_exits_retire_after_their_live_body_finishes_but_not_with_nested_pending_work() {
    let p = program(vec![word(STOP, 0, 0), word(EMIT, 0, 0)]);
    let mut body = state(0, 2);
    body.position = vec![0, 0];
    let mut inner = state(1, 7);
    inner.position = vec![0, 1];
    inner.body = vec![0, 0];
    inner.offer = vec![0];
    let mut outer = state(1, 7);
    outer.position = vec![1];
    outer.body = vec![0];
    outer.offer = vec![];
    let mut retired = inner.clone();
    retired.kind = 2;
    retired.body.clear();
    retired.offer.clear();
    let waiting = outer.clone();
    assert_eq!(run(&p, &[body, inner, outer], 1), vec![retired, waiting]);
}

#[test]
fn complete_capture_multiplicity_survives_long_arena_copy_and_stable_insertion() {
    let p = program(vec![
        word(CAPTURE, 0, 0),
        word(FORK, 0, 3),
        word(EMIT, 0, 0),
        word(EMIT, 0, 0),
    ]);
    let mut input = state(0, 2);
    input.node = 2;
    input.position = vec![1; 1024];
    input.cursors = (0..1024).map(|slot| [slot, 1, 0, 0]).collect();
    input.repeats = (0..1024).map(|index| [index, 3, 0, 0]).collect();
    input.captures = vec![[1, 0, 0, 7]; 1024];
    input.captures.push([3, 0, 17, 24]);
    let mut first = input.clone();
    first.pc = 2;
    first.captures.insert(1024, [2, 0, 8, 16]);
    first.position.push(0);
    let mut second = first.clone();
    second.pc = 3;
    *second.position.last_mut().unwrap() = 1;
    assert_eq!(run(&p, &[input], 1), vec![first, second]);
}

#[test]
fn structural_expansions_produce_each_complete_occurrence_in_logical_order() {
    for op in [CHILD, DESCEND, FOLLOWING] {
        for mode in [0, NAMED] {
            let p = program(vec![word(op, mode, 0), word(EMIT, 0, 0)]);
            let mut input = state(0, 2);
            input.node = if op == FOLLOWING { 1 } else { 0 };
            input.position = vec![9];
            input.captures = vec![[1, 0, 0, 7]];
            let expected = (if op == FOLLOWING { 2 } else { 1 }..=3)
                .enumerate()
                .map(|(choice, node)| {
                    let mut s = input.clone();
                    s.pc = 1;
                    s.node = node;
                    s.position.push(choice as u32);
                    s
                })
                .collect::<Vec<_>>();
            assert_eq!(run(&p, &[input], 1), expected, "op={op}, mode={mode}");
        }
    }
}

#[test]
fn successors_reject_invalid_values_and_capacities_without_publishing_payloads() {
    let p = program(vec![word(REPEAT_BUMP, 0, 9), word(EMIT, 0, 0)]);
    let mut input = state(0, 2);
    input.repeats = vec![[0, 8, 0, 0]];
    let mut failure = input.clone();
    failure.kind = 9;
    assert_eq!(run(&p, std::slice::from_ref(&input), 1), vec![failure]);
    input.repeats[0][1] = 9;
    for defect in 0..18 {
        let result = attempt(&p, &[input.clone()], 1, |s, out, ready| match defect {
            0 => write(&s.values.repeats, &[[1u32, 9, 0, 0]]),
            1 => write(&s.states.counts, &[1u32, 0, 2, 1, 1]),
            2 => write(&s.control.count, &[u32::MAX]),
            3 => write(&ready.indices, &[u32::MAX]),
            4 => {
                out.states.capacity = 0;
                out.control.state_capacity = 0;
            }
            5 => out.values.repeat_capacity = 0,
            6 => {
                s.control.generation = 7;
            }
            7 => {
                ready.generation = 7;
            }
            8 => {
                let mut columns = read::<u32>(&s.states.columns, COLUMNS);
                columns[3] = 2;
                write(&s.states.columns, &columns);
            }
            9 => {
                out.control.digits.buffer = s.control.digits.buffer.to_owned();
            }
            10 => {
                write(
                    &ready.counts,
                    &[2u32, 0, 0, 0, 1, 1, 1, 0, 1, 1, 0, 1, 1, 0, 1, 1],
                );
            }
            11 => {
                write(&s.control.spans, &[0u32, 0, 0, 1, 0, 0]);
                write(&s.control.count, &[1u32]);
            }
            12 => {
                let mut columns = read::<u32>(&s.states.columns, COLUMNS);
                columns[11] = 1;
                write(&s.states.columns, &columns);
                write(&s.states.counts, &[1u32, 1, 1, 1, 1]);
                write(&s.values.captures, &[[1u32, 9, 0, 7]]);
            }
            13 => write(&s.control.path_work, &[(MAX_STEPS_PER_PATH + 1) as u32]),
            14 => {
                out.control.path_work.buffer = s.control.path_work.buffer.clone();
            }
            15..=17 => {
                let mut columns = read::<u32>(&s.states.columns, COLUMNS);
                // These must reject before a packed header could turn them into
                // an apparently valid kind (0 or 2) through integer narrowing.
                columns[5] = [256, 258, u32::MAX][defect - 15];
                write(&s.states.columns, &columns);
            }
            _ => unreachable!(),
        });
        assert_eq!(
            result,
            Err(if defect == 4 || defect == 5 {
                Error::BackendCapacity
            } else {
                Error::BackendFailure
            }),
            "defect {defect}"
        );
    }
}

#[test]
fn independent_tasks_share_broad_successor_stages_and_preserve_every_occurrence() {
    let p = program(vec![word(FORK, 0, 2), word(EMIT, 0, 0), word(EMIT, 0, 0)]);
    for tasks in [0, 1, 128, 4096] {
        let rows = (0..tasks)
            .map(|task| {
                let mut s = state(0, 2);
                s.task = task as u32;
                s.node = task as u32 + 1;
                s
            })
            .collect::<Vec<_>>();
        let expected = rows
            .iter()
            .flat_map(|s| {
                (0..2).map(move |choice| {
                    let mut result = s.clone();
                    result.pc = choice + 1;
                    result.position.push(choice);
                    result
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(run(&p, &rows, tasks), expected, "tasks={tasks}");
    }
}

#[test]
fn query_admission_uses_full_values_and_logical_positions_while_ordinary_states_keep_occurrences() {
    let ordinary = program(vec![
        word(REPEAT_BEGIN, 0, 9),
        word(REPEAT_END, 0, 9),
        word(EMIT, 0, 0),
    ]);
    let query = ordinary.clone().with_tree_sitter_query_semantics().unwrap();
    let rows = [3, 1, 2].map(|digit| {
        let mut s = state(0, 0);
        s.position = vec![digit];
        s.cursors = vec![[1, 2, 0, 0]];
        s.captures = vec![[1, 0, 0, 7]; 257];
        s
    });
    let mut expected = rows[1].clone();
    expected.pc = 2;
    expected.kind = 1;
    assert_eq!(run(&query, &rows, 1), vec![expected.clone()]);
    let mut different = rows[1].clone();
    different.cursors[0][1] = 3;
    different.position = vec![4];
    let mut expected_different = different.clone();
    expected_different.pc = 2;
    expected_different.kind = 1;
    let mut more = rows.to_vec();
    more.push(different);
    assert_eq!(run(&query, &more, 1), vec![expected, expected_different]);
    let rows = rows.map(|mut s| {
        s.kind = 2;
        s
    });
    let expected = rows
        .iter()
        .map(|s| {
            let mut s = s.clone();
            s.kind = 3;
            s.pc = 2;
            s
        })
        .collect::<Vec<_>>();
    assert_eq!(run(&ordinary, &rows, 1), expected);
}

#[test]
fn successors_reconstruct_sibling_requirements_at_both_branch_destinations() {
    let p = program(vec![
        word(FORK, 0, 3),
        word(FIRST, 0, 0),
        word(EMIT, 0, 0),
        word(LAST, 0, 0),
        word(EMIT, 0, 0),
    ]);
    let input = state(0, 2);
    let mut first = input.clone();
    first.pc = 1;
    first.position = vec![0];
    let mut second = input.clone();
    second.pc = 3;
    second.position = vec![1];
    assert_eq!(run(&p, &[input], 1), vec![first, second]);
}

#[test]
fn unrelated_query_values_do_not_require_all_pairs_within_a_task() {
    let p = program(vec![
        word(REPEAT_BEGIN, 0, 9),
        word(REPEAT_END, 0, 9),
        word(EMIT, 0, 0),
    ])
    .with_tree_sitter_query_semantics()
    .unwrap();
    let rows = (0..1024)
        .map(|slot| {
            let mut s = state(0, 0);
            s.position = vec![slot];
            s.cursors = vec![[slot, 1, 0, 0]];
            s.captures = vec![[1, 0, 0, 7]; 20];
            s
        })
        .collect::<Vec<_>>();
    let expected = rows
        .iter()
        .map(|s| {
            let mut s = s.clone();
            s.pc = 2;
            s.kind = 1;
            s
        })
        .collect::<Vec<_>>();
    assert_eq!(run(&p, &rows, 1), expected);
}

#[test]
fn equal_hash_samples_do_not_merge_distinct_complete_values_or_capture_orders() {
    let p = Program::new(
        vec![
            word(REPEAT_BEGIN, 0, 9),
            word(REPEAT_END, 0, 9),
            word(EMIT, 0, 0),
        ],
        vec![b"first".to_vec(), b"second".to_vec()],
        vec![],
        vec![0, 1],
        vec![],
    )
    .unwrap()
    .with_tree_sitter_query_semantics()
    .unwrap();
    let mut base = state(0, 0);
    base.position = vec![8];
    base.cursors = (0..5).map(|slot| [slot, 1, 0, 0]).collect();
    base.captures = vec![[1, 0, 0, 7]; 5];
    let mut earliest = base.clone();
    earliest.position = vec![1];
    let mut cursor = base.clone();
    cursor.position = vec![2];
    cursor.cursors[1][1] = 2;
    let mut first_order = base.clone();
    first_order.position = vec![3];
    first_order.captures[1][1] = 1;
    let mut second_order = base.clone();
    second_order.position = vec![4];
    second_order.captures[3][1] = 1;
    // Equal lengths and equal first/middle/last samples force the same bucket.
    // Full equality must retain both label orders and the interior cursor change.
    let mut expected = vec![
        earliest.clone(),
        cursor.clone(),
        first_order.clone(),
        second_order.clone(),
    ];
    for result in &mut expected {
        result.pc = 2;
        result.kind = 1;
    }
    for rows in [
        vec![
            base.clone(),
            earliest.clone(),
            cursor.clone(),
            first_order.clone(),
            second_order.clone(),
        ],
        vec![second_order, first_order, cursor, earliest, base],
    ] {
        let mut actual = run(&p, &rows, 1);
        actual.sort_by(|a, b| a.position.cmp(&b.position));
        assert_eq!(actual, expected);
    }
}

#[test]
fn active_key_workspace_retains_all_keys_and_earliest_positions_without_power_of_two_slack() {
    active_key_workspace(false);
}

#[test]
fn ordinary_rows_do_not_expand_the_exact_query_key_domain() {
    active_key_workspace(true);
}

fn active_key_workspace(mixed: bool) {
    let tasks = 65_537_usize;
    let ordinary = program(vec![
        word(REPEAT_BEGIN, 0, 9),
        word(CAPTURE, 0, 0),
        word(REPEAT_END, 0, 9),
        word(EMIT, 0, 0),
    ]);
    let p = ordinary.clone().with_tree_sitter_query_semantics().unwrap();
    let programs = [p.clone(), ordinary];
    let source = Arc::new("# first\n".repeat(tasks));
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let native = StrDoc::new(source.as_str(), SupportLang::Python);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
    let doc = TapeDoc::restore(source, grammar, &image, &|| false).unwrap();
    let query_tasks = if mixed { 64 } else { tasks };
    let roots = vec![vec![
        (1..=query_tasks as u32).collect::<Vec<_>>(),
        ((query_tasks + 1) as u32..=tasks as u32).collect(),
    ]];
    let resident = ResidentInput::new(
        MetalGridInput::build(&programs, &[doc], &roots).unwrap(),
        None,
    )
    .unwrap();
    let plan = ExecutionPlan::new(&resident, &programs).unwrap();
    let mut rows = (0..tasks)
        .map(|task| {
            let mut row = state(3, if task < query_tasks { 1 } else { 3 });
            row.task = task as u32;
            row.node = task as u32 + 1;
            row.position = vec![9, task as u32];
            row.captures = vec![[task as u32 + 1, 0, task as u32 * 8, task as u32 * 8 + 7]];
            row
        })
        .collect::<Vec<_>>();
    let mut expected = rows.clone();
    for task in 0..64 {
        let mut earlier = rows[task].clone();
        earlier.position[0] = 1;
        expected[task] = earlier.clone();
        rows.push(earlier);
    }
    if mixed {
        for task in tasks - 64..tasks {
            let mut occurrence = rows[task].clone();
            occurrence.position[0] = 1;
            rows.push(occurrence.clone());
            expected.push(occurrence);
        }
        expected.sort_by_key(|row| row.task);
    }
    let _guard = metal_execution_guard().unwrap();
    let scope = super::metal_backend::launch::LaunchScope::enter();
    let device = Device::system_default().unwrap();
    let data = super::metal_register_execution::upload(&device, &resident).unwrap();
    let input = upload(&device, &rows, &p);
    let n = rows.len();
    write(&input.control.path_work, &vec![4_u32; n]);
    let mut columns = read::<u32>(&input.states.columns, n * COLUMNS);
    for (i, row) in rows.iter().enumerate() {
        columns[4 * n + i] = u32::from(mixed && row.task >= 64);
    }
    write(&input.states.columns, &columns);
    let mut ids = vec![u32::MAX; input.states.capacity * 4];
    for (i, slot) in ids[3 * input.states.capacity..][..n].iter_mut().enumerate() {
        *slot = i as u32;
    }
    let mut header = [0_u32; 17];
    header[3] = n as u32;
    for k in 0..4 {
        header[4 + k * 3] = header[k].div_ceil(256);
        header[5 + k * 3] = 1;
        header[6 + k * 3] = 1;
    }
    let ready = ReadyBatch {
        indices: MappedBuffer::from_slice(&device, &ids).unwrap(),
        counts: MappedBuffer::from_slice(&device, &header).unwrap(),
        capacity: input.states.capacity,
        generation: 8,
    };
    let source = SuccessorInput {
        resident: &resident,
        plan: &plan,
        data: &data,
        states: &input.states,
        values: &input.values,
        control: &input.control,
        ready: &ready,
    };
    let queue = device.new_command_queue();
    let prepared = metal_successor_prepare::prepare(&device, &queue, &source).unwrap();
    assert_eq!(prepared.check_completed().unwrap().states, n);
    let needs = prepared.comparison_needs().unwrap();
    assert!(needs.active && !needs.pending);
    assert_eq!(needs.active_states, Some(query_tasks + 64));
    // Completed producer counts authorize storage. Inconsistent counts or
    // flags must reject before a consumer can use them as an allocation proof.
    let total = prepared.expansion.total();
    let original =
        unsafe { std::slice::from_raw_parts(total.contents().cast::<u32>(), 4).to_vec() };
    for (keys, error) in [
        (0, Error::BackendFailure),
        (n as u32 + 1, Error::BackendFailure),
        (u32::MAX, Error::BackendCapacity),
    ] {
        unsafe { total.contents().cast::<u32>().add(3).write(keys) };
        assert!(matches!(prepared.comparison_needs(), Err(actual) if actual == error));
    }
    unsafe { total.contents().cast::<u32>().add(3).write(original[3]) };
    write(&prepared.status, &[0u32, u32::MAX, u32::MAX, 0]);
    assert!(matches!(
        prepared.comparison_needs(),
        Err(Error::BackendFailure)
    ));
    write(&prepared.status, &[0u32, u32::MAX, u32::MAX, 1]);
    let output = storage(&device, n, 2 * n + 32, 9);
    let allocations = super::metal_backend::BufferObservation::enter();
    let started = std::time::Instant::now();
    let command = queue.new_command_buffer();
    let counted = metal_successor::encode_counts(
        &device,
        command,
        &source,
        prepared,
        metal_successor::CountLimits {
            states: n,
            cursors: 0,
            repeats: 0,
            captures: n,
            digits: 2 * n,
        },
    )
    .unwrap();
    let bytes = allocations.live_bytes();
    let pending = metal_successor::encode_publish(
        command,
        &source,
        SuccessorOutput {
            states: &output.states,
            values: &output.values,
            control: &output.control,
        },
        counted,
    )
    .unwrap();
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    pending.check_completed().unwrap();
    let backend_seconds = started.elapsed().as_secs_f64();
    let mut actual = observe_programs(&output, &p, |task| u32::from(mixed && task >= 64));
    actual.sort_by_key(|row| row.task);
    assert_eq!(actual.len(), expected.len());
    let outputs = expected.len();
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert_eq!(
            actual, expected,
            "full active key and earliest complete position"
        );
    }
    let work = read::<u32>(&output.control.path_work, output.control.state_capacity);
    assert!(
        work[..outputs].iter().all(|&value| value == 4),
        "completed states must not replay their executed prefix"
    );
    assert!(work[outputs..].iter().all(|&value| value == u32::MAX));
    assert!(scope.snapshot().iter().all(|r| r.completed));
    eprintln!(
        "ACTIVE_KEY_WORKSPACE mixed={mixed} rows={n} tasks={tasks} outputs={outputs} bytes={bytes} backendSeconds={backend_seconds}"
    );
    let per_row = if mixed { 216 } else { 232 };
    assert!(
        bytes <= per_row * n + 512 * 1024,
        "batch-local exact keys must follow the relevant work domain: {bytes}"
    );
}

#[test]
fn gpu_counted_capacity_hands_complete_values_to_a_fresh_ready_batch_without_prefix_replay() {
    use super::metal_ready_batch::{self, ReadyInput};
    use super::metal_successor_prepare::{self, Capacity};
    let _guard = metal_execution_guard().unwrap();
    let scope = super::metal_backend::launch::LaunchScope::enter();
    let device = Device::system_default().unwrap();
    let p = program(vec![
        word(FORK, GREEDY, 2),
        word(EMIT, 0, 0),
        word(EMIT, 0, 0),
    ]);
    let (resident, plan) = fixture(&p, 1);
    let data = super::metal_register_execution::upload(&device, &resident).unwrap();
    let mut row = state(0, 2);
    row.position = vec![4, 7];
    row.captures = vec![[1, 0, 0, 7]; 128];
    let input = upload(&device, &[row.clone()], &p);
    let batch = ReadyBatch {
        indices: MappedBuffer::zeroed::<[u32; 4]>(&device, input.states.capacity).unwrap(),
        counts: MappedBuffer::from_slice(
            &device,
            &[1u32, 0, 0, 0, 1, 1, 1, 0, 1, 1, 0, 1, 1, 0, 1, 1, 0],
        )
        .unwrap(),
        capacity: input.states.capacity,
        generation: 8,
    };
    let make_input = || SuccessorInput {
        resident: &resident,
        plan: &plan,
        data: &data,
        states: &input.states,
        values: &input.values,
        control: &input.control,
        ready: &batch,
    };
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    let preparation =
        metal_successor_prepare::encode_inputs(&device, command, &make_input()).unwrap();
    assert_eq!(preparation.check_completed(), Err(Error::BackendFailure));
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    let command = queue.new_command_buffer();
    let preparation =
        metal_successor_prepare::encode_groups(&device, command, &make_input(), preparation)
            .unwrap();
    assert_eq!(preparation.check_completed(), Err(Error::BackendFailure));
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    assert_eq!(preparation.check_completed(), Ok(1));
    let command = queue.new_command_buffer();
    let preparation =
        metal_successor_prepare::encode_steps(&device, command, &make_input(), preparation)
            .unwrap();
    assert_eq!(preparation.check_completed(), Err(Error::BackendFailure));
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    let bounds = preparation.check_completed().unwrap();
    assert_eq!(
        bounds,
        Capacity {
            states: 2,
            cells: 256
        }
    );
    let output = storage(&device, bounds.states, bounds.cells, 9);
    let ready = ReadyBatch {
        indices: MappedBuffer::from_slice(&device, &vec![u32::MAX; bounds.states * 4]).unwrap(),
        counts: MappedBuffer::from_slice(&device, &[u32::MAX; 17]).unwrap(),
        capacity: bounds.states,
        generation: 9,
    };
    let command = queue.new_command_buffer();
    let pending = metal_successor::encode_prepared(
        &device,
        command,
        make_input(),
        SuccessorOutput {
            states: &output.states,
            values: &output.values,
            control: &output.control,
        },
        preparation,
    )
    .unwrap();
    let scheduled = metal_ready_batch::encode(
        &device,
        command,
        ReadyInput {
            resident: &resident,
            plan: &plan,
            data: &data,
            states: &output.states,
            values: &output.values,
        },
        &ready,
    )
    .unwrap();
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    pending.check_completed().unwrap();
    scheduled.check_completed().unwrap();
    let mut body = row.clone();
    body.pc = 1;
    body.position.push(0);
    let mut exit = row.clone();
    exit.pc = 2;
    exit.kind = 7;
    exit.position.push(1);
    exit.body = body.position.clone();
    exit.offer = row.position;
    assert_eq!(observe(&output, &p), vec![body.clone(), exit.clone()]);
    assert_eq!(
        read::<u32>(&ready.counts, 16),
        vec![1, 0, 1, 0, 1, 1, 1, 0, 1, 1, 1, 1, 1, 0, 1, 1]
    );
    assert_eq!(
        read::<u32>(&ready.indices, 8),
        vec![
            0,
            u32::MAX,
            u32::MAX,
            u32::MAX,
            1,
            u32::MAX,
            u32::MAX,
            u32::MAX
        ]
    );
    // Reuse the GPU output directly. No host reconstruction or fresh upload can
    // hide an inconsistent position span, arena header, or continuation kind.
    let terminal = storage(&device, 2, 256, 10);
    let command = queue.new_command_buffer();
    let advanced = metal_successor::encode(
        &device,
        &queue,
        command,
        SuccessorInput {
            resident: &resident,
            plan: &plan,
            data: &data,
            states: &output.states,
            values: &output.values,
            control: &output.control,
            ready: &ready,
        },
        SuccessorOutput {
            states: &terminal.states,
            values: &terminal.values,
            control: &terminal.control,
        },
    )
    .unwrap();
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    advanced.check_completed().unwrap();
    body.kind = 3;
    exit.kind = 2;
    exit.body.clear();
    exit.offer.clear();
    assert_eq!(observe(&terminal, &p), vec![body, exit]);
    let trace = scope.snapshot();
    assert!(trace.iter().all(|r| r.completed));
    assert_eq!(
        trace
            .iter()
            .filter(|r| r.kernel == "successor_prefix")
            .count(),
        2
    );
    assert_eq!(
        trace
            .iter()
            .map(|r| r.command)
            .collect::<BTreeSet<_>>()
            .len(),
        8
    );
    let groups = trace
        .iter()
        .filter(|record| record.kernel == "successor_group_items")
        .collect::<Vec<_>>();
    let prefixes = trace
        .iter()
        .filter(|record| record.kernel == "successor_prefix")
        .collect::<Vec<_>>();
    assert_eq!(groups.len(), 2);
    for (group, prefix) in groups.into_iter().zip(prefixes) {
        assert_ne!(
            group.command, prefix.command,
            "completed input metadata must retire before finite frame allocation"
        );
    }
    assert!(trace.len() <= 100);
}

#[test]
fn typed_values_cannot_reference_a_different_resident_file() {
    let p = program(vec![word(EMIT, 0, 0)]);
    for capture in [false, true] {
        let fixture = fixture_sources(&p, 1, &["# first\n".into(), "# foreign\n".into()]);
        let foreign = fixture.0.input.packed.files[1].node_start + 1;
        let mut input = state(0, 2);
        if capture {
            input.captures = vec![[foreign, 0, 0, 9]];
        } else {
            input.cursors = vec![[0, foreign, 0, 0]];
        }
        assert_eq!(
            attempt_resident(&p, &[input], 1, fixture, |_, _, _| {}),
            Err(Error::BackendFailure),
            "foreign capture={capture} must fail before payload publication"
        );
    }
}

#[test]
fn named_navigation_filters_anonymous_nodes_and_numbers_only_surviving_choices() {
    // Preorder: module, expression_statement, assignment, identifier, '=', integer.
    for op in [CHILD, DESCEND, FOLLOWING] {
        for mode in [0, NAMED] {
            let p = program(vec![word(op, mode, 0), word(EMIT, 0, 0)]);
            let fixture = fixture_sources(&p, 1, &["first = 1\n".into()]);
            let mut input = state(0, 2);
            input.node = if op == FOLLOWING { 3 } else { 2 };
            input.position = vec![5, 8];
            let nodes = match (op, mode) {
                (FOLLOWING, 0) => vec![4, 5],
                (FOLLOWING, _) => vec![5],
                (_, 0) => vec![3, 4, 5],
                _ => vec![3, 5],
            };
            let expected = nodes
                .into_iter()
                .enumerate()
                .map(|(choice, node)| {
                    let mut child = input.clone();
                    child.pc = 1;
                    child.node = node;
                    child.position.push(choice as u32);
                    child
                })
                .collect::<Vec<_>>();
            assert_eq!(
                attempt_resident(&p, &[input], 1, fixture, |_, _, _| {}).unwrap(),
                expected
            );
        }
    }
}

#[test]
fn duplicate_query_forks_do_not_leave_an_extra_greedy_exit_after_body_admission() {
    let p = program(vec![
        word(REPEAT_BEGIN, 0, 9),
        word(REPEAT_BUMP, 0, 9),
        word(FORK, GREEDY, 8),
        word(NEXT, 0, 0),
        word(FORK, 0, 6),
        word(FLAGS, 0, 0),
        word(REPEAT_BUMP, 0, 9),
        word(GOTO, 0, 2),
        word(REPEAT_END, 0, 9),
        word(EMIT, 0, 0),
    ])
    .with_tree_sitter_query_semantics()
    .unwrap();
    let mut later = state(2, 0);
    later.repeats = vec![[0, 9, 0, 0]];
    later.position = vec![2];
    let mut first = state(2, 0);
    first.repeats = later.repeats.clone();
    first.position = vec![1];
    let mut body = first.clone();
    body.pc = 3;
    body.position.push(0);
    let mut exit = first.clone();
    exit.pc = 8;
    exit.kind = 6;
    exit.position.push(1);
    exit.body = body.position.clone();
    exit.offer = first.position.clone();
    assert_eq!(run(&p, &[later, first], 1), vec![body, exit]);
}

#[test]
fn full_branch_positions_preserve_logical_order_independently_of_arrival_order() {
    let p = program(vec![word(FORK, 0, 2), word(EMIT, 0, 0), word(EMIT, 0, 0)]);
    let mut later = state(0, 2);
    later.position = vec![2];
    let mut earlier = state(0, 2);
    earlier.position = vec![1];
    let mut expected = Vec::new();
    for parent in [&earlier, &later] {
        for choice in 0..2 {
            let mut child = parent.clone();
            child.pc = 1 + choice;
            child.position.push(choice);
            expected.push(child);
        }
    }
    for rows in [[later.clone(), earlier.clone()], [earlier, later]] {
        let mut actual = run(&p, &rows, 1);
        // Observation orders full values by the public control contract. The
        // production representation deliberately contains no scalar ordinal.
        actual.sort_by(|a, b| a.position.cmp(&b.position));
        assert_eq!(
            actual, expected,
            "all ordinary occurrences and their logical order"
        );
    }
}

#[test]
fn ordinary_work_budget_edges_preserve_the_last_executed_capture_and_exact_failure_pc() {
    let p = program(vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)]);
    for remaining in [2, 1, 0] {
        let input = state(0, 2);
        let mut expected = input.clone();
        expected.pc = u32::from(remaining > 0);
        expected.kind = if remaining == 2 { 3 } else { 13 };
        if remaining > 0 {
            expected.captures = vec![[1, 0, 0, 7]];
        }
        let mut output_work = None;
        let actual = attempt(&p, &[input], 1, |source, output, _| {
            write(
                &source.control.path_work,
                &[(MAX_STEPS_PER_PATH - remaining) as u32],
            );
            output_work = Some(output.control.path_work.buffer.clone());
        });
        assert_eq!(actual, Ok(vec![expected]), "remaining work={remaining}");
        let work = unsafe { output_work.unwrap().contents().cast::<u32>().read() };
        assert_eq!(
            work,
            (MAX_STEPS_PER_PATH + usize::from(remaining < 2)) as u32
        );
    }
    let failed = state(0, 13);
    assert_eq!(
        attempt(&p, std::slice::from_ref(&failed), 1, |_, _, _| {}),
        Err(Error::BackendFailure)
    );
    assert_eq!(
        attempt(&p, std::slice::from_ref(&failed), 1, |source, _, _| write(
            &source.control.path_work,
            &[(MAX_STEPS_PER_PATH + 1) as u32]
        )),
        Ok(vec![failed])
    );
}

#[test]
fn successor_input_storage_preserves_every_completed_row_with_compact_validation() {
    use super::metal_successor_prepare;
    let tasks = 16_384;
    let p = program(vec![word(EMIT, 0, 0)]);
    let rows = (0..tasks)
        .map(|task| {
            let mut row = state(0, 3);
            row.task = task as u32;
            row.captures = vec![[1, 0, 0, 7]];
            row.position = vec![task as u32];
            row
        })
        .collect::<Vec<_>>();
    assert_eq!(run(&p, &rows, tasks), rows);

    let _guard = metal_execution_guard().unwrap();
    let scope = super::metal_backend::launch::LaunchScope::enter();
    let device = Device::system_default().unwrap();
    let (resident, plan) = fixture(&p, tasks);
    let data = super::metal_register_execution::upload(&device, &resident).unwrap();
    let input = upload(&device, &rows, &p);
    let mut ids = vec![u32::MAX; input.states.capacity * 4];
    for (i, slot) in ids[3 * input.states.capacity..][..tasks]
        .iter_mut()
        .enumerate()
    {
        *slot = i as u32;
    }
    let batch = ReadyBatch {
        indices: MappedBuffer::from_slice(&device, &ids).unwrap(),
        counts: MappedBuffer::from_slice(
            &device,
            &[
                0,
                0,
                0,
                tasks as u32,
                0,
                1,
                1,
                0,
                1,
                1,
                0,
                1,
                1,
                tasks.div_ceil(256) as u32,
                1,
                1,
                0,
            ],
        )
        .unwrap(),
        capacity: input.states.capacity,
        generation: input.states.generation,
    };
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    let allocations = super::metal_backend::BufferObservation::enter();
    let counted = metal_successor_prepare::encode_inputs(
        &device,
        command,
        &SuccessorInput {
            resident: &resident,
            plan: &plan,
            data: &data,
            states: &input.states,
            values: &input.values,
            control: &input.control,
            ready: &batch,
        },
    )
    .unwrap();
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    assert_eq!(counted.check_completed(), Ok(tasks));
    let bytes = allocations.live_bytes();
    eprintln!("TEMPO_SUCCESSOR_INPUT_STORAGE rows={tasks} bytes={bytes}");
    assert!(
        bytes <= 44 * input.states.capacity + 192 * 1024,
        "successor input preparation must fit its per-active-row metadata allowance: {bytes}"
    );
    assert!(scope.snapshot().iter().all(|record| record.completed));
}

#[test]
fn successor_input_validation_rejects_corruption_after_the_first_cooperative_tile() {
    let p = program(vec![word(EMIT, 0, 0)]);
    let mut row = state(0, 3);
    row.captures = vec![[1, 0, 0, 7]; 513];
    row.cursors = (0..513).map(|slot| [slot, 1, 0, 0]).collect();
    row.repeats = (0..513).map(|slot| [slot, 0, 0, 0]).collect();
    row.position = vec![17; 513];
    assert_eq!(run(&p, std::slice::from_ref(&row), 1), vec![row.clone()]);
    for arena in 0..3 {
        let result = attempt(&p, std::slice::from_ref(&row), 1, |input, _, _| {
            let buffer = match arena {
                0 => &input.values.cursors,
                1 => &input.values.repeats,
                _ => &input.values.captures,
            };
            let mut cells = read::<[u32; 4]>(buffer, 513);
            cells[512][2] = u32::MAX;
            write(buffer, &cells);
        });
        assert_eq!(result, Err(Error::BackendFailure), "arena {arena}");
    }
}

#[test]
fn successor_prefix_storage_tracks_its_finite_edit_shape() {
    use super::metal_successor_prepare;
    let tasks = 16_384;
    let p = program(vec![
        word(SAVE_CURSOR, 0, 7),
        word(CAPTURE, 0, 0),
        word(EMIT, 0, 0),
    ]);
    let rows = (0..tasks)
        .map(|task| {
            let mut row = state(0, 2);
            row.task = task as u32;
            row.captures = vec![[1, 0, 0, 7]];
            row.position = vec![task as u32];
            row
        })
        .collect::<Vec<_>>();
    let expected = rows
        .iter()
        .cloned()
        .map(|mut row| {
            row.pc = 2;
            row.kind = 3;
            row.cursors = vec![[7, 1, 0, 0]];
            row.captures.push([1, 0, 0, 7]);
            row
        })
        .collect::<Vec<_>>();
    assert_eq!(run(&p, &rows, tasks), expected);

    let _guard = metal_execution_guard().unwrap();
    let scope = super::metal_backend::launch::LaunchScope::enter();
    let device = Device::system_default().unwrap();
    let (resident, plan) = fixture(&p, tasks);
    let data = super::metal_register_execution::upload(&device, &resident).unwrap();
    let input = upload(&device, &rows, &p);
    let mut ids = vec![u32::MAX; input.states.capacity * 4];
    for (i, slot) in ids[..tasks].iter_mut().enumerate() {
        *slot = i as u32;
    }
    let batch = ReadyBatch {
        indices: MappedBuffer::from_slice(&device, &ids).unwrap(),
        counts: MappedBuffer::from_slice(
            &device,
            &[
                tasks as u32,
                0,
                0,
                0,
                tasks.div_ceil(256) as u32,
                1,
                1,
                0,
                1,
                1,
                0,
                1,
                1,
                0,
                1,
                1,
                0,
            ],
        )
        .unwrap(),
        capacity: input.states.capacity,
        generation: input.states.generation,
    };
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    let counted = metal_successor_prepare::encode_inputs(
        &device,
        command,
        &SuccessorInput {
            resident: &resident,
            plan: &plan,
            data: &data,
            states: &input.states,
            values: &input.values,
            control: &input.control,
            ready: &batch,
        },
    )
    .unwrap();
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    assert_eq!(counted.check_completed(), Ok(tasks));
    let allocations = super::metal_backend::BufferObservation::enter();
    let command = queue.new_command_buffer();
    let counted = metal_successor_prepare::encode_groups(
        &device,
        command,
        &SuccessorInput {
            resident: &resident,
            plan: &plan,
            data: &data,
            states: &input.states,
            values: &input.values,
            control: &input.control,
            ready: &batch,
        },
        counted,
    )
    .unwrap();
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    assert_eq!(counted.check_completed(), Ok(tasks));
    let command = queue.new_command_buffer();
    let prepared = metal_successor_prepare::encode_steps(
        &device,
        command,
        &SuccessorInput {
            resident: &resident,
            plan: &plan,
            data: &data,
            states: &input.states,
            values: &input.values,
            control: &input.control,
            ready: &batch,
        },
        counted,
    )
    .unwrap();
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    assert_eq!(
        prepared.check_completed().unwrap(),
        metal_successor_prepare::Capacity {
            states: tasks,
            cells: 2 * tasks,
        }
    );
    let bytes = allocations.live_bytes();
    eprintln!("TEMPO_SUCCESSOR_PREFIX_STORAGE rows={tasks} bytes={bytes}");
    assert!(
        bytes <= 128 * input.states.capacity + 256 * 1024,
        "finite prefix storage must follow its cursor, capture, repeat, and branch edits: {bytes}"
    );
    assert!(scope.snapshot().iter().all(|record| record.completed));
}

// This owner follows the production completed-count handoff. The existing
// same-command tests retain their separate encoder composition coverage.
fn run_completed_publication(
    p: &Program,
    rows: &[State],
    tasks: usize,
    after_publication: impl FnOnce(&Storage),
) -> Result<Vec<State>, Error> {
    run_publication(p, rows, tasks, after_publication, false)
}

fn run_publication(
    p: &Program,
    rows: &[State],
    tasks: usize,
    after_publication: impl FnOnce(&Storage),
    private_frames: bool,
) -> Result<Vec<State>, Error> {
    let _guard = metal_execution_guard().unwrap();
    let scope = super::metal_backend::launch::LaunchScope::enter();
    let device = Device::system_default().unwrap();
    let (resident, plan) = fixture(p, tasks);
    let data = super::metal_register_execution::upload(&device, &resident)?;
    let input = upload(&device, rows, p);
    let ready = ReadyBatch {
        indices: MappedBuffer::zeroed::<[u32; 4]>(&device, input.states.capacity)?,
        counts: MappedBuffer::zeroed::<u32>(&device, 17)?,
        capacity: input.states.capacity,
        generation: input.states.generation,
    };
    let queue = device.new_command_queue();
    metal::objc::rc::autoreleasepool(|| {
        let command = queue.new_command_buffer();
        let pending = super::metal_ready_batch::encode(
            &device,
            command,
            super::metal_ready_batch::ReadyInput {
                resident: &resident,
                plan: &plan,
                data: &data,
                states: &input.states,
                values: &input.values,
            },
            &ready,
        )?;
        commit_checked(command)?;
        wait_checked(command)?;
        pending.check_completed()
    })?;
    let source = SuccessorInput {
        resident: &resident,
        plan: &plan,
        data: &data,
        states: &input.states,
        values: &input.values,
        control: &input.control,
        ready: &ready,
    };
    let prepared = if private_frames {
        metal_successor_prepare::prepare_register(&device, &queue, &source)?
    } else {
        metal_successor_prepare::prepare(&device, &queue, &source)?
    };
    let output = storage(&device, rows.len() * 4 + 7, rows.len() * 128 + 64, 9);
    let counted = metal::objc::rc::autoreleasepool(|| {
        let command = queue.new_command_buffer();
        let counted = metal_successor::encode_counts(
            &device,
            command,
            &source,
            prepared,
            metal_successor::CountLimits {
                states: output.states.capacity,
                cursors: output.values.cursor_capacity,
                repeats: output.values.repeat_capacity,
                captures: output.values.capture_capacity,
                digits: output.control.digit_capacity,
            },
        )?;
        commit_checked(command)?;
        wait_checked(command)?;
        counted.check_completed()?;
        Ok::<_, Error>(counted)
    })?;
    let publication = counted.into_publication(source)?;
    metal::objc::rc::autoreleasepool(|| {
        let command = queue.new_command_buffer();
        let pending = publication.encode(
            command,
            SuccessorOutput {
                states: &output.states,
                values: &output.values,
                control: &output.control,
            },
        )?;
        commit_checked(command)?;
        wait_checked(command)?;
        after_publication(&output);
        pending.check_completed()
    })?;
    assert!(scope.snapshot().iter().all(|record| record.completed));
    Ok(observe(&output, p))
}

#[test]
fn completed_publication_preserves_occurrences_exact_query_winners_and_pending_task_owners() {
    let p = program(vec![word(FORK, 0, 2), word(EMIT, 0, 0), word(EMIT, 0, 0)]);
    let mut input = state(0, 2);
    input.position = vec![7];
    let expected = (0..2)
        .map(|choice| {
            let mut output = input.clone();
            output.pc = choice + 1;
            output.position.push(choice);
            output
        })
        .collect::<Vec<_>>();
    assert_eq!(
        run_completed_publication(&p, &[input], 1, |_| {}),
        Ok(expected)
    );

    let p = program(vec![
        word(REPEAT_BEGIN, 0, 0),
        word(CAPTURE, 0, 0),
        word(EMIT, 0, 0),
    ])
    .with_tree_sitter_query_semantics()
    .unwrap();
    let mut early = state(2, 1);
    early.repeats = vec![[0, 0, 0, 0]];
    early.captures = vec![[1, 0, 0, 7]];
    early.position = vec![1];
    let mut late = early.clone();
    late.position = vec![3];
    assert_eq!(
        run_completed_publication(&p, &[late, early.clone()], 1, |_| {}),
        Ok(vec![early])
    );

    let p = program(vec![
        word(FORK, GREEDY, 3),
        word(STOP, 0, 0),
        word(STOP, 0, 0),
        word(EMIT, 0, 0),
    ]);
    let mut rows = vec![];
    let mut expected = vec![];
    for task in [2, 0, 1] {
        let mut body = state(1, 2);
        body.task = task;
        body.node = task + 1;
        body.position = vec![0];
        let mut pending = body.clone();
        pending.pc = 3;
        pending.kind = 7;
        pending.position = vec![1];
        pending.body = vec![0];
        let mut retired = pending.clone();
        retired.kind = 2;
        retired.body.clear();
        rows.extend([body, pending]);
        expected.push(retired);
    }
    assert_eq!(
        run_completed_publication(&p, &rows, 3, |_| {}),
        Ok(expected)
    );
}

#[test]
fn completed_publication_rejects_inconsistent_lengths_instead_of_accepting_a_partial_result() {
    let p = program(vec![
        word(SAVE_CURSOR, 0, 0),
        word(REPEAT_BEGIN, 0, 9),
        word(CAPTURE, 0, 0),
        word(EMIT, 0, 0),
    ]);
    let mut input = state(0, 2);
    input.position = vec![7];
    let mut expected = input.clone();
    expected.pc = 3;
    expected.kind = 3;
    expected.cursors = vec![[0, 1, 0, 0]];
    expected.repeats = vec![[0, 9, 0, 0]];
    expected.captures = vec![[1, 0, 0, 7]];
    for defect in 0..8 {
        let actual =
            run_completed_publication(&p, std::slice::from_ref(&input), 1, |output| match defect {
                0 => {}
                1 => write(&output.states.counts, &[0u32, 1, 0, 1, 1]),
                2 => write(&output.states.counts, &[1u32, 0, 1, 1, 1]),
                3 => write(&output.values.counts, &[0u32, 1]),
                4 => write(&output.values.counts, &[1u32, 0]),
                5 => write(&output.control.count, &[0u32]),
                6 => write(&output.states.counts, &[1u32, 1, 0, 1, 1]),
                7 => write(&output.values.counts, &[u32::MAX, 1]),
                _ => unreachable!(),
            });
        if defect == 0 {
            assert_eq!(actual, Ok(vec![expected.clone()]));
        } else {
            assert_eq!(actual, Err(Error::BackendFailure), "defect={defect}");
        }
    }
}

#[test]
fn private_finite_frames_preserve_full_typed_edits_branch_positions_and_executed_work() {
    let p = program(vec![
        word(SAVE_CURSOR, 0, 5),
        word(NEXT, 0, 0),
        word(SAVE_CURSOR, 0, 2),
        word(REPEAT_BEGIN, 0, 9),
        word(REPEAT_BUMP, 0, 9),
        word(CAPTURE, 0, 0),
        word(FORK, 0, 9),
        word(EMIT, 0, 0),
        word(STOP, 0, 0),
        word(EMIT, 0, 0),
    ]);
    let mut input = state(0, 2);
    input.cursors = vec![[0, 3, 0, 0], [7, 1, 0, 0]];
    input.repeats = vec![[0, 4, 0, 0]];
    input.captures = vec![[1, 0, 0, 7]];
    input.position = vec![4, 7];
    let mut first = input.clone();
    first.pc = 7;
    first.node = 2;
    first.cursors = vec![[0, 3, 0, 0], [2, 2, 0, 0]];
    first.repeats.push([1, 9, 0, 0]);
    first.captures.push([2, 0, 8, 16]);
    first.position.push(0);
    let mut second = first.clone();
    second.pc = 9;
    *second.position.last_mut().unwrap() = 1;
    let actual = run_publication(
        &p,
        &[input],
        1,
        |output| {
            let work = read::<u32>(&output.control.path_work, output.control.state_capacity);
            assert_eq!(&work[..2], &[7, 7]);
        },
        true,
    )
    .unwrap();
    assert_eq!(actual, vec![first, second]);
}

#[test]
fn private_finite_frames_keep_operand_failures_before_later_rejection_and_allow_empty_output() {
    let p = program(vec![
        word(REPEAT_BEGIN, 0, 0),
        word(FORK, 0, 6),
        word(RESTORE_CURSOR, 0, 0),
        word(FORK, 0, 5),
        word(EMIT, 0, 0),
        word(EMIT, 0, 0),
        word(EMIT, 0, 0),
    ])
    .with_tree_sitter_query_semantics()
    .unwrap();
    let mut input = state(0, 0);
    input.position = vec![9];
    let mut failed = state(2, 8);
    failed.repeats = vec![[0, 0, 0, 0]];
    failed.position = vec![9, 0];
    let mut complete = state(6, 1);
    complete.repeats = vec![[0, 0, 0, 0]];
    complete.position = vec![9, 1];
    assert_eq!(
        run_publication(&p, &[input], 1, |_| {}, true),
        Ok(vec![failed, complete])
    );
    let p = program(vec![word(KIND, 0, 0), word(EMIT, 0, 0)]);
    assert_eq!(
        run_publication(&p, &[state(0, 2)], 1, |_| {}, true),
        Ok(vec![])
    );
}

#[test]
fn private_finite_frames_preserve_greedy_body_and_pending_owner_positions() {
    let p = program(vec![
        word(FORK, GREEDY, 2),
        word(EMIT, 0, 0),
        word(EMIT, 0, 0),
    ]);
    let mut input = state(0, 2);
    input.position = vec![3, 5];
    input.captures = vec![[1, 0, 0, 7]];
    let mut body = input.clone();
    body.pc = 1;
    body.position.push(0);
    let mut exit = input.clone();
    exit.pc = 2;
    exit.kind = 7;
    exit.position.push(1);
    exit.body = body.position.clone();
    exit.offer = input.position.clone();
    assert_eq!(
        run_publication(&p, &[input], 1, |_| {}, true),
        Ok(vec![body, exit])
    );
}

fn run_view(
    p: &Program,
    rows: &[State],
    tasks: usize,
    inspect: impl FnOnce(&Storage),
) -> Result<Vec<State>, Error> {
    run_view_with_work(p, rows, tasks, &vec![0; rows.len()], inspect)
}

fn run_view_with_work(
    p: &Program,
    rows: &[State],
    tasks: usize,
    work: &[u32],
    inspect: impl FnOnce(&Storage),
) -> Result<Vec<State>, Error> {
    let regular = run_view_with_producer(p, rows, tasks, work, false, |_| {});
    let compact = run_view_with_producer(p, rows, tasks, work, true, inspect);
    assert_eq!(regular, compact);
    compact
}
fn run_view_with_producer(
    p: &Program,
    rows: &[State],
    tasks: usize,
    work: &[u32],
    private_frames: bool,
    inspect: impl FnOnce(&Storage),
) -> Result<Vec<State>, Error> {
    assert_eq!(work.len(), rows.len());
    let _guard = metal_execution_guard().unwrap();
    let scope = super::metal_backend::launch::LaunchScope::enter();
    let device = Device::system_default().unwrap();
    let (resident, plan) = fixture(p, tasks);
    let data = super::metal_register_execution::upload(&device, &resident)?;
    let input = upload(&device, rows, p);
    write(&input.control.path_work, work);
    let queue = device.new_command_queue();
    let output = super::metal_dynamic_execution::execute_view_for_test(
        &device,
        &queue,
        &resident,
        &plan,
        &data,
        super::metal_dynamic_execution::ViewTestInput {
            private_frames,
            states: input.states,
            values: input.values,
            control: input.control,
        },
    )?;
    let output = Storage {
        states: output.states,
        values: output.values,
        control: output.control,
    };
    inspect(&output);
    assert!(scope.snapshot().iter().all(|record| record.completed));
    Ok(observe(&output, p))
}

#[test]
fn successor_views_preserve_typed_edits_work_and_same_pc_branch_occurrences() {
    let p = program(vec![
        word(SAVE_CURSOR, 0, 5),
        word(NEXT, 0, 0),
        word(SAVE_CURSOR, 0, 2),
        word(REPEAT_BEGIN, 0, 9),
        word(REPEAT_BUMP, 0, 9),
        word(CAPTURE, 0, 0),
        word(FORK, 0, 9),
        word(EMIT, 0, 0),
        word(STOP, 0, 0),
        word(EMIT, 0, 0),
    ]);
    let mut input = state(0, 2);
    input.cursors = vec![[0, 3, 0, 0], [7, 1, 0, 0]];
    input.repeats = vec![[0, 4, 0, 0]];
    input.captures = vec![[1, 0, 0, 7]];
    input.position = vec![4, 7];
    let mut first = input.clone();
    first.pc = 7;
    first.node = 2;
    first.cursors = vec![[0, 3, 0, 0], [2, 2, 0, 0]];
    first.repeats.push([1, 9, 0, 0]);
    first.captures.push([2, 0, 8, 16]);
    first.position.push(0);
    let mut second = first.clone();
    second.pc = 9;
    *second.position.last_mut().unwrap() = 1;
    assert_eq!(
        run_view(&p, &[input], 1, |output| assert_eq!(
            read::<u32>(&output.control.path_work, 2),
            vec![7, 7]
        )),
        Ok(vec![first, second])
    );
    let p = program(vec![word(FORK, 0, 1), word(EMIT, 0, 0)]);
    let mut input = state(0, 2);
    input.position = vec![7];
    let mut first = input.clone();
    first.pc = 1;
    first.position.push(0);
    let mut second = first.clone();
    *second.position.last_mut().unwrap() = 1;
    assert_eq!(run_view(&p, &[input], 1, |_| {}), Ok(vec![first, second]));
}

#[test]
fn successor_views_preserve_full_capture_identity_stable_ties_and_earliest_complete_positions() {
    let p = program(vec![
        word(REPEAT_BEGIN, 0, 0),
        word(CAPTURE, 0, 0),
        word(EMIT, 0, 0),
    ])
    .with_tree_sitter_query_semantics()
    .unwrap();
    let mut later = state(1, 0);
    later.repeats = vec![[0, 0, 0, 0]];
    later.captures = vec![[1, 0, 0, 7], [1, 0, 0, 7]];
    later.position = vec![9, 1];
    let mut early = later.clone();
    early.position = vec![9];
    let mut other = early.clone();
    other.captures.pop();
    other.position = vec![1];
    let mut expected_early = early.clone();
    expected_early.pc = 2;
    expected_early.kind = 1;
    expected_early.captures.push([1, 0, 0, 7]);
    let mut expected_other = other.clone();
    expected_other.pc = 2;
    expected_other.kind = 1;
    expected_other.captures.push([1, 0, 0, 7]);
    assert_eq!(
        run_view(&p, &[later, early, other], 1, |_| {}),
        Ok(vec![expected_early, expected_other])
    );
    let p = program(vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)]);
    let mut input = state(0, 2);
    input.node = 2;
    input.captures = vec![[1, 0, 0, 7], [3, 0, 17, 24]];
    let mut expected = input.clone();
    expected.pc = 1;
    expected.kind = 3;
    expected.captures.insert(1, [2, 0, 8, 16]);
    assert_eq!(run_view(&p, &[input], 1, |_| {}), Ok(vec![expected]));
}

#[test]
fn successor_views_preserve_failures_and_reject_pending_ownership_instead_of_losing_it() {
    let p = program(vec![word(RESTORE_CURSOR, 0, 7), word(EMIT, 0, 0)]);
    let mut expected = state(0, 9);
    expected.position = vec![3];
    let mut input = state(0, 2);
    input.position = vec![3];
    assert_eq!(run_view(&p, &[input], 1, |_| {}), Ok(vec![expected]));
    let p = program(vec![word(KIND, 0, 0), word(EMIT, 0, 0)]);
    assert_eq!(run_view(&p, &[state(0, 2)], 1, |_| {}), Ok(vec![]));
    let p = program(vec![
        word(FORK, GREEDY, 2),
        word(EMIT, 0, 0),
        word(EMIT, 0, 0),
    ]);
    assert_eq!(
        run_view(&p, &[state(0, 2)], 1, |_| {}),
        Err(Error::BackendFailure)
    );
}

#[test]
fn successor_views_compare_interior_capture_identity_and_preserve_query_tie_order() {
    let p = Program::new(
        vec![
            word(REPEAT_BEGIN, 0, 0),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 0),
        ],
        vec![b"same".to_vec(), b"other".to_vec()],
        vec![],
        vec![0, 1],
        vec![],
    )
    .unwrap()
    .with_tree_sitter_query_semantics()
    .unwrap();
    let mut later = state(1, 0);
    later.repeats = vec![[0, 0, 0, 0]];
    later.captures = vec![
        [1, 0, 0, 7],
        [1, 1, 0, 7],
        [1, 0, 0, 7],
        [1, 0, 0, 7],
        [1, 0, 0, 7],
        [1, 0, 0, 7],
    ];
    later.position = vec![4, 1];
    let mut earlier = later.clone();
    earlier.position = vec![4];
    let mut different = earlier.clone();
    different.captures[1][1] = 0;
    different.position = vec![3];
    let mut first = earlier.clone();
    first.pc = 2;
    first.kind = 1;
    first.captures.push([1, 0, 0, 7]);
    let mut second = different.clone();
    second.pc = 2;
    second.kind = 1;
    second.captures.push([1, 0, 0, 7]);
    assert_eq!(
        run_view(&p, &[later, earlier, different], 1, |_| {}),
        Ok(vec![first, second])
    );
}

#[test]
fn successor_guard_work_follows_full_key_admission_and_preserves_the_failed_instruction() {
    let p = program(vec![
        word(REPEAT_BEGIN, 0, 9),
        word(GOTO, 0, 3),
        word(STOP, 0, 0),
        word(SIZE, 0, 0),
        word(EMIT, 0, 0),
    ])
    .with_tree_sitter_query_semantics()
    .unwrap();
    let mut earlier = state(1, 0);
    earlier.repeats = vec![[0, 9, 0, 0]];
    earlier.position = vec![0];
    let mut later = earlier.clone();
    later.position = vec![1];
    let high = MAX_STEPS_PER_PATH as u32 - 1;
    assert_eq!(
        run_view_with_work(&p, &[earlier.clone(), later.clone()], 1, &[0, high], |_| {}),
        Ok(vec![])
    );
    let mut failed = earlier.clone();
    failed.pc = 3;
    failed.kind = 12;
    assert_eq!(
        run_view_with_work(&p, &[earlier, later], 1, &[high, 0], |output| {
            assert_eq!(
                read::<u32>(&output.control.path_work, 1),
                vec![MAX_STEPS_PER_PATH as u32 + 1]
            );
        }),
        Ok(vec![failed])
    );
}

#[test]
fn successor_guards_stop_at_readiness_barriers_and_preserve_query_greedy_programs() {
    let mut words = vec![
        word(REPEAT_BEGIN, 0, 0),
        word(REPEAT_BUMP, 0, 0),
        word(FORK, 0, 6),
        word(NEXT, 0, 0),
        word(REPEAT_BUMP, 0, 0),
        word(GOTO, 0, 2),
        word(REPEAT_END, 0, 0),
        word(FORK, 0, 10),
        word(FLAGS, 0, 0),
        word(GOTO, 0, 10),
        word(SIZE, 0, 0),
        word(EMIT, 0, 0),
    ];
    let p = program(words.clone())
        .with_tree_sitter_query_semantics()
        .unwrap();
    let mut input = state(9, 0);
    input.position = vec![2];
    let mut expected = input.clone();
    expected.pc = 10;
    assert_eq!(
        run_view(&p, &[input], 1, |output| assert_eq!(
            read::<u32>(&output.control.path_work, 1),
            vec![1]
        )),
        Ok(vec![expected])
    );
    words[2] = word(FORK, GREEDY, 6);
    let p = program(words).with_tree_sitter_query_semantics().unwrap();
    let mut input = state(9, 0);
    input.position = vec![2];
    let mut expected = input.clone();
    expected.pc = 10;
    assert_eq!(
        run_view(&p, &[input], 1, |output| assert_eq!(
            read::<u32>(&output.control.path_work, 1),
            vec![1]
        )),
        Ok(vec![expected])
    );
}

#[test]
fn successor_guard_units_resume_at_the_first_unexecuted_guard_with_exact_work() {
    let mut words = vec![word(GOTO, 0, 2), word(STOP, 0, 0)];
    words.extend(vec![word(FLAGS, 0, 0); 9]);
    words.push(word(EMIT, 0, 0));
    let p = program(words);
    let mut input = state(0, 2);
    input.position = vec![3];
    let mut expected = input.clone();
    expected.pc = 10;
    assert_eq!(
        run_view(&p, &[input], 1, |output| assert_eq!(
            read::<u32>(&output.control.path_work, 1),
            vec![9]
        )),
        Ok(vec![expected])
    );
}

#[test]
fn successor_query_views_insert_new_capture_before_the_saved_capture() {
    let p = program(vec![
        word(REPEAT_BEGIN, 0, 0),
        word(CAPTURE, 0, 0),
        word(EMIT, 0, 0),
    ])
    .with_tree_sitter_query_semantics()
    .unwrap();
    let mut input = state(1, 0);
    input.repeats = vec![[0, 0, 0, 0]];
    input.captures = vec![[2, 0, 8, 16]];
    input.position = vec![3];
    let mut expected = input.clone();
    expected.pc = 2;
    expected.kind = 1;
    expected.captures.insert(0, [1, 0, 0, 7]);
    assert_eq!(run_view(&p, &[input], 1, |_| {}), Ok(vec![expected]));
}

#[test]
fn successor_query_views_preserve_saved_captures_between_empty_and_nonempty_prefixes() {
    let mut words = vec![
        word(REPEAT_BEGIN, 0, 0),
        word(FORK, 0, 4),
        word(CAPTURE, 0, 0),
        word(EMIT, 0, 0),
        word(EMIT, 0, 0),
        word(STOP, 0, 0),
    ];
    // Exercise the same shared guard library as a mixed query batch.
    words.extend([
        word(KIND, 0, 0),
        word(KIND, 0, 0),
        word(FIELD, 0, 0),
        word(FLAGS, 0, 0),
        word(FLAGS, 1, 0),
        word(FIRST, 1, 0),
        word(LAST, 1, 0),
        word(STOP, 0, 0),
    ]);
    let p = program(words).with_tree_sitter_query_semantics().unwrap();
    for tasks in [75, 8193, 131073] {
        let mut rows = Vec::new();
        let mut expected = Vec::new();
        for task in 0..tasks {
            let mut finished = state(4, 1);
            finished.task = task;
            finished.repeats = vec![[0, 0, 0, 0]];
            finished.position = vec![1];
            if task < 11 {
                finished.captures = vec![[2, 0, 8, 16]];
            }
            let mut active = finished.clone();
            active.kind = 0;
            active.pc = 2;
            active.position = vec![0];
            let mut completed = active.clone();
            completed.kind = 1;
            completed.pc = 3;
            completed.captures.insert(0, [1, 0, 0, 7]);
            expected.push(finished.clone());
            expected.push(completed);
            rows.push(finished);
            rows.push(active);
        }
        assert_eq!(run_view(&p, &rows, tasks as usize, |_| {}), Ok(expected));
    }
}

#[test]
fn successor_query_views_preserve_captures_across_different_merge_lengths() {
    let p = program(vec![
        word(REPEAT_BEGIN, 0, 0),
        word(FORK, 0, 4),
        word(CAPTURE, 0, 0),
        word(EMIT, 0, 0),
        word(EMIT, 0, 0),
        word(CAPTURE, 0, 0),
        word(NEXT, 0, 0),
        word(CAPTURE, 0, 0),
        word(EMIT, 0, 0),
        word(STOP, 0, 0),
        word(KIND, 0, 0),
        word(KIND, 0, 0),
        word(FIELD, 0, 0),
        word(FLAGS, 0, 0),
        word(FLAGS, 1, 0),
        word(FIRST, 1, 0),
        word(LAST, 1, 0),
        word(STOP, 0, 0),
    ])
    .with_tree_sitter_query_semantics()
    .unwrap();
    for tasks in [75, 8193] {
        let mut rows = Vec::new();
        let mut expected = Vec::new();
        for task in 0..tasks {
            let mut finished = state(4, 1);
            finished.task = task;
            finished.repeats = vec![[0, 0, 0, 0]];
            finished.position = vec![1];
            if task < 11 {
                finished.captures = vec![[2, 0, 8, 16]];
            }
            let mut active = finished.clone();
            active.kind = 0;
            active.position = vec![0];
            if task < 11 {
                active.pc = 2;
            } else {
                active.pc = 5;
                active.node = 2;
                active.captures = vec![[1, 0, 0, 7]];
            }
            let mut completed = active.clone();
            completed.kind = 1;
            if task < 11 {
                completed.pc = 3;
                completed.captures.insert(0, [1, 0, 0, 7]);
            } else {
                completed.pc = 8;
                completed.node = 3;
                completed.captures.extend([[2, 0, 8, 16], [3, 0, 17, 24]]);
            }
            expected.push(finished.clone());
            expected.push(completed);
            rows.push(finished);
            rows.push(active);
        }
        assert_eq!(run_view(&p, &rows, tasks as usize, |_| {}), Ok(expected));
    }
}

#[test]
fn successor_query_capture_copy_preserves_every_value_across_divergent_merge_partitions() {
    let p = Program::new(
        vec![
            word(REPEAT_BEGIN, 0, 0),
            word(FORK, 0, 4),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 0),
            word(EMIT, 0, 0),
            word(STOP, 0, 0),
            word(KIND, 0, 0),
            word(KIND, 0, 0),
            word(FIELD, 0, 0),
            word(FLAGS, 0, 0),
            word(FLAGS, 1, 0),
            word(FIRST, 1, 0),
            word(LAST, 1, 0),
            word(STOP, 0, 0),
        ],
        vec![b"first".to_vec(), b"second".to_vec(), b"third".to_vec()],
        vec![],
        vec![0, 1, 2],
        vec![],
    )
    .unwrap()
    .with_tree_sitter_query_semantics()
    .unwrap();
    let _guard = metal_execution_guard().unwrap();
    let scope = super::metal_backend::launch::LaunchScope::enter();
    let device = Device::system_default().unwrap();
    let queue = device.new_command_queue();
    let (resident, plan) = fixture(&p, 1);
    let data = super::metal_register_execution::upload(&device, &resident).unwrap();
    fn capture(next: &mut impl FnMut() -> u32) -> [u32; 4] {
        let mut value = [[1, 0, 0, 7], [2, 0, 8, 16], [3, 0, 17, 24]][next() as usize % 3];
        value[1] = next() % 3;
        value
    }
    for rows in [8191, 8192, 8193, 16383, 16384, 16385] {
        let mut seed = 0x1462u32;
        let mut next = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            seed >> 8
        };
        let mut fixtures = Vec::new();
        let mut expected = Vec::new();
        for _ in 0..rows {
            let saved_count = next() % 4;
            let added_count = next() % 33;
            let mut saved = (0..saved_count)
                .map(|_| capture(&mut next))
                .collect::<Vec<_>>();
            let mut added = (0..added_count)
                .map(|_| capture(&mut next))
                .collect::<Vec<_>>();
            saved.sort_by_key(|v| (v[2], std::cmp::Reverse(v[3])));
            added.sort_by_key(|v| (v[2], std::cmp::Reverse(v[3])));
            let mut merged = saved.clone();
            merged.extend_from_slice(&added);
            // Stable CPU ordering preserves old-before-new ties and every occurrence.
            merged.sort_by_key(|v| (v[2], std::cmp::Reverse(v[3])));
            expected.extend(merged);
            fixtures.push(super::metal_dynamic_execution::CaptureCopyFixture { saved, added });
        }
        let observed = super::metal_dynamic_execution::copy_captures_for_test(
            &device, &queue, &resident, &plan, &data, &fixtures,
        )
        .unwrap();
        assert_eq!(observed.len(), expected.len(), "rows={rows}");
        for (cell, (actual, expected)) in observed.iter().zip(&expected).enumerate() {
            assert_eq!(actual, expected, "rows={rows} cell={cell}");
        }
    }
    let trace = scope.snapshot();
    let copies = trace
        .iter()
        .filter(|record| record.kernel == "expanded_copy")
        .collect::<Vec<_>>();
    assert_eq!(
        copies.len(),
        6,
        "every capture partition must execute the production row transport"
    );
    assert!(copies.iter().all(|record| record.group == [32, 1, 1]));
    assert!(trace.iter().all(|record| record.completed));
}

#[test]
fn active_successor_generations_keep_complete_winners_within_eighteen_dispatches() {
    let p = program(vec![
        word(REPEAT_BEGIN, 0, 0),
        word(CAPTURE, 0, 0),
        word(EMIT, 0, 0),
    ])
    .with_tree_sitter_query_semantics()
    .unwrap();
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let queue = device.new_command_queue();
    for tasks in [1usize, 257, 4096] {
        let scope = super::metal_backend::launch::LaunchScope::enter();
        let start = scope.snapshot().len();
        let (resident, plan) = fixture(&p, tasks);
        let data = super::metal_register_execution::upload(&device, &resident).unwrap();
        let mut rows = Vec::new();
        let mut expected = Vec::new();
        for task in 0..tasks {
            for capture_count in 0..3 {
                let mut winner = state(1, 0);
                winner.task = task as u32;
                winner.cursors = vec![[0, 1, 0, 0]];
                winner.repeats = vec![[0, 0, 0, 0]];
                winner.captures = vec![[1, 0, 0, 7]; capture_count];
                winner.position = vec![task as u32, 7];
                let mut later = winner.clone();
                later.position.extend([0, 3]);
                // Include row zero as a winner, replacement of a later path,
                // equal-position ties, and different complete capture values.
                if rows.is_empty() {
                    rows.extend([winner.clone(), later, winner.clone()]);
                } else {
                    rows.extend([later, winner.clone(), winner.clone()]);
                }
                winner.pc = 2;
                winner.kind = 1;
                winner.captures.push([1, 0, 0, 7]);
                expected.push(winner);
            }
        }
        let input = upload(&device, &rows, &p);
        let started = std::time::Instant::now();
        let output = super::metal_dynamic_execution::execute_view_for_test(
            &device,
            &queue,
            &resident,
            &plan,
            &data,
            super::metal_dynamic_execution::ViewTestInput {
                private_frames: true,
                states: input.states,
                values: input.values,
                control: input.control,
            },
        )
        .unwrap();
        let actual = observe(
            &Storage {
                states: output.states,
                values: output.values,
                control: output.control,
            },
            &p,
        );
        assert_eq!(actual, expected, "tasks={tasks}");
        let trace = scope.snapshot();
        let trace = &trace[start..];
        eprintln!(
            "ACTIVE_GENERATION tasks={tasks} inputs={} survivors={} dispatches={} seconds={}",
            rows.len(),
            actual.len(),
            trace.len(),
            started.elapsed().as_secs_f64()
        );
        assert!(
            trace
                .iter()
                .all(|r| r.encoded && r.submitted && r.completed)
        );
        assert!(
            trace.len() <= 18,
            "one complete active generation exceeds its dispatch allowance: {}",
            trace.len()
        );
    }
}
