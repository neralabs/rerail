//! Immutable scheduling facts derived from the shared Program proofs.
use super::metal_grid_plan::ResidentInput;
use super::{Error, Program};
use std::sync::Arc;

#[path = "metal_program_tables.rs"]
mod program_tables;

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProgramProof {
    entries: Vec<[u32; 2]>,
}

fn after_component(rank: usize) -> Result<u32, Error> {
    u32::try_from(rank)
        .ok()
        .and_then(|rank| rank.checked_mul(2))
        .and_then(|rank| rank.checked_add(1))
        .filter(|&rank| rank != u32::MAX)
        .ok_or(Error::BackendCapacity)
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReadyProof {
    programs: Vec<ProgramProof>,
    tables: Arc<program_tables::ProgramTables>,
    pub closure: super::metal_closure_prepare::ClosureCode,
    repetitions: std::collections::BTreeMap<u32, super::terminal_repeat_proof::TerminalRepeat>,
}
impl ReadyProof {
    pub(super) fn repetitions(
        &self,
    ) -> &std::collections::BTreeMap<u32, super::terminal_repeat_proof::TerminalRepeat> {
        &self.repetitions
    }
    pub(super) fn new(programs: &[Program]) -> Result<Self, Error> {
        let mut proved = Vec::new();
        for program in programs {
            program.validate()?;
            let mut entries = vec![[u32::MAX, 0]; program.words.len()];
            if program.tree_sitter_query {
                if program
                    .words
                    .iter()
                    .any(|&word| super::split(word).0 == super::REPEAT_BEGIN)
                {
                    for (join, rank) in super::greedy_contract::acyclic_joins(program) {
                        entries[join] = [after_component(rank)?, 2];
                    }
                }
                for (header, rank) in super::greedy_contract::ordered_headers(&program.words)? {
                    entries[header][0] = after_component(rank)?;
                }
                for region in super::closure_proof::regions(program)? {
                    entries[region.header][1] = 1;
                    // Keep the initial BUMP live until all earlier free work
                    // reaches this join. Its exact key is admitted before its
                    // instruction budget or repeat operand is checked. All
                    // nodes rendezvous before any bulk header can retire.
                    entries[region.header - 1] = [entries[region.header][0] - 1, 2];
                }
            }
            proved.push(ProgramProof { entries });
        }
        Ok(Self {
            programs: proved,
            tables: Arc::new(program_tables::ProgramTables::new(programs)?),
            closure: super::metal_closure_prepare::compile(programs)?,
            repetitions: programs
                .iter()
                .enumerate()
                .filter_map(|(index, program)| {
                    super::terminal_repeat_proof::prove(program).map(|proof| (index as u32, proof))
                })
                .collect(),
        })
    }
    pub(super) fn validate(&self, resident: &ResidentInput) -> Result<(), Error> {
        self.tables.validate(&resident.input.packed)
    }

    pub(super) fn entries(&self, resident: &ResidentInput) -> Result<Vec<[u32; 2]>, Error> {
        self.validate(resident)?;
        let mut entries = vec![[u32::MAX, 0]; resident.input.packed.words.len()];
        for (proof, program) in self.programs.iter().zip(&resident.input.packed.programs) {
            let first = program.structural_start as usize;
            entries[first..first + proof.entries.len()].copy_from_slice(&proof.entries);
        }
        Ok(entries)
    }
}
