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
fn device() {
    static CHOSEN: std::sync::Once = std::sync::Once::new();
    CHOSEN.call_once(|| {
        if std::env::var_os("RERAIL_GPU").is_none() {
            // SAFETY: before any device opens (every test asks here first), and every test sets the same values.
            unsafe {
                std::env::set_var("RERAIL_GPU", "cpu");
                std::env::set_var("RERAIL_CPU_EXEC", "vec");
            }
        }
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
/// is registered; a broken one is refused; `run_on` then runs the device's default version.
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
    run_on(&reg, &dir, op, gpu, &c.slots, &[], Tiling { tile: 100, seed: 9 }).unwrap();
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

