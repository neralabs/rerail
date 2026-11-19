//! The parsers on Metal, against the CPU: every ladder step's tier 1 as generated MSL equals the
//! interpreter (`every_step_on_metal_equals_the_interpreter`), and the JSON forms' Metal
//! measurements over a corpus (`json_metal_bench`, `json_metal_serial_bench`, run by name). The
//! backend is the Metal crate's (`rerail_metal::block_program`).

use crate::json::{base, files_in, form};
use rerail_compiler::ladder::*;
use rerail_compiler::{block, block_native as native_ir};
use rerail_metal::block_program;

/// Every step's tier 1 on Metal, generated MSL (one lane width, the class planes), equals the
/// CPU interpreter on every document: outputs and extracted positions. A step whose program needs
/// a form generated MSL lacks (`block_program::missing_form`: a fixed point, a back fill or load,
/// an early start) is refused by that name and counted, not compared; most steps run.
#[test]
fn every_step_on_metal_equals_the_interpreter() {
    use block_program::{ClassStrategy, Executor, LaneWidth, run_split};
    let steps = steps_in(&dir());
    let compiled: Vec<Compiled> = steps.iter().map(|(n, s)| compile_step(n, s)).collect();
    let docs: Vec<Vec<Vec<u8>>> = compiled.iter().zip(&steps).map(|(c, (n, _))| documents(&Lang::of(&c.stmts), seed(n), &|b| reference(c, b).is_some()).0).collect();
    let all: Vec<&native_ir::BlockNative> = compiled.iter().map(|c| &c.native.block_programs[0]).collect();
    // The steps Metal runs, by their place in `steps`; the others with the form they lack.
    let (run_on, refused): (Vec<usize>, Vec<usize>) = (0..steps.len()).partition(|&k| block_program::missing_form(all[k]).is_none());
    for &k in &refused {
        eprintln!("  refused {}: {}", steps[k].0, block_program::missing_form(all[k]).unwrap_or_default());
    }
    assert!(refused.len() * 2 < steps.len(), "most steps run on Metal: {} of {} refused", refused.len(), steps.len());
    let forms: Vec<&native_ir::BlockNative> = run_on.iter().map(|&k| all[k]).collect();
    let files: Vec<block::BatchFile<'_>> = run_on.iter().enumerate().flat_map(|(p, &k)| docs[k].iter().map(move |b| block::BatchFile { bytes: b, program: p })).collect();
    let run = run_split(&forms, &files, LaneWidth::W32, Executor::Generated(ClassStrategy::Planes), 64).unwrap_or_else(|e| panic!("metal: {e:?}"));
    let (mut differ, mut excused_differ) = (Vec::new(), 0);
    for (f, got) in files.iter().zip(&run.files) {
        let k = run_on[f.program];
        let program = &compiled[k].program.block_programs[0];
        let want = program.run_files(&[f.bytes]).remove(0);
        // Where the interpreter makes the tokens' records itself (the loop's starts and lasts,
        // decision 0177 stage 2a; the lasts by the gaps, 0411), they are proved equal to the
        // masks' only where the file is taken: a refused file's are excused (tier 2 reads none),
        // so Metal's starts and lasts extracts are not compared there.
        let refused = want.totals.iter().any(|&t| t != 0);
        let excused: Vec<usize> = program
            .records
            .as_ref()
            .filter(|r| refused && (r.skip.is_some() || r.gap.is_some()))
            .map_or(Vec::new(), |r| vec![usize::from(r.starts), usize::from(r.lasts)]);
        excused_differ += usize::from(excused.iter().any(|&e| got.positions.get(e) != want.positions.get(e)));
        let outputs: Vec<usize> = (0..want.outputs.len()).filter(|&o| got.outputs.get(o) != want.outputs.get(o)).collect();
        let positions: Vec<usize> = (0..want.positions.len()).filter(|e| !excused.contains(e) && got.positions.get(*e) != want.positions.get(*e)).collect();
        if !outputs.is_empty() || !positions.is_empty() || got.outputs.len() != want.outputs.len() || got.positions.len() != want.positions.len() {
            differ.push(format!("{} (refused {refused}; outputs {outputs:?}, extracts {positions:?}): {:?}", steps[k].0, String::from_utf8_lossy(f.bytes)));
        }
    }
    eprintln!("{} files over {} steps on Metal, {} refused; {excused_differ} refused files' records differ (excused)", files.len(), run_on.len(), refused.len());
    assert!(differ.is_empty(), "{} files differ:\n{}", differ.len(), differ.join("\n"));
}

/// Tier 1 on Metal in segments (`block_program::serial`), over the corpus as one batch: GPU
/// seconds (the fastest of `RERAIL_ITERS` runs, default 5), segments run again and checks, for
/// `RERAIL_SEG` blocks a segment (default 29) and `RERAIL_SEG_WARM` warm-up blocks (default 1); the
/// outputs, positions and totals equal Cranelift's. `RERAIL_REPEAT` and `RERAIL_FORM` as in
/// `json_metal_bench`.
#[test]
#[ignore = "a measurement: run by name with RERAIL_JSON_CORPUS and --nocapture"]
fn json_metal_serial_bench() {
    use block_program::serial::{Segments, run_serial_split};
    let env = |k: &str, d: usize| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let iters = env("RERAIL_ITERS", 5);
    let mut paths = files_in("RERAIL_JSON_CORPUS");
    let mut files: Vec<Vec<u8>> = paths.iter().map(|p| std::fs::read(p).expect("reads")).collect();
    let n = env("RERAIL_REPEAT", 1);
    (files, paths) = ((0..n).flat_map(|_| files.clone()).collect(), (0..n).flat_map(|_| paths.clone()).collect());
    let refs: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
    let total: usize = files.iter().map(Vec::len).sum();
    let segments = Segments { blocks: env("RERAIL_SEG", 29), warm: env("RERAIL_SEG_WARM", 1) };
    println!("{} files, {} bytes, {segments:?}", files.len(), total);
    println!("form\tgpu_ms\tGB/s\tfirst_ms\tagain_ms\tsinks_ms\tsegments\treruns\trounds");
    for name in ["14-json", "15-jsonc"] {
        if std::env::var("RERAIL_FORM").is_ok_and(|f| !name.starts_with(&f)) {
            continue;
        }
        let c = form(name);
        let native = &c.native.block_programs[0];
        let want = c.parser.tier1.run_files(&refs);
        let batch: Vec<block::BatchFile<'_>> = refs.iter().map(|b| block::BatchFile { bytes: b, program: 0 }).collect();
        let mut best: Option<block_program::serial::SerialRun> = None;
        for _ in 0..iters {
            let run = run_serial_split(&[native], &batch, segments).unwrap_or_else(|e| panic!("metal: {e:?}"));
            for (k, (got, want)) in run.run.files.iter().zip(&want).enumerate() {
                assert!(got.outputs == want.outputs, "{name}: {} outputs differ", base(&paths[k]));
                assert!(got.positions == want.positions && got.totals == want.totals, "{name}: {} positions or totals differ", base(&paths[k]));
            }
            if best.as_ref().is_none_or(|b| run.run.seconds < b.run.seconds) {
                best = Some(run);
            }
        }
        let b = best.expect("a run");
        println!(
            "{name}\t{:.3}\t{:.2}\t{:.3}\t{:.3}\t{:.3}\t{}\t{}\t{}",
            b.run.seconds * 1e3,
            total as f64 / b.run.seconds / 1e9,
            b.first_seconds * 1e3,
            b.rerun_seconds * 1e3,
            b.sink_seconds * 1e3,
            b.segments,
            b.reruns,
            b.rounds
        );
    }
}

/// Tier 1 on Metal, generated MSL, over the corpus as one batch: GPU seconds (the fastest of
/// `RERAIL_ITERS` runs, default 5) and launches at each lane width, each form; the outputs and
/// positions equal Cranelift's.
#[test]
#[ignore = "a measurement: run by name with RERAIL_JSON_CORPUS and --nocapture"]
fn json_metal_bench() {
    use block_program::{ClassStrategy, Executor, LaneWidth, SPINS, run_split};
    let iters: usize = std::env::var("RERAIL_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(5);
    let mut paths = files_in("RERAIL_JSON_CORPUS");
    let mut files: Vec<Vec<u8>> = paths.iter().map(|p| std::fs::read(p).expect("reads")).collect();
    // `RERAIL_SPLIT=n`: each file cut into `n` files (at a space or comma, so each piece is
    // tier-1 input like any other), to see what a file's length costs.
    // `RERAIL_REPEAT=n`: the files n times over, as one batch of many files.
    if let Some(n) = std::env::var("RERAIL_REPEAT").ok().and_then(|v| v.parse::<usize>().ok()) {
        (files, paths) = ((0..n).flat_map(|_| files.clone()).collect(), (0..n).flat_map(|_| paths.clone()).collect());
    }
    if let Some(n) = std::env::var("RERAIL_SPLIT").ok().and_then(|v| v.parse::<usize>().ok()) {
        let (mut fs, mut ps) = (Vec::new(), Vec::new());
        for (f, p) in files.iter().zip(&paths) {
            for piece in f.chunks(f.len().div_ceil(n).max(1)) {
                fs.push(piece.to_vec());
                ps.push(p.clone());
            }
        }
        (files, paths) = (fs, ps);
    }
    let refs: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
    let total: usize = files.iter().map(Vec::len).sum();
    let widths: Vec<LaneWidth> = match std::env::var("RERAIL_WIDTH").as_deref() {
        Ok("16") => vec![LaneWidth::W16],
        Ok("32") => vec![LaneWidth::W32],
        Ok("64") => vec![LaneWidth::W64],
        _ => LaneWidth::ALL.to_vec(),
    };
    println!("{} files, {} bytes", files.len(), total);
    println!("form\twidth\tgpu_ms\tGB/s\tlaunches");
    for name in ["14-json", "15-jsonc"] {
        if std::env::var("RERAIL_FORM").is_ok_and(|f| !name.starts_with(&f)) {
            continue;
        }
        let c = form(name);
        let native = &c.native.block_programs[0];
        let want = c.parser.tier1.run_files(&refs);
        for &width in &widths {
            eprintln!("{name}, {}-byte lanes: {}", width.bytes(), block_program::kernel_report(native, width).expect("compiles"));
        }
        let batch: Vec<block::BatchFile<'_>> = refs.iter().map(|b| block::BatchFile { bytes: b, program: 0 }).collect();
        for &width in &widths {
            let mut best = f64::MAX;
            let mut launches = 0;
            for _ in 0..iters {
                let run = run_split(&[native], &batch, width, Executor::Generated(ClassStrategy::Planes), std::env::var("RERAIL_SPINS").ok().and_then(|v| v.parse().ok()).unwrap_or(SPINS)).unwrap_or_else(|e| panic!("metal: {e:?}"));
                for (k, (got, want)) in run.files.iter().zip(&want).enumerate() {
                    assert!(got.outputs == want.outputs && got.positions == want.positions, "{name}, {}-byte lanes: {} differs", width.bytes(), base(&paths[k]));
                }
                best = best.min(run.seconds);
                launches = run.launches;
            }
            println!("{name}\t{}\t{:.3}\t{:.2}\t{launches}", width.bytes(), best * 1e3, total as f64 / best / 1e9);
        }
    }
}
