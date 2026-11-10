//! Gated regions on `.rr`'s grammar (tier 1's `gating_tests` has the small ones): its `\\u`
//! check behind its gate.

use super::native_cpu_block::State;
use super::native_ir::{BlockNative, Pass};

/// Each pass's regions, as the whole program's values: the gate, and what it skips (its
/// members, and the vector values only they read).
fn regions(native: &BlockNative) -> Vec<(usize, Vec<usize>)> {
    let passes = native.passes();
    let mut out = Vec::new();
    for (k, pass) in passes.passes.iter().enumerate() {
        let Pass::Forward { body, .. } = pass else { continue };
        let origin = &passes.origins[k];
        let plan = State::of(body).plan;
        for r in &plan.regions {
            out.push((origin[r.gate] as usize, r.members.iter().chain(&r.vectors).map(|&m| origin[m] as usize).collect()));
        }
    }
    out
}

#[test]
fn rr_gates_its_unicode_escape_check() {
    let c = super::rr_parser::compiled().expect("compiles");
    let regions = regions(&c.native);
    let op = |name: &str| usize::from(c.values.iter().find(|v| v.name == name).unwrap_or_else(|| panic!("no `{name}`")).op);
    for name in ["h4", "b_short", "b_four", "b_five", "b_six", "u_ok"] {
        assert!(regions.iter().any(|(_, m)| m.contains(&op(name))), "`{name}` is not gated: {} regions", regions.len());
    }
}

#[test]
#[ignore = "a dump: run by hand"]
fn dump_rr_unicode() {
    let c = super::rr_parser::compiled().expect("compiles");
    let passes = c.native.passes_with(&c.native.pure());
    let names = ["uesc", "h4", "u_four", "u_brace", "b_short", "b_four", "b_six", "u_ok"];
    for (k, pass) in passes.passes.iter().enumerate() {
        let Pass::Forward { body, .. } = pass else { continue };
        let origin = &passes.origins[k];
        let plan = State::of(body).plan;
        let local = |name: &str| c.values.iter().find(|v| v.name == name).and_then(|v| origin.iter().position(|&o| o as usize == usize::from(v.op)));
        for name in names {
            let Some(x) = local(name) else { continue };
            let region: Vec<usize> = plan.regions.iter().enumerate().filter(|(_, r)| r.members.contains(&x) || r.vectors.contains(&x)).map(|(i, _)| i).collect();
            let users: Vec<String> = body.insts.iter().enumerate().filter(|(_, i)| i.operands().contains(&x)).map(|(u, i)| format!("%{u} {i:?} lane {} mask {} region {:?}", plan.lanes[u], plan.mask[u], plan.region_of[u])).collect();
            eprintln!("pass {k} `{name}` %{x} {:?} lane {} mask {} regions {region:?} users {users:?}", body.insts[x], plan.lanes[x], plan.mask[x]);
        }
        for (i, r) in plan.regions.iter().enumerate() {
            eprintln!("pass {k} region {i}: gate %{} {:?}, {} members, {} vectors", r.gate, body.insts[r.gate], r.members.len(), r.vectors.len());
        }
    }
}
