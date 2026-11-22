//! A SIMD group's own stack (decision 0567, step 3): the compiled kernel runs a SIMD group as a fiber, so a barrier
//! or a poll is a switch to the next SIMD group, the values live across it kept where the compiled code keeps them
//! (its registers saved by the switch, its stack as it was), not stored for the code to find again.
//!
//! A switch saves the callee-saved registers of the platform's convention on the stack it leaves and loads them
//! from the one it enters: AArch64's x19 to x30 and d8 to d15, x86-64 System V's rbx, rbp and r12 to r15.

/// A fiber's saved stack pointer (its registers are on its stack).
#[repr(C)]
#[derive(Default)]
pub struct Ctx {
    sp: u64,
}

/// A stack: its memory, kept while the fiber is. A thread keeps the stacks its fibers are done with (`POOL`), so a
/// launch takes its SIMD groups' stacks from there rather than mapping (and faulting in) new ones.
pub struct Stack {
    mem: Box<[std::mem::MaybeUninit<u128>]>,
}

/// A stack's bytes: the compiled code's frames (no recursion: a call chain is the kernel's call graph's depth) and
/// the helpers it calls.
pub const STACK: usize = 1 << 20;

/// The stacks a thread keeps for its next fibers, at most.
const POOLED: usize = 256;

thread_local! {
    static POOL: std::cell::RefCell<Vec<Box<[std::mem::MaybeUninit<u128>]>>> = const { std::cell::RefCell::new(Vec::new()) };
}

impl Stack {
    pub fn new() -> Stack {
        let kept = POOL.try_with(|p| p.borrow_mut().pop()).ok().flatten();
        // A stack's memory needs no value: a fiber writes it before it reads it.
        Stack { mem: kept.unwrap_or_else(|| Box::new_uninit_slice(STACK / 16)) }
    }

    fn top(&mut self) -> u64 {
        // SAFETY: one past the end of the allocation.
        unsafe { self.mem.as_mut_ptr().add(self.mem.len()) as u64 }
    }
}

impl Drop for Stack {
    fn drop(&mut self) {
        let mem = std::mem::take(&mut self.mem);
        let _ = POOL.try_with(|p| {
            let mut p = p.borrow_mut();
            if p.len() < POOLED {
                p.push(mem);
            }
        });
    }
}

impl Default for Stack {
    fn default() -> Self {
        Self::new()
    }
}

/// A fiber's start: `f(arg)`, which never returns (it switches away at its end).
pub type Start = unsafe extern "C" fn(arg: u64) -> !;

/// `ctx` set to start `f(arg)` on `stack` at its next switch to it.
///
/// # Safety
///
/// `stack` outlives every switch to `ctx`, and `f` never returns.
pub unsafe fn prepare(ctx: &mut Ctx, stack: &mut Stack, f: Start, arg: u64) {
    let top = stack.top() & !15;
    #[cfg(target_arch = "aarch64")]
    {
        // The frame `switch` pops: x19..x30 at 0..96, d8..d15 at 96..160, 176 bytes in all.
        let sp = top - 176;
        let w = |off: u64, v: u64| {
            // SAFETY: within the stack, below its top.
            unsafe { ((sp + off) as *mut u64).write(v) }
        };
        for k in 0..22 {
            w(8 * k, 0);
        }
        w(0, arg); // x19
        w(8, f as usize as u64); // x20
        w(88, trampoline as *const () as u64); // x30
        ctx.sp = sp;
    }
    #[cfg(target_arch = "x86_64")]
    {
        // The frame `switch` pops: r15, r14, r13, r12, rbx, rbp, then the return address; past the return, the stack
        // is 16-aligned, as before a call.
        let sp = top - 72;
        let w = |off: u64, v: u64| {
            // SAFETY: within the stack, below its top.
            unsafe { ((sp + off) as *mut u64).write(v) }
        };
        for k in 0..9 {
            w(8 * k, 0);
        }
        w(24, arg); // r12
        w(16, f as usize as u64); // r13
        w(48, trampoline as usize as u64); // the return address
        ctx.sp = sp;
    }
}

/// Saves the running fiber's registers in `from` and runs the one `to` holds.
///
/// # Safety
///
/// `to` holds a fiber prepared (`prepare`) or switched away from, whose stack is live.
#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
pub unsafe extern "C" fn switch(from: *mut Ctx, to: *const Ctx) {
    core::arch::naked_asm!(
        "sub sp, sp, #176",
        "stp x19, x20, [sp, #0]",
        "stp x21, x22, [sp, #16]",
        "stp x23, x24, [sp, #32]",
        "stp x25, x26, [sp, #48]",
        "stp x27, x28, [sp, #64]",
        "stp x29, x30, [sp, #80]",
        "stp d8, d9, [sp, #96]",
        "stp d10, d11, [sp, #112]",
        "stp d12, d13, [sp, #128]",
        "stp d14, d15, [sp, #144]",
        "mov x9, sp",
        "str x9, [x0]",
        "ldr x9, [x1]",
        "mov sp, x9",
        "ldp x19, x20, [sp, #0]",
        "ldp x21, x22, [sp, #16]",
        "ldp x23, x24, [sp, #32]",
        "ldp x25, x26, [sp, #48]",
        "ldp x27, x28, [sp, #64]",
        "ldp x29, x30, [sp, #80]",
        "ldp d8, d9, [sp, #96]",
        "ldp d10, d11, [sp, #112]",
        "ldp d12, d13, [sp, #128]",
        "ldp d14, d15, [sp, #144]",
        "add sp, sp, #176",
        "ret",
    )
}

#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
pub unsafe extern "C" fn switch(from: *mut Ctx, to: *const Ctx) {
    core::arch::naked_asm!(
        "push rbp", "push rbx", "push r12", "push r13", "push r14", "push r15", "mov [rdi], rsp", "mov rsp, [rsi]", "pop r15", "pop r14", "pop r13", "pop r12", "pop rbx", "pop rbp", "ret",
    )
}

/// A new fiber's first instructions: its start called with its argument.
#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
unsafe extern "C" fn trampoline() {
    core::arch::naked_asm!("mov x0, x19", "mov x29, #0", "blr x20", "brk #0")
}

#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
unsafe extern "C" fn trampoline() {
    core::arch::naked_asm!("mov rdi, r12", "xor ebp, ebp", "call r13", "ud2")
}
