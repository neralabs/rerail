//! Typed parameters and one guarded dispatch boundary for output operators.
use super::Error;
use super::metal_backend::{MappedBuffer, pipeline_named};
use metal::{BufferRef, CommandBufferRef};

pub(super) const SOURCE: &str = concat!(
    include_str!("metal_resident.metal"),
    include_str!("metal_typed_output.metal"),
    include_str!("metal_output_selection.metal"),
    include_str!("metal_output_mime.metal"),
    include_str!("metal_capture_arguments.metal"),
    include_str!("metal_lua_types.metal"),
    include_str!("metal_regex_types.metal"),
    "\n#define REGEX_ADDRESS device\n",
    include_str!("metal_regex_nfa.metal"),
    "\n#undef REGEX_ADDRESS\n#define REGEX_ADDRESS threadgroup\n",
    include_str!("metal_regex_nfa.metal"),
    "\n#undef REGEX_ADDRESS\n",
    include_str!("metal_regex_frontier.metal"),
    include_str!("metal_lua_frontier.metal"),
    include_str!("metal_value_preflight.metal"),
    include_str!("metal_capture_predicates.metal"),
    include_str!("metal_capture_groups.metal"),
    include_str!("metal_capture_literals.metal"),
    include_str!("metal_output_strip.metal")
);

#[derive(Clone, Copy)]
pub(super) struct OutputParams {
    pub results: u32,
    pub captures: u32,
    pub step: u32,
    pub max_operators: u32,
    pub merge_width: u32,
    pub source_results: u32,
    pub source_captures: u32,
    pub result_base: u32,
    pub capture_base: u32,
    pub dynamic_slots: u32,
    pub transform_bytes: u32,
}
impl OutputParams {
    fn words(self) -> [u32; 11] {
        [
            self.results,
            self.captures,
            self.step,
            self.max_operators,
            self.merge_width,
            self.source_results,
            self.source_captures,
            self.result_base,
            self.capture_base,
            self.dynamic_slots,
            self.transform_bytes,
        ]
    }
}

pub(super) fn direct(
    command: &CommandBufferRef,
    name: &str,
    buffers: &[&BufferRef],
    p: &OutputParams,
    count: usize,
) -> Result<(), Error> {
    super::metal_parallel::encode(
        command,
        &pipeline_named(SOURCE, name)?,
        name,
        buffers,
        &p.words(),
        count.div_ceil(256).max(1),
        None,
    );
    Ok(())
}

pub(super) struct OperatorInput<'a> {
    pub data: &'a MappedBuffer,
    pub rows: &'a MappedBuffer,
    pub captures: &'a MappedBuffer,
    pub metadata: &'a MappedBuffer,
    pub owners: &'a MappedBuffer,
    pub status: &'a MappedBuffer,
    pub parameters: OutputParams,
}
impl OperatorInput<'_> {
    pub(super) fn run(
        &self,
        command: &CommandBufferRef,
        name: &str,
        buffers: &[&BufferRef],
        count: usize,
    ) -> Result<(), Error> {
        direct(command, name, buffers, &self.parameters, count)
    }
    pub(super) fn indirect(
        &self,
        command: &CommandBufferRef,
        name: &str,
        buffers: &[&BufferRef],
        arguments: &MappedBuffer,
    ) -> Result<(), Error> {
        super::metal_parallel::encode(
            command,
            &pipeline_named(SOURCE, name)?,
            name,
            buffers,
            &self.parameters.words(),
            0,
            Some((&arguments.buffer, 0)),
        );
        Ok(())
    }
}
