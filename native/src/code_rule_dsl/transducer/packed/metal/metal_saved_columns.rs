//! Lossless physical storage for completed GPU output. Every scalar stays literal.
use super::{Error, OutputArenas};
use crate::policy_dsl::transducer::metal_backend::launch::{commit_checked, wait_checked};
use crate::policy_dsl::transducer::metal_backend::{MappedBuffer, pipeline_named};
use crate::policy_dsl::transducer::metal_parallel::{Scan, encode};
use crate::policy_dsl::transducer::metal_register_execution::read;
use metal::{CommandQueueRef, Device};

const SOURCE: &str = include_str!("metal_saved_columns.metal");
const MINIMUM_RAW_BYTES: usize = 32 * 1024 * 1024;

#[cfg(test)]
#[path = "metal_saved_columns_tests.rs"]
mod tests;

pub(super) enum SavedArenas {
    Literal(OutputArenas),
    Semantic(crate::policy_dsl::transducer::metal_semantic_columns::SemanticColumns),
    Encoded {
        headers: MappedBuffer,
        payload: MappedBuffer,
        bytes: MappedBuffer,
        counts: [u32; 4],
        typed: bool,
        words: u32,
        groups: u32,
    },
}

impl SavedArenas {
    pub(super) fn counts(&self) -> [u32; 4] {
        match self {
            Self::Literal(arenas) => arenas.counts,
            Self::Semantic(arenas) => arenas.counts,
            Self::Encoded { counts, .. } => *counts,
        }
    }
    pub(super) fn typed(&self) -> bool {
        match self {
            Self::Literal(arenas) => arenas.layout.is_some(),
            Self::Semantic(_) => true,
            Self::Encoded { typed, .. } => *typed,
        }
    }
    pub(super) fn encoding(&self) -> [u32; 3] {
        match self {
            Self::Literal(_) => [0; 3],
            Self::Semantic(a) => [2, a.words, a.groups],
            Self::Encoded { words, groups, .. } => [1, *words, *groups],
        }
    }
    /// Bindings follow the selected schema: rows, captures, properties, bytes,
    /// layout, and headers. Literal schemas never dereference the header binding.
    pub(super) fn buffers(&self) -> [&MappedBuffer; 6] {
        match self {
            Self::Semantic(a) => [
                &a.payload,
                &a.payload,
                &a.payload,
                &a.complex.bytes,
                &a.payload,
                &a.headers,
            ],
            Self::Literal(a) => [
                &a.rows,
                &a.captures,
                &a.properties,
                &a.bytes,
                a.layout.as_ref().unwrap_or(&a.rows),
                &a.rows,
            ],
            Self::Encoded {
                headers,
                payload,
                bytes,
                ..
            } => [payload, payload, payload, bytes, payload, headers],
        }
    }
    pub(super) fn semantic_metadata(&self) -> [u32; 4] {
        match self {
            Self::Semantic(a) => [
                a.spans,
                a.complex.counts[0],
                a.complex.counts[1],
                a.complex.counts[2],
            ],
            _ => [0; 4],
        }
    }
    /// Nonsemantic formats bind existing owners in slots their shaders never read.
    pub(super) fn semantic_buffers(&self) -> [&MappedBuffer; 6] {
        match self {
            Self::Semantic(a) => [
                &a.prefixes,
                &a.segments,
                &a.complex.rows,
                &a.complex.captures,
                &a.complex.properties,
                a.complex.layout.as_ref().unwrap_or(&a.complex.rows),
            ],
            _ => {
                let rows = self.buffers()[0];
                [rows; 6]
            }
        }
    }
}

pub(super) fn save(
    device: &Device,
    queue: &CommandQueueRef,
    arenas: OutputArenas,
) -> Result<SavedArenas, Error> {
    arenas.validate()?;
    let counts = [
        arenas.counts[0],
        arenas.counts[1],
        arenas.counts[2],
        if arenas.layout.is_some() {
            arenas.counts[0]
        } else {
            0
        },
    ];
    let widths = [10_usize, 9, 8, 4];
    let mut raw_bytes = 0_usize;
    let mut groups = 0_usize;
    for (count, width) in counts.into_iter().zip(widths) {
        raw_bytes = raw_bytes
            .checked_add(
                (count as usize)
                    .checked_mul(width * 4)
                    .ok_or(Error::BackendCapacity)?,
            )
            .ok_or(Error::BackendCapacity)?;
        groups = groups
            .checked_add(
                (count as usize)
                    .div_ceil(256)
                    .checked_mul(width)
                    .ok_or(Error::BackendCapacity)?,
            )
            .ok_or(Error::BackendCapacity)?;
    }
    // This is a closed physical format choice before work. A malformed encoded
    // producer always rejects; it never switches back to the literal format.
    if raw_bytes < MINIMUM_RAW_BYTES {
        return Ok(SavedArenas::Literal(arenas));
    }
    let groups = u32::try_from(groups).map_err(|_| Error::BackendCapacity)?;
    let mut parameters = counts.to_vec();
    parameters.extend([groups, 0]);
    let minima = MappedBuffer::zeroed::<[u32; 2]>(device, groups as usize)?;
    let scan = Scan::new(device, groups as usize)?;
    let status = MappedBuffer::zeroed::<u32>(device, 1)?;
    let layout = arenas.layout.as_ref().unwrap_or(&arenas.rows);
    metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
        let command = queue.new_command_buffer();
        encode(
            command,
            &pipeline_named(SOURCE, "saved_column_bounds")?,
            "saved_column_bounds",
            &[
                &arenas.rows.buffer,
                &arenas.captures.buffer,
                &arenas.properties.buffer,
                &layout.buffer,
                &minima.buffer,
                &scan.values.buffer,
            ],
            &parameters,
            groups as usize,
            None,
        );
        scan.encode(command)?;
        commit_checked(command)?;
        wait_checked(command)
    })?;
    let total = unsafe { scan.total().contents().cast::<[u32; 4]>().read() };
    if total.contains(&u32::MAX) || total[0] > 500_000_000 {
        return Err(Error::BackendCapacity);
    }
    if total[1..] != [0, 0, 0] || !total[0].is_multiple_of(8) {
        return Err(Error::BackendFailure);
    }
    parameters[5] = total[0];
    let headers = MappedBuffer::zeroed::<[u32; 2]>(device, groups as usize)?;
    let words = MappedBuffer::zeroed::<u32>(device, total[0] as usize)?;
    metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
        let command = queue.new_command_buffer();
        encode(
            command,
            &pipeline_named(SOURCE, "saved_column_pack")?,
            "saved_column_pack",
            &[
                &arenas.rows.buffer,
                &arenas.captures.buffer,
                &arenas.properties.buffer,
                &layout.buffer,
                &minima.buffer,
                &scan.values.buffer,
                &headers.buffer,
                &words.buffer,
                &status.buffer,
            ],
            &parameters,
            (groups.max(total[0]) as usize).div_ceil(256),
            None,
        );
        commit_checked(command)?;
        wait_checked(command)
    })?;
    if read::<u32>(&status, 1)? != [0] {
        return Err(Error::BackendFailure);
    }
    // Only this tagged owner contains encoded records. Raw OutputArenas keeps
    // its original invariant and cannot accidentally expose encoded words.
    let typed = arenas.layout.is_some();
    Ok(SavedArenas::Encoded {
        headers,
        payload: words,
        bytes: arenas.bytes,
        counts: arenas.counts,
        typed,
        words: total[0],
        groups,
    })
}
