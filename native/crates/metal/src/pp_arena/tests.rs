//! R1 of decision 0448: the searches equal a host reference, across SIMD-group and group edges.
//! The scans, segmented scans and sorts are UKL's stages (`ukl/stages.h`), tested with UKL's library
//! (`gpu/src/library_tests.rs`: `the_stages_*`).

use super::*;
use crate::lane::Pod;

/// Sizes that cross a SIMD group (32), a group (256) and a scan block (1024).
const SIZES: [usize; 12] = [0, 1, 31, 33, 255, 257, 1023, 1025, 4097, 70_001, 1_048_577, 1_100_000];

/// A small generator (xorshift), so the inputs are the same every run.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// A test's own buffer of `v` (at least one value's room).
fn buf<T: Pod + Default>(v: &[T]) -> Buffer {
    let device = lane::device().unwrap();
    if v.is_empty() { lane::buffer_of(device, &[T::default()]) } else { lane::buffer_of(device, v) }
}

fn zeroed<T: Pod + Default>(n: usize) -> Buffer {
    buf(&vec![T::default(); n.max(1)])
}

fn read<T: Copy>(b: &Buffer, n: usize) -> Vec<T> {
    // SAFETY: the device is done with the buffer, which holds at least `n` values.
    unsafe { std::slice::from_raw_parts(b.contents().cast::<T>(), n).to_vec() }
}

/// Runs `f`'s launches in one command, and waits.
fn run<'a>(f: impl FnOnce(&Enc<'a>)) {
    crate::pp_eval::run(|e| {
        f(e);
        Ok(())
    })
    .unwrap();
}

/// The `#if` evaluator by itself (`pp_if.uglykl`'s `sy_feed`, `sy_end`), a thread an expression:
/// each one's truth and value as the serial pass gives them.
#[test]
fn the_if_evaluator_is_the_serial_passes() {
    // Operators as the evaluator numbers them (`EO_*`); 0 a value.
    const PLUS: u32 = 14;
    const MINUS: u32 = 15;
    const STAR: u32 = 16;
    const EQ: u32 = 6;
    const LPAREN: u32 = 22;
    const RPAREN: u32 = 23;
    const QUESTION: u32 = 24;
    const COLON: u32 = 25;
    let v = |x: u64| (0u32, x);
    let o = |op: u32| (op, 0u64);
    // An expression's text, its items (an operator and a value each), its truth and its value's low word.
    type Case = (&'static str, Vec<(u32, u64)>, u32, u32);
    let cases: Vec<Case> = vec![
        ("1", vec![v(1)], 1, 1),
        ("1 == 1", vec![v(1), o(EQ), v(1)], 1, 1),
        ("1 + 1", vec![v(1), o(PLUS), v(1)], 1, 2),
        ("1 + 2 * 3", vec![v(1), o(PLUS), v(2), o(STAR), v(3)], 1, 7),
        ("1 + 2 * 3 == 7", vec![v(1), o(PLUS), v(2), o(STAR), v(3), o(EQ), v(7)], 1, 1),
        ("(1 + 2) * 3 == 9", vec![o(LPAREN), v(1), o(PLUS), v(2), o(RPAREN), o(STAR), v(3), o(EQ), v(9)], 1, 1),
        ("10 - 2 - 3 == 5", vec![v(10), o(MINUS), v(2), o(MINUS), v(3), o(EQ), v(5)], 1, 1),
        ("1 ? 2 : 3", vec![v(1), o(QUESTION), v(2), o(COLON), v(3)], 1, 2),
        ("0 ? 2 : 0", vec![v(0), o(QUESTION), v(2), o(COLON), v(0)], 0, 0),
    ];
    let (mut offsets, mut ops, mut vals) = (vec![0u32], Vec::new(), Vec::new());
    for (_, items, _, _) in &cases {
        for &(op, x) in items {
            ops.push(op);
            vals.push(x);
        }
        offsets.push(ops.len() as u32);
    }
    let flags = vec![0u32; ops.len()];
    let (ob, pb, vb, fb, out) = (buf(&offsets), buf(&ops), buf(&vals), buf(&flags), zeroed::<u32>(4 * cases.len()));
    run(|e| e.run("pb_if_eval", &[("offsets", &ob), ("ops", &pb), ("vals", &vb), ("flags", &fb), ("out", &out)], Some(ProbeArgs([cases.len() as u32, 0, 0, 0])), 1));
    let got: Vec<u32> = read(&out, 4 * cases.len());
    for (k, (what, _, truth, value)) in cases.iter().enumerate() {
        let g = &got[4 * k..4 * k + 4];
        assert_eq!((g[0], g[1]), (*truth, *value), "{what}: truth and value (depths {:#x}, state {:#x})", g[2], g[3]);
    }
}

/// The `#if` evaluator from its tokens' text, read as `p1_if_eval` reads numbers and character
/// constants (`if_number`, `if_char`): each expression's truth and 64-bit value as the serial
/// pass gives them. Q367: on an M1 Max (macOS 26.2), a thread's bytes stored at a counter other
/// than the loop's (`b[n++] = p[i++]`, as `if_bytes` and `if_number` copied a token) read back
/// wrong, so every decimal number and character constant was refused or misread.
#[test]
fn the_if_evaluator_reads_the_texts_values() {
    let texts: [(&str, u32, u64); 24] = [
        ("1", 1, 1),
        ("0", 0, 0),
        ("2", 1, 2),
        ("1 + 1", 1, 2),
        ("1 == 1", 1, 1),
        ("1 + 2 * 3 == 7", 1, 1),
        ("0x10 + 010 + 0b11 == 27", 1, 1),
        ("1'000 == 1000", 1, 1),
        ("10uLL > 9", 1, 1),
        ("0xFFFFFFFFFFFFFFFFull", 1, u64::MAX),
        ("18446744073709551615", 1, u64::MAX),
        ("99999999999999999999", 0, 0),
        ("1.0", 0, 0),
        ("'a' == 97", 1, 1),
        ("'a'", 1, 97),
        (r"'\n' + '\x41' + '\101' == 140", 1, 1),
        (r"'\377'", 1, u64::MAX),
        ("'ab'", 1, 0x6162),
        (r"L'\xffffffff'", 1, u64::MAX),
        (r"U'\xffffffff'", 1, 0xffff_ffff),
        (r"u8'a'", 1, 97),
        ("'\\u00e9'", 1, 0xffff_ffff_ffff_ffe9),
        // Two bytes of UTF-8 in a plain constant: two characters.
        ("'\u{e9}'", 1, 0xc3a9),
        ("u'\u{e9}'", 1, 0xe9),
    ];
    let (mut bytes, mut starts, mut offsets) = (Vec::new(), vec![0u32], vec![0u32]);
    for (t, _, _) in &texts {
        for tok in t.split(' ') {
            bytes.extend_from_slice(tok.as_bytes());
            starts.push(bytes.len() as u32);
        }
        offsets.push(starts.len() as u32 - 1);
    }
    let (bb, sb, ob, out) = (buf(&bytes), buf(&starts), buf(&offsets), zeroed::<u32>(4 * texts.len()));
    run(|e| e.run("pb_if_text", &[("bytes", &bb), ("starts", &sb), ("offsets", &ob), ("out", &out)], Some(ProbeArgs([texts.len() as u32, 0, 0, 0])), 1));
    let got: Vec<u32> = read(&out, 4 * texts.len());
    let wrong: Vec<String> = texts
        .iter()
        .enumerate()
        .filter_map(|(k, (what, truth, value))| {
            let g = &got[4 * k..4 * k + 4];
            let v = u64::from(g[2]) << 32 | u64::from(g[1]);
            // A refused expression's value is no value.
            let v = if g[3] & 1 != 0 { 0 } else { v };
            ((g[0], v) != (*truth, *value)).then(|| format!("{what}: truth {} value {v:#x} state {:#x}", g[0], g[3]))
        })
        .collect();
    assert!(wrong.is_empty(), "{wrong:?}");
}

#[test]
fn search_and_expand_equal_the_host() {
    let mut r = Rng(17);
    for &n in &SIZES[..10] {
        let mut sorted: Vec<u64> = (0..n).map(|_| r.next() % 5000).collect();
        sorted.sort_unstable();
        let queries: Vec<u64> = (0..n.max(1) * 2).map(|_| r.next() % 5100).collect();
        let (sb, qb, out, counts) = (buf(&sorted), buf(&queries), zeroed::<u32>(queries.len()), buf(&[n as u32, queries.len() as u32]));
        run(|e| e.run("pb_search", &[("sorted", &sb), ("ns", &counts), ("queries", &qb), ("nq", &counts), ("out", &out)], Some(ProbeArgs([0, 1, 0, 0])), (queries.len() as u64).div_ceil(tg())));
        let want: Vec<u32> = queries.iter().map(|&q| sorted.partition_point(|&s| s < q) as u32).collect();
        assert_eq!(read::<u32>(&out, queries.len()), want, "search, n = {n}");
        // Expand: items with counts 0 to 3, their offsets an exclusive scan.
        let item_counts: Vec<u32> = (0..n).map(|_| (r.next() % 4) as u32).collect();
        let mut acc = 0u32;
        let offsets: Vec<u32> = item_counts.iter().map(|&c| { let a = acc; acc += c; a }).collect();
        let total = acc as usize;
        let (ob, nb, tb) = (buf(&offsets), buf(&[n as u32]), buf(&[acc]));
        let (item, rank) = (zeroed::<u32>(total), zeroed::<u32>(total));
        run(|e| e.run("pb_expand", &[("offsets", &ob), ("n", &nb), ("total", &tb), ("item", &item), ("rank", &rank)], Some(ProbeArgs::default()), (total as u64).div_ceil(tg())));
        let mut want = Vec::new();
        for (k, &c) in item_counts.iter().enumerate() {
            for j in 0..c {
                want.push((k as u32, j));
            }
        }
        let got: Vec<(u32, u32)> = read::<u32>(&item, total).into_iter().zip(read::<u32>(&rank, total)).collect();
        assert_eq!(got, want, "expand, n = {n}");
    }
}

/// R9 of decision 0448: an evaluation is one command, so the host never reads between its
/// passes. `pp_eval::run` takes a whole evaluation's launches (an expansion's included) and
/// refuses one that used a second command.
#[test]
fn an_evaluation_is_one_command_buffer() {
    let b = [buf(&[1u64, 3]), buf(&[2u32, 1]), buf(&[2u64]), buf(&[0u32])];
    fn search<'a>(enc: &Enc<'a>, b: &'a [Buffer; 4]) {
        enc.run("pb_search", &[("sorted", &b[0]), ("ns", &b[1]), ("queries", &b[2]), ("nq", &b[1]), ("out", &b[3])], Some(ProbeArgs([0, 1, 0, 0])), 1);
    }
    let one = crate::pp_eval::run(|enc| {
        search(enc, &b);
        Ok(())
    });
    assert_eq!(one.unwrap(), 1);
    let two = crate::pp_eval::run(|enc| {
        search(enc, &b);
        // A second command, waited on mid-evaluation: a host read between passes.
        let other = Enc::new("a second command");
        search(&other, &b);
        other.finish().map(|_| ())
    });
    assert!(two.is_err(), "a second command in one evaluation");
}

/// A unit's tiles a phase are one a `TILE_ITEMS` of its items, at least one, at most `TILES_MOST`, and 1 on the CPU
/// (its one worker): `TILE_ITEMS` is `ukl/stages.h`'s `UKL_STAGE_SMALL` (a stage of one tile), `TILES_MOST` the
/// program's `PP_TLIB` (the most tiles `SUMS` holds a stage's sums of).
#[test]
fn a_units_tiles_are_one_a_stages_small_items() {
    let header = rerail_ukl::LIBRARY.iter().find(|(p, _)| *p == "ukl/stages.h").expect("UKL's stages header").1;
    assert!(header.contains(&format!("#define UKL_STAGE_SMALL {TILE_ITEMS}u")), "UKL_STAGE_SMALL is TILE_ITEMS");
    assert!(include_str!("../pp_grid.uglykl").contains(&format!("#define PP_TLIB {TILES_MOST}u")), "PP_TLIB is TILES_MOST");
    if matches!(lane::device(), Ok(lane::Device::Cpu(_))) {
        assert_eq!((tiles_for(0), tiles_for(usize::MAX / 2), workers()), (1, 1, 1), "the CPU's one worker, a tile a phase");
        return;
    }
    assert_eq!(tiles_for(0), 1);
    assert_eq!(tiles_for(TILE_ITEMS), 1);
    assert_eq!(tiles_for(TILE_ITEMS + 1), 2);
    assert_eq!(tiles_for(usize::MAX / 2), TILES_MOST);
}

/// Decision 0653: no persistent grid. The evaluator's source names no grid barrier, join or residency (`ukl/grid.h`
/// is gone), and a launch's ring holds its budget's pushes, a phase's tiles a worker past it, and the tiles queued.
#[test]
fn the_setup_is_a_program_on_a_queue() {
    for gone in ["grid.h", "ukl_grid_", "UKL_GRID_", "UklGrid", "pp_setup", "pp_teams", "pp_check"] {
        assert!(!source().contains(gone), "the evaluator's source names {gone}");
    }
    assert!(source().contains("#include <ukl/stages.h>") && source().contains("#include <ukl/workqueue.h>"));
    let tiles = [1, 7, TILES_MOST];
    assert_eq!(queue_slots(&tiles), BUDGET as usize + workers() as usize * TILES_MOST as usize + (8 + TILES_MOST as usize) + 64);
    assert!(2 * PP_LAUNCHES <= crate::launch::KERNEL_LAUNCH_LIMIT, "a command's launches within the limit");
}

/// A unit's runs (`runs`, `LPRUN`) are laid out as the kernel reads them: a tile's `RUN_SLOTS` slots of `RUN_BYTES`
/// each (`pp_grid.uglykl`'s `LP_SLOTS`, `LP_RUN_BYTES`), by the loop's phase's tile, and the run fits its slot (the
/// source's static assertion: it compiles).
#[test]
fn a_units_runs_are_the_kernels_slots() {
    let src = source();
    assert!(src.contains(&format!("#define LP_RUN_BYTES ({}u)", RUN_BYTES)), "LP_RUN_BYTES is RUN_BYTES");
    assert!(src.contains(&format!("#define LP_SLOTS ({}u / UKL_SIMD_WIDTH)", TG)), "LP_SLOTS is TG's SIMD groups");
    assert_eq!(RUN_SLOTS as u64, TG / 32);
    assert!(src.contains("UKL_STATIC_ASSERT(sizeof(LpRun) <= LP_RUN_BYTES"), "the run's size is checked");
    assert!(src.contains("lp_run_slot(AT(uint8_t, LPRUN), g.tile, g.lid)"), "a SIMD group's run by its tile");
}

/// The expansion's loops are bounded by the unit's work (`LP_UNIT_WORK`, `lp_spend`): a run that does not progress
/// (a write lost, a piece posted again and again) is refused for work, never a launch that holds the GPU. One held
/// for minutes restarted a Mac (2026-11-24). The two loops with no count in their heads count in their bodies: a
/// piece's rounds, and the queue's items, each spending work.
#[test]
fn the_expansions_loops_are_bounded_by_the_units_work() {
    let src = source();
    assert!(src.contains("#define LP_UNIT_WORK ("), "the unit's work bound");
    let forever = src.matches("for (;;)").count() + src.matches("while (true)").count() + src.matches("while (1)").count();
    assert_eq!(forever, 2, "two loops with no count in their heads: a piece's rounds and the queue's");
    let queue = src.split("for (;;)").nth(2).expect("the queue's loop");
    let body = &queue[..queue.find("\n    }\n").expect("its end")];
    assert!(body.contains("lp_spend(a, trn, LP_ITEM_WORK)"), "the queue's loop spends work per item");
    assert!(src.contains("if ((step & 15u) == 0u && ukl_atomic_load(&a.ctl[2]) != 0u)"), "a refused unit's pieces stop at their first step");
    assert!(src.contains("for (uint32_t turn = 0u; ; turn++)") && src.contains("if (turn == LP_WORK)"), "lp_lex's turns are bounded");
    assert!(src.contains("if (++rounds >= LP_WORK"), "a piece's rounds are bounded");
}

/// The setup's Metal source splits one unit a barriered phase function (decision 0571), and one a large body called at
/// several sites (decision 0672): the program's phases (`pp_step`, decision 0653), the stages it calls and
/// `pp_advance`, which calls it, calls 5 deep: a tape read is two calls, `clif_tape_<name>` and
/// 16z's block (`clif_z_block`; decision 0576 checks 16z in the public read, with no `_z` read between); the small
/// tape reads stay in their callers. The loop's look for a `(` (`lp_next_is_lparen`) is called, never inlined
/// (inlined, it crashed the M3's back end); small, it is kept in its callers' units.
#[test]
fn the_setup_splits_one_unit_a_barriered_function() {
    let msl = rerail_ukl::metal_source(&rerail_ukl::Target::named("apple7").unwrap(), "pp_run", source()).expect("the setup's source preprocesses");
    let split = rerail_ukl::split::split(&msl).expect("the setup has barriered functions");
    for (l, units) in split.levels().iter().enumerate() {
        eprintln!("level {l}: {}", units.iter().map(|u| format!("{}({})", u.name, u.calls.join(","))).collect::<Vec<_>>().join(" "));
    }
    eprintln!("local: {}; depth {}", split.local.join(" "), split.depth);
    assert!(split.units.iter().any(|u| u.name == "pp_step"), "pp_step is a unit");
    assert!(split.local.iter().any(|n| n.starts_with("node_")), "the tape's reads stay local");
    // The look for a `(` is called, never inlined (inlined with the node frames' reads it crashed the M3's back end):
    // a `UKL_ONCE` function, a unit of its own or, small and barrier-free, kept noinline in its callers' units.
    assert!(split.units.iter().any(|u| u.name == "lp_next_is_lparen") || split.local.iter().any(|n| n == "lp_next_is_lparen"), "the look for a `(` is called");
    // The loop's phase is a unit of its own (`lp_loop_run`): inlined into `pp_step`, the M3's back end miscompiled it.
    assert!(split.units.iter().any(|u| u.name == "lp_loop_run"), "the loop's phase is a unit");
    // Compiled once and called, each its own unit (decision 0672): inlined at their several sites, their loop exits
    // counted past 255 in `lp_piece` and `pp_step`, which the M1's back end miscompiles. The rare cases those
    // reached inlined (a region's growth, a line splice's end) are calls kept in their callers' units.
    for once in ["lp_substitute", "lp_call_next", "p1_if_line", "p1_find_header"] {
        assert!(split.units.iter().any(|u| u.name == once), "{once} is a unit");
    }
    for rare in ["lp_grow", "splice_end"] {
        assert!(split.local.iter().any(|n| n == rare), "{rare} is called in its callers' units");
    }
    // The stages (`ukl/stages.h`: 4 of the scans, 3 of the segmented scans, 4 of the sort), the loop's phase and the
    // four above, `pp_step` and `pp_advance`, which runs the afters: `pp_advance`, `pp_step`, `lp_loop_run`,
    // `lp_call_next`, `lp_substitute` and its stage calls 7 deep.
    assert_eq!(split.units.len(), 18);
    assert_eq!(split.depth, 7);
    assert!(split.kernel.contains("kernel void pp_run") && split.units.iter().all(|u| !u.source.contains("kernel void")));
}

/// The evaluator's kernels compile (a failed compile is fatal: its message is the panic's).
#[test]
fn the_source_compiles() {
    // `RERAIL_PP_DUMP`: the source as Metal's front end takes it, written there (its errors' lines).
    if let Some(path) = std::env::var_os("RERAIL_PP_DUMP") {
        std::fs::write(path, rerail_ukl::metal_source(&rerail_ukl::Target::named("apple7").unwrap(), "pp_run", source()).unwrap_or_else(|e| e.to_string())).unwrap();
    }
    for name in ["pp_run", "pp_rebase", "pb_search", "pb_expand", "pd_relex_test"] {
        kernel(name);
    }
}

/// §5 of decision 0468: the evaluator's source is UKL for every backend, not Metal's alone: for
/// OpenCL (its SIMD group emulated, width 32) it is OpenCL C 1.2 that clang's front end accepts,
/// so it holds no C++ and every function that reaches a SIMD-group operation takes its state
/// (`__UKL_PARAMS`). The OpenCL gate of decision 0440 §3.5.
#[test]
fn the_source_is_opencl_c_1_2() {
    use rerail_ukl::{Backend, Preprocess};
    let main = "/<kernels>/pp.cl";
    let src = Preprocess::new(Backend::OpenCl).define("UKL_SIMD_WIDTH", Some("32u")).file(main, source()).run(main).unwrap_or_else(|e| panic!("{e}"));
    let file = std::env::temp_dir().join(format!("rerail-pp-cl-{}.cl", std::process::id()));
    std::fs::write(&file, &src).unwrap();
    let Ok(o) = std::process::Command::new("clang").args(["-x", "cl", "-cl-std=CL1.2", "-fsyntax-only", "-Werror=implicit-function-declaration", "-ferror-limit=40"]).arg(&file).output() else {
        eprintln!("no clang here: the evaluator preprocessed for OpenCL, not compiled");
        return;
    };
    let errors = String::from_utf8_lossy(&o.stderr).lines().filter(|l| l.contains("error")).take(40).collect::<Vec<_>>().join("\n");
    if o.status.success() {
        std::fs::remove_file(&file).ok();
    }
    assert!(o.status.success(), "the evaluator is not OpenCL C 1.2 ({}):\n{errors}", file.display());
}
