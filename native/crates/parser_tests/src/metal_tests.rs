//! The parsers on Metal, against the CPU: every JSON grammar's tier 1 as generated MSL equals the
//! interpreter (`every_grammar_on_metal_equals_the_interpreter`), and the JSON forms' Metal
//! measurements over a corpus (`json_metal_bench`, `json_metal_serial_bench`, run by name). The
//! backend is the Metal crate's (`rerail_metal::block_program`).

use crate::json::{base, files_in, form};
use rerail_compiler::ladder::*;
use rerail_compiler::{block, block_native as native_ir};
use rerail_metal::block_program;

/// Each JSON grammar's tier 1 on Metal, generated MSL (one lane width, the class planes), equals the
/// CPU interpreter on every document: its outputs, and its extracted positions where tier 1 accepts
/// it (decision 0411). A step whose program needs
/// a form generated MSL lacks (`block_program::missing_form`: a fixed point, a back fill or load)
/// is refused by that name and counted, not compared; most steps run.
#[test]
fn every_grammar_on_metal_equals_the_interpreter() {
    use block_program::{ClassStrategy, Executor, LaneWidth, run_split};
    let steps = grammars();
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
        // A file a total refuses goes to the fallback: its positions are read by no one, and the CPU's
        // stop at the refusal; its totals are compared.
        let positions: Vec<usize> = (0..want.positions.len()).filter(|e| !refused && !excused.contains(e) && got.positions.get(*e) != want.positions.get(*e)).collect();
        if !outputs.is_empty() || !positions.is_empty() || got.outputs.len() != want.outputs.len() || got.totals != want.totals || !refused && got.positions.len() != want.positions.len() {
            differ.push(format!("{} (refused {refused}; outputs {outputs:?}, extracts {positions:?}): {:?}", steps[k].0, String::from_utf8_lossy(f.bytes)));
        }
    }
    eprintln!("{} files over {} steps on Metal, {} refused; {excused_differ} refused files' records differ (excused)", files.len(), run_on.len(), refused.len());
    assert!(differ.is_empty(), "{} files differ:\n{}", differ.len(), differ.join("\n"));
}

/// An extra's record on Metal keeps the blank marks of its own gap, as the interpreter's
/// (`Records::extra`): `BLANKS_STEP`'s comments after a line break, one after another too, have
/// semi's bit, and the codes and positions equal the interpreter's over texts of line breaks between and inside its comments.
#[test]
fn an_extra_keeps_its_blank_marks_on_metal() {
    use block_program::serial::{Segments, run_serial_split};
    let c = compile_step("blanks", BLANKS_STEP);
    let texts: [&[u8]; 8] = [b"a\nb\n", b"a /* x\ny */ b\n", b"a // x\nb\n", b"k(\"x\ny\")\nb", b"a /* x */\n/* y */ b\n", b"a\n\n// x\n\nb", b"k /* \n */ (b)\n", BLANKS_TEXT];
    let docs: Vec<Vec<u8>> = texts.iter().map(|t| t.to_vec()).collect();
    let p = &c.native.block_programs[0];
    let files: Vec<block::BatchFile<'_>> = docs.iter().map(|b| block::BatchFile { bytes: b, program: 0 }).collect();
    // The forward carry in segments of three blocks: a region's state carried across them.
    let run = run_serial_split(&[p], &files, Segments { blocks: 3, warm: 1 }).unwrap_or_else(|e| panic!("metal: {e:?}")).run;
    let differ: Vec<String> = files.iter().zip(&run.files).filter(|(f, got)| {
        let want = c.program.block_programs[0].run_files(&[f.bytes]).remove(0);
        got.positions != want.positions || got.codes != want.codes
    }).map(|(f, _)| format!("{:?}", String::from_utf8_lossy(f.bytes))).collect();
    assert!(differ.is_empty(), "{} files differ:\n{}", differ.len(), differ.join("\n"));
    let last = run.files.last().expect("the text's run");
    let semi: Vec<u32> = last.codes.iter().map(|&k| k >> rerail_compiler::records::MARK_SHIFT & 1).collect();
    assert_eq!(semi, [0, 0, 1, 1, 0, 1]);
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
    for name in ["json-rfc", "jsonc"] {
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
    for name in ["json-rfc", "jsonc"] {
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

/// A file's start on Metal (decisions 0330, 0335): an `early` (a value of a file's first bytes
/// shifted down in its word) and a prefix (the stream after a byte order mark) give the
/// interpreter's outputs, positions and totals, at every lane width, interpreted and generated,
/// and in segments. The files have the BOM whole, cut, doubled, not at the start and none, at
/// lengths over the lanes' and blocks' edges.
#[test]
fn a_files_start_on_metal_is_the_interpreters() {
    use block_program::serial::{Segments, run_serial};
    use block_program::{ClassStrategy, Executor, LaneWidth, run_split};
    // The BOM as masks (0330's form): its bytes, two bytes late, cut from a mask at delay 0
    // by an `early`.
    const EARLY: &str = "lexer t
class ef = 0xef
class bb = 0xbb
class bf = 0xbf
class s = ' '
mask first = ~prev(~empty)
mask bom = first & ef & next(bb) & next(bf, 2)
mask bom_bytes = bom | prev(bom, 1) | prev(bom, 2)
mask lone = (s | ef) & ~bom_bytes
escape \"l\" when lone
";
    let bom: &[u8] = &[0xef, 0xbb, 0xbf];
    let units: [Vec<u8>; 5] = [
        [bom, br#"{"a": [1, 2.5, "x"#, b"\xc3\xa9", br#""],  "b" : true}  "#].concat(),
        [bom, bom, b"[1]  "].concat(),
        vec![0xef, 0xbb, b' ', b' ', 0xbf],
        [b"  ", bom, b"[]"].concat(),
        br#"{"k": null, "v": [false, 10]} "#.to_vec(),
    ];
    let mut files: Vec<Vec<u8>> = Vec::new();
    for unit in &units {
        for len in [0usize, 1, 2, 3, 4, 5, 15, 16, 17, 31, 32, 33, 63, 64, 65, 66, 127, 128, 130, 200] {
            files.push(unit.iter().copied().cycle().take(len).collect());
        }
    }
    let refs: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
    let forms = [("early", compile_step("early", EARLY)), ("json-rfc", form("json-rfc")), ("jsonc", form("jsonc"))];
    for (name, c) in &forms {
        let native = &c.native.block_programs[0];
        let late = native.insts.iter().any(|i| matches!(i, native_ir::BlockInst::Early(..) | native_ir::BlockInst::Prefix(..)));
        assert!(late, "{name} starts late: an `early` or a prefix");
        assert_eq!(block_program::missing_form(native), None, "{name} runs on Metal");
        let want = c.program.block_programs[0].run_files(&refs);
        let batch: Vec<block::BatchFile<'_>> = refs.iter().map(|b| block::BatchFile { bytes: b, program: 0 }).collect();
        for width in [LaneWidth::W16, LaneWidth::W32, LaneWidth::W64] {
            for executor in [Executor::Interpreter, Executor::Generated(ClassStrategy::Planes)] {
                let run = run_split(&[native], &batch, width, executor, 64).unwrap_or_else(|e| panic!("{name} {width:?} {executor:?}: {e:?}"));
                for (k, (got, want)) in run.files.iter().zip(&want).enumerate() {
                    assert!(got.outputs == want.outputs && got.positions == want.positions, "{name} {width:?} {executor:?}: {:02x?}", refs[k]);
                }
            }
        }
        for segments in [Segments { blocks: 29, warm: 1 }, Segments { blocks: 3, warm: 1 }] {
            let run = run_serial(&[native], &batch, segments).unwrap_or_else(|e| panic!("{name} {segments:?}: {e:?}"));
            for (k, (got, want)) in run.run.files.iter().zip(&want).enumerate() {
                assert!(got.outputs == want.outputs, "{name} {segments:?} outputs: {:02x?}", refs[k]);
                assert!(got.positions == want.positions && got.totals == want.totals, "{name} {segments:?} positions or totals: {:02x?}", refs[k]);
            }
        }
    }
}
