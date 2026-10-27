//! Stable tile sorting for complete terminal publication keys.
use super::super::Error;
use super::super::metal_backend::pipeline_named;
use super::super::metal_parallel::encode as dispatch;
use metal::{BufferRef, CommandBufferRef};

pub(super) fn encode_tiles(
    command: &CommandBufferRef,
    header: &BufferRef,
    metadata: &BufferRef,
    ids: [&BufferRef; 2],
    status: &BufferRef,
    capacity: usize,
) -> Result<usize, Error> {
    let mut owners = std::collections::BTreeSet::new();
    for (buffer, count, bytes) in [
        (header, 1, 4),
        (metadata, capacity, 16),
        (ids[0], capacity, 4),
        (ids[1], capacity, 4),
        (status, 1, 4),
    ] {
        if !owners.insert(buffer as *const BufferRef as usize)
            || count.checked_mul(bytes).ok_or(Error::BackendCapacity)? as u64 > buffer.length()
        {
            return Err(Error::BackendFailure);
        }
    }
    let capacity = u32::try_from(capacity).map_err(|_| Error::BackendCapacity)?;
    let pipeline = pipeline_named(
        include_str!("metal_terminal_publication_sort.metal"),
        "publication_sort_tiles",
    )?;
    dispatch(
        command,
        &pipeline,
        "publication_sort_tiles",
        &[header, metadata, ids[0], ids[1], status],
        &[capacity],
        (capacity as usize).div_ceil(256).max(1),
        None,
    );
    Ok(1)
}

#[cfg(test)]
#[path = "metal_terminal_publication_sort_tests.rs"]
mod tests;
