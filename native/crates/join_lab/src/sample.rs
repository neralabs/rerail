//! `join-lab profile --sample`: the replaying thread's stacks, sampled while it replays, folded for the flame graphs.
//!
//! A thread of the lab's suspends the replaying thread every interval, reads its registers, walks its frame pointers
//! (every frame on Apple's arm64 keeps one), and resumes it; the stack's addresses are copied into a buffer of its own
//! while the thread is suspended (no allocation then: the suspended thread may hold the allocator's lock), and kept
//! once it runs again. Each sample carries the engine the replay ran on then (`tag`). At the end the addresses are
//! symbolized once (`atos -i`, whose lines include the inlined frames, as a profiling build's are), and each sample's
//! frames folded root first (`a;b;c n`).
//!
//! macOS only: Linux hosts have no sampler here (the Sparks have no `perf`); a profile there says so and goes on.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};

/// The most frames a sample keeps.
const FRAMES: usize = 160;

/// A sampler of the thread that made it.
pub struct Sampler {
    stop: Arc<AtomicBool>,
    tag: Arc<AtomicU32>,
    handle: Option<std::thread::JoinHandle<Vec<(u32, Vec<u64>)>>>,
}

impl Sampler {
    /// Samples the calling thread every `every`, while a tag is set (`tag`): none where the system has no sampler.
    pub fn start(every: std::time::Duration) -> Result<Sampler, String> {
        #[cfg(target_vendor = "apple")]
        {
            let target = Target::this();
            let (stop, tag) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicU32::new(0)));
            let (s, t) = (Arc::clone(&stop), Arc::clone(&tag));
            let handle = std::thread::Builder::new().name("join-lab sampler".into()).spawn(move || target.sample(every, &s, &t)).map_err(|e| e.to_string())?;
            return Ok(Sampler { stop, tag, handle: Some(handle) });
        }
        #[allow(unreachable_code)]
        {
            let _ = every;
            Err("no sampler on this system (macOS's only)".into())
        }
    }

    /// Samples from now on are `t`'s (0: none kept).
    pub fn tag(&self, t: u32) {
        self.tag.store(t, Relaxed);
    }

    /// Its samples: each its tag and its addresses, the leaf first.
    pub fn finish(mut self) -> Vec<(u32, Vec<u64>)> {
        self.stop.store(true, Relaxed);
        self.handle.take().map_or_else(Vec::new, |h| h.join().unwrap_or_default())
    }
}

#[cfg(target_vendor = "apple")]
mod mach {
    pub type Port = u32;
    unsafe extern "C" {
        pub fn mach_thread_self() -> Port;
        pub fn thread_suspend(thread: Port) -> i32;
        pub fn thread_resume(thread: Port) -> i32;
        pub fn thread_get_state(thread: Port, flavor: i32, state: *mut u32, count: *mut u32) -> i32;
        pub fn pthread_get_stackaddr_np(t: libc::pthread_t) -> *mut libc::c_void;
        pub fn pthread_get_stacksize_np(t: libc::pthread_t) -> libc::size_t;
    }
    /// `ARM_THREAD_STATE64`: x0..x28, fp, lr, sp, pc, cpsr and a pad, as 68 words.
    pub const ARM_THREAD_STATE64: i32 = 6;
    pub const ARM_THREAD_STATE64_COUNT: u32 = 68;
}

/// The thread sampled: its port, and its stack's bounds (a frame pointer outside them ends the walk).
#[cfg(target_vendor = "apple")]
struct Target {
    port: mach::Port,
    lo: u64,
    hi: u64,
}

#[cfg(target_vendor = "apple")]
impl Target {
    fn this() -> Target {
        // SAFETY: this thread's own port and its stack's bounds, read of itself.
        unsafe {
            let me = libc::pthread_self();
            let hi = mach::pthread_get_stackaddr_np(me) as u64;
            let size = mach::pthread_get_stacksize_np(me) as u64;
            Target { port: mach::mach_thread_self(), lo: hi - size, hi }
        }
    }

    fn sample(&self, every: std::time::Duration, stop: &AtomicBool, tag: &AtomicU32) -> Vec<(u32, Vec<u64>)> {
        let mut out = Vec::with_capacity(1 << 16);
        let mut frames = [0u64; FRAMES];
        while !stop.load(Relaxed) {
            std::thread::sleep(every);
            let t = tag.load(Relaxed);
            if t == 0 {
                continue;
            }
            // SAFETY: the thread is suspended while its registers and stack are read, its stack's words only between
            // its bounds; nothing is allocated meanwhile.
            let n = unsafe {
                if mach::thread_suspend(self.port) != 0 {
                    continue;
                }
                let mut state = [0u64; 34];
                let mut count = mach::ARM_THREAD_STATE64_COUNT;
                let n = if mach::thread_get_state(self.port, mach::ARM_THREAD_STATE64, state.as_mut_ptr().cast(), &mut count) == 0 {
                    let (mut fp, pc) = (state[29], state[32]);
                    frames[0] = pc;
                    let mut n = 1;
                    while n < FRAMES && fp % 8 == 0 && fp >= self.lo && fp + 16 <= self.hi {
                        let (next, ret) = (*(fp as *const u64), *((fp + 8) as *const u64));
                        if ret == 0 {
                            break;
                        }
                        frames[n] = ret;
                        n += 1;
                        if next <= fp {
                            break;
                        }
                        fp = next;
                    }
                    n
                } else {
                    0
                };
                mach::thread_resume(self.port);
                n
            };
            if n > 0 && tag.load(Relaxed) == t {
                out.push((t, frames[..n].to_vec()));
            }
        }
        out
    }
}

/// `samples` folded by tag: each tag's stacks, root first, their names `atos`'s (`exe` loaded at `load`), with how
/// many samples each had. A return address is symbolized one byte back (the call it returns from).
pub fn fold(samples: &[(u32, Vec<u64>)]) -> Result<HashMap<u32, Vec<(String, u64)>>, String> {
    let mut addrs: Vec<u64> = samples.iter().flat_map(|(_, f)| f.iter().enumerate().map(|(i, &a)| if i == 0 { a } else { a - 1 })).collect();
    addrs.sort_unstable();
    addrs.dedup();
    let names = symbolize(&addrs)?;
    let mut by: HashMap<u32, HashMap<String, u64>> = HashMap::new();
    for (t, f) in samples {
        let mut path: Vec<&str> = Vec::new();
        for (i, &a) in f.iter().enumerate().rev() {
            let a = if i == 0 { a } else { a - 1 };
            if let Some(n) = names.get(&a) {
                path.extend(n.iter().rev().map(String::as_str));
            }
        }
        *by.entry(*t).or_default().entry(path.join(";")).or_default() += 1;
    }
    Ok(by.into_iter().map(|(t, m)| {
        let mut v: Vec<(String, u64)> = m.into_iter().collect();
        v.sort();
        (t, v)
    }).collect())
}

/// Each address's frames, innermost first (the inlined ones, then the function they are in), by `atos -i` over this
/// binary: one run, the addresses each followed by a sentinel (`0x1`, which nothing holds, echoed back) that ends its
/// lines.
#[cfg(target_vendor = "apple")]
fn symbolize(addrs: &[u64]) -> Result<HashMap<u64, Vec<String>>, String> {
    use std::io::Write as _;
    unsafe extern "C" {
        fn _dyld_get_image_header(i: u32) -> *const u8;
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // SAFETY: image 0 is the main executable, loaded for the process's life.
    let load = unsafe { _dyld_get_image_header(0) } as u64;
    let mut child = std::process::Command::new("atos")
        .args(["-i", "-o"])
        .arg(&exe)
        .args(["-l", &format!("{load:#x}")])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("atos: {e}"))?;
    let mut input = String::with_capacity(addrs.len() * 24);
    for a in addrs {
        input.push_str(&format!("{a:#x}\n0x1\n"));
    }
    let mut stdin = child.stdin.take().ok_or("atos: no stdin")?;
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let output = child.wait_with_output().map_err(|e| format!("atos: {e}"))?;
    let _ = writer.join();
    let text = String::from_utf8_lossy(&output.stdout);
    let mut out = HashMap::with_capacity(addrs.len());
    let mut lines = text.lines();
    for &a in addrs {
        let mut frames = Vec::new();
        for l in lines.by_ref() {
            if l.trim() == "0x1" {
                break;
            }
            if !l.trim().is_empty() {
                frames.push(clean(l));
            }
        }
        out.insert(a, frames);
    }
    Ok(out)
}

#[cfg(not(target_vendor = "apple"))]
fn symbolize(_: &[u64]) -> Result<HashMap<u64, Vec<String>>, String> {
    Err("no symbolizer on this system".into())
}

/// A frame's name as `atos` gave it, without its binary, its place, or its hash: `a::b::c`.
fn clean(line: &str) -> String {
    let name = line.split(" (in ").next().unwrap_or(line).trim();
    let name = match name.rfind("::h") {
        Some(i) if name.len() - i == 19 && name[i + 3..].bytes().all(|b| b.is_ascii_hexdigit()) => &name[..i],
        _ => name,
    };
    // A folded stack's separator and its count's space are the format's own.
    name.replace(';', ",").replace(' ', "")
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_frames_name_loses_its_binary_place_and_hash() {
        assert_eq!(super::clean("rerail_ukl_rust::join_program::Prog::item::h0123456789abcdef (in join-lab) (join_program.rs:4100)"), "rerail_ukl_rust::join_program::Prog::item");
        assert_eq!(super::clean("main (in join-lab) + 12"), "main");
        assert_eq!(super::clean("0x1000"), "0x1000");
    }
}
