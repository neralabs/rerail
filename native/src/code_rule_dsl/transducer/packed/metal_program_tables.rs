//! Exact immutable Program meaning, independent of files, tasks, and mutable arenas.
use super::super::metal_batch_abi::{
    LuaInstructionRecord, LuaSetMemberRecord, MetalProgram, MetalString, NfaRangeRecord,
    NfaStateRecord, ValueRecord,
};
use super::super::packed_batch::{AssertionRecord, PackedBatch};
use super::super::{Error, Program};

#[derive(Debug, Eq, PartialEq)]
pub(super) struct ProgramTables {
    programs: Vec<MetalProgram>,
    words: Vec<u32>,
    longs: Vec<u64>,
    strings: Vec<MetalString>,
    string_bytes: Vec<u8>,
    labels: Vec<u16>,
    values: Vec<ValueRecord>,
    nfa_states: Vec<NfaStateRecord>,
    nfa_data: Vec<u32>,
    nfa_ranges: Vec<NfaRangeRecord>,
    lua_instructions: Vec<LuaInstructionRecord>,
    lua_members: Vec<LuaSetMemberRecord>,
    assertions: Vec<AssertionRecord>,
    assertion_data: Vec<u32>,
}
impl ProgramTables {
    pub(super) fn new(programs: &[Program]) -> Result<Self, Error> {
        // Use the production packer without any execution inputs. This produces
        // canonical table addresses as well as exact table contents.
        let batch = PackedBatch::new_grid_inputs(programs, &[], &[])?;
        Ok(Self {
            programs: batch.programs,
            words: batch.words,
            longs: batch.longs,
            strings: batch.strings,
            string_bytes: batch.string_bytes,
            labels: batch.labels,
            values: batch.values,
            nfa_states: batch.nfa_states,
            nfa_data: batch.nfa_data,
            nfa_ranges: batch.nfa_ranges,
            lua_instructions: batch.lua_instructions,
            lua_members: batch.lua_members,
            assertions: batch.assertions,
            assertion_data: batch.assertion_data,
        })
    }
    pub(super) fn validate(&self, batch: &PackedBatch) -> Result<(), Error> {
        if self.programs != batch.programs
            || self.words != batch.words
            || self.longs != batch.longs
            || self.strings != batch.strings
            || self.string_bytes != batch.string_bytes
            || self.labels != batch.labels
            || self.values != batch.values
            || self.nfa_states != batch.nfa_states
            || self.nfa_data != batch.nfa_data
            || self.nfa_ranges != batch.nfa_ranges
            || self.lua_instructions != batch.lua_instructions
            || self.lua_members != batch.lua_members
            || self.assertions != batch.assertions
            || self.assertion_data != batch.assertion_data
        {
            return Err(Error::InvalidAnchor);
        }
        Ok(())
    }
}
