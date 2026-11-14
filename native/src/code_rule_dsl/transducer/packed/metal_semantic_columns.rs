//! Final literal columns indexed by complete semantic spans, with checked arena conversion.
use super::Error;
use super::metal_backend::MappedBuffer;
use super::metal_grid_plan::ResidentInput;
use super::metal_output_pages::OutputArenas;

pub(super) struct SemanticColumns {
    pub prefixes: MappedBuffer,
    /// Three uint4 records per span: source/header/counts, counts/reserved, source bases.
    pub segments: MappedBuffer,
    pub headers: MappedBuffer,
    pub payload: MappedBuffer,
    pub complex: OutputArenas,
    pub counts: [u32; 4],
    pub words: u32,
    pub groups: u32,
    pub spans: u32,
    pub owner: [usize; 2],
}

impl SemanticColumns {
    pub(super) fn validate(
        &self,
        resident: &ResidentInput,
        data: &MappedBuffer,
    ) -> Result<(), Error> {
        if self.owner
            != [
                resident as *const _ as usize,
                (&*data.buffer as *const metal::BufferRef) as usize,
            ]
            || self.spans as usize != resident.input.grid.semantic_span_order.len()
            || self.words > 500_000_000
            || !self.words.is_multiple_of(8)
            || (self.complex.counts[0] != 0 && self.complex.layout.is_none())
            || (self.counts[0] != 0 && self.spans == 0)
            || (self.counts[0] == 0 && self.counts != [0; 4])
            || self
                .counts
                .iter()
                .zip(self.complex.counts)
                .any(|(total, complex)| *total < complex)
            || self.counts[3] != self.complex.counts[3]
        {
            return Err(Error::BackendFailure);
        }
        for (buffer, count, stride) in [
            (&self.prefixes, self.spans, 16_u64),
            (&self.segments, self.spans, 48),
            (&self.headers, self.groups, 8),
            (&self.payload, self.words, 4),
        ] {
            if count as u64 * stride > buffer.buffer.length() {
                return Err(Error::BackendFailure);
            }
        }
        self.complex.validate()
    }
}
