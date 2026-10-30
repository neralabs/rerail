use super::{
    metal_batch_abi::{
        ARENA_KIND_COUNT, FORMAT_VERSION, FinalCaptureRecord, GreedyEntryRecord, LinkRecord, LuaInstructionRecord,
        LuaSetMemberRecord, MetalBatchHeader, MetalEvaluation, MetalFile as BatchFile,
        MetalGlobalArenas, MetalNode as BatchNode, MetalProgram as BatchProgram,
        MetalString as BatchString, MetalSupertype as BatchSupertype, MetalTask as BatchTask, NONE,
        NfaRangeRecord, NfaStateRecord, PendingRecord, PropertyRecord, QueryBucketRecord,
        QueryMemberRecord, QueuedStateRecord, RepeatRecord, ResultRecord, TaskStatus, ValueRecord,
    },
    native_ir::ArenaKind,
    packed_workspace::{TaskCapacities, WorkspaceTotals, plan_shared},
    value_packing::PackedValueTables,
    *,
};
use crate::code_rule_dsl::tape::TapeDoc;

const MAX_OUTPUT_CAPTURES_PER_TASK: u32 = 4_096;
const MAX_PROPERTIES_PER_TASK: u32 = 4_096;

/// Packed query ordering and exact-admission metadata, derived from the Program.
pub(super) fn program_flags(program: &Program) -> u32 {
    u32::from(program.tree_sitter_query)
        | (u32::from(
            program.tree_sitter_query
                && program
                    .words
                    .iter()
                    .any(|word| split(*word).0 == REPEAT_BEGIN),
        ) << 1)
}

fn bounded_capacity(needed: u32, maximum: u32) -> u32 {
    needed.min(maximum)
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct AssertionRecord {
    pub(super) kind: u32,
    pub(super) first: u32,
    pub(super) second: u32,
    pub(super) data_start: u32,
    pub(super) data_count: u32,
    pub(super) flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct OutputInstructionRecord {
    kind: u32,
    first: u32,
    second: u32,
    third: u32,
    fourth: u32,
    fifth: u32,
}

#[derive(Clone, Default)]
pub(super) struct PackedBatch {
    pub(super) header: MetalBatchHeader,
    pub(super) nodes: Vec<BatchNode>,
    pub(super) children: Vec<u32>,
    pub(super) source: Vec<u8>,
    pub(super) line_starts: Vec<u32>,
    pub(super) supertypes: Vec<BatchSupertype>,
    /// The files' derived classes (`TapeDoc::with_labels`) over the batch's nodes: per class from
    /// `DERIVED_CLASS_BASE`, `derived_words` words of bits, class after class.
    pub(super) derived: Vec<u32>,
    pub(super) derived_classes: u32,
    pub(super) derived_words: u32,
    pub(super) files: Vec<BatchFile>,
    pub(super) words: Vec<u32>,
    pub(super) longs: Vec<u64>,
    pub(super) string_bytes: Vec<u8>,
    pub(super) strings: Vec<BatchString>,
    pub(super) values: Vec<ValueRecord>,
    pub(super) nfa_states: Vec<NfaStateRecord>,
    pub(super) nfa_data: Vec<u32>,
    pub(super) nfa_ranges: Vec<NfaRangeRecord>,
    pub(super) lua_instructions: Vec<LuaInstructionRecord>,
    pub(super) lua_members: Vec<LuaSetMemberRecord>,
    pub(super) assertions: Vec<AssertionRecord>,
    pub(super) assertion_data: Vec<u32>,
    pub(super) labels: Vec<u16>,
    pub(super) programs: Vec<BatchProgram>,
    pub(super) tasks: Vec<BatchTask>,
    pub(super) evaluations: Vec<MetalEvaluation>,
    pub(super) evaluation_lanes: Vec<MetalEvaluation>,
    pub(super) global_arenas: MetalGlobalArenas,
}

fn checked_u32(value: usize) -> Result<u32, Error> {
    u32::try_from(value).map_err(|_| Error::BackendFailure)
}

fn global_node(base: u32, node: Option<u32>) -> Result<u32, Error> {
    node.map_or(Ok(NONE), |node| {
        base.checked_add(node).ok_or(Error::BackendFailure)
    })
}

const DURABLE_RESULT_PAGE_CAPACITY: u32 = 4_194_304;
const DURABLE_CAPTURE_PAGE_CAPACITY: u32 = 8_388_608;
const DURABLE_PROPERTY_PAGE_CAPACITY: u32 = 2_097_152;
const DURABLE_OUTPUT_BYTE_PAGE_CAPACITY: u32 = 64 * 1024 * 1024;

impl PackedBatch {
    /// Metal's supertypes buffer and files: with derived classes, the buffer starts with them as
    /// the interpreter's `has_supertype` reads them (one record of the class and word counts, then
    /// the bits), and each file's supertypes start past them.
    pub(super) fn metal_supertypes(&self) -> (std::borrow::Cow<'_, [BatchSupertype]>, std::borrow::Cow<'_, [BatchFile]>) {
        if self.derived_classes == 0 {
            return (std::borrow::Cow::Borrowed(&self.supertypes), std::borrow::Cow::Borrowed(&self.files));
        }
        let prefix = 1 + self.derived.len().div_ceil(2);
        let mut supertypes = Vec::with_capacity(prefix + self.supertypes.len());
        supertypes.push(BatchSupertype {
            node: self.derived_classes,
            kind: self.derived_words,
        });
        supertypes.extend(self.derived.chunks(2).map(|pair| BatchSupertype {
            node: pair[0],
            kind: pair.get(1).copied().unwrap_or(0),
        }));
        supertypes.extend_from_slice(&self.supertypes);
        let mut files = self.files.clone();
        for file in &mut files {
            file.supertype_start += prefix as u32;
        }
        (std::borrow::Cow::Owned(supertypes), std::borrow::Cow::Owned(files))
    }

    pub(super) fn arena_start(&self, kind: ArenaKind) -> u32 {
        self.global_arenas.arenas[kind as usize].start
    }

    pub(super) fn arena_used(&self, kind: ArenaKind) -> u32 {
        self.global_arenas.arenas[kind as usize].cursor
    }

    pub(super) fn evaluation_limit(&self, task: &BatchTask, kind: ArenaKind) -> u32 {
        self.evaluations[task.evaluation as usize].quotas[kind as usize].limit
    }

    fn push_regex(&mut self, nfa: &value::ByteNfa) -> Result<u32, Error> {
        let mut tables = PackedValueTables {
            values: std::mem::take(&mut self.values),
            nfa_states: std::mem::take(&mut self.nfa_states),
            nfa_data: std::mem::take(&mut self.nfa_data),
            nfa_ranges: std::mem::take(&mut self.nfa_ranges),
            lua_instructions: std::mem::take(&mut self.lua_instructions),
            lua_members: std::mem::take(&mut self.lua_members),
        };
        let index = checked_u32(tables.values.len())?;
        tables.push_regex(nfa)?;
        self.values = tables.values;
        self.nfa_states = tables.nfa_states;
        self.nfa_data = tables.nfa_data;
        self.nfa_ranges = tables.nfa_ranges;
        self.lua_instructions = tables.lua_instructions;
        self.lua_members = tables.lua_members;
        Ok(index)
    }

    pub(super) fn new(
        programs: &[Program],
        files: &[TapeDoc],
        anchors: &[Vec<Vec<u32>>],
    ) -> Result<Self, Error> {
        Self::build(programs, files, anchors, true)
    }

    pub(super) fn new_grid_inputs(
        programs: &[Program],
        files: &[TapeDoc],
        anchors: &[Vec<Vec<u32>>],
    ) -> Result<Self, Error> {
        Self::build(programs, files, anchors, false)
    }

    pub(super) fn new_shared_grid_inputs(
        programs: &[Program],
        files: &[TapeDoc],
        anchors: &[Vec<Arc<[u32]>>],
    ) -> Result<Self, Error> {
        Self::build(programs, files, anchors, false)
    }

    fn build<R: AsRef<[u32]>>(
        programs: &[Program],
        files: &[TapeDoc],
        anchors: &[Vec<R>],
        materialize_tasks: bool,
    ) -> Result<Self, Error> {
        Self::build_from_tapes(
            programs,
            files.len(),
            anchors,
            materialize_tasks,
            |file| Ok(std::borrow::Cow::Borrowed(&files[file])),
            |_| Ok(()),
        )
    }

    #[cfg(test)]
    pub(super) fn new_streamed_grid_inputs(
        programs: &[Program],
        files: &[crate::code_rule_dsl::tape::TapeSource],
        anchors: &[Vec<Arc<[u32]>>],
    ) -> Result<Self, Error> {
        Self::new_streamed_grid_inputs_with_upload(programs, files, anchors, |_| Ok(()))
    }

    pub(super) fn new_streamed_grid_inputs_with_upload(
        programs: &[Program],
        files: &[crate::code_rule_dsl::tape::TapeSource],
        anchors: &[Vec<Arc<[u32]>>],
        after_file: impl FnMut(&mut Self) -> Result<(), Error>,
    ) -> Result<Self, Error> {
        Self::build_from_tapes(
            programs,
            files.len(),
            anchors,
            false,
            |file| {
                files[file]
                    .restore(&|| false)
                    .map(std::borrow::Cow::Owned)
                    .map_err(|_| Error::BackendFailure)
            },
            after_file,
        )
    }

    fn build_from_tapes<'a, R: AsRef<[u32]>>(
        programs: &[Program],
        file_count: usize,
        anchors: &[Vec<R>],
        materialize_tasks: bool,
        mut load: impl FnMut(usize) -> Result<std::borrow::Cow<'a, TapeDoc>, Error>,
        mut after_file: impl FnMut(&mut Self) -> Result<(), Error>,
    ) -> Result<Self, Error> {
        if anchors.len() != file_count || anchors.iter().any(|file| file.len() != programs.len()) {
            return Err(Error::InvalidAnchor);
        }
        let mut batch = Self {
            nfa_data: super::value_packing::unicode_word_data(),
            ..Self::default()
        };
        let mut navigation = [0usize; 4];
        // Per derived class, the batch's nodes that have it.
        let mut derived: Vec<Vec<u32>> = Vec::new();
        for file in 0..file_count {
            let tape = load(file)?;
            let node_start = checked_u32(navigation[0])?;
            let source_start = checked_u32(batch.source.len())?;
            let source = tape.transducer_source();
            batch.source.extend_from_slice(source);
            let line_start = checked_u32(navigation[3])?;
            let first_line = batch.line_starts.len();
            batch.line_starts.push(source_start);
            batch.line_starts.extend(
                source
                    .iter()
                    .enumerate()
                    .filter(|(_, byte)| **byte == b'\n')
                    .map(|(index, _)| source_start + index as u32 + 1),
            );
            let supertype_start = checked_u32(navigation[2])?;
            let first_supertype = batch.supertypes.len();
            batch
                .supertypes
                .extend(
                    tape.transducer_supertypes()
                        .map(|(node, kind)| BatchSupertype {
                            node: node_start + node,
                            kind: u32::from(kind),
                        }),
                );
            for (class, bits) in tape.transducer_labels().iter().enumerate() {
                if derived.len() <= class {
                    derived.resize_with(class + 1, Vec::new);
                }
                for (index, &word) in bits.iter().enumerate() {
                    let mut word = word;
                    while word != 0 {
                        derived[class].push(node_start + (index * 64) as u32 + word.trailing_zeros());
                        word &= word - 1;
                    }
                }
            }
            let mut local_children = Vec::new();
            for node in 0..tape.transducer_len() as u32 {
                let child_start = checked_u32(navigation[1] + local_children.len())?;
                let first_child = local_children.len();
                for child in tape.transducer_children(node) {
                    local_children
                        .push(node_start.checked_add(child).ok_or(Error::BackendFailure)?);
                }
                let span = tape.transducer_span(node);
                batch.nodes.push(BatchNode {
                    parent: global_node(node_start, tape.transducer_parent(node))?,
                    next_any: global_node(node_start, tape.transducer_next_sibling(node, false))?,
                    next_named: global_node(node_start, tape.transducer_next_sibling(node, true))?,
                    child_start,
                    child_count: checked_u32(local_children.len() - first_child)?,
                    descendant_end: node_start
                        .checked_add(tape.transducer_descendants(node).end)
                        .ok_or(Error::BackendFailure)?,
                    source_start: source_start
                        .checked_add(checked_u32(span.start)?)
                        .ok_or(Error::BackendFailure)?,
                    source_end: source_start
                        .checked_add(checked_u32(span.end)?)
                        .ok_or(Error::BackendFailure)?,
                    kind: u32::from(tape.transducer_kind(node)),
                    field: u32::from(tape.transducer_field(node)),
                    flags: u32::from(tape.transducer_flags(node)),
                });
            }
            navigation[0] += tape.transducer_len();
            navigation[1] += local_children.len();
            navigation[2] += batch.supertypes.len() - first_supertype;
            navigation[3] += batch.line_starts.len() - first_line;
            batch.children.extend(local_children);
            batch.files.push(BatchFile {
                node_start,
                node_count: checked_u32(tape.transducer_len())?,
                source_start,
                source_length: checked_u32(source.len())?,
                line_start,
                line_count: checked_u32(batch.line_starts.len() - first_line)?,
                supertype_start,
                supertype_count: checked_u32(batch.supertypes.len() - first_supertype)?,
            });
            after_file(&mut batch)?;
        }
        if !derived.is_empty() {
            let words = navigation[0].div_ceil(32).max(1);
            batch.derived = vec![0; derived.len() * words];
            for (class, nodes) in derived.iter().enumerate() {
                for &node in nodes {
                    batch.derived[class * words + node as usize / 32] |= 1 << (node % 32);
                }
            }
            batch.derived_classes = checked_u32(derived.len())?;
            batch.derived_words = checked_u32(words)?;
        }
        for program in programs {
            let word_start = checked_u32(batch.words.len())?;
            let long_start = checked_u32(batch.longs.len())?;
            let string_start = checked_u32(batch.strings.len())?;
            for string in &program.strings {
                let start = checked_u32(batch.string_bytes.len())?;
                batch.string_bytes.extend_from_slice(string);
                batch.strings.push(BatchString {
                    byte_start: start,
                    byte_length: checked_u32(string.len())?,
                });
            }
            let mut value_tables = PackedValueTables {
                values: std::mem::take(&mut batch.values),
                nfa_states: std::mem::take(&mut batch.nfa_states),
                nfa_data: std::mem::take(&mut batch.nfa_data),
                nfa_ranges: std::mem::take(&mut batch.nfa_ranges),
                lua_instructions: std::mem::take(&mut batch.lua_instructions),
                lua_members: std::mem::take(&mut batch.lua_members),
            };
            let value_start = value_tables.push_program(&program.value_programs)?;
            batch.values = value_tables.values;
            batch.nfa_states = value_tables.nfa_states;
            batch.nfa_data = value_tables.nfa_data;
            batch.nfa_ranges = value_tables.nfa_ranges;
            batch.lua_instructions = value_tables.lua_instructions;
            batch.lua_members = value_tables.lua_members;
            let capture_start = checked_u32(batch.assertions.len())?;
            let label_start = checked_u32(batch.labels.len())?;
            batch.words.extend_from_slice(&program.words);
            batch.longs.extend_from_slice(&program.longs);
            for capture in &program.capture_programs {
                let mut record = AssertionRecord::default();
                match capture {
                    value::CaptureProgram::Equal {
                        left,
                        right,
                        negated,
                        any,
                    } => {
                        record.kind = 0;
                        record.first = u32::from(*left);
                        record.second = u32::from(*right);
                        record.flags = u32::from(*negated) | (u32::from(*any) << 1);
                    }
                    value::CaptureProgram::Text {
                        capture,
                        value,
                        negated,
                        any,
                    } => {
                        record.kind = 1;
                        record.first = u32::from(*capture);
                        record.second = u32::from(*value);
                        record.flags = u32::from(*negated) | (u32::from(*any) << 1);
                    }
                    value::CaptureProgram::AnyOf {
                        capture,
                        strings,
                        negated,
                        any,
                    } => {
                        record.kind = 2;
                        record.first = u32::from(*capture);
                        record.data_start = checked_u32(batch.assertion_data.len())?;
                        record.data_count = checked_u32(strings.len())?;
                        batch
                            .assertion_data
                            .extend(strings.iter().map(|value| u32::from(*value)));
                        record.flags = u32::from(*negated) | (u32::from(*any) << 1);
                    }
                    value::CaptureProgram::Contains {
                        capture,
                        strings,
                        negated,
                        any,
                    } => {
                        record.kind = 4;
                        record.first = u32::from(*capture);
                        record.data_start = checked_u32(batch.assertion_data.len())?;
                        record.data_count = checked_u32(strings.len())?;
                        batch
                            .assertion_data
                            .extend(strings.iter().map(|value| u32::from(*value)));
                        record.flags = u32::from(*negated) | (u32::from(*any) << 1);
                    }
                    value::CaptureProgram::Kind {
                        capture,
                        kinds,
                        negated,
                        any,
                        empty_all,
                    } => {
                        record.kind = 3;
                        record.first = u32::from(*capture);
                        record.data_start = checked_u32(batch.assertion_data.len())?;
                        record.data_count = checked_u32(kinds.len())?;
                        batch
                            .assertion_data
                            .extend(kinds.iter().map(|kind| u32::from(*kind)));
                        record.flags = u32::from(*negated)
                            | (u32::from(*any) << 1)
                            | (u32::from(*empty_all) << 3);
                    }
                    value::CaptureProgram::Relation {
                        capture,
                        kinds,
                        ancestor,
                        negated,
                    } => {
                        record.kind = 5;
                        record.first = u32::from(*capture);
                        record.data_start = checked_u32(batch.assertion_data.len())?;
                        record.data_count = checked_u32(kinds.len())?;
                        batch
                            .assertion_data
                            .extend(kinds.iter().map(|kind| u32::from(*kind)));
                        record.flags = u32::from(*negated) | (u32::from(*ancestor) << 2);
                    }
                    value::CaptureProgram::SameLine {
                        left,
                        right,
                        negated,
                    } => {
                        record.kind = 6;
                        record.first = u32::from(*left);
                        record.second = u32::from(*right);
                        record.flags = u32::from(*negated);
                    }
                }
                batch.assertions.push(record);
            }
            let output_start = checked_u32(batch.assertion_data.len())?;
            for instruction in &program.output_program.instructions {
                let mut record = OutputInstructionRecord::default();
                match instruction {
                    output::OutputInstruction::MatchProperty { key, value } => {
                        record.kind = 0;
                        record.first = u32::from(*key);
                        record.second = value.map_or(NONE, u32::from);
                    }
                    output::OutputInstruction::CaptureProperty {
                        capture,
                        key,
                        value,
                    } => {
                        record.kind = 1;
                        record.first = u32::from(*capture);
                        record.second = u32::from(*key);
                        record.third = value.map_or(NONE, u32::from);
                    }
                    output::OutputInstruction::Offset { capture, offsets } => {
                        record.kind = 2;
                        record.first = u32::from(*capture);
                        record.second = offsets[0] as u32;
                        record.third = offsets[1] as u32;
                        record.fourth = offsets[2] as u32;
                        record.fifth = offsets[3] as u32;
                    }
                    output::OutputInstruction::Strip { capture, regex } => {
                        record.kind = 3;
                        record.first = u32::from(*capture);
                        record.second = batch.push_regex(regex)?;
                    }
                    output::OutputInstruction::SelectAdjacent { capture, target } => {
                        record.kind = 4;
                        record.first = u32::from(*capture);
                        record.second = u32::from(*target);
                    }
                    output::OutputInstruction::Downcase { capture } => {
                        record.kind = 5;
                        record.first = u32::from(*capture);
                    }
                    output::OutputInstruction::LanguageFromMime { capture } => {
                        record.kind = 6;
                        record.first = u32::from(*capture);
                    }
                }
                batch.assertion_data.extend([
                    record.kind,
                    record.first,
                    record.second,
                    record.third,
                    record.fourth,
                    record.fifth,
                ]);
            }
            batch.labels.extend_from_slice(&program.labels);
            let maximum_cursor_slot = program
                .words
                .iter()
                .filter_map(|word| {
                    let (opcode, _, operand) = split(*word);
                    matches!(opcode, SAVE_CURSOR | RESTORE_CURSOR).then_some(u32::from(operand) + 1)
                })
                .max()
                .unwrap_or(0);
            let maximum_lua_capture_slot = program
                .value_programs
                .iter()
                .map(value::ValueProgram::maximum_lua_capture_slot)
                .max()
                .unwrap_or(0);
            batch.programs.push(BatchProgram {
                structural_start: word_start,
                structural_count: checked_u32(program.words.len())?,
                long_start,
                long_count: checked_u32(program.longs.len())?,
                string_start,
                string_count: checked_u32(program.strings.len())?,
                value_start,
                value_count: checked_u32(batch.values.len())?
                    .checked_sub(value_start)
                    .ok_or(Error::BackendFailure)?,
                capture_start,
                capture_count: checked_u32(program.capture_programs.len())?,
                output_start,
                output_count: checked_u32(program.output_program.instructions.len())?,
                label_start,
                label_count: checked_u32(program.labels.len())?,
                maximum_cursor_slot,
                maximum_lua_capture_slot,
                flags: program_flags(program),
            });
        }
        for (file, file_anchors) in anchors.iter().enumerate() {
            for (program, roots) in file_anchors.iter().enumerate() {
                let roots = roots.as_ref();
                if roots.windows(2).any(|pair| pair[0] >= pair[1]) {
                    return Err(Error::InvalidAnchor);
                }
                for &root in roots {
                    if root >= batch.files[file].node_count {
                        return Err(Error::InvalidAnchor);
                    }
                    if !materialize_tasks {
                        continue;
                    }
                    let ordinal = checked_u32(batch.tasks.len())?;
                    let root = batch.files[file]
                        .node_start
                        .checked_add(root)
                        .ok_or(Error::BackendFailure)?;
                    let task_nodes = batch.nodes[root as usize]
                        .descendant_end
                        .saturating_sub(root)
                        .max(1);
                    let has_repeats = programs[program]
                        .words
                        .iter()
                        .any(|word| split(*word).0 == REPEAT_BEGIN);
                    let scoped_nodes = if has_repeats {
                        batch.files[file].node_count
                    } else {
                        task_nodes
                    };
                    let workspace_work_estimate = u64::from(scoped_nodes)
                        .saturating_mul(programs[program].words.len() as u64)
                        .saturating_mul(4_096)
                        .clamp(4_096, MAX_WORK_PER_TASK as u64)
                        as u32;
                    batch.tasks.push(BatchTask {
                        batch_ordinal: ordinal,
                        file: checked_u32(file)?,
                        program: checked_u32(program)?,
                        root,
                        workspace_work_estimate,
                        ..BatchTask::default()
                    });
                }
            }
        }
        batch.header = MetalBatchHeader {
            format_version: FORMAT_VERSION,
            task_count: checked_u32(batch.tasks.len())?,
            file_count: checked_u32(batch.files.len())?,
            program_count: checked_u32(batch.programs.len())?,
            node_count: checked_u32(navigation[0])?,
            child_count: checked_u32(navigation[1])?,
            supertype_count: checked_u32(navigation[2])?,
            line_start_count: checked_u32(navigation[3])?,
            string_count: checked_u32(batch.strings.len())?,
            structural_word_count: checked_u32(batch.words.len())?,
            value_instruction_count: programs.iter().try_fold(0_u32, |count, program| {
                count
                    .checked_add(checked_u32(program.value_programs.len())?)
                    .ok_or(Error::BackendFailure)
            })?,
            capture_instruction_count: programs.iter().try_fold(0_u32, |count, program| {
                count
                    .checked_add(checked_u32(program.capture_programs.len())?)
                    .ok_or(Error::BackendFailure)
            })?,
            output_instruction_count: programs.iter().try_fold(0_u32, |count, program| {
                count
                    .checked_add(checked_u32(program.output_program.instructions.len())?)
                    .ok_or(Error::BackendFailure)
            })?,
            workspace_byte_count: 0,
            evaluation_count: 0,
            query_bucket_byte_start: 0,
            ..MetalBatchHeader::default()
        };
        Ok(batch)
    }
}

#[derive(Clone, Copy)]
struct ProgramCapacityFacts {
    pending_operations: u32,
    capture_operations: u32,
    cursor_operations: u32,
    repeat_operations: u32,
    greedy_operations: u32,
    emit_operations: u32,
    directive_count: u32,
}

impl ProgramCapacityFacts {
    fn new(program: &Program) -> Result<Self, Error> {
        let operations = program.words.iter().map(|word| split(*word));
        let count = |predicate: &dyn Fn(u8, u8) -> bool| {
            checked_u32(
                operations
                    .clone()
                    .filter(|(opcode, mode, _)| predicate(*opcode, *mode))
                    .count(),
            )
        };
        Ok(Self {
            pending_operations: count(&|opcode, _| {
                matches!(opcode, CHILD | DESCEND | FOLLOWING | FORK)
            })?,
            capture_operations: count(&|opcode, _| opcode == CAPTURE)?,
            cursor_operations: count(&|opcode, _| opcode == SAVE_CURSOR)?,
            repeat_operations: count(&|opcode, _| {
                matches!(opcode, REPEAT_BEGIN | REPEAT_BUMP | REPEAT_END)
            })?,
            greedy_operations: count(&|opcode, mode| opcode == FORK && mode == 1)?,
            emit_operations: count(&|opcode, _| opcode == EMIT)?,
            directive_count: checked_u32(program.output_program.instructions.len())?,
        })
    }
}

impl PackedBatch {
    pub(super) fn workspace_allocation_bytes(
        &self,
        totals: super::packed_workspace::WorkspaceTotals,
    ) -> Option<u64> {
        let records = [
            (totals.pending, std::mem::size_of::<PendingRecord>()),
            (totals.captures, std::mem::size_of::<LinkRecord>()),
            (totals.cursors, std::mem::size_of::<LinkRecord>()),
            (totals.repeats, std::mem::size_of::<RepeatRecord>()),
            (
                totals.query_states,
                std::mem::size_of::<QueryBucketRecord>(),
            ),
            (
                totals.query_members,
                std::mem::size_of::<QueryMemberRecord>(),
            ),
            (
                totals.queued_states,
                std::mem::size_of::<QueuedStateRecord>(),
            ),
            (
                totals.greedy_entries,
                std::mem::size_of::<GreedyEntryRecord>(),
            ),
            (totals.value_states, std::mem::size_of::<u32>()),
            (totals.results, std::mem::size_of::<ResultRecord>()),
            (
                totals.output_captures,
                std::mem::size_of::<FinalCaptureRecord>(),
            ),
            (totals.properties, std::mem::size_of::<PropertyRecord>()),
            (self.tasks.len() as u32, std::mem::size_of::<TaskStatus>()),
            (self.header.workspace_byte_count, 1),
        ];
        records.iter().try_fold(0_u64, |total, (count, size)| {
            total.checked_add(u64::from(*count).checked_mul(*size as u64)?)
        })
    }

    pub(super) fn workspace_fits(
        programs: &[Program],
        files: &[TapeDoc],
        anchors: &[Vec<Vec<u32>>],
    ) -> Result<(), Error> {
        let mut batch = Self::new(programs, files, anchors)?;
        batch.assign_workspace(programs).map(|_| ())
    }

    pub(super) fn assign_workspace(
        &mut self,
        programs: &[Program],
    ) -> Result<super::packed_workspace::WorkspaceTotals, Error> {
        self.assign_workspace_with_result_limits(programs, None)
    }

    pub(super) fn assign_workspace_with_result_limits(
        &mut self,
        programs: &[Program],
        result_limits: Option<&[u32]>,
    ) -> Result<super::packed_workspace::WorkspaceTotals, Error> {
        self.assign_workspace_with_result_limits_and_lanes(programs, result_limits, 0)
    }

    pub(super) fn assign_workspace_with_result_limits_and_lanes(
        &mut self,
        programs: &[Program],
        result_limits: Option<&[u32]>,
        lanes_per_evaluation: usize,
    ) -> Result<super::packed_workspace::WorkspaceTotals, Error> {
        if result_limits.is_some_and(|limits| limits.len() != programs.len()) {
            return Err(Error::InvalidAnchor);
        }
        let facts = programs
            .iter()
            .map(ProgramCapacityFacts::new)
            .collect::<Result<Vec<_>, _>>()?;
        // What a task's capacities take from its program alone, once per program.
        let per_program = programs
            .iter()
            .zip(&self.programs)
            .map(|(program, packed_program)| {
                let value_end = packed_program
                    .value_start
                    .checked_add(packed_program.value_count)
                    .ok_or(Error::BackendCapacity)? as usize;
                let program_values = self
                    .values
                    .get(packed_program.value_start as usize..value_end)
                    .ok_or(Error::BackendFailure)?;
                let strip_states = program.output_program.instructions.iter().try_fold(
                    0u32,
                    |maximum, instruction| {
                        let needed =
                            if let output::OutputInstruction::Strip { regex, .. } = instruction {
                                checked_u32(regex.states.len())?
                                    .checked_mul(7)
                                    .and_then(|n| n.checked_add(1))
                                    .ok_or(Error::BackendCapacity)?
                            } else {
                                0
                            };
                        Ok::<_, Error>(maximum.max(needed))
                    },
                )?;
                let source_transform = program.output_program.instructions.iter().any(|instruction| {
                    matches!(
                        instruction,
                        output::OutputInstruction::Strip { .. } | output::OutputInstruction::Downcase { .. }
                    )
                });
                let mime_transform = program
                    .output_program
                    .instructions
                    .iter()
                    .any(|instruction| matches!(instruction, output::OutputInstruction::LanguageFromMime { .. }));
                Ok::<_, Error>((program_values, strip_states, source_transform, mime_transform))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let capacities = self
            .tasks
            .iter()
            .map(|task| {
                let facts = facts[task.program as usize];
                let (program_values, strip_states, source_transform, mime_transform) = per_program[task.program as usize];
                let file = &self.files[task.file as usize];
                let root = self.nodes[task.root as usize];
                let work_capacity = |operations: u32| {
                    if operations == 0 {
                        0
                    } else {
                        task.workspace_work_estimate.min(65_536)
                    }
                };
                let pending_states = work_capacity(facts.pending_operations);
                let capture_states = work_capacity(facts.capture_operations);
                let cursor_states = work_capacity(facts.cursor_operations);
                let repeat_states = work_capacity(facts.repeat_operations);
                let repeat_bound = repeat_states;
                let greedy_states = work_capacity(facts.greedy_operations);
                let result_states = work_capacity(facts.emit_operations);
                let node_source_length = root.source_end.saturating_sub(root.source_start);
                let value_states = program_values.iter().try_fold(0_u32, |maximum, value| {
                    let needed = if value.kind == 2 {
                        value
                            .state_count
                            .checked_mul(4)
                            .ok_or(Error::BackendCapacity)?
                    } else if value.kind == 3 {
                        let slots = value.operand >> 2;
                        let stride = 2_u32
                            .checked_add(slots.checked_mul(3).ok_or(Error::BackendCapacity)?)
                            .ok_or(Error::BackendCapacity)?;
                        value
                            .state_count
                            .checked_add(1)
                            .and_then(|items| items.checked_mul(node_source_length + 1))
                            .and_then(|alternatives| alternatives.checked_mul(stride))
                            .and_then(|words| words.checked_add(slots * 3))
                            .ok_or(Error::BackendCapacity)?
                    } else {
                        0
                    };
                    Ok::<u32, Error>(maximum.max(needed))
                })?;
                let value_states = value_states.max(strip_states);
                let output_bytes_per_result = if source_transform {
                    file.source_length
                } else if mime_transform {
                    18
                } else {
                    0
                };
                let output_bytes = result_states
                    .saturating_mul(output_bytes_per_result)
                    .min(64 * 1024 * 1024);
                let properties = result_states
                    .checked_mul(facts.directive_count)
                    .map(|capacity| capacity.min(MAX_PROPERTIES_PER_TASK))
                    .ok_or(Error::BackendCapacity)?;
                Ok(TaskCapacities {
                    pending: pending_states,
                    captures: capture_states,
                    cursors: cursor_states,
                    repeats: repeat_states,
                    query_states: if repeat_bound != 0 {
                        repeat_bound
                            .checked_mul(2)
                            .and_then(u32::checked_next_power_of_two)
                            .ok_or(Error::BackendCapacity)?
                    } else {
                        0
                    },
                    query_members: repeat_bound,
                    queued_states: repeat_bound,
                    greedy_entries: greedy_states,
                    value_states,
                    candidates: result_states,
                    results: result_states,
                    output_bytes,
                    output_captures: if result_states != 0 {
                        capture_states.min(MAX_OUTPUT_CAPTURES_PER_TASK)
                    } else {
                        0
                    },
                    properties,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let shared_caps = (lanes_per_evaluation > 0).then(|| {
            let mut caps = [u32::MAX; ARENA_KIND_COUNT];
            caps[ArenaKind::Result as usize] = DURABLE_RESULT_PAGE_CAPACITY;
            caps[ArenaKind::FinalCapture as usize] = DURABLE_CAPTURE_PAGE_CAPACITY;
            caps[ArenaKind::Property as usize] = DURABLE_PROPERTY_PAGE_CAPACITY;
            caps[ArenaKind::OutputByte as usize] = DURABLE_OUTPUT_BYTE_PAGE_CAPACITY;
            caps
        });
        let shared = plan_shared(&self.tasks, &capacities, shared_caps)?;
        self.tasks = shared.tasks;
        self.evaluations = shared.evaluations;
        self.global_arenas = shared.global;
        if let Some(result_limits) = result_limits {
            let mut result_capacity = 0_u32;
            for evaluation in &mut self.evaluations {
                let program = self.tasks[evaluation.task_start as usize].program as usize;
                let quota = &mut evaluation.quotas[ArenaKind::Result as usize];
                quota.limit = quota.limit.min(result_limits[program]);
                result_capacity = if lanes_per_evaluation > 0 {
                    result_capacity
                        .saturating_add(quota.limit)
                        .min(DURABLE_RESULT_PAGE_CAPACITY)
                } else {
                    result_capacity
                        .checked_add(quota.limit)
                        .ok_or(Error::BackendCapacity)?
                };
            }
            self.global_arenas.arenas[ArenaKind::Result as usize].capacity = result_capacity;
        }
        let transient_kinds = [
            ArenaKind::Pending,
            ArenaKind::Capture,
            ArenaKind::Cursor,
            ArenaKind::Repeat,
            ArenaKind::QueryBucket,
            ArenaKind::QueryMember,
            ArenaKind::QueuedState,
            ArenaKind::GreedyEntry,
            ArenaKind::ValueState,
            ArenaKind::Candidate,
        ];
        self.evaluation_lanes.clear();
        if lanes_per_evaluation > 0 {
            let lane_count = lanes_per_evaluation.min(self.tasks.len()).max(1);
            let mut lane_template = MetalEvaluation::default();
            for kind in transient_kinds {
                let limit = self
                    .evaluations
                    .iter()
                    .map(|evaluation| evaluation.quotas[kind as usize].limit)
                    .max()
                    .unwrap_or(0);
                lane_template.quotas[kind as usize].limit = limit;
                self.global_arenas.arenas[kind as usize].capacity = limit
                    .checked_mul(checked_u32(lane_count)?)
                    .ok_or(Error::BackendCapacity)?;
            }
            lane_template.pending.limit = lane_template.quotas[ArenaKind::Pending as usize].limit;
            lane_template.capture.limit = lane_template.quotas[ArenaKind::Capture as usize].limit;
            lane_template.cursor.limit = lane_template.quotas[ArenaKind::Cursor as usize].limit;
            lane_template.repeat.limit = lane_template.quotas[ArenaKind::Repeat as usize].limit;
            lane_template.query_bucket.limit =
                lane_template.quotas[ArenaKind::QueryBucket as usize].limit;
            lane_template.query_member.limit =
                lane_template.quotas[ArenaKind::QueryMember as usize].limit;
            lane_template.queued_state.limit =
                lane_template.quotas[ArenaKind::QueuedState as usize].limit;
            lane_template.greedy_entry.limit =
                lane_template.quotas[ArenaKind::GreedyEntry as usize].limit;
            lane_template.value_state.limit =
                lane_template.quotas[ArenaKind::ValueState as usize].limit;
            lane_template.candidate.limit =
                lane_template.quotas[ArenaKind::Candidate as usize].limit;
            self.evaluation_lanes.resize(lane_count, lane_template);
            for (kind, page_capacity) in [
                (ArenaKind::Result, DURABLE_RESULT_PAGE_CAPACITY),
                (ArenaKind::FinalCapture, DURABLE_CAPTURE_PAGE_CAPACITY),
                (ArenaKind::Property, DURABLE_PROPERTY_PAGE_CAPACITY),
                (ArenaKind::OutputByte, DURABLE_OUTPUT_BYTE_PAGE_CAPACITY),
            ] {
                self.global_arenas.arenas[kind as usize].capacity = self.global_arenas.arenas
                    [kind as usize]
                    .capacity
                    .min(page_capacity);
            }
        }
        let totals = WorkspaceTotals::from_global(&self.global_arenas);
        let property_bytes = usize::try_from(totals.properties)
            .ok()
            .and_then(|count| count.checked_mul(std::mem::size_of::<PropertyRecord>()))
            .ok_or(Error::BackendCapacity)?;
        let property_bytes_u32 = checked_u32(property_bytes)?;
        self.global_arenas.arenas[ArenaKind::Result as usize].start = totals.candidates;
        self.global_arenas.arenas[ArenaKind::OutputByte as usize].start = property_bytes_u32;
        self.header.workspace_byte_count = property_bytes_u32
            .checked_add(totals.output_bytes)
            .ok_or(Error::BackendCapacity)?;
        self.header.evaluation_count = checked_u32(self.evaluations.len())?;
        let directory_bytes = std::mem::size_of::<MetalGlobalArenas>()
            .checked_add(
                self.evaluations
                    .len()
                    .checked_mul(std::mem::size_of::<MetalEvaluation>())
                    .ok_or(Error::BackendCapacity)?,
            )
            .and_then(|bytes| {
                bytes.checked_add(
                    self.evaluation_lanes
                        .len()
                        .checked_mul(std::mem::size_of::<MetalEvaluation>())?,
                )
            })
            .ok_or(Error::BackendCapacity)?;
        self.header.query_bucket_byte_start = checked_u32(directory_bytes)?;
        let query_bytes = totals
            .query_states
            .checked_mul(std::mem::size_of::<QueryBucketRecord>() as u32)
            .ok_or(Error::BackendCapacity)?;
        let member_bytes = totals
            .query_members
            .checked_mul(std::mem::size_of::<QueryMemberRecord>() as u32)
            .ok_or(Error::BackendCapacity)?;
        let queued_bytes = totals
            .queued_states
            .checked_mul(std::mem::size_of::<QueuedStateRecord>() as u32)
            .ok_or(Error::BackendCapacity)?;
        let greedy_bytes = totals
            .greedy_entries
            .checked_mul(std::mem::size_of::<GreedyEntryRecord>() as u32)
            .ok_or(Error::BackendCapacity)?;
        self.header.query_member_byte_start = self
            .header
            .query_bucket_byte_start
            .checked_add(query_bytes)
            .ok_or(Error::BackendCapacity)?;
        self.header.queued_state_byte_start = self
            .header
            .query_member_byte_start
            .checked_add(member_bytes)
            .ok_or(Error::BackendCapacity)?;
        self.header.greedy_entry_byte_start = self
            .header
            .queued_state_byte_start
            .checked_add(queued_bytes)
            .ok_or(Error::BackendCapacity)?;
        self.header.scheduler_byte_count = self
            .header
            .greedy_entry_byte_start
            .checked_add(greedy_bytes)
            .ok_or(Error::BackendCapacity)?;

        Ok(totals)
    }
}

// N's (5bf45ff0), which bb4c9605's merge dropped where G's version of this file replaced the
// mechanism; kept as copied (open item O21).
#[allow(dead_code)]
pub(super) fn stable_status_error(statuses: &[TaskStatus]) -> Option<Error> {
    statuses.iter().find_map(status_error)
}

fn status_error(status: &TaskStatus) -> Option<Error> {
    (status.error != 0).then_some(match status.error {
        1 | 3 | 5 | 6 => Error::InvalidOpcode(status.program_counter as usize),
        2 => Error::BackendCapacity,
        4 => Error::WorkLimit,
        7 => Error::InvalidOperand(status.program_counter as usize),
        _ => Error::BackendFailure,
    })
}

struct PackedResults<'a> {
    batch: &'a PackedBatch,
    statuses: &'a [TaskStatus],
    results: &'a [ResultRecord],
    final_captures: &'a [FinalCaptureRecord],
    properties: &'a [PropertyRecord],
    output_storage: &'a [u8],
}

fn checked_range(start: u32, count: u32, length: usize) -> Result<std::ops::Range<usize>, Error> {
    let end = start.checked_add(count).ok_or(Error::BackendFailure)?;
    let range = start as usize..end as usize;
    if range.end > length {
        return Err(Error::BackendFailure);
    }
    Ok(range)
}

impl<'a> PackedResults<'a> {
    fn new(
        batch: &'a PackedBatch,
        statuses: &'a [TaskStatus],
        results: &'a [ResultRecord],
        final_captures: &'a [FinalCaptureRecord],
        properties: &'a [PropertyRecord],
        output_storage: &'a [u8],
    ) -> Result<Self, Error> {
        if statuses.len() != batch.tasks.len() {
            return Err(Error::BackendFailure);
        }
        let packed = Self {
            batch,
            statuses,
            results,
            final_captures,
            properties,
            output_storage,
        };
        packed.validate()?;
        Ok(packed)
    }

    fn validate(&self) -> Result<(), Error> {
        let mut task_by_ordinal = vec![None; self.batch.tasks.len()];
        for (index, task) in self.batch.tasks.iter().enumerate() {
            let slot = task_by_ordinal
                .get_mut(task.batch_ordinal as usize)
                .ok_or(Error::BackendFailure)?;
            if slot.replace(index).is_some() {
                return Err(Error::BackendFailure);
            }
        }
        let mut result_counts = vec![0_u32; self.batch.tasks.len()];
        for (task, status) in self.batch.tasks.iter().zip(self.statuses) {
            if status.error != 0
                && (status.result_count != 0
                    || status.final_capture_count != 0
                    || status.property_count != 0
                    || status.output_byte_count != 0)
            {
                return Err(Error::BackendFailure);
            }
            if status.result_count > self.batch.evaluation_limit(task, ArenaKind::Result)
                || status.final_capture_count
                    > self.batch.evaluation_limit(task, ArenaKind::FinalCapture)
                || status.property_count > self.batch.evaluation_limit(task, ArenaKind::Property)
                || status.output_byte_count
                    > self.batch.evaluation_limit(task, ArenaKind::OutputByte)
            {
                return Err(Error::BackendFailure);
            }
        }
        let result_range = checked_range(
            self.batch.arena_start(ArenaKind::Result),
            self.batch.arena_used(ArenaKind::Result),
            self.results.len(),
        )?;
        let capture_range = checked_range(
            self.batch.arena_start(ArenaKind::FinalCapture),
            self.batch.arena_used(ArenaKind::FinalCapture),
            self.final_captures.len(),
        )?;
        let property_range = checked_range(
            self.batch.arena_start(ArenaKind::Property),
            self.batch.arena_used(ArenaKind::Property),
            self.properties.len(),
        )?;
        let byte_range = checked_range(
            self.batch.arena_start(ArenaKind::OutputByte),
            self.batch.arena_used(ArenaKind::OutputByte),
            self.output_storage.len(),
        )?;
        let mut result_tasks = vec![None; result_range.len()];
        let mut result_property_counts = vec![(0_u32, 0_u32); result_range.len()];
        let mut capture_files = vec![None; capture_range.len()];
        let mut capture_property_counts = vec![0_u32; capture_range.len()];
        for (offset, result) in self.results[result_range.clone()].iter().enumerate() {
            let task_index = task_by_ordinal
                .get(result.task_ordinal as usize)
                .and_then(|index| *index)
                .ok_or(Error::BackendFailure)?;
            result_counts[task_index] = result_counts[task_index]
                .checked_add(1)
                .ok_or(Error::BackendFailure)?;
            result_tasks[offset] = Some(task_index);
            let task = &self.batch.tasks[task_index];
            if self.statuses[task_index].error != 0 {
                return Err(Error::BackendFailure);
            }
            let file = self
                .batch
                .files
                .get(task.file as usize)
                .ok_or(Error::BackendFailure)?;
            let file_node_end = file
                .node_start
                .checked_add(file.node_count)
                .ok_or(Error::BackendFailure)?;
            let file_source_end = file
                .source_start
                .checked_add(file.source_length)
                .ok_or(Error::BackendFailure)?;
            if file_node_end as usize > self.batch.nodes.len()
                || file_source_end as usize > self.batch.source.len()
            {
                return Err(Error::BackendFailure);
            }
            let captures = checked_range(
                result.capture_start,
                result.capture_count,
                self.final_captures.len(),
            )?;
            if captures.start < capture_range.start || captures.end > capture_range.end {
                return Err(Error::BackendFailure);
            }
            for (capture_offset, capture) in
                self.final_captures[captures.clone()].iter().enumerate()
            {
                capture_files[captures.start - capture_range.start + capture_offset] =
                    Some(task.file as usize);
                if capture.node < file.node_start
                    || capture.node >= file_node_end
                    || capture.start < file.source_start
                    || capture.end < capture.start
                    || capture.end > file_source_end
                {
                    return Err(Error::BackendFailure);
                }
                if capture.text_start != NONE {
                    let text = checked_range(
                        capture.text_start,
                        capture.text_length,
                        self.output_storage.len(),
                    )?;
                    if text.start < byte_range.start
                        || text.end > byte_range.end
                        || std::str::from_utf8(&self.output_storage[text]).is_err()
                    {
                        return Err(Error::BackendFailure);
                    }
                }
            }
        }
        for property in &self.properties[property_range] {
            match property.kind {
                0 | 2 => {
                    let offset = property
                        .owner
                        .checked_sub(result_range.start as u32)
                        .map(|offset| offset as usize)
                        .filter(|offset| *offset < result_tasks.len())
                        .ok_or(Error::BackendFailure)?;
                    let task_index = result_tasks[offset].ok_or(Error::BackendFailure)?;
                    let counts = &mut result_property_counts[offset];
                    if property.kind == 0 {
                        counts.0 = counts.0.checked_add(1).ok_or(Error::BackendFailure)?;
                        continue;
                    }
                    counts.1 = counts.1.checked_add(1).ok_or(Error::BackendFailure)?;
                    let task = &self.batch.tasks[task_index];
                    let file = &self.batch.files[task.file as usize];
                    let file_source_end = file
                        .source_start
                        .checked_add(file.source_length)
                        .ok_or(Error::BackendFailure)?;
                    let key = checked_range(
                        property.key_start,
                        property.key_length,
                        self.output_storage.len(),
                    )?;
                    if key.start < byte_range.start
                        || key.end > byte_range.end
                        || std::str::from_utf8(&self.output_storage[key]).is_err()
                    {
                        return Err(Error::BackendFailure);
                    }
                    let value = checked_range(
                        property.value_start,
                        property.value_length,
                        if property.value == 1 {
                            self.output_storage.len()
                        } else {
                            self.batch.source.len()
                        },
                    )?;
                    let valid = if property.value == 1 {
                        value.start >= byte_range.start
                            && value.end <= byte_range.end
                            && std::str::from_utf8(&self.output_storage[value]).is_ok()
                    } else {
                        value.start >= file.source_start as usize
                            && value.end <= file_source_end as usize
                            && std::str::from_utf8(&self.batch.source[value]).is_ok()
                    };
                    if !valid {
                        return Err(Error::BackendFailure);
                    }
                }
                1 => {
                    let offset = property
                        .owner
                        .checked_sub(capture_range.start as u32)
                        .map(|offset| offset as usize)
                        .filter(|offset| *offset < capture_files.len())
                        .ok_or(Error::BackendFailure)?;
                    capture_files[offset].ok_or(Error::BackendFailure)?;
                    capture_property_counts[offset] = capture_property_counts[offset]
                        .checked_add(1)
                        .ok_or(Error::BackendFailure)?;
                }
                _ => return Err(Error::BackendFailure),
            }
        }
        for (offset, result) in self.results[result_range.clone()].iter().enumerate() {
            if result_property_counts[offset] != (result.property_count, result.dynamic_count) {
                return Err(Error::BackendFailure);
            }
        }
        for (offset, capture) in self.final_captures[capture_range].iter().enumerate() {
            if capture_property_counts[offset] != capture.property_count {
                return Err(Error::BackendFailure);
            }
        }
        if result_counts
            .iter()
            .zip(self.statuses)
            .any(|(count, status)| *count != status.result_count)
        {
            return Err(Error::BackendFailure);
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct PackedExecution {
    pub(super) batch: Arc<PackedBatch>,
    pages: Arc<Vec<PackedResultPage>>,
    evaluation_indexes: Arc<Vec<Option<usize>>>,
    file_task_ordinal_bases: Arc<Vec<u32>>,
    grid_errors: Option<Arc<Vec<Option<Error>>>>,
    file_start: usize,
    file_count: usize,
    program_start: usize,
    program_count: usize,
    publication_files: Range<usize>,
}

#[track_caller]
fn grid_result_failure() -> Error {
    if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
        eprintln!(
            "TEMPO_GRID_RESULT_INVARIANT {}",
            std::panic::Location::caller()
        );
    }
    Error::BackendFailure
}

impl PackedExecution {
    fn single_page(&self) -> Result<&PackedResultPage, Error> {
        match self.pages.as_slice() {
            [page] => Ok(page),
            _ => Err(grid_result_failure()),
        }
    }

    // Test oracle: one file's interpreter values through the production publication adapter.
    #[cfg(test)]
    pub(super) fn interpret_reference(
        programs: &[Program],
        file: &TapeDoc,
        roots: &[Vec<u32>],
        limits: &[u32],
    ) -> Result<Self, Error> {
        Self::interpret(programs, std::slice::from_ref(file), &[roots.to_vec()], limits)
    }

    /// The batch on the CPU interpreter, published as the JIT's would be, without planning a JIT
    /// workspace: the CPU backend until its compiled programs are ready (`compilation`).
    pub(super) fn interpret(
        programs: &[Program],
        files: &[TapeDoc],
        anchors: &[Vec<Vec<u32>>],
        limits: &[u32],
    ) -> Result<Self, Error> {
        fn append_bytes(arena: &mut Vec<u8>, bytes: &[u8]) -> Result<(u32, u32), Error> {
            let start = checked_u32(arena.len())?;
            let count = checked_u32(bytes.len())?;
            if start == NONE || start.checked_add(count).is_none() {
                return Err(Error::BackendCapacity);
            }
            arena.extend_from_slice(bytes);
            Ok((start, count))
        }
        if anchors.len() != files.len()
            || anchors.iter().any(|roots| roots.len() != programs.len())
            || programs.len() != limits.len()
        {
            return Err(Error::InvalidAnchor);
        }
        let input = super::metal_grid::MetalGridInput::build(programs, files, anchors)?;
        let BatchOutcome { outputs, errors } = super::execute_batch_outcomes(programs, files, anchors)?;
        if outputs.len() != files.len()
            || errors.len() != files.len()
            || outputs.iter().zip(&errors).any(|(outputs, errors)| outputs.len() != programs.len() || errors.len() != programs.len())
        {
            return Err(Error::BackendFailure);
        }
        let mut results = Vec::new();
        let mut captures = Vec::new();
        let mut properties = Vec::new();
        let mut bytes = Vec::new();
        // The interpreter's nodes and spans are the file's; the batch's are past the files before.
        let bases = input.packed.files.iter().map(|file| (file.node_start, file.source_start)).collect::<Vec<_>>();
        for ((base, source), program, error, mut outputs) in outputs
            .into_iter()
            .zip(&errors)
            .zip(&bases)
            .flat_map(|((outputs, errors), &bases)| {
                outputs.into_iter().zip(errors).enumerate().map(move |(program, (outputs, error))| (bases, program, error, outputs))
            })
        {
            if error.is_some() && !outputs.is_empty() {
                return Err(Error::BackendFailure);
            }
            // Limits select complete interpreter results. They cannot hide a
            // later operand or work failure in the original evaluation.
            outputs.truncate(usize::try_from(limits[program]).map_err(|_| Error::BackendCapacity)?);
            for output in outputs {
                let row = checked_u32(results.len())?;
                let capture_start = checked_u32(captures.len())?;
                let property_start = checked_u32(properties.len())?;
                let property_count = checked_u32(output.properties.len())?;
                for (key, value) in output.properties {
                    properties.push(PropertyRecord {
                        kind: 0,
                        owner: row,
                        key: u32::from(key),
                        value: value.map_or(NONE, u32::from),
                        ..PropertyRecord::default()
                    });
                }
                for capture in output.captures {
                    let owner = checked_u32(captures.len())?;
                    let property_start = checked_u32(properties.len())?;
                    let property_count = checked_u32(capture.properties.len())?;
                    for (key, value) in capture.properties {
                        properties.push(PropertyRecord {
                            kind: 1,
                            owner,
                            key: u32::from(key),
                            value: value.map_or(NONE, u32::from),
                            ..PropertyRecord::default()
                        });
                    }
                    let (text_start, text_length) = match capture.text {
                        Some(text) => append_bytes(&mut bytes, &text)?,
                        None => (NONE, 0),
                    };
                    captures.push(FinalCaptureRecord {
                        label: u32::from(capture.label),
                        node: base + capture.node,
                        start: source + checked_u32(capture.span.start)?,
                        end: source + checked_u32(capture.span.end)?,
                        text_start,
                        text_length,
                        property_start,
                        property_count,
                        retained: 1,
                    });
                }
                let dynamic_start = checked_u32(properties.len())?;
                let dynamic_count = checked_u32(output.dynamic_properties.len())?;
                for (key, value) in output.dynamic_properties {
                    let value = value.ok_or(Error::BackendFailure)?;
                    let (key_start, key_length) = append_bytes(&mut bytes, &key)?;
                    let (value_start, value_length) = append_bytes(&mut bytes, &value)?;
                    properties.push(PropertyRecord {
                        kind: 2,
                        owner: row,
                        value: 1,
                        key_start,
                        key_length,
                        value_start,
                        value_length,
                        ..PropertyRecord::default()
                    });
                }
                results.push(ResultRecord {
                    task_ordinal: output.task_ordinal,
                    local_ordinal: output.local_ordinal,
                    start: base + output.start,
                    label: u32::from(output.label),
                    capture_start,
                    capture_count: checked_u32(captures.len())? - capture_start,
                    property_start,
                    property_count,
                    dynamic_start,
                    dynamic_count,
                });
            }
        }
        let errors = input
            .grid
            .evaluation_spans
            .iter()
            .map(|span| errors[span.file as usize][span.program as usize].clone())
            .collect();
        Self::from_grid_arenas(
            input.packed,
            &input.grid.evaluation_spans,
            results,
            captures,
            properties,
            bytes,
            errors,
        )
    }

    pub(super) fn new(batch: PackedBatch, page: PackedResultPage) -> Self {
        let file_count = batch.files.len();
        let program_count = batch.programs.len();
        let mut evaluation_indexes = vec![None; file_count.saturating_mul(program_count)];
        let mut file_task_ordinal_bases = vec![u32::MAX; file_count];
        for (evaluation_index, evaluation) in batch.evaluations.iter().enumerate() {
            let task = &batch.tasks[evaluation.task_start as usize];
            evaluation_indexes[task.file as usize * program_count + task.program as usize] =
                Some(evaluation_index);
            file_task_ordinal_bases[task.file as usize] =
                file_task_ordinal_bases[task.file as usize].min(task.batch_ordinal);
        }
        file_task_ordinal_bases
            .iter_mut()
            .filter(|base| **base == u32::MAX)
            .for_each(|base| *base = 0);
        Self {
            batch: Arc::new(batch),
            pages: Arc::new(vec![page]),
            evaluation_indexes: Arc::new(evaluation_indexes),
            file_task_ordinal_bases: Arc::new(file_task_ordinal_bases),
            grid_errors: None,
            file_start: 0,
            file_count,
            program_start: 0,
            program_count,
            publication_files: 0..file_count,
        }
    }

    pub(super) fn from_grid_records(
        batch: impl Into<Arc<PackedBatch>>,
        evaluations: &[super::metal_grid::EvaluationSpan],
        results: Vec<ResultRecord>,
        captures: Vec<FinalCaptureRecord>,
        errors: Vec<Option<Error>>,
    ) -> Result<Self, Error> {
        Self::from_grid_arenas(
            batch,
            evaluations,
            results,
            captures,
            Vec::new(),
            Vec::new(),
            errors,
        )
    }

    pub(super) fn from_grid_arenas(
        batch: impl Into<Arc<PackedBatch>>,
        evaluations: &[super::metal_grid::EvaluationSpan],
        results: Vec<ResultRecord>,
        captures: Vec<FinalCaptureRecord>,
        properties: Vec<PropertyRecord>,
        output_storage: Vec<u8>,
        errors: Vec<Option<Error>>,
    ) -> Result<Self, Error> {
        let publication = GridPublication::new(batch.into(), evaluations, errors)?;
        let fragment = publication.validate_page(results, captures, properties, output_storage)?;
        publication.execution(vec![fragment], 0..publication.template.file_count)
    }

    pub(super) fn into_outcome(self) -> Result<BatchOutcome, Error> {
        if self.file_start != 0
            || self.file_count != self.batch.files.len()
            || self.program_start != 0
            || self.program_count != self.batch.programs.len()
        {
            return Err(Error::InvalidAnchor);
        }
        if self.grid_errors.is_some() {
            let mut outputs = vec![vec![Vec::new(); self.program_count]; self.file_count];
            let mut errors = vec![vec![None; self.program_count]; self.file_count];
            for file in 0..self.file_count {
                for program in 0..self.program_count {
                    let (results, error) = self.program_results(file, program)?;
                    errors[file][program] = error;
                    for result in results {
                        outputs[file][program].push(Output {
                            task_ordinal: result.record.task_ordinal,
                            local_ordinal: result.local_ordinal(),
                            start: result.start_node(),
                            label: result.label(),
                            captures: result
                                .captures()
                                .map(|capture| Capture {
                                    label: capture.label(),
                                    node: capture.node(),
                                    span: capture.span(),
                                    text: capture.transformed_text().map(<[u8]>::to_vec),
                                    properties: capture.properties().collect(),
                                })
                                .collect(),
                            properties: result.properties().collect(),
                            dynamic_properties: result
                                .dynamic_properties()
                                .map(|(key, value)| (key.to_vec(), value.map(<[u8]>::to_vec)))
                                .collect(),
                        });
                    }
                }
            }
            Ok(BatchOutcome { outputs, errors })
        } else {
            self.single_page()?.decode_outcomes(&self.batch)
        }
    }

    pub(super) fn view(
        &self,
        file_start: usize,
        file_count: usize,
        program_start: usize,
        program_count: usize,
    ) -> Result<Self, Error> {
        if file_start
            .checked_add(file_count)
            .is_none_or(|end| end > self.batch.files.len())
            || program_start
                .checked_add(program_count)
                .is_none_or(|end| end > self.batch.programs.len())
        {
            return Err(Error::InvalidAnchor);
        }
        Ok(Self {
            file_start,
            file_count,
            program_start,
            program_count,
            ..self.clone()
        })
    }

    pub(crate) fn file_count(&self) -> usize {
        self.file_count
    }

    pub(crate) fn program_count(&self) -> usize {
        self.program_count
    }

    #[cfg(all(test, target_os = "macos"))]
    pub(super) fn host_arena_counts(&self) -> [usize; 4] {
        self.pages.iter().fold([0; 4], |mut counts, page| {
            for (count, length) in counts.iter_mut().zip([
                page.results.len(),
                page.final_captures.len(),
                page.properties.len(),
                page.output_storage.len(),
            ]) {
                *count += length;
            }
            counts
        })
    }

    #[cfg(all(test, target_os = "macos"))]
    pub(super) fn physical_file_count(&self) -> usize {
        self.batch.files.len()
    }

    pub(crate) fn program_results(
        &self,
        file: usize,
        program: usize,
    ) -> Result<(Vec<PackedResultView<'_>>, Option<Error>), Error> {
        if file >= self.file_count || program >= self.program_count {
            return Err(Error::InvalidAnchor);
        }
        if !self.publication_files.contains(&file) {
            return Ok((Vec::new(), None));
        }
        let file = self.file_start + file;
        let program = self.program_start + program;
        let evaluation = self.evaluation_indexes[file * self.batch.programs.len() + program];
        let Some(evaluation) = evaluation else {
            return Ok((Vec::new(), None));
        };
        if let Some(errors) = &self.grid_errors {
            if let Some(error) = &errors[evaluation] {
                return Ok((Vec::new(), Some(error.clone())));
            }
        } else {
            let record = &self.batch.evaluations[evaluation];
            let statuses = &self.single_page()?.statuses
                [record.task_start as usize..(record.task_start + record.task_count) as usize];
            if let Some(error) = statuses.iter().find_map(status_error) {
                return Ok((Vec::new(), Some(error)));
            }
        }
        let file_record = &self.batch.files[file];
        let task_ordinal_base = self.file_task_ordinal_bases[file];
        let mut results = Vec::new();
        for page in self.pages.iter() {
            for &index in page.evaluation_results.get(evaluation)? {
                results.push(PackedResultView {
                    page,
                    record: &page.results[index as usize],
                    file: file_record,
                    source: &self.batch.source,
                    task_ordinal_base,
                });
            }
        }
        Ok((results, None))
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PackedCaptureView<'a> {
    page: &'a PackedResultPage,
    record: &'a FinalCaptureRecord,
    file: &'a BatchFile,
}

impl<'a> PackedCaptureView<'a> {
    pub(crate) fn label(self) -> u16 {
        self.record.label as u16
    }

    pub(crate) fn node(self) -> u32 {
        self.record.node - self.file.node_start
    }

    pub(crate) fn span(self) -> std::ops::Range<usize> {
        (self.record.start - self.file.source_start) as usize
            ..(self.record.end - self.file.source_start) as usize
    }

    pub(crate) fn transformed_text(self) -> Option<&'a [u8]> {
        (self.record.text_start != NONE).then(|| {
            &self.page.output_storage[self.record.text_start as usize
                ..(self.record.text_start + self.record.text_length) as usize]
        })
    }

    pub(crate) fn properties(self) -> impl Iterator<Item = (u16, Option<u16>)> + 'a {
        let start = self.record.property_start as usize;
        let end = start + self.record.property_count as usize;
        self.page.properties[start..end].iter().map(|property| {
            (
                property.key as u16,
                (property.value != NONE).then_some(property.value as u16),
            )
        })
    }
}

#[derive(Clone, Copy)]
pub(crate) struct PackedResultView<'a> {
    page: &'a PackedResultPage,
    record: &'a ResultRecord,
    file: &'a BatchFile,
    source: &'a [u8],
    task_ordinal_base: u32,
}

impl<'a> PackedResultView<'a> {
    pub(crate) fn task_ordinal(self) -> u32 {
        self.record.task_ordinal - self.task_ordinal_base
    }

    pub(crate) fn local_ordinal(self) -> u32 {
        self.record.local_ordinal
    }

    pub(crate) fn start_node(self) -> u32 {
        self.record.start - self.file.node_start
    }

    pub(crate) fn label(self) -> u16 {
        self.record.label as u16
    }

    pub(crate) fn captures(self) -> impl Iterator<Item = PackedCaptureView<'a>> + 'a {
        let start = self.record.capture_start;
        self.page.final_captures[start as usize..(start + self.record.capture_count) as usize]
            .iter()
            .enumerate()
            .filter(|(_, capture)| capture.retained != 0)
            .map(move |(_, record)| PackedCaptureView {
                page: self.page,
                record,
                file: self.file,
            })
    }

    pub(crate) fn properties(self) -> impl Iterator<Item = (u16, Option<u16>)> + 'a {
        let start = self.record.property_start as usize;
        let end = start + self.record.property_count as usize;
        self.page.properties[start..end].iter().map(|property| {
            (
                property.key as u16,
                (property.value != NONE).then_some(property.value as u16),
            )
        })
    }

    pub(crate) fn dynamic_properties(
        self,
    ) -> impl Iterator<Item = (&'a [u8], Option<&'a [u8]>)> + 'a {
        let start = self.record.dynamic_start as usize;
        let end = start + self.record.dynamic_count as usize;
        self.page.properties[start..end]
            .iter()
            .map(move |property| {
                let key = &self.page.output_storage[property.key_start as usize
                    ..(property.key_start + property.key_length) as usize];
                let bytes = if property.value == 1 {
                    &self.page.output_storage
                } else {
                    self.source
                };
                let value = &bytes[property.value_start as usize
                    ..(property.value_start + property.value_length) as usize];
                (key, Some(value))
            })
    }
}

// Sparse indexes retain the complete evaluation domain without allocating one
// empty vector per evaluation for every bounded output fragment.
#[derive(Debug)]
struct EvaluationRows {
    count: usize,
    rows: BTreeMap<usize, Vec<u32>>,
}
impl EvaluationRows {
    fn new(count: usize) -> Self {
        Self {
            count,
            rows: BTreeMap::new(),
        }
    }
    fn push(&mut self, evaluation: usize, row: u32) -> Result<(), Error> {
        if evaluation >= self.count {
            return Err(grid_result_failure());
        }
        self.rows.entry(evaluation).or_default().push(row);
        Ok(())
    }
    fn get(&self, evaluation: usize) -> Result<&[u32], Error> {
        if evaluation >= self.count {
            return Err(grid_result_failure());
        }
        Ok(self.rows.get(&evaluation).map_or(&[], Vec::as_slice))
    }
}
type GridOrder = (u32, u32, Option<u32>, u32);
pub(super) struct GridFragment {
    page: PackedResultPage,
    first: Option<GridOrder>,
    last: Option<GridOrder>,
}
pub(super) struct GridPublication {
    template: PackedExecution,
    evaluations: Vec<super::metal_grid::EvaluationSpan>,
}
impl GridPublication {
    pub(super) fn new(
        batch: Arc<PackedBatch>,
        evaluations: &[super::metal_grid::EvaluationSpan],
        errors: Vec<Option<Error>>,
    ) -> Result<Self, Error> {
        if !batch.tasks.is_empty() || errors.len() != evaluations.len() {
            return Err(grid_result_failure());
        }
        let file_count = batch.files.len();
        let program_count = batch.programs.len();
        let mut evaluation_indexes = vec![
            None;
            file_count
                .checked_mul(program_count)
                .ok_or(Error::BackendCapacity)?
        ];
        let mut file_bases = vec![u32::MAX; file_count];
        for (index, evaluation) in evaluations.iter().enumerate() {
            let slot = evaluation_indexes
                .get_mut(evaluation.file as usize * program_count + evaluation.program as usize)
                .ok_or(Error::BackendFailure)?;
            if slot.replace(index).is_some() {
                return Err(grid_result_failure());
            }
            file_bases[evaluation.file as usize] =
                file_bases[evaluation.file as usize].min(evaluation.semantic_start);
        }
        file_bases
            .iter_mut()
            .filter(|base| **base == u32::MAX)
            .for_each(|base| *base = 0);
        Ok(Self {
            evaluations: evaluations.to_vec(),
            template: PackedExecution {
                batch,
                pages: Arc::new(Vec::new()),
                evaluation_indexes: Arc::new(evaluation_indexes),
                file_task_ordinal_bases: Arc::new(file_bases),
                grid_errors: Some(Arc::new(errors)),
                file_start: 0,
                file_count,
                program_start: 0,
                program_count,
                publication_files: 0..file_count,
            },
        })
    }
    pub(super) fn validate_page(
        &self,
        results: Vec<ResultRecord>,
        captures: Vec<FinalCaptureRecord>,
        properties: Vec<PropertyRecord>,
        output_storage: Vec<u8>,
    ) -> Result<GridFragment, Error> {
        let batch = &self.template.batch;
        let evaluations = &self.evaluations;
        let errors = self
            .template
            .grid_errors
            .as_ref()
            .ok_or(Error::BackendFailure)?;
        let mut expected_capture_start = 0_usize;
        let mut expected_property_start = 0_usize;
        let mut expected_byte_start = 0_usize;
        let mut previous_order = None;
        let mut first_order = None;
        let mut evaluation_results = EvaluationRows::new(evaluations.len());
        for (index, result) in results.iter().enumerate() {
            let evaluation_index = evaluations
                .partition_point(|span| span.semantic_start <= result.task_ordinal)
                .checked_sub(1)
                .ok_or(Error::BackendFailure)?;
            let evaluation = evaluations[evaluation_index];
            if result.task_ordinal - evaluation.semantic_start >= evaluation.root_count
                || errors[evaluation_index].is_some()
            {
                return Err(grid_result_failure());
            }
            let program = batch.programs[evaluation.program as usize];
            if result.label >= program.label_count
                || result.capture_start as usize != expected_capture_start
            {
                return Err(grid_result_failure());
            }
            let file = batch.files[evaluation.file as usize];
            if result.start < file.node_start || result.start - file.node_start >= file.node_count {
                return Err(grid_result_failure());
            }
            let range = checked_range(result.capture_start, result.capture_count, captures.len())?;
            expected_capture_start = range.end;
            let first_capture = if program.flags & 1 != 0 {
                captures[range.clone()]
                    .iter()
                    .find(|capture| capture.retained != 0)
                    .map(|capture| capture.node)
            } else {
                None
            };
            let order = (
                result.task_ordinal,
                result.start,
                first_capture,
                result.local_ordinal,
            );
            if previous_order.is_some_and(|previous| previous >= order) {
                return Err(grid_result_failure());
            }
            first_order.get_or_insert(order);
            previous_order = Some(order);
            if captures[range].iter().any(|capture| {
                capture.label >= program.label_count
                    || capture.retained > 1
                    || (program.output_count == 0
                        && (capture.retained != 1
                            || capture.text_start != NONE
                            || capture.property_count != 0))
                    || capture.node < file.node_start
                    || capture.node - file.node_start >= file.node_count
                    || capture.start < file.source_start
                    || capture.end < capture.start
                    || capture.end - file.source_start > file.source_length
            }) {
                return Err(grid_result_failure());
            }
            let match_properties = checked_range(
                result.property_start,
                result.property_count,
                properties.len(),
            )?;
            if result.property_count != 0 && match_properties.start != expected_property_start {
                return Err(grid_result_failure());
            }
            if properties[match_properties.clone()].iter().any(|property| {
                property.kind != 0
                    || property.owner as usize != index
                    || property.key >= program.string_count
                    || (property.value != NONE && property.value >= program.string_count)
            }) {
                return Err(grid_result_failure());
            }
            expected_property_start += result.property_count as usize;
            for (capture_index, capture) in captures
                [checked_range(result.capture_start, result.capture_count, captures.len())?]
            .iter()
            .enumerate()
            {
                let range = checked_range(
                    capture.property_start,
                    capture.property_count,
                    properties.len(),
                )?;
                if capture.property_count != 0 && range.start != expected_property_start {
                    return Err(grid_result_failure());
                }
                if properties[range].iter().any(|property| {
                    property.kind != 1
                        || property.owner != result.capture_start + capture_index as u32
                        || property.key >= program.string_count
                        || (property.value != NONE && property.value >= program.string_count)
                }) {
                    return Err(grid_result_failure());
                }
                expected_property_start += capture.property_count as usize;
                if capture.text_start != NONE {
                    if capture.text_start as usize != expected_byte_start {
                        return Err(grid_result_failure());
                    }
                    expected_byte_start = expected_byte_start
                        .checked_add(capture.text_length as usize)
                        .ok_or(Error::BackendCapacity)?;
                    let text = checked_range(
                        capture.text_start,
                        capture.text_length,
                        output_storage.len(),
                    )?;
                    if std::str::from_utf8(&output_storage[text]).is_err() {
                        return Err(grid_result_failure());
                    }
                } else if capture.text_length != 0 {
                    return Err(grid_result_failure());
                }
            }
            let dynamic =
                checked_range(result.dynamic_start, result.dynamic_count, properties.len())?;
            if result.dynamic_count != 0 && dynamic.start != expected_property_start {
                return Err(grid_result_failure());
            }
            for property in &properties[dynamic] {
                if property.kind != 2 || property.owner as usize != index || property.value > 1 {
                    return Err(grid_result_failure());
                }
                if property.key_start as usize != expected_byte_start {
                    return Err(grid_result_failure());
                }
                expected_byte_start = expected_byte_start
                    .checked_add(property.key_length as usize)
                    .ok_or(Error::BackendCapacity)?;
                if property.value == 1 {
                    if property.value_start as usize != expected_byte_start {
                        return Err(grid_result_failure());
                    }
                    expected_byte_start = expected_byte_start
                        .checked_add(property.value_length as usize)
                        .ok_or(Error::BackendCapacity)?;
                }
                let key = checked_range(
                    property.key_start,
                    property.key_length,
                    output_storage.len(),
                )?;
                if std::str::from_utf8(&output_storage[key]).is_err() {
                    return Err(grid_result_failure());
                }
                let source = if property.value == 1 {
                    output_storage.as_slice()
                } else {
                    batch.source.as_slice()
                };
                let value =
                    checked_range(property.value_start, property.value_length, source.len())?;
                if property.value == 0
                    && (value.start < file.source_start as usize
                        || value.end > (file.source_start + file.source_length) as usize)
                {
                    return Err(grid_result_failure());
                }
                if std::str::from_utf8(&source[value]).is_err() {
                    return Err(grid_result_failure());
                }
            }
            expected_property_start += result.dynamic_count as usize;
            evaluation_results.push(evaluation_index, checked_u32(index)?)?;
        }
        if expected_capture_start != captures.len()
            || expected_property_start != properties.len()
            || expected_byte_start != output_storage.len()
        {
            return Err(grid_result_failure());
        }
        let page = PackedResultPage {
            statuses: Vec::new(),
            result_order: (0..checked_u32(results.len())?).collect(),
            results,
            final_captures: captures,
            properties,
            output_storage,
            evaluation_results,
        };
        Ok(GridFragment {
            page,
            first: first_order,
            last: previous_order,
        })
    }
    pub(super) fn execution(
        &self,
        fragments: Vec<GridFragment>,
        files: Range<usize>,
    ) -> Result<PackedExecution, Error> {
        if files.start > files.end || files.end > self.template.file_count {
            return Err(Error::InvalidAnchor);
        }
        let mut previous = None;
        let mut pages = Vec::with_capacity(fragments.len());
        for fragment in fragments {
            if let Some(first) = fragment.first {
                if previous.is_some_and(|last| last >= first) {
                    return Err(grid_result_failure());
                }
                previous = fragment.last;
            }
            pages.push(fragment.page);
        }
        let mut execution = self.template.clone();
        execution.pages = Arc::new(pages);
        execution.publication_files = files;
        Ok(execution)
    }
}
#[derive(Debug)]
pub(super) struct PackedResultPage {
    statuses: Vec<TaskStatus>,
    results: Vec<ResultRecord>,
    final_captures: Vec<FinalCaptureRecord>,
    properties: Vec<PropertyRecord>,
    output_storage: Vec<u8>,
    result_order: Vec<u32>,
    evaluation_results: EvaluationRows,
}

fn copy_output_bytes(
    storage: &[u8],
    start: u32,
    length: u32,
    compact: &mut Vec<u8>,
) -> Result<u32, Error> {
    let range = checked_range(start, length, storage.len())?;
    let compact_start = checked_u32(compact.len())?;
    compact.extend_from_slice(&storage[range]);
    Ok(compact_start)
}

impl PackedResultPage {
    pub(super) fn empty() -> Self {
        Self {
            statuses: Vec::new(),
            results: Vec::new(),
            final_captures: Vec::new(),
            properties: Vec::new(),
            output_storage: Vec::new(),
            result_order: Vec::new(),
            evaluation_results: EvaluationRows::new(0),
        }
    }

    pub(super) fn compact(
        batch: &PackedBatch,
        statuses: &[TaskStatus],
        results: &[ResultRecord],
        final_captures: &[FinalCaptureRecord],
        properties: &[PropertyRecord],
        output_storage: &[u8],
    ) -> Result<Self, Error> {
        PackedResults::new(
            batch,
            statuses,
            results,
            final_captures,
            properties,
            output_storage,
        )?;
        let mut failed_evaluations = vec![false; batch.evaluations.len()];
        let mut task_by_ordinal = vec![None; batch.tasks.len()];
        for (task_index, task) in batch.tasks.iter().enumerate() {
            task_by_ordinal[task.batch_ordinal as usize] = Some(task_index);
            if statuses[task_index].error != 0 {
                failed_evaluations[task.evaluation as usize] = true;
            }
        }
        let result_range = checked_range(
            batch.arena_start(ArenaKind::Result),
            batch.arena_used(ArenaKind::Result),
            results.len(),
        )?;
        let capture_range = checked_range(
            batch.arena_start(ArenaKind::FinalCapture),
            batch.arena_used(ArenaKind::FinalCapture),
            final_captures.len(),
        )?;
        let property_range = checked_range(
            batch.arena_start(ArenaKind::Property),
            batch.arena_used(ArenaKind::Property),
            properties.len(),
        )?;
        let mut result_properties = vec![Vec::new(); result_range.len()];
        let mut capture_properties = vec![Vec::new(); capture_range.len()];
        for property in &properties[property_range] {
            let buckets = if property.kind == 1 {
                &mut capture_properties
            } else {
                &mut result_properties
            };
            let start = if property.kind == 1 {
                capture_range.start
            } else {
                result_range.start
            };
            let offset = property.owner as usize - start;
            buckets[offset].push(property);
        }
        let mut page = Self {
            statuses: statuses.to_vec(),
            results: Vec::new(),
            final_captures: Vec::new(),
            properties: Vec::new(),
            output_storage: Vec::new(),
            result_order: Vec::new(),
            evaluation_results: EvaluationRows::new(batch.evaluations.len()),
        };
        let mut result_evaluations = Vec::new();
        for (result_offset, source_result) in results[result_range.clone()].iter().enumerate() {
            let task_index = task_by_ordinal[source_result.task_ordinal as usize]
                .ok_or(Error::BackendFailure)?;
            let task = &batch.tasks[task_index];
            if failed_evaluations[task.evaluation as usize] {
                continue;
            }
            let mut result = *source_result;
            result.capture_start = checked_u32(page.final_captures.len())?;
            let captures = checked_range(
                source_result.capture_start,
                source_result.capture_count,
                final_captures.len(),
            )?;
            for (capture_offset, source_capture) in
                final_captures[captures.clone()].iter().enumerate()
            {
                let source_capture_index = captures.start + capture_offset;
                let mut capture = *source_capture;
                capture.property_start = checked_u32(page.properties.len())?;
                capture.property_count = 0;
                if capture.text_start != NONE {
                    capture.text_start = copy_output_bytes(
                        output_storage,
                        capture.text_start,
                        capture.text_length,
                        &mut page.output_storage,
                    )?;
                }
                let compact_capture_index = checked_u32(page.final_captures.len())?;
                for source_property in
                    &capture_properties[source_capture_index - capture_range.start]
                {
                    let mut property = **source_property;
                    property.owner = compact_capture_index;
                    page.properties.push(property);
                    capture.property_count = capture
                        .property_count
                        .checked_add(1)
                        .ok_or(Error::BackendCapacity)?;
                }
                page.final_captures.push(capture);
            }
            result.property_start = checked_u32(page.properties.len())?;
            result.property_count = 0;
            let compact_result_index = checked_u32(page.results.len())?;
            for source_property in result_properties[result_offset]
                .iter()
                .filter(|property| property.kind == 0)
            {
                let mut property = **source_property;
                property.owner = compact_result_index;
                page.properties.push(property);
                result.property_count = result
                    .property_count
                    .checked_add(1)
                    .ok_or(Error::BackendCapacity)?;
            }
            result.dynamic_start = checked_u32(page.properties.len())?;
            result.dynamic_count = 0;
            for source_property in result_properties[result_offset]
                .iter()
                .filter(|property| property.kind == 2)
            {
                let mut property = **source_property;
                property.owner = compact_result_index;
                property.key_start = copy_output_bytes(
                    output_storage,
                    property.key_start,
                    property.key_length,
                    &mut page.output_storage,
                )?;
                if property.value == 1 {
                    property.value_start = copy_output_bytes(
                        output_storage,
                        property.value_start,
                        property.value_length,
                        &mut page.output_storage,
                    )?;
                }
                page.properties.push(property);
                result.dynamic_count = result
                    .dynamic_count
                    .checked_add(1)
                    .ok_or(Error::BackendCapacity)?;
            }
            page.result_order.push(compact_result_index);
            result_evaluations.push(task.evaluation);
            page.results.push(result);
        }
        page.result_order.sort_by_key(|index| {
            let result = &page.results[*index as usize];
            let task = batch.tasks[task_by_ordinal[result.task_ordinal as usize].unwrap()];
            let query = batch.programs[task.program as usize].flags & 1 != 0;
            let first_capture = if query {
                page.final_captures[result.capture_start as usize
                    ..(result.capture_start + result.capture_count) as usize]
                    .iter()
                    .find(|capture| capture.retained != 0)
                    .map(|capture| capture.node)
            } else {
                None
            };
            (
                result.task_ordinal,
                result.start,
                first_capture,
                result.local_ordinal,
            )
        });
        for &result in &page.result_order {
            page.evaluation_results
                .push(result_evaluations[result as usize] as usize, result)?;
        }
        Ok(page)
    }

    pub(super) fn decode_outcomes(&self, batch: &PackedBatch) -> Result<BatchOutcome, Error> {
        let mut outputs = (0..batch.files.len())
            .map(|_| {
                batch
                    .programs
                    .iter()
                    .map(|_| Vec::new())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let mut errors = (0..batch.files.len())
            .map(|_| batch.programs.iter().map(|_| None).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        for (task, status) in batch.tasks.iter().zip(&self.statuses) {
            if let Some(error) = status_error(status) {
                errors[task.file as usize][task.program as usize].get_or_insert(error);
                outputs[task.file as usize][task.program as usize].clear();
                continue;
            }
            if errors[task.file as usize][task.program as usize].is_some() {
                continue;
            }
            let file = &batch.files[task.file as usize];
            for result_index in self.result_order.iter().copied() {
                let record = &self.results[result_index as usize];
                if record.task_ordinal != task.batch_ordinal {
                    continue;
                }
                let mut found_captures = Vec::with_capacity(record.capture_count as usize);
                for (capture_offset, captured) in self.final_captures[record.capture_start as usize
                    ..(record.capture_start + record.capture_count) as usize]
                    .iter()
                    .enumerate()
                {
                    if captured.retained == 0 {
                        continue;
                    }
                    let capture_index = record.capture_start + capture_offset as u32;
                    let capture_properties = self
                        .properties
                        .iter()
                        .filter(|property| property.kind == 1 && property.owner == capture_index)
                        .map(|property| {
                            (
                                property.key as u16,
                                (property.value != NONE).then_some(property.value as u16),
                            )
                        })
                        .collect();
                    let text = (captured.text_start != NONE).then(|| {
                        self.output_storage[captured.text_start as usize
                            ..(captured.text_start + captured.text_length) as usize]
                            .to_vec()
                    });
                    found_captures.push(Capture {
                        label: captured.label as u16,
                        node: captured.node - file.node_start,
                        span: (captured.start - file.source_start) as usize
                            ..(captured.end - file.source_start) as usize,
                        text,
                        properties: capture_properties,
                    });
                }
                let indexed_properties = self
                    .properties
                    .iter()
                    .filter(|property| property.kind == 0 && property.owner == result_index)
                    .map(|property| {
                        (
                            property.key as u16,
                            (property.value != NONE).then_some(property.value as u16),
                        )
                    })
                    .collect();
                let dynamic_properties = self
                    .properties
                    .iter()
                    .filter(|property| property.kind == 2 && property.owner == result_index)
                    .map(|property| {
                        let key = self.output_storage[property.key_start as usize
                            ..(property.key_start + property.key_length) as usize]
                            .to_vec();
                        let bytes = if property.value == 1 {
                            &self.output_storage
                        } else {
                            &batch.source
                        };
                        let value = bytes[property.value_start as usize
                            ..(property.value_start + property.value_length) as usize]
                            .to_vec();
                        (key, Some(value))
                    })
                    .collect();
                outputs[task.file as usize][task.program as usize].push(Output {
                    task_ordinal: record.task_ordinal,
                    local_ordinal: record.local_ordinal,
                    start: record.start - file.node_start,
                    label: record.label as u16,
                    captures: found_captures,
                    properties: indexed_properties,
                    dynamic_properties,
                });
            }
        }
        Ok(BatchOutcome { outputs, errors })
    }
}

impl PackedBatch {
    pub(super) fn decode_outcomes(
        &self,
        statuses: &[TaskStatus],
        results: &[ResultRecord],
        final_captures: &[FinalCaptureRecord],
        properties: &[PropertyRecord],
        output_storage: &[u8],
    ) -> Result<BatchOutcome, Error> {
        let packed = PackedResults::new(
            self,
            statuses,
            results,
            final_captures,
            properties,
            output_storage,
        )?;
        let mut outputs = (0..self.files.len())
            .map(|_| self.programs.iter().map(|_| Vec::new()).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        let mut errors = (0..self.files.len())
            .map(|_| self.programs.iter().map(|_| None).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        for (task, status) in packed.batch.tasks.iter().zip(packed.statuses) {
            if let Some(error) = status_error(status) {
                errors[task.file as usize][task.program as usize].get_or_insert(error);
                outputs[task.file as usize][task.program as usize].clear();
                continue;
            }
            if errors[task.file as usize][task.program as usize].is_some() {
                continue;
            }
            let file = &self.files[task.file as usize];
            let result_start = self.arena_start(ArenaKind::Result) as usize;
            let result_end = result_start + self.arena_used(ArenaKind::Result) as usize;
            for (result_offset, record) in results[result_start..result_end]
                .iter()
                .enumerate()
                .filter(|(_, record)| record.task_ordinal == task.batch_ordinal)
            {
                let result_index = result_start as u32 + result_offset as u32;
                let task_properties = &properties[self.arena_start(ArenaKind::Property) as usize
                    ..(self.arena_start(ArenaKind::Property) + self.arena_used(ArenaKind::Property))
                        as usize];
                let mut found_captures = Vec::with_capacity(record.capture_count as usize);
                for (capture_index, captured) in final_captures[record.capture_start as usize
                    ..(record.capture_start + record.capture_count) as usize]
                    .iter()
                    .enumerate()
                {
                    if captured.retained == 0 {
                        continue;
                    }
                    let global_index = record.capture_start + capture_index as u32;
                    let capture_properties = task_properties
                        .iter()
                        .filter(|property| property.kind == 1 && property.owner == global_index)
                        .map(|property| {
                            (
                                property.key as u16,
                                (property.value != NONE).then_some(property.value as u16),
                            )
                        })
                        .collect();
                    let text = (captured.text_start != NONE).then(|| {
                        output_storage[captured.text_start as usize
                            ..(captured.text_start + captured.text_length) as usize]
                            .to_vec()
                    });
                    found_captures.push(Capture {
                        label: captured.label as u16,
                        node: captured.node - file.node_start,
                        span: (captured.start - file.source_start) as usize
                            ..(captured.end - file.source_start) as usize,
                        text,
                        properties: capture_properties,
                    });
                }
                let indexed_properties = task_properties
                    .iter()
                    .filter(|property| property.kind == 0 && property.owner == result_index)
                    .map(|property| {
                        (
                            property.key as u16,
                            (property.value != NONE).then_some(property.value as u16),
                        )
                    })
                    .collect();
                let dynamic_properties = task_properties
                    .iter()
                    .filter(|property| property.kind == 2 && property.owner == result_index)
                    .map(|property| {
                        let key = output_storage[property.key_start as usize
                            ..(property.key_start + property.key_length) as usize]
                            .to_vec();
                        let bytes = if property.value == 1 {
                            output_storage
                        } else {
                            &self.source
                        };
                        let value = bytes[property.value_start as usize
                            ..(property.value_start + property.value_length) as usize]
                            .to_vec();
                        (key, Some(value))
                    })
                    .collect();
                outputs[task.file as usize][task.program as usize].push(Output {
                    task_ordinal: record.task_ordinal,
                    local_ordinal: record.local_ordinal,
                    start: record.start - file.node_start,
                    label: record.label as u16,
                    captures: found_captures,
                    properties: indexed_properties,
                    dynamic_properties,
                });
            }
        }
        for file_outputs in &mut outputs {
            for (program_index, program_outputs) in file_outputs.iter_mut().enumerate() {
                let query = self.programs[program_index].flags & 1 != 0;
                program_outputs.sort_by_key(|output| {
                    (
                        output.task_ordinal,
                        output.start,
                        if query {
                            output.captures.first().map(|capture| capture.node)
                        } else {
                            None
                        },
                        output.local_ordinal,
                    )
                });
            }
        }
        Ok(BatchOutcome { outputs, errors })
    }

    pub(super) fn decode_outputs(
        &self,
        statuses: &[TaskStatus],
        results: &[ResultRecord],
        final_captures: &[FinalCaptureRecord],
        properties: &[PropertyRecord],
        output_storage: &[u8],
    ) -> Result<Vec<Vec<Vec<Output>>>, Error> {
        self.decode_outcomes(
            statuses,
            results,
            final_captures,
            properties,
            output_storage,
        )?
        .into_result()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect as SupportLang;
    use ast_grep_core::tree_sitter::LanguageExt;

    #[test]
    fn streamed_tapes_preserve_all_immutable_grid_input_bytes() {
        use crate::code_rule_dsl::tape::{Grammar, TapeImage, TapeSource};
        use ast_grep_core::tree_sitter::StrDoc;
        let grammar = Grammar::new(SupportLang::Python).unwrap();
        let inputs = ["# comment\nvalue = f'{item}'\n", "def broken(\n  pass\n"].map(|text| {
            let source = Arc::new(text.to_owned());
            let native = StrDoc::new(text, SupportLang::Python);
            TapeSource {
                source: Arc::clone(&source),
                grammar: Arc::clone(&grammar),
                image: Arc::new(
                    TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false)
                        .unwrap(),
                ),
                lines: Arc::new(crate::code_rule_dsl::lines::LineTable::new(&source, crate::code_rule_dsl::lines::Terminators::Newlines)),
            }
        });
        let docs = inputs
            .iter()
            .map(|input| input.restore(&|| false).unwrap())
            .collect::<Vec<_>>();
        let programs = [Program::compile_tree_sitter_query_pattern(
            "(_) @node",
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap()];
        let anchors = docs
            .iter()
            .map(|doc| {
                vec![Arc::<[u32]>::from(
                    (0..doc.transducer_len() as u32).collect::<Vec<_>>(),
                )]
            })
            .collect::<Vec<_>>();
        let expected = PackedBatch::new_shared_grid_inputs(&programs, &docs, &anchors).unwrap();
        let actual = PackedBatch::new_streamed_grid_inputs(&programs, &inputs, &anchors).unwrap();
        assert_eq!(actual.header, expected.header);
        assert_eq!(actual.nodes, expected.nodes);
        assert_eq!(actual.children, expected.children);
        assert_eq!(actual.supertypes, expected.supertypes);
        assert_eq!(actual.line_starts, expected.line_starts);
        assert_eq!(actual.files, expected.files);
        assert_eq!(actual.source, expected.source);
        assert_eq!(actual.words, expected.words);
        assert!(actual.tasks.is_empty());
        let (mut nodes, mut children, mut supertypes, mut lines) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let streamed = PackedBatch::new_streamed_grid_inputs_with_upload(
            &programs,
            &inputs,
            &anchors,
            |batch| {
                nodes.append(&mut batch.nodes);
                children.append(&mut batch.children);
                supertypes.append(&mut batch.supertypes);
                lines.append(&mut batch.line_starts);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(streamed.header, expected.header);
        assert_eq!(streamed.files, expected.files);
        assert_eq!(nodes, expected.nodes);
        assert_eq!(children, expected.children);
        assert_eq!(supertypes, expected.supertypes);
        assert_eq!(lines, expected.line_starts);
        assert!(streamed.nodes.is_empty() && streamed.children.is_empty());
    }

    fn batch() -> PackedBatch {
        let mut batch = PackedBatch::default();
        batch.files.push(BatchFile {
            node_count: 1,
            source_length: 1,
            line_count: 1,
            ..BatchFile::default()
        });
        batch.programs.push(BatchProgram::default());
        batch.tasks.push(BatchTask::default());
        let mut evaluation = MetalEvaluation::default();
        for kind in [
            ArenaKind::Result,
            ArenaKind::FinalCapture,
            ArenaKind::Property,
            ArenaKind::OutputByte,
        ] {
            evaluation.quotas[kind as usize].limit = 1;
            batch.global_arenas.arenas[kind as usize].capacity = 1;
        }
        batch.evaluations.push(evaluation);
        batch.source.push(b'x');
        batch.nodes.push(BatchNode::default());
        batch
    }

    #[test]
    fn direct_grid_result_validation_rejects_malformed_typed_records() {
        let evaluation = super::super::metal_grid::EvaluationSpan {
            root_count: 1,
            ..Default::default()
        };
        let result = ResultRecord {
            capture_count: 1,
            ..Default::default()
        };
        let capture = FinalCaptureRecord {
            end: 1,
            text_start: NONE,
            retained: 1,
            ..Default::default()
        };
        let check = |results: Vec<ResultRecord>,
                     captures: Vec<FinalCaptureRecord>,
                     error: Option<Error>| {
            let mut batch = batch();
            batch.tasks.clear();
            batch.evaluations.clear();
            batch.programs[0].label_count = 1;
            PackedExecution::from_grid_records(batch, &[evaluation], results, captures, vec![error])
        };
        assert!(check(vec![result], vec![capture], None).is_ok());
        for malformed in [
            ResultRecord {
                task_ordinal: 1,
                ..result
            },
            ResultRecord {
                capture_count: 2,
                ..result
            },
            ResultRecord {
                property_count: 1,
                ..result
            },
            ResultRecord { label: 1, ..result },
        ] {
            assert!(matches!(
                check(vec![malformed], vec![capture], None),
                Err(Error::BackendFailure)
            ));
        }
        for malformed in [
            FinalCaptureRecord {
                label: 1,
                ..capture
            },
            FinalCaptureRecord {
                text_start: 0,
                ..capture
            },
            FinalCaptureRecord {
                retained: 2,
                ..capture
            },
            FinalCaptureRecord { end: 2, ..capture },
        ] {
            assert!(matches!(
                check(vec![result], vec![malformed], None),
                Err(Error::BackendFailure)
            ));
        }
        assert!(matches!(
            check(vec![result], vec![capture], Some(Error::WorkLimit)),
            Err(Error::BackendFailure)
        ));
        assert!(matches!(
            check(vec![result, result], vec![capture, capture], None),
            Err(Error::BackendFailure)
        ));
    }

    #[test]
    fn bounded_task_capacity_keeps_zero_and_stops_at_the_limit() {
        assert_eq!(bounded_capacity(0, 256), 0);
        assert_eq!(bounded_capacity(17, 256), 17);
        assert_eq!(bounded_capacity(257, 256), 256);
    }

    #[test]
    fn production_result_limit_caps_evaluation_and_global_result_capacity() {
        let program = Program::new(
            vec![word(STOP, 0, 0), word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap();
        let programs = [program];
        let mut batch = batch();
        let totals = batch
            .assign_workspace_with_result_limits(&programs, Some(&[0]))
            .unwrap();
        assert_eq!(totals.results, 0);
        assert_eq!(
            batch.evaluations[0].quotas[ArenaKind::Result as usize].limit,
            0
        );
    }

    #[test]
    fn capacity_facts_do_not_reserve_unused_mutable_arenas() {
        let linear = Program::new(
            vec![word(STOP, 0, 0), word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap();
        let facts = ProgramCapacityFacts::new(&linear).unwrap();
        assert_eq!(facts.pending_operations, 0);
        assert_eq!(facts.capture_operations, 0);
        assert_eq!(facts.cursor_operations, 0);
        assert_eq!(facts.repeat_operations, 0);
        assert_eq!(facts.greedy_operations, 0);
        assert_eq!(facts.emit_operations, 1);
        assert_eq!(facts.directive_count, 0);
    }

    #[test]
    fn capacity_facts_include_each_structural_scratch_arena() {
        let structural = Program::compile_tree_sitter_query_pattern(
            "(module (comment)+ @item)",
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap();
        let facts = ProgramCapacityFacts::new(&structural).unwrap();
        assert_ne!(facts.pending_operations, 0);
        assert_ne!(facts.capture_operations, 0);
        assert_ne!(facts.cursor_operations, 0);
        assert_ne!(facts.repeat_operations, 0);
        assert_ne!(facts.emit_operations, 0);

        let greedy = Program::new(
            vec![word(FORK, GREEDY, 2), word(EMIT, 0, 0), word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap();
        assert_ne!(
            ProgramCapacityFacts::new(&greedy)
                .unwrap()
                .greedy_operations,
            0
        );
    }

    #[test]
    fn packed_results_reject_every_out_of_range_task_count() {
        for status in [
            TaskStatus {
                result_count: 2,
                ..TaskStatus::default()
            },
            TaskStatus {
                final_capture_count: 2,
                ..TaskStatus::default()
            },
            TaskStatus {
                property_count: 2,
                ..TaskStatus::default()
            },
            TaskStatus {
                output_byte_count: 2,
                ..TaskStatus::default()
            },
        ] {
            assert_eq!(
                batch().decode_outputs(
                    &[status],
                    &[ResultRecord::default()],
                    &[FinalCaptureRecord::default()],
                    &[PropertyRecord::default()],
                    &[0],
                ),
                Err(Error::BackendFailure),
            );
        }
    }

    #[test]
    fn packed_results_keep_task_errors_scoped_to_their_program() {
        let mut batch = batch();
        batch.programs.push(BatchProgram::default());
        batch.evaluations[0].task_count = 1;
        batch.tasks.push(BatchTask {
            evaluation: 1,
            batch_ordinal: 1,
            program: 1,
            ..BatchTask::default()
        });
        batch.evaluations.push(MetalEvaluation {
            task_start: 1,
            task_count: 1,
            ..batch.evaluations[0]
        });
        let outcome = batch
            .decode_outcomes(
                &[
                    TaskStatus {
                        error: 2,
                        ..TaskStatus::default()
                    },
                    TaskStatus::default(),
                ],
                &[ResultRecord::default(); 2],
                &[FinalCaptureRecord::default(); 2],
                &[PropertyRecord::default(); 2],
                &[0; 2],
            )
            .unwrap();
        assert_eq!(
            outcome.errors,
            vec![vec![Some(Error::BackendCapacity), None]]
        );
        assert_eq!(outcome.outputs, vec![vec![Vec::new(), Vec::new()]]);
    }

    #[test]
    fn compact_result_page_preserves_nested_oracle_output() {
        let mut batch = batch();
        batch.global_arenas.arenas[ArenaKind::Result as usize].cursor = 1;
        let statuses = [TaskStatus {
            result_count: 1,
            ..TaskStatus::default()
        }];
        let results = [ResultRecord {
            task_ordinal: 0,
            local_ordinal: 3,
            start: 0,
            label: 7,
            ..ResultRecord::default()
        }];
        let captures = [FinalCaptureRecord::default()];
        let properties = [PropertyRecord::default()];
        let bytes = [0];
        let expected = batch
            .decode_outcomes(&statuses, &results, &captures, &properties, &bytes)
            .unwrap();

        let page =
            PackedResultPage::compact(&batch, &statuses, &results, &captures, &properties, &bytes)
                .unwrap();

        assert_eq!(page.decode_outcomes(&batch).unwrap(), expected);
        assert_eq!(page.results.len(), 1);
        assert_eq!(page.results[0].local_ordinal, 3);
    }

    #[test]
    fn compact_result_page_drops_every_child_of_a_failed_evaluation() {
        let mut batch = batch();
        batch.tasks.push(BatchTask {
            batch_ordinal: 1,
            ..BatchTask::default()
        });
        batch.global_arenas.arenas[ArenaKind::Result as usize].cursor = 1;
        let statuses = [
            TaskStatus {
                result_count: 1,
                ..TaskStatus::default()
            },
            TaskStatus {
                error: 2,
                ..TaskStatus::default()
            },
        ];
        let results = [ResultRecord {
            task_ordinal: 0,
            ..ResultRecord::default()
        }];

        let page = PackedResultPage::compact(
            &batch,
            &statuses,
            &results,
            &[FinalCaptureRecord::default()],
            &[PropertyRecord::default()],
            &[0],
        )
        .unwrap();
        let outcome = page.decode_outcomes(&batch).unwrap();

        assert!(page.results.is_empty());
        assert!(outcome.outputs[0][0].is_empty());
        assert_eq!(outcome.errors[0][0], Some(Error::BackendCapacity));
    }

    #[test]
    fn packed_results_reject_partial_output_from_failed_tasks() {
        assert_eq!(
            batch().decode_outcomes(
                &[TaskStatus {
                    error: 2,
                    result_count: 1,
                    ..TaskStatus::default()
                }],
                &[ResultRecord::default()],
                &[FinalCaptureRecord::default()],
                &[PropertyRecord::default()],
                &[0],
            ),
            Err(Error::BackendFailure),
        );
    }

    #[test]
    fn packed_results_reject_invalid_utf8_transformed_capture_text() {
        let status = TaskStatus {
            result_count: 1,
            final_capture_count: 1,
            output_byte_count: 1,
            ..TaskStatus::default()
        };
        let result = ResultRecord {
            capture_count: 1,
            ..ResultRecord::default()
        };
        let capture = FinalCaptureRecord {
            text_start: 0,
            text_length: 1,
            ..FinalCaptureRecord::default()
        };
        assert_eq!(
            batch().decode_outputs(
                &[status],
                &[result],
                &[capture],
                &[PropertyRecord::default()],
                &[0xff],
            ),
            Err(Error::BackendFailure),
        );
    }
}
