//! `nvc`: NVRTC alone, a measurement aid for decision 0395 §6's compile costs. It compiles a source the engine dumped
//! (`RERAIL_GPU_DUMP`), or a variant of it, outside the engine, bounded in time and memory.
//!
//! ```text
//! nvc <file.cu> <seconds> <max MB> [options...]
//! ```
//! The dump's first line `// options: ...` gives the options, then those given. Past the seconds or the resident
//! megabytes the process exits (status 124 and 125), so a compile that does not end cannot take the host with it.

#[cfg(target_os = "linux")]
fn main() {
    use std::ffi::{CString, c_char, c_void};
    use std::time::Instant;
    type Ptr = *mut c_void;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (path, seconds, max_mb) = (&args[0], args[1].parse::<f64>().unwrap(), args[2].parse::<u64>().unwrap());
    let text = std::fs::read_to_string(path).unwrap();
    let mut options: Vec<String> = text.lines().next().and_then(|l| l.strip_prefix("// options: ")).unwrap_or("").split_whitespace().map(str::to_owned).collect();
    options.extend(args[3..].iter().cloned());
    let started = Instant::now();
    // The bounds: a watcher of the clock and of the process's resident pages.
    let label = format!("{path} [{}]", options.join(" "));
    let watched = label.clone();
    std::thread::spawn(move || {
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;
        let mut peak = 0;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let rss = std::fs::read_to_string("/proc/self/statm").ok().and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok()).unwrap_or(0) * page >> 20;
            peak = peak.max(rss);
            let s = started.elapsed().as_secs_f64();
            if s > seconds {
                println!("nvc {watched}: not done after {s:.1} s (resident {rss} MB, peak {peak} MB)");
                std::process::exit(124);
            }
            if rss > max_mb {
                println!("nvc {watched}: resident {rss} MB past {max_mb} MB after {s:.1} s");
                std::process::exit(125);
            }
        }
    });
    let lib = ["libnvrtc.so", "libnvrtc.so.13", "libnvrtc.so.12"]
        .iter()
        .map(|n| unsafe { libc::dlopen(CString::new(*n).unwrap().as_ptr(), libc::RTLD_NOW) })
        .find(|p| !p.is_null())
        .expect("no libnvrtc");
    let sym = |n: &str| {
        let p = unsafe { libc::dlsym(lib, CString::new(n).unwrap().as_ptr()) };
        assert!(!p.is_null(), "no {n}");
        p
    };
    unsafe {
        let create: unsafe extern "C" fn(*mut Ptr, *const c_char, *const c_char, i32, *const *const c_char, *const *const c_char) -> i32 = std::mem::transmute(sym("nvrtcCreateProgram"));
        let compile: unsafe extern "C" fn(Ptr, i32, *const *const c_char) -> i32 = std::mem::transmute(sym("nvrtcCompileProgram"));
        let log_size: unsafe extern "C" fn(Ptr, *mut usize) -> i32 = std::mem::transmute(sym("nvrtcGetProgramLogSize"));
        let log: unsafe extern "C" fn(Ptr, *mut c_char) -> i32 = std::mem::transmute(sym("nvrtcGetProgramLog"));
        let cubin_size: unsafe extern "C" fn(Ptr, *mut usize) -> i32 = std::mem::transmute(sym("nvrtcGetCUBINSize"));
        let src = CString::new(text).unwrap();
        let mut prog = std::ptr::null_mut();
        assert_eq!(create(&mut prog, src.as_ptr(), c"nvc.cu".as_ptr(), 0, std::ptr::null(), std::ptr::null()), 0);
        let opts: Vec<CString> = options.iter().map(|o| CString::new(o.as_str()).unwrap()).collect();
        let ptrs: Vec<*const c_char> = opts.iter().map(|o| o.as_ptr()).collect();
        let t = Instant::now();
        let r = compile(prog, ptrs.len() as i32, ptrs.as_ptr());
        let s = t.elapsed().as_secs_f64();
        let mut n = 0;
        cubin_size(prog, &mut n);
        println!("nvc {label}: {r} in {s:.2} s, cubin {n} bytes");
        if r != 0 {
            let mut n = 0;
            log_size(prog, &mut n);
            let mut v = vec![0u8; n.max(1)];
            log(prog, v.as_mut_ptr().cast());
            for l in String::from_utf8_lossy(&v).lines().filter(|l| l.contains("error")).take(10) {
                println!("  {l}");
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("nvc: Linux only (NVRTC)");
}
