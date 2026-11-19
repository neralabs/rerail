//! Instructions retired and cycles (macOS `proc_pid_rusage`, as `rerail-compiler`'s `retired`; Linux
//! `perf_event_open`, user space only, 0 where the machine gives no counters), and wall time.

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
    #[cfg(target_os = "linux")]
    {
        return linux::retired();
    }
    #[allow(unreachable_code)]
    (0, 0)
}

#[cfg(target_os = "linux")]
mod linux {
    /// This thread's counters (instructions, cycles), opened once; -1 where the kernel refuses one.
    fn open(config: u64) -> i32 {
        // perf_event_attr: type @0, size @4, config @8, flags @40 (exclude_kernel bit 5, exclude_hv bit 6).
        let mut attr = [0u8; 128];
        attr[0..4].copy_from_slice(&0u32.to_ne_bytes()); // PERF_TYPE_HARDWARE
        attr[4..8].copy_from_slice(&128u32.to_ne_bytes());
        attr[8..16].copy_from_slice(&config.to_ne_bytes());
        attr[40..48].copy_from_slice(&((1u64 << 5) | (1u64 << 6)).to_ne_bytes());
        // SAFETY: a valid attribute block; pid 0 (this thread), any CPU, no group, no flags.
        unsafe { libc::syscall(libc::SYS_perf_event_open, attr.as_ptr(), 0, -1, -1, 0) as i32 }
    }
    fn read(fd: i32) -> u64 {
        if fd < 0 {
            return 0;
        }
        let mut v = 0u64;
        // SAFETY: eight bytes into a u64.
        let n = unsafe { libc::read(fd, (&raw mut v).cast(), 8) };
        if n == 8 { v } else { 0 }
    }
    thread_local! {
        // PERF_COUNT_HW_INSTRUCTIONS = 1, PERF_COUNT_HW_CPU_CYCLES = 0.
        static FDS: (i32, i32) = (open(1), open(0));
    }
    pub fn retired() -> (u64, u64) {
        FDS.with(|&(i, c)| (read(i), read(c)))
    }
}

/// The machine and the build, for a results file: CPU, OS, architecture, the target features compiled in.
pub fn machine() -> String {
    let cpu = {
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("sysctl").args(["-n", "machdep.cpu.brand_string"]).output().ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
        }
        #[cfg(not(target_os = "macos"))]
        {
            std::fs::read_to_string("/proc/cpuinfo").ok()
                .and_then(|s| s.lines().find(|l| l.starts_with("model name")).map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string()))
                .unwrap_or_default()
        }
    };
    let mut feats = Vec::new();
    macro_rules! f {
        ($($name:tt),*) => { $( if cfg!(target_feature = $name) { feats.push($name); } )* };
    }
    #[cfg(target_arch = "x86_64")]
    f!("popcnt", "lzcnt", "bmi1", "bmi2", "avx2", "avx512f");
    #[cfg(target_arch = "aarch64")]
    f!("neon", "lse", "sve");
    let translated = {
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        {
            // Rosetta: sysctl.proc_translated is 1 for a translated process.
            std::process::Command::new("sysctl").args(["-n", "sysctl.proc_translated"]).output().ok()
                .is_some_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
        }
        #[cfg(not(all(target_os = "macos", target_arch = "x86_64")))]
        {
            false
        }
    };
    format!("machine: {cpu}; {} {}{}; built with: {}", std::env::consts::OS, std::env::consts::ARCH,
        if translated { " (translated by Rosetta: timings are not this CPU's)" } else { "" },
        if feats.is_empty() { "baseline".to_string() } else { feats.join(", ") })
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
