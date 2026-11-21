use cranelift_codegen::Context;
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Module, ModuleError};

/// The process's instructions retired and cycles so far (macOS: `proc_pid_rusage`, no root
/// needed), for benchmarks: their difference over a run on one thread is that run's counts.
/// Zero elsewhere.
pub fn retired() -> (u64, u64) {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: the call writes the structure, which is plain integers.
        let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
        let ok = unsafe { libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V4, (&raw mut info).cast()) };
        if ok == 0 {
            return (info.ri_instructions, info.ri_cycles);
        }
    }
    (0, 0)
}

/// What a caller asks of the compiles its run makes (decision 0442: the run's own, `options`).
pub struct Options {
    /// Cranelift's optimizer (`opt_level = speed`) for tier 1 and the rows' code, to measure it.
    /// Off by default: decision 0069 keeps tier 1's constants in its loop, and the mid-end's code
    /// motion hoists them. Tier 2 has its own switch (`native_cpu_clusters::Cut::speed`).
    pub optimize_tier1: std::sync::atomic::AtomicBool,
    /// The same for tier 2's default `compile` (one function).
    pub optimize_tier2: std::sync::atomic::AtomicBool,
    /// A class's lane is the bit test itself, not its complement (`native_cpu_lanes`).
    pub bit_test: std::sync::atomic::AtomicBool,
    /// Runs again on the bound's tape (`CompiledStream::call`), for the profile.
    pub room_retries: std::sync::atomic::AtomicU64,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            optimize_tier1: Default::default(),
            optimize_tier2: Default::default(),
            bit_test: std::sync::atomic::AtomicBool::new(crate::native_cpu_lanes::BIT_TEST_DEFAULT),
            room_retries: Default::default(),
        }
    }
}

/// The run's compile options.
pub fn options() -> std::sync::Arc<Options> {
    rerail_context::local::<Options>()
}

/// The JIT builder for tier 1 and the rows: the JIT's defaults, with `optimize_tier1`.
pub fn tier1_builder() -> Result<cranelift_jit::JITBuilder, Box<ModuleError>> {
    let flags: &[(&str, &str)] = if options().optimize_tier1.load(std::sync::atomic::Ordering::Relaxed) { &[("opt_level", "speed")] } else { &[] };
    cranelift_jit::JITBuilder::with_flags(flags, cranelift_module::default_libcall_names()).map_err(Box::new)
}

/// Each jump table (`br_table`) `switch_trees` leaves made a tree of compares, for Cranelift's code only: its Arm
/// backend emits every jump table with a speculation barrier (`csel`, then `csdb`, in
/// `JTSequence`, whatever the settings), and the jump waits for its index. In the C
/// preprocessing words' return dispatches that barrier was most of their cycles (work queue
/// Q184, Q186).
///
/// The index is tested against the table's length (the default past it), then a balanced
/// tree over the table's runs of one target finds the entry: a 256-entry table of few targets
/// is a few compares deep. The last step of `switch_trees`, which runs before Cranelift compiles a
/// function (`define_function`, `define_functions`, and the preprocessing words' own).
fn balanced_trees(func: &mut cranelift_codegen::ir::Function) {
    use cranelift_codegen::cursor::{Cursor, FuncCursor};
    use cranelift_codegen::ir::{BlockArg, InstBuilder, InstructionData, condcodes::IntCC};
    let branches: Vec<_> = func
        .layout
        .blocks()
        .filter_map(|b| func.layout.last_inst(b).map(|i| (b, i)))
        .filter(|&(_, i)| matches!(func.dfg.insts[i], InstructionData::BranchTable { .. }))
        .collect();
    for (block, inst) in branches {
        let InstructionData::BranchTable { arg, table, .. } = func.dfg.insts[inst] else { unreachable!() };
        let pool = &func.dfg.value_lists;
        let call = |c: &cranelift_codegen::ir::BlockCall| (c.block(pool), c.args(pool).collect::<Vec<BlockArg>>());
        let jt = &func.stencil.dfg.jump_tables[table];
        let default = call(&jt.default_block());
        // Runs of one target: (first index, target).
        let mut runs: Vec<(u64, (cranelift_codegen::ir::Block, Vec<BlockArg>))> = Vec::new();
        for (k, c) in jt.as_slice().iter().enumerate() {
            let c = call(c);
            if runs.last().is_none_or(|r| r.1 != c) {
                runs.push((k as u64, c));
            }
        }
        let len = jt.as_slice().len() as i64;
        let mut pos = FuncCursor::new(func).at_inst(inst);
        pos.remove_inst();
        let f = pos.func;
        let mut pos = FuncCursor::new(f).at_bottom(block);
        if runs.is_empty() {
            pos.ins().jump(default.0, &default.1);
            continue;
        }
        let within = pos.func.dfg.make_block();
        pos.func.layout.insert_block_after(within, block);
        let past = pos.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, arg, len);
        pos.ins().brif(past, default.0, &default.1, within, &[]);
        let mut work = vec![(within, 0usize, runs.len())];
        while let Some((b, lo, hi)) = work.pop() {
            let mut pos = FuncCursor::new(pos.func).at_bottom(b);
            if hi - lo == 1 {
                let (t, args) = &runs[lo].1;
                pos.ins().jump(*t, args);
                continue;
            }
            let mid = (lo + hi) / 2;
            let (left, right) = (pos.func.dfg.make_block(), pos.func.dfg.make_block());
            pos.func.layout.insert_block_after(right, b);
            pos.func.layout.insert_block_after(left, b);
            let below = pos.ins().icmp_imm(IntCC::UnsignedLessThan, arg, runs[mid].0 as i64);
            pos.ins().brif(below, left, &[], right, &[]);
            work.push((right, mid, hi));
            work.push((left, lo, mid));
        }
    }
}

pub fn define_function(
    module: &mut JITModule,
    id: FuncId,
    context: &mut Context,
) -> Result<(), Box<ModuleError>> {
    // The tape's hot reads it calls, inlined (decision 0310).
    rerail_clif::inline::reads(module, context)?;
    // JIT code mappings can be farther apart than a direct branch can reach.
    // Local linkage proves symbol ownership, not physical code proximity.
    // Ask Cranelift for long-range calls before it selects instructions.
    for (_, function) in context.func.dfg.ext_funcs.iter_mut() {
        function.colocated = false;
    }
    // Every table a tree, rustc's where 0322 holds, balanced for the rest: no jump table in
    // Cranelift's code (Q186).
    switch_trees(&mut context.func);
    // Each integer constant made again in the blocks that read it (decision 0351).
    remat_consts(&mut context.func);
    // No complement of a complement (0381).
    fold_nots(&mut context.func);
    // The function may have changed since its flow graph was made (`clif_fold`, `switch_trees`,
    // `remat_consts`): made again.
    context.cfg.clear();
    context.domtree.clear();
    if let Err(e) = module.define_function(id, context) {
        if std::env::var_os("RERAIL_DEBUG_STREAM").is_some() {
            eprintln!("define {}: {e:?}\n{}", context.func.name, context.func.display());
        }
        return Err(Box::new(e));
    }
    #[cfg(all(any(test, feature = "test-hooks"), target_os = "macos", target_arch = "aarch64"))]
    relocations::record(id, context);
    Ok(())
}

/// The tape's cold reads (decision 0310), defined once in `module` as every function is
/// (`define_function`): what a call the pass leaves calls. The closure hands `define_cold` Cranelift's own
/// `ModuleError`, as its signature asks.
#[allow(clippy::result_large_err)]
pub fn define_reads(module: &mut JITModule) -> Result<(), Box<ModuleError>> {
    rerail_clif::inline::define_cold(module, &mut |module, id, context| define_function(module, id, context).map_err(|error| *error))?;
    Ok(())
}

/// Functions compiled on threads, then defined in `module` in order: Cranelift compiles each
/// function alone (tier 2's clusters, decision in doc 28), so they compile at once, a thread a
/// share of them, up to the machine's threads. Returns the contexts, each holding its compiled
/// code. Each thread's pass times are added to `parallel_pass_times`.
///
/// `near`: the module's code is in one arena (`ArenaMemoryProvider`, within a direct branch's
/// reach), so a call of its own functions is a direct branch (`colocated`); else every call is
/// long-range, as `define_function`'s.
pub fn define_functions(module: &mut JITModule, items: Vec<(FuncId, Context)>, near: bool) -> Result<Vec<Context>, Box<ModuleError>> {
    use cranelift_module::{Module, ModuleReloc};
    let mut items = items;
    for (_, context) in &mut items {
        // The tape's hot reads each calls, inlined (decision 0310); then the flow graph made again, as the function
        // changed since it was made (`clif_fold`, and the inlining).
        rerail_clif::inline::reads(module, context)?;
        // Each switch a tree of compares, as rustc lowers a match (0322): after the inlining, so
        // an inlined read's switches are lowered too.
        switch_trees(&mut context.func);
        context.cfg.clear();
        context.domtree.clear();
        for (_, function) in context.func.dfg.ext_funcs.iter_mut() {
            function.colocated = near && matches!(function.name, cranelift_codegen::ir::ExternalName::User(_));
        }
    }
    // On the machine's pool (decisions 0100 §2, 0442), a share a thread of it.
    let threads = rerail_context::cores_threads().min(items.len()).max(1);
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
    // The run's pass times, taken here: the loop's items run on pool threads, which hold no run.
    let times = rerail_context::local::<ParallelTimes>();
    let done: Vec<Result<Vec<(usize, FuncId, Context)>, String>> = {
        use rayon::prelude::*;
        rerail_context::on_cores(|| {
            shares
                .into_par_iter()
                .map(|share| {
                    let mut out = Vec::with_capacity(share.len());
                    for (i, id, mut c) in share {
                        c.compile(isa, &mut Default::default()).map_err(|e| format!("{:?}", e.inner))?;
                        out.push((i, id, c));
                    }
                    if let Ok(mut t) = times.0.lock() {
                        t.add(&cranelift_codegen::timing::take_current());
                    }
                    Ok(out)
                })
                .collect()
        })
    };
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

/// The run's pass times of `define_functions`' threads (a measurement, decision 0442).
#[derive(Default)]
struct ParallelTimes(std::sync::Mutex<cranelift_codegen::timing::PassTimes>);

/// The pass times of `define_functions`' threads since the last call, added.
pub fn parallel_pass_times() -> cranelift_codegen::timing::PassTimes {
    rerail_context::local::<ParallelTimes>().0.lock().map(|mut t| std::mem::take(&mut *t)).unwrap_or_default()
}

/// The relocations' checks: every call of a generated function reaches its callee however far
/// apart the code is mapped. Its hooks are the engine's tests' too (feature `test-hooks`).
#[cfg(all(any(test, feature = "test-hooks"), target_os = "macos", target_arch = "aarch64"))]
#[path = "native_cpu_relocation_tests.rs"]
pub mod relocations;

/// The most runs of one target a `br_table` may have to be lowered as a tree (`switch_trees`):
/// five levels of compares at most.
pub const SWITCH_TREE_RUNS: usize = 32;

/// Each `br_table` of at most `SWITCH_TREE_RUNS` runs, lowered as rustc lowers a `match`
/// (LLVM's switch lowering, `native_cpu_stream::dispatch`): a tree of compares and bit tests
/// over the runs of one target. Cranelift's `br_table` on aarch64 is a bound check, then `csel`
/// and `csdb` before the indirect branch, always (a Spectre guard, no flag): the barrier keeps
/// the core from running ahead into the arm, so each token waits for its entry's load. Tier
/// 1's numbers took 75 cycles a token by the JIT against 17 by the AOT, most samples at the
/// table's load. The function is rewritten just before Cranelift lowers it.
/// A table whose targets take arguments, or of more runs, is a balanced tree of compares
/// (`balanced_trees`, Q186): no `br_table` reaches Cranelift's code.
/// Each integer constant a block reads but another made, made again in the block, before its
/// first reader there (decision 0351): the optimizer's `remat` of an `iconst`, which tier 1 runs
/// without (0069). A constant the token loop's entry makes (a table's address, the word hash's
/// multiplier, zero, all ones) was one value live through the loop, its calls included: so it was
/// spilled and reloaded at each token, where the lean lexer has it in an immediate or a `movz`
/// beside its reader. Only the JIT's copy: the AOT's rustc does the same.
/// A complement read complemented, folded (0381): `band_not`, `bor_not` and `bxor_not` of a
/// `bnot y` are `band`, `bor` and `bxor` of `y`. The body keeps a class complemented while its
/// readers can take it so (`Sym`); where one could not, it made the `bnot`, and a later reader
/// that wanted the complement complemented it again: without the optimizer (0069) nothing folds
/// that, and the `bnot` was one more value live. Only the JIT's copy; rustc folds the AOT's.
pub fn fold_nots(func: &mut cranelift_codegen::ir::Function) {
    use cranelift_codegen::ir::{InstructionData, Opcode, ValueDef};
    let not_of = |func: &cranelift_codegen::ir::Function, v: cranelift_codegen::ir::Value| -> Option<cranelift_codegen::ir::Value> {
        let v = func.dfg.resolve_aliases(v);
        let ValueDef::Result(def, 0) = func.dfg.value_def(v) else { return None };
        match func.dfg.insts[def] {
            InstructionData::Unary { opcode: Opcode::Bnot, arg } => Some(arg),
            _ => None,
        }
    };
    let blocks: Vec<cranelift_codegen::ir::Block> = func.layout.blocks().collect();
    for block in blocks {
        let insts: Vec<cranelift_codegen::ir::Inst> = func.layout.block_insts(block).collect();
        for inst in insts {
            let InstructionData::Binary { opcode, args: [a, b] } = func.dfg.insts[inst] else { continue };
            let plain = match opcode {
                Opcode::BandNot => Opcode::Band,
                Opcode::BorNot => Opcode::Bor,
                Opcode::BxorNot => Opcode::Bxor,
                _ => continue,
            };
            if let Some(y) = not_of(func, b) {
                func.dfg.insts[inst] = InstructionData::Binary { opcode: plain, args: [a, y] };
            }
        }
    }
}

pub fn remat_consts(func: &mut cranelift_codegen::ir::Function) {
    use cranelift_codegen::cursor::{Cursor, FuncCursor};
    use cranelift_codegen::ir::{InstBuilder, InstructionData, Opcode, ValueDef};
    let blocks: Vec<cranelift_codegen::ir::Block> = func.layout.blocks().collect();
    for block in blocks {
        let mut made: std::collections::HashMap<cranelift_codegen::ir::Value, cranelift_codegen::ir::Value> = Default::default();
        let insts: Vec<cranelift_codegen::ir::Inst> = func.layout.block_insts(block).collect();
        for inst in insts {
            let reads: Vec<cranelift_codegen::ir::Value> = func.dfg.inst_values(inst).collect();
            let mut swap: Vec<(cranelift_codegen::ir::Value, cranelift_codegen::ir::Value)> = Vec::new();
            for v in reads {
                let v = func.dfg.resolve_aliases(v);
                let ValueDef::Result(def, 0) = func.dfg.value_def(v) else { continue };
                if func.layout.inst_block(def) == Some(block) {
                    continue;
                }
                let InstructionData::UnaryImm { opcode: Opcode::Iconst, imm } = func.dfg.insts[def] else { continue };
                let again = match made.get(&v) {
                    Some(&x) => x,
                    None => {
                        let ty = func.dfg.value_type(v);
                        let mut pos = FuncCursor::new(func).at_inst(inst);
                        let x = pos.ins().iconst(ty, imm);
                        made.insert(v, x);
                        x
                    }
                };
                swap.push((v, again));
            }
            if !swap.is_empty() {
                let dfg = &mut func.dfg;
                let resolved: Vec<cranelift_codegen::ir::Value> = dfg.inst_values(inst).map(|x| dfg.resolve_aliases(x)).collect();
                let mut k = 0;
                let new: Vec<cranelift_codegen::ir::Value> = resolved.iter().map(|&x| swap.iter().find(|s| s.0 == x).map_or(x, |s| s.1)).collect();
                dfg.map_inst_values(inst, |_| {
                    let x = new[k];
                    k += 1;
                    x
                });
            }
        }
    }
}

pub fn switch_trees(func: &mut cranelift_codegen::ir::Function) {
    use cranelift_codegen::ir::{Block, InstructionData};
    let mut found = Vec::new();
    for block in func.layout.blocks() {
        let Some(inst) = func.layout.last_inst(block) else { continue };
        if let InstructionData::BranchTable { arg, table, .. } = func.dfg.insts[inst] {
            let (jt, pool) = (&func.dfg.jump_tables[table], &func.dfg.value_lists);
            if jt.all_branches().iter().any(|c| c.args(pool).len() != 0) {
                continue;
            }
            // The table's targets, then its default on past them.
            let targets = jt.as_slice().iter().map(|c| c.block(pool)).chain([jt.default_block().block(pool)]);
            let mut runs: Vec<(u32, Block)> = Vec::new();
            for (x, b) in targets.enumerate() {
                if runs.last().is_none_or(|&(_, l)| l != b) {
                    runs.push((x as u32, b));
                }
            }
            if runs.len() <= SWITCH_TREE_RUNS {
                found.push((block, inst, arg, runs));
            }
        }
    }
    if !found.is_empty() {
        let mut ctx = cranelift_frontend::FunctionBuilderContext::new();
        let mut f = cranelift_frontend::FunctionBuilder::new(func, &mut ctx);
        for (block, inst, index, runs) in found {
            f.func.layout.remove_inst(inst);
            f.switch_to_block(block);
            let end = 1u64 << f.func.dfg.value_type(index).bits();
            crate::native_cpu_stream::dispatch(&mut f, index, &runs, end);
        }
    }
    // Every table left (its targets take arguments, or it has more runs): a balanced tree, so no
    // jump table reaches Cranelift's code (Q186).
    balanced_trees(func);
}

#[cfg(test)]
#[path = "native_cpu_codegen_tests.rs"]
mod tests;
