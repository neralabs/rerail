//! The Neural Engine lane against the reference, its writer against coremltools', and its refusals.
//!
//! Shapes are kept small enough to compile in well under a second each, yet above [`super::MIN_FLOPS`] so that Core
//! ML places them on the Neural Engine. The timing tests at the real workloads' sizes are ignored; run each alone:
//! `cargo test -p rerail-accel --no-default-features --features ane ane::tests::time_w1 -- --ignored --nocapture`.

use super::package::{self, Template};
use super::*;
use crate::cpu::tests::{bits, Rng};
use crate::exec::{CpuBackend, Executor, Launch};
use crate::{Batch, Kernel, Lane};

fn ane() -> Ane {
    Ane::new(Arc::new(MemoryStore::default()))
}

fn product(a: &Bits, b: &Bits, min: Option<Vec<u32>>) -> Kernel {
    Kernel::BitProduct { a: a.clone(), b: b.clone(), min: min.map(Into::into) }
}

/// Thresholds around the counts' middle, so that some pairs pass and some do not.
fn thresholds(rng: &mut Rng, b: &Bits, a_density: u64) -> Vec<u32> {
    (0..b.rows)
        .map(|j| {
            let ones: u32 = b.row(j).iter().map(|w| w.count_ones()).sum();
            (ones as u64 * a_density / 16) as u32 + rng.below(3) as u32
        })
        .collect()
}

#[test]
fn writer_matches_coremltools() {
    let want_model = include_bytes!("fixtures/coremltools-2piece.mlmodel");
    let want_weights = include_bytes!("fixtures/coremltools-2piece.weight.bin");
    // The fixture's constants (W0: 128 x 24 at data offset 0x80, W1: 64 x 24 at 0x240) back to b's rows.
    let (n, k0, k1) = (24usize, 128usize, 64usize);
    let bit = |off: usize, e: usize| want_weights[off + e / 8] >> (e % 8) & 1 == 1;
    let mut data = vec![0u64; n * 3];
    for j in 0..n {
        for c in 0..k0 + k1 {
            let on = if c < k0 { bit(0x80, c * n + j) } else { bit(0x240, (c - k0) * n + j) };
            if on {
                data[j * 3 + c / 64] |= 1 << (c % 64);
            }
        }
    }
    let t = Template { m: 64, n, pieces: vec![(0, 2), (2, 3)] };
    let (model, weights) = package::write_model(&t, 3, &data);
    assert_eq!(weights, want_weights, "weight.bin differs from coremltools'");
    assert_eq!(model, want_model, "model.mlmodel differs from coremltools'");
}

#[test]
fn plan_splits_pieces() {
    let mut rng = Rng(7);
    // Sparse and wide: split by the 16,384-bit limit only.
    let b = bits(&mut rng, 40, 20000, 1);
    let t = package::plan(b.rows, b.words, &b.data, 2048).unwrap();
    assert_eq!(t.pieces, vec![(0, 256), (256, b.words)]);
    // Dense: split so that no row has more than 2,048 ones in a piece.
    let b = bits(&mut rng, 40, 8192, 14);
    let t = package::plan(b.rows, b.words, &b.data, 2048).unwrap();
    assert!(t.pieces.len() >= 4, "{:?}", t.pieces);
    for &(w0, w1) in &t.pieces {
        assert!((w1 - w0) * 64 <= package::MAX_K);
        for r in 0..b.rows {
            assert!(b.row(r)[w0..w1].iter().map(|w| w.count_ones()).sum::<u32>() <= package::EXACT);
        }
    }
    // All ones over 20,480 bits: ten pieces of 2,048, more than a template takes.
    let all = Bits::new(4, 320, vec![u64::MAX; 4 * 320]);
    assert!(package::plan(4, 320, &all.data, 2048).unwrap_err().contains("10 pieces"));
}

#[test]
fn half_counts() {
    assert_eq!(half_count(0), Some(0));
    assert_eq!(half_count(0x3c00), Some(1));
    assert_eq!(half_count(0x4000), Some(2));
    assert_eq!(half_count(0x4200), Some(3));
    assert_eq!(half_count(0x6800), Some(2048));
    assert_eq!(half_count(0x6801), None); // 2050: past the exact range
    assert_eq!(half_count(0x3800), None); // 0.5
    assert_eq!(half_count(0xbc00), None); // -1
    assert_eq!(half_count(0x7c00), None); // inf
    for h in 0..=u16::MAX {
        let mut o = [5u32];
        assert_eq!(add_counts(&mut o, &[h]), half_count(h).is_some(), "{h:#x}");
        if let Some(c) = half_count(h) {
            assert_eq!(o[0], 5 + c);
        }
    }
    for v in 0..=2048u32 {
        // fp16 of v, built from f32's bits.
        let f = (v as f32).to_bits();
        let h = if v == 0 { 0 } else { ((f >> 16) & 0x8000) | ((((f >> 23) & 0xff) - 127 + 15) << 10) | ((f >> 13) & 0x3ff) };
        assert_eq!(half_count(h as u16), Some(v));
    }
}

#[test]
fn archive_round_trip() {
    let base = std::env::temp_dir().join(format!("rerail-ane-archive-{}", std::process::id()));
    let src = base.join("src.mlmodelc");
    std::fs::create_dir_all(src.join("weights")).unwrap();
    std::fs::write(src.join("model.mil"), b"program").unwrap();
    std::fs::write(src.join("weights/weight.bin"), vec![7u8; 1000]).unwrap();
    let a = package::archive(&src).unwrap();
    let dst = base.join("dst.mlmodelc");
    package::unarchive(&a, &dst).unwrap();
    assert_eq!(std::fs::read(dst.join("model.mil")).unwrap(), b"program");
    assert_eq!(std::fs::read(dst.join("weights/weight.bin")).unwrap(), vec![7u8; 1000]);
    assert_eq!(package::archive(&dst).unwrap(), a);
    assert!(package::unarchive(&a[..a.len() - 1], &dst).is_err());
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn refusals() {
    let lane = ane();
    let mut rng = Rng(11);
    let a = bits(&mut rng, 100, 2048, 4);
    // Not fixed.
    let b = bits(&mut rng, 200, 2048, 4);
    assert!(lane.refusal(&product(&a, &b, None)).unwrap().contains("not a fixed relation"));
    // Too many pieces: every row all ones over 20,480 bits.
    let wide = bits(&mut rng, 100, 20480, 4);
    let all = Bits::new(64, 320, vec![u64::MAX; 64 * 320]).fixed();
    assert!(lane.refusal(&product(&wide, &all, None)).unwrap().contains("pieces"));
    // Too small for the Neural Engine.
    let small = bits(&mut rng, 8, 128, 4).fixed();
    let a2 = bits(&mut rng, 10, 128, 4);
    assert!(lane.refusal(&product(&a2, &small, None)).unwrap().contains("CPU"));
    // Another form.
    let k = Kernel::Histogram { off: vec![0u32, 1].into(), codes: vec![0u16].into(), width: 1 };
    assert!(!lane.can(&k));
    // Nothing was compiled for these.
    assert!(lane.state.lock().unwrap().loaded.is_empty());
}

#[test]
fn placement_off_the_engine_is_refused() {
    // A tiny product: Core ML's planner puts it on the CPU, and the plan check says so.
    let lane = ane();
    let mut rng = Rng(17);
    let b = bits(&mut rng, 24, 128, 8);
    let t = Template { m: 64, n: 24, pieces: vec![(0, 2)] };
    std::fs::create_dir_all(&lane.dir).unwrap();
    let pkg = lane.dir.join("tiny.mlpackage");
    package::write_package(&pkg, &t, b.words, &b.data).unwrap();
    let compiled = coreml::compile(&pkg).unwrap();
    let r = coreml::check_placement(&compiled, &lane.cfg);
    let _ = std::fs::remove_dir_all(&compiled);
    let e = r.unwrap_err();
    assert!(e.contains("matmul on"), "{e}");
}

#[test]
fn exact_random_shapes() {
    let lane = ane();
    let mut rng = Rng(3);
    let mut ks = Vec::new();
    for _ in 0..2 {
        let width = 512 + rng.below(1536) as usize;
        let n = 128 + rng.below(200) as usize;
        let db = 1 + rng.below(6);
        let b = bits(&mut rng, n, width, db).fixed();
        // Several launches against each constant: one batch call per model, one of them over several tiles.
        for rows in [1 + rng.below(300) as usize, 2048 + rng.below(3000) as usize] {
            let da = 1 + rng.below(6);
            let a = bits(&mut rng, rows, width, da);
            let min = thresholds(&mut rng, &b, da);
            ks.push(product(&a, &b, None));
            ks.push(product(&a, &b, Some(min)));
        }
    }
    let refs: Vec<&Kernel> = ks.iter().collect();
    for k in &refs {
        assert_eq!(lane.refusal(k), None);
    }
    let got = lane.submit(&refs).unwrap();
    for (k, g) in refs.iter().zip(&got) {
        assert_eq!(*g, crate::cpu::run(k));
    }
    assert_eq!(lane.state.lock().unwrap().loaded.len(), 2);
}

#[test]
fn exact_past_2048_and_past_16384() {
    let lane = ane();
    let mut rng = Rng(5);
    // Dense: counts reach about 3,000, so the shared dimension is split by the count bound.
    let b = bits(&mut rng, 96, 4096, 12).fixed();
    let a = bits(&mut rng, 700, 4096, 13);
    // Wide and sparse: 20,000 bits, split by the 16,384-bit limit.
    let wb = bits(&mut rng, 64, 20000, 1).fixed();
    let wa = bits(&mut rng, 300, 20000, 2);
    let ks = [product(&a, &b, None), product(&wa, &wb, None), product(&wa, &wb, Some(vec![2; 64]))];
    let refs: Vec<&Kernel> = ks.iter().collect();
    let got = lane.submit(&refs).unwrap();
    for (k, g) in refs.iter().zip(&got) {
        assert_eq!(*g, crate::cpu::run(k));
    }
    let Output::Counts { data, .. } = &got[0] else { panic!() };
    assert!(*data.iter().max().unwrap() > 2048, "counts past fp16's exact range");
    assert!(lane.prepared(&b).unwrap().pieces >= 2);
    assert_eq!(lane.prepared(&wb).unwrap().pieces, 2);
}

#[test]
fn surfaces_and_arrays_agree_and_surfaces_are_reused() {
    // The IOSurface path (every shape here fits one) and the plain-array path (kept for shapes no IOSurface holds),
    // on the same lane and model, against the reference; a later submission takes its surfaces from the pool.
    let lane = ane();
    let mut rng = Rng(23);
    let b = bits(&mut rng, 140, 3000, 4).fixed();
    let a = bits(&mut rng, 2048 + 900, 3000, 3);
    let ks = [product(&a, &b, None), product(&a, &b, Some(thresholds(&mut rng, &b, 3)))];
    let refs: Vec<&Kernel> = ks.iter().collect();
    let want: Vec<Output> = refs.iter().map(|k| crate::cpu::run(k)).collect();
    assert_eq!(lane.submit(&refs).unwrap(), want);
    assert!(lane.last_split().surfaces);
    let pooled = |l: &Ane| l.pool.lock().unwrap().values().map(Vec::len).sum::<usize>();
    let held = pooled(&lane);
    assert!(held >= 2, "the surfaces went back to the pool");
    assert_eq!(lane.submit(&refs).unwrap(), want);
    assert_eq!(pooled(&lane), held, "the second submission reused them");
    lane.arrays_only.store(true, Ordering::Relaxed);
    assert_eq!(lane.submit(&refs).unwrap(), want);
    assert!(!lane.last_split().surfaces);
}

#[test]
fn overlapped_tiles_agree_with_serial() {
    // Five tiles (the last one short) through the pipeline and one after another, on the same lane, model and
    // surfaces, against the reference; with two launches, so a tile spans both. The pipeline's phases are reported.
    let lane = ane();
    let mut rng = Rng(29);
    let b = bits(&mut rng, 150, 2500, 4).fixed();
    let a1 = bits(&mut rng, 3 * 2048 + 100, 2500, 3);
    let a2 = bits(&mut rng, 2048 + 500, 2500, 3);
    let ks = [product(&a1, &b, None), product(&a2, &b, Some(thresholds(&mut rng, &b, 3)))];
    let refs: Vec<&Kernel> = ks.iter().collect();
    let want: Vec<Output> = refs.iter().map(|k| crate::cpu::run(k)).collect();
    let (got, phases) = lane.submit_timed(&refs).unwrap();
    assert_eq!(got, want);
    let s = lane.last_split();
    assert!(s.surfaces && s.overlap);
    assert_eq!((s.tiles, s.calls), (5, 5));
    assert!(phases.encode > 0.0 && phases.decode > 0.0 && phases.device.is_some_and(|d| d > 0.0), "{phases:?}");
    let pooled = |l: &Ane| l.pool.lock().unwrap().values().map(Vec::len).sum::<usize>();
    assert_eq!(pooled(&lane), 2 * DEPTH * lane.prepared(&b).unwrap().pieces, "both sets went back to the pool");
    lane.set_overlap(false);
    assert_eq!(lane.submit(&refs).unwrap(), want);
    assert!(!lane.last_split().overlap);
}

#[test]
fn store_and_aot_skip_the_compile() {
    let store: Arc<dyn ModelStore> = Arc::new(MemoryStore::default());
    let mut rng = Rng(9);
    let b = bits(&mut rng, 160, 1600, 3).fixed();
    let a = bits(&mut rng, 500, 1600, 3);
    let k = product(&a, &b, None);
    let want = crate::cpu::run(&k);
    let first = Ane::new(store.clone());
    assert_eq!(first.submit(&[&k]).unwrap(), vec![want.clone()]);
    assert_eq!(first.prepared(&b).unwrap().source, "jit");
    drop(first);
    // A second process with the same store: the model is a fact already.
    let second = Ane::new(store);
    assert_eq!(second.submit(&[&k]).unwrap(), vec![want.clone()]);
    assert_eq!(second.prepared(&b).unwrap().source, "store");
    // Ahead of time: compiled by `xcrun coremlcompiler`, as the build step does.
    let work = second.dir.join("aot-test");
    let t = package::plan(b.rows, b.words, &b.data, package::TILE_ROWS).unwrap();
    package::write_package(&work.join("m.mlpackage"), &t, b.words, &b.data).unwrap();
    let st = std::process::Command::new("xcrun")
        .arg("coremlcompiler")
        .arg("compile")
        .arg(work.join("m.mlpackage"))
        .arg(&work)
        .stdout(std::process::Stdio::null())
        .status();
    if !st.is_ok_and(|s| s.success()) {
        eprintln!("xcrun coremlcompiler is not available: the ahead-of-time check is skipped");
        return;
    }
    let archive: Arc<[u8]> = package::archive(&work.join("m.mlmodelc")).unwrap().into();
    let constant = package::write_constant(b.rows, b.words, &b.data);
    let third = Ane::with_aot(Arc::new(MemoryStore::default()), vec![(constant, archive)]);
    assert_eq!(third.submit(&[&k]).unwrap(), vec![want]);
    assert_eq!(third.prepared(&b).unwrap().source, "aot");
}

#[test]
fn embedded_models_load() {
    // Each model the build step embedded (none unless `accel/models/aot.list` names some) is used, not recompiled.
    let lane = ane();
    let mut rng = Rng(19);
    for &(c, _) in aot::AOT {
        let (rows, words, data) = package::read_constant(c).unwrap();
        let b = Bits::new(rows, words, data).fixed();
        let a = bits(&mut rng, 300, words * 64, 3);
        let k = product(&a, &b, None);
        assert_eq!(lane.submit(&[&k]).unwrap(), vec![crate::cpu::run(&k)]);
        assert_eq!(lane.prepared(&b).unwrap().source, "aot");
    }
}

#[test]
fn executor_places_on_the_lane() {
    assert!(backend().is_some_and(|b| b.lane() == Lane::Ane));
    let lane: Arc<dyn Backend> = Arc::new(ane());
    let ex = Executor::with(vec![Arc::new(CpuBackend), lane]);
    let mut rng = Rng(13);
    let b = bits(&mut rng, 200, 1500, 2).fixed();
    let a = bits(&mut rng, 900, 1500, 2);
    let k = product(&a, &b, Some(vec![1; 200]));
    let not_fixed = product(&a, &Bits { fixed: None, ..b.clone() }, None);
    let (done, report) = ex.run(Batch(vec![Launch::on(k.clone(), Lane::Ane), Launch::new(not_fixed.clone())])).unwrap();
    assert_eq!(done[0].lane, Lane::Ane);
    assert_eq!(done[0].out, crate::cpu::run(&k));
    assert_eq!(done[1].lane, Lane::Cpu);
    assert_eq!(report.launches[Lane::Ane.index()], 1);
    assert!(ex.run(Batch(vec![Launch::on(not_fixed, Lane::Ane)])).is_err());
}

// ---------------------------------------------------------------------------------------------------------------
// Timing at the real workloads' sizes (ignored; each alone, within two minutes).

fn time(name: &str, a: Bits, b: Bits, iters: usize) {
    let lane = ane();
    let k = product(&a, &b, None);
    let t0 = Instant::now();
    assert_eq!(lane.refusal(&k), None);
    let prep_wall = t0.elapsed().as_secs_f64();
    let p = lane.prepared(&b).unwrap();
    let want = crate::cpu::run(&k);
    let tiles = a.rows.div_ceil(package::TILE_ROWS);
    println!(
        "{name}: a {} x {} words, b {} rows; {} pieces, {tiles} tiles; units {:.3e}; prepare {:.3} s (write {:.3}, compile {:.3}, plan {:.3}, first load {:.3})",
        a.rows, a.words, b.rows, p.pieces, k.units(), prep_wall, p.write, p.compile, p.plan, p.load,
    );
    // The same lane and model, on IOSurfaces and then on plain arrays.
    for arrays in [false, true] {
        lane.arrays_only.store(arrays, Ordering::Relaxed);
        let first = lane.submit(&[&k]).unwrap();
        assert_eq!(first[0], want, "{name}: exact");
        assert_eq!(lane.last_split().surfaces, !arrays);
        let (mut walls, mut calls, c0) = (Vec::new(), Vec::new(), crate::exec::process_cpu());
        for _ in 0..iters {
            let t = Instant::now();
            let out = lane.submit(&[&k]).unwrap();
            walls.push(t.elapsed().as_secs_f64());
            std::hint::black_box(out);
            calls.push(lane.last_split());
        }
        let cpu = (crate::exec::process_cpu() - c0) / iters as f64;
        let core_ml_cpu = calls.iter().map(|c| c.predict_cpu).sum::<f64>() / iters as f64;
        let s = calls[calls.len() / 2];
        calls.sort_by(|x, y| x.predict.total_cmp(&y.predict));
        walls.sort_by(f64::total_cmp);
        println!(
            "{name} on {}: submit wall min {:.3} ms / median {:.3} ms; host CPU (user+sys) {:.3} ms a submit, of which {:.3} ms in the prediction calls; \
             per unit {:.3e} s (min wall / units); prediction call min {:.3} ms / median {:.3} ms ({} calls, {} tiles), per unit {:.3e} s; \
             a median submit's host expand {:.3} ms, sum {:.3} ms",
            if arrays { "plain arrays" } else { "IOSurfaces" },
            walls[0] * 1e3, walls[walls.len() / 2] * 1e3, cpu * 1e3, core_ml_cpu * 1e3, walls[0] / k.units(),
            calls[0].predict * 1e3, calls[calls.len() / 2].predict * 1e3, s.calls, s.tiles, calls[0].predict / k.units(), s.expand * 1e3, s.sum * 1e3
        );
    }
}

#[test]
#[ignore]
fn time_w1() {
    // The prefilter's overlap: 13,088 documents x 444 rules over 767 rule-name bits (12 words).
    let mut rng = Rng(21);
    let b = bits(&mut rng, 444, 767, 1).fixed();
    let a = bits(&mut rng, 13088, 767, 1);
    time("W1", a, b, 30);
}

#[test]
#[ignore]
fn time_w5() {
    // Name co-occurrence: 2,048 names x 2,048 names over 13,088 document bits (205 words), dense enough that
    // counts pass 2,048 and the shared dimension splits by the count bound.
    let mut rng = Rng(22);
    let b = bits(&mut rng, 2048, 13088, 5).fixed();
    let a = Bits { fixed: None, ..b.clone() };
    time("W5", a, b, 10);
}
