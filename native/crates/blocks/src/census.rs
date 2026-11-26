//! A census of the run's rooms' regions (`RERAIL_ANON_CENSUS=path`): the threads' stacks', their large rooms' and their
//! long-lived rooms' (`STACK`, `LARGE`, `LONG`). For each region its type, bytes, room, the stack that asked for it, the
//! thread, and when it went (in regions handed out since, and in time). Events: `[region, thread << 3 | room << 1 |
//! gone, bytes | site << 40]`. Written to the
//! path at exit, the stacks as raw addresses with the image's slide, for `atos`. A measuring tool, off by default.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex, OnceLock};

const DEPTH: usize = 28;

#[derive(Default)]
struct Site {
    name: &'static str,
    no: u64,
    stack: Vec<usize>,
    n: u64,
    bytes: u64,
    freed: u64,
    freed_bytes: u64,
    same_thread: u64,
    /// Freed regions by log2 of the regions handed out while they lived.
    life_regions: [u64; 32],
    /// Freed regions by log2 of the microseconds they lived.
    life_us: [u64; 32],
}

struct Census {
    path: String,
    seq: AtomicU64,
    t0: std::time::Instant,
    sites: Mutex<HashMap<(usize, u64), Site>>,
    /// Every take and give in order, under `sites`' lock: (region, thread << 1 | gone, bytes).
    events: Mutex<Vec<[u64; 3]>>,
}

fn census() -> Option<&'static Census> {
    static C: OnceLock<Option<Census>> = OnceLock::new();
    C.get_or_init(|| {
        let path = std::env::var("RERAIL_ANON_CENSUS").ok()?;
        extern "C" fn dump() {
            if let Some(c) = census() {
                c.write();
            }
        }
        // SAFETY: registers a plain function to run at exit.
        unsafe { libc::atexit(dump) };
        Some(Census { path, seq: AtomicU64::new(0), t0: std::time::Instant::now(), sites: Mutex::new(HashMap::new()), events: Mutex::new(Vec::new()) })
    })
    .as_ref()
}

fn thread_id() -> u64 {
    thread_local! { static ID: u64 = { static N: AtomicU64 = AtomicU64::new(1); N.fetch_add(1, Relaxed) }; }
    ID.with(|i| *i)
}

/// The token a counted region holds: its going is recorded.
pub(crate) struct Token {
    key: (usize, u64),
    bytes: u64,
    seq: u64,
    thread: u64,
    kind: u64,
    at: std::time::Instant,
}

impl Drop for Token {
    fn drop(&mut self) {
        let Some(c) = census() else { return };
        let lived = c.seq.load(Relaxed).saturating_sub(self.seq).max(1);
        let us = (self.at.elapsed().as_micros() as u64).max(1);
        let mut s = c.sites.lock().unwrap_or_else(|e| e.into_inner());
        c.events.lock().unwrap_or_else(|e| e.into_inner()).push([self.seq, thread_id() << 3 | self.kind << 1 | 1, self.bytes]);
        if let Some(site) = s.get_mut(&self.key) {
            site.freed += 1;
            site.freed_bytes += self.bytes;
            if thread_id() == self.thread {
                site.same_thread += 1;
            }
            site.life_regions[(63 - lived.leading_zeros()) as usize] += 1;
            site.life_us[(63 - us.leading_zeros()) as usize] += 1;
        }
    }
}

/// The rooms a region of the census lies in (its events' `kind`).
pub(crate) const STACK: u64 = 0;
pub(crate) const LARGE: u64 = 1;
pub(crate) const LONG: u64 = 3;

/// A region of `bytes` for `name` handed out now from a room of `kind`: its token, when the census runs.
pub(crate) fn counted(kind: u64, name: &'static str, bytes: usize) -> Option<Arc<dyn std::any::Any + Send + Sync>> {
    let c = census()?;
    let mut buf = [std::ptr::null_mut::<libc::c_void>(); DEPTH];
    // SAFETY: a buffer of DEPTH frames.
    let n = unsafe { libc::backtrace(buf.as_mut_ptr(), DEPTH as libc::c_int) }.max(0) as usize;
    let stack: Vec<usize> = buf[..n].iter().map(|p| *p as usize).collect();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &a in &stack {
        h = (h ^ a as u64).wrapping_mul(0x0100_0000_01b3);
    }
    let key = (name.as_ptr() as usize, h);
    let seq = c.seq.fetch_add(1, Relaxed);
    {
        let mut s = c.sites.lock().unwrap_or_else(|e| e.into_inner());
        let no = s.len() as u64;
        let site = s.entry(key).or_insert_with(|| Site { name, no, stack, ..Default::default() });
        c.events.lock().unwrap_or_else(|e| e.into_inner()).push([seq, thread_id() << 3 | kind << 1, bytes as u64 | site.no << 40]);
        site.n += 1;
        site.bytes += bytes as u64;
    }
    Some(Arc::new(Token { key, bytes: bytes as u64, seq, thread: thread_id(), kind, at: std::time::Instant::now() }))
}

impl Census {
    fn write(&self) {
        use std::fmt::Write;
        let s = self.sites.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: image 0 is the program.
        let slide = unsafe { libc::_dyld_get_image_vmaddr_slide(0) };
        let mut out = format!("slide\t{slide:#x}\nregions\t{}\nelapsed_us\t{}\n", self.seq.load(Relaxed), self.t0.elapsed().as_micros());
        for site in s.values() {
            let _ = write!(out, "site\t{}\t{}\t{}\t{}\t{}\t{}", site.name, site.n, site.bytes, site.freed, site.freed_bytes, site.same_thread);
            let _ = write!(out, "\tlr={}", site.life_regions.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","));
            let _ = write!(out, "\tlu={}", site.life_us.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","));
            let _ = write!(out, "\tno={}", site.no);
            let _ = writeln!(out, "\tst={}", site.stack.iter().map(|a| format!("{a:#x}")).collect::<Vec<_>>().join(","));
        }
        let _ = std::fs::write(&self.path, out);
        let ev = self.events.lock().unwrap_or_else(|e| e.into_inner());
        let bytes: Vec<u8> = ev.iter().flat_map(|e| e.iter().flat_map(|w| w.to_le_bytes())).collect();
        let _ = std::fs::write(format!("{}.events", self.path), bytes);
    }
}
