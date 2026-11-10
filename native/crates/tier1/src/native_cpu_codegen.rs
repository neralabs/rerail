use cranelift_codegen::Context;
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Module, ModuleError};

pub fn define_function(
    module: &mut JITModule,
    id: FuncId,
    context: &mut Context,
) -> Result<(), Box<ModuleError>> {
    // JIT code mappings can be farther apart than a direct branch can reach.
    // Local linkage proves symbol ownership, not physical code proximity.
    // Ask Cranelift for long-range calls before it selects instructions.
    for (_, function) in context.func.dfg.ext_funcs.iter_mut() {
        function.colocated = false;
    }
    module.define_function(id, context)?;
    #[cfg(all(any(test, feature = "test-hooks"), target_os = "macos", target_arch = "aarch64"))]
    relocations::record(id, context);
    Ok(())
}

/// The relocations' checks: every call of a generated function reaches its callee however far
/// apart the code is mapped. Its hooks are the engine's tests' too (feature `test-hooks`).
#[cfg(all(any(test, feature = "test-hooks"), target_os = "macos", target_arch = "aarch64"))]
#[path = "native_cpu_relocation_tests.rs"]
pub mod relocations;
