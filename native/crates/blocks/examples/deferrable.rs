//! Decision 0295, path step 1 (0260's open Q-R5): can a deferrable slab, a `MAP_SHARED` file mapping wrapped
//! no-copy for the GPU, be written by a kernel, frozen, given to the OS (`msync(MS_ASYNC)`, unmapped), and read back
//! from the file equal? And what does the process's footprint (`phys_footprint`, what the OS charges it) count of
//! those pages: while mapped and dirty, and once unmapped?
//!
//! For comparison, the same with an anonymous block (pages that only swap can take away), and the switches that
//! found 0295 §1's protocol: `REMSYNC=1` (the range unmapped, the file mapped again and `msync`ed: the GPU's pages
//! written back), `DEALLOC=1` (a wrap with a deallocator), `PWRITE=1` (a bounce buffer), `TOUCH`, `KEEP`, `MSYNC`,
//! `RELEASE`, `POOL`, `WAIT`.
//!
//! ```text
//! cargo run -p rerail-blocks --profile profiling --example deferrable -- DIR
//! ```

#[cfg(target_os = "macos")]
fn main() {
    use rerail_blocks::block::{Block, kind};
    use rerail_device::device::Buffer;
    // What the OS charges the process (`phys_footprint`) and what it has resident, in bytes.
    let footprint = || unsafe {
        let mut info: libc::rusage_info_v4 = std::mem::zeroed();
        libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V4, (&raw mut info).cast());
        (info.ri_phys_footprint, info.ri_resident_size)
    };
    use std::time::Instant;

    // The kernel in UKL, on the process's device (`RERAIL_GPU`; decision 0440 step C). It writes `n` words from word
    // `at` of the block's buffer.
    const SRC: &str = r#"
#include <ukl/backend.h>
typedef struct FillP { uint32_t at; uint32_t n; uint32_t seed; uint32_t pad; } FillP;
UKL_REFLECT_STRUCT(FillP, at, n, seed, pad)
DEFINE_KERNEL2(deferrable, fill, RWBUFFER, uint32_t, p, "[block]", ARGS, FillP, a, , GID0, uint32_t, i, )
{
    if (i < a.n) p[a.at + i] = (i * 2654435761u) ^ a.seed;
}
"#;
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct FillP {
        at: u32,
        n: u32,
        seed: u32,
        pad: u32,
    }
    // SAFETY: `#[repr(C)]` u32s.
    unsafe impl rerail_gpu::Pod for FillP {}
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("a directory for the slabs' files"));
    std::fs::create_dir_all(&dir).unwrap();
    // MSYNC=0: no msync, the OS writes back alone; RELEASE=0: the wrap is kept until the block is unmapped.
    let msync = std::env::var("MSYNC").map_or(true, |v| v != "0");
    let release = std::env::var("RELEASE").map_or(true, |v| v != "0");
    // POOL=0: the dispatch is not inside an autorelease pool.
    let touch = std::env::var("TOUCH").is_ok_and(|v| v == "1");
    let touch_after = std::env::var("TOUCH_AFTER").is_ok_and(|v| v == "1");
    let pwrite = std::env::var("PWRITE").is_ok_and(|v| v == "1");
    let remsync = std::env::var("REMSYNC").is_ok_and(|v| v == "1");
    let dealloc = std::env::var("DEALLOC").is_ok_and(|v| v == "1");
    let keep = std::env::var("KEEP").is_ok_and(|v| v == "1");
    let wait_s: u64 = std::env::var("WAIT").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
    let pool = std::env::var("POOL").map_or(true, |v| v != "0");
    println!("msync {msync}, wrap released before freeze {release}, dispatch in an autorelease pool {pool}, pages touched by the host {touch}");
    let shared = rerail_gpu::shared::get().expect("a device");
    let fill = rerail_gpu::kernels::program("deferrable.uglykl", SRC).unwrap_or_else(|e| panic!("{e}")).function("fill").unwrap();
    // One dispatch of `n` words from byte `off` of `buf`, waited for.
    let dispatch = |buf: &Buffer, off: usize, n: u64, seed: u32| {
        let mut q = rerail_gpu::device::ComputeQueue::new(&shared.queue);
        rerail_gpu::kernels::launch(&q, fill)
            .buffer("p", buf)
            .value("a", &FillP { at: (off / 4) as u32, n: n as u32, seed, pad: 0 })
            .run(&[n.div_ceil(256)], &[256])
            .unwrap_or_else(|e| panic!("{e}"));
        q.flush().await_().unwrap();
    };
    let ms = |t: Instant| t.elapsed().as_secs_f64() * 1e3;
    let mb = |a: (u64, u64), b: (u64, u64)| {
        let d = |x: u64, y: u64| (x as i64 - y as i64) as f64 / (1 << 20) as f64;
        format!("{:.0}/{:.0}", d(a.0, b.0), d(a.1, b.1))
    };
    let expect = |i: u32, seed: u32| i.wrapping_mul(2654435761) ^ seed;

    println!(
        "{:>6} {:>5} {:>4} {:>8} {:>11} {:>10} {:>9} {:>9} {:>11} {:>9} {:>11} {:>6} {:>6} {:>30} {:>10} {:>9}",
        "MiB", "kind", "by", "fill ms", "fill fp/rss", "freeze ms", "msync ms", "unmap ms", "unmap fp/rss", "check ms", "check fp/rss", "equal", "dirty", "written back", "pwrite ms", "freed ms"
    );
    println!("(fp/rss: MiB more than before the block was made: phys_footprint / resident size)");
    // Warm the pipeline and the driver once, so the first row does not carry their memory.
    {
        let w = Block::anon(kind::JOURNAL, 1 << 20);
        w.alloc(1 << 16, 64);
        dispatch(&w.device_buffer(&shared.device).unwrap(), 0, 1 << 14, 0);
    }
    for &mib in &[64usize, 256, 1024] {
        for (file, gpu) in if pwrite { vec![(false, true), (false, false)] } else { vec![(true, true), (true, false)] } {
            let seed = 0x9e37_79b9u32 ^ mib as u32;
            let len = mib << 20;
            let path = dir.join(format!("seg-{mib}-{}.blk", if gpu { "gpu" } else { "cpu" }));
            let f0 = footprint();
            let b = if file { Block::file(&path, kind::JOURNAL, len + (1 << 16)).unwrap() } else { Block::anon(kind::JOURNAL, len + (1 << 16)) };
            let off = b.alloc(len, 64);
            let n = (len / 4) as u64;
            let t = Instant::now();
            if !gpu {
                let p = b.ptr::<u32>(off);
                for i in 0..n as u32 {
                    unsafe { *p.add(i as usize) = expect(i, seed) };
                }
            }
            // The wrap covers the used extent, so allocate first, then bind at the range's offset. DEALLOC=1: the wrap is
            // made here with a deallocator, which Metal calls once it is done with the memory: a question about Metal's
            // own wraps, so Metal's alone.
            let freed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let buf = if dealloc {
                let device = shared.device.metal().expect("DEALLOC=1: a wrap with Metal's deallocator, on Metal");
                let f = freed.clone();
                let d = block::ConcreteBlock::new(move |_p: *const std::ffi::c_void, _n: u64| {
                    f.store(true, std::sync::atomic::Ordering::Release);
                })
                .copy();
                let wrap_len = b.used().div_ceil(16384) * 16384;
                Buffer::Native(device.new_buffer_with_bytes_no_copy(b.base_ptr().cast(), wrap_len as u64, metal::MTLResourceOptions::StorageModeShared, Some(&d)))
            } else {
                b.device_buffer(&shared.device).unwrap()
            };
            // The command buffer is autoreleased, and it retains the buffers it binds: without a pool around it, the
            // wrap (and the pages it holds) outlives every handle the block lets go of.
            if gpu && pool {
                metal::objc::rc::autoreleasepool(|| dispatch(&buf, off, n, seed));
            } else if gpu {
                dispatch(&buf, off, n, seed);
            }
            // TOUCH=1: the host writes one word of each page back to itself after the kernel, so the VM marks the page
            // dirty (the device's writes through the wrap do not).
            if gpu && touch && !touch_after {
                let p = b.ptr::<u8>(off & !16383);
                for pg in (0..len + (off & 16383)).step_by(16384) {
                    unsafe { std::ptr::write_volatile(p.add(pg), std::ptr::read_volatile(p.add(pg))) };
                }
            }
            let fill = ms(t);
            let f1 = footprint();
            drop(buf);
            // The pages the VM sees as modified (by this process or another), of those the range spans.
            let dirty = {
                let page = 16384usize;
                let start = (b.base_ptr() as usize + off) & !(page - 1);
                let end = b.base_ptr() as usize + off + len;
                let pages = (end - start).div_ceil(page);
                let mut v = vec![0 as libc::c_char; pages];
                let r = unsafe { libc::mincore(start as *const libc::c_void, end - start, v.as_mut_ptr()) };
                assert_eq!(r, 0, "mincore");
                let modified = v.iter().filter(|&&c| c as i32 & (libc::MINCORE_MODIFIED | libc::MINCORE_MODIFIED_OTHER) != 0).count();
                format!("{:.0}%", 100.0 * modified as f64 / pages as f64)
            };

            if release {
                b.release_device_buffer();
            }
            // With a deallocator: wait (5 s at most) until Metal has let go of the memory.
            let mut freed_ms = String::from("-");
            if dealloc && gpu {
                let w = Instant::now();
                while !freed.load(std::sync::atomic::Ordering::Acquire) && w.elapsed().as_secs() < 5 {
                    std::thread::sleep(std::time::Duration::from_micros(200));
                }
                freed_ms = if freed.load(std::sync::atomic::Ordering::Acquire) { format!("{:.1}", ms(w)) } else { "never".into() };
            }
            // TOUCH_AFTER=1: the touch is made once the wrap is gone.
            if gpu && touch && touch_after {
                let p = b.ptr::<u8>(off & !16383);
                for pg in (0..len + (off & 16383)).step_by(16384) {
                    unsafe { std::ptr::write_volatile(p.add(pg), std::ptr::read_volatile(p.add(pg))) };
                }
            }
            let t = Instant::now();
            let frozen = b.freeze();
            let freeze = ms(t);
            let t = Instant::now();
            if file && msync {
                let r = unsafe { libc::msync(frozen.base_ptr().cast(), frozen.used(), libc::MS_ASYNC) };
                assert_eq!(r, 0, "msync");
            }
            let sync = ms(t);
            // PWRITE=1: an anonymous block is written to the file by `pwrite` once frozen (a bounce buffer: the kernel's
            // page cache takes a copy, and the OS writes it back), and checked from the file as a file block is.
            let mut pw = String::from("-");
            if pwrite && !file {
                let t = Instant::now();
                let f = std::fs::File::create(&path).unwrap();
                std::os::unix::fs::FileExt::write_all_at(&f, frozen.bytes(0, frozen.used()), 0).unwrap();
                pw = format!("{:.1}", ms(t));
            }
            let from_file = file || pwrite;
            // A check of the anonymous block reads it in place, before it is unmapped.
            let anon_equal = (!from_file).then(|| {
                let p = frozen.ptr::<u32>(off);
                (0..n as u32).all(|i| unsafe { *p.add(i as usize) } == expect(i, seed))
            });
            let t = Instant::now();
            drop(frozen);
            let unmap = ms(t);
            let f2 = footprint();

            let mut clean = String::from("-");
            let t = Instant::now();
            let equal = match anon_equal {
                Some(e) => e,
                None => {
                    let back = Block::open_file(&path, kind::JOURNAL).unwrap();
                    let p = back.ptr::<u32>(off);
                    let eq = (0..n as u32).all(|i| unsafe { *p.add(i as usize) } == expect(i, seed));
                    // Written back: no page of the file still modified (polled for 20 s at most).
                    let page = 16384usize;
                    let pages = back.used().div_ceil(page);
                    let mut v = vec![0 as libc::c_char; pages];
                    let w = Instant::now();
                    loop {
                        // REMSYNC=1: the fresh mapping (the first was unmapped) is msync'd each round.
                        if remsync {
                            unsafe { libc::msync(back.base_ptr().cast(), back.used(), libc::MS_ASYNC) };
                        }
                        let r = unsafe { libc::mincore(back.base_ptr() as *const libc::c_void, back.used(), v.as_mut_ptr()) };
                        assert_eq!(r, 0, "mincore");
                        let left = v.iter().filter(|&&c| c as i32 & (libc::MINCORE_MODIFIED | libc::MINCORE_MODIFIED_OTHER) != 0).count();
                        if left == 0 || w.elapsed().as_secs() >= wait_s {
                            let blocks = std::fs::metadata(&path).map(|m| std::os::unix::fs::MetadataExt::blocks(&m)).unwrap_or(0);
                            clean = format!("{} ms, {left} left, {} MiB on disk", w.elapsed().as_millis(), (blocks * 512) >> 20);
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    eq
                }
            };
            let check = ms(t);
            let f3 = footprint();
            println!(
                "{mib:>6} {:>5} {:>4} {fill:>8.1} {:>11} {freeze:>10.1} {sync:>9.1} {unmap:>9.1} {:>11} {check:>9.1} {:>11} {equal:>6} {dirty:>6} {clean:>30} {pw:>10} {freed_ms:>9}",
                if file { "file" } else { "anon" },
                if gpu { "gpu" } else { "cpu" },
                mb(f1, f0),
                mb(f2, f0),
                mb(f3, f0),
            );
            // KEEP=1: the files stay, for a look at what reached the disk after the process ends.
            if !keep {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {}
