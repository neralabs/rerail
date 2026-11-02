//! The cross-file model as one object: the frozen layers (base, cluster overlays), the
//! worktrees over them, M2 for the small plans; `CrossFile` is the daemon's interface
//! (spikes/facts-a/src/daemon/crossfile.rs): `apply(worktree, members, delta) -> FindingsDelta`,
//! with contents named by their blake3 hex (the daemon's store maps `SourceId` to it).
use super::m2::M2;
use super::wire::{self, Layer, Shared, StepOut, Top, DEAD_CODE, FAMILIES};
use crate::ygraph::world::Below;
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
    pub m2: M2,
    pub wts: BTreeMap<u32, Wt>,
    /// The daemon's worktree names.
    pub names: BTreeMap<String, u32>,
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
    /// A worktree with `members`, over layer `layer`: the step from the layer's membership, and
    /// the M2 plans from scratch.
    pub fn create(&mut self, w: u32, layer: usize, members: &[(String, String)]) -> StepOut {
        let World { sh, layers, m2, wts, .. } = self;
        let b = below(layers, layer);
        let (top, mut st) = wire::over(sh, &b, &layers[layer], members);
        let t = std::time::Instant::now();
        let ms = m2.add(w, members);
        st.m2_ms = t.elapsed().as_secs_f64() * 1e3;
        st.delta.extend(ms.delta.iter().cloned());
        st.m2 = Some(ms);
        st.ms += st.m2_ms;
        wts.insert(w, Wt { layer, top, epoch: 1 });
        st
    }

    /// One event in worktree `w`.
    pub fn apply(&mut self, w: u32, changes: &[(String, Option<String>, Option<String>)]) -> StepOut {
        let World { sh, layers, m2, wts, .. } = self;
        let wt = wts.get_mut(&w).unwrap_or_else(|| panic!("worktree {w} unknown"));
        let b = below(layers, wt.layer);
        let mut st = wire::step(sh, &b, &mut wt.top, changes);
        let t = std::time::Instant::now();
        let ms = wire::m2_step(m2, w, changes);
        st.m2_ms = t.elapsed().as_secs_f64() * 1e3;
        st.delta.extend(ms.delta.iter().cloned());
        st.m2 = Some(ms);
        st.ms += st.m2_ms;
        wt.epoch += 1;
        st
    }

    /// Live runtime refusals of `w`: the reference graph refused or incomplete, a member without
    /// exports, an M2 plan's refusal.
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
            out.push(format!("{DEAD_CODE}, {FAMILIES}: no exports for {} {}", crate::y2::facts::string(*p), crate::y2::facts::string(*c)));
        }
        for (k, r) in self.m2.results[&w].iter().enumerate() {
            if !r.refusals.is_empty() {
                out.push(format!("{}: {} refusals", self.m2.plans[k].name, r.refusals.len()));
            }
        }
        out
    }

    pub fn forget(&mut self, w: u32) {
        self.wts.remove(&w);
        self.m2.forget(w);
    }

    /// Findings held across worktrees (the five plans).
    pub fn held_findings(&self) -> usize {
        let y2: usize = self.wts.values().map(|wt| wt.top.dc.as_ref().map_or(0, |d| d.finding_rows().len()) + wt.top.ef.as_ref().map_or(0, |e| e.finding_rows().len())).sum();
        y2 + self.m2.held()
    }
}

// ------------------------------------------------------------------------------- CrossFile

/// One path whose content changed (`None`: absent before / after); contents by blake3 hex.
pub struct PathChange {
    pub path: String,
    pub old: Option<String>,
    pub new: Option<String>,
}

/// A worktree's current membership, as the cross-file side reads it (path, blake3 hex).
pub trait Members {
    fn for_each(&self, f: &mut dyn FnMut(&str, &str));
}

/// A cross-file finding: the plan (clause) and its row as text.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Finding {
    pub plan: String,
    pub row: String,
}

#[derive(Default, Debug)]
pub struct FindingsDelta {
    pub introduced: Vec<Finding>,
    pub removed: Vec<Finding>,
    /// The cross-file plans' live runtime refusals (current, not a delta).
    pub failures: Vec<String>,
    /// The step's work (per component counts and milliseconds).
    pub work: Option<StepOut>,
}

/// The daemon's interface (P3c): the worktree's membership after the change and the paths that
/// changed; the findings delta. Y3 keeps memberships itself: `members` is read only when the
/// worktree is new (it is then the step from the base's membership to it).
pub trait CrossFile {
    fn name(&self) -> &'static str;
    fn apply(&mut self, worktree: &str, members: &dyn Members, delta: &[PathChange]) -> FindingsDelta;
    fn forget(&mut self, worktree: &str);
    fn held_findings(&self) -> usize;
}

impl CrossFile for World {
    fn name(&self) -> &'static str {
        "y3"
    }

    fn apply(&mut self, worktree: &str, members: &dyn Members, delta: &[PathChange]) -> FindingsDelta {
        let st = match self.names.get(worktree).copied() {
            Some(w) => {
                let ch: Vec<(String, Option<String>, Option<String>)> = delta.iter().map(|c| (c.path.clone(), c.old.clone(), c.new.clone())).collect();
                self.apply(w, &ch)
            }
            None => {
                let w = self.wts.keys().next_back().map_or(1, |x| x + 1).max(self.names.values().max().map_or(1, |x| x + 1));
                self.names.insert(worktree.to_owned(), w);
                let mut m: Vec<(String, String)> = Vec::new();
                members.for_each(&mut |p, h| m.push((p.to_owned(), h.to_owned())));
                self.create(w, 0, &m)
            }
        };
        let w = self.names[worktree];
        let mut out = FindingsDelta { failures: self.failures(w), ..Default::default() };
        for (plan, row, added) in &st.delta {
            let f = Finding { plan: plan.clone(), row: row.clone() };
            if *added {
                out.introduced.push(f);
            } else {
                out.removed.push(f);
            }
        }
        out.work = Some(st);
        out
    }

    fn forget(&mut self, worktree: &str) {
        if let Some(w) = self.names.remove(worktree) {
            World::forget(self, w);
        }
    }

    fn held_findings(&self) -> usize {
        World::held_findings(self)
    }
}

/// A membership given as pairs (tests, harnesses).
pub struct PairMembers<'a>(pub &'a [(String, String)]);

impl Members for PairMembers<'_> {
    fn for_each(&self, f: &mut dyn FnMut(&str, &str)) {
        for (p, h) in self.0 {
            f(p, h);
        }
    }
}
