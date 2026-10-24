//! The graph API Y2's local rules read (`docs/facts/reports/explore-y1-graph.md` § API):
//! `Graph` is the base, columnar (CSR by (module, def name) and by destination def); `GraphView`
//! is the base plus one worktree's delta, with the same methods through `GraphRead`.
//!
//! Ids are u32 throughout: `Sym` (one interner), `ModuleId` (the module's path Sym), `DefId`
//! (one per (path, def idx), stable across content edits), `ContentId` (the content hash Sym),
//! `NodeId` (the def's index in its content's summary).
use super::engine::T_BYNAME;
use super::intern::{Interner, Sym};
use super::path::CLASS;
use super::world::{Base, Delta, Shared};
use rustc_hash::{FxHashMap, FxHashSet};

pub type ModuleId = u32;
pub type DefId = u32;
pub type ContentId = u32;
pub type NodeId = u32;

/// What a reference edge is. The linker's graph has one kind; whether the target is a class is
/// the target's `Def::kind` (it can change without the edge changing).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum EdgeKind {
    Ref = 0,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Edge {
    pub src_def: DefId,
    pub dst_def: DefId,
    pub kind: EdgeKind,
    /// (content of the source's path, the source def's node)
    pub site: (ContentId, NodeId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum DefKind {
    Function = 0,
    Class = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Def {
    pub id: DefId,
    pub module: ModuleId,
    pub name: Sym,
    pub kind: DefKind,
    pub content: ContentId,
    pub node: NodeId,
    pub start: u32,
    pub end: u32,
    /// the summary's flags (bit 1 class, bit 6 shadowed)
    pub flags: u32,
}

/// The read side shared by `Graph` and `GraphView`.
pub trait GraphRead {
    /// Edges out of the definitions named `name` in `module`, by source def then target.
    fn edges_from(&self, module: ModuleId, name: Sym) -> &[Edge];
    /// Edges into `def`, by source def.
    fn edges_to(&self, def: DefId) -> &[Edge];
    /// The module's definitions, by node.
    fn definitions(&self, module: ModuleId) -> &[Def];
    /// The linked module of a member path (None: not a member, out of scope, or refused).
    fn module_of(&self, path: &str) -> Option<ModuleId>;
    /// The definition of an id in this graph (None: its path holds no such node here).
    fn def(&self, id: DefId) -> Option<&Def>;
    /// `by_name`: the definitions a qualified name (either spelling) resolves to.
    fn defs_by_name(&self, name: Sym) -> &[DefId];
    fn epoch(&self) -> u64;
    /// native's "reference graph incomplete" (an open summary in scope): edges are not a graph
    fn incomplete(&self) -> bool;
    /// a linker refusal holds: the graph must not be read
    fn refused(&self) -> bool;
    fn sym(&self, s: &str) -> Option<Sym>;
    fn str(&self, s: Sym) -> &str;
}

/// A module's edges, sorted by (source name, source, target), with a name index.
#[derive(Default, Clone)]
struct ModuleEdges {
    edges: Vec<Edge>,
    by_name: Vec<(Sym, u32, u32)>,
}

impl ModuleEdges {
    fn slice(&self, name: Sym) -> &[Edge] {
        match self.by_name.binary_search_by_key(&name, |x| x.0) {
            Ok(i) => &self.edges[self.by_name[i].1 as usize..self.by_name[i].2 as usize],
            Err(_) => &[],
        }
    }
}

fn module_defs(sh: &Shared, p: Sym, r: &super::world::PathRec) -> Vec<Def> {
    let _ = &sh.it;
    r.defs
        .iter()
        .map(|d| Def {
            id: *sh.defs.map.get(&(p, d.idx)).unwrap_or(&u32::MAX),
            module: p,
            name: d.name,
            kind: if d.flags & CLASS != 0 { DefKind::Class } else { DefKind::Function },
            content: r.content,
            node: d.idx,
            start: d.start,
            end: d.stop,
            flags: d.flags,
        })
        .collect()
}

fn module_edges(content: ContentId, defs: &[Def], out: &[(u32, u32)], src_id: &dyn Fn(u32) -> DefId) -> ModuleEdges {
    let name_of: FxHashMap<u32, Sym> = defs.iter().map(|d| (d.node, d.name)).collect();
    let mut es: Vec<(Sym, Edge)> = out
        .iter()
        .map(|&(idx, dst)| (name_of.get(&idx).copied().unwrap_or(0), Edge { src_def: src_id(idx), dst_def: dst, kind: EdgeKind::Ref, site: (content, idx) }))
        .collect();
    es.sort_unstable_by_key(|(n, e)| (*n, e.src_def, e.dst_def));
    let mut by_name: Vec<(Sym, u32, u32)> = Vec::new();
    for (i, (n, _)) in es.iter().enumerate() {
        match by_name.last_mut() {
            Some(l) if l.0 == *n => l.2 = i as u32 + 1,
            _ => by_name.push((*n, i as u32, i as u32 + 1)),
        }
    }
    ModuleEdges { edges: es.into_iter().map(|x| x.1).collect(), by_name }
}

/// Give every definition of the base (`d` None) or of a delta's paths its id.
pub fn assign_def_ids(sh: &mut Shared, base: &Base, d: Option<&Delta>) {
    match d {
        None => {
            for (p, r) in &base.paths {
                for dd in r.defs.iter() {
                    sh.defs.id(*p, dd.idx);
                }
            }
        }
        Some(d) => {
            for (p, r) in d.paths.iter() {
                if let Some(r) = r {
                    for dd in r.defs.iter() {
                        sh.defs.id(*p, dd.idx);
                    }
                }
            }
        }
    }
}

pub struct Graph<'a> {
    it: &'a Interner,
    modules: FxHashMap<ModuleId, (Vec<Def>, ModuleEdges)>,
    def_at: FxHashMap<DefId, (ModuleId, u32)>,
    to: Vec<Edge>,
    to_index: FxHashMap<DefId, (u32, u32)>,
    by_name: FxHashMap<Sym, Vec<DefId>>,
    incomplete: bool,
    refused: bool,
}

fn src_ids<'s>(sh: &'s Shared, p: Sym) -> impl Fn(u32) -> DefId + 's {
    move |idx| *sh.defs.map.get(&(p, idx)).unwrap_or(&u32::MAX)
}

fn by_name_defs(sh: &Shared, rows: &[([u32; 4], u32)]) -> Vec<DefId> {
    let mut v: Vec<DefId> = rows.iter().filter_map(|r| sh.defs.map.get(&(r.0[0], r.0[1])).copied()).collect();
    v.sort_unstable();
    v.dedup();
    v
}

impl<'a> Graph<'a> {
    /// Materialise the base's graph (the shared part every view reads).
    /// Needs every def's id (`assign_def_ids` over the base first).
    pub fn build(sh: &'a Shared, base: &Base) -> Graph<'a> {
        let mut modules = FxHashMap::default();
        let mut def_at = FxHashMap::default();
        let mut to: Vec<Edge> = Vec::new();
        for (p, r) in &base.paths {
            if !r.linked {
                continue;
            }
            let defs = module_defs(sh, *p, r);
            for (i, d) in defs.iter().enumerate() {
                def_at.insert(d.id, (*p, i as u32));
            }
            let me = module_edges(r.content, &defs, base.out.get(p).map_or(&[], |o| o), &src_ids(sh, *p));
            to.extend_from_slice(&me.edges);
            modules.insert(*p, (defs, me));
        }
        to.sort_unstable_by_key(|e| (e.dst_def, e.src_def));
        let mut to_index = FxHashMap::default();
        let mut i = 0;
        while i < to.len() {
            let mut j = i;
            while j < to.len() && to[j].dst_def == to[i].dst_def {
                j += 1;
            }
            to_index.insert(to[i].dst_def, (i as u32, j as u32));
            i = j;
        }
        let by_name = base.tables.t[T_BYNAME as usize].iter().map(|(k, rows)| (*k, by_name_defs(sh, rows))).collect();
        Graph { it: &sh.it, modules, def_at, to, to_index, by_name, incomplete: base.counters.open_paths > 0, refused: base.counters.refused_paths + base.counters.refusal_roots > 0 }
    }

    pub fn edge_count(&self) -> usize {
        self.to.len()
    }

    /// Heap bytes of the materialised base graph.
    pub fn bytes(&self) -> usize {
        let e = std::mem::size_of::<Edge>();
        let d = std::mem::size_of::<Def>();
        self.modules.capacity() * 8
            + self.modules.values().map(|(ds, me)| 48 + ds.capacity() * d + me.edges.capacity() * e + me.by_name.capacity() * 12).sum::<usize>()
            + self.def_at.capacity() * 13
            + self.to.capacity() * e
            + self.to_index.capacity() * 13
            + self.by_name.capacity() * 28
            + self.by_name.values().map(|v| v.capacity() * 4).sum::<usize>()
    }
}

impl GraphRead for Graph<'_> {
    fn edges_from(&self, module: ModuleId, name: Sym) -> &[Edge] {
        self.modules.get(&module).map_or(&[], |m| m.1.slice(name))
    }
    fn edges_to(&self, def: DefId) -> &[Edge] {
        self.to_index.get(&def).map_or(&[], |&(a, b)| &self.to[a as usize..b as usize])
    }
    fn definitions(&self, module: ModuleId) -> &[Def] {
        self.modules.get(&module).map_or(&[], |m| &m.0)
    }
    fn module_of(&self, path: &str) -> Option<ModuleId> {
        let p = self.it.get(path)?;
        self.modules.contains_key(&p).then_some(p)
    }
    fn def(&self, id: DefId) -> Option<&Def> {
        self.def_at.get(&id).map(|&(m, i)| &self.modules[&m].0[i as usize])
    }
    fn defs_by_name(&self, name: Sym) -> &[DefId] {
        self.by_name.get(&name).map_or(&[], |v| v)
    }
    fn epoch(&self) -> u64 {
        0
    }
    fn incomplete(&self) -> bool {
        self.incomplete
    }
    fn refused(&self) -> bool {
        self.refused
    }
    fn sym(&self, s: &str) -> Option<Sym> {
        self.it.get(s)
    }
    fn str(&self, s: Sym) -> &str {
        self.it.str(s)
    }
}

/// A worktree's view: the base graph with the modules, incoming lists and by_name keys that
/// differ replaced (`GraphView { base: &Graph, delta: &Delta }` of the contract, materialised
/// from the maintained delta in time proportional to it).
pub struct GraphView<'a> {
    pub base: &'a Graph<'a>,
    modules: FxHashMap<ModuleId, Option<(Vec<Def>, ModuleEdges)>>,
    def_at: FxHashMap<DefId, Option<(ModuleId, u32)>>,
    to: FxHashMap<DefId, Vec<Edge>>,
    by_name: FxHashMap<Sym, Vec<DefId>>,
    epoch: u64,
    incomplete: bool,
    refused: bool,
}

impl<'a> GraphView<'a> {
    /// Needs every def's id (`assign_def_ids` over the delta first).
    pub fn build(g: &'a Graph<'a>, sh: &Shared, base: &Base, d: &Delta) -> GraphView<'a> {
        let mut over: FxHashSet<ModuleId> = d.paths.keys().copied().collect();
        over.extend(d.out.keys().copied());
        let mut modules = FxHashMap::default();
        let mut def_at = FxHashMap::default();
        let mut new_in: FxHashMap<DefId, Vec<Edge>> = FxHashMap::default();
        let mut affected: FxHashSet<DefId> = FxHashSet::default();
        for &p in &over {
            for e in g.modules.get(&p).map_or(&[][..], |m| &m.1.edges) {
                affected.insert(e.dst_def);
            }
            for dd in g.definitions(p) {
                def_at.insert(dd.id, None);
            }
            match d.rec(base, p).filter(|r| r.linked) {
                Some(r) => {
                    let defs = module_defs(sh, p, r);
                    for (i, dd) in defs.iter().enumerate() {
                        def_at.insert(dd.id, Some((p, i as u32)));
                    }
                    let me = module_edges(r.content, &defs, d.out(base, p), &src_ids(sh, p));
                    for e in &me.edges {
                        affected.insert(e.dst_def);
                        new_in.entry(e.dst_def).or_default().push(*e);
                    }
                    modules.insert(p, Some((defs, me)));
                }
                None => {
                    modules.insert(p, None);
                }
            }
        }
        let src_path = |e: &Edge| sh.defs.defs[e.src_def as usize].0;
        let mut to = FxHashMap::default();
        for dst in affected {
            let mut l: Vec<Edge> = g.edges_to(dst).iter().filter(|e| !over.contains(&src_path(e))).copied().collect();
            l.extend(new_in.remove(&dst).unwrap_or_default());
            l.sort_unstable_by_key(|e| (e.src_def, e.dst_def));
            to.insert(dst, l);
        }
        let by_name = d.tables.t[T_BYNAME as usize].iter().map(|(k, rows)| (*k, by_name_defs(sh, rows))).collect();
        GraphView { base: g, modules, def_at, to, by_name, epoch: d.epoch, incomplete: d.incomplete(), refused: d.refused() }
    }

    /// Heap bytes of the view's delta.
    pub fn bytes(&self) -> usize {
        let e = std::mem::size_of::<Edge>();
        let dd = std::mem::size_of::<Def>();
        self.modules.capacity() * 8
            + self.modules.values().flatten().map(|(ds, me)| 48 + ds.capacity() * dd + me.edges.capacity() * e + me.by_name.capacity() * 12).sum::<usize>()
            + self.def_at.capacity() * 13
            + self.to.capacity() * 28
            + self.to.values().map(|v| v.capacity() * e).sum::<usize>()
            + self.by_name.capacity() * 28
            + self.by_name.values().map(|v| v.capacity() * 4).sum::<usize>()
    }
}

impl GraphRead for GraphView<'_> {
    fn edges_from(&self, module: ModuleId, name: Sym) -> &[Edge] {
        match self.modules.get(&module) {
            Some(Some(m)) => m.1.slice(name),
            Some(None) => &[],
            None => self.base.edges_from(module, name),
        }
    }
    fn edges_to(&self, def: DefId) -> &[Edge] {
        match self.to.get(&def) {
            Some(l) => l,
            None => self.base.edges_to(def),
        }
    }
    fn definitions(&self, module: ModuleId) -> &[Def] {
        match self.modules.get(&module) {
            Some(Some(m)) => &m.0,
            Some(None) => &[],
            None => self.base.definitions(module),
        }
    }
    fn module_of(&self, path: &str) -> Option<ModuleId> {
        let p = self.base.it.get(path)?;
        match self.modules.get(&p) {
            Some(m) => m.as_ref().map(|_| p),
            None => self.base.module_of(path),
        }
    }
    fn def(&self, id: DefId) -> Option<&Def> {
        match self.def_at.get(&id) {
            Some(Some((m, i))) => self.modules[m].as_ref().map(|x| &x.0[*i as usize]),
            Some(None) => None,
            None => self.base.def(id),
        }
    }
    fn defs_by_name(&self, name: Sym) -> &[DefId] {
        match self.by_name.get(&name) {
            Some(v) => v,
            None => self.base.defs_by_name(name),
        }
    }
    fn epoch(&self) -> u64 {
        self.epoch
    }
    fn incomplete(&self) -> bool {
        self.incomplete
    }
    fn refused(&self) -> bool {
        self.refused
    }
    fn sym(&self, s: &str) -> Option<Sym> {
        self.base.it.get(s)
    }
    fn str(&self, s: Sym) -> &str {
        self.base.it.str(s)
    }
}
