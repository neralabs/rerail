use cranelift_codegen::Context;
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Module, ModuleError};
use cranelift_codegen::ir::{Block, InstBuilder, Value, condcodes::IntCC, types};
use cranelift_frontend::FunctionBuilder;

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
            bit_test: std::sync::atomic::AtomicBool::new(BIT_TEST_DEFAULT),
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
    cranelift_jit::JITBuilder::with_flags(&rerail_clif::jit::flags(flags), cranelift_module::default_libcall_names()).map_err(Box::new)
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
    define_functions_inlining(module, items, near, &[], &|_| {})
}

/// `define_functions`, with the items named in `hot` the module's hot helpers (decision 0310): every call of one
/// inlined, in the same pass as the tape's hot reads.
pub fn define_functions_inlining(module: &mut JITModule, items: Vec<(FuncId, Context)>, near: bool, hot: &[FuncId], after: &dyn Fn(&mut cranelift_codegen::ir::Function)) -> Result<Vec<Context>, Box<ModuleError>> {
    use cranelift_module::{Module, ModuleReloc};
    let mut items = items;
    // The hot helpers' bodies, legalized, as the inliner takes a callee.
    let mut helpers = std::collections::HashMap::new();
    for (id, context) in &items {
        if hot.contains(id) {
            let mut c = Context::for_function(context.func.clone());
            c.legalize(module.isa()).map_err(|e| Box::new(ModuleError::Compilation(e)))?;
            helpers.insert(*id, c.func);
        }
    }
    for (_, context) in &mut items {
        // The tape's hot reads each calls, and the module's hot helpers, inlined (decision 0310); then the flow graph
        // made again, as the function changed since it was made (`clif_fold`, and the inlining).
        rerail_clif::inline::with_helpers(module, context, &helpers)?;
        after(&mut context.func);
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
            dispatch(&mut f, index, &runs, end);
        }
    }
    // Every table left (its targets take arguments, or it has more runs): a balanced tree, so no
    // jump table reaches Cranelift's code (Q186).
    balanced_trees(func);
}

#[cfg(test)]
#[path = "native_cpu_codegen_tests.rs"]
mod tests;

/// The backend tests a vector's bits against a mask in one instruction (NEON's `cmtst`, the vendored Cranelift's
/// aarch64 lowering since 0381): a class's lane is then the test itself, not its complement. On by default where the vendored
/// Cranelift is built for aarch64 (`cranelift_clmul`, its mark); upstream's has no `cmtst`.
pub const BIT_TEST_DEFAULT: bool = cfg!(all(cranelift_clmul, target_arch = "aarch64"));

/// The name of the symbol a table's address is read from (`table_address`): placed at address 0, so that its
/// relocation, `0 + addend`, writes the table's address itself.
pub const TABLES: &str = "rerail_tier2_tables";
/// `TABLES`'s data, the first the module declares.
pub const TABLES_DATA: u32 = 0;

/// A table's address, as a symbol's (`TABLES`, the table's address its offset): one instruction of the same form
/// wherever the table is. An address as a constant is moved in as many instructions as it has halfwords not zero, so
/// the code, and regalloc2's choices round it, changed with where the heap put the table.
pub fn table_address<T>(f: &mut FunctionBuilder<'_>, at: *const T) -> Value {
    let name = f.func.declare_imported_user_function(cranelift_codegen::ir::UserExternalName::new(1, TABLES_DATA));
    let gv = f.create_global_value(cranelift_codegen::ir::GlobalValueData::Symbol {
        name: cranelift_codegen::ir::ExternalName::user(name),
        offset: cranelift_codegen::ir::immediates::Imm64::new(at as i64),
        colocated: false,
        tls: false,
    });
    f.ins().symbol_value(types::I64, gv)
}

/// A compare tree over a byte (or a place), as rustc lowers a `match` (LLVM's switch lowering):
/// `runs` are runs of one target (the first from 0, the last on to `end`, past every value the
/// switch reads: 256 for a byte). The target with the most runs is the default; the tree is over
/// the other runs alone (the cases), split at the middle, each side knowing the range its value
/// is in; a case is one test at its leaf (an equality, a bound, or a range), and a value in no
/// case goes to the default. A switch that expects one kind is one compare, as `expect(kind)`.
/// Cases of at most three targets within 64 values are bit tests, as LLVM's: one shift, then a
/// mask a target.
pub fn dispatch(f: &mut FunctionBuilder<'_>, x: Value, runs: &[(u32, Block)], end: u64) {
    let ends: Vec<u64> = runs.iter().skip(1).map(|r| u64::from(r.0)).chain([end]).collect();
    let spans: Vec<(u64, u64, Block)> = runs.iter().zip(&ends).enumerate().map(|(i, (r, &e))| (if i == 0 { 0 } else { u64::from(r.0) }, e, r.1)).filter(|&(a, b, _)| a < b).collect();
    // The default: the target with the most runs (the first such).
    let mut count: Vec<(Block, usize)> = Vec::new();
    for &(_, _, b) in &spans {
        match count.iter_mut().find(|(x, _)| *x == b) {
            Some(c) => c.1 += 1,
            None => count.push((b, 1)),
        }
    }
    let default = count.iter().max_by_key(|&&(_, n)| n).map(|&(b, _)| b).expect("a run");
    let cases: Vec<(u64, u64, Block)> = spans.into_iter().filter(|&(_, _, b)| b != default).collect();
    let clusters = clusters(&cases);
    cases_tree(f, x, 0, end, &clusters, default);
}

/// A leaf of `dispatch`'s tree: the values `a..b`, to one target (`Range`), or tested by bits
/// (`Bits`: each target with its mask over `x - a`; a value in none goes to the default).
enum Cluster {
    Range(u64, u64, Block),
    Bits(u64, u64, Vec<(Block, u64)>),
}

impl Cluster {
    fn start(&self) -> u64 {
        match *self {
            Cluster::Range(a, ..) | Cluster::Bits(a, ..) => a,
        }
    }
}

/// `cases` (rising, apart) as clusters: from each case on, the longest run of cases within 64
/// values and of at most three targets, as bit tests where LLVM finds them worth it (one
/// target and 3 compares saved, two and 5, three and 6: a range counting two), else the case.
fn clusters(cases: &[(u64, u64, Block)]) -> Vec<Cluster> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < cases.len() {
        let mut best = None;
        let (mut dests, mut cmps): (Vec<Block>, usize) = (Vec::new(), 0);
        for j in i..cases.len() {
            let (a, b, t) = cases[j];
            if b - cases[i].0 > 64 || b == u64::MAX {
                break;
            }
            if !dests.contains(&t) {
                if dests.len() == 3 {
                    break;
                }
                dests.push(t);
            }
            cmps += if b == a + 1 { 1 } else { 2 };
            let worth = match dests.len() {
                1 => cmps >= 3,
                2 => cmps >= 5,
                _ => cmps >= 6,
            };
            if worth {
                best = Some(j);
            }
        }
        match best {
            Some(j) => {
                let base = cases[i].0;
                let mut masks: Vec<(Block, u64)> = Vec::new();
                for &(a, b, t) in &cases[i..=j] {
                    let m = (a..b).fold(0u64, |m, v| m | 1 << (v - base));
                    match masks.iter_mut().find(|(x, _)| *x == t) {
                        Some(x) => x.1 |= m,
                        None => masks.push((t, m)),
                    }
                }
                // The target with the most values first, as LLVM orders them.
                masks.sort_by_key(|&(_, m)| std::cmp::Reverse(m.count_ones()));
                out.push(Cluster::Bits(base, cases[j].1, masks));
                i = j + 1;
            }
            None => {
                let (a, b, t) = cases[i];
                out.push(Cluster::Range(a, b, t));
                i += 1;
            }
        }
    }
    out
}

/// `dispatch`'s tree over `cases` for a value known to be in `lo..hi`.
fn cases_tree(f: &mut FunctionBuilder<'_>, x: Value, lo: u64, hi: u64, cases: &[Cluster], default: Block) {
    match cases {
        [] => {
            f.ins().jump(default, &[]);
        }
        [Cluster::Range(a, b, to)] => {
            let (a, b, to) = (*a, *b, *to);
            let inside = if a <= lo && b >= hi {
                f.ins().jump(to, &[]);
                return;
            } else if a <= lo {
                f.ins().icmp_imm(IntCC::UnsignedLessThan, x, b as i64)
            } else if b >= hi {
                f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, x, a as i64)
            } else if b == a + 1 {
                f.ins().icmp_imm(IntCC::Equal, x, a as i64)
            } else {
                let d = f.ins().iadd_imm(x, -(a as i64));
                f.ins().icmp_imm(IntCC::UnsignedLessThan, d, (b - a) as i64)
            };
            f.ins().brif(inside, to, &[], default, &[]);
        }
        [Cluster::Bits(a, b, masks)] => {
            let (a, b) = (*a, *b);
            let d = if a == 0 { x } else { f.ins().iadd_imm(x, -(a as i64)) };
            if a > lo || b < hi {
                let tests = f.create_block();
                let inside = f.ins().icmp_imm(IntCC::UnsignedLessThan, d, (b - a) as i64);
                f.ins().brif(inside, tests, &[], default, &[]);
                f.switch_to_block(tests);
            }
            let one = f.ins().iconst(types::I64, 1);
            let bit = f.ins().ishl(one, d);
            // Every value of the cluster some target's: the last target needs no test.
            let full = (a.max(lo)..b.min(hi)).all(|v| masks.iter().any(|&(_, m)| m >> (v - a) & 1 != 0));
            for (k, &(to, m)) in masks.iter().enumerate() {
                if full && k + 1 == masks.len() {
                    f.ins().jump(to, &[]);
                    return;
                }
                let next = if k + 1 == masks.len() { default } else { f.create_block() };
                let t = f.ins().band_imm(bit, m as i64);
                f.ins().brif(t, to, &[], next, &[]);
                if next != default {
                    f.switch_to_block(next);
                }
            }
        }
        _ => {
            let mid = cases.len() / 2;
            let pivot = cases[mid].start();
            let (low, high) = (f.create_block(), f.create_block());
            let below = f.ins().icmp_imm(IntCC::UnsignedLessThan, x, pivot as i64);
            f.ins().brif(below, low, &[], high, &[]);
            f.switch_to_block(low);
            cases_tree(f, x, lo, pivot, &cases[..mid], default);
            f.switch_to_block(high);
            cases_tree(f, x, pivot, hi, &cases[mid..], default);
        }
    }
}
