//! The JIT's one inlining pass (decision 0310): a function's calls of the tape's reads
//! (`reads::READS`, found by their declared names) are inlined where the read is hot, from its one
//! CLIF body, and stay calls where it is cold, of the read defined once in the module. The policy is
//! the read's mark, never the call site's.
//!
//! A read is declared in a module by name (`func_ref`), as any other function the JIT's code calls.
//! The pass needs no state of its own: it reads the module's declarations, builds the bodies of the
//! reads the function reaches, and defines a cold read's body, itself through the pass, the first
//! time a function calls it.

// The module's errors are Cranelift's `ModuleError`, as `Module` returns them: large, and passed up as they are.
#![allow(clippy::result_large_err)]

use crate::reads::{self, READS, Read};
use cranelift_codegen::Context;
use cranelift_codegen::inline::{Inline, InlineCommand};
use cranelift_codegen::ir::{self, ExternalName, Function, Signature, UserFuncName};
use cranelift_module::{FuncId, Linkage, Module, ModuleError};
use std::borrow::Cow;
use std::collections::HashMap;

/// The caller's reference to the read `name`, declared in `module` (once; again returns its id).
pub fn func_ref<M: Module>(module: &mut M, name: &str, func: &mut Function) -> Result<ir::FuncRef, ModuleError> {
    let id = declare(module, reads::read(name).ok_or_else(|| ModuleError::Undeclared(name.to_owned()))?)?;
    Ok(module.declare_func_in_func(id, func))
}

fn declare<M: Module>(module: &mut M, read: &Read) -> Result<FuncId, ModuleError> {
    let signature = reads::signature(read, module.isa().default_call_conv());
    module.declare_function(read.name, Linkage::Local, &signature)
}

/// The reads `func`'s call instructions call: (id, read), each once.
fn called<M: Module>(module: &M, func: &Function) -> Vec<(FuncId, &'static Read)> {
    let mut out = Vec::new();
    for block in func.layout.blocks() {
        for inst in func.layout.block_insts(block) {
            let ir::InstructionData::Call { func_ref, .. } = func.dfg.insts[inst] else { continue };
            let ExternalName::User(name) = &func.dfg.ext_funcs[func_ref].name else { continue };
            let name = &func.params.user_named_funcs()[*name];
            if name.namespace != 0 {
                continue;
            }
            let id = FuncId::from_u32(name.index);
            let declared = &module.declarations().get_function_decl(id).name;
            if let Some(read) = declared.as_deref().and_then(reads::read)
                && !out.iter().any(|&(known, _)| known == id)
            {
                out.push((id, read));
            }
        }
    }
    out
}

/// The names of the reads `func` still calls (after the pass: the cold ones it reaches).
pub fn reads_called<M: Module>(module: &M, func: &Function) -> Vec<&'static str> {
    called(module, func).into_iter().map(|(_, read)| read.name).collect()
}

struct ModuleCallee<'m, M: Module> {
    module: &'m mut M,
}

impl<M: Module> reads::Callee for ModuleCallee<'_, M> {
    fn func_ref(&mut self, name: &str, _: &Signature, func: &mut Function) -> ir::FuncRef {
        func_ref(self.module, name, func).expect("a read declared")
    }
}

/// The read's body in `module`, legalized (as the inliner takes a callee).
fn body<M: Module>(module: &mut M, id: FuncId, read: &Read) -> Result<Function, ModuleError> {
    let call_conv = module.isa().default_call_conv();
    let func = reads::build(read, call_conv, UserFuncName::user(0, id.as_u32()), &mut ModuleCallee { module });
    let mut context = Context::for_function(func);
    context.legalize(module.isa()).map_err(ModuleError::Compilation)?;
    Ok(context.func)
}

/// The hot reads' bodies, by id.
struct Policy {
    bodies: HashMap<FuncId, Function>,
}

impl Inline for &Policy {
    fn inline(&mut self, caller: &Function, _: ir::Inst, opcode: ir::Opcode, callee: ir::FuncRef, _: &[ir::Value]) -> InlineCommand<'_> {
        if opcode != ir::Opcode::Call {
            return InlineCommand::KeepCall;
        }
        let ExternalName::User(name) = &caller.dfg.ext_funcs[callee].name else { return InlineCommand::KeepCall };
        let name = &caller.params.user_named_funcs()[*name];
        match self.bodies.get(&FuncId::from_u32(name.index)).filter(|_| name.namespace == 0) {
            Some(body) => InlineCommand::Inline { callee: Cow::Borrowed(body), visit_callee: true },
            None => InlineCommand::KeepCall,
        }
    }
}

/// The pass over `context`'s function before it is compiled in `module`: every call of a hot read
/// inlined (and the reads its body calls, through the pass), every cold read it still calls defined
/// in the module. A function that calls no read is left as it is.
pub fn reads<M: Module>(module: &mut M, context: &mut Context) -> Result<(), ModuleError> {
    if called(module, &context.func).is_empty() {
        return Ok(());
    }
    // Every hot read: their bodies call each other.
    let mut bodies = HashMap::new();
    for read in READS.iter().filter(|read| read.hot) {
        let id = declare(module, read)?;
        bodies.insert(id, body(module, id, read)?);
    }
    let policy = Policy { bodies };
    context.inline(&policy).map_err(ModuleError::Compilation)?;
    for (id, read) in called(module, &context.func) {
        define(module, id, read)?;
    }
    Ok(())
}

/// The read's body defined in `module`, through the pass, unless it is already.
fn define<M: Module>(module: &mut M, id: FuncId, read: &Read) -> Result<(), ModuleError> {
    let mut context = Context::for_function(body(module, id, read)?);
    reads(module, &mut context)?;
    match module.define_function(id, &mut context) {
        Ok(()) | Err(ModuleError::DuplicateDefinition(_)) => Ok(()),
        Err(error) => Err(error),
    }
}
