//! The Neural Engine lane's build step: ahead-of-time models, part of the executable.
//!
//! `accel/models/aot.list` names constant files (one a line, relative to `accel/models/`; `#` starts a comment),
//! each a fixed relation in the `RRBITS01` layout (`src/ane/package.rs`). For each, this writes the bit product's
//! model package with the lane's own writer (the same file, included here), compiles it with
//! `xcrun coremlcompiler compile`, and archives the compiled model. `$OUT_DIR/ane_aot.rs` lists them as
//! `(constant file, archive)` pairs, embedded with `include_bytes!`; the lane checks them before compiling at run
//! time. Without `xcrun` (or with an empty list) the registry is empty, and a warning says why when entries are
//! skipped.

#[path = "../src/ane/package.rs"]
mod package;

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn run() {
    println!("cargo:rerun-if-changed=build/ane.rs");
    println!("cargo:rerun-if-changed=src/ane/package.rs");
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let models = manifest.join("models");
    let list = models.join("aot.list");
    println!("cargo:rerun-if-changed={}", list.display());
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let mut entries = Vec::new();
    let names: Vec<String> = std::fs::read_to_string(&list)
        .unwrap_or_default()
        .lines()
        .map(|l| l.split('#').next().unwrap().trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if !names.is_empty() {
        let xcrun = Command::new("xcrun").args(["--find", "coremlcompiler"]).output();
        if !xcrun.is_ok_and(|o| o.status.success()) {
            println!("cargo:warning=ane: xcrun coremlcompiler is not available; the {} ahead-of-time models of models/aot.list are not built (they will compile at run time)", names.len());
        } else {
            let work = out.join("ane");
            let _ = std::fs::create_dir_all(&work);
            for (i, name) in names.iter().enumerate() {
                let src = models.join(name);
                println!("cargo:rerun-if-changed={}", src.display());
                match build_one(&src, &work, i) {
                    Ok(archive) => entries.push((src, archive)),
                    Err(e) => println!("cargo:warning=ane: ahead-of-time model {name}: {e}"),
                }
            }
        }
    }
    let mut rs = String::from("pub static AOT: &[(&[u8], &[u8])] = &[\n");
    for (c, a) in &entries {
        rs += &format!("    (include_bytes!({:?}), include_bytes!({:?})),\n", c.display().to_string(), a.display().to_string());
    }
    rs += "];\n";
    std::fs::write(out.join("ane_aot.rs"), rs).unwrap();
}

fn build_one(src: &Path, work: &Path, i: usize) -> Result<PathBuf, String> {
    let bytes = std::fs::read(src).map_err(|e| e.to_string())?;
    let (rows, words, data) = package::read_constant(&bytes)?;
    let t = package::plan(rows, words, &data, package::TILE_ROWS)?;
    let pkg = work.join(format!("m{i}.mlpackage"));
    package::write_package(&pkg, &t, words, &data).map_err(|e| e.to_string())?;
    let dst = work.join(format!("m{i}"));
    let _ = std::fs::remove_dir_all(&dst);
    std::fs::create_dir_all(&dst).map_err(|e| e.to_string())?;
    let o = Command::new("xcrun").arg("coremlcompiler").arg("compile").arg(&pkg).arg(&dst).output().map_err(|e| e.to_string())?;
    if !o.status.success() {
        return Err(format!("coremlcompiler failed: {}", String::from_utf8_lossy(&o.stderr).trim()));
    }
    let compiled = dst.join(format!("m{i}.mlmodelc"));
    let archive = package::archive(&compiled).map_err(|e| e.to_string())?;
    let path = work.join(format!("m{i}.rrmlmc"));
    std::fs::write(&path, archive).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&pkg);
    let _ = std::fs::remove_dir_all(&dst);
    Ok(path)
}
