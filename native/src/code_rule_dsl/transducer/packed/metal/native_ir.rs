use super::{
    ABSENT_FIELD, ASSERT, CAPTURE, CHILD, DESCEND, EMIT, FIELD, FIRST, FLAGS, FOLLOWING, FORK,
    GOTO, KIND, LAST, NEXT, PARENT, Program, REPEAT_BEGIN, REPEAT_BUMP, REPEAT_END, RESTORE_CURSOR,
    SAVE_CURSOR, SIZE, STOP, SUPERTYPE, TEXT, split,
    value::{CaptureProgram, ValueProgram},
};

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum ArenaKind {
    Pending,
    Capture,
    Cursor,
    Repeat,
    QueryBucket,
    QueryMember,
    QueuedState,
    GreedyEntry,
    ValueState,
    Candidate,
    Result,
    FinalCapture,
    Property,
    OutputByte,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReservationStrategy {
    Chunked,
    Exact,
    LazyBlock,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReservationOrigin {
    Structural(u16),
    Scheduler,
    Reduction,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ReservationSite {
    pub arena: ArenaKind,
    pub strategy: ReservationStrategy,
    pub origin: ReservationOrigin,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FunctionRef {
    Value(u16),
    Capture(u16),
    Output,
    Reduction,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StructuralOperation {
    Stop,
    Kind { kind: u16 },
    Field { field: u16 },
    Flags { required: u8, forbidden: u16 },
    Text { function: FunctionRef },
    Size { mode: u8, operand: u16 },
    Assert { function: FunctionRef },
    First { named: bool },
    Last { named: bool },
    AbsentField { field: u16 },
    Supertype { kind: u16 },
    Child { named: bool },
    Next { named: bool },
    Parent,
    Descend { named: bool },
    Following { named: bool },
    SaveCursor { slot: u16 },
    RestoreCursor { slot: u16 },
    Capture { label: u16 },
    RepeatBegin { id: u16 },
    RepeatBump { id: u16 },
    RepeatEnd { id: u16 },
    Fork { greedy: bool, target: u16 },
    Goto { target: u16 },
    Emit { label: u16, output: FunctionRef },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct StructuralBlock {
    pub position: u16,
    pub operation: StructuralOperation,
    pub next: Option<u16>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct NativeModule {
    pub structural: Vec<StructuralBlock>,
    pub value_functions: u16,
    pub capture_functions: u16,
    pub tree_sitter_query: bool,
    pub has_repeats: bool,
    pub maximum_cursor_slot: u32,
    pub reservation_sites: Vec<ReservationSite>,
    pub longs: Vec<u64>,
    pub strings: Vec<Vec<u8>>,
    pub value_programs: Vec<ValueProgram>,
    pub capture_programs: Vec<CaptureProgram>,
    pub output_program: super::output::OutputProgram,
    pub output: FunctionRef,
    pub reduction: FunctionRef,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LoweringError {
    EmptyProgram,
    TooManyInstructions,
    InvalidOpcode { position: u16, opcode: u8 },
    InvalidTarget { position: u16, target: u16 },
    InvalidValueFunction { position: u16, function: u16 },
    InvalidCaptureFunction { position: u16, function: u16 },
}

fn next_position(position: usize, length: usize) -> Option<u16> {
    (position + 1 < length).then_some((position + 1) as u16)
}

fn structural_reservations(block: StructuralBlock) -> impl Iterator<Item = ReservationSite> {
    let arena = match block.operation {
        StructuralOperation::Child { .. }
        | StructuralOperation::Descend { .. }
        | StructuralOperation::Following { .. }
        | StructuralOperation::Fork { .. } => Some(ArenaKind::Pending),
        StructuralOperation::Capture { .. } => Some(ArenaKind::Capture),
        StructuralOperation::SaveCursor { .. } => Some(ArenaKind::Cursor),
        StructuralOperation::RepeatBegin { .. } => Some(ArenaKind::Repeat),
        StructuralOperation::Emit { .. } => Some(ArenaKind::Candidate),
        _ => None,
    };
    arena.into_iter().map(move |arena| ReservationSite {
        arena,
        strategy: ReservationStrategy::Chunked,
        origin: ReservationOrigin::Structural(block.position),
    })
}

fn runtime_reservations(tree_sitter_query: bool) -> impl Iterator<Item = ReservationSite> {
    let mut sites = vec![
        ReservationSite {
            arena: ArenaKind::ValueState,
            strategy: ReservationStrategy::Chunked,
            origin: ReservationOrigin::Scheduler,
        },
        ReservationSite {
            arena: ArenaKind::GreedyEntry,
            strategy: ReservationStrategy::LazyBlock,
            origin: ReservationOrigin::Scheduler,
        },
        ReservationSite {
            arena: ArenaKind::Result,
            strategy: ReservationStrategy::Exact,
            origin: ReservationOrigin::Reduction,
        },
        ReservationSite {
            arena: ArenaKind::FinalCapture,
            strategy: ReservationStrategy::Exact,
            origin: ReservationOrigin::Reduction,
        },
        ReservationSite {
            arena: ArenaKind::Property,
            strategy: ReservationStrategy::Exact,
            origin: ReservationOrigin::Reduction,
        },
        ReservationSite {
            arena: ArenaKind::OutputByte,
            strategy: ReservationStrategy::Exact,
            origin: ReservationOrigin::Reduction,
        },
    ];
    if tree_sitter_query {
        sites.extend([
            ReservationSite {
                arena: ArenaKind::QueryBucket,
                strategy: ReservationStrategy::LazyBlock,
                origin: ReservationOrigin::Scheduler,
            },
            ReservationSite {
                arena: ArenaKind::QueryMember,
                strategy: ReservationStrategy::Chunked,
                origin: ReservationOrigin::Scheduler,
            },
            ReservationSite {
                arena: ArenaKind::QueuedState,
                strategy: ReservationStrategy::LazyBlock,
                origin: ReservationOrigin::Scheduler,
            },
        ]);
    }
    sites.into_iter()
}

pub(super) fn lower(program: &Program) -> Result<NativeModule, LoweringError> {
    if program.words.is_empty() {
        return Err(LoweringError::EmptyProgram);
    }
    if program.words.len() > usize::from(u16::MAX) + 1 {
        return Err(LoweringError::TooManyInstructions);
    }
    let value_functions = u16::try_from(program.value_programs.len())
        .map_err(|_| LoweringError::TooManyInstructions)?;
    let capture_functions = u16::try_from(program.capture_programs.len())
        .map_err(|_| LoweringError::TooManyInstructions)?;
    let mut structural = Vec::with_capacity(program.words.len());
    for (position, word) in program.words.iter().copied().enumerate() {
        let position = position as u16;
        let (opcode, mode, operand) = split(word);
        let operation = match opcode {
            STOP => StructuralOperation::Stop,
            KIND => StructuralOperation::Kind { kind: operand },
            FIELD => StructuralOperation::Field { field: operand },
            FLAGS => StructuralOperation::Flags {
                required: mode,
                forbidden: operand,
            },
            TEXT => {
                if operand >= value_functions {
                    return Err(LoweringError::InvalidValueFunction {
                        position,
                        function: operand,
                    });
                }
                StructuralOperation::Text {
                    function: FunctionRef::Value(operand),
                }
            }
            SIZE => StructuralOperation::Size { mode, operand },
            ASSERT => {
                if operand >= capture_functions {
                    return Err(LoweringError::InvalidCaptureFunction {
                        position,
                        function: operand,
                    });
                }
                StructuralOperation::Assert {
                    function: FunctionRef::Capture(operand),
                }
            }
            FIRST => StructuralOperation::First { named: mode != 0 },
            LAST => StructuralOperation::Last { named: mode != 0 },
            ABSENT_FIELD => StructuralOperation::AbsentField { field: operand },
            SUPERTYPE => StructuralOperation::Supertype { kind: operand },
            CHILD => StructuralOperation::Child { named: mode != 0 },
            NEXT => StructuralOperation::Next { named: mode != 0 },
            PARENT => StructuralOperation::Parent,
            DESCEND => StructuralOperation::Descend { named: mode != 0 },
            FOLLOWING => StructuralOperation::Following { named: mode != 0 },
            SAVE_CURSOR => StructuralOperation::SaveCursor { slot: operand },
            RESTORE_CURSOR => StructuralOperation::RestoreCursor { slot: operand },
            CAPTURE => StructuralOperation::Capture { label: operand },
            REPEAT_BEGIN => StructuralOperation::RepeatBegin { id: operand },
            REPEAT_BUMP => StructuralOperation::RepeatBump { id: operand },
            REPEAT_END => StructuralOperation::RepeatEnd { id: operand },
            FORK => {
                if usize::from(operand) >= program.words.len() {
                    return Err(LoweringError::InvalidTarget {
                        position,
                        target: operand,
                    });
                }
                StructuralOperation::Fork {
                    greedy: mode != 0,
                    target: operand,
                }
            }
            GOTO => {
                if usize::from(operand) >= program.words.len() {
                    return Err(LoweringError::InvalidTarget {
                        position,
                        target: operand,
                    });
                }
                StructuralOperation::Goto { target: operand }
            }
            EMIT => StructuralOperation::Emit {
                label: operand,
                output: FunctionRef::Output,
            },
            _ => {
                return Err(LoweringError::InvalidOpcode { position, opcode });
            }
        };
        structural.push(StructuralBlock {
            position,
            operation,
            next: next_position(usize::from(position), program.words.len()),
        });
    }
    let has_repeats = structural
        .iter()
        .any(|block| matches!(block.operation, StructuralOperation::RepeatBegin { .. }));
    let maximum_cursor_slot = structural
        .iter()
        .filter_map(|block| match block.operation {
            StructuralOperation::SaveCursor { slot }
            | StructuralOperation::RestoreCursor { slot } => Some(u32::from(slot) + 1),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let reservation_sites = structural
        .iter()
        .copied()
        .flat_map(structural_reservations)
        .chain(runtime_reservations(program.tree_sitter_query))
        .collect();
    Ok(NativeModule {
        structural,
        value_functions,
        capture_functions,
        tree_sitter_query: program.tree_sitter_query,
        has_repeats,
        maximum_cursor_slot,
        reservation_sites,
        longs: program.longs.clone(),
        strings: program.strings.clone(),
        value_programs: program.value_programs.clone(),
        capture_programs: program.capture_programs.clone(),
        output_program: program.output_program.clone(),
        output: FunctionRef::Output,
        reduction: FunctionRef::Reduction,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy_dsl::transducer::{GREEDY, NAMED, Program, word};

    #[test]
    fn native_ir_decodes_control_flow_without_a_host_call_operation() {
        let program = Program::new(
            vec![
                word(KIND, 0, 7),
                word(FORK, GREEDY, 4),
                word(CHILD, NAMED, 0),
                word(GOTO, 0, 4),
                word(EMIT, 0, 0),
            ],
            vec![b"unused".to_vec(), b"unused".to_vec(), b"match".to_vec()],
            Vec::new(),
            vec![2],
            Vec::new(),
        )
        .unwrap();
        let native = lower(&program).unwrap();
        assert_eq!(native.structural.len(), 5);
        assert_eq!(
            native.structural[1].operation,
            StructuralOperation::Fork {
                greedy: true,
                target: 4,
            }
        );
        assert_eq!(
            native.structural[2].operation,
            StructuralOperation::Child { named: true }
        );
        assert_eq!(native.structural[3].next, Some(4));
        assert_eq!(native.structural[4].next, None);
        assert!(!native.tree_sitter_query);
        assert!(!native.has_repeats);
        assert_eq!(native.maximum_cursor_slot, 0);
        assert!(native.reservation_sites.contains(&ReservationSite {
            arena: ArenaKind::Pending,
            strategy: ReservationStrategy::Chunked,
            origin: ReservationOrigin::Structural(1),
        }));
        assert!(native.reservation_sites.contains(&ReservationSite {
            arena: ArenaKind::Pending,
            strategy: ReservationStrategy::Chunked,
            origin: ReservationOrigin::Structural(2),
        }));
        assert!(native.reservation_sites.contains(&ReservationSite {
            arena: ArenaKind::Result,
            strategy: ReservationStrategy::Exact,
            origin: ReservationOrigin::Reduction,
        }));
        assert!(
            !native
                .reservation_sites
                .iter()
                .any(|site| site.arena == ArenaKind::QueryBucket)
        );
    }

    #[test]
    fn program_verification_rejects_an_invalid_branch_target_before_native_lowering() {
        let Err(error) = Program::new(
            vec![word(GOTO, 0, 2)],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ) else {
            panic!("invalid target was accepted");
        };
        assert!(
            matches!(
                error,
                crate::policy_dsl::transducer::Error::InvalidTarget(_)
            ),
            "{error:?}"
        );
    }
}
