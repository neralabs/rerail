//! The source guard for decision 0100's one executor: the process has one Metal device and one command queue, one
//! pool of CPU threads, and every other thread is a named service. So the only places that may make any of them are
//! their owners:
//!
//! - the device and the queue: `forest/src/lib.rs` (`gpu::shared`, the lowest crate that uses Metal), which
//!   `rerail_accel::gpu::shared` and `rerail_metal::{device, queue}` hand out;
//! - the pool, the device lanes' workers and every service thread: `accel/src/exec.rs` (`configure`, `service`,
//!   `scoped`).
//!
//! Anything else that makes a device, a queue, a thread or a pool fails here, by file and line, unless it is one of
//! `EXCEPTIONS`, each with its reason. Test code (a `#[cfg(test)]` module, a `*tests.rs` file) is not read.

use std::path::{Path, PathBuf};

/// What may not be made outside its owner, and the owner.
const MADE: [(&str, &str); 7] = [
    ("new_command_queue(", "forest/src/lib.rs"),
    ("Device::system_default(", "forest/src/lib.rs"),
    ("thread::spawn(", "accel/src/exec.rs"),
    ("thread::Builder::new(", "accel/src/exec.rs"),
    ("thread::scope(", "accel/src/exec.rs"),
    ("ThreadPoolBuilder::new(", "accel/src/exec.rs"),
    ("dispatch_queue_create(", "accel/src/exec.rs"),
];

/// The sites allowed besides the owners, each with why: a file, the pattern, and how many times it may appear.
const EXCEPTIONS: [(&str, &str, usize, &str); 5] = [
    ("native/src/code_rule_dsl/logic/costs.rs", "ThreadPoolBuilder::new(", 1, "the cost model trains at background QoS on two threads, beside the run; the one pool runs at the user's"),
    ("forest/src/mldb/plugins/jml/randomforest_procedure.rs", "ThreadPoolBuilder::new(", 2, "the forest's training bounds the bags trained at once; it runs inside the cost model's background pool"),
    ("accel/src/price_bench.rs", "ThreadPoolBuilder::new(", 1, "the price benchmark measures one core alone"),
    ("native/src/content_store/journal.rs", "dispatch_queue_create(", 2, "the journal's file events come on a GCD queue (FSEvents delivers to one): its declaration and its one call"),
    ("native/src/code_rule_dsl/transducer/packed/metal_frontier_reduce.rs", "new_command_queue(", 1, "inside the conformance tests' module, which `cfg(test)` guards from the parent"),
];

/// Files that are test code as a whole (their parent declares them under `cfg(test)`).
const TEST_FILES: [&str; 3] = ["metal_frontier_conformance.rs", "metal_register_conformance.rs", "parity.rs"];

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn sources() -> Vec<PathBuf> {
    let root = repo();
    let mut walk: Vec<PathBuf> = vec![root.join("native/src"), root.join("accel/src"), root.join("forest/src")];
    if let Ok(crates) = std::fs::read_dir(root.join("native/crates")) {
        walk.extend(crates.flatten().map(|c| c.path().join("src")));
    }
    let mut out = Vec::new();
    while let Some(d) = walk.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if p.is_dir() {
                if name != "tests" {
                    walk.push(p);
                }
            } else if name.ends_with(".rs") && !name.ends_with("tests.rs") && !name.ends_with("_test.rs") && !name.starts_with("test_") && !TEST_FILES.contains(&name) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// A file's lines outside its `#[cfg(test)]` modules (a module's lines end at its closing brace at its own indent).
fn product_lines(text: &str) -> Vec<(usize, &str)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        if l.trim_start().starts_with("#[cfg(test)]") || l.trim_start().starts_with("#[cfg(all(test") || l.trim_start().starts_with("#[cfg(any(test") {
            // The item it guards: a module (skipped to its end), else the next line alone.
            let mut j = i + 1;
            while j < lines.len() && lines[j].trim_start().starts_with("#[") {
                j += 1;
            }
            if j < lines.len() {
                let item = lines[j];
                let indent = item.len() - item.trim_start().len();
                if item.trim_end().ends_with('{') {
                    let mut k = j + 1;
                    while k < lines.len() && !(lines[k].len() - lines[k].trim_start().len() == indent && lines[k].trim_start().starts_with('}')) {
                        k += 1;
                    }
                    i = k + 1;
                    continue;
                }
            }
            i = j + 1;
            continue;
        }
        out.push((i + 1, l));
        i += 1;
    }
    out
}

#[test]
fn executors_are_shared() {
    let root = repo();
    let mut found: Vec<(String, &str, usize)> = Vec::new();
    let mut bad = Vec::new();
    for f in sources() {
        let rel = f.strip_prefix(&root).unwrap_or(&f).display().to_string();
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        for (pat, owner) in MADE {
            let at: Vec<usize> = product_lines(&text).into_iter().filter(|(_, l)| l.contains(pat) && !l.trim_start().starts_with("//")).map(|(n, _)| n).collect();
            if at.is_empty() || rel == owner {
                continue;
            }
            match EXCEPTIONS.iter().find(|(file, p, _, _)| *file == rel && *p == pat) {
                Some((_, _, n, _)) if *n == at.len() => found.push((rel.clone(), pat, at.len())),
                Some((_, _, n, _)) => bad.push(format!("{rel}: `{pat}` {} times, where its exception allows {n} (lines {at:?})", at.len())),
                None => bad.push(format!("{rel}:{}: `{pat}` outside its owner {owner}", at.iter().map(usize::to_string).collect::<Vec<_>>().join(","))),
            }
        }
    }
    for (file, pat, _, why) in EXCEPTIONS {
        if !found.iter().any(|(f, p, _)| f == file && *p == pat) && !bad.iter().any(|b| b.starts_with(file)) {
            bad.push(format!("{file}: the exception for `{pat}` ({why}) is no longer needed: remove it"));
        }
    }
    assert!(bad.is_empty(), "made outside the one executor (decision 0100 §2, §12): use `rerail_metal::queue()`/`device()`, `rerail_accel::exec::service`/`scoped`/`drop_beside`, or the pool:\n{}", bad.join("\n"));
}
