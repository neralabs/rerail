//! `ukl_pp metal|cuda|hip|cpu|opencl|vec VERSION MAIN [-I DIR]...`: MAIN preprocessed for the backend (VERSION: Metal's
//! language version, 310; CUDA's compute capability, 121; HIP's wavefront, 32; OpenCL's and the CPU vector code's SIMD width, 32), on stdout. For a kernel's text to hand to a compiler by hand.

use rerail_ukl::{Backend, Preprocess};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let usage = "ukl_pp metal|cuda|hip|cpu|opencl|vec VERSION MAIN [-I DIR]...";
    let v: u32 = a.get(2).and_then(|s| s.parse().ok()).expect(usage);
    let backend = match a.get(1).map(|s| s.as_str()) {
        Some("metal") => Backend::Metal { version: v },
        Some("cuda") => Backend::Cuda { sm: v },
        Some("hip") => Backend::Hip { wave: v },
        Some("cpu") => Backend::Cpu,
        Some("opencl") => Backend::OpenCl,
        Some("vec") => Backend::Vec,
        _ => panic!("{usage}"),
    };
    let main = a.get(3).expect(usage);
    let mut p = Preprocess::new(backend);
    if backend.is_c() {
        p = p.define("UKL_SIMD_WIDTH", Some(&format!("{v}u")));
    }
    let mut k = 4;
    while k + 1 < a.len() && a[k] == "-I" {
        p = p.dir(a[k + 1].clone());
        k += 2;
    }
    match p.run(main) {
        Ok(t) => print!("{t}"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
