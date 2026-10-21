//! Resident memory accounting: retained-residency profiling and the memory profile estimate.

pub(super) mod census;

use super::global::Partition;
use super::results::ResultPageIndex;
use super::{Candidates, ClauseId, FileState, WorkspaceState};
use crate::code_rule_dsl::violations::Witness;
use serde_json::{Value, json};
/// The length of `value` serialized, standing in for its heap bytes.
fn json_len(value: &impl serde::Serialize) -> usize {
    serde_json::to_string(value).map_or(0, |text| text.len())
}

/// Estimated heap bytes of one witness's payload.
pub(super) fn witness_payload_bytes(witness: &Witness) -> usize {
    witness.path.len()
        + json_len(&witness.span)
        + json_len(&witness.captures)
        + witness.message.as_ref().map_or(0, String::len)
        + witness.fix.as_ref().map_or(0, |fix| fix.to_string().len())
}

pub(super) fn workspace_memory_profile(state: &WorkspaceState) -> Value {
    use std::mem::size_of;
    let file_state_bytes = state
        .files
        .iter()
        .map(|(path, file)| {
            size_of::<String>()
                + path.capacity()
                + size_of::<FileState>()
                + file.input.path.capacity()
                + file.input.language.capacity()
                + file.input.source.capacity()
                + file.witnesses.len() * size_of::<(ClauseId, Vec<Witness>)>()
                + file
                    .witnesses
                    .values()
                    .map(|witnesses| witnesses.capacity() * size_of::<Witness>())
                    .sum::<usize>()
                + file.failures.len() * size_of::<(ClauseId, (&str, &str))>()
                + file
                    .witnesses
                    .rendered
                    .get()
                    .map_or(0, super::results::Rendered::bytes)
        })
        .sum::<usize>();
    let witness_payload_bytes = state
        .files
        .values()
        .flat_map(|file| file.witnesses.values().flatten())
        .map(witness_payload_bytes)
        .sum::<usize>();
    let candidate_dispatch_bytes = state
        .candidate_sets
        .values()
        .map(|set| {
            size_of::<Candidates>()
                + (set.regex.capacity() + set.ast.capacity()) * size_of::<ClauseId>()
                + set.by_kind.capacity() * size_of::<Vec<usize>>()
                + set
                    .by_kind
                    .iter()
                    .map(|entries| entries.capacity() * size_of::<usize>())
                    .sum::<usize>()
                + set.unrestricted.capacity() * size_of::<usize>()
        })
        .sum::<usize>();
    let leaf_path_bytes = state
        .evidence
        .values()
        .map(|partitions| {
            partitions
                .iter()
                .map(|(path, partition)| {
                    size_of::<String>()
                        + path.capacity()
                        + size_of::<Partition>()
                        + partition
                            .witnesses
                            .iter()
                            .map(|witness| {
                                size_of::<Witness>()
                                    + witness.path.capacity()
                                    + json_len(&witness.span)
                                    + json_len(&witness.captures)
                                    + witness.message.as_ref().map_or(0, String::capacity)
                            })
                            .sum::<usize>()
                })
                .sum::<usize>()
        })
        .sum::<usize>();
    let witness_count = state
        .files
        .values()
        .map(|file| file.witnesses.values().map(Vec::len).sum::<usize>())
        .sum::<usize>();
    let result_page_index_bytes = state
        .result_page_index
        .as_ref()
        .map_or(0, ResultPageIndex::estimated_bytes);
    let mut ownership = census::WitnessMemory::default();
    let mut producers = Vec::new();
    for file in state.files.values() {
        for witness in file.witnesses.values().flatten() {
            ownership.visit(witness);
        }
    }
    for (id, partitions) in &state.evidence {
        let mut rows = 0usize;
        for partition in partitions.values() {
            rows += partition.witnesses.len();
            for witness in &partition.witnesses {
                ownership.visit(witness);
            }
        }
        producers.push(json!({"producer": format!("{id:?}"), "rows": rows}));
    }
    producers.sort_by_key(|row| std::cmp::Reverse(row["rows"].as_u64().unwrap_or(0)));
    json!({
        "ownership": ownership,
        "producers": producers,
        "accounting": "Explicit census; Arc projections counted once across files and evidence. Heap estimates exclude allocator overhead; legacy estimates can double-count shared payloads.",
        "files": state.files.len(),
        "residentFileStates": state.files.len(),
        "witnesses": witness_count,
        "fileStateBytesEstimate": file_state_bytes,
        "witnessPayloadBytesEstimate": witness_payload_bytes,
        "candidateDispatchBytesEstimate": candidate_dispatch_bytes,
        "leafPathBytesEstimate": leaf_path_bytes,
        "resultPageIndexBytesEstimate": result_page_index_bytes,
        "residentDocumentBytes": state.documents.resident_bytes(),
        "evictedFileVersions": state.versions.len(),
        "estimatedBytes": file_state_bytes + witness_payload_bytes
            + candidate_dispatch_bytes + leaf_path_bytes + result_page_index_bytes
            + state.documents.resident_bytes(),
    })
}
