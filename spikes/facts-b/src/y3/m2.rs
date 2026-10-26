//! The repository plans that stay on P3a's M2 (`required-contracts`, `quality-project-taxonomy`,
//! `state-flow-project-1`): resident per-content exports (`hybrid::Exports`, hash-consed and
//! reference-counted by the worktrees holding each content), each plan's last result per
//! worktree, and per event the D1 skip (no export block the plan reads differs at a changed path)
//! or a from-scratch run of the plan (`batch::eval`) over the worktree's members, in process.
//! This is `hybrid::run`'s model as a struct (P3a's open item 1), without its replay driver.
use crate::derive::World;
use crate::gen::batch::{self, Counts};
use crate::gen::hybrid::{self, d1_affected, res_of, show, Exports, Plan, Res};
use crate::gen::lower::{self, Source, R};
use crate::gen::real::{self, Real};
use crate::gen::val;
use crate::store::{self, Cid, Store, Sym};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

pub type Members = BTreeMap<Sym, Cid>;

pub struct M2 {
    pub real: Rc<Real>,
    pub plans: Vec<Plan>,
    pub exports: Rc<RefCell<Exports>>,
    pub world: Rc<RefCell<World>>,
    src: Source,
    sd: PathBuf,
    /// Plans grouped by the graph they share (one `batch::eval` per group).
    units: Vec<Vec<usize>>,
    pub members: BTreeMap<u32, Members>,
    pub results: BTreeMap<u32, Vec<Res>>,
}

/// One worktree event's M2 work.
#[derive(Default, Debug, Clone, serde::Serialize)]
pub struct M2Step {
    pub ran: Vec<String>,
    pub skipped: usize,
    pub d1_ms: f64,
    pub eval_ms: f64,
    pub acquire_ms: f64,
    pub counts: Counts,
    /// (plan, finding row, introduced) of the results that changed
    #[serde(skip)]
    pub delta: Vec<(String, String, bool)>,
}

/// A plan result's findings (rows of its outputs, and its refusals) as text with multiplicity.
pub fn res_rows(r: &Res) -> BTreeMap<String, R> {
    let mut out = BTreeMap::new();
    for (name, m) in &r.outputs {
        for (row, c) in m {
            *out.entry(format!("{name} {}", show(row))).or_default() += c;
        }
    }
    for row in &r.refusals {
        *out.entry(format!("refusal {}", show(row))).or_default() += 1;
    }
    out
}

fn diff_res(plan: &str, old: &Res, new: &Res, out: &mut Vec<(String, String, bool)>) {
    let (a, b) = (res_rows(old), res_rows(new));
    for (k, c) in &a {
        for _ in 0..(*c - b.get(k).copied().unwrap_or(0)).max(0) {
            out.push((plan.to_owned(), k.clone(), false));
        }
    }
    for (k, c) in &b {
        for _ in 0..(*c - a.get(k).copied().unwrap_or(0)).max(0) {
            out.push((plan.to_owned(), k.clone(), true));
        }
    }
}

impl M2 {
    pub fn new(spec: &Path) -> M2 {
        let world = Rc::new(RefCell::new(World::new(Store { ids_only: true, ..Default::default() })));
        val::install(world.clone());
        let real = Rc::new(real::load(spec));
        assert!(real.refused.is_empty(), "M2 plans refused at compile time: {}", serde_json::to_string(&real.refused).unwrap());
        let plans = hybrid::plans(&real);
        let exports = Rc::new(RefCell::new(Exports::new(plans.iter().flat_map(|p| p.reads.iter().copied()).collect())));
        let ex = exports.clone();
        let src: Source = Rc::new(move |e, p, c| ex.borrow().rows(e, p, c));
        let mut units: Vec<Vec<usize>> = Vec::new();
        for (k, p) in plans.iter().enumerate() {
            match units.iter_mut().find(|u| Rc::ptr_eq(&plans[u[0]].graph, &p.graph)) {
                Some(u) => u.push(k),
                None => units.push(vec![k]),
            }
        }
        M2 { real, plans, exports, world, src, sd: store::store_dir(), units, members: BTreeMap::new(), results: BTreeMap::new() }
    }

    pub fn plan_names(&self) -> Vec<String> {
        self.plans.iter().map(|p| p.name.clone()).collect()
    }

    fn ids(&self, path: &str, hash: Option<&str>) -> (Sym, Option<Cid>) {
        let mut w = self.world.borrow_mut();
        let p = w.path(path);
        let c = hash.map(|h| w.store.load(&self.sd, &store::parse_hex(h)));
        (p, c)
    }

    fn acquire(&self, p: Sym, c: Cid) {
        let (path, hash) = {
            let w = self.world.borrow();
            (w.store.syms.str(p).to_owned(), crate::facts::hex(&w.store.contents[c as usize].hash))
        };
        self.exports.borrow_mut().acquire(&self.real, p, c, &path, &hash);
    }

    /// Every plan from scratch over `files`, in plan order.
    pub fn eval_all(&self, files: &[(Sym, Cid)]) -> (Vec<Res>, Counts) {
        let mut out: Vec<Option<Res>> = vec![None; self.plans.len()];
        let mut counts = Counts::default();
        for u in &self.units {
            let names: Vec<&str> = u.iter().map(|k| self.plans[*k].name.as_str()).collect();
            let o = batch::eval(&names, &self.plans[u[0]].graph, &self.src, files);
            counts.add(&o.counts);
            for (k, r) in u.iter().zip(res_of(&o)) {
                out[*k] = Some(r);
            }
        }
        (out.into_iter().map(Option::unwrap).collect(), counts)
    }

    fn files(&self, w: u32) -> Vec<(Sym, Cid)> {
        self.members[&w].iter().map(|(p, c)| (*p, *c)).collect()
    }

    /// A worktree with these members: every content acquired, every plan run.
    pub fn add(&mut self, w: u32, members: &[(String, String)]) -> M2Step {
        let t = Instant::now();
        let m: Members = members.iter().map(|(p, h)| {
            let (p, c) = self.ids(p, Some(h));
            (p, c.unwrap())
        }).collect();
        for (p, c) in &m {
            self.acquire(*p, *c);
        }
        let acquire_ms = t.elapsed().as_secs_f64() * 1e3;
        self.members.insert(w, m);
        let t = Instant::now();
        let (rs, counts) = self.eval_all(&self.files(w));
        let mut st = M2Step { ran: self.plan_names(), acquire_ms, eval_ms: t.elapsed().as_secs_f64() * 1e3, counts, ..Default::default() };
        let empty = Res { outputs: Vec::new(), refusals: Vec::new() };
        for (k, r) in rs.iter().enumerate() {
            diff_res(&self.plans[k].name, &empty, r, &mut st.delta);
        }
        self.results.insert(w, rs);
        st
    }

    /// One event's changes in worktree `w`: D1 per plan, the affected plans from scratch.
    pub fn apply(&mut self, w: u32, changes: &[(String, Option<String>, Option<String>)]) -> M2Step {
        let mut st = M2Step::default();
        let t = Instant::now();
        let mut changed: Vec<(Sym, Option<Cid>, Option<Cid>)> = Vec::new();
        for (path, old, new) in changes {
            let (p, o) = self.ids(path, old.as_deref());
            let (_, n) = self.ids(path, new.as_deref());
            let m = self.members.get_mut(&w).unwrap();
            assert_eq!(m.get(&p).copied(), o, "M2: {path}: old content");
            match n {
                Some(c) => m.insert(p, c),
                None => m.remove(&p),
            };
            if let Some(c) = n {
                self.acquire(p, c);
            }
            changed.push((p, o, n));
        }
        st.acquire_ms = t.elapsed().as_secs_f64() * 1e3;
        let t = Instant::now();
        let ex = self.exports.borrow();
        let ran: Vec<usize> = (0..self.plans.len()).filter(|k| d1_affected(&self.plans[*k].reads, &changed, &|e, p, c| ex.block(e, p, c))).collect();
        drop(ex);
        st.d1_ms = t.elapsed().as_secs_f64() * 1e3;
        st.skipped = self.plans.len() - ran.len();
        if !ran.is_empty() {
            let t = Instant::now();
            let files = self.files(w);
            for u in &self.units {
                let ks: Vec<usize> = u.iter().copied().filter(|k| ran.contains(k)).collect();
                if ks.is_empty() {
                    continue;
                }
                let names: Vec<&str> = ks.iter().map(|k| self.plans[*k].name.as_str()).collect();
                let o = batch::eval(&names, &self.plans[ks[0]].graph, &self.src, &files);
                st.counts.add(&o.counts);
                for (k, r) in ks.iter().zip(res_of(&o)) {
                    let old = std::mem::replace(&mut self.results.get_mut(&w).unwrap()[*k], r);
                    diff_res(&self.plans[*k].name, &old, &self.results[&w][*k], &mut st.delta);
                    st.ran.push(self.plans[*k].name.clone());
                }
            }
            st.eval_ms = t.elapsed().as_secs_f64() * 1e3;
        }
        for (p, o, _) in &changed {
            if let Some(c) = o {
                self.exports.borrow_mut().release(*p, *c);
            }
        }
        st
    }

    /// Drop worktree `w`: its results, and its hold on every content.
    pub fn forget(&mut self, w: u32) {
        if let Some(m) = self.members.remove(&w) {
            for (p, c) in m {
                self.exports.borrow_mut().release(p, c);
            }
        }
        self.results.remove(&w);
    }

    /// The stored results of `w` against every plan run from scratch: plans that differ.
    pub fn check(&self, w: u32) -> Vec<String> {
        let (fresh, _) = self.eval_all(&self.files(w));
        self.plans.iter().enumerate().filter(|(k, _)| self.results[&w][*k] != fresh[*k]).map(|(_, p)| p.name.clone()).collect()
    }

    /// Findings held (rows over every plan and worktree).
    pub fn held(&self) -> usize {
        self.results.values().flatten().map(Res::rows).sum()
    }

    /// Memory by component: export blocks, their index, the interner, members, results.
    pub fn bytes(&self) -> serde_json::Value {
        let (blocks, block_bytes, _, at) = self.exports.borrow().footprint();
        let (nsyms, sym_bytes) = val::syms_footprint();
        let member_entries: usize = self.members.values().map(|m| m.len()).sum();
        let res_bytes: usize = self.results.values().flatten().map(Res::bytes).sum();
        serde_json::json!({
            "export_blocks": blocks, "export_block_bytes": block_bytes, "export_rows": self.exports.borrow().rows_resident,
            "export_index_bytes_est": at * 32, "contents_held": self.exports.borrow().held.len(),
            "interned": nsyms, "interned_bytes": sym_bytes,
            "member_entries": member_entries, "member_bytes_est": member_entries * 40,
            "result_rows": self.held(), "result_bytes_est": res_bytes,
            "refusals_raised": lower::refusals().0,
        })
    }

    /// The plans' result rows of `w` as text (checks across processes).
    pub fn result_rows(&self, w: u32) -> Vec<BTreeMap<String, R>> {
        self.results[&w].iter().map(res_rows).collect()
    }
}
