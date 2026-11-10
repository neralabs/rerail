use super::{
    ABSENT_FIELD, ASSERT, CAPTURE, CHILD, DESCEND, EMIT, FIELD, FIRST, FLAGS, FOLLOWING, FORK,
    GOTO, KIND, LAST, NEXT, PARENT, Program, REPEAT_BEGIN, REPEAT_BUMP, REPEAT_END, RESTORE_CURSOR,
    SAVE_CURSOR, SIZE, STOP, SUPERTYPE, TEXT, split,
    ACCEPT, CLOSE, EMPTY, LEAF, MARK, MERGE, RETAG, WRAP, ROW, NODE_FIELD, NODE_SUPER, OPEN, PUSH, RETURN, SPAN, SWITCH, TOKEN, WORD_IS,
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

/// The node a `RETAG` or a `NODE_FIELD` acts on, its mode: the innermost open node (a retag's)
/// or the last node (a field's); the node the last wrap moved; the node made last at the
/// innermost open node's level (`relabel`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum On {
    Here = 0,
    Moved = 1,
    Made = 2,
    /// A field's only: the node the node made last, a wrap, holds first.
    Inner = 3,
}

impl On {
    fn of(mode: u8) -> On {
        match mode {
            0 => On::Here,
            1 => On::Moved,
            2 => On::Made,
            _ => On::Inner,
        }
    }
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
    // Tier 2's words over the token stream (decision 0087, `stream`).
    /// The next start of tier 1's extract `extract`.
    Token { extract: u16 },
    Switch { table: u16 },
    /// The token's end: `len` bytes, or (`last`) one past the next position of extract `len`.
    Span { last: bool, len: u16 },
    /// Refuses unless tier 1's total `total` is zero.
    Empty { total: u16 },
    /// The token's bytes against a string: refuse where they differ, or with `test`, skip the
    /// next word.
    Word { string: u16, test: bool },
    Open { kind: u16 },
    /// At the end of this token (`this`), or of the one before.
    Close { this: bool },
    /// A token's leaf; `deferred`: an extra's, left out of the nodes closed before the next
    /// node (docs/parser-generators/24-rr-and-bitlex-plan.md §2.3).
    /// `extra`: an extra's leaf placed now, never the node made last at its level.
    Leaf { kind: u16, deferred: bool, extra: bool },
    /// Mark `order` (its bit in mask `mask` at the token's start; at the stream's end, `end`),
    /// if not yet taken nor passed: take it and run the next word; else skip that word.
    Mark { mask: u16, order: u8, end: bool },
    /// The token starts at a bit of mask `mask`: run the next word; else skip it.
    Row { mask: u16 },
    /// A node becomes a `kind`: the innermost open node, or another (`On`).
    Retag { kind: u16, on: On },
    /// The node made last at the innermost open node's level and the nodes inside it, read by
    /// reading `reading` (`RETAG m=3`, `reread`).
    Reread { reading: u16 },
    /// A node of `kind` around the node last made at this level, open: it takes that node's
    /// field and start (docs/parser-generators/24-rr-and-bitlex-plan.md §2.5).
    Wrap { kind: u16 },
    /// The token's leaf joins the grammar's leaf before it, as one leaf of `kind`; `fold`: the
    /// innermost open node's two leaves, all it holds, become one of `kind` (the token's leaf is
    /// not touched).
    Merge { kind: u16, fold: bool },
    /// A node's field: the last node's, or another's (`On`).
    NodeField { field: u16, on: On },
    NodeSuper { kind: u16 },
    Push { target: u16 },
    /// To switch on this token again (`again`), or to take the next.
    Return { again: bool },
    Accept,
}

impl StructuralOperation {
    pub(super) fn encoded(self) -> (u8, u8, u16) {
        match self {
            Self::Stop => (STOP, 0, 0),
            Self::Kind { kind } => (KIND, 0, kind),
            Self::Field { field } => (FIELD, 0, field),
            Self::Flags {
                required,
                forbidden,
            } => (FLAGS, required, forbidden),
            Self::Text {
                function: FunctionRef::Value(function),
            } => (TEXT, 0, function),
            Self::Size { mode, operand } => (SIZE, mode, operand),
            Self::Assert {
                function: FunctionRef::Capture(function),
            } => (ASSERT, 0, function),
            Self::First { named } => (FIRST, u8::from(named), 0),
            Self::Last { named } => (LAST, u8::from(named), 0),
            Self::AbsentField { field } => (ABSENT_FIELD, 0, field),
            Self::Supertype { kind } => (SUPERTYPE, 0, kind),
            Self::Child { named } => (CHILD, u8::from(named), 0),
            Self::Next { named } => (NEXT, u8::from(named), 0),
            Self::Parent => (PARENT, 0, 0),
            Self::Descend { named } => (DESCEND, u8::from(named), 0),
            Self::Following { named } => (FOLLOWING, u8::from(named), 0),
            Self::SaveCursor { slot } => (SAVE_CURSOR, 0, slot),
            Self::RestoreCursor { slot } => (RESTORE_CURSOR, 0, slot),
            Self::Capture { label } => (CAPTURE, 0, label),
            Self::RepeatBegin { id } => (REPEAT_BEGIN, 0, id),
            Self::RepeatBump { id } => (REPEAT_BUMP, 0, id),
            Self::RepeatEnd { id } => (REPEAT_END, 0, id),
            Self::Fork { greedy, target } => (FORK, u8::from(greedy), target),
            Self::Goto { target } => (GOTO, 0, target),
            Self::Emit { label, .. } => (EMIT, 0, label),
            Self::Token { extract } => (TOKEN, 0, extract),
            Self::Empty { total } => (EMPTY, 0, total),
            Self::Switch { table } => (SWITCH, 0, table),
            Self::Span { last, len } => (SPAN, u8::from(last), len),
            Self::Word { string, test } => (WORD_IS, u8::from(test), string),
            Self::Open { kind } => (OPEN, 0, kind),
            Self::Close { this } => (CLOSE, u8::from(this), 0),
            Self::Leaf { kind, deferred, extra } => (LEAF, if extra { 2 } else { u8::from(deferred) }, kind),
            Self::Mark { mask, order, end } => (MARK, order | if end { 32 } else { 0 }, mask),
            Self::Row { mask } => (ROW, 0, mask),
            Self::Retag { kind, on } => (RETAG, on as u8, kind),
            Self::Reread { reading } => (RETAG, 3, reading),
            Self::Wrap { kind } => (WRAP, 0, kind),
            Self::Merge { kind, fold } => (MERGE, u8::from(fold), kind),
            Self::NodeField { field, on } => (NODE_FIELD, on as u8, field),
            Self::NodeSuper { kind } => (NODE_SUPER, 0, kind),
            Self::Push { target } => (PUSH, 0, target),
            Self::Return { again } => (RETURN, u8::from(again), 0),
            Self::Accept => (ACCEPT, 0, 0),
            Self::Text { .. } | Self::Assert { .. } => {
                unreachable!("native lowering created an invalid function reference")
            }
        }
    }
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
    /// Set mode with one emitted label (`set_mode`): a task ends at its first emit.
    pub key_stop: bool,
    pub has_repeats: bool,
    pub maximum_cursor_slot: u32,
    pub reservation_sites: Vec<ReservationSite>,
    pub longs: Vec<u64>,
    pub strings: Vec<Vec<u8>>,
    /// Tier 2's switch tables (`SWITCH`).
    pub switches: Vec<super::stream::ByteSwitch>,
    pub readings: super::stream::Readings,
    pub value_programs: Vec<ValueProgram>,
    /// The block programs, lowered (`BlockNative`).
    pub block_programs: Vec<BlockNative>,
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
            TOKEN => StructuralOperation::Token { extract: operand },
            EMPTY => StructuralOperation::Empty { total: operand },
            SWITCH => StructuralOperation::Switch { table: operand },
            SPAN => StructuralOperation::Span { last: mode != 0, len: operand },
            WORD_IS => StructuralOperation::Word { string: operand, test: mode != 0 },
            OPEN => StructuralOperation::Open { kind: operand },
            CLOSE => StructuralOperation::Close { this: mode != 0 },
            LEAF => StructuralOperation::Leaf { kind: operand, deferred: mode == 1, extra: mode == 2 },
            MARK => StructuralOperation::Mark { mask: operand, order: mode % 32, end: mode & 32 != 0 },
            ROW => StructuralOperation::Row { mask: operand },
            RETAG if mode == 3 => StructuralOperation::Reread { reading: operand },
            RETAG => StructuralOperation::Retag { kind: operand, on: On::of(mode) },
            WRAP => StructuralOperation::Wrap { kind: operand },
            MERGE => StructuralOperation::Merge { kind: operand, fold: mode == 1 },
            NODE_FIELD => StructuralOperation::NodeField { field: operand, on: On::of(mode) },
            NODE_SUPER => StructuralOperation::NodeSuper { kind: operand },
            PUSH => StructuralOperation::Push { target: operand },
            RETURN => StructuralOperation::Return { again: mode != 0 },
            ACCEPT => StructuralOperation::Accept,
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
        key_stop: program.set_mode
            && super::set_mode::Reach::of(&program.words).is_some_and(|reach| reach.labels().len() == 1),
        has_repeats,
        maximum_cursor_slot,
        reservation_sites,
        longs: program.longs.clone(),
        strings: program.strings.clone(),
        switches: program.switches.clone(),
        readings: program.readings.clone(),
        value_programs: program.value_programs.clone(),
        block_programs: program.block_programs.iter().map(BlockNative::lower).collect(),
        capture_programs: program.capture_programs.clone(),
        output_program: program.output_program.clone(),
        output: FunctionRef::Output,
        reduction: FunctionRef::Reduction,
    })
}

// The block programs' native IR (`BlockNative`) is tier 1's, in its own crate.
pub(super) use rerail_tier1::block_native::*;

#[cfg(test)]
mod tests {
    use super::*;
    // G's tests (b90a7b0a), dropped by bb4c9605's merge; `policy_dsl::transducer` is `packed`.
    use super::super::{GREEDY, NAMED, Program, word};

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
                super::super::Error::InvalidTarget(_)
            ),
            "{error:?}"
        );
    }
}
