//! Format 16's parent read of every node of a document, its three versions timed: the host's read (the scalar
//! reference, `reads::f16::parent`), the hand-vectorized NEON read and rerail-ukl-vec's Rust of `f16_parent` at 8
//! lanes, and the hand-vectorized read of every node from the answers before it (`hand::parent_all_scan`, not the
//! kernel's algorithm). Each version writes a fresh vector of the answers; the fastest of `REPS` is printed.
//!
//! `cargo run -p rerail-ukl-aot --profile profiling --example parent_bench [records|numbers|random] [SIZE]`

use rerail_ukl_aot::parent::{self, Doc};
use std::time::{Duration, Instant};

const REPS: usize = 7;

fn time(name: &str, d: &Doc, want: &[u32], mut run: impl FnMut() -> Vec<u32>) {
    let mut best = Duration::MAX;
    let mut got = Vec::new();
    for _ in 0..REPS {
        let t = Instant::now();
        got = run();
        best = best.min(t.elapsed());
    }
    assert!(got == want, "{name}: wrong parents");
    let per = best.as_secs_f64() * 1e9 / f64::from(d.nodes());
    println!("{name:>10}: {:>9.3} ms, {per:.2} ns a node", best.as_secs_f64() * 1e3);
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let kind = a.first().map_or("records", String::as_str);
    let size: u32 = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(200_000);
    let text = match kind {
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
    };
    let lang = parent::json_lang();
    let d = parent::json_doc(&lang, kind, text.as_bytes());
    println!("{kind}: {} bytes of JSON, {} nodes, slot width {}", text.len(), d.nodes(), d.view.widths & 15);
    // One version only, 20 times (for a count of its instructions and cycles; `none`: the setup alone).
    if let Ok(only) = std::env::var("PARENT_ONLY") {
        let mut out = vec![0u32; d.nodes() as usize];
        for _ in 0..20 {
            match only.as_str() {
                "host" => out = parent::scalar(&d),
                #[cfg(target_arch = "aarch64")]
                "hand" => unsafe { rerail_ukl_aot::hand::parent_all(&d.bytes, &d.view, &mut out) },
                #[cfg(target_arch = "aarch64")]
                "scan" => unsafe { rerail_ukl_aot::hand::parent_all_scan(&d.bytes, &d.view, &mut out) },
                #[cfg(target_arch = "aarch64")]
                "ukl" => out = unsafe { parent::run(rerail_ukl_aot::parent_group_w8::parent_group_w8, &d, 256) },
                #[cfg(target_arch = "aarch64")]
                "two" => out = unsafe { parent::run_two(rerail_ukl_aot::parent_step_w8::parent_step_w8, rerail_ukl_aot::parent_link_w8::parent_link_w8, &d, 256) }.unwrap(),
                #[cfg(target_arch = "aarch64")]
                "back" => out = unsafe { parent::run_back(rerail_ukl_aot::parent_back_w8::parent_back_w8, &d, 256) },
                _ => {}
            }
            std::hint::black_box(&out);
        }
        return;
    }
    let want = parent::scalar(&d);
    time("host", &d, &want, || parent::scalar(&d));
    #[cfg(target_arch = "aarch64")]
    {
        time("hand", &d, &want, || {
            let mut out = vec![0u32; d.nodes() as usize];
            unsafe { rerail_ukl_aot::hand::parent_all(&d.bytes, &d.view, &mut out) };
            out
        });
        time("hand scan", &d, &want, || {
            let mut out = vec![0u32; d.nodes() as usize];
            unsafe { rerail_ukl_aot::hand::parent_all_scan(&d.bytes, &d.view, &mut out) };
            out
        });
        time("ukl w8", &d, &want, || unsafe { parent::run(rerail_ukl_aot::parent_group_w8::parent_group_w8, &d, 256) });
        time("ukl two w8", &d, &want, || unsafe { parent::run_two(rerail_ukl_aot::parent_step_w8::parent_step_w8, rerail_ukl_aot::parent_link_w8::parent_link_w8, &d, 256) }.unwrap());
        time("ukl back w8", &d, &want, || unsafe { parent::run_back(rerail_ukl_aot::parent_back_w8::parent_back_w8, &d, 256) });
    }
}
