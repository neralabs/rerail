// Tier 1's source from a compiled grammar (`rust_block`): the build script's and `aot`'s, one
// expression, so the two write the same source. `c` is the `rerail_kotlin_parser::Compiled`.
{
    let program = &c.program.block_programs[0];
    let native = rerail_compiler::block_native::BlockNative::lower(program);
    let inputs = rerail_rust_stream::rust_block::Inputs {
        program,
        layout: c.parser.tier1.records.as_ref().map(|t| t.layout).unwrap_or_default(),
        lazy: rerail_cpu_block::native_cpu_block::lazy_code_bits(&native),
        token_names: &c.program.names.tokens,
        mark_names: &c.program.names.marks,
        unmade: native.records_made(),
    };
    rerail_rust_stream::rust_block::rust_source(&inputs)
}
