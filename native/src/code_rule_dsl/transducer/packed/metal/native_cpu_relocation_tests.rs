use super::*;
use cranelift_codegen::binemit::Reloc;
use cranelift_codegen::ir::{AbiParam, InstBuilder, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{BranchProtection, JITBuilder, JITMemoryProvider, SystemMemoryProvider};
use cranelift_module::{
    Linkage, ModuleReloc, ModuleRelocTarget, ModuleResult, default_libcall_names,
};
use std::cell::RefCell;
use std::io;

const RESERVATION: usize = 256 * 1024 * 1024;
const SEPARATION: usize = 192 * 1024 * 1024;

type Relocations = Vec<(FuncId, Vec<ModuleReloc>)>;

thread_local! {
    static RELOCATIONS: RefCell<Option<Relocations>> = const { RefCell::new(None) };
}

pub(super) fn record(id: FuncId, context: &Context) {
    RELOCATIONS.with_borrow_mut(|records| {
        if let Some(records) = records {
            let relocations = context
                .compiled_code()
                .unwrap()
                .buffer
                .relocs()
                .iter()
                .map(|reloc| ModuleReloc::from_mach_reloc(reloc, &context.func, id))
                .collect();
            records.push((id, relocations));
        }
    });
}

struct Recording;

impl Recording {
    fn start() -> Self {
        RELOCATIONS.with_borrow_mut(|records| {
            assert!(records.is_none());
            *records = Some(Vec::new());
        });
        Self
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        RELOCATIONS.with_borrow_mut(|records| *records = None);
    }
}

// Reserve address space without committing the gap. Alternate real generated
// functions between two regions to exercise forward and backward distant calls.
struct SeparatedCode {
    base: usize,
    page: usize,
    positions: [usize; 2],
    allocations: Vec<(usize, usize)>,
    system: SystemMemoryProvider,
}

impl SeparatedCode {
    fn new() -> Self {
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        assert!(page > 0);
        let base = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                RESERVATION,
                libc::PROT_NONE,
                libc::MAP_PRIVATE | libc::MAP_ANON,
                -1,
                0,
            )
        };
        assert_ne!(base, libc::MAP_FAILED);
        Self {
            base: base as usize,
            page: page as usize,
            positions: [0, 0],
            allocations: Vec::new(),
            system: SystemMemoryProvider::new(),
        }
    }
}

impl JITMemoryProvider for SeparatedCode {
    fn allocate_readexec(&mut self, size: usize, align: u64) -> io::Result<*mut u8> {
        assert!(align <= self.page as u64);
        let region = self.allocations.len() % 2;
        let size = size.div_ceil(self.page) * self.page;
        assert!(self.positions[region] + size < RESERVATION - SEPARATION);
        let address = self.base + region * SEPARATION + self.positions[region];
        let status =
            unsafe { libc::mprotect(address as *mut _, size, libc::PROT_READ | libc::PROT_WRITE) };
        if status != 0 {
            return Err(io::Error::last_os_error());
        }
        self.positions[region] += size;
        self.allocations.push((address, size));
        Ok(address as *mut u8)
    }

    fn allocate_readwrite(&mut self, size: usize, align: u64) -> io::Result<*mut u8> {
        self.system.allocate_readwrite(size, align)
    }

    fn allocate_readonly(&mut self, size: usize, align: u64) -> io::Result<*mut u8> {
        self.system.allocate_readonly(size, align)
    }

    unsafe fn free_memory(&mut self) {
        unsafe { self.system.free_memory() };
    }

    fn finalize(&mut self, protection: BranchProtection) -> ModuleResult<()> {
        self.system.finalize(protection)?;
        unsafe extern "C" {
            fn sys_icache_invalidate(start: *mut libc::c_void, len: usize);
        }
        for &(address, size) in &self.allocations {
            unsafe {
                sys_icache_invalidate(address as *mut _, size);
                assert_eq!(
                    libc::mprotect(address as *mut _, size, libc::PROT_READ | libc::PROT_EXEC),
                    0
                );
            }
        }
        Ok(())
    }
}

impl Drop for SeparatedCode {
    fn drop(&mut self) {
        // Test function pointers never outlive their module.
        unsafe {
            self.system.free_memory();
            assert_eq!(libc::munmap(self.base as *mut _, RESERVATION), 0);
        }
    }
}

fn separated_module() -> JITModule {
    let mut builder = JITBuilder::new(default_libcall_names()).unwrap();
    builder.memory_provider(Box::new(SeparatedCode::new()));
    JITModule::new(builder)
}

fn assert_targets(module: &JITModule) -> usize {
    RELOCATIONS.with_borrow(|records| {
        let mut distant_calls = 0;
        for (id, relocations) in records.as_ref().unwrap() {
            let caller = module.get_finalized_function(*id);
            for reloc in relocations {
                let ModuleRelocTarget::User { namespace: 0, index } = reloc.name else {
                    continue;
                };
                let expected = (module.get_finalized_function(FuncId::from_u32(index)) as usize)
                    .wrapping_add_signed(reloc.addend as isize);
                let at = unsafe { caller.add(reloc.offset as usize) };
                let actual = match reloc.kind {
                    Reloc::Arm64Call => {
                        let instruction = unsafe { at.cast::<u32>().read_unaligned() };
                        assert_eq!(instruction & 0xfc000000, 0x94000000);
                        let displacement = ((instruction << 6) as i32 >> 6) as isize * 4;
                        (at as usize).wrapping_add_signed(displacement)
                    }
                    Reloc::Abs8 => unsafe { at.cast::<u64>().read_unaligned() as usize },
                    other => panic!("unexpected function relocation: {other:?}"),
                };
                if expected.abs_diff(at as usize) >= 128 * 1024 * 1024 {
                    distant_calls += 1;
                }
                assert_eq!(
                    actual, expected,
                    "finalized call must reach its declared callee: caller={id:?}, pc={at:p}, callee={index}, relocation={:?}",
                    reloc.kind
                );
            }
        }
        distant_calls
    })
}

#[test]
fn native_calls_reach_distant_callees_in_both_directions() {
    let _recording = Recording::start();
    let mut module = separated_module();
    let mut signature = module.make_signature();
    signature.returns.push(AbiParam::new(types::I64));
    let ids = ["leaf", "middle", "entry"].map(|name| {
        module
            .declare_function(name, Linkage::Local, &signature)
            .unwrap()
    });
    for (index, &id) in ids.iter().enumerate() {
        let mut context = module.make_context();
        context.func.signature = signature.clone();
        let reference =
            (index > 0).then(|| module.declare_func_in_func(ids[index - 1], &mut context.func));
        let mut frontend = FunctionBuilderContext::new();
        let mut function = FunctionBuilder::new(&mut context.func, &mut frontend);
        let block = function.create_block();
        function.switch_to_block(block);
        let value = if let Some(reference) = reference {
            let call = function.ins().call(reference, &[]);
            let value = function.inst_results(call)[0];
            function.ins().iadd_imm(value, 1)
        } else {
            function.ins().iconst(types::I64, 40)
        };
        function.ins().return_(&[value]);
        function.seal_all_blocks();
        function.finalize();
        define_function(&mut module, id, &mut context).unwrap();
    }
    module.finalize_definitions().unwrap();
    // Check bytes before execution, so the red test fails without a bad jump.
    assert_eq!(assert_targets(&module), 2);
    let entry = unsafe {
        std::mem::transmute::<*const u8, unsafe extern "C" fn() -> u64>(
            module.get_finalized_function(ids[2]),
        )
    };
    assert_eq!(unsafe { entry() }, 42);
}

#[test]
fn production_regex_helpers_reach_distant_callees() {
    let _recording = Recording::start();
    let mut module = separated_module();
    crate::policy_dsl::transducer::native_cpu_regex::define_helpers(&mut module).unwrap();
    module.finalize_definitions().unwrap();
    assert!(assert_targets(&module) > 0);
}
