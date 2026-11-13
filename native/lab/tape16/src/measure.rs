//! Instructions retired and cycles (macOS `proc_pid_rusage`, as `rerail-compiler`'s `retired`), and wall time.

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

#[derive(Clone, Copy, Debug, Default)]
pub struct Cost {
    pub instructions: u64,
    pub cycles: u64,
    pub nanos: u64,
}

impl std::ops::AddAssign for Cost {
    fn add_assign(&mut self, o: Self) {
        self.instructions += o.instructions;
        self.cycles += o.cycles;
        self.nanos += o.nanos;
    }
}

/// What `f` costs, on this thread (the lab measures single-threaded).
pub fn cost<T>(f: impl FnOnce() -> T) -> (T, Cost) {
    let (i0, c0) = retired();
    let t0 = std::time::Instant::now();
    let v = f();
    let nanos = t0.elapsed().as_nanos() as u64;
    let (i1, c1) = retired();
    (v, Cost { instructions: i1 - i0, cycles: c1 - c0, nanos })
}
