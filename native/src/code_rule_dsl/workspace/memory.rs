//! Resident memory accounting: retained-residency profiling and the memory profile estimate.

use super::global::Partition;
use super::results::ResultPageIndex;
use super::{Candidates, ClauseId, FileState, WorkspaceState};
use crate::code_rule_dsl::violations::Witness;
#[cfg(feature = "code-rule-profile")]
use crate::code_rule_dsl::{profile, tape};
use serde_json::{Value, json};
#[cfg(feature = "code-rule-profile")]
use std::sync::Arc;

#[cfg(feature = "code-rule-profile")]
pub(super) fn record_workspace_residency(state: &WorkspaceState) {
    use std::mem::size_of;
    let source = state
        .files
        .values()
        .map(|file| {
            size_of::<Arc<String>>()
                + size_of::<String>()
                + file.input.source.capacity()
                + file.input.path.capacity()
                + file.input.language.capacity()
        })
        .sum();
    let tape = state
        .files
        .values()
        .filter_map(|file| file.tape.as_ref())
        .map(|(_, image)| image.estimated_bytes())
        .sum();
    // Tree-sitter grammar tables are static library storage, not workspace-owned.
    let grammar = state
        .grammars
        .keys()
        .map(|name| name.capacity() + size_of::<Arc<tape::Grammar>>() + size_of::<tape::Grammar>())
        .sum();
    let dispatch = state
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
        .sum();
    let results = state
        .files
        .values()
        .map(|file| {
            file.witnesses
                .values()
                .flatten()
                .map(|witness| {
                    witness.path.len()
                        + json_len(&witness.span)
                        + json_len(&witness.captures)
                        + witness.message.as_ref().map_or(0, String::len)
                        + witness.fix.as_ref().map_or(0, |fix| fix.to_string().len())
                        + size_of::<Witness>()
                })
                .sum::<usize>()
                + file.failures.len() * size_of::<(ClauseId, (&str, &str))>()
        })
        .sum::<usize>()
        + state
            .global_results
            .values()
            .map(|result| {
                result
                    .violations
                    .iter()
                    .chain(&result.failures)
                    .map(|value| value.to_string().len())
                    .sum::<usize>()
            })
            .sum::<usize>();
    profile::record_retained(source, tape, grammar, dispatch, results);
}

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
    json!({
        "executionEngine": state.execution_engine.as_str(),
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
