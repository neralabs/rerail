//! The cross-file model as one object: the frozen layers (base, cluster overlays) and the
//! worktrees over them. As copied from facts-b, less M2 (the three small plans) and the facts
//! daemon's `CrossFile` interface, which are not ported.
use super::wire::{self, Layer, Shared, StepOut, Top, DEAD_CODE};
use crate::code_rule_dsl::facts::ygraph::world::Below;
use std::collections::BTreeMap;

pub struct Wt {
    /// The layer it sits on (0: the base).
    pub layer: usize,
    pub top: Top,
    pub epoch: u64,
}

pub struct World {
    pub sh: Shared,
    /// 0: the base; then one overlay per cluster (each over the base).
    pub layers: Vec<Layer>,
    pub wts: BTreeMap<u32, Wt>,
}

/// The layers under layer `i`, top first.
pub fn below(layers: &[Layer], i: usize) -> Below<'_> {
    if i == 0 {
        Below::new(vec![&layers[0].y1])
    } else {
        Below::new(vec![&layers[i].y1, &layers[0].y1])
    }
}

impl World {
    /// A worktree with `members`, over layer `layer`: the step from the layer's membership.
    pub fn create(&mut self, w: u32, layer: usize, members: &[(String, String)]) -> StepOut {
        let World { sh, layers, wts } = self;
        let b = below(layers, layer);
        let (top, st) = wire::over(sh, &b, &layers[layer], members);
        wts.insert(w, Wt { layer, top, epoch: 1 });
        st
    }

    /// One event in worktree `w`.
    pub fn apply(&mut self, w: u32, changes: &[(String, Option<String>, Option<String>)]) -> StepOut {
        let World { sh, layers, wts } = self;
        let wt = wts.get_mut(&w).unwrap_or_else(|| panic!("worktree {w} unknown"));
        let b = below(layers, wt.layer);
        let st = wire::step(sh, &b, &mut wt.top, changes);
        wt.epoch += 1;
        st
    }

    /// Live runtime refusals of `w`: the reference graph refused or incomplete, a member without
    /// exports.
    pub fn failures(&self, w: u32) -> Vec<String> {
        let wt = &self.wts[&w];
        let mut out = Vec::new();
        if wt.top.y1.refused() {
            out.push(format!("{DEAD_CODE}: reference graph refused ({:?})", wt.top.y1.counters));
        }
        if wt.top.y1.incomplete() {
            out.push(format!("{DEAD_CODE}: reference graph incomplete ({} open paths)", wt.top.y1.counters.open_paths));
        }
        for (p, c) in wt.top.sym.refused.iter() {
            out.push(format!("{DEAD_CODE}: no exports for {} {}", crate::code_rule_dsl::facts::y2::facts::string(*p), crate::code_rule_dsl::facts::y2::facts::string(*c)));
        }
        out
    }

    pub fn forget(&mut self, w: u32) {
        self.wts.remove(&w);
    }
}
