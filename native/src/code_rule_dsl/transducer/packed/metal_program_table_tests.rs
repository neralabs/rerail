//! A valid table shape cannot silently change the declared Program meaning.
use super::*;
use metal_grid_plan::{ExecutionPlan, ResidentInput};
use packed_batch::PackedBatch;

fn declared_program() -> Program {
    Program::new_with_capture_predicates(
        vec![
            word(TEXT, 0, 0),
            word(CAPTURE, 0, 0),
            word(CAPTURE, 0, 1),
            word(ASSERT, 0, 0),
            word(EMIT, 0, 0),
        ],
        vec![
            b"left".to_vec(),
            b"right".to_vec(),
            b"needle".to_vec(),
            b"role".to_vec(),
            b"name".to_vec(),
        ],
        vec![TextPredicate::Equal(2)],
        vec![CapturePredicate::Text {
            capture: 0,
            predicate: 0,
            negated: false,
            any: false,
        }],
        vec![0, 1],
        vec![],
    )
    .unwrap()
    .with_directives(vec![
        Directive::Set {
            key: 3,
            value: Some(4),
        },
        Directive::SetCapture {
            capture: 0,
            key: 3,
            value: None,
        },
    ])
    .unwrap()
}

fn resident(programs: &[Program]) -> ResidentInput {
    ResidentInput::new(
        metal_grid::MetalGridInput::build(
            programs,
            &[tape("needle\n")],
            &[vec![vec![1]; programs.len()]],
        )
        .unwrap(),
        None,
    )
    .unwrap()
}

fn change_meaning(batch: &mut PackedBatch, defect: usize) {
    let pg = batch.programs[0];
    match defect {
        0 => batch.values[pg.value_start as usize].kind = 1,
        1 => batch.values[pg.value_start as usize].operand = 3,
        2 => batch.assertions[pg.capture_start as usize].flags ^= 1,
        3 => batch.assertions[pg.capture_start as usize].first = 1,
        4 => batch.assertion_data[pg.output_start as usize + 1] = 0,
        5 => batch.assertion_data[pg.output_start as usize + 2] = 0,
        6 => batch.labels[pg.label_start as usize] = 1,
        7 => {
            let at = batch.strings[pg.string_start as usize + 2].byte_start as usize;
            batch.string_bytes[at] = b'N';
        }
        8 => batch.programs[0].maximum_cursor_slot = 1,
        _ => unreachable!(),
    }
}

#[test]
fn plan_construction_rejects_shape_valid_tables_with_a_different_program_meaning() {
    let programs = [declared_program()];
    let expected = execute_batch(&programs, &[tape("needle\n")], Some(&[vec![vec![1]]])).unwrap();
    assert_eq!(expected[0][0].len(), 1);
    assert_eq!(expected[0][0][0].captures.len(), 2);
    for defect in 0..9 {
        let mut input = resident(&programs);
        change_meaning(Arc::make_mut(&mut input.input.packed), defect);
        assert_eq!(
            ExecutionPlan::new(&input, &programs).err(),
            Some(Error::InvalidAnchor),
            "defect {defect}"
        );
    }
}

#[test]
fn a_prepared_plan_rejects_semantic_table_changes_before_device_access() {
    let programs = [declared_program()];
    for defect in 0..9 {
        let mut input = resident(&programs);
        let plan = ExecutionPlan::new(&input, &programs).unwrap();
        change_meaning(Arc::make_mut(&mut input.input.packed), defect);
        assert_eq!(
            plan.validate(&input),
            Err(Error::InvalidAnchor),
            "defect {defect}"
        );
    }
}

#[test]
fn prepared_plans_reject_changed_nfa_and_lua_tables_even_when_all_addresses_remain_valid() {
    let programs = [
        Program::new(
            vec![word(TEXT, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            vec![br"\bneedle|[a-z]".to_vec(), b"value".to_vec()],
            vec![TextPredicate::Regex {
                source: 0,
                compiled: regex::Regex::new(r"\bneedle|[a-z]").unwrap(),
            }],
            vec![1],
            vec![],
        )
        .unwrap(),
        Program::new(
            vec![word(TEXT, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            vec![b"^[a-z]+".to_vec(), b"value".to_vec()],
            vec![TextPredicate::LuaPattern {
                source: 0,
                compiled: LuaPattern::parse(b"^[a-z]+").unwrap(),
            }],
            vec![1],
            vec![],
        )
        .unwrap(),
    ];
    for defect in 0..6 {
        let mut input = resident(&programs);
        let plan = ExecutionPlan::new(&input, &programs).unwrap();
        let batch = Arc::make_mut(&mut input.input.packed);
        match defect {
            0 => {
                let range = &mut batch.nfa_ranges[0];
                if range.start < range.end {
                    range.start += 1;
                } else {
                    range.start = (range.start + 1) % 256;
                    range.end = range.start;
                }
            }
            1 => {
                let nfa = batch.values[0];
                let states = &mut batch.nfa_states
                    [nfa.state_start as usize..(nfa.state_start + nfa.state_count) as usize];
                let state = states.iter_mut().find(|state| state.kind == 3).unwrap();
                state.next = (state.next + 1) % nfa.state_count;
            }
            2 => batch.nfa_data[1] += 1,
            3 => batch.lua_instructions[0].matcher_first ^= 1,
            4 => batch.lua_members[0].first += 1,
            5 => {
                let index = batch.programs[1].value_start as usize;
                batch.values[index].operand ^= 1;
            }
            _ => unreachable!(),
        }
        assert_eq!(
            plan.validate(&input),
            Err(Error::InvalidAnchor),
            "defect {defect}"
        );
    }
}
