//! The vectorizer's build (`UKL_VEC_BUILD`): a hash of what makes a kernel's code tape from its source, this crate's
//! sources and the C parser's (`rerail-c-lean`). The device's store of compiled kernels keys a kernel's code tape by
//! it with the preprocessed source (`rerail_gpu`'s `vec`), so a tape another build of these made is never loaded.

use std::path::{Path, PathBuf};

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn main() {
    let here = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("the manifest's directory"));
    let dirs = [here.join("src"), here.join("../c_lean/src")];
    let mut paths = Vec::new();
    for d in &dirs {
        println!("cargo:rerun-if-changed={}", d.display());
        files(d, &mut paths);
    }
    paths.sort();
    // FNV-1a over each file's path from its directory and its bytes.
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for p in &paths {
        let name = p.strip_prefix(&here).unwrap_or(p).to_string_lossy().into_owned();
        for &b in name.as_bytes().iter().chain(&[0]).chain(std::fs::read(p).unwrap_or_default().iter()).chain(&[0]) {
            h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    println!("cargo:rustc-env=UKL_VEC_BUILD={h:016x}");
}
