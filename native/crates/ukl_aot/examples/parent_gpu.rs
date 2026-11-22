//! `kernels/f16_parent.uglykl` on the process's GPU (Metal on macOS, CUDA on Linux; `RERAIL_GPU=cpu` the UKL CPU
//! backend), against the host's read on every node: the walk (`parent_all`, each thread its node's walk) and, where
//! the source has them, one step a node and the chase of its links (`parent_step`, then `parent_link`). Each is timed
//! by the GPU's own clock: the median of 9 runs after a first, the tape and view uploaded once before (not timed).
//!
//! `cargo run -p rerail-ukl-aot --profile profiling --example parent_gpu [records|numbers|random] [SIZE]`

use rerail_gpu::kernels::{alloc, buffer_of, launch, program, read};
use rerail_ukl_aot::parent::{self, Doc};

const SOURCE: &str = include_str!("../kernels/f16_parent.uglykl");

#[repr(C)]
#[derive(Clone, Copy)]
struct Args {
    nodes: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
}
// SAFETY: plain u32s.
unsafe impl rerail_gpu::Pod for Args {}

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
    let size: u32 = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(40_000);
    let Some(shared) = rerail_gpu::shared::get() else {
        println!("no device");
        return;
    };
    let backend = rerail_gpu::device::backend();
    let lang = parent::json_lang();
    let d: Doc = parent::json_doc(&lang, &kind, text(&kind, size).as_bytes());
    let n = d.nodes() as usize;
    println!("{backend}: {kind}: {n} nodes, slot width {}", d.view.widths & 15);
    let want = parent::scalar(&d);
    let started = std::time::Instant::now();
    let p = program("f16_parent", SOURCE).unwrap_or_else(|e| panic!("{backend}: f16_parent: {e}"));
    println!("{backend}: compiled in {:.2} s", started.elapsed().as_secs_f64());
    let mut q = rerail_gpu::device::ComputeQueue::new(&shared.queue);
    // The bytes, readable through the word that holds the last one.
    let mut bytes = d.bytes.clone();
    bytes.resize(bytes.len().next_multiple_of(8) + 8, 0);
    // SAFETY: the view is plain u32s.
    let words = unsafe { std::slice::from_raw_parts(std::ptr::from_ref(&d.view).cast::<u32>(), size_of::<rerail_clif::view::TapeView>() / 4) };
    let (tb, vb) = (buffer_of(&bytes).unwrap(), buffer_of(words).unwrap());
    let (out, res) = (alloc::<u32>(n).unwrap(), alloc::<u32>(n).unwrap());
    let groups = [n.div_ceil(256) as u64];
    let args = |_: u32| Args { nodes: n as u32, pad0: 0, pad1: 0, pad2: 0 };
    let median = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    let mut time = |name: &str, run: &dyn Fn(&rerail_gpu::device::ComputeQueue)| {
        let mut t = Vec::new();
        for k in 0..10 {
            run(&q);
            let e = q.flush();
            e.await_().unwrap();
            if k > 0 {
                t.push(e.gpu_seconds() * 1e3);
            }
        }
        let got: Vec<u32> = read(&out, n);
        if let Some(i) = (0..n).find(|&i| got[i] != want[i]) {
            panic!("{backend}: {name}: node {i}'s parent {} (the host's {})", got[i], want[i]);
        }
        let ms = median(t);
        println!("{backend}: {name:>12}: {ms:>8.3} ms, {:.2} ns a node", ms * 1e6 / n as f64);
    };
    if let Ok(f) = p.function("parent_all") {
        time("walk", &|q| {
            launch(q, f).buffer("tape", &tb).buffer("view", &vb).buffer("out", &out).value("args", &args(0)).run(&groups, &[256]).unwrap();
        });
    }
    if let (Ok(step), Ok(link)) = (p.function("parent_step"), p.function("parent_link")) {
        time("step + links", &|q| {
            launch(q, step).buffer("tape", &tb).buffer("view", &vb).buffer("steps", &res).value("args", &args(0)).run(&groups, &[256]).unwrap();
            launch(q, link).buffer("tape", &tb).buffer("view", &vb).buffer("steps", &res).buffer("out", &out).value("args", &args(0)).run(&groups, &[256]).unwrap();
        });
    }
    if let Ok(f) = p.function("parent_group") {
        time("group", &|q| {
            launch(q, f).buffer("tape", &tb).buffer("view", &vb).buffer("out", &out).value("args", &args(0)).run(&groups, &[256]).unwrap();
        });
    }
}
