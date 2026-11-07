//! Phase 9 row "No-copy buffers" (05), at its planned path. G's `metal_backend.rs`
//! (`MappedRegion`, `MappedBuffer`, `new_buffer_with_bytes_no_copy`, the pipeline cache) was placed
//! whole with the packed runtime it serves, as `transducer::packed::metal_backend`. This module
//! names it here; the slab residency built on it is Phase 9's write-new row (provenance, O19).

#[allow(unused_imports)]
pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::metal_backend::MappedBuffer;
