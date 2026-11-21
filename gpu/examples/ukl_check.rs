//! Compiles a UKL source for every backend this machine has (decision 0440 step C): the process's own device (Metal
//! on a Mac, CUDA where there is one), the CPU and OpenCL. A port is checked so before anything launches it.
//!
//! ```text
//! cargo run -p rerail-gpu --example ukl_check -- [-D NAME=VALUE]... [-k KERNEL]... FILE...
//! ```
//!
//! The files are joined in order (a kernel file after the pieces it is compiled with), and each `-k` kernel is taken
//! from every backend's library. A backend whose compile fails keeps its preprocessed text in the temporary
//! directory, named in the report.

use rerail_gpu::device::{ComputeFunctionLibrary, Device};
use rerail_gpu::Preprocessing as _;

fn main() {
    let mut defines: Vec<(String, String)> = Vec::new();
    let (mut kernels, mut files) = (Vec::new(), Vec::new());
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-D" => {
                let d = args.next().expect("-D NAME=VALUE");
                let (n, v) = d.split_once('=').unwrap_or((&d, "1"));
                defines.push((n.to_owned(), v.to_owned()));
            }
            "-k" => kernels.push(args.next().expect("-k KERNEL")),
            _ => files.push(a),
        }
    }
    assert!(!files.is_empty(), "usage: ukl_check [-D NAME=VALUE]... [-k KERNEL]... FILE...");
    let text: String = files.iter().map(|f| std::fs::read_to_string(f).unwrap_or_else(|e| panic!("{f}: {e}"))).collect();
    let name = std::path::Path::new(files.last().unwrap()).file_name().unwrap().to_string_lossy().into_owned();
    let path = format!("/<check>/{name}");

    let mut devices: Vec<(&str, Device)> = Vec::new();
    if let Some(shared) = rerail_gpu::shared::get()
        && !shared.device.is_cpu()
        && !matches!(shared.device, Device::OpenCl(_))
    {
        devices.push(("the device", shared.device.clone()));
    }
    if let Some(cpu) = rerail_gpu::compute_kernel_cpu::CpuDevice::system_default() {
        devices.push(("the CPU", Device::Cpu(cpu)));
    }
    if let Ok(cl) = rerail_gpu::compute_kernel_opencl::ClDevice::open() {
        devices.push(("OpenCL", Device::OpenCl(cl)));
    }
    let mut failed = false;
    for (which, device) in &devices {
        let mut p = device.preprocess().file(&path, text.as_str());
        for (n, v) in &defines {
            p = p.define(n, Some(v));
        }
        let pre = match p.run(&path) {
            Ok(pre) => pre,
            Err(e) => {
                println!("{which}: preprocessing: {e}");
                failed = true;
                continue;
            }
        };
        let kept = std::env::temp_dir().join(format!("ukl_check.{}.{}.txt", which.replace(' ', "_"), std::process::id()));
        match ComputeFunctionLibrary::compile_from_source(device, &pre, &name) {
            Ok(lib) => {
                // Metal makes a specialized kernel only with its constant's value, which this check does not give:
                // the library compiled is all it checks there.
                let specialized = *which == "the device" && defines.iter().any(|(n, _)| n == "UKL_CONSTANT_0");
                let wanted: &[String] = if specialized { &[] } else { &kernels };
                let missing: Vec<String> = wanted.iter().filter_map(|k| lib.get_function(device, k).err().map(|e| format!("{k}: {e}"))).collect();
                if missing.is_empty() {
                    println!("{which}: ok");
                } else {
                    let _ = std::fs::write(&kept, &pre);
                    println!("{which}: compiled, but {} (the text: {})", missing.join("; "), kept.display());
                    failed = true;
                }
            }
            Err(e) => {
                let _ = std::fs::write(&kept, &pre);
                println!("{which}: {e}\n(the text: {})", kept.display());
                failed = true;
            }
        }
    }
    std::process::exit(i32::from(failed));
}
