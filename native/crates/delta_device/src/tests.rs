//! The device's tests: a launch's device descriptor table read by a kernel (E2), and UKL versions run by the
//! executor and checked against their twins (E3), in the host's memory domain and in a device domain of its own.
//!
//! They run on the CPU device's vector path (`RERAIL_GPU=cpu`, `RERAIL_CPU_EXEC=vec`: decision 0567's `ukl_vec`), as
//! the plan checks UKL versions, unless the process names a device itself (`RERAIL_GPU`).

use super::Gpu;
use rerail_blocks::block::kind;
use rerail_blocks::gens::{BODY_AT, Table};
use rerail_blocks::region::pin_block;
use rerail_delta_exec::dir::{BlockId, Directory};
use rerail_delta_exec::op::*;
use rerail_delta_twins::NoParams;
use rerail_delta_twins::samples::{self, SQUARE};

/// The tests' device, chosen before the process opens one: the CPU's vector path, unless `RERAIL_GPU` names another.
pub(crate) fn device() {
    static CHOSEN: std::sync::Once = std::sync::Once::new();
    CHOSEN.call_once(|| {
        if std::env::var_os("RERAIL_GPU").is_none() {
            // SAFETY: before any device opens (every test asks here first), and every test sets the same values.
            unsafe {
                std::env::set_var("RERAIL_GPU", "cpu");
                std::env::set_var("RERAIL_CPU_EXEC", "vec");
            }
        }
        let d = Gpu::host().map(|g| g.devices());
        eprintln!("the tests' device: {d:?} (RERAIL_GPU={:?})", std::env::var_os("RERAIL_GPU"));
    });
}

fn values_block(dir: &mut Directory, v: &[i64]) -> BlockId {
    let id = dir.make(kind::FACTS, samples::vals_bytes(v.len()));
    {
        let b = dir.block(id).unwrap();
        let mut w = pin_block(&b, "fill", true).unwrap();
        samples::vals_mut(w.bytes_mut().unwrap(), v.len()).copy_from_slice(v);
    }
    dir.freeze(id).unwrap();
    id
}

/// A values block's facts: `(position, value)`.
fn values(_: &Table<'_>, _: usize, b: &[u8]) -> Vec<Vec<i64>> {
    samples::vals(b).iter().enumerate().map(|(i, &v)| vec![i as i64, v]).collect()
}

/// `square`: each value squared, by index (the twin's `samples::SQUARE`).
fn square() -> &'static Signature {
    let slots = vec![SlotSpec { name: "in", mode: Mode::Read, kind: kind::FACTS }, SlotSpec { name: "out", mode: Mode::Write, kind: kind::FACTS }];
    let shape = vec![Phase::lanes("map", Found::Fixed, Across::Nothing, Output::ByIndex, &[])];
    Box::leak(Box::new(Signature::new("square", Box::leak(slots.into_boxed_slice()), ParamsLayout::of::<NoParams>(), Box::leak(shape.into_boxed_slice()), Independence::Along("items"), values)))
}

/// `square`'s UKL version: `square_map_items`, the input's count; `square_map`, an item's square, read and written
/// through the device table where the blocks lie. `power` 3 is a broken one, the cube.
fn square_ukl(power: u32) -> &'static str {
    let (n_at, vals_at) = (BODY_AT + 8, BODY_AT + 16);
    let v = if power == 3 { "v * v * v" } else { "v * v" };
    let args = r#"ROBUFFER, uint8_t, t, "[table]", ROBUFFER, uint32_t, slots, "[table]", ROBUFFER, uint8_t, p, "[table]", ARGS, TileP, tile, , RWBUFFER, uint64_t, res, "[1]", GID0, uint32_t, i, "#;
    Box::leak(
        format!(
            r#"
DEFINE_KERNEL2(delta_square, square_map_items, {args})
{{
    if (i != 0u) return;
    CLIF_BLOCK_SPACE const DescRow* src = desc_row(t, slots[0]);
    res[0] = *(CLIF_BLOCK_SPACE const uint64_t*)(src->addr + {n_at}u);
}}
DEFINE_KERNEL2(delta_square, square_map, {args})
{{
    uint32_t k = tile.lo + i;
    if (k >= tile.hi) return;
    CLIF_BLOCK_SPACE const DescRow* src = desc_row(t, slots[0]);
    CLIF_BLOCK_SPACE const DescRow* dst = desc_row(t, slots[1]);
    uint64_t n = *(CLIF_BLOCK_SPACE const uint64_t*)(src->addr + {n_at}u);
    CLIF_BLOCK_SPACE uint8_t* o = (CLIF_BLOCK_SPACE uint8_t*)dst->addr;
    *(CLIF_BLOCK_SPACE uint64_t*)(o + {BODY_AT}u) = 16u + 8u * n;
    *(CLIF_BLOCK_SPACE uint64_t*)(o + {n_at}u) = n;
    int64_t v = ((CLIF_BLOCK_SPACE const int64_t*)(src->addr + {vals_at}u))[k];
    ((CLIF_BLOCK_SPACE int64_t*)(o + {vals_at}u))[k] = {v};
}}
"#
        )
        .into_boxed_str(),
    )
}

const EVERY_BACKEND: Devices = Devices(Devices::CPU.0 | Devices::METAL.0 | Devices::CUDA.0 | Devices::HIP.0 | Devices::OPENCL.0);

/// A launch of `square` over 1,000 values, its blocks placed in `gpu`'s domain.
fn square_case(gpu: &Gpu, dir: &mut Directory) -> Case {
    let v: Vec<i64> = (0..1000).map(|i| (i * 7919 % 5000) - 2500).collect();
    let (i, o) = (values_block(dir, &v), dir.make(kind::FACTS, samples::vals_bytes(v.len())));
    if gpu.domain() != 0 {
        gpu.replicate(dir, i).unwrap();
        gpu.replicate(dir, o).unwrap();
    }
    Case { slots: vec![i, o], params: vec![] }
}

/// E3 on a device: a UKL version is run by the executor (its launch pinned, its device table made, both handed to one
/// fence, its phase's kernels launched by tiles in any order) and checked against the twin at every tiling before it
/// is registered; a broken one is refused; the runner's `run_on` then runs the device's default version.
fn a_ukl_version_checked_in(gpu: &Gpu) {
    let sig = square();
    let mut reg = Registry::default();
    let op = reg.declare(sig).unwrap();
    reg.register_reference(Version::twin(op, "twin", SQUARE, sig.shape)).unwrap();
    let ukl = |name, power| Version { imp: Impl::Ukl { source: square_ukl(power), entry: "square" }, devices: EVERY_BACKEND, status: Status::Default, ..Version::twin(op, name, SQUARE, sig.shape) };
    let case = |dir: &mut Directory| square_case(gpu, dir);
    let tilings = [Tiling::WHOLE, Tiling { tile: 7, seed: 5 }, Tiling { tile: 64, seed: 3 }];
    let Err(e) = reg.check(&ukl("square ukl", 2), &case, &tilings) else { panic!("a UKL version checked with no device") };
    assert!(e.contains("no device was given"), "{e}");
    let Err(e) = reg.check_on(&ukl("cube ukl", 3), gpu, &case, &tilings) else { panic!("the cube checked as the square") };
    assert!(e.contains("wrote other facts than the reference"), "{e}");
    let checked = reg.check_on(&ukl("square ukl", 2), gpu, &case, &tilings).unwrap();
    reg.register(ukl("square ukl", 2), checked).unwrap();

    let mut dir = Directory::default();
    let c = square_case(gpu, &mut dir);
    Runner::of(reg).run_on(&dir, sig, gpu, &c.slots, &[], Tiling { tile: 100, seed: 9 }).unwrap();
    let got = written_facts(sig, &dir, &c.slots).unwrap();
    let mut want: Vec<Vec<i64>> = (0..1000).map(|i: i64| (i * 7919 % 5000) - 2500).enumerate().map(|(k, x)| vec![k as i64, x * x]).collect();
    want.sort();
    assert_eq!(got, vec![want], "run_on ran the device's version");
}

#[test]
fn a_ukl_version_is_checked_against_its_twin_on_the_device() {
    device();
    a_ukl_version_checked_in(&Gpu::host().expect("the tests' device"));
}

/// The same in a device domain of its own: every block a replica there, the table's own block too, uploaded at the
/// handoff and the written slot brought back after.
#[test]
fn a_ukl_version_runs_on_replicas_in_a_device_domain() {
    device();
    a_ukl_version_checked_in(&Gpu::discrete(1).expect("the tests' device"));
}

/// E2 on a device: a kernel follows references through a launch's device descriptor table, reading each reached
/// block where it lies (its header, its reference table, its values) as the CPU's table gives it; an id the launch
/// cannot reach has no row. On the tests' device (`device`).
#[test]
fn a_kernel_follows_references_through_the_device_table() {
    use rerail_gpu::device::ComputeQueue;
    use rerail_gpu::kernels::{alloc, buffer_of, launch, read};
    device();
    let shared = rerail_gpu::shared::get().expect("the tests' device");
    let mut dir = Directory::default();
    let a = values_block(&mut dir, &(0..1000).map(|i| i * 3 + 1).collect::<Vec<_>>());
    let b = values_block(&mut dir, &(0..500).map(|i| -i).collect::<Vec<_>>());
    let c = dir.make(kind::FACTS, 64);
    dir.refer(c, a, BODY_AT).unwrap();
    dir.refer(c, b, BODY_AT).unwrap();
    dir.freeze(c).unwrap();
    let top = dir.make(kind::FACTS, 64);
    dir.refer(top, c, BODY_AT).unwrap();
    let l = dir.read("device probe", &[top]).unwrap();
    let t = l.table();
    let dt = rerail_blocks::gens::device_table(&t, &l.pins(), &shared.device).unwrap();
    let ids = |rows: &[rerail_blocks::gens::Desc]| rows.iter().map(|d| (d.id, d.kind, d.version)).collect::<Vec<_>>();
    assert_eq!(ids(dt.rows()), ids(t.rows()), "the device's table names the blocks the CPU's does");

    // Each query: a block's id and a byte offset in it, of a word the kernel reads through the table.
    let vals_at = |id: BlockId, i: usize| {
        let bytes = t.block(id.0).unwrap();
        (samples::vals(bytes).as_ptr() as usize - bytes.as_ptr() as usize + i * 8) as u64
    };
    let mut queries: Vec<(u32, u64)> = vec![(a.0, vals_at(a, 0)), (a.0, vals_at(a, 999)), (b.0, vals_at(b, 0)), (b.0, vals_at(b, 499))];
    for id in [a, b, c, top] {
        queries.extend([(id.0, 0), (id.0, rerail_blocks::gens::REFS_AT as u64), (id.0, rerail_blocks::gens::BODY_AT as u64)]);
    }
    queries.push((u32::MAX - 1, 0));
    let n = queries.len();
    let q: Vec<u64> = queries.iter().flat_map(|&(id, off)| [u64::from(id), off]).collect();

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ProbeP {
        n: u32,
        pad: u32,
    }
    unsafe impl rerail_gpu::Pod for ProbeP {}
    const KERNEL: &str = r#"
typedef struct ProbeP { uint32_t n; uint32_t pad; } ProbeP;
UKL_REFLECT_STRUCT(ProbeP, n, pad)
DEFINE_KERNEL2(delta_exec_tests, desc_probe, ROBUFFER, uint8_t, t, "[table]", ROBUFFER, uint64_t, q, "[n * 2]", ARGS, ProbeP, p, ,
               RWBUFFER, uint64_t, out, "[n * 3]", GID0, uint32_t, i, )
{
    if (i >= p.n) return;
    CLIF_BLOCK_SPACE const DescRow* r = desc_row(t, (uint32_t)q[i * 2u]);
    UKL_GLOBAL uint64_t* o = out + i * 3u;
    if (r == 0) {
        o[0] = 0; o[1] = 0; o[2] = 0;
        return;
    }
    o[0] = (uint64_t)r->kind + 1u;
    o[1] = r->len;
    o[2] = *(CLIF_BLOCK_SPACE const uint64_t*)(r->addr + q[i * 2u + 1u]);
}
"#;
    let src = format!("#include <ukl/backend.h>\n{}{KERNEL}", rerail_blocks::gens::DESC_READS);
    let program = rerail_gpu::kernels::program("desc_probe.uglykl", Box::leak(src.into_boxed_str())).unwrap_or_else(|e| panic!("the kernel compiles: {e}"));
    let probe = program.function("desc_probe").unwrap();
    let qb = buffer_of(&q).unwrap();
    let out = alloc::<u64>(n * 3).unwrap();
    // What each query must read, from the CPU's table, before the launch is the device's.
    let want: Vec<[u64; 3]> = queries
        .iter()
        .map(|&(id, off)| match (t.row(id), dt.rows().iter().find(|d| d.id == id)) {
            (Some(d), Some(dd)) => {
                let bytes = t.block(id).unwrap();
                [u64::from(d.kind) + 1, dd.len, u64::from_le_bytes(bytes[off as usize..][..8].try_into().unwrap())]
            }
            _ => [0, 0, 0],
        })
        .collect();
    // E1: the launch's pins are the device's until its work completes (the engine's watched queue completes the fence
    // with the command buffer; here, after it).
    // The kernel binds the table's buffer and reaches the blocks' (device handles, which the fence's uses keep).
    let (table_buffer, table_at, reached) = (dt.buffer.clone(), dt.at, dt.reached.clone());
    let fence = rerail_blocks::ledger::Fence::new();
    dt.to_fence(&fence, "device probe").unwrap();
    l.to_fence(&fence, "device probe", 0).map_err(|b| b.1).unwrap();
    assert!(dir.launch("rival", &[(top, true)]).is_err(), "a block the device reads is not written meanwhile");
    let mut cq = ComputeQueue::new(&shared.queue);
    let mut run = launch(&cq, probe).buffer_at("t", &table_buffer, table_at).buffer("q", &qb).value("p", &ProbeP { n: n as u32, pad: 0 }).buffer("out", &out);
    for r in &reached {
        run = run.uses_read(r);
    }
    run.run(&[(n as u64).div_ceil(64)], &[64]).unwrap_or_else(|e| panic!("{e}"));
    cq.flush().await_().unwrap();
    fence.mark_committed();
    fence.complete();
    assert!(dir.launch("next", &[(top, true)]).is_ok(), "the device's pins end with its fence");
    let got: Vec<u64> = read(&out, n * 3);
    for (k, &(id, off)) in queries.iter().enumerate() {
        assert_eq!(&got[k * 3..][..3], &want[k], "block #{id}, byte {off}: read through the device's table as in place");
    }
}


/// Every operation's UKL version, checked on the tests' device against its twin and registered: a runner there.
#[test]
fn every_ukl_version_checks_against_its_twin() {
    device();
    let gpu = Gpu::host().expect("the tests' device");
    Runner::on(Box::new(gpu)).unwrap_or_else(|e| panic!("{e}"));
}

/// l0's cold boot run on the tests' device, and on runners that pick the device or the CPU for each launch by a seed:
/// each one's K-form is the CPU run's, by meaning.
#[test]
fn the_boot_run_on_the_device_or_mixed_is_the_cpus() {
    use rerail_delta_exec::boot::cold_k_form;
    use rerail_delta_exec::contract::Bench;
    device();
    let base = |name: &str| std::env::temp_dir().join(format!("rerail-boot-{}-{name}", std::process::id()));
    let want = cold_k_form(&mut Bench::new().unwrap(), &base("cpu")).unwrap_or_else(|e| panic!("{e}"));
    assert!(want.iter().map(|r| r.len()).sum::<usize>() > 0, "l0 has a K-form");
    let on = Runner::on(Box::new(Gpu::host().expect("the tests' device"))).unwrap_or_else(|e| panic!("{e}"));
    let got = cold_k_form(&mut Bench::on(on), &base("device")).unwrap_or_else(|e| panic!("{e}"));
    assert!(got == want, "the device's K-form is the CPU's");
    for seed in 1..=3u64 {
        let mut bench = Bench::on(Runner::mixed(Box::new(Gpu::host().expect("the tests' device")), seed).unwrap_or_else(|e| panic!("{e}")));
        let got = cold_k_form(&mut bench, &base(&format!("mixed-{seed}"))).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        let [cpu, dev] = bench.ops.mixed_launches().expect("a mixed runner");
        eprintln!("seed {seed}: {cpu} launches on the CPU, {dev} on the device");
        assert!(cpu > 0 && dev > 0, "seed {seed}: launches on both ({cpu} on the CPU, {dev} on the device)");
        assert!(got == want, "seed {seed}: a mixed run's K-form is the CPU's ({cpu} launches on the CPU, {dev} on the device)");
    }
}

/// The harness's laws for L1 over l0's chain of edits on disk (`harness_over_edits`: L1, L2, L3, L4 and L5, its cache
/// dropped at a step each seed picks, and D1 under a changing want) on the tests' device and on a mixed runner.
#[test]
fn the_harness_passes_l1_over_edits_of_l0_on_the_device_or_mixed() {
    use rerail_delta_exec::boot::harness_over_edits;
    use rerail_delta_exec::contract::Bench;
    device();
    let base = |name: &str| std::env::temp_dir().join(format!("rerail-harness-{}-{name}", std::process::id()));
    let on = Runner::on(Box::new(Gpu::host().expect("the tests' device"))).unwrap_or_else(|e| panic!("{e}"));
    let mut bench = Bench::on(on);
    harness_over_edits(&mut bench, &base("device"), false, 1..=2).unwrap_or_else(|e| panic!("the device: {e}"));
    harness_over_edits(&mut bench, &base("device-want"), true, 4..=4).unwrap_or_else(|e| panic!("the device, a want: {e}"));
    for seed in 1..=2u64 {
        let mut bench = Bench::on(Runner::mixed(Box::new(Gpu::host().expect("the tests' device")), seed).unwrap_or_else(|e| panic!("{e}")));
        harness_over_edits(&mut bench, &base(&format!("mixed-{seed}")), seed == 2, seed..=seed).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        let [cpu, dev] = bench.ops.mixed_launches().expect("a mixed runner");
        assert!(cpu > 0 && dev > 0, "seed {seed}: launches on both ({cpu} on the CPU, {dev} on the device)");
    }
}

/// L6, L7, L8 and P1's drivers (`rerail_delta_exec::laws`) on the tests' device and on a mixed runner: each gives the
/// CPU's verdicts, work and launches (the CPU's tests hold those to the laws).
#[test]
fn the_laws_hold_on_the_device_or_mixed() {
    use rerail_delta_exec::contract::Bench;
    use rerail_delta_exec::laws::{coalescing, composition_and_cutoff, edit_among};
    device();
    let base = |name: &str| std::env::temp_dir().join(format!("rerail-laws-dev-{}-{name}", std::process::id()));
    let versions = rerail_delta_exec::boot::chain_versions();
    let versions = versions.each_ref().map(String::as_str);
    let laws = |bench: &mut Bench, name: &str| {
        let c = composition_and_cutoff(bench, &base(&format!("{name}-l6")), &versions).unwrap_or_else(|e| panic!("{name}: {e}"));
        let among = [0, 3].map(|n| edit_among(bench, &base(&format!("{name}-among-{n}")), n).unwrap_or_else(|e| panic!("{name}: {e}")));
        let l8 = coalescing(bench, &base(&format!("{name}-l8"))).unwrap_or_else(|e| panic!("{name}: {e}"));
        (c.same_as_cold, (c.touched.asked, c.touched.launches, c.touched.work), among, l8)
    };
    let want = laws(&mut Bench::new().unwrap(), "cpu");
    assert!(want.0.iter().all(|&s| s), "the CPU's runs are the cold composed runs");
    let on = Runner::on(Box::new(Gpu::host().expect("the tests' device"))).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(laws(&mut Bench::on(on), "device"), want, "the device's are the CPU's");
    let mut bench = Bench::on(Runner::mixed(Box::new(Gpu::host().expect("the tests' device")), 7).unwrap_or_else(|e| panic!("{e}")));
    assert_eq!(laws(&mut bench, "mixed"), want, "a mixed runner's are the CPU's");
    let [cpu, dev] = bench.ops.mixed_launches().expect("a mixed runner");
    assert!(cpu > 0 && dev > 0, "launches on both ({cpu} on the CPU, {dev} on the device)");
}

/// The boot chain (0686: Shape and Invariants, §3) on the tests' device and on three mixed runners: cold, unchanged,
/// comments only and a rule added, each from the last run's pins. Every run shows what the CPU's must
/// (`check_boot_chain`), and its K-form, its pins' changes and its launches are the CPU run's.
#[test]
fn the_boot_runs_as_a_delta_on_the_device_or_mixed() {
    use rerail_delta_exec::boot::{Ran, boot_chain, chain_versions, check_boot_chain};
    use rerail_delta_exec::contract::Bench;
    device();
    let base = |name: &str| std::env::temp_dir().join(format!("rerail-chain-{}-{name}", std::process::id()));
    let versions = chain_versions();
    let versions = versions.each_ref().map(String::as_str);
    let chain = |bench: &mut Bench, name: &str| -> Vec<Ran> {
        let runs = boot_chain(bench, &base(name), &versions).unwrap_or_else(|e| panic!("{name}: {e}"));
        let atoms = rerail_delta_exec::sym::Atoms(runs[0].after.atoms.block);
        check_boot_chain(&runs, |x| atoms.find(&bench.dir, x));
        runs
    };
    let want = chain(&mut Bench::new().unwrap(), "cpu");
    let same = |name: &str, got: &[Ran]| {
        for (k, (g, w)) in got.iter().zip(&want).enumerate() {
            assert!(g.kform == w.kform, "{name}, run {k}: the K-form is the CPU's");
            assert_eq!(g.launches, w.launches, "{name}, run {k}: the CPU's launches");
            assert_eq!(g.before.as_ref() == Some(&g.after), w.before.as_ref() == Some(&w.after), "{name}, run {k}: the identity where the CPU's is");
        }
    };
    let on = Runner::on(Box::new(Gpu::host().expect("the tests' device"))).unwrap_or_else(|e| panic!("{e}"));
    same("the device", &chain(&mut Bench::on(on), "device"));
    for seed in 1..=3u64 {
        let mut bench = Bench::on(Runner::mixed(Box::new(Gpu::host().expect("the tests' device")), seed).unwrap_or_else(|e| panic!("{e}")));
        let runs = chain(&mut bench, &format!("mixed-{seed}"));
        let [cpu, dev] = bench.ops.mixed_launches().expect("a mixed runner");
        assert!(cpu > 0 && dev > 0, "seed {seed}: launches on both ({cpu} on the CPU, {dev} on the device)");
        same(&format!("seed {seed}"), &runs);
    }
}

/// l0's cold boot run on the tests' device, `RERAIL_BOOT_RUNS` times (default 1) on one runner: what a profiler
/// samples the device's boot run by. Prints the runner's making (its kernels compiled and checked) and each run's time.
#[test]
#[ignore = "a profiler's load, not a check"]
fn the_boot_run_on_the_device_repeated() {
    use rerail_delta_exec::boot::cold_k_form;
    use rerail_delta_exec::contract::Bench;
    device();
    let n: usize = std::env::var("RERAIL_BOOT_RUNS").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    let base = std::env::temp_dir().join(format!("rerail-boot-device-profile-{}", std::process::id()));
    let t = std::time::Instant::now();
    let mut ops = Runner::on(Box::new(Gpu::host().expect("the tests' device"))).unwrap_or_else(|e| panic!("{e}"));
    eprintln!("the device's runner: {:?}", t.elapsed());
    let t = std::time::Instant::now();
    for k in 0..n {
        let mut bench = Bench::on(ops);
        cold_k_form(&mut bench, &base.join(k.to_string())).unwrap_or_else(|e| panic!("{e}"));
        ops = bench.ops;
    }
    eprintln!("{n} boot runs: {:?} each", t.elapsed() / n.max(1) as u32);
}
