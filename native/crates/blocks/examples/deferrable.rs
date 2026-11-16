//! Decision 0295, path step 1 (0260's open Q-R5): can a deferrable slab, a `MAP_SHARED` file mapping wrapped
//! no-copy for the GPU, be written by a kernel, frozen, given to the OS (`msync(MS_ASYNC)`, unmapped), and read back
//! from the file equal? And what does the process's footprint (`phys_footprint`, what the OS charges it) count of
//! those pages: while mapped and dirty, and once unmapped?
//!
//! For comparison, the same with an anonymous block (pages that only swap can take away).
//!
//! ```text
//! cargo run -p rerail-blocks --profile profiling --example deferrable -- DIR
//! ```

#[cfg(target_os = "macos")]
fn main() {
    use metal::*;
    use rerail_blocks::block::{Block, kind};
    // What the OS charges the process (`phys_footprint`) and what it has resident, in bytes.
    let footprint = || unsafe {
        let mut info: libc::rusage_info_v4 = std::mem::zeroed();
        libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V4, (&raw mut info).cast());
        (info.ri_phys_footprint, info.ri_resident_size)
    };
    use std::time::Instant;

    const SRC: &str = "kernel void fill(device uint *p [[buffer(0)]], constant uint &seed [[buffer(1)]], \
                       uint i [[thread_position_in_grid]]) { p[i] = (i * 2654435761u) ^ seed; }";
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("a directory for the slabs' files"));
    std::fs::create_dir_all(&dir).unwrap();
    // MSYNC=0: no msync, the OS writes back alone; RELEASE=0: the wrap is kept until the block is unmapped.
    let msync = std::env::var("MSYNC").map_or(true, |v| v != "0");
    let release = std::env::var("RELEASE").map_or(true, |v| v != "0");
    // POOL=0: the dispatch is not inside an autorelease pool.
    let touch = std::env::var("TOUCH").is_ok_and(|v| v == "1");
    let touch_after = std::env::var("TOUCH_AFTER").is_ok_and(|v| v == "1");
    let wait_s: u64 = std::env::var("WAIT").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
    let pool = std::env::var("POOL").map_or(true, |v| v != "0");
    println!("msync {msync}, wrap released before freeze {release}, dispatch in an autorelease pool {pool}, pages touched by the host {touch}");
    let device = Device::system_default().expect("a Metal device");
    let lib = device.new_library_with_source(SRC, &CompileOptions::new()).unwrap();
    let pipe = device.new_compute_pipeline_state_with_function(&lib.get_function("fill", None).unwrap()).unwrap();
    let queue = device.new_command_queue();
    let ms = |t: Instant| t.elapsed().as_secs_f64() * 1e3;
    let mb = |a: (u64, u64), b: (u64, u64)| {
        let d = |x: u64, y: u64| (x as i64 - y as i64) as f64 / (1 << 20) as f64;
        format!("{:.0}/{:.0}", d(a.0, b.0), d(a.1, b.1))
    };
    let expect = |i: u32, seed: u32| i.wrapping_mul(2654435761) ^ seed;

    println!(
        "{:>6} {:>5} {:>4} {:>8} {:>11} {:>10} {:>9} {:>9} {:>11} {:>9} {:>11} {:>6} {:>6} {:>30}",
        "MiB", "kind", "by", "fill ms", "fill fp/rss", "freeze ms", "msync ms", "unmap ms", "unmap fp/rss", "check ms", "check fp/rss", "equal", "dirty", "written back"
    );
    println!("(fp/rss: MiB more than before the block was made: phys_footprint / resident size)");
    // Warm the pipeline and the driver once, so the first row does not carry their memory.
    {
        let w = Block::anon(kind::JOURNAL, 1 << 20);
        w.alloc(1 << 16, 64);
        let buf = w.gpu_buffer(&device);
        let cmd = queue.new_command_buffer();
        let enc = cmd.new_compute_command_encoder();
        enc.set_compute_pipeline_state(&pipe);
        enc.set_buffer(0, Some(&buf), 0);
        let seed = 0u32;
        enc.set_bytes(1, 4, (&raw const seed).cast());
        enc.dispatch_threads(MTLSize::new(1 << 14, 1, 1), MTLSize::new(256, 1, 1));
        enc.end_encoding();
        cmd.commit();
        cmd.wait_until_completed();
    }
    for &mib in &[64usize, 256, 1024] {
        for (file, gpu) in [(true, true), (true, false)] {
            let seed = 0x9e37_79b9u32 ^ mib as u32;
            let len = mib << 20;
            let path = dir.join(format!("seg-{mib}.blk"));
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
            // The wrap covers the used extent, so allocate first, then bind at the range's offset.
            let buf = b.gpu_buffer(&device);
            // The command buffer is autoreleased, and it retains the buffers it binds: without a pool around it, the
            // wrap (and the pages it holds) outlives every handle the block lets go of.
            if gpu && pool {
                metal::objc::rc::autoreleasepool(|| {
                    let cmd = queue.new_command_buffer();
                    let enc = cmd.new_compute_command_encoder();
                    enc.set_compute_pipeline_state(&pipe);
                    enc.set_buffer(0, Some(&buf), off as u64);
                    enc.set_bytes(1, 4, (&raw const seed).cast());
                    enc.dispatch_threads(MTLSize::new(n, 1, 1), MTLSize::new(256, 1, 1));
                    enc.end_encoding();
                    cmd.commit();
                    cmd.wait_until_completed();
                });
            } else if gpu {
            let cmd = queue.new_command_buffer();
            let enc = cmd.new_compute_command_encoder();
            enc.set_compute_pipeline_state(&pipe);
            enc.set_buffer(0, Some(&buf), off as u64);
            enc.set_bytes(1, 4, (&raw const seed).cast());
            enc.dispatch_threads(MTLSize::new(n, 1, 1), MTLSize::new(256, 1, 1));
            enc.end_encoding();
            cmd.commit();
            cmd.wait_until_completed();
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
                b.release_gpu_buffer();
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
            // A check of the anonymous block reads it in place, before it is unmapped.
            let anon_equal = (!file).then(|| {
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
                        let r = unsafe { libc::mincore(back.base_ptr() as *const libc::c_void, back.used(), v.as_mut_ptr()) };
                        assert_eq!(r, 0, "mincore");
                        let left = v.iter().filter(|&&c| c as i32 & (libc::MINCORE_MODIFIED | libc::MINCORE_MODIFIED_OTHER) != 0).count();
                        if left == 0 || w.elapsed().as_secs() >= wait_s {
                            let blocks = std::fs::metadata(&path).map(|m| std::os::unix::fs::MetadataExt::blocks(&m)).unwrap_or(0);
                            clean = format!("{} ms, {left} left, {} MiB on disk", w.elapsed().as_millis(), blocks * 512 >> 20);
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
                "{mib:>6} {:>5} {:>4} {fill:>8.1} {:>11} {freeze:>10.1} {sync:>9.1} {unmap:>9.1} {:>11} {check:>9.1} {:>11} {equal:>6} {dirty:>6} {clean:>30}",
                if file { "file" } else { "anon" },
                if gpu { "gpu" } else { "cpu" },
                mb(f1, f0),
                mb(f2, f0),
                mb(f3, f0),
            );
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {}
