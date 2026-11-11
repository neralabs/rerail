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

/// Functions compiled on threads, then defined in `module` in order: Cranelift compiles each
/// function alone (tier 2's clusters, decision in doc 28), so they compile at once, a thread a
/// share of them, up to the machine's threads. Returns the contexts, each holding its compiled
/// code. Each thread's pass times are added to `parallel_pass_times`.
pub fn define_functions(module: &mut JITModule, items: Vec<(FuncId, Context)>) -> Result<Vec<Context>, Box<ModuleError>> {
    use cranelift_module::{Module, ModuleReloc};
    let mut items = items;
    for (_, context) in &mut items {
        for (_, function) in context.func.dfg.ext_funcs.iter_mut() {
            function.colocated = false;
        }
    }
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(items.len()).max(1);
    let isa = module.isa();
    // The largest first, dealt round the threads.
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(items[i].1.func.dfg.num_insts()));
    let mut shares: Vec<Vec<(usize, FuncId, Context)>> = (0..threads).map(|_| Vec::new()).collect();
    let mut slots: Vec<Option<(FuncId, Context)>> = items.into_iter().map(Some).collect();
    for (k, &i) in order.iter().enumerate() {
        let (id, c) = slots[i].take().expect("each once");
        shares[k % threads].push((i, id, c));
    }
    let done: Vec<Result<Vec<(usize, FuncId, Context)>, String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = shares
            .into_iter()
            .map(|share| {
                scope.spawn(move || {
                    let mut out = Vec::with_capacity(share.len());
                    for (i, id, mut c) in share {
                        c.compile(isa, &mut Default::default()).map_err(|e| format!("{:?}", e.inner))?;
                        out.push((i, id, c));
                    }
                    if let Ok(mut t) = PARALLEL_TIMES.lock() {
                        t.add(&cranelift_codegen::timing::take_current());
                    }
                    Ok(out)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("a compile thread panicked".to_owned()))).collect()
    });
    let mut compiled: Vec<Option<(FuncId, Context)>> = (0..order.len()).map(|_| None).collect();
    for share in done {
        for (i, id, c) in share.map_err(|e| Box::new(anyhow_free(e)))? {
            compiled[i] = Some((id, c));
        }
    }
    let mut out = Vec::with_capacity(compiled.len());
    for (id, c) in compiled.into_iter().flatten() {
        let code = c.compiled_code().expect("compiled");
        let relocs: Vec<ModuleReloc> = code.buffer.relocs().iter().map(|r| ModuleReloc::from_mach_reloc(r, &c.func, id)).collect();
        module.define_function_bytes(id, code.buffer.alignment as u64, code.code_buffer(), &relocs)?;
        #[cfg(all(any(test, feature = "test-hooks"), target_os = "macos", target_arch = "aarch64"))]
        relocations::record(id, &c);
        out.push(c);
    }
    Ok(out)
}

/// A compile's error as a module's.
fn anyhow_free(e: String) -> cranelift_module::ModuleError {
    cranelift_module::ModuleError::Compilation(cranelift_codegen::CodegenError::Unsupported(e))
}

static PARALLEL_TIMES: std::sync::LazyLock<std::sync::Mutex<cranelift_codegen::timing::PassTimes>> = std::sync::LazyLock::new(Default::default);

/// The pass times of `define_functions`' threads since the last call, added.
pub fn parallel_pass_times() -> cranelift_codegen::timing::PassTimes {
    PARALLEL_TIMES.lock().map(|mut t| std::mem::take(&mut *t)).unwrap_or_default()
}

/// The relocations' checks: every call of a generated function reaches its callee however far
/// apart the code is mapped. Its hooks are the engine's tests' too (feature `test-hooks`).
#[cfg(all(any(test, feature = "test-hooks"), target_os = "macos", target_arch = "aarch64"))]
#[path = "native_cpu_relocation_tests.rs"]
pub mod relocations;
