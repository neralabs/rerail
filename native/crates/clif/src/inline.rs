//! The JIT's one inlining pass (decision 0310): a function's calls of the tape's reads
//! (`reads::READS`, found by their declared names) are inlined where the read is hot, from its one
//! CLIF body, and stay calls where it is cold, of the read defined once in the module. The policy is
//! the read's mark, never the call site's.
//!
//! A read is declared in a module by name (`func_ref`), as any other function the JIT's code calls.
//! The pass needs no state of its own: it reads the module's declarations and builds the bodies of the
//! hot reads the function reaches. The module defines its cold reads once, when it is made
//! (`define_cold`).
//!
//! The functions here return Cranelift's own `ModuleError`, as `Module`'s methods do: its size is Cranelift's.
#![allow(clippy::result_large_err)]

use crate::reads::{self, READS, Read};
use cranelift_codegen::Context;
use cranelift_codegen::inline::{Inline, InlineCommand};
use cranelift_codegen::ir::{self, ExternalName, Function, Signature, UserFuncName};
use cranelift_codegen::isa::CallConv;
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

/// Every read declared in a module, by id: what a function's builder imports a read by, with no module at
/// hand (a helper deep in the code that builds a function). Copied where it is needed.
#[derive(Clone, Copy, Debug)]
pub struct ReadIds {
    ids: [FuncId; READS.len()],
    call_conv: CallConv,
}

impl ReadIds {
    /// Every read declared in `module` (again: the same ids).
    pub fn declare<M: Module>(module: &mut M) -> Result<Self, ModuleError> {
        let mut ids = [FuncId::from_u32(0); READS.len()];
        for (id, read) in ids.iter_mut().zip(READS) {
            *id = declare(module, read)?;
        }
        Ok(Self { ids, call_conv: module.isa().default_call_conv() })
    }

    /// The read `name`'s external name in the module: what a check of a function's calls allows.
    pub fn name(&self, name: &str) -> ir::UserExternalName {
        let k = READS.iter().position(|read| read.name == name).expect("a read");
        ir::UserExternalName { namespace: 0, index: self.ids[k].as_u32() }
    }

    /// `func`'s reference to the read `name`: its import of it, made the first time.
    pub fn func_ref(&self, name: &str, func: &mut Function) -> ir::FuncRef {
        let k = READS.iter().position(|read| read.name == name).expect("a read");
        let index = self.ids[k].as_u32();
        let known = func.dfg.ext_funcs.iter().find(|(_, data)| match &data.name {
            ExternalName::User(name) => {
                let name = &func.params.user_named_funcs()[*name];
                name.namespace == 0 && name.index == index
            }
            _ => false,
        });
        if let Some((func_ref, _)) = known {
            return func_ref;
        }
        let signature = func.import_signature(reads::signature(&READS[k], self.call_conv));
        let name = func.declare_imported_user_function(ir::UserExternalName { namespace: 0, index });
        func.import_function(ir::ExtFuncData { name: ExternalName::user(name), signature, colocated: false })
    }
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

/// The pass over `context`'s function before it is compiled in `module`: every call of a hot read inlined,
/// and the hot reads its body calls, through the pass. A cold read stays a call, of the read the module
/// defined once (`define_cold`). A function that calls no hot read is left as it is.
pub fn reads<M: Module>(module: &mut M, context: &mut Context) -> Result<(), ModuleError> {
    let mut work: Vec<(FuncId, &Read)> = called(module, &context.func).into_iter().filter(|(_, read)| read.hot).collect();
    if work.is_empty() {
        return Ok(());
    }
    // The hot reads the function reaches: their bodies call each other.
    let mut bodies = HashMap::new();
    while let Some((id, read)) = work.pop() {
        if bodies.contains_key(&id) {
            continue;
        }
        let body = body(module, id, read)?;
        work.extend(called(module, &body).into_iter().filter(|(id, read)| read.hot && !bodies.contains_key(id)));
        bodies.insert(id, body);
    }
    context.inline(&Policy { bodies }).map(|_| ()).map_err(ModuleError::Compilation)
}

/// Every cold read defined in `module`, once, by `define` (the module's way to define a function), each
/// through the pass: what a call the pass leaves calls.
pub fn define_cold<M: Module>(module: &mut M, define: &mut dyn FnMut(&mut M, FuncId, &mut Context) -> Result<(), ModuleError>) -> Result<(), ModuleError> {
    let all: Vec<&str> = READS.iter().filter(|read| !read.hot).map(|read| read.name).collect();
    define_cold_of(module, &all, define)
}

/// `define_cold`, of the cold reads named (those a module's functions still call after the pass,
/// `reads_called`) and the cold reads their bodies call: a module whose code reaches few reads defines
/// those alone.
pub fn define_cold_of<M: Module>(module: &mut M, names: &[&str], define: &mut dyn FnMut(&mut M, FuncId, &mut Context) -> Result<(), ModuleError>) -> Result<(), ModuleError> {
    let mut work: Vec<&'static Read> = names.iter().filter_map(|name| reads::read(name)).filter(|read| !read.hot).collect();
    let mut done: Vec<&str> = Vec::new();
    while let Some(read) = work.pop() {
        if done.contains(&read.name) {
            continue;
        }
        done.push(read.name);
        let id = declare(module, read)?;
        let mut context = Context::for_function(body(module, id, read)?);
        reads(module, &mut context)?;
        work.extend(called(module, &context.func).into_iter().map(|(_, read)| read).filter(|read| !read.hot && !done.contains(&read.name)));
        define(module, id, &mut context)?;
    }
    Ok(())
}
