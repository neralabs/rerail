use std::mem::{align_of, size_of};

pub(super) const FORMAT_VERSION: u32 = 4;
pub(super) const ARENA_KIND_COUNT: usize = 14;
pub(super) const NONE: u32 = u32::MAX;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct MetalBatchHeader {
    pub format_version: u32,
    pub task_count: u32,
    pub file_count: u32,
    pub program_count: u32,
    pub node_count: u32,
    pub child_count: u32,
    pub supertype_count: u32,
    pub line_start_count: u32,
    pub string_count: u32,
    pub structural_word_count: u32,
    pub value_instruction_count: u32,
    pub capture_instruction_count: u32,
    pub output_instruction_count: u32,
    pub workspace_byte_count: u32,
    pub evaluation_count: u32,
    pub query_bucket_byte_start: u32,
    pub scheduler_byte_count: u32,
    pub query_member_byte_start: u32,
    pub queued_state_byte_start: u32,
    pub greedy_entry_byte_start: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct MetalNode {
    pub parent: u32,
    pub next_any: u32,
    pub next_named: u32,
    pub child_start: u32,
    pub child_count: u32,
    pub descendant_end: u32,
    pub source_start: u32,
    pub source_end: u32,
    pub kind: u32,
    pub field: u32,
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct MetalFile {
    pub node_start: u32,
    pub node_count: u32,
    pub source_start: u32,
    pub source_length: u32,
    pub line_start: u32,
    pub line_count: u32,
    pub supertype_start: u32,
    pub supertype_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct MetalProgram {
    pub structural_start: u32,
    pub structural_count: u32,
    pub long_start: u32,
    pub long_count: u32,
    pub string_start: u32,
    pub string_count: u32,
    pub value_start: u32,
    pub value_count: u32,
    pub capture_start: u32,
    pub capture_count: u32,
    pub output_start: u32,
    pub output_count: u32,
    pub label_start: u32,
    pub label_count: u32,
    pub maximum_cursor_slot: u32,
    pub maximum_lua_capture_slot: u32,
    /// Bit 0: query capture ordering. Bit 1: exact admission for a query with repeats.
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct MetalString {
    pub byte_start: u32,
    pub byte_length: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct MetalSupertype {
    pub node: u32,
    pub kind: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct ValueRecord {
    pub kind: u32,
    pub operand: u32,
    pub auxiliary: u32,
    pub state_start: u32,
    pub state_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct NfaStateRecord {
    pub kind: u32,
    pub data_start: u32,
    pub data_count: u32,
    pub next: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct NfaRangeRecord {
    pub start: u32,
    pub end: u32,
    pub target: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct LuaInstructionRecord {
    pub kind: u32,
    pub first: u32,
    pub second: u32,
    pub matcher_kind: u32,
    pub matcher_first: u32,
    pub matcher_second: u32,
    pub data_start: u32,
    pub data_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct LuaSetMemberRecord {
    pub kind: u32,
    pub first: u32,
    pub second: u32,
    pub negated: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct PathRecord {
    pub pc: u32,
    pub node: u32,
    pub start: u32,
    pub captures: u32,
    pub cursors: u32,
    pub repeats: u32,
    pub steps: u32,
    pub following: u32,
    pub capture_hash: u32,
    pub cursor_hash: u32,
    pub repeat_hash: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct LinkRecord {
    pub previous: u32,
    pub first: u32,
    pub second: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct RepeatRecord {
    pub previous: u32,
    pub value: u32,
    pub previous_hash: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct QueryStateRecord {
    pub occupied: u32,
    pub pc: u32,
    pub node: u32,
    pub following: u32,
    pub capture_hash: u32,
    pub cursor_hash: u32,
    pub repeat_hash: u32,
    pub captures: u32,
    pub cursors: u32,
    pub repeats: u32,
}

pub(super) const PENDING_PATH: u32 = 0;
pub(super) const PENDING_GREEDY: u32 = 1;
pub(super) const PENDING_FOLLOWING: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct PendingRecord {
    pub kind: u32,
    pub greedy_slot: u32,
    pub path: PathRecord,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct QueryBucketRecord {
    pub occupied: u32,
    pub hash: u32,
    pub pc: u32,
    pub node: u32,
    pub sibling_requirement: u32,
    pub cursors: u32,
    pub repeats: u32,
    pub first_member: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct QueryMemberRecord {
    pub next_member: u32,
    pub captures: u32,
    pub capture_count: u32,
    pub capture_hash: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct QueuedStateRecord {
    pub occupied: u32,
    pub hash: u32,
    pub pc: u32,
    pub node: u32,
    pub sibling_requirement: u32,
    pub captures: u32,
    pub cursors: u32,
    pub repeats: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct GreedyEntryRecord {
    pub occupied: u32,
    pub hash: u32,
    pub pc: u32,
    pub start: u32,
    pub captures: u32,
    pub cursors: u32,
    pub repeats: u32,
    pub path: PathRecord,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct ResultRecord {
    pub task_ordinal: u32,
    pub local_ordinal: u32,
    pub start: u32,
    pub label: u32,
    pub capture_start: u32,
    pub capture_count: u32,
    pub property_start: u32,
    pub property_count: u32,
    pub dynamic_start: u32,
    pub dynamic_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct FinalCaptureRecord {
    pub label: u32,
    pub node: u32,
    pub start: u32,
    pub end: u32,
    pub text_start: u32,
    pub text_length: u32,
    pub property_start: u32,
    pub property_count: u32,
    pub retained: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct PropertyRecord {
    pub kind: u32,
    pub owner: u32,
    pub key: u32,
    pub value: u32,
    pub key_start: u32,
    pub key_length: u32,
    pub value_start: u32,
    pub value_length: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct TaskStatus {
    pub error: u32,
    pub result_count: u32,
    pub capture_count: u32,
    pub program_counter: u32,
    pub final_capture_count: u32,
    pub property_count: u32,
    pub output_byte_count: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct SharedArenaRecord {
    pub start: u32,
    pub capacity: u32,
    pub cursor: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct EvaluationQuotaRecord {
    pub used: u32,
    pub limit: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct LazyArenaBlockRecord {
    pub start: u32,
    pub limit: u32,
    pub state: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct MetalEvaluation {
    pub outcome: u32,
    pub generation: u32,
    pub failure_arena: u32,
    pub failure_task: u32,
    pub failure_pc: u32,
    pub work_used: u32,
    pub workspace_work_estimate: u32,
    pub task_start: u32,
    pub task_count: u32,
    pub quotas: [EvaluationQuotaRecord; ARENA_KIND_COUNT],
    pub pending: LazyArenaBlockRecord,
    pub capture: LazyArenaBlockRecord,
    pub cursor: LazyArenaBlockRecord,
    pub repeat: LazyArenaBlockRecord,
    pub value_state: LazyArenaBlockRecord,
    pub candidate: LazyArenaBlockRecord,
    pub query_bucket: LazyArenaBlockRecord,
    pub query_member: LazyArenaBlockRecord,
    pub queued_state: LazyArenaBlockRecord,
    pub greedy_entry: LazyArenaBlockRecord,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct MetalGlobalArenas {
    pub arenas: [SharedArenaRecord; ARENA_KIND_COUNT],
    pub task_cursor: u32,
    pub reserved: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct MetalTask {
    pub evaluation: u32,
    pub batch_ordinal: u32,
    pub file: u32,
    pub program: u32,
    pub root: u32,
    /// Workspace estimate only. Runtime task and path budgets are independent.
    pub workspace_work_estimate: u32,
}

const _: [(); 80] = [(); size_of::<MetalBatchHeader>()];
const _: [(); 44] = [(); size_of::<MetalNode>()];
const _: [(); 32] = [(); size_of::<MetalFile>()];
const _: [(); 68] = [(); size_of::<MetalProgram>()];
const _: [(); 8] = [(); size_of::<MetalString>()];
const _: [(); 8] = [(); size_of::<MetalSupertype>()];
const _: [(); 44] = [(); size_of::<PathRecord>()];
const _: [(); 12] = [(); size_of::<LinkRecord>()];
const _: [(); 12] = [(); size_of::<RepeatRecord>()];
const _: [(); 40] = [(); size_of::<QueryStateRecord>()];
const _: [(); 52] = [(); size_of::<PendingRecord>()];
const _: [(); 32] = [(); size_of::<QueryBucketRecord>()];
const _: [(); 16] = [(); size_of::<QueryMemberRecord>()];
const _: [(); 32] = [(); size_of::<QueuedStateRecord>()];
const _: [(); 72] = [(); size_of::<GreedyEntryRecord>()];
const _: [(); 40] = [(); size_of::<ResultRecord>()];
const _: [(); 36] = [(); size_of::<FinalCaptureRecord>()];
const _: [(); 32] = [(); size_of::<PropertyRecord>()];
const _: [(); 32] = [(); size_of::<TaskStatus>()];
const _: [(); 12] = [(); size_of::<SharedArenaRecord>()];
const _: [(); 8] = [(); size_of::<EvaluationQuotaRecord>()];
const _: [(); 16] = [(); size_of::<LazyArenaBlockRecord>()];
const _: [(); 308] = [(); size_of::<MetalEvaluation>()];
const _: [(); 184] = [(); size_of::<MetalGlobalArenas>()];
const _: [(); 24] = [(); size_of::<MetalTask>()];

const _: [(); 4] = [(); align_of::<MetalBatchHeader>()];
const _: [(); 4] = [(); align_of::<MetalNode>()];
const _: [(); 4] = [(); align_of::<MetalFile>()];
const _: [(); 4] = [(); align_of::<MetalProgram>()];
const _: [(); 4] = [(); align_of::<MetalTask>()];
const _: [(); 4] = [(); align_of::<MetalEvaluation>()];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metal_batch_abi_has_stable_sizes_and_alignment() {
        assert_eq!(size_of::<MetalBatchHeader>(), 80);
        assert_eq!(size_of::<MetalNode>(), 44);
        assert_eq!(size_of::<MetalFile>(), 32);
        assert_eq!(size_of::<MetalProgram>(), 68);
        assert_eq!(size_of::<MetalString>(), 8);
        assert_eq!(size_of::<MetalSupertype>(), 8);
        assert_eq!(size_of::<PathRecord>(), 44);
        assert_eq!(size_of::<LinkRecord>(), 12);
        assert_eq!(size_of::<RepeatRecord>(), 12);
        assert_eq!(size_of::<QueryStateRecord>(), 40);
        assert_eq!([PENDING_PATH, PENDING_GREEDY, PENDING_FOLLOWING], [0, 1, 2]);
        assert_eq!(size_of::<PendingRecord>(), 52);
        assert_eq!(size_of::<QueryBucketRecord>(), 32);
        assert_eq!(size_of::<QueryMemberRecord>(), 16);
        assert_eq!(size_of::<QueuedStateRecord>(), 32);
        assert_eq!(size_of::<GreedyEntryRecord>(), 72);
        assert_eq!(size_of::<ResultRecord>(), 40);
        assert_eq!(size_of::<FinalCaptureRecord>(), 36);
        assert_eq!(size_of::<PropertyRecord>(), 32);
        assert_eq!(size_of::<TaskStatus>(), 32);
        assert_eq!(size_of::<SharedArenaRecord>(), 12);
        assert_eq!(size_of::<EvaluationQuotaRecord>(), 8);
        assert_eq!(size_of::<LazyArenaBlockRecord>(), 16);
        assert_eq!(size_of::<MetalEvaluation>(), 308);
        assert_eq!(size_of::<MetalGlobalArenas>(), 184);
        assert_eq!(size_of::<MetalTask>(), 24);
        assert_eq!(align_of::<MetalBatchHeader>(), 4);
        assert_eq!(align_of::<MetalTask>(), 4);
    }

    #[test]
    fn metal_batch_abi_uses_an_explicit_version() {
        assert_ne!(FORMAT_VERSION, 0);
        let header = MetalBatchHeader {
            format_version: FORMAT_VERSION,
            ..MetalBatchHeader::default()
        };
        assert_eq!(header.format_version, 4);
        assert_eq!(FORMAT_VERSION, 4);
    }

    #[test]
    fn shared_arena_kind_order_is_stable_in_version_four() {
        use super::super::native_ir::ArenaKind;

        assert_eq!(ArenaKind::Pending as u32, 0);
        assert_eq!(ArenaKind::Capture as u32, 1);
        assert_eq!(ArenaKind::Cursor as u32, 2);
        assert_eq!(ArenaKind::Repeat as u32, 3);
        assert_eq!(ArenaKind::QueryBucket as u32, 4);
        assert_eq!(ArenaKind::QueryMember as u32, 5);
        assert_eq!(ArenaKind::QueuedState as u32, 6);
        assert_eq!(ArenaKind::GreedyEntry as u32, 7);
        assert_eq!(ArenaKind::ValueState as u32, 8);
        assert_eq!(ArenaKind::Candidate as u32, 9);
        assert_eq!(ArenaKind::Result as u32, 10);
        assert_eq!(ArenaKind::FinalCapture as u32, 11);
        assert_eq!(ArenaKind::Property as u32, 12);
        assert_eq!(ArenaKind::OutputByte as u32, 13);
        assert_eq!(ARENA_KIND_COUNT, 14);
    }
}
