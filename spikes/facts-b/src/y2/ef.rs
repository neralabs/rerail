//! `exported-families` as local queries, keyed by family group and by family.
//!
//! The rule's windows are partitioned by keys, not global: a surface belongs to one group
//! `G = (scope, owner, base_name)` (`ef__slots`, `ef__family_groups`), an eligible group to one
//! family partition `F = (scope, base_name, implementations)` (`ef__families`). A surface is a
//! local query at its source: an exported root's definition (function arm), or an exported class
//! and its public methods (method arm, `proj_public_methods` at the class, i.e. the class's
//! out-edges of that kind). So an edit's changed roots, traits and public-method rows name the
//! sources to recompute; their old and new surfaces name the groups; the groups' old and new
//! eligibility names the family partitions; and only those partitions' violations are
//! recomputed. Each step reads only rows at the keys it is given, so the scoping is exact by the
//! same read-set argument as dead code's (`dc.rs`).
use super::facts::{at, string, sym, with, Change, Facts, Kind, Tab, Val};
use regex::Regex;
use rustc_hash::{FxHashMap, FxHashSet};

#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
enum Src {
    Function(u32),
    Class(u32),
}

// Slots in SQL TEXT order: '__invalid_mixed_name__' < 'async' < 'lazy' < 'sync'.
const SLOT_INVALID: u8 = 0;
const SLOT_ASYNC: u8 = 1;
const SLOT_LAZY: u8 = 2;
const SLOT_SYNC: u8 = 3;
const SLOT_NAMES: [&str; 4] = ["__invalid_mixed_name__", "async", "lazy", "sync"];

#[derive(Clone, PartialEq, Eq, Debug)]
struct Surface {
    symbol: u32,
    method: bool,
    owner: u32,
    ordinal: u32,
    mixed: bool,
    base_name: u32,
    slot: u8,
    execution_surface: Option<i64>,
}

type G = (bool, u32, u32);
type F = (bool, u32, u32);

#[derive(Clone, PartialEq, Eq, Debug)]
struct Group {
    exemplar: u32,
    ids: [Option<u32>; 4],
    mixed: bool,
    execution_surface: Option<i64>,
    implementations: u32,
    /// The selected slot rows (slot, symbol), for `direct_wrapper_bypasses_lazy_owner`.
    slots: Vec<(u8, u32)>,
}

pub type Finding = (u32, i64, i64, &'static str, u32);

struct Rx {
    lazy: Regex,
    async_sync: Regex,
    suffix: Regex,
    lazy_end: Regex,
    async_end: Regex,
    class_skip: Regex,
    connection: Regex,
    handle: Regex,
}

#[derive(Default, Clone, Debug, serde::Serialize)]
pub struct Stats {
    pub sources: usize,
    pub surfaces_recomputed: usize,
    pub groups: usize,
    pub families: usize,
    pub findings_changed: usize,
    pub prepare_ms: f64,
    pub finish_ms: f64,
}

pub struct Prep {
    sources: FxHashSet<Src>,
    groups: FxHashSet<G>,
    stats: Stats,
}

pub struct Ef {
    rx: Rx,
    surf_by_src: FxHashMap<Src, Vec<Surface>>,
    surf_by_g: FxHashMap<G, Vec<Surface>>,
    groups: FxHashMap<G, Group>,
    groups_of_symbol: FxHashMap<u32, Vec<G>>,
    eligible_by_f: FxHashMap<F, Vec<G>>,
    find_by_f: FxHashMap<F, Vec<Finding>>,
    find_by_src: FxHashMap<u32, Vec<Finding>>,
}

fn sq(s: u32) -> String {
    string(s)
}

impl Ef {
    pub fn scratch(f: &Facts) -> Ef {
        let rx = Rx {
            lazy: Regex::new("lazy").unwrap(),
            async_sync: Regex::new("async|sync").unwrap(),
            suffix: Regex::new("_(?:lazy|async|sync)$").unwrap(),
            lazy_end: Regex::new("_lazy$").unwrap(),
            async_end: Regex::new("_async$").unwrap(),
            class_skip: Regex::new("Protocol$|^Async").unwrap(),
            connection: Regex::new("Connection$").unwrap(),
            handle: Regex::new("^(?:Sync|Async|Lazy).*(?:Transaction|SchemaTransaction|SessionHandle|VersionHandle)$").unwrap(),
        };
        let mut ef = Ef { rx, surf_by_src: FxHashMap::default(), surf_by_g: FxHashMap::default(), groups: FxHashMap::default(), groups_of_symbol: FxHashMap::default(), eligible_by_f: FxHashMap::default(), find_by_f: FxHashMap::default(), find_by_src: FxHashMap::default() };
        let mut sources = FxHashSet::default();
        for &s in f.roots_by_symbol.keys() {
            sources.insert(Src::Function(s));
            sources.insert(Src::Class(s));
        }
        let p = Prep { sources, groups: FxHashSet::default(), stats: Stats::default() };
        ef.finish(f, p);
        ef
    }

    pub fn state_words(&self) -> usize {
        let s: usize = self.surf_by_src.values().map(|v| v.len() * 9).sum();
        let g: usize = self.groups.values().map(|g| 10 + g.slots.len() * 2).sum();
        let f: usize = self.find_by_f.values().chain(self.find_by_src.values()).map(|v| v.len() * 5).sum();
        s * 2 + g + self.groups_of_symbol.values().map(|v| v.len() * 3).sum::<usize>() + f
    }

    pub fn counts(&self) -> (usize, usize, usize) {
        (self.surf_by_g.values().map(Vec::len).sum(), self.groups.len(), self.eligible_by_f.values().map(Vec::len).sum())
    }

    fn mixed(&self, name: u32) -> bool {
        with(name, |n| {
            let l = n.to_ascii_lowercase();
            self.rx.lazy.is_match(&l) && self.rx.async_sync.is_match(&l)
        })
    }

    fn surface(&self, symbol: u32, name: u32, legacy_async: bool, execution_surface: Option<i64>, method: bool, owner: u32, ordinal: u32) -> Surface {
        let mixed = self.mixed(name);
        let base_name = if mixed { name } else { sym(&self.rx.suffix.replace(&sq(name), "")) };
        let slot = with(name, |n| {
            if mixed && method {
                SLOT_INVALID
            } else if self.rx.lazy_end.is_match(n) {
                SLOT_LAZY
            } else if self.rx.async_end.is_match(n) || legacy_async {
                SLOT_ASYNC
            } else {
                SLOT_SYNC
            }
        });
        Surface { symbol, method, owner, ordinal, mixed, base_name, slot, execution_surface }
    }

    /// `ef__surfaces` rows whose source is `s`.
    fn surfaces(&self, f: &Facts, s: Src) -> Vec<Surface> {
        let mut out = Vec::new();
        match s {
            Src::Function(r) => {
                let Some(d) = f.ef.get(&r) else { return out };
                if d.kind != Kind::Function || with(d.name, |n| n.starts_with('_')) || with(d.path, |p| p.starts_with("_internal/")) {
                    return out;
                }
                for root in at(&f.roots_by_symbol, &r) {
                    let ordinal = sym(&format!("{}\0{}", sq(root.module), sq(root.name)));
                    out.push(self.surface(d.id, d.name, d.legacy_async, d.execution_surface, false, root.module, ordinal));
                }
            }
            Src::Class(c) => {
                if !f.roots_by_symbol.contains_key(&c) {
                    return out;
                }
                let Some(ce) = f.ef.get(&c) else { return out };
                if ce.kind != Kind::Class || ce.private_api || with(ce.name, |n| self.rx.class_skip.is_match(n)) {
                    return out;
                }
                if f.defs.get(&c).is_some_and(|x| x.structural_protocol) {
                    return out;
                }
                for &p in at(&f.public_by_owner, &c) {
                    let Some(d) = f.ef.get(&p) else { continue };
                    if d.private_api {
                        continue;
                    }
                    out.push(self.surface(d.id, d.name, d.legacy_async, d.execution_surface, true, ce.name, d.name));
                }
            }
        }
        out
    }

    pub fn prepare(&mut self, f: &Facts, changes: &[Change]) -> Prep {
        let t = std::time::Instant::now();
        let mut sources = FxHashSet::default();
        let mut groups = FxHashSet::default();
        for ch in changes {
            let r = &ch.row;
            match ch.tab {
                Tab::Roots => {
                    sources.insert(Src::Function(r[3].s()));
                    sources.insert(Src::Class(r[3].s()));
                }
                Tab::Public => {
                    sources.insert(Src::Class(r[0].s()));
                }
                Tab::DeadTraits => {
                    sources.insert(Src::Class(r[10].s()));
                }
                Tab::EfTraits => {
                    let x = r[10].s();
                    sources.insert(Src::Function(x));
                    sources.insert(Src::Class(x));
                    for &o in at(&f.public_by_symbol, &x) {
                        sources.insert(Src::Class(o));
                    }
                    groups.extend(at(&self.groups_of_symbol, &x).iter().copied());
                }
                _ => {}
            }
        }
        let stats = Stats { sources: sources.len(), prepare_ms: t.elapsed().as_secs_f64() * 1e3, ..Default::default() };
        Prep { sources, groups, stats }
    }

    pub fn finish(&mut self, f: &Facts, p: Prep) -> Stats {
        let t = std::time::Instant::now();
        let Prep { sources, mut groups, mut stats } = p;
        for s in sources {
            let new = self.surfaces(f, s);
            stats.surfaces_recomputed += new.len();
            let old = self.surf_by_src.remove(&s).unwrap_or_default();
            for x in &old {
                let g = (x.method, x.owner, x.base_name);
                groups.insert(g);
                let v = self.surf_by_g.get_mut(&g).unwrap();
                let i = v.iter().position(|y| y == x).unwrap();
                v.swap_remove(i);
                if v.is_empty() {
                    self.surf_by_g.remove(&g);
                }
            }
            for x in &new {
                let g = (x.method, x.owner, x.base_name);
                groups.insert(g);
                self.surf_by_g.entry(g).or_default().push(x.clone());
            }
            if let Src::Function(r) = s {
                let fm: Vec<Finding> = new.iter().filter(|x| x.mixed).filter_map(|x| {
                    let fam = sym(&serde_json::to_string(&serde_json::json!(["function", sq(x.owner), sq(x.base_name), sq(x.ordinal)])).unwrap());
                    self.locate(f, x.symbol, "invalid_mixed_lazy_execution_name", fam)
                }).collect();
                let old = self.find_by_src.remove(&r).unwrap_or_default();
                if old != fm {
                    stats.findings_changed += 1;
                }
                if !fm.is_empty() {
                    self.find_by_src.insert(r, fm);
                }
            }
            if !new.is_empty() {
                self.surf_by_src.insert(s, new);
            }
        }
        stats.groups = groups.len();
        let mut fams: FxHashSet<F> = FxHashSet::default();
        for g in groups {
            if let Some(old) = self.groups.remove(&g) {
                for (_, s) in &old.slots {
                    let v = self.groups_of_symbol.get_mut(s).unwrap();
                    let i = v.iter().position(|y| *y == g).unwrap();
                    v.swap_remove(i);
                    if v.is_empty() {
                        self.groups_of_symbol.remove(s);
                    }
                }
                let fk = (g.0, g.2, old.implementations);
                if let Some(v) = self.eligible_by_f.get_mut(&fk) {
                    if let Some(i) = v.iter().position(|y| *y == g) {
                        v.swap_remove(i);
                        if v.is_empty() {
                            self.eligible_by_f.remove(&fk);
                        }
                    }
                }
                fams.insert(fk);
            }
            if let Some(new) = self.group(f, g) {
                for (_, s) in &new.slots {
                    self.groups_of_symbol.entry(*s).or_default().push(g);
                }
                let fk = (g.0, g.2, new.implementations);
                if self.eligible(f, g, &new) {
                    self.eligible_by_f.entry(fk).or_default().push(g);
                }
                fams.insert(fk);
                self.groups.insert(g, new);
            }
        }
        stats.families = fams.len();
        for fk in fams {
            let new = self.family_findings(f, fk);
            let old = self.find_by_f.remove(&fk).unwrap_or_default();
            if old != new {
                stats.findings_changed += 1;
            }
            if !new.is_empty() {
                self.find_by_f.insert(fk, new);
            }
        }
        stats.finish_ms = t.elapsed().as_secs_f64() * 1e3;
        stats
    }

    /// `ef__slots` and `ef__family_groups` at one group key.
    fn group(&self, f: &Facts, g: G) -> Option<Group> {
        let rows = at(&self.surf_by_g, &g);
        let _ = f;
        // Per slot: the selected row (max ordinal, then min symbol) and the slot's min ordinal.
        let mut per: [Option<(String, String, Surface)>; 4] = Default::default();
        let mut first: [Option<String>; 4] = Default::default();
        for x in rows {
            if !x.method && x.mixed {
                continue;
            }
            let k = x.slot as usize;
            let (o, s) = (sq(x.ordinal), sq(x.symbol));
            if first[k].as_ref().is_none_or(|m| o < *m) {
                first[k] = Some(o.clone());
            }
            let better = match &per[k] {
                None => true,
                Some((bo, bs, _)) => o > *bo || (o == *bo && s < *bs),
            };
            if better {
                per[k] = Some((o, s, x.clone()));
            }
        }
        if per.iter().all(Option::is_none) {
            return None;
        }
        let mut exemplar: Option<(String, u8, u32)> = None;
        let mut ids = [None; 4];
        let (mut mixed, mut es) = (false, None::<i64>);
        let mut slots = Vec::new();
        let mut impl_rows = Vec::new();
        for k in 0..4 {
            let Some((_, _, x)) = &per[k] else { continue };
            let fo = first[k].clone().unwrap();
            if exemplar.as_ref().is_none_or(|(eo, es_, _)| fo < *eo || (fo == *eo && (k as u8) < *es_)) {
                exemplar = Some((fo, k as u8, x.symbol));
            }
            ids[k] = Some(x.symbol);
            mixed |= x.mixed;
            if let Some(v) = x.execution_surface {
                es = Some(es.map_or(v, |e: i64| e.max(v)));
            }
            slots.push((k as u8, x.symbol));
            impl_rows.push(serde_json::json!([SLOT_NAMES[k], sq(x.symbol)]));
        }
        let implementations = sym(&serde_json::to_string(&impl_rows).unwrap());
        Some(Group { exemplar: exemplar.unwrap().2, ids, mixed, execution_surface: es, implementations, slots })
    }

    /// `ef__eligible_groups` at one group (`executionOnly` true).
    fn eligible(&self, f: &Facts, g: G, gr: &Group) -> bool {
        let Some(d) = f.ef.get(&gr.exemplar) else { return false };
        if gr.mixed {
            return true;
        }
        let (bn, owner, path) = (sq(g.2), sq(g.1), sq(d.path));
        let bni = |xs: &[&str]| xs.contains(&bn.as_str());
        let ow = |xs: &[&str]| xs.contains(&owner.as_str());
        let excluded = (bni(&["plan", "query", "query_unfiltered"]) && ["l2050_transaction/transaction.py", "l7200_versioned_transactions/versioned_transactions.py"].contains(&path.as_str()) && ow(&["ReadTransaction", "WriteTransaction", "LazyVersionedReadTransaction", "LazyVersionedWriteTransaction"]))
            || (owner == "CacheTransaction" && bni(&["commit", "rollback"]))
            || (owner == "BaseMigration" && bn == "rollback")
            || (bn == "execute_ddl" && (ow(&["SchemaTransaction", "AsyncSchemaTransaction"]) || path == "l2000_connection/schema_transaction.py"))
            || (self.rx.connection.is_match(&owner) && bn == "table_exists")
            || (gr.ids[SLOT_SYNC as usize].is_none() && gr.ids[SLOT_ASYNC as usize].is_none() && gr.ids[SLOT_LAZY as usize].is_some() && (bn.to_ascii_lowercase().contains("bootstrap") || owner.to_ascii_lowercase().contains("bootstrap") || bni(&["check_ddl_lockdown", "column_exists", "physical_table_exists_many", "seal_table_exists_cache"])))
            || ow(&["LazyVersionedReadTransaction", "LazyVersionedWriteTransaction", "SyncSessionHandle", "AsyncSessionHandle", "SyncVersionHandle", "AsyncVersionHandle"])
            || self.rx.handle.is_match(&owner);
        !excluded && gr.execution_surface.unwrap_or(0) != 0
    }

    fn locate(&self, f: &Facts, symbol: u32, issue: &'static str, family: u32) -> Option<Finding> {
        f.ef.get(&symbol).map(|d| (d.path, d.start, d.end, issue, family))
    }

    /// `ef__families` and `ef__triple_violations` (and the findings join) at one partition.
    fn family_findings(&self, f: &Facts, fk: F) -> Vec<Finding> {
        let mut gs: Vec<(String, G)> = at(&self.eligible_by_f, &fk).iter().map(|g| (sq(g.1), *g)).collect();
        gs.sort();
        let mut out = Vec::new();
        for (i, (owner, g)) in gs.iter().enumerate() {
            let gr = &self.groups[g];
            if !(gr.mixed || i == 0) {
                continue;
            }
            let scope = if g.0 { "method" } else { "function" };
            let family = sym(&serde_json::to_string(&serde_json::json!([scope, owner, sq(g.2), sq(gr.implementations)])).unwrap());
            let [_, a, l, s] = gr.ids;
            let mut v = |sym_: u32, issue: &'static str| {
                if let Some(x) = self.locate(f, sym_, issue, family) {
                    out.push(x);
                }
            };
            if gr.mixed {
                v(gr.exemplar, "invalid_mixed_lazy_execution_name");
                continue;
            }
            if s.is_none() && a.is_some() {
                v(gr.exemplar, "missing_sync");
            }
            if a.is_none() && s.is_some() {
                v(gr.exemplar, "missing_async");
            }
            if l.is_none() {
                v(gr.exemplar, "missing_lazy");
            }
            if let (Some(s), Some(l)) = (s, l) {
                if let (Some(sd), Some(ld)) = (f.ef.get(&s), f.ef.get(&l)) {
                    if sd.params == ld.params && !sd.legacy_sync {
                        v(s, "missing_legacy_sync");
                    }
                }
            }
            if let (Some(a), Some(l)) = (a, l) {
                if let (Some(ad), Some(ld)) = (f.ef.get(&a), f.ef.get(&l)) {
                    if ad.params == ld.params && !ad.legacy_async {
                        v(a, "missing_legacy_async");
                    }
                }
            }
            if let Some(l) = l {
                if f.ef.get(&l).is_some_and(|d| !d.lazy_decorated) {
                    v(l, "missing_lazy_decorator");
                }
                let mut best: Option<(String, i64, String, u32)> = None;
                for &(k, sy) in &gr.slots {
                    if k != SLOT_SYNC && k != SLOT_ASYNC {
                        continue;
                    }
                    let Some(d) = f.ef.get(&sy) else { continue };
                    if !d.direct_lazy_wrapper {
                        continue;
                    }
                    let key = (sq(d.path), d.start, sq(sy), sy);
                    if best.as_ref().is_none_or(|b| (&key.0, key.1, &key.2) < (&b.0, b.1, &b.2)) {
                        best = Some(key);
                    }
                }
                if let Some(b) = best {
                    v(b.3, "direct_wrapper_bypasses_lazy_owner");
                }
            }
        }
        out.sort();
        out
    }

    /// `ef__families.family` of every selected family (checked against the dump's stage).
    pub fn families(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (fk, gs) in &self.eligible_by_f {
            let mut gs: Vec<(String, G)> = gs.iter().map(|g| (sq(g.1), *g)).collect();
            gs.sort();
            for (i, (owner, g)) in gs.iter().enumerate() {
                let gr = &self.groups[g];
                if gr.mixed || i == 0 {
                    let scope = if fk.0 { "method" } else { "function" };
                    out.push(serde_json::to_string(&serde_json::json!([scope, owner, sq(g.2), sq(gr.implementations)])).unwrap());
                }
            }
        }
        out.sort();
        out
    }

    /// `ef__findings` rows as the dump prints them.
    pub fn finding_rows(&self) -> Vec<Vec<Val>> {
        let mut v: Vec<Vec<Val>> = self.find_by_f.values().chain(self.find_by_src.values()).flatten().map(|(p, s, e, i, fam)| vec![Val::S(*p), Val::Int(*s), Val::Int(*e), Val::S(sym(i)), Val::S(*fam)]).collect();
        v.sort();
        v
    }
}
