//! Per-file derivations: pure functions of (path, content) that turn a content's exported facts
//! into the records the cross-file rules read. Memoizable per (path, content); shared by the
//! dataflow and the from-scratch oracle, so the oracle checks the cross-file evaluation (joins,
//! counts, negation, aggregation, recursion), not these.
//!
//! Files with a syntax error contribute nothing (Jeremy's inputs are `syntax: valid`).
use crate::store::{Cid, Content, Store, Sym, NONE};
use rustc_hash::FxHashMap;

/// A call-graph node: (path, qualname).
pub type Node = (Sym, Sym);

#[derive(Clone, Copy, Debug)]
pub struct PathInfo {
    pub module: Sym,
    /// The module a relative import inside this file is relative to (Jeremy's `prefix`).
    pub package: Sym,
    pub is_init: bool,
    /// internal-dead-code scope, `^(api|data_layer)\.` module filter and `impl/browser/` filter.
    pub r1_scope: bool,
    /// serverplan_state_tripwires scope.
    pub r3_scope: bool,
    /// `^data_layer/[^/]+\.py$`
    pub export_file: bool,
    pub internal: bool,
}

const R1_INCLUDE: &[&str] = &[
    "api", "impl", "data_layer", "_internal", "l1000_cache", "l1000_schema", "l1050_model_registry", "l1050_polymorphism",
    "l1100_exceptions", "l1200_capabilities", "l1200_content_hash", "l1200_json", "l1200_versioned_ids", "l1250_security",
    "l1300_session", "l1350_foundational_types", "l1400_config", "l1400_types", "l1450_index_policy_contracts", "l1450_keys",
    "l1470_matching_contracts", "l1500_llm", "l1900_retry", "l2000_connection", "l2050_transaction", "l2055_lazy_result",
    "l2060_query_builder", "l2070_batching", "l2080_lazy_orm", "l2090_byte_tracking", "l2090_response", "l2091_rtt_tracking",
    "l2092_timing_tracking", "l2100_include", "l2110_limits", "l2150_prepared_cache", "l2200_notification", "l2500_binary_diff",
    "l3000_schema_builder", "l3100_schema_introspector", "l3150_schema_diff", "l3200_metadata", "l3250_migrations",
    "l3260_startup", "l3500_bulk_ops", "l4000_permissions", "l4050_background_reaper", "l4100_queue", "l4150_scheduler",
    "l4170_inbound_bridge", "l4200_event_pipeline", "l4250_timers", "l4300_ipc", "l4300_timers", "l4350_timer_driver",
    "l4400_scheduler", "l4500_trust", "l5000_search_index_policy", "l5000_subscriptions", "l5100_fts", "l5100_llm_router",
    "l5175_chunking", "l5195_embedding_compute_tables", "l5200_vector_index", "l5250_matching", "l5300_config_compatibility",
    "l6000_log_table", "l6300_config", "l6500_evidence", "l6510_credentials", "l6520_auth", "l6530_managed_secrets",
    "l6600_cleanup", "l7000_versioned_schema", "l7100_versioned_operations", "l7200_versioned_transactions",
    "l7250_versioned_operations", "l7350_versioned_timer_driver", "l7390_versioned_diff_entrypoint", "l7400_versioned_diff",
    "l7450_versioned_revert", "l7500_commit_triggers", "l7800_versioned_merge", "l7900_versioned_rebase", "l8000_admin",
    "l8100_ops_analytics", "l9600_graphs", "l9700_graph_query", "l9710_graph_algorithms", "l9720_sparql", "l9750_graphql",
    "l9800_rippling", "l9850_workflow", "l9875_mcp", "l9900_auto_api",
];

pub fn module_of(path: &str) -> String {
    let stem = path.strip_suffix("/__init__.py").or_else(|| path.strip_suffix(".py")).unwrap_or(path);
    stem.replace('/', ".")
}

fn parent(m: &str) -> &str {
    m.rfind('.').map(|i| &m[..i]).unwrap_or("")
}

fn path_info(path: &str, syms: &mut crate::store::Interner) -> PathInfo {
    let module = module_of(path);
    let is_init = path.ends_with("/__init__.py");
    let package = if is_init { module.clone() } else { parent(&module).to_string() };
    let parts: Vec<&str> = path.split('/').collect();
    let dirs = &parts[..parts.len() - 1];
    let r1_excluded = dirs.iter().any(|d| matches!(*d, ".git" | ".venv" | "tmp" | "__pycache__" | "tests"));
    let r1_scope = parts.len() > 1
        && R1_INCLUDE.contains(&parts[0])
        && !r1_excluded
        && !(module.starts_with("api.") || module.starts_with("data_layer."))
        && !path.starts_with("impl/browser/");
    let r3_excluded = dirs.iter().any(|d| matches!(*d, "tests" | "test" | "__pycache__" | "tmp" | "venv" | ".venv" | ".git"));
    let first = parts[0];
    let lnum = first.len() > 6
        && first.as_bytes()[0] == b'l'
        && first.as_bytes()[1..5].iter().all(|b| b.is_ascii_digit())
        && first.as_bytes()[5] == b'_';
    let r3_scope = parts.len() > 1 && (matches!(first, "impl" | "data_layer" | "_internal") || lnum) && !r3_excluded;
    let export_file = parts.len() == 2 && parts[0] == "data_layer";
    PathInfo {
        module: syms.intern(&module),
        package: syms.intern(&package),
        is_init,
        r1_scope,
        r3_scope,
        export_file,
        internal: path.starts_with("_internal/"),
    }
}

/// Fixed symbols the rules compare against.
pub struct Fixed {
    pub function: Sym,
    pub method: Sym,
    pub class: Sym,
    pub init: Sym,
}

pub struct World {
    pub store: Store,
    pub paths: FxHashMap<Sym, PathInfo>,
    pub fixed: Fixed,
    /// Absolute module of a relative import: (package, raw module) -> module.
    abs_cache: FxHashMap<(Sym, Sym), Sym>,
    /// `qualname + ".__init__"`
    init_cache: FxHashMap<Sym, Sym>,
    /// `module + "." + dotted middle`
    join_cache: FxHashMap<(Sym, Sym), Sym>,
}

impl World {
    pub fn new(mut store: Store) -> Self {
        let s = &mut store.syms;
        let fixed = Fixed { function: s.intern("function"), method: s.intern("method"), class: s.intern("class"), init: s.intern("__init__") };
        World { store, paths: FxHashMap::default(), fixed, abs_cache: FxHashMap::default(), init_cache: FxHashMap::default(), join_cache: FxHashMap::default() }
    }

    pub fn path(&mut self, p: &str) -> Sym {
        let s = self.store.syms.intern(p);
        if !self.paths.contains_key(&s) {
            let info = path_info(p, &mut self.store.syms);
            self.paths.insert(s, info);
        }
        s
    }

    pub fn info(&self, p: Sym) -> PathInfo {
        self.paths[&p]
    }

    fn content(&self, c: Cid) -> &Content {
        &self.store.contents[c as usize]
    }

    /// Absolute module for an import's module text as written in a file of package `pkg`.
    fn absolute(&mut self, pkg: Sym, raw: Sym) -> Sym {
        if let Some(&m) = self.abs_cache.get(&(pkg, raw)) {
            return m;
        }
        let text = self.store.syms.str(raw).to_string();
        let level = text.chars().take_while(|&c| c == '.').count();
        let m = if level == 0 {
            raw
        } else {
            let mut prefix = self.store.syms.str(pkg).to_string();
            for _ in 1..level {
                prefix = parent(&prefix).to_string();
            }
            let rest = &text[level..];
            let full = match (prefix.is_empty(), rest.is_empty()) {
                (true, _) => rest.to_string(),
                (false, true) => prefix,
                (false, false) => format!("{prefix}.{rest}"),
            };
            self.store.syms.intern(&full)
        };
        self.abs_cache.insert((pkg, raw), m);
        m
    }

    fn init_of(&mut self, q: Sym) -> Sym {
        if let Some(&s) = self.init_cache.get(&q) {
            return s;
        }
        let s = self.store.syms.intern(&format!("{}.__init__", self.store.syms.str(q)));
        self.init_cache.insert(q, s);
        s
    }

    fn join_mod(&mut self, m: Sym, mid: &str) -> Sym {
        if mid.is_empty() {
            return m;
        }
        let k = self.store.syms.intern(mid);
        if let Some(&s) = self.join_cache.get(&(m, k)) {
            return s;
        }
        let ms = self.store.syms.str(m);
        let full = if ms.is_empty() { mid.to_string() } else { format!("{ms}.{mid}") };
        let s = self.store.syms.intern(&full);
        self.join_cache.insert((m, k), s);
        s
    }

    /// Declarations: defs deduplicated by qualname, the last (max line) kept, as Jeremy's
    /// `declarations` keeps the last by start byte per id.
    pub fn decls(&self, c: Cid) -> Vec<(Sym, Sym, Sym, u32)> {
        let ct = self.content(c);
        if ct.has_error {
            return Vec::new();
        }
        let mut best: FxHashMap<Sym, (Sym, Sym, Sym, u32)> = FxHashMap::default();
        for &d in &ct.defs {
            let e = best.entry(d.2).or_insert(d);
            if d.3 > e.3 {
                *e = d;
            }
        }
        let mut v: Vec<_> = best.into_values().collect();
        v.sort_unstable();
        v
    }

    // ----- R1 ---------------------------------------------------------------------------------

    pub fn r1_refs(&self, c: Cid) -> Vec<Sym> {
        let ct = self.content(c);
        if ct.has_error { Vec::new() } else { ct.refs.clone() }
    }

    /// (name, path, qualname) of the candidate declarations of an in-scope file.
    pub fn r1_cand(&self, p: Sym, c: Cid) -> Vec<(Sym, Sym, Sym)> {
        if !self.info(p).r1_scope {
            return Vec::new();
        }
        let syms = &self.store.syms;
        self.decls(c)
            .into_iter()
            .filter(|&(name, _, _, _)| {
                let n = syms.str(name);
                let dunder = n.len() >= 4 && n.starts_with("__") && n.ends_with("__");
                !dunder && !matches!(n, "main" | "pytest_configure" | "pytest_collection_modifyitems")
            })
            .map(|(name, _, q, _)| (name, p, q))
            .collect()
    }

    // ----- R2 ---------------------------------------------------------------------------------

    /// Symbol imports after Jeremy's dedupe: the last per local name. (local, abs module, name, top).
    fn symbol_bindings(&mut self, p: Sym, c: Cid) -> Vec<(Sym, Sym, Sym, bool)> {
        let ct = &self.store.contents[c as usize];
        if ct.has_error {
            return Vec::new();
        }
        let mut last: FxHashMap<Sym, (Sym, Sym, bool)> = FxHashMap::default();
        for i in ct.imports.clone() {
            if i.name != NONE {
                last.insert(i.local, (i.module, i.name, i.top));
            }
        }
        let pkg = self.info(p).package;
        let mut out: Vec<_> = last.into_iter().map(|(l, (m, n, t))| (l, m, n, t)).collect();
        for r in &mut out {
            r.1 = self.absolute(pkg, r.1);
        }
        out.sort_unstable();
        out
    }

    /// Top-level declarations keyed by (module, name): value (path, flags) with flags bit0 =
    /// function, bit1 = legacy_async decorator.
    pub fn r2_decl(&self, p: Sym, c: Cid) -> Vec<((Sym, Sym), (Sym, u8))> {
        let m = self.info(p).module;
        let ct = self.content(c);
        let syms = &self.store.syms;
        self.decls(c)
            .into_iter()
            .filter(|&(_, k, _, _)| k == self.fixed.function || k == self.fixed.class)
            .map(|(name, k, q, _)| {
                let legacy = ct.decorators.iter().any(|&(dq, dn)| {
                    dq == q && {
                        let s = syms.str(dn);
                        s.ends_with("legacy_async") || s.ends_with("legacy_module_async") || s.ends_with("direct_legacy_module_async")
                    }
                });
                ((m, name), (p, (k == self.fixed.function) as u8 | (legacy as u8) << 1))
            })
            .collect()
    }

    /// Re-export hops: (module, local) -> (abs module, name), from every symbol binding.
    pub fn r2_bind(&mut self, p: Sym, c: Cid) -> Vec<((Sym, Sym), (Sym, Sym))> {
        let m = self.info(p).module;
        self.symbol_bindings(p, c).into_iter().map(|(l, am, n, _)| ((m, l), (am, n))).collect()
    }

    /// Export roots of an export file: target (module, name) -> (export module, export name).
    pub fn r2_export(&mut self, p: Sym, c: Cid) -> Vec<((Sym, Sym), (Sym, Sym))> {
        let info = self.info(p);
        if !info.export_file {
            return Vec::new();
        }
        let m = info.module;
        let mut out = Vec::new();
        for (l, am, n, top) in self.symbol_bindings(p, c) {
            if top {
                out.push(((am, n), (m, l)));
            }
        }
        for ((_, name), _) in self.r2_decl(p, c) {
            out.push(((m, name), (m, name)));
        }
        let ct = self.content(c);
        let syms = &self.store.syms;
        out.retain(|&(_, (_, name))| {
            !syms.str(name).starts_with('_')
                && match ct.dunder_all {
                    None => true,
                    Some(false) => false,
                    Some(true) => ct.dunder_all_names.contains(&name),
                }
        });
        out.sort_unstable();
        out.dedup();
        out
    }

    // ----- R3 ---------------------------------------------------------------------------------

    /// Call targets keyed by (module, name): a top-level function, or a top-level class's
    /// `__init__` declared in the same file.
    pub fn r3_target(&mut self, p: Sym, c: Cid) -> Vec<((Sym, Sym), Node)> {
        let m = self.info(p).module;
        let decls = self.decls(c);
        let mut out = Vec::new();
        for &(name, k, q, _) in &decls {
            if k == self.fixed.function {
                out.push(((m, name), (p, q)));
            } else if k == self.fixed.class {
                let init = self.init_of(q);
                if decls.iter().any(|d| d.2 == init && d.1 == self.fixed.method) {
                    out.push(((m, name), (p, init)));
                }
            }
        }
        out
    }

    /// Call sites: (caller node, Local(callee node) | Ext(module, name)).
    pub fn r3_calls(&mut self, p: Sym, c: Cid) -> (Vec<(Node, Node)>, Vec<((Sym, Sym), Node)>) {
        let decls = self.decls(c);
        let ct = self.store.contents[c as usize].clone();
        let pkg = self.info(p).package;
        let mut local_edges = Vec::new();
        let mut ext = Vec::new();
        if ct.has_error {
            return (local_edges, ext);
        }
        let (f_fn, f_m, f_cls) = (self.fixed.function, self.fixed.method, self.fixed.class);
        let callable = |q: Sym| decls.iter().any(|d| d.2 == q && (d.1 == f_fn || d.1 == f_m));
        let kind_of = |q: Sym| decls.iter().find(|d| d.2 == q).map(|d| d.1);
        // Module-scope bindings: last import per local name, and top-level declarations.
        let mut imp: FxHashMap<Sym, (Sym, Sym)> = FxHashMap::default(); // local -> (raw module, name|NONE)
        for i in &ct.imports {
            if i.module_scope {
                imp.insert(i.local, (i.module, i.name));
            }
        }
        for &(caller, callee) in &ct.calls {
            let Some(ck) = kind_of(caller) else { continue };
            if ck != f_fn && ck != f_m {
                continue;
            }
            let from = (p, caller);
            let text = self.store.syms.str(callee).to_string();
            let parts: Vec<&str> = text.split('.').collect();
            let head = parts[0];
            let head_sym = self.store.syms.intern(head);
            let decl_head = kind_of(head_sym).filter(|&k| k == f_fn || k == f_cls);
            let imp_head = imp.get(&head_sym).copied();
            if parts.len() == 1 {
                match (decl_head, imp_head) {
                    (Some(k), None) => {
                        if k == f_fn {
                            local_edges.push((from, (p, head_sym)));
                        } else {
                            let init = self.init_of(head_sym);
                            if callable(init) {
                                local_edges.push((from, (p, init)));
                            }
                        }
                    }
                    (None, Some((raw, name))) if name != NONE => {
                        let am = self.absolute(pkg, raw);
                        ext.push(((am, name), from));
                    }
                    _ => {}
                }
                continue;
            }
            if (head == "self" || head == "cls") && parts.len() == 2 && ck == f_m {
                let cls = self.store.syms.str(caller).rsplit_once('.').map(|x| x.0.to_string()).unwrap();
                if let Some(q) = self.store.syms.get(&format!("{cls}.{}", parts[1])) {
                    if callable(q) {
                        local_edges.push((from, (p, q)));
                    }
                }
                continue;
            }
            match (decl_head, imp_head) {
                (Some(k), None) if k == f_cls && parts.len() == 2 => {
                    if let Some(q) = self.store.syms.get(&text) {
                        if callable(q) {
                            local_edges.push((from, (p, q)));
                        }
                    }
                }
                (None, Some((raw, NONE))) => {
                    // `import a.b.c` binds `a`; `import a.b as x` binds `x` to `a.b`.
                    let raw_s = self.store.syms.str(raw).to_string();
                    let bound = if raw_s.split('.').next() == Some(head) { head.to_string() } else { raw_s };
                    let bound = self.store.syms.intern(&bound);
                    let mid = parts[1..parts.len() - 1].join(".");
                    let module = self.join_mod(bound, &mid);
                    let name = self.store.syms.intern(parts[parts.len() - 1]);
                    ext.push(((module, name), from));
                }
                _ => {}
            }
        }
        local_edges.sort_unstable();
        local_edges.dedup();
        ext.sort_unstable();
        ext.dedup();
        (local_edges, ext)
    }

    pub fn r3_seeds(&self, p: Sym, c: Cid) -> Vec<(Node, Sym)> {
        let decls = self.decls(c);
        let ct = self.content(c);
        if ct.has_error {
            return Vec::new();
        }
        ct.seeds
            .iter()
            .filter(|(q, _)| decls.iter().any(|d| d.2 == *q && (d.1 == self.fixed.function || d.1 == self.fixed.method)))
            .map(|&(q, fam)| ((p, q), fam))
            .collect()
    }
}
