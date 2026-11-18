use super::*;
use rerail_c_lean::pp::{P, PpLexer};
use std::process::Command;

/// `forest/src`, where MLDB's tree is.
fn forest() -> String {
    format!("{}/../../../forest/src", env!("CARGO_MANIFEST_DIR"))
}

/// The tokens of preprocessed text, its `#` lines (line markers, pragmas) left out.
fn tokens(text: &str) -> Vec<String> {
    let b = text.as_bytes();
    let mut lx = PpLexer::new(b, Options::cpp(20, true));
    let mut v = Vec::new();
    loop {
        let t = lx.next();
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
    let mut c = Command::new("clang++");
    c.args(["-x", "c++", "-std=gnu++20", "-undef", "-nostdinc", "-E", "-P", "-w"]);
    c.arg(format!("-I{f}")).arg(format!("-I{f}/mldb/block/ukl")).arg(format!("-I{f}/mldb/builtin/{}/ukl", backend.dir()));
    for (n, v) in backend.predefines() {
        c.arg(format!("-D{n}={}", v.unwrap_or_default()));
    }
    let o = c.arg(main).output().ok()?;
    assert!(o.status.success(), "clang -E failed: {}", String::from_utf8_lossy(&o.stderr));
    Some(String::from_utf8_lossy(&o.stdout).into_owned())
}

/// Ours and clang's tokens are the same; on a difference, the first one with its context.
fn same_as_clang(backend: Backend, main: &str) {
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
    assert!(a.len() > 1000, "too few tokens ({}) for the random forest's kernels", a.len());
}

/// The random forest's kernels for Metal, as `forest/build.rs` preprocesses them.
#[test]
fn metal_random_forest_kernels_preprocess_as_clang_does() {
    same_as_clang(Backend::Metal { version: 310 }, &format!("{}/mldb/plugins/jml/randomforest_kernels_ukl.h", forest()));
}

/// The same source for CUDA: one `extern "C" __global__` entry per kernel, its threadgroup storage declared.
#[test]
fn cuda_random_forest_kernels_preprocess_as_clang_does() {
    let main = format!("{}/mldb/plugins/jml/randomforest_kernels_ukl.h", forest());
    same_as_clang(Backend::Cuda { sm: 121 }, &main);
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
