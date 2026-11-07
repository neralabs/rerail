//! Evaluation outcomes selected by logical position on the GPU.
use super::Error;
use super::metal_backend::MappedBuffer;

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct EvaluationFailureRecord {
    /// Zero is successful completion; one is an operand failure; two is exhausted ordinary path work.
    kind: u32,
    program_counter: u32,
}

#[must_use = "Propagate evaluation failures before result publication"]
pub(super) struct TerminalCompletion {
    pub errors: Vec<Option<Error>>,
}

pub(super) fn decode(
    records: &MappedBuffer,
    program_lengths: &[u32],
) -> Result<TerminalCompletion, Error> {
    let rows = super::metal_register_execution::read::<EvaluationFailureRecord>(
        records,
        program_lengths.len(),
    )?;
    let errors = rows
        .into_iter()
        .zip(program_lengths)
        .map(|(row, &length)| match (row.kind, row.program_counter) {
            (0, 0) => Ok(None),
            (1, pc) if pc < length => Ok(Some(Error::InvalidOperand(pc as usize))),
            (2, pc) if pc < length => Ok(Some(Error::WorkLimit)),
            _ => Err(Error::BackendFailure),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(TerminalCompletion { errors })
}
