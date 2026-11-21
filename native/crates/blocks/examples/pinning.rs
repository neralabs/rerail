//! What a GPU dispatch over a block costs by how the block is made visible to the device: wrapped afresh for each
//! dispatch (the spike's join), or wrapped once and the buffer kept (a pinned pool, recycled). Each dispatch
//! touches the first 1 MiB only, so any cost past that is the wrapped length's.
//!
//! ```text
//! cargo run -p rerail-blocks --profile profiling --example pinning
//! ```

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn main() {
    use rerail_blocks::block::{Block, kind};
    use rerail_device::device::Buffer;
    use std::time::Instant;

    // The kernel in UKL, on the process's device (`RERAIL_GPU` chooses its backend; decision 0440 step C).
    const SRC: &str = r#"
#include <ukl/backend.h>
DEFINE_KERNEL2(pinning, touch, RWBUFFER, uint32_t, p, "[n]", GID0, uint32_t, i, ) { p[i] += 1u; }
"#;
    let shared = rerail_gpu::shared::get().expect("a device");
    let touch = rerail_gpu::kernels::program("pinning.uglykl", SRC).unwrap_or_else(|e| panic!("{e}")).function("touch").unwrap();
    let threads = (1u64 << 20) / 4;
    let run = |buf: &Buffer| {
        let mut q = rerail_gpu::device::ComputeQueue::new(&shared.queue);
        rerail_gpu::kernels::launch(&q, touch).buffer("p", buf).run(&[threads / 256], &[256]).unwrap_or_else(|e| panic!("{e}"));
        q.flush().await_().unwrap();
    };
    // SAFETY: each block outlives the wraps made of it.
    let wrap = |b: &Block, len: usize| unsafe { rerail_device::device::wrap_host_memory(&shared.device, b.base_ptr(), len) }.expect("a wrap");
    let ms = |t: Instant| t.elapsed().as_secs_f64() * 1e3;
    // Warm the pipeline and the driver once.
    let warm = Block::anon(kind::FACTS, 1 << 20);
    warm.alloc((1 << 20) - (1 << 16), 64);
    run(&wrap(&warm, 1 << 20));

    println!("{:>9} {:>9} {:>11} {:>13} {:>13} {:>12} {:>12}", "wrapped", "touched", "wrap ms", "fresh: ms/op", "kept: ms/op", "kept 1st ms", "footprint Δ");
    for &mib in &[1usize, 16, 64, 256, 1024, 2048] {
        for touched in [false, true] {
            let len = mib << 20;
            let b = Block::anon(kind::FACTS, len);
            let off = b.alloc(len - (1 << 16), 64);
            if touched {
                // Every page written once, as a block filled to that length would be.
                let p = b.ptr::<u8>(off);
                for pg in (0..len - (1 << 16)).step_by(16384) {
                    unsafe { *p.add(pg) = 1 };
                }
            }
            let f0 = rerail_blocks::footprint();
            let t = Instant::now();
            let kept = wrap(&b, len);
            let wrap_ms = ms(t);
            let t = Instant::now();
            run(&kept);
            let first = ms(t);
            let t = Instant::now();
            for _ in 0..10 {
                run(&kept);
            }
            let kept_ms = ms(t) / 10.0;
            let t = Instant::now();
            for _ in 0..10 {
                run(&wrap(&b, len));
            }
            let fresh_ms = ms(t) / 10.0;
            let df = (rerail_blocks::footprint() as i64 - f0 as i64) >> 20;
            println!("{:>7}Mi {:>9} {:>11.3} {:>13.3} {:>13.3} {:>12.3} {:>10}MB", mib, touched, wrap_ms, fresh_ms, kept_ms, first, df);
        }
    }

    // A pool: four slabs wrapped once; each "run" takes the next, resets it, writes 8 MiB and dispatches.
    let slab = 256usize << 20;
    let pool: Vec<(Block, Buffer)> = (0..4)
        .map(|_| {
            let b = Block::anon(kind::FACTS, slab);
            b.alloc(slab - (1 << 16), 64);
            let buf = wrap(&b, slab);
            (b, buf)
        })
        .collect();
    let t = Instant::now();
    for r in 0..40 {
        let (b, buf) = &pool[r % 4];
        unsafe { std::ptr::write_bytes(b.base_ptr().add(1 << 16), 0, 8 << 20) };
        run(buf);
    }
    println!("pool of 4 x 256 MiB slabs, wrapped once, 40 runs recycling them: {:.3} ms a run", ms(t) / 40.0);
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn main() {}
