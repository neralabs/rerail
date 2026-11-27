//! What each thread is doing now, for the profile's CPU sampler (`rerail_prof::work`): one word a thread, its span,
//! join, step and kind of work, which the thread writes as it goes and a sampler reads from another thread. The
//! sampler charges each thread's own CPU time since its last look to the word it finds, so its counts are CPU time
//! by what the engine was doing, not events.
//!
//! A word is carried with a job to the thread that runs it (`job_run`, `enter`), as the heap span is. Off (no thread
//! registered, nothing written) unless the profile turned it on (`enable`).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

/// A thread's word and its CPU clock's handle (a Mach thread port on macOS, a CPU-time clock id on Linux).
pub struct Slot {
    pub word: AtomicU64,
    os: u64,
    /// The thread's name when it registered.
    pub name: String,
}

impl Slot {
    /// Sets the parts `mask` of the word to `value`'s, returning the word before.
    #[inline]
    pub fn set_part(&self, mask: u64, value: u64) -> u64 {
        let was = self.word.load(Relaxed);
        self.word.store((was & !mask) | (value & mask), Relaxed);
        was
    }
    /// The thread's CPU time (user and system) in ns, read from any thread; `None` once the thread is gone, or where
    /// the platform has no such read.
    pub fn cpu_ns(&self) -> Option<u64> {
        thread_cpu_ns(self.os).map(|(ns, _)| ns)
    }
    /// The thread's CPU time in ns, and whether it is in its own code now (`None` where the platform's CPU clock is
    /// exact, and the sampler charges the CPU added since its last look instead). On macOS a running thread's CPU time
    /// is added late, at its switches and the clock's ticks (a look from another thread finds no CPU added in about a
    /// third of the milliseconds a spinning thread ran), and a thread that yields or spins in the kernel reads as
    /// running: the sampler stops the thread for a moment and reads where it is (`in_user_code`), and counts the looks
    /// that find it out of the kernel, as a stack sampler does, and spreads the thread's CPU over them.
    pub fn look(&self) -> Option<(u64, Option<bool>)> {
        let (cpu, running) = thread_cpu_ns(self.os)?;
        Some((cpu, running.map(|r| r && in_user_code(self.os))))
    }
}

static ON: AtomicBool = AtomicBool::new(false);
static SLOTS: parking_lot::Mutex<Vec<&'static Slot>> = parking_lot::Mutex::new(Vec::new());

thread_local! {
    static MINE: std::cell::Cell<Option<&'static Slot>> = const { std::cell::Cell::new(None) };
}

/// Turns the words on for the process: each thread registers at its first write.
pub fn enable() {
    ON.store(true, Relaxed);
}

/// Whether the words are on.
#[inline]
pub fn on() -> bool {
    ON.load(Relaxed)
}

/// The threads registered so far.
pub fn slots() -> Vec<&'static Slot> {
    SLOTS.lock().clone()
}

/// This thread's slot, registered now if it has none (words on): kept by a caller that writes its word often (a
/// walk's step clock), to write it without finding the thread's slot each time.
#[inline]
pub fn mine() -> Option<&'static Slot> {
    if !on() {
        return None;
    }
    MINE.try_with(|m| match m.get() {
        Some(s) => Some(s),
        None => {
            let os = this_thread()?;
            // One a thread, kept for the process: the sampler reads it after the thread ends (its clock then fails).
            let s: &'static Slot = Box::leak(Box::new(Slot { word: AtomicU64::new(0), os, name: std::thread::current().name().unwrap_or("unnamed").to_owned() }));
            SLOTS.lock().push(s);
            m.set(Some(s));
            Some(s)
        }
    })
    .ok()
    .flatten()
}

/// This thread's word (0: none, or off).
#[inline]
pub fn word() -> u64 {
    mine().map_or(0, |s| s.word.load(Relaxed))
}

/// Sets this thread's word, returning the one before.
#[inline]
pub fn set(word: u64) -> u64 {
    mine().map_or(0, |s| s.word.swap(word, Relaxed))
}

/// The word's parts: span id (bits 0..32), join id (32..48), step + 1 (48..56, 0 none), kind of work (56..64).
pub const SPAN: u64 = 0xffff_ffff;
pub const JOIN: u64 = 0xffff << 32;
pub const STEP: u64 = 0xff << 48;
pub const KIND: u64 = 0xff << 56;

/// Sets the parts `mask` of this thread's word to `value`'s, returning the word before.
#[inline]
pub fn set_part(mask: u64, value: u64) -> u64 {
    mine().map_or(0, |s| s.set_part(mask, value))
}

/// Puts back a word `set` or `set_part` returned.
#[inline]
pub fn restore(word: u64) {
    if let Some(s) = mine() {
        s.word.store(word, Relaxed);
    }
}

/// A kind of work set until the guard drops (`doing`): the thread's slot and the word before, put back.
pub struct Doing(Option<(&'static Slot, u64)>);

impl Doing {
    /// The word as it was before the guard, until the returned guard drops: for a callback a read calls (its caller's
    /// work, the rest of a walk), which is not the read's.
    #[inline]
    pub fn paused(&self) -> Doing {
        Doing(self.0.map(|(s, was)| (s, s.word.swap(was, Relaxed))))
    }
}

impl Drop for Doing {
    #[inline]
    fn drop(&mut self) {
        if let Some((s, w)) = self.0 {
            s.word.store(w, Relaxed);
        }
    }
}

/// This thread is doing work of kind `kind` (a code `rerail_prof::work` names) until the guard drops: the innermost
/// guard names the work, so a read guards itself wherever it is called from.
#[inline]
pub fn doing(kind: u8) -> Doing {
    Doing(mine().map(|s| (s, s.set_part(KIND, u64::from(kind) << 56))))
}

#[cfg(target_vendor = "apple")]
fn this_thread() -> Option<u64> {
    // SAFETY: the calling thread's port; its reference is kept with the slot for the process.
    Some(u64::from(unsafe { libc::mach_thread_self() }))
}

#[cfg(target_vendor = "apple")]
fn thread_cpu_ns(os: u64) -> Option<(u64, Option<bool>)> {
    let mut info = std::mem::MaybeUninit::<libc::thread_basic_info>::zeroed();
    let mut count = (std::mem::size_of::<libc::thread_basic_info>() / std::mem::size_of::<libc::natural_t>()) as libc::mach_msg_type_number_t;
    // SAFETY: `info` has room for `count` words of the basic info; the port is a thread's, or the call fails.
    let r = unsafe { libc::thread_info(os as libc::thread_inspect_t, libc::THREAD_BASIC_INFO as libc::thread_flavor_t, info.as_mut_ptr().cast(), &mut count) };
    if r != 0 {
        return None;
    }
    // SAFETY: written by the call that succeeded.
    let info = unsafe { info.assume_init() };
    let us = |t: libc::time_value_t| t.seconds as u64 * 1_000_000 + t.microseconds as u64;
    let running = info.run_state == libc::TH_STATE_RUNNING && info.flags & libc::TH_FLAGS_IDLE == 0;
    Some(((us(info.user_time) + us(info.system_time)) * 1000, Some(running)))
}

#[cfg(target_vendor = "apple")]
unsafe extern "C" {
    fn thread_suspend(thread: libc::mach_port_t) -> libc::kern_return_t;
    fn thread_resume(thread: libc::mach_port_t) -> libc::kern_return_t;
    fn thread_get_state(thread: libc::mach_port_t, flavor: libc::c_int, state: *mut u32, count: *mut libc::mach_msg_type_number_t) -> libc::kern_return_t;
}

/// Whether the thread `os` is in user code now, not in a system call (a wait, a yield, a read): stopped for the time
/// it takes to read its registers, and its instruction before the pc read, which is a system call's (`svc #0x80`,
/// `syscall`) where the thread waits in the kernel. Nothing here allocates or locks while the thread is stopped (it
/// may hold the allocator's lock); a pc at its page's start is taken as user code (the page before may be unmapped).
#[cfg(target_vendor = "apple")]
fn in_user_code(os: u64) -> bool {
    let port = os as libc::mach_port_t;
    // SAFETY: a thread's port; stopped and resumed here, its registers read into `state`, which has room for them.
    unsafe {
        if thread_suspend(port) != 0 {
            return false;
        }
        let mut state = [0u32; 68];
        #[cfg(target_arch = "aarch64")]
        let (flavor, count, pc_at) = (6, 68, 32 * 2);
        #[cfg(target_arch = "x86_64")]
        let (flavor, count, pc_at) = (4, 42, 16 * 2);
        let mut n = count;
        let got = thread_get_state(port, flavor, state.as_mut_ptr(), &mut n) == 0;
        let pc = u64::from(state[pc_at]) | u64::from(state[pc_at + 1]) << 32;
        let in_kernel = got && pc & 0xfff >= 4 && {
            #[cfg(target_arch = "aarch64")]
            {
                std::ptr::read_volatile((pc - 4) as *const u32) == 0xd400_1001
            }
            #[cfg(target_arch = "x86_64")]
            {
                std::ptr::read_volatile((pc - 2) as *const u16) == 0x050f
            }
        };
        thread_resume(port);
        got && !in_kernel
    }
}

#[cfg(target_os = "linux")]
fn this_thread() -> Option<u64> {
    let mut id: libc::clockid_t = 0;
    // SAFETY: the calling thread's CPU-time clock, written to `id`.
    (unsafe { libc::pthread_getcpuclockid(libc::pthread_self(), &mut id) } == 0).then_some(id as u32 as u64)
}

#[cfg(target_os = "linux")]
fn thread_cpu_ns(os: u64) -> Option<(u64, Option<bool>)> {
    let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: a thread's CPU-time clock id; the call fails once the thread is gone. Exact for a running thread too.
    (unsafe { libc::clock_gettime(os as u32 as libc::clockid_t, &mut t) } == 0).then(|| (t.tv_sec as u64 * 1_000_000_000 + t.tv_nsec as u64, None))
}

#[cfg(not(any(target_vendor = "apple", target_os = "linux")))]
fn this_thread() -> Option<u64> {
    None
}

#[cfg(not(any(target_vendor = "apple", target_os = "linux")))]
fn thread_cpu_ns(_os: u64) -> Option<(u64, Option<bool>)> {
    None
}
