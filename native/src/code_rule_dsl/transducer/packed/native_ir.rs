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

/// A block program lowered for the code generators: the ephemeral native IR that Cranelift
/// (`native_cpu_block`) and MSL (`metal_block_program`) both emit, one straight-line body per
/// 64-byte block. Each instruction is one value, a mask word, built only from bit operations;
/// there is no branch and no loop inside a body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct BlockNative {
    pub tables: Vec<super::block::ClassTable>,
    pub insts: Vec<BlockInst>,
    /// The values written as masks. Output `k` is value `outputs[k]` at delay
    /// `output_delays[k]`: every backend writes the stream's mask, bit `p` the value's bit
    /// `p + d`, cut at the stream's end. No host step shifts it.
    pub outputs: Vec<u16>,
    /// The outputs (indexes into `outputs`) whose positions the extract pass writes: each set
    /// bit of the stream's mask as its byte position, in order, at the offset of the bits before
    /// it (a running count, carried from block to block).
    pub extracts: Vec<usize>,
    /// The outputs (indexes into `outputs`) tested: each file's flag, whether the stream's mask
    /// has a bit (`block::BlockProgram::totals`).
    pub totals: Vec<usize>,
    /// The outputs (indexes into `outputs`) tier 2 reads as masks (`block::BlockProgram::masks`).
    pub masks: Vec<usize>,
    /// How many bytes past the stream the program runs (`block::BlockProgram::delay`).
    pub delay: u8,
    /// Each output's delay (`block::BlockProgram::output_delay`).
    pub output_delays: Vec<u8>,
    /// The fixed points (`block::BlockProgram::rounds`): each group's members and its carries
    /// with their nexts.
    pub groups: Vec<super::block::Group>,
}

/// An instruction of a block body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum BlockInst {
    /// The block's bytes compared with a byte, as a mask: on the CPU four 16-byte compares and a
    /// bitmask; on Metal SWAR over the lane's `uint4`s.
    EqByte(u8),
    /// The block's bytes in any class of `bits` of table `table`: on the CPU two table lookups
    /// per 16 bytes (`tbl`); on Metal, see `metal_block_program::ClassStrategy`.
    Class { table: u16, bits: u8 },
    /// Bit `k` of every byte of the block: one bit-plane (`BlockNative::planes_form`).
    Plane(u8),
    /// The bytes whose byte before is in `before`'s classes and which are in `table`'s, for a
    /// shared class of `bits` (`block::BlockOp::Pair`): on the CPU the class bytes of the block
    /// loaded one byte back, ANDed with the block's own, then one test.
    Pair { before: u16, table: u16, bits: u8 },
    /// Bit `k` of every byte's byte before (0x00 before the stream): one bit-plane of the
    /// source one byte back, for `pair` in the planes form. Local: the byte before a lane is read
    /// from the source.
    PlaneBefore(u8),
    /// A 64-bit pattern repeated over the stream.
    Const(u64),
    Not(u16),
    And(u16, u16),
    Or(u16, u16),
    Xor(u16, u16),
    AndNot(u16, u16),
    /// The carried operations (`block::carry`): on the CPU the carry passes in a register from
    /// block to block; on Metal by a scan of lane summaries (`metal_block_program`).
    Prev(u16, u8),
    Toggle(u16),
    Escaped(u16),
    /// `fill(s, e)` (`block::BlockOp::Fill`): one add per word, its carry a borrow.
    Fill(u16, u16),
    /// `back_fill(s, e)` (`block::BlockOp::BackFill`): `fill` over the bits reversed, its carry
    /// from the next block. Never in a body: a pass of its own, last block to first (`passes`).
    BackFill(u16, u16),
    /// The word a pass before stored in slot `k` for the block (`passes`): a value of the
    /// program made in an earlier pass, as that pass made it.
    Load(u16),
    /// The bytes inside the stream at a delay (`block::BlockOp::Within`), from its length.
    Within(u8),
    /// The bytes whose position is `r` mod `k` (`block::BlockOp::Phase`), from the position.
    Phase(u8, u8),
    /// Plane `j` of each byte's position (`block::BlockOp::Position`), from the position.
    Position(u8),
    /// A fixed point's carry (`block::BlockOp::Carry`): its initial value's word in a block's
    /// first round, its next's after.
    Carry(u16),
    /// A row program's comparison (`block::BlockOp::Cmp`): 64 rows' lanes compared, one bit
    /// each.
    Cmp { col: u16, op: super::block::CmpOp, rhs: super::block::Rhs, width: u8 },
    /// A row program's set test (`block::BlockOp::Bit`): 64 rows' values looked up in a set.
    Bit { set: u16, col: u16 },
}

impl BlockInst {
    /// Whether the instruction carries across blocks.
    pub(super) fn carried(&self) -> bool {
        matches!(self, BlockInst::Prev(..) | BlockInst::Toggle(_) | BlockInst::Escaped(_) | BlockInst::Fill(..) | BlockInst::BackFill(..))
    }

    /// The values the instruction reads.
    pub(super) fn operands(&self) -> Vec<usize> {
        match *self {
            BlockInst::Not(a) | BlockInst::Prev(a, _) | BlockInst::Toggle(a) | BlockInst::Escaped(a) | BlockInst::Carry(a) => vec![usize::from(a)],
            BlockInst::And(a, b) | BlockInst::Or(a, b) | BlockInst::Xor(a, b) | BlockInst::AndNot(a, b) | BlockInst::Fill(a, b) | BlockInst::BackFill(a, b) => {
                vec![usize::from(a), usize::from(b)]
            }
            _ => vec![],
        }
    }
}

impl BlockNative {
    pub(super) fn lower(program: &super::block::BlockProgram) -> Self {
        use super::block::BlockOp;
        let insts = program
            .ops
            .iter()
            .map(|op| match *op {
                BlockOp::Eq(byte) => BlockInst::EqByte(byte),
                BlockOp::Class { table, bits } => BlockInst::Class { table, bits },
                BlockOp::Pair { before, table, bits } => BlockInst::Pair { before, table, bits },
                BlockOp::Const(c) => BlockInst::Const(c),
                BlockOp::Not(a) => BlockInst::Not(a),
                BlockOp::And(a, b) => BlockInst::And(a, b),
                BlockOp::Or(a, b) => BlockInst::Or(a, b),
                BlockOp::Xor(a, b) => BlockInst::Xor(a, b),
                BlockOp::AndNot(a, b) => BlockInst::AndNot(a, b),
                BlockOp::Prev(a, k) => BlockInst::Prev(a, k),
                BlockOp::Toggle(a) => BlockInst::Toggle(a),
                BlockOp::Escaped(a) => BlockInst::Escaped(a),
                BlockOp::Fill(a, b) => BlockInst::Fill(a, b),
                BlockOp::BackFill(a, b) => BlockInst::BackFill(a, b),
                BlockOp::Within(d) => BlockInst::Within(d),
                BlockOp::Phase(k, r) => BlockInst::Phase(k, r),
                BlockOp::Position(j) => BlockInst::Position(j),
                BlockOp::Carry(a) => BlockInst::Carry(a),
                BlockOp::Cmp { col, op, rhs, width } => BlockInst::Cmp { col, op, rhs, width },
                BlockOp::Bit { set, col } => BlockInst::Bit { set, col },
            })
            .collect();
        Self {
            tables: program.tables.clone(),
            insts,
            outputs: program.outputs.clone(),
            extracts: program.extract_outputs(),
            totals: program.total_outputs(),
            masks: program.mask_outputs(),
            delay: program.delay,
            output_delays: (0..program.outputs.len()).map(|k| program.output_delay(k)).collect(),
            groups: super::block::groups(&program.ops, &program.rounds).expect("a verified program"),
        }
    }

    /// The same program with every `class` written as a boolean formula of the eight bit-planes
    /// (`Plane`): an OR over the selected class bits of a function of the low four planes and one
    /// of the high four, each by Shannon expansion on its highest plane, constants folded and
    /// common terms shared. Metal runs this form (it measured faster than table lookups); the
    /// CPU keeps `class`, which is two `tbl`s per 16 bytes there.
    pub(super) fn planes_form(&self) -> BlockNative {
        let mut f = PlaneFormulas::default();
        let mut at: Vec<u16> = Vec::with_capacity(self.insts.len());
        for inst in &self.insts {
            let m = |x: u16| at[usize::from(x)];
            let index = match *inst {
                BlockInst::Class { table, bits } => {
                    let t = self.tables[usize::from(table)];
                    let mut word = Term::Zero;
                    for c in (0..8).filter(|c| bits >> c & 1 == 1) {
                        let lo = (0..16).fold(0u16, |a, n| a | (u16::from(t.lo[n] >> c & 1) << n));
                        let hi = (0..16).fold(0u16, |a, n| a | (u16::from(t.hi[n] >> c & 1) << n));
                        let l = f.function(0, 4, lo);
                        let h = f.function(4, 4, hi);
                        let both = f.and(l, h);
                        word = f.or(word, both);
                    }
                    f.index(word)
                }
                // The same with the byte before's planes (8..16) for `before`'s functions.
                BlockInst::Pair { before, table, bits } => {
                    let (p, t) = (self.tables[usize::from(before)], self.tables[usize::from(table)]);
                    let half = |n: &[u8; 16], c: usize| (0..16).fold(0u16, |a, i| a | (u16::from(n[i] >> c & 1) << i));
                    let mut word = Term::Zero;
                    for c in (0..8).filter(|c| bits >> c & 1 == 1) {
                        let pl = f.function(8, 4, half(&p.lo, c));
                        let ph = f.function(12, 4, half(&p.hi, c));
                        let l = f.function(0, 4, half(&t.lo, c));
                        let h = f.function(4, 4, half(&t.hi, c));
                        let a = f.and(pl, ph);
                        let b = f.and(l, h);
                        let both = f.and(a, b);
                        word = f.or(word, both);
                    }
                    f.index(word)
                }
                BlockInst::Not(a) => f.push(BlockInst::Not(m(a))),
                BlockInst::And(a, b) => f.push(BlockInst::And(m(a), m(b))),
                BlockInst::Or(a, b) => f.push(BlockInst::Or(m(a), m(b))),
                BlockInst::Xor(a, b) => f.push(BlockInst::Xor(m(a), m(b))),
                BlockInst::AndNot(a, b) => f.push(BlockInst::AndNot(m(a), m(b))),
                BlockInst::Prev(a, k) => f.push(BlockInst::Prev(m(a), k)),
                BlockInst::Toggle(a) => f.push(BlockInst::Toggle(m(a))),
                BlockInst::Escaped(a) => f.push(BlockInst::Escaped(m(a))),
                BlockInst::Fill(a, b) => f.push(BlockInst::Fill(m(a), m(b))),
                BlockInst::BackFill(a, b) => f.push(BlockInst::BackFill(m(a), m(b))),
                BlockInst::Carry(a) => f.push(BlockInst::Carry(m(a))),
                other => f.push(other),
            };
            at.push(index);
        }
        let place = |x: usize| usize::from(at[x]);
        BlockNative {
            tables: self.tables.clone(),
            insts: f.insts,
            outputs: self.outputs.iter().map(|&v| at[usize::from(v)]).collect(),
            extracts: self.extracts.clone(),
            totals: self.totals.clone(),
            masks: self.masks.clone(),
            delay: self.delay,
            output_delays: self.output_delays.clone(),
            groups: self
                .groups
                .iter()
                .map(|g| super::block::Group { members: g.members.iter().map(|&x| place(x)).collect(), carries: g.carries.iter().map(|&(c, x)| (place(c), place(x))).collect() })
                .collect(),
        }
    }
}

/// A pass over a file's blocks, of a program in passes (`BlockNative::passes`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Pass {
    /// First block to first: a body, as a program without passes runs it. Its `Load`s read the
    /// slots of passes before it. Before the last pass, output `k` is stored to slot
    /// `stores[k]`, each block's word as the body made it (no cut, no delay); the last pass's
    /// outputs are the program's.
    Forward { body: BlockNative, stores: Vec<u16> },
    /// Last block to first: each `back_fill` (`block::carry::back_fill`), its start's slot, its
    /// end's, and its own, the carry zero at the file's last block.
    Back(Vec<(u16, u16, u16)>),
}

/// A program as passes over each file's blocks, and the slots they store per block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Passes {
    pub passes: Vec<Pass>,
    pub slots: u16,
}

impl BlockNative {
    /// The program as passes. A `back_fill` reads its operands over the whole file before it
    /// gives its first word, so each value's level is the most `back_fill`s on a way to it (a
    /// fixed point's members all at the highest of theirs): level 0's values in a first forward
    /// pass, each level's `back_fill`s in a backward pass after the level before, its other
    /// values in a forward pass after that. A value read in a later pass, and an output made
    /// before the last, has a slot. A program without `back_fill` is one forward pass.
    pub(super) fn passes(&self) -> Passes {
        let n = self.insts.len();
        let back = |x: usize| matches!(self.insts[x], BlockInst::BackFill(..));
        let mut level = vec![0usize; n];
        loop {
            let mut changed = false;
            for x in 0..n {
                let l = self.insts[x].operands().into_iter().map(|o| level[o]).max().unwrap_or(0) + usize::from(back(x));
                if l > level[x] {
                    level[x] = l;
                    changed = true;
                }
            }
            for g in &self.groups {
                let top = g.members.iter().map(|&m| level[m]).max().unwrap_or(0);
                for &m in &g.members {
                    if level[m] < top {
                        level[m] = top;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        let top = level.iter().copied().max().unwrap_or(0);
        if top == 0 {
            return Passes { passes: vec![Pass::Forward { body: self.clone(), stores: Vec::new() }], slots: 0 };
        }
        // Each value's pass: level L's `back_fill`s in pass 2L - 1, its others in pass 2L.
        let pass = |x: usize| 2 * level[x] - usize::from(back(x));
        let last = 2 * top;
        let mut slot: Vec<Option<u16>> = vec![None; n];
        let mut slots = 0u16;
        let mut give = |x: usize, slot: &mut Vec<Option<u16>>| {
            if slot[x].is_none() {
                slot[x] = Some(slots);
                slots += 1;
            }
        };
        for x in 0..n {
            for o in self.insts[x].operands() {
                if pass(o) != pass(x) {
                    give(o, &mut slot);
                }
            }
        }
        for &v in &self.outputs {
            if pass(usize::from(v)) != last {
                give(usize::from(v), &mut slot);
            }
        }
        let mut passes = Vec::new();
        for p in 0..=last {
            if p % 2 == 1 {
                let fills: Vec<(u16, u16, u16)> = (0..n)
                    .filter(|&x| pass(x) == p)
                    .map(|x| match self.insts[x] {
                        BlockInst::BackFill(a, b) => (slot[usize::from(a)].expect("read later"), slot[usize::from(b)].expect("read later"), slot[x].expect("read later")),
                        _ => unreachable!("a back pass holds `back_fill`s"),
                    })
                    .collect();
                passes.push(Pass::Back(fills));
                continue;
            }
            let mine: Vec<usize> = (0..n).filter(|&x| pass(x) == p).collect();
            // The values of passes before read here, loaded first; then the pass's own, in order.
            let mut loads: Vec<usize> = mine.iter().flat_map(|&x| self.insts[x].operands()).filter(|&o| pass(o) != p).collect();
            if p == last {
                loads.extend(self.outputs.iter().map(|&v| usize::from(v)).filter(|&v| pass(v) != p));
            }
            loads.sort_unstable();
            loads.dedup();
            let mut local: Vec<Option<u16>> = vec![None; n];
            let mut insts = Vec::with_capacity(loads.len() + mine.len());
            for &x in &loads {
                local[x] = Some(insts.len() as u16);
                insts.push(BlockInst::Load(slot[x].expect("read later")));
            }
            for &x in &mine {
                local[x] = Some(insts.len() as u16);
                let m = |o: u16| local[usize::from(o)].expect("made before");
                insts.push(match self.insts[x] {
                    BlockInst::Not(a) => BlockInst::Not(m(a)),
                    BlockInst::And(a, b) => BlockInst::And(m(a), m(b)),
                    BlockInst::Or(a, b) => BlockInst::Or(m(a), m(b)),
                    BlockInst::Xor(a, b) => BlockInst::Xor(m(a), m(b)),
                    BlockInst::AndNot(a, b) => BlockInst::AndNot(m(a), m(b)),
                    BlockInst::Prev(a, k) => BlockInst::Prev(m(a), k),
                    BlockInst::Toggle(a) => BlockInst::Toggle(m(a)),
                    BlockInst::Escaped(a) => BlockInst::Escaped(m(a)),
                    BlockInst::Fill(a, b) => BlockInst::Fill(m(a), m(b)),
                    BlockInst::Carry(a) => BlockInst::Carry(m(a)),
                    BlockInst::BackFill(..) | BlockInst::Load(_) => unreachable!("not in a forward pass"),
                    other => other,
                });
            }
            let place = |x: usize| usize::from(local[x].expect("the pass's"));
            let groups: Vec<super::block::Group> = self
                .groups
                .iter()
                .filter(|g| g.members.iter().all(|&m| pass(m) == p))
                .map(|g| super::block::Group { members: g.members.iter().map(|&x| place(x)).collect(), carries: g.carries.iter().map(|&(c, x)| (place(c), place(x))).collect() })
                .collect();
            let body = |outputs: Vec<u16>, last: bool| BlockNative {
                tables: self.tables.clone(),
                insts: insts.clone(),
                output_delays: if last { self.output_delays.clone() } else { vec![0; outputs.len()] },
                outputs,
                extracts: if last { self.extracts.clone() } else { Vec::new() },
                totals: if last { self.totals.clone() } else { Vec::new() },
                masks: if last { self.masks.clone() } else { Vec::new() },
                delay: self.delay,
                groups: groups.clone(),
            };
            if p == last {
                let outputs = self.outputs.iter().map(|&v| local[usize::from(v)].expect("loaded or made")).collect();
                passes.push(Pass::Forward { body: body(outputs, true), stores: Vec::new() });
            } else {
                let stored: Vec<usize> = mine.iter().copied().filter(|&x| slot[x].is_some()).collect();
                if stored.is_empty() {
                    continue;
                }
                let outputs = stored.iter().map(|&x| local[x].expect("the pass's")).collect();
                passes.push(Pass::Forward { body: body(outputs, false), stores: stored.iter().map(|&x| slot[x].expect("stored")).collect() });
            }
        }
        Passes { passes, slots }
    }
}

/// A formula's value: a constant, or an instruction's.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Term {
    Zero,
    Ones,
    At(u16),
}

/// The instructions of `planes_form`, with formulas shared.
#[derive(Default)]
struct PlaneFormulas {
    insts: Vec<BlockInst>,
    /// Planes 0..8 of the bytes, 8..16 of the bytes before.
    planes: [Option<u16>; 16],
    functions: std::collections::HashMap<(usize, usize, u16), Term>,
    shared: std::collections::HashMap<BlockInst, u16>,
}

impl PlaneFormulas {
    /// An instruction's index, shared with an earlier identical one.
    fn push(&mut self, inst: BlockInst) -> u16 {
        let pure = !matches!(inst, BlockInst::Class { .. } | BlockInst::Pair { .. });
        if pure {
            if let Some(&i) = self.shared.get(&inst) {
                return i;
            }
        }
        let i = self.insts.len() as u16;
        self.insts.push(inst);
        if pure {
            self.shared.insert(inst, i);
        }
        i
    }

    fn index(&mut self, t: Term) -> u16 {
        match t {
            Term::Zero => self.push(BlockInst::Const(0)),
            Term::Ones => self.push(BlockInst::Const(u64::MAX)),
            Term::At(i) => i,
        }
    }

    fn plane(&mut self, k: usize) -> u16 {
        if let Some(i) = self.planes[k] {
            return i;
        }
        let inst = if k < 8 { BlockInst::Plane(k as u8) } else { BlockInst::PlaneBefore(k as u8 - 8) };
        let i = self.push(inst);
        self.planes[k] = Some(i);
        i
    }

    /// The function of planes `base..base + vars` whose truth table, indexed by those planes'
    /// bits, is the low `2^vars` bits of `tt`.
    fn function(&mut self, base: usize, vars: usize, tt: u16) -> Term {
        let size = 1u32 << vars;
        let full = if size == 16 { u16::MAX } else { (1u16 << size) - 1 };
        let tt = tt & full;
        if tt == 0 {
            return Term::Zero;
        }
        if tt == full {
            return Term::Ones;
        }
        if let Some(&t) = self.functions.get(&(base, vars, tt)) {
            return t;
        }
        let half = size / 2;
        let low = (1u16 << half) - 1;
        let (t0, t1) = (tt & low, (tt >> half) & low);
        let f0 = self.function(base, vars - 1, t0);
        let f1 = self.function(base, vars - 1, t1);
        let x = self.plane(base + vars - 1);
        let term = match (f0, f1) {
            _ if f0 == f1 => f0,
            (Term::Zero, Term::Ones) => Term::At(x),
            (Term::Ones, Term::Zero) => Term::At(self.push(BlockInst::Not(x))),
            (Term::Zero, Term::At(b)) => Term::At(self.push(BlockInst::And(x, b))),
            (Term::At(a), Term::Zero) => Term::At(self.push(BlockInst::AndNot(a, x))),
            (Term::At(a), Term::Ones) => Term::At(self.push(BlockInst::Or(x, a))),
            (Term::Ones, Term::At(b)) => {
                let n = self.push(BlockInst::Not(x));
                Term::At(self.push(BlockInst::Or(b, n)))
            }
            (Term::At(a), Term::At(_)) if t1 == !t0 & low => Term::At(self.push(BlockInst::Xor(x, a))),
            (Term::At(a), Term::At(b)) => {
                // f0 ^ ((f0 ^ f1) & x)
                let d = self.push(BlockInst::Xor(a, b));
                let s = self.push(BlockInst::And(d, x));
                Term::At(self.push(BlockInst::Xor(a, s)))
            }
            _ => unreachable!("constants are handled first"),
        };
        self.functions.insert((base, vars, tt), term);
        term
    }

    fn and(&mut self, a: Term, b: Term) -> Term {
        match (a, b) {
            (Term::Zero, _) | (_, Term::Zero) => Term::Zero,
            (Term::Ones, t) | (t, Term::Ones) => t,
            (Term::At(a), Term::At(b)) => Term::At(self.push(BlockInst::And(a, b))),
        }
    }

    fn or(&mut self, a: Term, b: Term) -> Term {
        match (a, b) {
            (Term::Ones, _) | (_, Term::Ones) => Term::Ones,
            (Term::Zero, t) | (t, Term::Zero) => t,
            (Term::At(a), Term::At(b)) => Term::At(self.push(BlockInst::Or(a, b))),
        }
    }
}

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
