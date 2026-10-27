//! Pure structural entry shapes. No execution state or scheduler phases.
use super::{
    ABSENT_FIELD, ASSERT, CAPTURE, EMIT, FIELD, FIRST, FLAGS, FORK, GOTO, KIND, LAST, Program,
    SIZE, STOP, SUPERTYPE, TEXT, split,
};

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum ExecutionClass {
    Direct,
    DirectWithOutput,
    BoundedStructural,
    BoundedWithOutput,
    DynamicFrontier,
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum ExecutionRoute {
    Direct,
    DirectWithOutput,
    RegisterStructural,
    RegisterWithOutput,
    ChildExpansion,
    DescendantExpansion,
    FollowingExpansion,
    ControlExpansion,
    RepeatTransition,
    Unsupported,
    Predicate,
}

pub(super) fn execution_class(program: &Program) -> ExecutionClass {
    if program
        .capture_programs
        .iter()
        .any(capture_uses_shared_frame)
        || program
            .value_programs
            .iter()
            .any(|value| needs_value_frontier(value, &program.strings))
    {
        return ExecutionClass::DynamicFrontier;
    }
    let direct = program.words.iter().all(|instruction| {
        matches!(
            split(*instruction).0,
            STOP | KIND
                | FIELD
                | FLAGS
                | TEXT
                | SIZE
                | ASSERT
                | FIRST
                | LAST
                | ABSENT_FIELD
                | SUPERTYPE
                | CAPTURE
                | FORK
                | GOTO
                | EMIT
        )
    });
    let has_output = !program.predicates.is_empty()
        || !program.value_programs.is_empty()
        || !program.capture_predicates.is_empty()
        || !program.capture_programs.is_empty()
        || !program.directives.is_empty();
    if direct {
        if has_output {
            ExecutionClass::DirectWithOutput
        } else {
            ExecutionClass::Direct
        }
    } else {
        let dynamic = program.words.iter().any(|instruction| {
            matches!(split(*instruction).0, super::DESCEND | super::FOLLOWING)
                || matches!(
                    split(*instruction).0,
                    super::REPEAT_BEGIN | super::REPEAT_BUMP | super::REPEAT_END
                )
        });
        if dynamic {
            ExecutionClass::DynamicFrontier
        } else if has_output {
            ExecutionClass::BoundedWithOutput
        } else {
            ExecutionClass::BoundedStructural
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct ProgramRequirements {
    pub cursor_slots: u32,
    pub capture_operations: u32,
    pub child_sites: u32,
    pub descendant_sites: u32,
    pub following_sites: u32,
    pub repeat_sites: u32,
    pub instruction_count: u32,
}

/// Prove complete register work, rather than trusting the catalog's coarse class.
/// Paths contain instruction positions, so shared signatures still load each Program's operands.
pub(super) fn register_program_paths(program: &Program) -> Option<Vec<Vec<usize>>> {
    register_paths_for(&program.words, program.tree_sitter_query, |operand| {
        program
            .value_programs
            .get(usize::from(operand))
            .is_some_and(|value| !needs_value_frontier(value, &program.strings))
    })
}

/// Source and packed inputs use one complete-work eligibility rule. Deferred
/// query assertions need shared values but do not split their finite structure.
pub(super) fn register_paths_for(
    words: &[u32],
    query: bool,
    mut finite_text: impl FnMut(u16) -> bool,
) -> Option<Vec<Vec<usize>>> {
    register_paths_for_with_children(words, query, &mut finite_text, |_, _| false)
}

pub(super) fn register_paths_for_with_children(
    words: &[u32],
    query: bool,
    mut finite_text: impl FnMut(u16) -> bool,
    child: impl Fn(&[usize], usize) -> bool,
) -> Option<Vec<Vec<usize>>> {
    if words.iter().any(|&instruction| {
        let (op, _, operand) = split(instruction);
        (op == ASSERT && !query) || (op == TEXT && !finite_text(operand))
    }) {
        return None;
    }
    if words.iter().any(|&word| split(word).0 == super::CHILD) {
        register_paths_with_children(words, &child)
    } else {
        register_paths(words)
    }
}

pub(super) const REGISTER_VISIT_LIMIT: usize = 4096;

pub(super) fn register_paths(words: &[u32]) -> Option<Vec<Vec<usize>>> {
    register_paths_with_children(words, &|_, _| false)
}

fn register_paths_with_children(
    words: &[u32],
    child: &dyn Fn(&[usize], usize) -> bool,
) -> Option<Vec<Vec<usize>>> {
    fn visit(
        words: &[u32],
        pc: usize,
        active: &mut BTreeSet<usize>,
        path: &mut Vec<usize>,
        outputs: &mut Vec<Vec<usize>>,
        visits: &mut usize,
        child: &dyn Fn(&[usize], usize) -> bool,
    ) -> Option<()> {
        *visits += 1;
        if *visits > REGISTER_VISIT_LIMIT
            || active.len() >= 256
            || path.len() >= 64
            || outputs.len() >= 256
            || !active.insert(pc)
        {
            return None;
        }
        let (op, mode, operand) = split(*words.get(pc)?);
        match op {
            STOP => {}
            GOTO if mode == 0 => visit(
                words,
                operand as usize,
                active,
                path,
                outputs,
                visits,
                child,
            )?,
            FORK if mode == 0 => {
                visit(words, pc + 1, active, path, outputs, visits, child)?;
                visit(
                    words,
                    operand as usize,
                    active,
                    path,
                    outputs,
                    visits,
                    child,
                )?;
            }
            EMIT => {
                let mut complete = path.clone();
                complete.push(pc);
                outputs.push(complete);
            }
            super::CHILD if child(path, pc) => {
                path.push(pc);
                visit(words, pc + 1, active, path, outputs, visits, child)?;
                path.pop();
            }
            KIND
            | FIELD
            | FLAGS
            | TEXT
            | SIZE
            | ASSERT
            | FIRST
            | LAST
            | ABSENT_FIELD
            | SUPERTYPE
            | super::NEXT
            | super::PARENT
            | CAPTURE
            | super::SAVE_CURSOR
            | super::RESTORE_CURSOR => {
                let mut slots = BTreeSet::new();
                let mut live_slots = BTreeSet::new();
                let mut captures = 0;
                for &position in path.iter() {
                    let (previous, _, slot) = split(words[position]);
                    if previous == super::SAVE_CURSOR {
                        slots.insert(slot);
                        // ExecutionArena::save_cursor resizes the frame, so a lower
                        // save invalidates every higher slot. Declaration count still
                        // includes all registers used by this finite path.
                        live_slots.retain(|&saved| saved <= slot);
                        live_slots.insert(slot);
                    }
                    captures += usize::from(previous == CAPTURE);
                }
                if op == super::RESTORE_CURSOR && !live_slots.contains(&operand) {
                    return None;
                }
                if op == super::SAVE_CURSOR {
                    slots.insert(operand);
                }
                if slots.len() > 8 || captures + usize::from(op == CAPTURE) > 8 {
                    return None;
                }
                path.push(pc);
                visit(words, pc + 1, active, path, outputs, visits, child)?;
                path.pop();
            }
            // Expansion is not bounded by the number of instructions in a Program.
            _ => return None,
        }
        active.remove(&pc);
        Some(())
    }
    use std::collections::BTreeSet;
    let mut paths = Vec::new();
    visit(
        words,
        0,
        &mut BTreeSet::new(),
        &mut Vec::new(),
        &mut paths,
        &mut 0,
        child,
    )?;
    Some(paths)
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EntryBoundary {
    Stop,
    Emit,
    Child,
    Descend,
    Following,
    Fork,
    Goto,
    RepeatBegin,
    RepeatBump,
    RepeatEnd,
    Unsupported,
    Predicate,
    ValueStart,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct EntryShape {
    pub signature: u32,
    pub route: ExecutionRoute,
    pub cursor_slots: u32,
}
const MAX_ENTRY_CURSOR_REGISTERS: usize = 8;
fn entry_boundary(opcode: u8) -> Option<EntryBoundary> {
    Some(match opcode {
        STOP => EntryBoundary::Stop,
        ASSERT => EntryBoundary::Predicate,
        EMIT => EntryBoundary::Emit,
        super::CHILD => EntryBoundary::Child,
        super::DESCEND => EntryBoundary::Descend,
        super::FOLLOWING => EntryBoundary::Following,
        FORK => EntryBoundary::Fork,
        GOTO => EntryBoundary::Goto,
        super::REPEAT_BEGIN => EntryBoundary::RepeatBegin,
        super::REPEAT_BUMP => EntryBoundary::RepeatBump,
        super::REPEAT_END => EntryBoundary::RepeatEnd,
        _ => return None,
    })
}

fn metal_deterministic_opcode(opcode: u8) -> bool {
    matches!(
        opcode,
        TEXT | KIND
            | FIELD
            | FLAGS
            | SIZE
            | FIRST
            | LAST
            | ABSENT_FIELD
            | SUPERTYPE
            | GOTO
            | CAPTURE
            | super::PARENT
            | super::NEXT
            | super::SAVE_CURSOR
            | super::RESTORE_CURSOR
    )
}

pub(super) fn value_execution_signature(value: &super::value::ValueProgram) -> Vec<u32> {
    match value {
        super::value::ValueProgram::Equal { .. } => vec![0],
        super::value::ValueProgram::Prefix { .. } => vec![1],
        super::value::ValueProgram::Regex(nfa) => {
            use super::value::NfaState;
            let mut key = vec![2, nfa.states.len() as u32, nfa.start, nfa.anchored_start];
            for state in &nfa.states {
                match state {
                    NfaState::Bytes(ranges) => {
                        key.extend([0, ranges.len() as u32]);
                        key.extend(ranges.iter().map(|range| range.2));
                    }
                    NfaState::Look { kind, next } => key.extend([1, *kind as u32, *next]),
                    NfaState::Split(next) => {
                        key.extend([2, next.len() as u32]);
                        key.extend(next);
                    }
                    NfaState::Capture { slot, next } => key.extend([3, *slot, *next]),
                    NfaState::Fail => key.push(4),
                    NfaState::Match => key.push(5),
                }
            }
            key
        }
        super::value::ValueProgram::Lua(_) => {
            let bytes = value.canonical_bytes();
            let mut key = vec![3, bytes.len() as u32];
            key.extend(bytes.chunks(4).map(|chunk| {
                let mut word = [0; 4];
                word[..chunk.len()].copy_from_slice(chunk);
                u32::from_le_bytes(word)
            }));
            key
        }
    }
}

pub(super) fn needs_value_frontier(
    value: &super::value::ValueProgram,
    strings: &[Vec<u8>],
) -> bool {
    match value {
        super::value::ValueProgram::Equal { string }
        | super::value::ValueProgram::Prefix { string } => strings[*string as usize].len() > 64,
        super::value::ValueProgram::Regex(_) | super::value::ValueProgram::Lua(_) => true,
    }
}

fn capture_requires_frontier(program: &Program, assertion: u16) -> bool {
    let capture = &program.capture_programs[assertion as usize];
    capture_uses_shared_frame(capture)
        || matches!(capture, super::value::CaptureProgram::Text { .. })
}

fn capture_uses_shared_frame(capture: &super::value::CaptureProgram) -> bool {
    matches!(
        capture,
        super::value::CaptureProgram::Equal { .. }
            | super::value::CaptureProgram::Kind { .. }
            | super::value::CaptureProgram::SameLine { .. }
            | super::value::CaptureProgram::AnyOf { .. }
            | super::value::CaptureProgram::Contains { .. }
            | super::value::CaptureProgram::Relation { .. }
    )
}

pub(super) fn cursor_layouts(programs: &[Program]) -> (Vec<(u32, u32)>, Vec<u32>) {
    let mut table = Vec::new();
    let mut intern = std::collections::BTreeMap::<Vec<u32>, u32>::new();
    let layouts = programs
        .iter()
        .map(|program| {
            let slots = program
                .words
                .iter()
                .filter_map(|instruction| {
                    let (opcode, _, operand) = split(*instruction);
                    matches!(opcode, super::SAVE_CURSOR | super::RESTORE_CURSOR)
                        .then_some(u32::from(operand))
                })
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            let count = slots.len() as u32;
            let start = *intern.entry(slots.clone()).or_insert_with(|| {
                let start = table.len() as u32;
                table.extend(slots);
                start
            });
            (start, count)
        })
        .collect();
    (layouts, table)
}

/// Describe only the first bounded structural entry. Keys contain no scheduler addresses.
pub(super) fn compile_entry_shapes(programs: &[Program]) -> Vec<EntryShape> {
    let mut signatures = std::collections::BTreeMap::<Vec<u32>, u32>::new();
    let (cursor_layouts, _) = cursor_layouts(programs);
    programs
        .iter()
        .enumerate()
        .map(|(program_index, program)| {
            let cursor_cache_limit = if cursor_layouts[program_index].1 > 64 {
                3
            } else {
                MAX_ENTRY_CURSOR_REGISTERS
            };
            let start = 0;
            let mut terminal_pc = start;
            let mut supported = true;
            let mut cached_slots = std::collections::BTreeSet::new();
            let mut register_split = false;
            let mut query_join = false;
            while terminal_pc < program.words.len() {
                let (opcode, _, operand) = split(program.words[terminal_pc]);
                if opcode == super::TEXT
                    && needs_value_frontier(
                        &program.value_programs[split(program.words[terminal_pc]).2 as usize],
                        &program.strings,
                    )
                {
                    break;
                }
                if terminal_pc - start >= 64 {
                    register_split = true;
                    break;
                }
                let adjacent_goto = opcode == GOTO && usize::from(operand) == terminal_pc + 1;
                if entry_boundary(opcode).is_some() && !adjacent_goto {
                    break;
                }
                query_join |= matches!(
                    opcode,
                    super::SAVE_CURSOR | super::RESTORE_CURSOR | super::NEXT | super::PARENT
                );
                if opcode == super::SAVE_CURSOR {
                    let slot = split(program.words[terminal_pc]).2;
                    if !cached_slots.contains(&slot) && cached_slots.len() == cursor_cache_limit {
                        register_split = true;
                        break;
                    }
                    cached_slots.insert(slot);
                }
                supported &= metal_deterministic_opcode(opcode);
                terminal_pc += 1;
            }
            let (terminal_kind, terminal_mode, terminal_operand) = if register_split {
                (EntryBoundary::Goto as u32, 0, terminal_pc as u32)
            } else if terminal_pc < program.words.len() {
                let (opcode, mode, operand) = split(program.words[terminal_pc]);
                (
                    if opcode == super::TEXT
                        || (opcode == ASSERT && capture_requires_frontier(program, operand))
                    {
                        EntryBoundary::ValueStart
                    } else {
                        entry_boundary(opcode).unwrap_or(EntryBoundary::Unsupported)
                    } as u32,
                    u32::from(mode),
                    u32::from(operand),
                )
            } else {
                (EntryBoundary::Stop as u32, 0, 0)
            };
            // Entry signatures distinguish repeat commitment and query identity requirements.
            if start == 0
                && !program.words.iter().any(|instruction| {
                    let (opcode, _, operand) = split(*instruction);
                    matches!(opcode, GOTO | FORK) && operand == 0
                })
            {
                query_join = false;
            }
            let prefix_flags = u32::from(query_join) * 4;
            let mut prefix_slots = std::collections::BTreeMap::new();
            let mut signature_key = Vec::new();
            for &instruction in &program.words[start..terminal_pc] {
                let (opcode, _, operand) = split(instruction);
                signature_key.push(instruction & 0xffff);
                let slot = if opcode == super::SAVE_CURSOR {
                    let next = prefix_slots.len() as u32;
                    *prefix_slots.entry(operand).or_insert(next)
                } else if opcode == super::RESTORE_CURSOR {
                    prefix_slots.get(&operand).copied().unwrap_or(u32::MAX)
                } else {
                    0
                };
                signature_key.push(slot);
            }
            signature_key.extend([
                u32::MAX,
                terminal_kind,
                terminal_mode,
                prefix_flags,
                cursor_layouts[program_index].1,
                cached_slots.len() as u32,
                u32::from(program.tree_sitter_query),
                u32::from(
                    program
                        .words
                        .iter()
                        .any(|instruction| split(*instruction).0 == super::REPEAT_BEGIN),
                ),
                // Blocks share an execution route. A proved repetition must
                // never absorb another Program with only the same entry.
                u32::from(super::terminal_repeat_proof::prove(program).is_some()),
            ]);
            if terminal_kind == EntryBoundary::Predicate as u32 {
                use super::value::CaptureProgram;
                let resource_key = match &program.capture_programs[terminal_operand as usize] {
                    CaptureProgram::Equal { negated, any, .. } => {
                        vec![0, u32::from(*negated), u32::from(*any)]
                    }
                    CaptureProgram::Text {
                        value,
                        negated,
                        any,
                        ..
                    } => vec![
                        1,
                        u32::from(*negated),
                        u32::from(*any),
                        match &program.value_programs[*value as usize] {
                            super::value::ValueProgram::Equal { .. } => 0,
                            super::value::ValueProgram::Prefix { .. } => 1,
                            super::value::ValueProgram::Regex(nfa) => 2 + nfa.states.len() as u32,
                            super::value::ValueProgram::Lua(lua) => 0x80000000 | lua.capture_count,
                        },
                    ],
                    CaptureProgram::AnyOf {
                        strings,
                        negated,
                        any,
                        ..
                    } => vec![
                        2,
                        strings.len() as u32,
                        u32::from(*negated),
                        u32::from(*any),
                    ],
                    CaptureProgram::Kind {
                        kinds,
                        negated,
                        any,
                        empty_all,
                        ..
                    } => vec![
                        3,
                        kinds.len() as u32,
                        u32::from(*negated),
                        u32::from(*any),
                        u32::from(*empty_all),
                    ],
                    CaptureProgram::Contains {
                        strings,
                        negated,
                        any,
                        ..
                    } => vec![
                        4,
                        strings.len() as u32,
                        u32::from(*negated),
                        u32::from(*any),
                    ],
                    CaptureProgram::Relation {
                        kinds,
                        ancestor,
                        negated,
                        ..
                    } => vec![
                        5,
                        kinds.len() as u32,
                        u32::from(*ancestor),
                        u32::from(*negated),
                    ],
                    CaptureProgram::SameLine { negated, .. } => vec![6, u32::from(*negated)],
                };
                signature_key.extend(resource_key);
            }
            // Predicate implementation and generated scalar layout are part of a stage's
            // signature even when TEXT occurs inside its deterministic prefix.
            for instruction in &program.words[start..terminal_pc] {
                let (opcode, _, operand) = split(*instruction);
                if opcode == super::TEXT {
                    let value =
                        value_execution_signature(&program.value_programs[operand as usize]);
                    signature_key.extend([u32::MAX - 1, value.len() as u32]);
                    signature_key.extend(value);
                } else if opcode == super::SAVE_CURSOR || opcode == super::RESTORE_CURSOR {
                    signature_key.extend([u32::MAX - 2, u32::from(operand)]);
                }
            }
            if terminal_kind == EntryBoundary::Predicate as u32
                && let super::value::CaptureProgram::Text { value, .. } =
                    &program.capture_programs[terminal_operand as usize]
            {
                signature_key.extend(value_execution_signature(
                    &program.value_programs[*value as usize],
                ));
            }
            let next = signatures.len() as u32;
            let signature = *signatures.entry(signature_key).or_insert(next);
            EntryShape {
                signature,
                route: entry_route(program, terminal_kind, supported),
                cursor_slots: cursor_layouts[program_index].1,
            }
        })
        .collect()
}
fn entry_route(program: &Program, boundary: u32, supported: bool) -> ExecutionRoute {
    let class = execution_class(program);
    if class == ExecutionClass::Direct {
        return ExecutionRoute::Direct;
    }
    if class == ExecutionClass::DirectWithOutput {
        return ExecutionRoute::DirectWithOutput;
    }
    if !supported {
        return ExecutionRoute::Unsupported;
    }
    match boundary {
        kind if kind == EntryBoundary::Child as u32 => ExecutionRoute::ChildExpansion,
        kind if kind == EntryBoundary::Descend as u32 => ExecutionRoute::DescendantExpansion,
        kind if kind == EntryBoundary::Following as u32 => ExecutionRoute::FollowingExpansion,
        kind if kind == EntryBoundary::Fork as u32 || kind == EntryBoundary::Goto as u32 => {
            ExecutionRoute::ControlExpansion
        }
        kind if kind == EntryBoundary::RepeatBegin as u32
            || kind == EntryBoundary::RepeatBump as u32
            || kind == EntryBoundary::RepeatEnd as u32 =>
        {
            ExecutionRoute::RepeatTransition
        }
        kind if kind == EntryBoundary::Predicate as u32
            || kind == EntryBoundary::ValueStart as u32 =>
        {
            ExecutionRoute::Predicate
        }
        kind if kind == EntryBoundary::Stop as u32 || kind == EntryBoundary::Emit as u32 => {
            if class == ExecutionClass::BoundedWithOutput {
                ExecutionRoute::RegisterWithOutput
            } else {
                ExecutionRoute::RegisterStructural
            }
        }
        _ => ExecutionRoute::Unsupported,
    }
}

pub(super) fn program_requirements(program: &Program) -> ProgramRequirements {
    let mut requirements = ProgramRequirements {
        instruction_count: program.words.len() as u32,
        cursor_slots: cursor_layouts(std::slice::from_ref(program)).0[0].1,
        ..ProgramRequirements::default()
    };
    for instruction in &program.words {
        let (opcode, _, _) = split(*instruction);
        match opcode {
            CAPTURE => requirements.capture_operations += 1,
            super::CHILD => requirements.child_sites += 1,
            super::DESCEND => requirements.descendant_sites += 1,
            super::FOLLOWING => requirements.following_sites += 1,
            super::REPEAT_BEGIN | super::REPEAT_BUMP | super::REPEAT_END => {
                requirements.repeat_sites += 1;
            }
            _ => {}
        }
    }
    requirements
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy_dsl::transducer::{
        CHILD, DESCEND, NEXT, PARENT, RESTORE_CURSOR, SAVE_CURSOR, word,
    };

    #[test]
    fn structural_entry_routes_follow_bounded_navigation_and_expansion() {
        let make =
            |words| Program::new(words, vec![b"match".to_vec()], vec![], vec![0], vec![]).unwrap();
        let programs = [
            make(vec![word(KIND, 0, 1), word(EMIT, 0, 0)]),
            make(vec![word(PARENT, 0, 0), word(EMIT, 0, 0)]),
            make(vec![word(CHILD, 0, 0), word(EMIT, 0, 0)]),
            make(vec![word(DESCEND, 0, 0), word(EMIT, 0, 0)]),
            make(vec![word(super::super::FOLLOWING, 0, 0), word(EMIT, 0, 0)]),
        ];
        assert_eq!(
            compile_entry_shapes(&programs)
                .iter()
                .map(|shape| shape.route)
                .collect::<Vec<_>>(),
            [
                ExecutionRoute::Direct,
                ExecutionRoute::RegisterStructural,
                ExecutionRoute::ChildExpansion,
                ExecutionRoute::DescendantExpansion,
                ExecutionRoute::FollowingExpansion
            ]
        );
    }

    #[test]
    fn entry_signatures_keep_cursor_identity_without_sparse_id_capacity() {
        let make = |second, restore| {
            Program::new(
                vec![
                    word(SAVE_CURSOR, 0, 7),
                    word(NEXT, 1, 0),
                    word(SAVE_CURSOR, 0, second),
                    word(RESTORE_CURSOR, 0, restore),
                    word(DESCEND, 0, 0),
                    word(EMIT, 0, 0),
                ],
                vec![b"match".to_vec()],
                vec![],
                vec![0],
                vec![],
            )
            .unwrap()
        };
        let programs = [make(7, 7), make(u16::MAX, u16::MAX), make(u16::MAX, 7)];
        let entries = compile_entry_shapes(&programs);
        assert_eq!(
            entries
                .iter()
                .map(|shape| shape.cursor_slots)
                .collect::<Vec<_>>(),
            [1, 2, 2]
        );
        assert_ne!(entries[1].signature, entries[2].signature);
        assert_eq!(program_requirements(&programs[1]).cursor_slots, 2);
    }

    #[test]
    fn compatible_entries_share_blocks_despite_different_unreached_suffixes() {
        let make = |tail| {
            Program::new(
                vec![word(DESCEND, 0, 0), tail, word(EMIT, 0, 0)],
                vec![b"match".to_vec()],
                vec![],
                vec![0],
                vec![],
            )
            .unwrap()
        };
        let entries = compile_entry_shapes(&[make(word(CAPTURE, 0, 0)), make(word(KIND, 0, 3))]);
        assert_eq!(entries[0], entries[1]);
    }
}
