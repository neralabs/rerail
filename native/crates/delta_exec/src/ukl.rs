//! The operations' UKL versions (decision 0686; the runner's step 4): each operation's kernels, written in
//! `delta_exec/ukl/<module>.uglykl` over the delta library (`<ukl/delta/*.h>`), checked on a device against its twin
//! and registered there, so a runner on that device runs them (`Runner::on`).
//!
//! A version's source is its module's file after its params' declarations (`ParamsLayout::ukl`). The block layouts
//! its kernels read are the twins' own constants, written into `<ukl/delta/layout.h>` (`layout_h`, generated and held
//! equal by a test), so each layout keeps one definition.

use crate::cache::{CACHE_COMMIT, CACHE_OPEN, OpCache};
use crate::contract::push_layer;
use crate::dir::Directory;
use crate::drop::{DROP_PARTS, Parts};
use crate::trace::{RollUp, Trace};
use core::mem::offset_of;
use rerail_blocks::gens::BODY_AT;
use rerail_blocks::layer::{ColType, LayerHead, MAX_LAYERS, NONE, RunsHead, Shape, ShapeW};
use rerail_blocks::relation::{FAR_BASE, Relation};
use crate::op::{Case, Devices, Impl, Kernels, Registry, Runner, Signature, Status, Tiling};
use rerail_delta_twins::Params;
use rerail_delta_twins::cache::{self as cache_twin, CacheP};
use rerail_delta_twins::drop::{self as drop_twin, DropP};

/// Every backend a UKL version runs on.
const EVERY_BACKEND: Devices = Devices(Devices::CPU.0 | Devices::METAL.0 | Devices::CUDA.0 | Devices::HIP.0 | Devices::OPENCL.0);

/// The tilings a version is checked at: whole, and in tiles run out of order.
const TILINGS: [Tiling; 2] = [Tiling::WHOLE, Tiling { tile: 3, seed: 7 }];

/// One operation's UKL version: its signature, its module's kernels, and the case it is checked on.
struct Ukl {
    sig: &'static Signature,
    module: &'static str,
    case: fn(&mut Directory) -> Case,
}

/// The modules' kernels, by name.
const MODULES: &[(&str, &str)] = &[("cache", include_str!("../ukl/cache.uglykl")), ("drop", include_str!("../ukl/drop.uglykl"))];

/// The operations with a UKL version.
const VERSIONS: &[Ukl] = &[
    Ukl { sig: &CACHE_OPEN, module: "cache", case: cache_open_case },
    Ukl { sig: &CACHE_COMMIT, module: "cache", case: cache_commit_case },
    Ukl { sig: &DROP_PARTS, module: "drop", case: drop_parts_case },
];

/// Each operation's UKL version checked on `device` against its twin, at `TILINGS`, and registered as the device's
/// default.
pub(crate) fn register_on(reg: &mut Registry, device: &dyn Kernels) -> Result<(), String> {
    for &(module, text) in MODULES {
        let params: Vec<String> = VERSIONS.iter().filter(|u| u.module == module && u.sig.params.size > 0).map(|u| u.sig.params.ukl()).collect();
        let mut decls: Vec<String> = Vec::new();
        for p in params {
            if !decls.contains(&p) {
                decls.push(p);
            }
        }
        let source: &'static str = Box::leak(format!("{}\n{text}", decls.join("\n")).into_boxed_str());
        for u in VERSIONS.iter().filter(|u| u.module == module) {
            let op = reg.find(u.sig).ok_or_else(|| format!("{}: not declared", u.sig.name))?;
            let twin = reg.reference(op).ok_or_else(|| format!("{}: no twin", u.sig.name))?;
            let name: &'static str = Box::leak(format!("{} ukl", u.sig.name).into_boxed_str());
            let v = crate::op::Version { name, imp: Impl::Ukl { source, entry: u.sig.name }, devices: EVERY_BACKEND, status: Status::Default, ..*twin };
            let checked = reg.check_on(&v, device, &u.case, &TILINGS)?;
            reg.register(v, checked)?;
        }
    }
    Ok(())
}

/// A case's params' bytes.
fn params<P: Params>(p: P) -> Vec<u8> {
    let mut b = vec![0; P::SIZE];
    p.write(&mut b);
    b
}

/// A cache committed at state `0xd1`, opened for another (`0xd2`): reset to ∅, then marked open.
fn cache_open_case(dir: &mut Directory) -> Case {
    let cpu = Runner::new().expect("the CPU's runner");
    let c = OpCache::new(dir);
    c.open(&cpu, dir, 0).and_then(|_| c.commit(&cpu, dir, 0xd1)).expect("a committed cache");
    Case { slots: vec![c.0], params: params(CacheP { state: 0xd2 }) }
}

/// A cache opened by a run, committed at the state it made (`0xd1`).
fn cache_commit_case(dir: &mut Directory) -> Case {
    let cpu = Runner::new().expect("the CPU's runner");
    let c = OpCache::new(dir);
    c.open(&cpu, dir, 0).expect("an open cache");
    Case { slots: vec![c.0], params: params(CacheP { state: 0xd1 }) }
}

/// A trace of two layers partitioned on its first column, the second killing a row of the first, its rows in
/// partitions 3 and 1 (listed out of order) dropped with sign -1: three rows, partition 1's then 3's, the kill skipped.
fn drop_parts_case(dir: &mut Directory) -> Case {
    let cpu = Runner::new().expect("the CPU's runner");
    let mut t = Trace::new(&Shape::run(vec![ColType::I32, ColType::I64], vec![0, 1], Some(0))).expect("a trace").with_roll_up(RollUp::Never);
    push_layer(&cpu, dir, &mut t, &[(vec![1, 10], 1), (vec![1, 11], 1), (vec![2, 20], 1), (vec![3, 30], 1)]).expect("a layer");
    let head = push_layer(&cpu, dir, &mut t, &[(vec![1, 11], -1), (vec![3, 31], 1), (vec![4, 40], 1)]).expect("a layer");
    let runs = t.open(dir, 16);
    let parts = Parts::write(dir, 2, |put| [3, 1].into_iter().for_each(put)).expect("a parts block");
    Case { slots: vec![head, parts.id(), runs], params: params(DropP { head: u64::from(head.0), diff: -1 }) }
}

/// The block layouts the kernels read, by name: the twins' own constants.
pub fn layout() -> Vec<(&'static str, usize)> {
    vec![
        ("BODY_AT", BODY_AT),
        ("CACHE_BYTES", cache_twin::CACHE_BYTES),
        ("CACHE_STATE_AT", cache_twin::STATE_AT),
        ("CACHE_OPEN_AT", cache_twin::OPEN_AT),
        ("CACHE_HELD_AT", cache_twin::HELD_AT),
        ("CACHE_USED_AT", cache_twin::USED_AT),
        // `rerail_delta_core::runs`, `::layer`, `::relation` and `::trace`.
        ("NONE", NONE as usize),
        ("MAX_LAYERS", MAX_LAYERS),
        ("SHAPE_ARITY_AT", offset_of!(ShapeW, arity)),
        ("SHAPE_PARTITION_AT", offset_of!(ShapeW, partition)),
        ("SHAPE_ORDER_AT", offset_of!(ShapeW, order)),
        ("RUNS_CAP_AT", BODY_AT + offset_of!(RunsHead, cap)),
        ("RUNS_CLAIMED_AT", BODY_AT + offset_of!(RunsHead, claimed)),
        ("RUNS_NEEDED_AT", BODY_AT + offset_of!(RunsHead, needed)),
        ("RUNS_TYPES_AT", BODY_AT + offset_of!(RunsHead, shape) + offset_of!(ShapeW, types)),
        ("RUNS_COL_AT", BODY_AT + offset_of!(RunsHead, col_at)),
        ("RUNS_DIFF_AT", BODY_AT + offset_of!(RunsHead, diff_at)),
        ("LAYER_ROWS_AT", BODY_AT + offset_of!(LayerHead, rows)),
        ("LAYER_PARTS_AT", BODY_AT + offset_of!(LayerHead, parts)),
        ("LAYER_GONE_AT", BODY_AT + offset_of!(LayerHead, gone)),
        ("LAYER_SHAPE_AT", BODY_AT + offset_of!(LayerHead, shape)),
        ("REL_FLAGS_AT", offset_of!(Relation, flags)),
        ("REL_BASE_ROWS_AT", offset_of!(Relation, base_rows)),
        ("REL_BASE_BLOCK_AT", offset_of!(Relation, base_block)),
        ("REL_FAR_BASE", FAR_BASE as usize),
        // `rerail_delta_twins::drop`'s parts block.
        ("DROP_N_AT", drop_twin::N_AT),
        ("DROP_PARTS_AT", drop_twin::PARTS_AT),
    ]
}

/// `<ukl/delta/layout.h>`'s text: `layout`'s constants.
pub fn layout_h() -> String {
    let mut s = String::from(
        "// Generated by native/crates/delta_exec/src/ukl.rs (`layout_h`); do not edit. The block layouts the delta operations'\n\
         // kernels read, each the twins' own constant (decision 0686). `RERAIL_WRITE_LAYOUT=1 cargo test -p rerail-delta-exec\n\
         // --lib ukl::tests::the_layout_header_is_the_twins` writes it again.\n#pragma once\n",
    );
    for (name, v) in layout() {
        s += &format!("#define {name} {v}u\n");
    }
    s
}

#[cfg(test)]
#[path = "ukl_tests.rs"]
mod tests;
