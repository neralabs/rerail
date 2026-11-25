//! The scratch survey (note t60): the regions the run's default room gives (every `Vector` and block map made with
//! no room of its own), and the heap's allocations (`heap_alloc`, called by the engine's global allocator): by call
//! site and element type ("heap" for the heap's), how many, how big, how often they grow, how long they live, on
//! which thread they end, and whether they are handed to a relation. On with `RERAIL_PROBE_SCRATCH=<file>`, written
//! at exit; call sites are return addresses off the image's base, symbolized afterwards (`atos`). Off, it costs a
//! region an empty tag and an allocation one load.

use std::cell::Cell;
use std::collections::HashMap;
use std::io::Write as _;
use std::sync::atomic::{AtomicU8, Ordering::Relaxed};
use std::sync::{Arc, Mutex, OnceLock};

const DEPTH: usize = 16;
const BINS: usize = 48;
const SHARDS: usize = 64;

/// The output file, once read: `None`, off.
pub fn on() -> bool {
    path().is_some()
}

/// Whether the probe is on, read without allocating: the global allocator asks it (`heap_alloc`).
#[inline]
pub fn heap_on() -> bool {
    static STATE: AtomicU8 = AtomicU8::new(0);
    match STATE.load(Relaxed) {
        2 => true,
        1 => false,
        _ => {
            unsafe extern "C" {
                fn getenv(name: *const std::ffi::c_char) -> *const std::ffi::c_char;
            }
            // SAFETY: a NUL-terminated name; the value is only tested for presence.
            let v = unsafe { getenv(c"RERAIL_PROBE_SCRATCH".as_ptr()) };
            // SAFETY: `getenv`'s value, when not null, is a NUL-terminated string.
            let set = !v.is_null() && unsafe { *v } != 0;
            STATE.store(if set { 2 } else { 1 }, Relaxed);
            set
        }
    }
}

thread_local! {
    /// This thread is in the probe: the allocations it makes for itself are not counted, and none re-enters it.
    static BUSY: Cell<bool> = const { Cell::new(false) };
}

/// `f`, unless this thread is in the probe already (or its thread-locals are gone).
fn quiet<R>(f: impl FnOnce() -> R) -> Option<R> {
    if BUSY.try_with(|b| b.replace(true)).unwrap_or(true) {
        return None;
    }
    let r = f();
    let _ = BUSY.try_with(|b| b.set(false));
    Some(r)
}

fn path() -> Option<&'static str> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| {
        let p = std::env::var("RERAIL_PROBE_SCRATCH").ok().filter(|p| !p.is_empty());
        if p.is_some() {
            unsafe extern "C" {
                fn atexit(f: extern "C" fn()) -> i32;
            }
            extern "C" fn dump_at_exit() {
                dump();
            }
            unsafe { atexit(dump_at_exit) };
        }
        p
    })
    .as_deref()
}

fn now() -> u64 {
    static T0: OnceLock<std::time::Instant> = OnceLock::new();
    T0.get_or_init(std::time::Instant::now).elapsed().as_nanos() as u64
}

fn thread() -> u32 {
    unsafe extern "C" {
        fn pthread_self() -> usize;
    }
    // The thread's handle, folded: distinct enough to tell a thread from the one that made a region.
    let t = unsafe { pthread_self() } as u64;
    ((t >> 12) ^ (t >> 32)) as u32
}

/// The return addresses up the frame-pointer chain (arm64; elsewhere none).
#[inline(always)]
fn site() -> [usize; DEPTH] {
    let mut out = [0usize; DEPTH];
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let mut fp: usize;
        std::arch::asm!("mov {}, x29", out(reg) fp);
        let mut i = 0;
        while fp != 0 && fp % 8 == 0 && i < DEPTH {
            let ret = *((fp + 8) as *const usize);
            let next = *(fp as *const usize);
            out[i] = ret & 0x0000_ffff_ffff_ffff;
            i += 1;
            if next <= fp || next - fp > 1 << 24 {
                break;
            }
            fp = next;
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    site: [usize; DEPTH],
    ty: &'static str,
}

struct Stats {
    births: i64,
    ends: u64,
    handed: u64,
    cross: u64,
    growths: u64,
    bytes: u64,
    max: u64,
    size: [u64; BINS],
    life: [u64; BINS],
    /// Bytes times nanoseconds lived (a footprint's area), in KB·ms.
    area: f64,
}

impl Default for Stats {
    fn default() -> Self {
        Stats { births: 0, ends: 0, handed: 0, cross: 0, growths: 0, bytes: 0, max: 0, size: [0; BINS], life: [0; BINS], area: 0.0 }
    }
}

fn shards() -> &'static [Mutex<HashMap<Key, Stats>>] {
    static S: OnceLock<Vec<Mutex<HashMap<Key, Stats>>>> = OnceLock::new();
    S.get_or_init(|| (0..SHARDS).map(|_| Mutex::new(HashMap::new())).collect())
}

fn with(key: &Key, f: impl FnOnce(&mut Stats)) {
    let mut m = shards()[thread() as usize % SHARDS].lock().unwrap_or_else(|e| e.into_inner());
    f(m.entry(*key).or_default());
}

fn bin(v: u64) -> usize {
    (64 - v.leading_zeros() as usize).min(BINS - 1)
}

/// A tagged region's record, from its array's first region on.
pub(crate) struct Live {
    key: Key,
    first: u64,
    thread: u32,
    growth: u32,
    bytes: usize,
    handed: bool,
    outgrown: bool,
}

/// A region's tag: `None` unless the probe is on and the default room gave it.
#[derive(Default)]
pub(crate) struct Tag(pub(crate) Option<Box<Live>>);

impl Tag {
    /// A region of `bytes` just given, for `ty`s: its array's first, unless a growth adopts it (`grown`).
    pub(crate) fn born(bytes: usize, ty: &'static str) -> Tag {
        let key = Key { site: site(), ty };
        quiet(|| {
            with(&key, |s| s.births += 1);
            Tag(Some(Box::new(Live { key, first: now(), thread: thread(), growth: 0, bytes, handed: false, outgrown: false })))
        })
        .unwrap_or_default()
    }

    /// `self`, a vector's new region, takes over `old`'s array (its site, birth and growths): the old one ends as
    /// outgrown, not as the array's end.
    pub(crate) fn grown(&mut self, old: &mut Tag) {
        let (Some(new), Some(was)) = (self.0.as_mut(), old.0.as_mut()) else { return };
        quiet(|| with(&new.key, |s| s.births -= 1));
        new.key = was.key;
        new.first = was.first;
        new.thread = was.thread;
        new.growth = was.growth + 1;
        was.outgrown = true;
    }

    pub(crate) fn handed(&mut self) {
        if let Some(l) = self.0.as_mut() {
            l.handed = true;
        }
    }
}

impl Drop for Tag {
    fn drop(&mut self) {
        let Some(l) = self.0.take() else { return };
        if l.outgrown {
            return;
        }
        quiet(|| {
            ended(&l);
            drop(l);
        });
    }
}

/// `l`'s array ended here, now.
fn ended(l: &Live) {
    let lived = now().saturating_sub(l.first);
    let cross = l.thread != thread();
    with(&l.key, |s| {
        s.ends += 1;
        s.handed += l.handed as u64;
        s.cross += cross as u64;
        s.growths += l.growth as u64;
        s.bytes += l.bytes as u64;
        s.max = s.max.max(l.bytes as u64);
        s.size[bin(l.bytes as u64)] += 1;
        s.life[bin(lived)] += 1;
        s.area += l.bytes as f64 / 1024.0 * lived as f64 / 1e6;
    });
}

/// The heap's live allocations, by address, in shards by address (a free may be on another thread).
fn heap() -> &'static [Mutex<HashMap<usize, Live>>] {
    static H: OnceLock<Vec<Mutex<HashMap<usize, Live>>>> = OnceLock::new();
    H.get_or_init(|| (0..SHARDS).map(|_| Mutex::new(HashMap::new())).collect())
}

fn heap_shard(p: usize) -> std::sync::MutexGuard<'static, HashMap<usize, Live>> {
    heap()[(p >> 4).wrapping_mul(0x9E37_79B9) % SHARDS].lock().unwrap_or_else(|e| e.into_inner())
}

/// The heap gave `bytes` at `p` (the global allocator, while `heap_on`).
#[inline(never)]
pub fn heap_alloc(p: usize, bytes: usize) {
    let site = site();
    quiet(|| {
        if path().is_none() {
            return;
        }
        let key = Key { site, ty: "heap" };
        with(&key, |s| s.births += 1);
        heap_shard(p).insert(p, Live { key, first: now(), thread: thread(), growth: 0, bytes, handed: false, outgrown: false });
    });
}

/// The heap moved `old` to `new`, now `bytes` long: the same array, grown (or shrunk).
#[inline(never)]
pub fn heap_realloc(old: usize, new: usize, bytes: usize) {
    let site = site();
    quiet(|| {
        if path().is_none() {
            return;
        }
        let was = heap_shard(old).remove(&old);
        let l = match was {
            Some(mut l) => {
                l.bytes = bytes;
                l.growth += 1;
                l
            }
            None => {
                let key = Key { site, ty: "heap" };
                with(&key, |s| s.births += 1);
                Live { key, first: now(), thread: thread(), growth: 0, bytes, handed: false, outgrown: false }
            }
        };
        heap_shard(new).insert(new, l);
    });
}

/// The heap took back `p`.
#[inline(never)]
pub fn heap_free(p: usize) {
    quiet(|| {
        if let Some(l) = heap_shard(p).remove(&p) {
            ended(&l);
        }
    });
}

/// The default room, its regions tagged.
pub(crate) struct ProbeRoom(pub(crate) Arc<dyn crate::Room>);

impl crate::Room for ProbeRoom {
    fn region(&self, bytes: usize, align: usize, name: &'static str) -> crate::Region {
        let mut r = self.0.region(bytes, align, name);
        r.tag = Tag::born(bytes, name);
        r
    }
}

fn base() -> usize {
    #[repr(C)]
    struct DlInfo {
        fname: *const i8,
        fbase: *mut u8,
        sname: *const i8,
        saddr: *mut u8,
    }
    unsafe extern "C" {
        fn dladdr(addr: *const u8, info: *mut DlInfo) -> i32;
    }
    let mut i = DlInfo { fname: std::ptr::null(), fbase: std::ptr::null_mut(), sname: std::ptr::null(), saddr: std::ptr::null_mut() };
    unsafe { dladdr(base as *const u8, &mut i) };
    i.fbase as usize
}

fn dump() {
    let _ = BUSY.try_with(|b| b.set(true));
    let Some(p) = path() else { return };
    let Ok(mut f) = std::fs::File::create(p) else { return };
    let b = base();
    let _ = writeln!(f, "base\t{b:#x}\texe\t{}", std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default());
    for shard in shards() {
        let m = shard.lock().unwrap_or_else(|e| e.into_inner());
        for (k, s) in m.iter() {
            if s.births == 0 && s.ends == 0 {
                continue;
            }
            let hist = |h: &[u64; BINS]| h.iter().enumerate().filter(|(_, n)| **n > 0).map(|(i, n)| format!("{i}:{n}")).collect::<Vec<_>>().join(",");
            let site = k.site.iter().take_while(|a| **a != 0).map(|a| format!("{:#x}", a.wrapping_sub(b))).collect::<Vec<_>>().join(",");
            let _ = writeln!(f, "row\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.1}\t{}\t{}\t{}", k.ty, s.births, s.ends, s.handed, s.cross, s.growths, s.bytes, s.max, s.area, hist(&s.size), hist(&s.life), site);
        }
    }
}
