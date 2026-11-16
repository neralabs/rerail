//! What a GPU dispatch over a block costs by how the block is made visible to the device: wrapped afresh for each
//! dispatch (the spike's join), or wrapped once and the buffer kept (a pinned pool, recycled). Each dispatch
//! touches the first 1 MiB only, so any cost past that is the wrapped length's.
//!
//! ```text
//! cargo run -p rerail-blocks --profile profiling --example pinning
//! ```

#[cfg(target_os = "macos")]
fn main() {
    use metal::*;
    use rerail_blocks::block::{Block, kind};
    use std::ffi::c_void;
    use std::time::Instant;

    const SRC: &str = "kernel void touch(device uint *p [[buffer(0)]], uint i [[thread_position_in_grid]]) { p[i] += 1; }";
    let device = Device::system_default().expect("a Metal device");
    let lib = device.new_library_with_source(SRC, &CompileOptions::new()).unwrap();
    let pipe = device.new_compute_pipeline_state_with_function(&lib.get_function("touch", None).unwrap()).unwrap();
    let queue = device.new_command_queue();
    let threads = (1u64 << 20) / 4;
    let run = |buf: &BufferRef| {
        let cmd = queue.new_command_buffer();
        let enc = cmd.new_compute_command_encoder();
        enc.set_compute_pipeline_state(&pipe);
        enc.set_buffer(0, Some(buf), 0);
        enc.dispatch_threads(MTLSize::new(threads, 1, 1), MTLSize::new(256, 1, 1));
        enc.end_encoding();
        cmd.commit();
        cmd.wait_until_completed();
    };
    let wrap = |b: &Block, len: usize| {
        device.new_buffer_with_bytes_no_copy(b.base_ptr() as *const c_void, len as u64, MTLResourceOptions::StorageModeShared, None)
    };
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

#[cfg(not(target_os = "macos"))]
fn main() {}
