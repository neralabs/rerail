//! cranelift-jit's API on wasm32. A browser gives a module no executable memory, so `JITBuilder::new` fails
//! with a backend error, the transducer's callers take their interpreter path, and no `JITModule` is ever made
//! (it is uninhabited: every method is unreachable by construction).
use cranelift_codegen::{Context, ir, isa};
use cranelift_control::ControlPlane;
use cranelift_module::*;

fn no_jit<T>() -> ModuleResult<T> {
    Err(ModuleError::Backend(anyhow::anyhow!("no JIT on wasm32")))
}

type LibcallNames = Box<dyn Fn(ir::LibCall) -> String + Send + Sync>;

pub struct JITBuilder(std::convert::Infallible);
impl JITBuilder {
    pub fn new(_: LibcallNames) -> ModuleResult<Self> {
        no_jit()
    }
    pub fn with_flags(_: &[(&str, &str)], _: LibcallNames) -> ModuleResult<Self> {
        no_jit()
    }
    pub fn symbol<K: Into<String>>(&mut self, _: K, _: *const u8) -> &mut Self {
        self
    }
}

pub struct JITModule(std::convert::Infallible);
impl JITModule {
    pub fn new(builder: JITBuilder) -> Self {
        match builder.0 {}
    }
    pub fn get_finalized_function(&self, _: FuncId) -> *const u8 {
        match self.0 {}
    }
    pub fn get_finalized_data(&self, _: DataId) -> (*const u8, usize) {
        match self.0 {}
    }
    pub fn finalize_definitions(&mut self) -> ModuleResult<()> {
        match self.0 {}
    }
    /// # Safety
    /// Never callable: no `JITModule` exists on wasm32.
    pub unsafe fn free_memory(self) {
        match self.0 {}
    }
}

impl Module for JITModule {
    fn isa(&self) -> &dyn isa::TargetIsa {
        match self.0 {}
    }
    fn declarations(&self) -> &ModuleDeclarations {
        match self.0 {}
    }
    fn declare_function(&mut self, _: &str, _: Linkage, _: &ir::Signature) -> ModuleResult<FuncId> {
        match self.0 {}
    }
    fn declare_anonymous_function(&mut self, _: &ir::Signature) -> ModuleResult<FuncId> {
        match self.0 {}
    }
    fn declare_data(&mut self, _: &str, _: Linkage, _: bool, _: bool) -> ModuleResult<DataId> {
        match self.0 {}
    }
    fn declare_anonymous_data(&mut self, _: bool, _: bool) -> ModuleResult<DataId> {
        match self.0 {}
    }
    fn define_function_with_control_plane(&mut self, _: FuncId, _: &mut Context, _: &mut ControlPlane) -> ModuleResult<()> {
        match self.0 {}
    }
    fn define_function_bytes(&mut self, _: FuncId, _: u64, _: &[u8], _: &[ModuleReloc]) -> ModuleResult<()> {
        match self.0 {}
    }
    fn define_data(&mut self, _: DataId, _: &DataDescription) -> ModuleResult<()> {
        match self.0 {}
    }
}
