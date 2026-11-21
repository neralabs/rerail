use super::*;
use rerail_c_lean::pp::{P, PpLexer};
use std::process::Command;

/// `forest/src`, where MLDB's tree is.
fn forest() -> String {
    format!("{}/../../../forest/src", std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"))
}

/// The tokens of preprocessed text, its `#` lines (line markers, pragmas) left out.
fn tokens(text: &str) -> Vec<String> {
    let b = text.as_bytes();
    let mut lx = PpLexer::new(b, Options::cpp(20, true));
    let mut v = Vec::new();
    loop {
        let t = lx.next_token();
        if t.kind == P::Eof {
            break;
        }
        if t.kind == P::Hash && t.flags & rerail_c_lean::pp::BOL != 0 {
            lx.skip_line();
            continue;
        }
        v.push(String::from_utf8_lossy(&b[t.start as usize..t.end as usize]).into_owned());
    }
    v
}

/// clang's `-E` of `main` under the same backend: no predefines but the language's (`-undef`), no system
/// headers, the same search order (the forest, then the carried headers' directories on disk).
fn clang(backend: Backend, main: &str) -> Option<String> {
    let f = forest();
    let mut c = Command::new("clang");
    if backend == Backend::OpenCl {
        c.args(["-x", "c", "-std=gnu99", "-undef", "-nostdinc", "-E", "-P", "-w"]);
    } else {
        c.args(["-x", "c++", "-std=gnu++20", "-undef", "-nostdinc", "-E", "-P", "-w"]);
    }
    c.arg(format!("-I{f}")).arg(format!("-I{f}/mldb/builtin/{}/ukl", backend.dir())).arg(format!("-I{f}/mldb/block/ukl"));
    c.arg(format!("-I{}/lib", std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")));
    for (n, v) in backend.predefines() {
        c.arg(format!("-D{n}={}", v.unwrap_or_default()));
    }
    let o = c.arg(main).output().ok()?;
    assert!(o.status.success(), "clang -E failed: {}", String::from_utf8_lossy(&o.stderr));
    Some(String::from_utf8_lossy(&o.stdout).into_owned())
}

/// Ours and clang's tokens are the same; on a difference, the first one with its context.
fn same_as_clang(backend: Backend, main: &str, min_tokens: usize) {
    let ours = Preprocess::new(backend).dir(forest()).run(main).unwrap_or_else(|e| panic!("{e}"));
    let Some(theirs) = clang(backend, main) else {
        eprintln!("no clang++ here: ours preprocessed {main} without error, unchecked against clang");
        return;
    };
    let (a, b) = (tokens(&ours), tokens(&theirs));
    if a != b {
        let k = a.iter().zip(&b).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()));
        let show = |v: &[String]| v[k.saturating_sub(8)..(k + 8).min(v.len())].join(" ");
        panic!("token {k} differs (ours {}, clang {}):\n  ours:  {}\n  clang: {}", a.len(), b.len(), show(&a), show(&b));
    }
    assert!(a.len() > min_tokens, "too few tokens ({}) for {main}", a.len());
}

/// Each library header alone, for each backend: it takes what it needs by `#include`, and preprocesses as clang does.
#[test]
fn every_library_header_stands_alone_on_every_backend() {
    let dir = format!("{}/lib", std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    for backend in [Backend::Metal { version: 310 }, Backend::Cuda { sm: 121 }, Backend::Cuda { sm: 70 }, Backend::Cpu, Backend::OpenCl] {
        // A body is included by its header, with its parameters, and refuses to stand alone.
        for (name, _) in LIBRARY.iter().filter(|(n, _)| !n.starts_with("ukl/body/")) {
            // The shim's tokens, at least, and the header's own.
            same_as_clang(backend, &format!("{dir}/{name}"), 300);
        }
    }
}

/// The OpenCL gate (decision 0440 §3.5): each library header, preprocessed for OpenCL, is OpenCL C 1.2 that clang's
/// front end accepts: with the SIMD group emulated (width 32), and at width 1.
#[test]
fn every_library_header_is_opencl_c_1_2() {
    let dir = format!("{}/lib", std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let tmp = std::env::temp_dir().join(format!("rerail-ukl-cl-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    for width in ["32u", "8u", "1u"] {
        for (name, _) in LIBRARY.iter().filter(|(n, _)| !n.starts_with("ukl/body/")) {
            let main = format!("{dir}/{name}");
            let src = Preprocess::new(Backend::OpenCl).define("UKL_SIMD_WIDTH", Some(width)).run(&main).unwrap_or_else(|e| panic!("{e}"));
            let file = tmp.join(format!("{}.cl", name.replace('/', "_")));
            std::fs::write(&file, &src).unwrap();
            let Ok(o) = Command::new("clang").args(["-x", "cl", "-cl-std=CL1.2", "-fsyntax-only", "-Werror=implicit-function-declaration"]).arg(&file).output() else {
                eprintln!("no clang here: {name} preprocessed for OpenCL, not compiled");
                return;
            };
            assert!(o.status.success(), "{name} at width {width} is not OpenCL C 1.2 ({}):\n{}", file.display(), String::from_utf8_lossy(&o.stderr).lines().take(30).collect::<Vec<_>>().join("\n"));
        }
    }
    std::fs::remove_dir_all(&tmp).ok();
}

/// A kernel reaches the library by `<ukl/...>`, from memory as from disk: the carried copy.
#[test]
fn a_kernel_includes_the_library() {
    let k = "#include <ukl/scan.h>\n";
    let cuda = Preprocess::new(Backend::Cuda { sm: 121 }).file("/mem/k.cu", k).run("/mem/k.cu").unwrap();
    assert!(cuda.contains("ukl_scan_u32_tiles__ukl_params"), "the scan's kernels, with their rows");
    assert!(cuda.contains("__reduce_add_sync"), "sm 8.0 and up: the native warp sum");
    let old = Preprocess::new(Backend::Cuda { sm: 70 }).file("/mem/k.cu", k).run("/mem/k.cu").unwrap();
    assert!(!old.contains("__reduce_add_sync"), "sm 7.0: the reference sum, by shuffles");
    let metal = Preprocess::new(Backend::Metal { version: 310 }).file("/mem/k", k).run("/mem/k").unwrap();
    assert!(metal.contains("simd_prefix_inclusive_sum"), "Metal's native scan");
}

/// The random forest's kernels for Metal, as `forest/build.rs` preprocesses them.
#[test]
fn metal_random_forest_kernels_preprocess_as_clang_does() {
    same_as_clang(Backend::Metal { version: 310 }, &format!("{}/mldb/plugins/jml/randomforest_kernels_ukl.h", forest()), 1000);
}

/// The same source for CUDA: one `extern "C" __global__` entry per kernel, its threadgroup storage declared.
#[test]
fn cuda_random_forest_kernels_preprocess_as_clang_does() {
    let main = format!("{}/mldb/plugins/jml/randomforest_kernels_ukl.h", forest());
    same_as_clang(Backend::Cuda { sm: 121 }, &main, 1000);
    let out = Preprocess::new(Backend::Cuda { sm: 121 }).dir(forest()).run(&main).unwrap();
    // An entry and its rows' descriptor per kernel; a layout kernel per reflected `args` structure.
    assert_eq!(out.matches("__ukl_params[] =").count(), 17, "one descriptor per kernel");
    let layouts = out.matches("__ukl_layout(unsigned long long").count();
    assert_eq!(layouts, 10, "one layout kernel per args structure");
    assert_eq!(out.matches("extern \"C\" __global__ void").count(), 17 + layouts);
    assert!(out.contains("__shared__ WIndexed wStartBest[2];"), "a SHARED row declares its storage");
}

/// A header the search does not find fails the kernel, naming it.
#[test]
fn a_missing_header_is_an_error() {
    let e = Preprocess::new(Backend::Metal { version: 310 })
        .file("/mem/k.metal", "#include <no_such_header.h>\nint x;\n")
        .run("/mem/k.metal")
        .unwrap_err();
    assert!(e.to_string().contains("NotFound"), "{e}");
}

/// A kernel held in memory reaches the carried shim: `ukl_shim.h` picks the backend from the predefines.
#[test]
fn an_in_memory_kernel_gets_its_backends_shim() {
    let k = "#include <ukl>\nKERNEL_RETURN();\nukl_threadgroup_barrier();\n";
    let metal = Preprocess::new(Backend::Metal { version: 310 }).file("/mem/k", k).run("/mem/k").unwrap();
    assert!(metal.contains("threadgroup_barrier(mem_flags::mem_threadgroup)"), "{metal}");
}

/// A hand-written MSL kernel that takes the tape's reads (decision 0440 §3.3): the Metal front end's own headers left
/// to it, the reads' hooks as the kernel defines them, and the tape's two sets of reads refused together.
#[test]
fn a_metal_kernel_takes_the_tapes_reads() {
    let k = "#include <metal_stdlib>\nusing namespace metal;\n#define CLIF_VIEW_SPACE device\n#define clif_view(v, a) the_kernels_view(v, a)\n#include <ukl/reads/tape.h>\nkernel void k() {}\n";
    let out = crate::metal_source("k.metal", k).unwrap_or_else(|e| panic!("{e}"));
    let squeezed: String = out.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(squeezed.contains("clif_tape_number(deviceconstuint8_t*t,deviceconstuint32_t*v"), "the view where the kernel reads it");
    assert!(out.contains("the_kernels_view(v,") || out.contains("the_kernels_view(v ,"), "the kernel's load of the view");
    assert!(!out.contains("metal_stdlib"), "the front end's own header, left to it");
    assert!(crate::takes_library(k) && !crate::takes_library("#include <metal_stdlib>\n"));
    let both = "#include <ukl/reads/tape.h>\n#include <ukl/reads/tape_z.h>\n";
    assert!(crate::metal_source("both.metal", both).is_err(), "one set of the tape's reads");
}

/// The fingerprint computed when the crate is compiled is the hash it was computed as at each call: FNV-1a over each
/// carried file's name and text, each followed by a 0. A changed value would key every stored kernel anew.
#[test]
fn the_fingerprint_is_fnv_over_the_carried_files() {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for (name, text) in HEADERS.iter().chain(LIBRARY) {
        for b in name.bytes().chain([0]).chain(text.bytes()).chain([0]) {
            h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    assert_eq!(fingerprint(), h);
}

/// Windows' LLP64: an integer literal's `long` suffix is written `long long`, so `(1ul << 39) - 1` and `~0ul` are the
/// 64-bit values UKL means; other literals are as written.
#[test]
fn a_long_literal_is_long_long_where_long_is_32_bits() {
    let w = |t: &str| super::long_long(t.as_bytes()).map(|v| String::from_utf8(v).unwrap());
    assert_eq!(w("1ul").as_deref(), Some("1ull"));
    assert_eq!(w("0x8000000000000000UL").as_deref(), Some("0x8000000000000000ull"));
    assert_eq!(w("7lu").as_deref(), Some("7ull"));
    assert_eq!(w("3L").as_deref(), Some("3ll"));
    for same in ["1ull", "2ll", "5u", "9", "1.0", "1e5", "0x1p3", "x1ul", "2.5l"] {
        assert_eq!(w(same), None, "{same}");
    }
}
