//! A test's JIT of every read, of every family (decisions 0310, 0325).

use crate::view::TapeView;
use cranelift_module::Module;

/// The reads written once in CLIF (decisions 0310, 0325), compiled by the JIT through its inlining pass, each
/// called through an exported function of its own (a hot read inlined into it, a cold one a call): what a test runs
/// against a hand-written reader. Building it checks the pass: every hot read inlined, every cold one a call.
pub struct Reads {
    _module: cranelift_jit::JITModule,
    fns: std::collections::HashMap<&'static str, *const u8>,
}

impl Reads {
    // The closure returns Cranelift's `ModuleError`, as `define_function` does.
    #[allow(clippy::new_without_default, clippy::result_large_err)]
    pub fn new() -> Self {
        use cranelift_codegen::ir::{InstBuilder, UserFuncName};
        use cranelift_module::Linkage;
        let mut builder = cranelift_jit::JITBuilder::new(cranelift_module::default_libcall_names()).unwrap();
        // One arena, so a call between the module's functions stays in a `bl`'s reach (±128 MiB on aarch64), however
        // far apart the process's other allocations put separately allocated code (as the compiler's JITs do).
        builder.memory_provider(Box::new(cranelift_jit::ArenaMemoryProvider::new_with_size(64 << 20).unwrap()));
        let mut module = cranelift_jit::JITModule::new(builder);
        crate::inline::define_cold(&mut module, &mut |module, id, context| module.define_function(id, context)).unwrap();
        let mut ids = Vec::new();
        for read in crate::reads::READS {
            let signature = crate::reads::signature(read, module.isa().default_call_conv());
            let id = module.declare_function(&format!("call_{}", read.name), Linkage::Export, &signature).unwrap();
            let mut context = module.make_context();
            context.func.signature = signature;
            context.func.name = UserFuncName::user(0, id.as_u32());
            let mut fctx = cranelift_frontend::FunctionBuilderContext::new();
            let mut f = cranelift_frontend::FunctionBuilder::new(&mut context.func, &mut fctx);
            let entry = f.create_block();
            f.append_block_params_for_function_params(entry);
            f.switch_to_block(entry);
            f.seal_block(entry);
            let params = f.block_params(entry).to_vec();
            let callee = crate::inline::func_ref(&mut module, read.name, f.func).unwrap();
            let call = f.ins().call(callee, &params);
            let result = f.inst_results(call)[0];
            f.ins().return_(&[result]);
            f.finalize();
            crate::inline::reads(&mut module, &mut context).unwrap();
            // A hot read is inlined (what it calls is cold); a cold one stays a call.
            let called = crate::inline::reads_called(&module, &context.func);
            assert_eq!(called.contains(&read.name), !read.hot, "{} after the pass calls {called:?}", read.name);
            assert!(called.iter().all(|name| !crate::reads::read(name).unwrap().hot), "{} calls a hot read: {called:?}", read.name);
            module.define_function(id, &mut context).unwrap();
            ids.push((read.name, id));
        }
        module.finalize_definitions().unwrap();
        let fns = ids.into_iter().map(|(name, id)| (name, module.get_finalized_function(id))).collect();
        Self { _module: module, fns }
    }

    /// The compiled read `name`, for a caller that types it by its signature (a relation's reads: its
    /// descriptor's address, then its own parameters).
    pub fn function(&self, name: &str) -> *const u8 {
        self.fns[name]
    }

    pub fn one(&self, name: &str, tape: &[u8], view: &TapeView, i: usize) -> u32 {
        // SAFETY: the read's signature: the tape's base, its view, a node.
        let f: extern "C" fn(*const u8, *const TapeView, u32) -> u32 = unsafe { std::mem::transmute(self.fns[name]) };
        f(tape.as_ptr(), view, i as u32)
    }

    pub fn two(&self, name: &str, tape: &[u8], view: &TapeView, i: usize, k: u32) -> u32 {
        // SAFETY: as `one`, with a second u32.
        let f: extern "C" fn(*const u8, *const TapeView, u32, u32) -> u32 = unsafe { std::mem::transmute(self.fns[name]) };
        f(tape.as_ptr(), view, i as u32, k)
    }
}

/// The writer's ops (decision 0399 §6) compiled by the JIT, each its own exported function, calling the others: what
/// a test runs against their Rust and the reference writer (R15). Each takes the state's address, then its own
/// parameters, in the platform's C convention.
pub struct Writer {
    _module: cranelift_jit::JITModule,
    fns: std::collections::HashMap<&'static str, *const u8>,
}

impl Writer {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let mut builder = cranelift_jit::JITBuilder::new(cranelift_module::default_libcall_names()).unwrap();
        // One arena, so a call between the ops stays in a `bl`'s reach (as `Reads`).
        builder.memory_provider(Box::new(cranelift_jit::ArenaMemoryProvider::new_with_size(16 << 20).unwrap()));
        let mut module = cranelift_jit::JITModule::new(builder);
        let ids = crate::writer::define_ops(&mut module, cranelift_module::Linkage::Export).unwrap();
        module.finalize_definitions().unwrap();
        let fns = ids.into_iter().map(|(name, (id, _))| (name, module.get_finalized_function(id))).collect();
        Self { _module: module, fns }
    }

    /// The compiled op `name`, for a caller that types it by its signature (`writer::signature`).
    pub fn function(&self, name: &str) -> *const u8 {
        self.fns[name]
    }
}
