//! The label passes' `label_parents` (the engine's batch parents, decision 0475) over one format 16 document: its Rust
//! from rerail-ukl-vec at 8 lanes called directly (`rust`), and the engine's CPU device's run of the same UKL (`cpu`,
//! with `RERAIL_GPU=cpu`): that Rust, which the device runs for this source (`rerail_ukl_rust::kernels`), or with
//! `RERAIL_CPU_RUST=0` its compiler's c++ build (a thread a group's thread). Each runs `LABEL_RUNS` times (20),
//! checked against the host's parents; `LABEL_ONLY=none` runs neither (the setup, to subtract from `/usr/bin/time
//! -l`'s instructions).
//!
//! `RERAIL_GPU=cpu LABEL_ONLY=rust cargo run -p rerail-ukl-aot --profile profiling --example label_bench [records|numbers|random] [SIZE]`

use rerail_ukl_aot::label::{self, Batch};
use rerail_ukl_aot::parent::{self, Doc};

fn runs() -> usize {
    std::env::var("LABEL_RUNS").ok().and_then(|r| r.parse().ok()).unwrap_or(20)
}

fn text(kind: &str, size: u32) -> String {
    match kind {
        "numbers" => format!("[{}]", (0..size).map(|k| k.to_string()).collect::<Vec<_>>().join(",")),
        "random" => {
            let mut g = parent::Gen::new(size.into());
            let mut s = String::from("[");
            for k in 0..size / 64 {
                if k > 0 {
                    s.push(',');
                }
                g.value(&mut s, 6, 6);
            }
            s.push(']');
            s
        }
        _ => parent::records(size),
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let kind = a.first().map_or("records", String::as_str).to_owned();
    let size: u32 = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(200_000);
    let only = std::env::var("LABEL_ONLY").unwrap_or_else(|_| "rust".into());
    let lang = parent::json_lang();
    let d: Doc = parent::json_doc(&lang, &kind, text(&kind, size).as_bytes());
    let want = parent::scalar(&d);
    println!("{kind}: {} nodes", d.nodes());
    let b = Batch::new(&[&d]);
    match only.as_str() {
        "rust" => {
            for _ in 0..runs() {
                let got = unsafe { label::run_parents(rerail_ukl_aot::label_parents_w8::label_parents_w8, &b) };
                assert!(got[0] == want, "rust: not the host's parents");
            }
        }
        "cpu" => {
            use rerail_gpu::kernels::{buffer_of, launch, program_with, read};
            let shared = rerail_gpu::shared::get().expect("a device");
            let p = program_with("label_passes", &label::source(), &[("UKL_CONSTANT_0", "16")]).unwrap_or_else(|e| panic!("label_passes: {e}"));
            let k = p.function("label_parents").unwrap();
            let words = b.words();
            let shape = label::Shape { recursive: words, file_count: 1, ..label::Shape::default() };
            // SAFETY: the views and the shape are plain u32s.
            let (views, shape) = unsafe {
                (std::slice::from_raw_parts(b.views.as_ptr().cast::<u32>(), size_of_val(&b.views[..]) / 4), std::slice::from_raw_parts((&raw const shape).cast::<u8>(), size_of::<label::Shape>()))
            };
            let (vb, cb, pb) = (buffer_of(views).unwrap(), buffer_of(&d.bytes).unwrap(), buffer_of(&vec![0u32; 32 * words as usize]).unwrap());
            let mut q = rerail_gpu::device::ComputeQueue::new(&shared.queue);
            for _ in 0..runs() {
                launch(&q, k).buffer("docs", &vb).buffer("parents", &pb).bytes("shape", shape).buffer("chunk", &cb).run(&[u64::from(words.div_ceil(label::GROUP))], &[u64::from(label::GROUP)]).unwrap();
                q.flush().await_().unwrap();
                let got = read::<u32>(&pb, d.nodes() as usize);
                assert!(got == want, "cpu: not the host's parents");
            }
        }
        _ => {}
    }
}
