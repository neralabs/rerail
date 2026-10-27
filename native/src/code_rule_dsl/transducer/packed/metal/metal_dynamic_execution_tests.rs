//! Complete-generation diagnostic over the existing operators and shared guard.
use super::*;
use crate::policy_dsl::transducer::metal_backend::launch::{
    LaunchScope, commit_checked, wait_checked,
};
use crate::policy_dsl::transducer::metal_backend::{metal_execution_guard, pipeline_named};
use crate::policy_dsl::transducer::{
    metal_frontier_conformance, metal_grid, metal_register_execution,
};

pub(super) fn profile_expansion<const FIELDS: usize>(
    device: &Device,
    queue: &metal::CommandQueueRef,
    input: &SuccessorInput<'_, FIELDS>,
    preparation: &PreparedSuccessors<FIELDS>,
) -> Result<(), Error> {
    let bound = preparation.check_completed()?;
    let profile =
        MappedBuffer::zeroed::<[u32; 8]>(device, input.resident.input.packed.programs.len())?;
    let source = format!(
        "{}\n{}",
        preparation.code.source,
        include_str!("metal_successor_profile.metal")
    );
    metal::objc::rc::autoreleasepool(|| {
        let command = queue.new_command_buffer();
        crate::policy_dsl::transducer::metal_parallel::encode(
            command,
            &pipeline_named(&source, "successor_profile_expansion")?,
            "successor_profile_expansion",
            &[
                &input.data.buffer,
                &input.states.columns.buffer,
                &input.states.counts.buffer,
                &preparation.steps.buffer,
                &preparation.expansion.values.buffer,
                preparation.expansion.total(),
                &profile.buffer,
            ],
            &preparation.parameters,
            (preparation.parameters[18] as usize).div_ceil(256).max(1),
            None,
        );
        commit_checked(command)?;
        wait_checked(command)
    })?;
    let rows = read::<[u32; 8]>(&profile, input.resident.input.packed.programs.len())?;
    let sums = std::array::from_fn::<_, 8, _>(|field| {
        rows.iter().map(|row| u64::from(row[field])).sum::<u64>()
    });
    assert_eq!(sums[0], u64::from(preparation.parameters[18]));
    assert_eq!(sums[2], bound.states as u64);
    assert_eq!(sums[3], bound.cells as u64);
    assert_eq!(
        sums[4],
        preparation.comparison_needs()?.active_states.unwrap() as u64
    );
    assert_eq!(sums[5], u64::from(preparation.parameters[20]));
    assert!(rows.iter().all(|row| row[1] <= row[0] && row[6] <= row[2]));
    eprintln!("TEMPO_METAL_EXPANSION_PROFILE {}",serde_json::json!(rows.iter().enumerate().filter(|(_,row)|row[0]!=0).map(|(program,row)|serde_json::json!({"program":program,"steps":row[0],"live":row[1],"successors":row[2],"cells":row[3],"keys":row[4],"words":row[5],"maximumFanout":row[6],"pending":row[7]})).collect::<Vec<_>>()));
    Ok(())
}

#[test]
#[ignore = "offline seed-domain diagnosis for the complete pinned query catalog"]
fn pinned_catalog_seed_domain_profile() {
    use crate::policy_dsl::transducer::Program;
    use ast_grep_language::{LanguageExt, SupportLang};
    let root = std::env::var("TEMPO_PYTHON_SCM_QUERY_DIR").unwrap();
    let mut paths = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "scm"))
        .collect::<Vec<_>>();
    paths.sort();
    let mut rows = Vec::new();
    for path in paths {
        for query in
            Program::tree_sitter_top_level_patterns(&std::fs::read_to_string(path).unwrap())
                .unwrap()
        {
            let program = Program::compile_tree_sitter_query_pattern(
                &query,
                &SupportLang::Python.get_ts_language(),
            )
            .unwrap();
            rows.push(serde_json::json!({"index": rows.len(), "kinds": program.leading_kinds(),
                "register": crate::policy_dsl::transducer::execution_shape::register_program_paths(&program).is_some(),
                "terminalRepeat": crate::policy_dsl::transducer::terminal_repeat_proof::prove(&program).is_some(),
                "words": program.words.iter().map(|&w| crate::policy_dsl::transducer::split(w)).collect::<Vec<_>>() }));
        }
    }
    assert_eq!(rows.len(), 335);
    eprintln!("PINNED_SEED_DOMAIN_PROFILE {}", serde_json::json!(rows));
}

fn reach_terminal(scope: &LaunchScope) -> Result<(), Error> {
    let (program, file, _) = metal_frontier_conformance::convergence_case(2);
    let resident = ResidentInput::new(
        metal_grid::MetalGridInput::build(
            std::slice::from_ref(&program),
            &[file],
            &[vec![vec![1]]],
        )?,
        None,
    )?;
    let plan = ExecutionPlan::new(&resident, &[program])?;
    let device = Device::system_default().ok_or(Error::UnsupportedBackend)?;
    let data = metal_register_execution::upload(&device, &resident)?;
    let roots = MappedBuffer::zeroed::<u32>(&device, resident.input.grid.roots.len())?;
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    let pipeline = pipeline_named(include_str!("metal_resident.metal"), "register_roots")?;
    crate::policy_dsl::transducer::metal_parallel::encode(
        command,
        &pipeline,
        "register_roots",
        &[&data.buffer, &roots.buffer],
        &[],
        resident.input.grid.roots.len().div_ceil(256).max(1),
        None,
    );
    let start = encode_start(&device, command, &resident, &plan, &data, &roots)?;
    commit_checked(command)?;
    wait_checked(command)?;
    let mut frontier = start
        .finish(&device, &queue, &resident, &plan, &data)?
        .positioned(&device, &queue, &resident, &plan, &data)?;
    loop {
        // Only readiness counts cross to the owner. No host state interpreter.
        let counts = unsafe {
            std::slice::from_raw_parts(frontier.batch.counts.buffer.contents().cast::<u32>(), 4)
        };
        eprintln!(
            "generation={} ready={counts:?} completed={}",
            frontier.states.generation,
            scope.snapshot().iter().filter(|r| r.completed).count()
        );
        assert_eq!(
            counts[1], 0,
            "this acyclic fixture has no bulk-closure work"
        );
        if counts[0] == 0 && counts[2] == 0 {
            assert!(counts[3] > 0, "complete candidates must reach the barrier");
            return Ok(());
        }
        let preparation = metal_successor_prepare::prepare(
            &device,
            &queue,
            &frontier.input(&resident, &plan, &data),
        )?;
        let bound = preparation.check_completed()?;
        frontier = advance(
            &device,
            &queue,
            frontier.input(&resident, &plan, &data),
            preparation,
            &bound,
        )?;
    }
}

#[test]
fn initial_child_prefix_handoff_preserves_all_branch_digits_and_typed_values() {
    use crate::policy_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use ast_grep_core::tree_sitter::StrDoc;
    use ast_grep_language::SupportLang;
    use std::sync::Arc;
    for mode in [0, super::super::NAMED] {
        let source = Arc::new("a.target(v)\nf(a,b,c)\n".to_owned());
        let grammar = Grammar::new(SupportLang::Python).unwrap();
        let language = grammar.language();
        let native = StrDoc::new(source.as_str(), SupportLang::Python);
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
        let file = TapeDoc::restore(source, grammar, &image, &|| false).unwrap();
        let find = |text: &[u8]| {
            (0..file.transducer_len() as u32)
                .find(|&node| file.transducer_text(node) == text)
                .unwrap()
        };
        let root = (0..file.transducer_len() as u32)
            .find(|&node| {
                file.transducer_text(node) == b"a.target(v)"
                    && file.transducer_kind(node) == language.id_for_node_kind("call", true)
            })
            .unwrap();
        let attribute = find(b"a.target");
        let target = find(b"target");
        let arguments = find(b"(v)");
        let selected = file
            .transducer_children(arguments)
            .filter(|&node| mode == 0 || file.transducer_flags(node) & 1 != 0)
            .collect::<Vec<_>>();
        assert_eq!(selected.len(), if mode == 0 { 3 } else { 1 });
        let target_span = file.transducer_span(target);
        assert_eq!(target_span, 2..8);
        let mut program =
            super::super::metal_unique_child_tests::initial_child_prefix_program(mode);
        program.words[8] = super::super::word(
            super::super::FIELD,
            0,
            language.field_id_for_name("attribute").unwrap().get(),
        );
        program.validate().unwrap();
        let resident = ResidentInput::new(
            metal_grid::MetalGridInput::build(
                std::slice::from_ref(&program),
                &[file],
                &[vec![vec![root]]],
            )
            .unwrap(),
            None,
        )
        .unwrap();
        let plan = ExecutionPlan::new(&resident, &[program]).unwrap();
        let _guard = metal_execution_guard().unwrap();
        let scope = LaunchScope::enter();
        let device = Device::system_default().unwrap();
        let data = metal_register_execution::upload(&device, &resident).unwrap();
        let roots = MappedBuffer::zeroed::<u32>(&device, resident.input.grid.roots.len()).unwrap();
        let queue = device.new_command_queue();
        let start = metal::objc::rc::autoreleasepool(|| {
            let command = queue.new_command_buffer();
            crate::policy_dsl::transducer::metal_parallel::encode(
                command,
                &pipeline_named(include_str!("metal_resident.metal"), "register_roots").unwrap(),
                "register_roots",
                &[&data.buffer, &roots.buffer],
                &[],
                resident.input.grid.roots.len().div_ceil(256).max(1),
                None,
            );
            let pending = encode_start(&device, command, &resident, &plan, &data, &roots).unwrap();
            commit_checked(command).unwrap();
            wait_checked(command).unwrap();
            pending
        });
        let frontier = start
            .finish(&device, &queue, &resident, &plan, &data)
            .unwrap()
            .positioned(&device, &queue, &resident, &plan, &data)
            .unwrap();
        let n = selected.len();
        assert_eq!(
            read::<u32>(&frontier.states.counts, 5).unwrap(),
            [n as u32, n as u32, 1, 1, 1]
        );
        let columns = read::<u32>(&frontier.states.columns, n * POSITIONED_COLUMNS).unwrap();
        let spans = read::<u32>(&frontier.control.spans, n * 6).unwrap();
        let digits = read::<u32>(&frontier.control.digits, n * 4).unwrap();
        for (i, node) in selected.into_iter().enumerate() {
            assert_eq!(
                std::array::from_fn::<_, POSITIONED_COLUMNS, _>(|field| columns[field * n + i]),
                [
                    0,
                    16,
                    node,
                    0,
                    0,
                    0,
                    2 * i as u32,
                    2,
                    i as u32,
                    1,
                    i as u32,
                    1
                ]
            );
            assert_eq!(
                std::array::from_fn::<_, 6, _>(|field| spans[field * n + i]),
                [4 * i as u32, 4, 4 * i as u32 + 4, 0, 4 * i as u32 + 4, 0]
            );
            assert_eq!(
                &digits[4 * i..4 * i + 4],
                &[0, if mode == 0 { 2 } else { 1 }, 1, i as u32]
            );
        }
        assert_eq!(
            read::<u32>(&frontier.control.path_work, n).unwrap(),
            vec![16; n]
        );
        assert_eq!(
            read::<u32>(&frontier.control.count, 1).unwrap(),
            [4 * n as u32]
        );
        assert_eq!(
            read::<u32>(&frontier.values.counts, 2).unwrap(),
            [2 * n as u32, n as u32]
        );
        assert_eq!(
            read::<[u32; 4]>(&frontier.values.cursors, 2 * n).unwrap(),
            [[0, root, 0, 0], [1, attribute, 0, 0]].repeat(n)
        );
        assert_eq!(
            read::<[u32; 4]>(&frontier.values.repeats, n).unwrap(),
            vec![[0, 9, 0, 0]; n]
        );
        assert_eq!(
            read::<[u32; 4]>(&frontier.values.captures, n).unwrap(),
            vec![[target, 0, 2, 8]; n]
        );
        assert!(scope.snapshot().iter().all(|record| record.completed));
    }
}

#[test]
fn complete_convergence_routing_reaches_terminal_barrier_within_shared_budget() {
    let _guard = metal_execution_guard().unwrap();
    let scope = LaunchScope::enter();
    let actual = reach_terminal(&scope);
    let trace = scope.snapshot();
    for (index, record) in trace.iter().enumerate() {
        eprintln!("dispatch {}: {record:?}", index + 1);
    }
    assert_eq!(actual, Ok(()));
    assert!(trace.len() <= 600);
    assert!(
        trace
            .iter()
            .all(|r| r.encoded && r.submitted && r.completed)
    );
}

#[test]
fn direct_tuples_do_not_reserve_dynamic_seed_state_or_successor_work() {
    use crate::policy_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use crate::policy_dsl::transducer::{
        CAPTURE, EMIT, Program, execute_batch, metal_grid_executor::GuardedExecutor, word,
    };
    use ast_grep_core::tree_sitter::StrDoc;
    use ast_grep_language::SupportLang;
    use std::sync::Arc;
    let source = "# direct\n".repeat(8192);
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let native = StrDoc::new(&source, SupportLang::Python);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
    let direct_file = TapeDoc::restore(Arc::new(source), grammar, &image, &|| false).unwrap();
    let (dynamic, dynamic_file, _) = metal_frontier_conformance::convergence_case(2);
    let direct = Program::new(
        vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"x".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let programs = [direct, dynamic];
    let files = [direct_file, dynamic_file];
    let anchors = [
        vec![
            (0..files[0].transducer_len() as u32).collect::<Vec<_>>(),
            vec![],
        ],
        vec![vec![], vec![1]],
    ];
    let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    assert_eq!(expected[0][0].len(), 8193);
    assert!(!expected[1][1].is_empty());
    let resident = ResidentInput::new(
        metal_grid::MetalGridInput::build(&programs, &files, &anchors).unwrap(),
        None,
    )
    .unwrap();
    assert_eq!(resident.input.grid.header.semantic_tuple_count, 8194);
    let plan = ExecutionPlan::new(&resident, &programs).unwrap();
    let scope = LaunchScope::enter();
    let actual = GuardedExecutor::new(resident, plan)
        .unwrap()
        .execute()
        .and_then(|p| p.into_outcome())
        .and_then(|o| o.into_result());
    let trace = scope.snapshot();
    drop(scope);
    assert_eq!(actual, Ok(expected));
    let seeds = trace
        .iter()
        .filter(|r| r.kernel == "seed_scatter")
        .collect::<Vec<_>>();
    assert_eq!(seeds.len(), 1);
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;
    assert_eq!(
        seeds[0].buffers[4].1, page,
        "only the one dynamic tuple belongs to seed-state storage"
    );
    assert!(
        trace.len() <= 600
            && trace
                .iter()
                .all(|r| r.encoded && r.submitted && r.completed)
    );
}

#[test]
fn empty_expansions_do_not_reserve_successor_state_for_every_selected_root() {
    use crate::policy_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use crate::policy_dsl::transducer::{
        CAPTURE, CHILD, EMIT, KIND, NAMED, Program, SAVE_CURSOR, execute_batch,
        metal_grid_executor::GuardedExecutor, word,
    };
    use ast_grep_core::tree_sitter::StrDoc;
    use ast_grep_language::{LanguageExt, SupportLang};
    use std::sync::Arc;
    let source = "# leaf\n".repeat(1024);
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let native = StrDoc::new(&source, SupportLang::Python);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
    let file = TapeDoc::restore(Arc::new(source), grammar, &image, &|| false).unwrap();
    let identifier = SupportLang::Python
        .get_ts_language()
        .id_for_node_kind("identifier", true);
    let expansion = Program::new(
        vec![
            word(SAVE_CURSOR, 0, 0),
            word(CAPTURE, 0, 0),
            word(CHILD, NAMED, 0),
            word(KIND, 0, identifier),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 0),
        ],
        vec![b"child".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let mut programs = vec![expansion; 2048];
    programs.push(
        Program::new(
            vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            vec![b"leaf".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap(),
    );
    assert_eq!(file.transducer_len(), 1025);
    assert!((1..1025).all(|node| file.transducer_children(node).next().is_none()));
    let mut selected = vec![(1..1025).collect::<Vec<u32>>(); 2048];
    selected.push(vec![1]);
    let files = [file];
    let anchors = [selected];
    let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    assert!(expected[0][..2048].iter().all(Vec::is_empty));
    assert_eq!(expected[0][2048].len(), 1);
    assert_eq!(expected[0][2048][0].captures[0].node, 1);
    let resident = ResidentInput::new(
        metal_grid::MetalGridInput::build(&programs, &files, &anchors).unwrap(),
        None,
    )
    .unwrap();
    let plan = ExecutionPlan::new(&resident, &programs).unwrap();
    let scope = LaunchScope::enter();
    let actual = GuardedExecutor::new(resident, plan)
        .unwrap()
        .execute()
        .and_then(|packed| packed.into_outcome())
        .and_then(|outcome| outcome.into_result());
    let trace = scope.snapshot();
    assert_eq!(
        actual,
        Ok(expected),
        "dead expansion inputs cannot exhaust the shared allocation guard"
    );
    assert!(!trace.is_empty() && trace.len() <= 600);
    assert!(trace.iter().all(|r| r.encoded
        && r.submitted
        && r.completed
        && r.peak_application_buffer_bytes <= 2_000_000_000));
    assert_eq!(
        trace
            .iter()
            .filter(|r| r.kernel == "register_roots")
            .count(),
        1
    );
}

#[test]
fn sparse_initial_expansions_preserve_full_ordinals_across_blocks_and_files() {
    fn tape(source: &str) -> crate::policy_dsl::tape::TapeDoc {
        use crate::policy_dsl::tape::{Grammar, TapeDoc, TapeImage};
        use ast_grep_core::tree_sitter::StrDoc;
        use ast_grep_language::SupportLang;
        let grammar = Grammar::new(SupportLang::Python).unwrap();
        let native = StrDoc::new(source, SupportLang::Python);
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
        TapeDoc::restore(
            std::sync::Arc::new(source.to_owned()),
            grammar,
            &image,
            &|| false,
        )
        .unwrap()
    }
    use crate::policy_dsl::transducer::{
        CAPTURE, CHILD, DESCEND, EMIT, FOLLOWING, NAMED, Program, execute_batch,
        metal_grid_executor::GuardedExecutor, word,
    };
    for op in [CHILD, DESCEND, FOLLOWING] {
        let source = if op == CHILD {
            format!(
                "{}f(a, b)\n{}g(c)\n",
                "# leaf\n".repeat(257),
                "# leaf\n".repeat(256)
            )
        } else {
            "f(a, b)\n# leaf\ng(c)\n".to_owned()
        };
        let programs = [0, NAMED].map(|mode| {
            Program::new(
                vec![word(op, mode, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
                vec![b"expanded".to_vec()],
                vec![],
                vec![0],
                vec![],
            )
            .unwrap()
        });
        let files = [tape(&source), tape("# empty\n")];
        let anchors = files
            .iter()
            .map(|file| vec![(0..file.transducer_len() as u32).collect(); 2])
            .collect::<Vec<_>>();
        let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        assert!(!expected[0][0].is_empty() && !expected[0][1].is_empty());
        let resident = ResidentInput::new(
            metal_grid::MetalGridInput::build(&programs, &files, &anchors).unwrap(),
            None,
        )
        .unwrap();
        let plan = ExecutionPlan::new(&resident, &programs).unwrap();
        let scope = LaunchScope::enter();
        let actual = GuardedExecutor::new(resident, plan)
            .unwrap()
            .execute()
            .and_then(|packed| packed.into_outcome())
            .and_then(|outcome| outcome.into_result());
        let trace = scope.snapshot();
        drop(scope);
        assert_eq!(actual, Ok(expected), "opcode={op}");
        assert!(!trace.is_empty() && trace.len() <= 600);
        assert!(trace.iter().all(|r| r.encoded
            && r.submitted
            && r.completed
            && r.peak_application_buffer_bytes <= 2_000_000_000));
        assert_eq!(
            trace
                .iter()
                .filter(|r| r.kernel == "register_roots")
                .count(),
            1
        );
    }
}

#[test]
fn completed_input_counts_retire_before_finite_successors() {
    use crate::policy_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use crate::policy_dsl::transducer::{CAPTURE, CHILD, EMIT, NAMED, Program, word};
    use ast_grep_core::tree_sitter::StrDoc;
    use ast_grep_language::SupportLang;
    use std::sync::Arc;
    let tasks = 65_536;
    let source = Arc::new("f(a,b)\n".repeat(tasks));
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let call = grammar.language().id_for_node_kind("call", true);
    let native = StrDoc::new(source.as_str(), SupportLang::Python);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
    let file = TapeDoc::restore(source, grammar, &image, &|| false).unwrap();
    let selected = (0..file.transducer_len() as u32)
        .filter(|&node| file.transducer_kind(node) == call)
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), tasks);
    let expected_children = selected
        .iter()
        .map(|&node| file.transducer_children(node).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    assert!(expected_children.iter().all(|children| children.len() == 2));
    let program = Program::new(
        vec![word(CHILD, NAMED, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"child".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let resident = ResidentInput::new(
        metal_grid::MetalGridInput::build(
            std::slice::from_ref(&program),
            &[file],
            &[vec![selected.clone()]],
        )
        .unwrap(),
        None,
    )
    .unwrap();
    let plan = ExecutionPlan::new(&resident, &[program]).unwrap();
    let _guard = metal_execution_guard().unwrap();
    let scope = LaunchScope::enter();
    let device = Device::system_default().unwrap();
    let data = metal_register_execution::upload(&device, &resident).unwrap();
    let roots = MappedBuffer::zeroed::<u32>(&device, resident.input.grid.roots.len()).unwrap();
    let queue = device.new_command_queue();
    let start = metal::objc::rc::autoreleasepool(|| {
        let command = queue.new_command_buffer();
        let pipeline =
            pipeline_named(include_str!("metal_resident.metal"), "register_roots").unwrap();
        crate::policy_dsl::transducer::metal_parallel::encode(
            command,
            &pipeline,
            "register_roots",
            &[&data.buffer, &roots.buffer],
            &[],
            resident.input.grid.roots.len().div_ceil(256).max(1),
            None,
        );
        let pending = encode_start(&device, command, &resident, &plan, &data, &roots).unwrap();
        commit_checked(command).unwrap();
        wait_checked(command).unwrap();
        pending
    });
    let observation = super::super::metal_backend::BufferObservation::enter();
    let InitialExecution { frontier, prepared } = start
        .finish(&device, &queue, &resident, &plan, &data)
        .unwrap();
    let (preparation, bound) = prepared.expect("this CHILD entry requires finite successor work");
    assert_eq!(frontier.states.capacity, tasks);
    assert_eq!(frontier.states.generation, 0);
    assert_eq!(frontier.values.cursor_capacity, 0);
    assert_eq!(frontier.values.repeat_capacity, 0);
    assert_eq!(frontier.values.capture_capacity, 0);
    assert_eq!(read::<u32>(&frontier.values.counts, 2).unwrap(), [0, 0]);
    assert_eq!(frontier.control.digit_capacity, 0);
    assert_eq!(read::<u32>(&frontier.control.count, 1).unwrap(), [0]);
    let columns = read::<u32>(&frontier.states.columns, tasks * INITIAL_COLUMNS).unwrap();
    for (task, &node) in selected.iter().enumerate() {
        let actual =
            std::array::from_fn::<_, INITIAL_COLUMNS, _>(|column| columns[column * tasks + task]);
        assert_eq!(actual, [task as u32, node, 0]);
    }
    assert_eq!(
        read::<u32>(&frontier.batch.indices, tasks).unwrap(),
        (0..tasks as u32).collect::<Vec<_>>()
    );
    assert_eq!(
        &read::<u32>(&frontier.batch.counts, 17).unwrap()[..4],
        &[tasks as u32, 0, 0, 0]
    );
    assert_eq!(
        bound,
        Capacity {
            states: 2 * tasks,
            cells: 2 * tasks
        }
    );
    let bytes = observation.live_bytes();
    let next = advance(
        &device,
        &queue,
        frontier.input(&resident, &plan, &data),
        preparation,
        &bound,
    )
    .unwrap();
    let count = read::<u32>(&next.states.counts, 5).unwrap()[0] as usize;
    assert_eq!(count, 2 * tasks);
    let columns = read::<u32>(&next.states.columns, count * POSITIONED_COLUMNS).unwrap();
    let spans = read::<u32>(&next.control.spans, count * 6).unwrap();
    let digits = read::<u32>(&next.control.digits, count).unwrap();
    let work = read::<u32>(&next.control.path_work, count).unwrap();
    let mut seen = vec![[false; 2]; tasks];
    for index in 0..count {
        let task = columns[index] as usize;
        assert!(task < tasks);
        assert_eq!(spans[count + index], 1);
        let choice = digits[spans[index] as usize] as usize;
        assert!(choice < 2 && !seen[task][choice]);
        seen[task][choice] = true;
        let actual = std::array::from_fn::<_, POSITIONED_COLUMNS, _>(|column| {
            columns[column * count + index]
        });
        assert_eq!(
            actual,
            [
                task as u32,
                1,
                expected_children[task][choice],
                0,
                0,
                2,
                0,
                0,
                0,
                0,
                0,
                0
            ]
        );
        assert_eq!(work[index], 1);
        assert_eq!(spans[3 * count + index], 0);
        assert_eq!(spans[5 * count + index], 0);
        assert_eq!(spans[2 * count + index], spans[index] + 1);
        assert_eq!(spans[4 * count + index], spans[index] + 1);
    }
    assert!(seen.iter().all(|children| *children == [true; 2]));
    eprintln!("TEMPO_INITIAL_SUCCESSOR_STORAGE tasks={tasks} bytes={bytes}");
    assert!(
        bytes <= 104 * tasks + 256 * 1024,
        "completed prefix scratch must retire before successor allocation: {bytes}"
    );
    assert!(scope.snapshot().iter().all(|record| record.completed));
}

#[test]
fn terminal_handoff_retires_completed_frontier_owners_before_containment() {
    use crate::policy_dsl::transducer::{CAPTURE, EMIT, Program, SAVE_CURSOR, execute_batch, word};
    for (cursor_slots, position_digits) in [(64usize, 0usize), (0, 64)] {
        for tasks in [4_096usize, 16_384] {
            metal::objc::rc::autoreleasepool(|| {
                let _guard = metal_execution_guard().unwrap();
                let scope = LaunchScope::enter();
                let terminal_pc = cursor_slots.max(position_digits) + 1;
                let mut words = if cursor_slots != 0 {
                    (0..cursor_slots)
                        .map(|slot| word(SAVE_CURSOR, 0, slot as u16))
                        .collect::<Vec<_>>()
                } else {
                    vec![word(super::super::FORK, 0, terminal_pc as u16 + 1); position_digits]
                };
                words.extend([word(CAPTURE, 0, 0), word(EMIT, 0, 0)]);
                if position_digits != 0 {
                    words.push(word(super::super::STOP, 0, 0));
                }
                let program =
                    Program::new(words, vec![b"selected".to_vec()], vec![], vec![0], vec![])
                        .unwrap();
                let (_, file, _) = metal_frontier_conformance::convergence_case(tasks);
                let roots = (1..=tasks as u32).collect::<Vec<_>>();
                let anchors = [vec![roots.clone()]];
                let expected = execute_batch(
                    std::slice::from_ref(&program),
                    std::slice::from_ref(&file),
                    Some(&anchors),
                )
                .unwrap();
                assert_eq!(expected[0][0].len(), tasks);
                for (root, output) in roots.iter().zip(&expected[0][0]) {
                    assert_eq!(output.start, *root);
                    assert_eq!(output.captures.len(), 1);
                    assert_eq!(output.captures[0].node, *root);
                }
                let resident = ResidentInput::new(
                    metal_grid::MetalGridInput::build(
                        std::slice::from_ref(&program),
                        &[file],
                        &anchors,
                    )
                    .unwrap(),
                    None,
                )
                .unwrap();
                let plan = ExecutionPlan::new(&resident, std::slice::from_ref(&program)).unwrap();
                let device = Device::system_default().unwrap();
                let queue = device.new_command_queue();
                let data = metal_register_execution::upload(&device, &resident).unwrap();
                let predicates = super::super::metal_capture_predicates::PredicatePrograms::new(
                    &device, &queue, &resident, &data,
                )
                .unwrap();
                let before = super::super::metal_backend::buffer_usage().0;
                let frontier = DynamicFrontier::<POSITIONED_COLUMNS>::allocate_exact(
                    &device,
                    &FrontierCapacity {
                        states: tasks,
                        cursors: tasks * cursor_slots,
                        repeats: 0,
                        captures: tasks,
                        digits: tasks * position_digits,
                    },
                    8,
                )
                .unwrap();
                let mut columns = vec![0u32; tasks * POSITIONED_COLUMNS];
                let mut cursors = Vec::with_capacity(tasks * cursor_slots);
                let mut captures = Vec::with_capacity(tasks);
                for (task, &node) in roots.iter().enumerate() {
                    let fields = [
                        task as u32,
                        terminal_pc as u32,
                        node,
                        program.sibling_requirement(terminal_pc as u16) as u32,
                        0,
                        3,
                        (task * cursor_slots) as u32,
                        cursor_slots as u32,
                        0,
                        0,
                        task as u32,
                        1,
                    ];
                    for (field, value) in fields.into_iter().enumerate() {
                        columns[field * tasks + task] = value;
                    }
                    for slot in 0..cursor_slots {
                        cursors.push([slot as u32, node, 0, 0]);
                    }
                    let source = resident.input.packed.nodes[node as usize];
                    captures.push([node, 0, source.source_start, source.source_end]);
                }
                let write = |buffer: &MappedBuffer, values: &[u32]| unsafe {
                    assert!(std::mem::size_of_val(values) as u64 <= buffer.buffer.length());
                    std::ptr::copy_nonoverlapping(
                        values.as_ptr(),
                        buffer.buffer.contents().cast(),
                        values.len(),
                    );
                };
                write(&frontier.states.columns, &columns);
                write(
                    &frontier.states.counts,
                    &[tasks as u32, tasks as u32, tasks.div_ceil(256) as u32, 1, 1],
                );
                write(&frontier.values.cursors, cursors.as_flattened());
                write(&frontier.values.captures, captures.as_flattened());
                write(&frontier.values.counts, &[(tasks * cursor_slots) as u32, 0]);
                write(
                    &frontier.control.path_work,
                    &vec![terminal_pc as u32; tasks],
                );
                if position_digits != 0 {
                    let mut spans = vec![0u32; tasks * 6];
                    for task in 0..tasks {
                        spans[task] = (task * position_digits) as u32;
                        spans[tasks + task] = position_digits as u32;
                    }
                    write(&frontier.control.spans, &spans);
                    write(&frontier.control.count, &[(tasks * position_digits) as u32]);
                }
                let mut ids = vec![u32::MAX; 4 * tasks];
                ids[3 * tasks..].copy_from_slice(&(0..tasks as u32).collect::<Vec<_>>());
                let mut ready = [0u32; 17];
                ready[3] = tasks as u32;
                for class in 0..4 {
                    ready[4 + 3 * class] = ready[class].div_ceil(256);
                    ready[5 + 3 * class] = 1;
                    ready[6 + 3 * class] = 1;
                }
                write(&frontier.batch.indices, &ids);
                write(&frontier.batch.counts, &ready);
                {
                    let foreign = MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
                    let clone_buffer = |b: &MappedBuffer| MappedBuffer {
                        buffer: b.buffer.clone(),
                    };
                    for mismatch_generation in [false, true] {
                        let values = StateValues {
                            cursors: clone_buffer(&frontier.values.cursors),
                            repeats: clone_buffer(&frontier.values.repeats),
                            captures: clone_buffer(&frontier.values.captures),
                            counts: clone_buffer(&frontier.values.counts),
                            cursor_capacity: frontier.values.cursor_capacity,
                            repeat_capacity: 0,
                            capture_capacity: tasks,
                            generation: if mismatch_generation { 9 } else { 8 },
                        };
                        let prior = scope.snapshot().len();
                        let wrong = metal_terminal::prepare(
                            &device,
                            &queue,
                            metal_terminal::TerminalContext {
                                resident: &resident,
                                data: if mismatch_generation { &data } else { &foreign },
                            },
                            metal_terminal::TerminalFrame {
                                resident: &resident,
                                plan: &plan,
                                data: &data,
                                states: &frontier.states,
                                control: &frontier.control,
                                ready: &frontier.batch,
                            },
                            values,
                        );
                        assert_eq!(wrong.err(), Some(Error::BackendFailure));
                        assert_eq!(
                            scope.snapshot().len(),
                            prior,
                            "foreign immutable owners must reject before dispatch"
                        );
                    }
                }
                let started = std::time::Instant::now();
                let DynamicOutput { candidates, errors } = finish_frontier(
                    &device,
                    &queue,
                    &resident,
                    &plan,
                    &data,
                    frontier,
                    &predicates,
                )
                .unwrap();
                assert!(errors.iter().all(Option::is_none));
                let (results, captures) = candidates.read().unwrap();
                let actual = super::super::packed_batch::PackedExecution::from_grid_records(
                    resident.input.packed.clone(),
                    &resident.input.grid.evaluation_spans,
                    results,
                    captures,
                    errors,
                )
                .unwrap()
                .into_outcome()
                .unwrap();
                assert!(actual.errors.iter().flatten().all(Option::is_none));
                assert_eq!(actual.outputs, expected);
                let trace = scope.snapshot();
                assert!(
                    trace
                        .iter()
                        .all(|r| r.encoded && r.submitted && r.completed)
                );
                let containment = trace
                    .iter()
                    .filter(|r| r.kernel == "frontier_initialize")
                    .map(|r| r.application_buffer_bytes.saturating_sub(before))
                    .max()
                    .expect("the terminal handoff must execute containment");
                eprintln!(
                    "TERMINAL_OWNER_EVIDENCE {}",
                    serde_json::json!({"tasks":tasks,"cursorEntries":tasks*cursor_slots,"positionDigits":tasks*position_digits,"containmentAdditionalBytes":containment,"dispatches":trace.len(),"seconds":started.elapsed().as_secs_f64()})
                );
                assert!(
                    containment <= 384 * tasks + 1024 * 1024,
                    "containment must exclude completed state, control, readiness, and cursor owners: {containment}"
                );
            });
        }
    }
}

#[test]
fn completed_capture_intervals_preserve_full_query_output_under_the_original_pair_limit() {
    use crate::policy_dsl::transducer::*;
    for nodes in [96usize, 128] {
        metal::objc::rc::autoreleasepool(|| {
            let _guard = metal_execution_guard().unwrap();
            let scope = LaunchScope::enter();
            let language = ast_grep_language::LanguageExt::get_ts_language(
                &ast_grep_language::SupportLang::Python,
            );
            let program = Program::new(
                vec![
                    word(REPEAT_BEGIN, 0, 9),
                    word(SAVE_CURSOR, 0, 0),
                    word(CHILD, NAMED, 0),
                    word(KIND, 0, language.id_for_node_kind("comment", true)),
                    word(CAPTURE, 0, 0),
                    word(REPEAT_BEGIN, 0, 0),
                    word(REPEAT_BUMP, 0, 0),
                    word(FORK, 0, 13),
                    word(NEXT, NAMED, 0),
                    word(KIND, 0, language.id_for_node_kind("comment", true)),
                    word(CAPTURE, 0, 0),
                    word(REPEAT_BUMP, 0, 0),
                    word(GOTO, 0, 7),
                    word(REPEAT_END, 0, 0),
                    word(RESTORE_CURSOR, 0, 0),
                    word(REPEAT_END, 0, 9),
                    word(EMIT, 0, 1),
                ],
                vec![b"items".to_vec(), b"module".to_vec()],
                vec![],
                vec![0, 1],
                vec![],
            )
            .unwrap()
            .with_tree_sitter_query_semantics()
            .unwrap();
            let (_, file, _) = metal_frontier_conformance::convergence_case(nodes);
            let anchors = [vec![vec![0]]];
            let expected = execute_batch(
                std::slice::from_ref(&program),
                std::slice::from_ref(&file),
                Some(&anchors),
            )
            .unwrap();
            assert_eq!(expected[0][0].len(), 1);
            assert_eq!(expected[0][0][0].captures.len(), nodes);
            assert_eq!(expected[0][0][0].local_ordinal, 0);
            let resident = ResidentInput::new(
                metal_grid::MetalGridInput::build(
                    std::slice::from_ref(&program),
                    &[file],
                    &anchors,
                )
                .unwrap(),
                None,
            )
            .unwrap();
            let plan = ExecutionPlan::new(&resident, std::slice::from_ref(&program)).unwrap();
            let device = Device::system_default().unwrap();
            let queue = device.new_command_queue();
            let data = metal_register_execution::upload(&device, &resident).unwrap();
            let predicates = super::super::metal_capture_predicates::PredicatePrograms::new(
                &device, &queue, &resident, &data,
            )
            .unwrap();
            let rows = nodes * (nodes + 1) / 2;
            let capture_count = nodes * (nodes + 1) * (nodes + 2) / 6;
            let frontier = DynamicFrontier::<POSITIONED_COLUMNS>::allocate_exact(
                &device,
                &FrontierCapacity {
                    states: rows,
                    cursors: rows,
                    repeats: 0,
                    captures: capture_count,
                    digits: capture_count + rows,
                },
                8,
            )
            .unwrap();
            let mut columns = vec![0u32; POSITIONED_COLUMNS * rows];
            let mut spans = vec![0u32; 6 * rows];
            let mut captures = Vec::with_capacity(capture_count);
            let mut cursors = Vec::with_capacity(rows);
            let mut digits = Vec::with_capacity(capture_count + rows);
            let mut work = Vec::with_capacity(rows);
            let mut row = 0;
            for start in 0..nodes {
                for length in (1..=nodes - start).rev() {
                    let fields = [
                        0,
                        16,
                        0,
                        program.sibling_requirement(16) as u32,
                        0,
                        1,
                        row as u32,
                        1,
                        0,
                        0,
                        captures.len() as u32,
                        length as u32,
                    ];
                    for (field, value) in fields.into_iter().enumerate() {
                        columns[field * rows + row] = value;
                    }
                    cursors.push([0u32, 0, 0, 0]);
                    spans[row] = digits.len() as u32;
                    spans[rows + row] = length as u32 + 1;
                    digits.push(start as u32);
                    digits.extend(std::iter::repeat_n(0, length - 1));
                    digits.push(1);
                    work.push((6 * length + 6) as u32);
                    for node in start + 1..=start + length {
                        let source = resident.input.packed.nodes[node];
                        captures.push([node as u32, 0, source.source_start, source.source_end]);
                    }
                    row += 1;
                }
            }
            assert_eq!(
                (row, captures.len(), digits.len()),
                (rows, capture_count, capture_count + rows)
            );
            let write = |buffer: &MappedBuffer, values: &[u32]| unsafe {
                assert!(std::mem::size_of_val(values) as u64 <= buffer.buffer.length());
                std::ptr::copy_nonoverlapping(
                    values.as_ptr(),
                    buffer.buffer.contents().cast(),
                    values.len(),
                );
            };
            write(&frontier.states.columns, &columns);
            write(
                &frontier.states.counts,
                &[
                    rows as u32,
                    capture_count as u32,
                    rows.div_ceil(256) as u32,
                    1,
                    1,
                ],
            );
            write(&frontier.values.cursors, cursors.as_flattened());
            write(&frontier.values.captures, captures.as_flattened());
            write(&frontier.values.counts, &[rows as u32, 0]);
            write(&frontier.control.spans, &spans);
            write(&frontier.control.path_work, &work);
            write(&frontier.control.digits, &digits);
            write(&frontier.control.count, &[digits.len() as u32]);
            let mut indices = vec![u32::MAX; 4 * rows];
            indices[3 * rows..].copy_from_slice(&(0..rows as u32).collect::<Vec<_>>());
            let mut ready = [0u32; 17];
            ready[3] = rows as u32;
            for class in 0..4 {
                ready[4 + class * 3] = ready[class].div_ceil(256);
                ready[5 + class * 3] = 1;
                ready[6 + class * 3] = 1;
            }
            write(&frontier.batch.indices, &indices);
            write(&frontier.batch.counts, &ready);
            let started = std::time::Instant::now();
            let actual = finish_frontier(
                &device,
                &queue,
                &resident,
                &plan,
                &data,
                frontier,
                &predicates,
            );
            let trace = scope.snapshot();
            eprintln!(
                "TERMINAL_INTERVAL_EVIDENCE {}",
                serde_json::json!({"nodes":nodes,"rows":rows,"captures":capture_count,"potentialPairs":rows*rows,"dispatches":trace.len(),"seconds":started.elapsed().as_secs_f64(),"error":format!("{:?}",actual.as_ref().err()),"lastKernel":trace.last().map(|r|&r.kernel),"peakMetalBytes":trace.iter().map(|r|r.peak_application_buffer_bytes).max()})
            );
            assert!(
                trace
                    .iter()
                    .all(|r| r.encoded && r.submitted && r.completed)
            );
            let DynamicOutput { candidates, errors } = actual.expect(
                "completed query intervals must reduce under the unchanged comparison limit",
            );
            assert!(errors.iter().all(Option::is_none));
            let (results, captures) = candidates.read().unwrap();
            let actual = super::super::packed_batch::PackedExecution::from_grid_records(
                resident.input.packed.clone(),
                &resident.input.grid.evaluation_spans,
                results,
                captures,
                errors,
            )
            .unwrap()
            .into_outcome()
            .unwrap();
            assert!(actual.errors.iter().flatten().all(Option::is_none));
            assert_eq!(actual.outputs, expected);
        });
    }
}
