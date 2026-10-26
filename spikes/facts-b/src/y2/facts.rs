//! The inputs of Y2's two rules for one worktree, as keyed tables with the inverse indexes the
//! rules' local queries and their scoping read: the reference graph (`views/reference_edges`,
//! Y1's structure once it lands), the symbol table (`proj_*`, rule-side membership facts), and
//! per-definition facts computed per content (`dc__dead_traits`, `ef__traits`, the probe's base
//! names and dynamic imports, `dc__module_names`).
//!
//! Every table is a multiset of rows (the dump's counts); indexes hold the rows that are present
//! (count > 0). `classify` turns a delta into presence changes before it is applied, so a rule can
//! read the old state for its scoping; `apply` then updates the tables.
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;

// ---- one interner (Graph::sym / Graph::str in the shared contract)

#[derive(Default)]
pub struct Syms {
    map: FxHashMap<Box<str>, u32>,
    strs: Vec<Box<str>>,
    bytes: usize,
}

thread_local! {
    static SYMS: RefCell<Syms> = RefCell::new(Syms::default());
}

/// Intern `s` (the reserved id 0 is never a string).
pub fn sym(s: &str) -> u32 {
    SYMS.with(|t| {
        let mut t = t.borrow_mut();
        if t.strs.is_empty() {
            t.strs.push("".into());
        }
        if let Some(&i) = t.map.get(s) {
            return i;
        }
        let i = t.strs.len() as u32;
        t.strs.push(s.into());
        t.map.insert(s.into(), i);
        t.bytes += s.len();
        i
    })
}

/// `s` if interned (lookups never grow the table).
pub fn get(s: &str) -> Option<u32> {
    SYMS.with(|t| t.borrow().map.get(s).copied())
}

pub fn with<R>(i: u32, f: impl FnOnce(&str) -> R) -> R {
    SYMS.with(|t| f(&t.borrow().strs[i as usize]))
}

pub fn string(i: u32) -> String {
    with(i, str::to_owned)
}

/// (strings, bytes of text).
pub fn footprint() -> (usize, usize) {
    SYMS.with(|t| {
        let t = t.borrow();
        (t.strs.len(), t.bytes)
    })
}

// ---- values and rows

#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum Val {
    Null,
    Int(i64),
    S(u32),
}

impl Val {
    pub fn s(self) -> u32 {
        match self {
            Val::S(s) => s,
            v => panic!("expected text, got {v:?}"),
        }
    }
    pub fn os(self) -> Option<u32> {
        match self {
            Val::S(s) => Some(s),
            Val::Null => None,
            v => panic!("expected text or null, got {v:?}"),
        }
    }
    pub fn i(self) -> i64 {
        match self {
            Val::Int(i) => i,
            v => panic!("expected integer, got {v:?}"),
        }
    }
    pub fn oi(self) -> Option<i64> {
        match self {
            Val::Int(i) => Some(i),
            Val::Null => None,
            v => panic!("expected integer or null, got {v:?}"),
        }
    }
    pub fn b(self) -> bool {
        self.i() != 0
    }
}

pub type Row = Box<[Val]>;

pub fn parse_row(s: &str) -> Row {
    let v: serde_json::Value = serde_json::from_str(s).unwrap_or_else(|e| panic!("row {s}: {e}"));
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| match x {
            serde_json::Value::Null => Val::Null,
            serde_json::Value::Number(n) => Val::Int(n.as_i64().unwrap_or_else(|| panic!("non-integer number in {s}"))),
            serde_json::Value::String(t) => Val::S(sym(t)),
            o => panic!("unexpected value {o} in {s}"),
        })
        .collect()
}

pub fn show_row(r: &[Val]) -> String {
    let parts: Vec<String> = r
        .iter()
        .map(|v| match v {
            Val::Null => "null".into(),
            Val::Int(i) => i.to_string(),
            Val::S(s) => format!("{:?}", string(*s)),
        })
        .collect();
    format!("[{}]", parts.join(","))
}

// ---- tables

/// The tables Y2 reads, by dump output name.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Tab {
    Refs,
    Ann,
    Imports,
    Resolved,
    Bases,
    Hier,
    Public,
    Roots,
    DeadTraits,
    ModuleNames,
    EfTraits,
    Probe,
    Dyn,
}

impl Tab {
    pub fn of_output(name: &str) -> Option<Tab> {
        let n = name.rsplit_once("/stage/").map(|x| x.1).unwrap_or(name);
        Some(match n {
            "views/reference_edges" => Tab::Refs,
            "proj_annotation_names" => Tab::Ann,
            "proj_imports" => Tab::Imports,
            "proj_resolved_names" => Tab::Resolved,
            "proj_base_edges" => Tab::Bases,
            "proj_hierarchy" => Tab::Hier,
            "proj_public_methods" => Tab::Public,
            "proj_exported_roots" => Tab::Roots,
            "dc__dead_traits" => Tab::DeadTraits,
            "dc__module_names" => Tab::ModuleNames,
            "ef__traits" => Tab::EfTraits,
            "y2-probe:y2-probe" => Tab::Probe,
            "y2-dyn:y2-dyn" => Tab::Dyn,
            _ => return None,
        })
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Kind {
    Function,
    Method,
    Class,
}

/// A definition (`proj_declarations`) with its per-content traits (`dc__dead_traits`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Def {
    pub path: u32,
    pub start: i64,
    pub end: i64,
    pub name: u32,
    pub class_name: Option<u32>,
    pub class_start: Option<i64>,
    pub module: u32,
    pub kind: Kind,
    pub id: u32,
    pub fqname: u32,
    pub base_name: u32,
    pub class_id: Option<u32>,
    pub abstract_: bool,
    pub property: bool,
    pub implicit: bool,
    pub protocol: bool,
    pub structural_protocol: bool,
}

impl Def {
    pub fn of(r: &[Val]) -> Def {
        assert_eq!(r.len(), 20, "dead_traits row width");
        let kind = with(r[9].s(), |k| match k {
            "function" => Kind::Function,
            "method" => Kind::Method,
            "class" => Kind::Class,
            o => panic!("kind {o}"),
        });
        Def {
            path: r[0].s(),
            start: r[1].i(),
            end: r[2].i(),
            name: r[3].s(),
            class_name: r[4].os(),
            class_start: r[6].oi(),
            module: r[8].s(),
            kind,
            id: r[10].s(),
            fqname: r[11].s(),
            base_name: r[13].s(),
            class_id: r[14].os(),
            abstract_: r[15].b(),
            property: r[16].b(),
            implicit: r[17].b(),
            protocol: r[18].b(),
            structural_protocol: r[19].b(),
        }
    }
}

/// `ef__traits`' own columns.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EfDef {
    pub id: u32,
    pub path: u32,
    pub start: i64,
    pub end: i64,
    pub name: u32,
    pub kind: Kind,
    pub legacy_sync: bool,
    pub legacy_async: bool,
    pub lazy_decorated: bool,
    pub private_api: bool,
    pub execution_surface: Option<i64>,
    pub params: u32,
    pub direct_lazy_wrapper: bool,
}

impl EfDef {
    fn of(r: &[Val]) -> EfDef {
        assert_eq!(r.len(), 20, "ef traits row width");
        let kind = with(r[9].s(), |k| match k {
            "function" => Kind::Function,
            "method" => Kind::Method,
            "class" => Kind::Class,
            o => panic!("kind {o}"),
        });
        EfDef {
            id: r[10].s(),
            path: r[0].s(),
            start: r[1].i(),
            end: r[2].i(),
            name: r[3].s(),
            kind,
            legacy_sync: r[13].b(),
            legacy_async: r[14].b(),
            lazy_decorated: r[15].b(),
            private_api: r[16].b(),
            execution_surface: r[17].oi(),
            params: r[18].s(),
            direct_lazy_wrapper: r[19].b(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Import {
    pub module: u32,
    pub name: u32,
    pub target: u32,
    pub symbol_kind: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Root {
    pub path: u32,
    pub module: u32,
    pub name: u32,
    pub symbol: u32,
}

pub type Pos = (u32, i64);

/// A presence change: `row` of `tab` appears (`added`) or disappears.
#[derive(Clone, Debug)]
pub struct Change {
    pub tab: Tab,
    pub row: Row,
    pub added: bool,
}

fn push<K: std::hash::Hash + Eq, V>(m: &mut FxHashMap<K, Vec<V>>, k: K, v: V) {
    m.entry(k).or_default().push(v);
}

fn pull<K: std::hash::Hash + Eq, V: PartialEq>(m: &mut FxHashMap<K, Vec<V>>, k: K, v: &V) {
    let e = m.get_mut(&k).expect("index key");
    let i = e.iter().position(|x| x == v).expect("index entry");
    e.swap_remove(i);
    if e.is_empty() {
        m.remove(&k);
    }
}

pub fn at<'a, K: std::hash::Hash + Eq, V>(m: &'a FxHashMap<K, Vec<V>>, k: &K) -> &'a [V] {
    m.get(k).map_or(&[], Vec::as_slice)
}

#[derive(Default)]
pub struct Facts {
    counts: FxHashMap<(Tab, Row), i32>,
    // definitions
    pub defs: FxHashMap<u32, Def>,
    pub by_pos: FxHashMap<Pos, u32>,
    pub by_module: FxHashMap<u32, Vec<u32>>,
    pub by_module_name: FxHashMap<(u32, u32), Vec<u32>>,
    pub by_module_class: FxHashMap<(u32, u32), Vec<u32>>,
    pub methods_by_class: FxHashMap<u32, Vec<u32>>,
    pub methods_by_name: FxHashMap<u32, Vec<u32>>,
    pub functions_by_module_base: FxHashMap<(u32, u32), Vec<u32>>,
    pub methods_by_class_base: FxHashMap<(u32, u32), Vec<u32>>,
    pub by_fqname: FxHashMap<u32, Vec<u32>>,
    pub classes_by_name: FxHashMap<u32, Vec<u32>>,
    pub ef: FxHashMap<u32, EfDef>,
    // reference graph (Y1's Graph: edges_from / edges_to)
    pub refs_from: FxHashMap<Pos, Vec<Pos>>,
    pub refs_to: FxHashMap<Pos, Vec<Pos>>,
    // symbol table
    pub bases_by_owner: FxHashMap<u32, Vec<u32>>,
    pub bases_by_base: FxHashMap<u32, Vec<u32>>,
    pub hier_by_owner: FxHashMap<u32, Vec<(u32, i64)>>,
    pub hier_by_base: FxHashMap<u32, Vec<(u32, i64)>>,
    pub public_by_owner: FxHashMap<u32, Vec<u32>>,
    pub public_by_symbol: FxHashMap<u32, Vec<u32>>,
    pub roots_by_symbol: FxHashMap<u32, Vec<Root>>,
    pub imports_by_target: FxHashMap<u32, Vec<Import>>,
    pub imports_by_module_name: FxHashMap<(u32, u32), Vec<Import>>,
    pub resolved_by_fq: FxHashMap<u32, Vec<u32>>,
    pub resolved_by_symbol: FxHashMap<u32, Vec<u32>>,
    pub ann_by_symbol: FxHashMap<u32, Vec<u32>>,
    pub ann_by_name: FxHashMap<u32, Vec<u32>>,
    pub module_names: FxHashMap<u32, Vec<u32>>,
    pub module_name_set: FxHashSet<(u32, u32)>,
    // per-content probes
    pub probe: FxHashMap<Pos, (bool, bool)>,
    pub dyn_fq: FxHashMap<u32, u32>,
}

impl Facts {
    pub fn rows(&self) -> usize {
        self.counts.len()
    }

    /// Presence changes `delta` makes, against the current state (nothing applied).
    pub fn classify(&self, delta: &[(Tab, Row, i32)]) -> Vec<Change> {
        let mut net: FxHashMap<(Tab, &Row), i32> = FxHashMap::default();
        for (t, r, d) in delta {
            *net.entry((*t, r)).or_default() += d;
        }
        let mut out = Vec::new();
        for ((t, r), d) in net {
            if d == 0 {
                continue;
            }
            let c = self.counts.get(&(t, r.clone())).copied().unwrap_or(0);
            assert!(c + d >= 0, "negative count for {t:?} {}", show_row(r));
            if (c > 0) != (c + d > 0) {
                out.push(Change { tab: t, row: r.clone(), added: c == 0 });
            }
        }
        // Removals first: a definition replaced at the same id leaves before it re-enters.
        out.sort_by_key(|c| c.added);
        out
    }

    /// Apply `delta`; its presence changes were `changes` (from `classify` on this state).
    pub fn apply(&mut self, delta: &[(Tab, Row, i32)], changes: &[Change]) {
        for (t, r, d) in delta {
            let k = (*t, r.clone());
            let c = self.counts.entry(k.clone()).or_default();
            *c += d;
            if *c == 0 {
                self.counts.remove(&k);
            }
        }
        for ch in changes {
            self.index(ch.tab, &ch.row, ch.added);
        }
    }

    /// Load a snapshot (every row with its count).
    pub fn load(&mut self, rows: Vec<(Tab, Row, i32)>) {
        let ch = self.classify(&rows);
        self.apply(&rows, &ch);
    }

    fn index(&mut self, t: Tab, r: &[Val], add: bool) {
        macro_rules! ix {
            ($m:expr, $k:expr, $v:expr) => {
                if add {
                    push(&mut $m, $k, $v)
                } else {
                    pull(&mut $m, $k, &$v)
                }
            };
        }
        match t {
            Tab::Refs => {
                let (s, d) = ((r[0].s(), r[1].i()), (r[3].s(), r[4].i()));
                ix!(self.refs_from, s, d);
                ix!(self.refs_to, d, s);
            }
            Tab::Ann => {
                ix!(self.ann_by_symbol, r[0].s(), r[1].s());
                ix!(self.ann_by_name, r[1].s(), r[0].s());
            }
            Tab::Imports => {
                let i = Import { module: r[3].s(), name: r[4].s(), target: r[5].s(), symbol_kind: with(r[6].s(), |k| k == "symbol") };
                ix!(self.imports_by_target, i.target, i);
                ix!(self.imports_by_module_name, (i.module, i.name), i);
            }
            Tab::Resolved => {
                ix!(self.resolved_by_fq, r[0].s(), r[1].s());
                ix!(self.resolved_by_symbol, r[1].s(), r[0].s());
            }
            Tab::Bases => {
                ix!(self.bases_by_owner, r[0].s(), r[1].s());
                ix!(self.bases_by_base, r[1].s(), r[0].s());
            }
            Tab::Hier => {
                ix!(self.hier_by_owner, r[0].s(), (r[1].s(), r[2].i()));
                ix!(self.hier_by_base, r[1].s(), (r[0].s(), r[2].i()));
            }
            Tab::Public => {
                ix!(self.public_by_owner, r[0].s(), r[2].s());
                ix!(self.public_by_symbol, r[2].s(), r[0].s());
            }
            Tab::Roots => {
                let x = Root { path: r[0].s(), module: r[1].s(), name: r[2].s(), symbol: r[3].s() };
                ix!(self.roots_by_symbol, x.symbol, x);
            }
            Tab::ModuleNames => {
                let (m, n) = (r[0].s(), r[1].s());
                ix!(self.module_names, m, n);
                if add {
                    self.module_name_set.insert((m, n));
                } else {
                    self.module_name_set.remove(&(m, n));
                }
            }
            Tab::Probe => {
                let p = (r[0].s(), r[1].i());
                if add {
                    assert!(self.probe.insert(p, (r[3].b(), r[4].b())).is_none(), "probe row twice");
                } else {
                    self.probe.remove(&p);
                }
            }
            Tab::Dyn => {
                if let Some(fq) = r[3].os() {
                    let c = self.dyn_fq.entry(fq).or_default();
                    if add {
                        *c += 1;
                    } else {
                        *c -= 1;
                        if *c == 0 {
                            self.dyn_fq.remove(&fq);
                        }
                    }
                }
            }
            Tab::EfTraits => {
                let e = EfDef::of(r);
                if add {
                    assert!(self.ef.insert(e.id, e).is_none(), "ef traits id twice");
                } else {
                    assert_eq!(self.ef.remove(&e.id).as_ref(), Some(&e));
                }
            }
            Tab::DeadTraits => {
                let d = Def::of(r);
                let id = d.id;
                if add {
                    assert!(!self.defs.contains_key(&id), "definition id twice: {}", string(id));
                    assert!(self.by_pos.insert((d.path, d.start), id).is_none(), "two definitions at one position");
                } else {
                    assert_eq!(self.defs.get(&id), Some(&d));
                    self.by_pos.remove(&(d.path, d.start));
                }
                ix!(self.by_module, d.module, id);
                ix!(self.by_module_name, (d.module, d.name), id);
                if let Some(c) = d.class_name {
                    ix!(self.by_module_class, (d.module, c), id);
                }
                if d.kind == Kind::Method {
                    ix!(self.methods_by_name, d.name, id);
                    if let Some(c) = d.class_id {
                        ix!(self.methods_by_class, c, id);
                        ix!(self.methods_by_class_base, (c, d.base_name), id);
                    }
                }
                if d.kind == Kind::Function {
                    ix!(self.functions_by_module_base, (d.module, d.base_name), id);
                }
                if d.kind == Kind::Class {
                    ix!(self.classes_by_name, d.name, id);
                }
                ix!(self.by_fqname, d.fqname, id);
                if add {
                    self.defs.insert(id, d);
                } else {
                    self.defs.remove(&id);
                }
            }
        }
    }
}
