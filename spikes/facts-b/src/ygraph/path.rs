//! Path-set and path-partitioned stages (X5's classes P and C): `module_path.sql` and the
//! substitution / substituted-summary / name-table rows of `reference_edges.sql`, for one path.
//! A path's block is a function of (path, content, the package directories among its ancestors)
//! only; every join in these stages is on path equality.
use super::intern::{Interner, Sym};
use super::summary::{RawDef, Summary, VRows};
use rustc_hash::FxHashSet;

/// `rx_extract(s, '^(.*)\.[^.]*$', 1)` (DOTALL): the text before the last dot.
pub fn parent(s: &str) -> Option<&str> {
    s.rfind('.').map(|i| &s[..i])
}

/// `rx_extract(s, '[^.]*$', 0)`: the text after the last dot (all of `s` without one).
pub fn last(s: &str) -> &str {
    s.rfind('.').map_or(s, |i| &s[i + 1..])
}

/// `coalesce(rx_extract(s, '^(.+)\.[^.]*$', 1), s)`: the last extension off unless the dot leads.
pub fn stem(s: &str) -> &str {
    match s.rfind('.') {
        Some(i) if i > 0 => &s[..i],
        _ => s,
    }
}

fn dir_of(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..i])
}

/// The module a path's in-scope package markers give it (module_path.sql).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleCtx {
    /// None: no module_path row ('module: none').
    pub segments: Option<Vec<String>>,
    pub package: bool,
}

/// `package_climb`/`package_root`/`module_rel`/`module_name_part`/`module_path`/`module_segment`.
pub fn module_ctx(path: &str, package_dirs: &dyn Fn(&str) -> bool) -> ModuleCtx {
    let dir = dir_of(path);
    let file = &path[path.rfind('/').map_or(0, |i| i + 1)..];
    let stem = stem(file);
    let mut root: Option<&str> = None;
    if package_dirs(dir) {
        let mut d = dir;
        loop {
            root = Some(d);
            if d.is_empty() {
                break;
            }
            let p = dir_of(d);
            if !package_dirs(p) {
                break;
            }
            d = p;
        }
    }
    let rel: Option<&str> = match root {
        None | Some("") => None,
        Some(r) if !r.contains('/') => Some(path),
        Some(r) => Some(&path[dir_of(r).len() + 1..]),
    };
    if let Some(rel) = rel {
        let mut parts: Vec<String> = rel.split('/').map(str::to_owned).collect();
        *parts.last_mut().unwrap() = stem.to_owned();
        if parts.iter().all(|p| !p.is_empty()) {
            return ModuleCtx { segments: Some(parts), package: true };
        }
    }
    if !stem.is_empty() {
        return ModuleCtx { segments: Some(vec![stem.to_owned()]), package: false };
    }
    ModuleCtx { segments: None, package: false }
}

/// The definitions of a linked path (`s_def`), as the graph's `Def` table reads them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SDef {
    pub idx: u32,
    pub name: Sym,
    pub start: u32,
    pub stop: u32,
    pub flags: u32,
}

pub const CLASS: u32 = 2;
pub const SHADOWED: u32 = 64;

/// Each def's occurrence among the path's defs of the same name (defs in idx order).
pub fn occurrences(defs: &[SDef]) -> Vec<u32> {
    let mut seen: rustc_hash::FxHashMap<Sym, u32> = Default::default();
    defs.iter()
        .map(|d| {
            let e = seen.entry(d.name).or_insert(0);
            *e += 1;
            *e - 1
        })
        .collect()
}

/// The def (path-local position) a (name, occurrence) identifies in `defs`.
pub fn find_def(defs: &[SDef], name: Sym, occ: u32) -> Option<usize> {
    defs.iter().enumerate().filter(|(_, d)| d.name == name).nth(occ as usize).map(|(i, _)| i)
}

/// Everything one path contributes: its own rows and its rows of the keyed global tables.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct Block {
    pub in_scope: bool,
    pub linked: bool,
    /// path refusal (module, substitution, per-file producer failure)
    pub refused: Option<String>,
    pub open: bool,
    /// module text (`file.module`), for `module_of`
    pub module: Option<Sym>,
    pub defs: Vec<SDef>,
    pub edges: Vec<(u32, Sym)>,
    /// by_name rows: (key, occurrence of the def's name in the path, flags, name). The def is
    /// identified by (path, name, occurrence), not by its idx: an edit that adds a def above
    /// shifts every idx below it but no (name, occurrence).
    pub by_name: Vec<(Sym, u32, u32, Sym)>,
    /// aliases (key, target)
    pub aliases: Vec<(Sym, Sym)>,
    pub opaque: Vec<Sym>,
    pub modules: Vec<Sym>,
    /// supers (cls key, target, ord)
    pub supers: Vec<(Sym, Sym, Sym)>,
    /// class def names (every class s_def row: reference_class counts rows)
    pub class_defs: Vec<Sym>,
    /// marker_class contributions (distinct per path)
    pub markers: Vec<Sym>,
}

fn substitute(raw: &str, n: i64, symlen: i64, prefixes: &[Option<String>]) -> Result<String, &'static str> {
    let mut done = String::new();
    let mut rest = raw.to_owned();
    while let Some(at) = rest.find('\x01') {
        let digits = rest[at + 1..].bytes().take_while(u8::is_ascii_digit).count();
        assert!(digits > 0, "sentinel without a number in {raw:?}");
        let r: i64 = rest[at + 1..at + 1 + digits].parse().unwrap();
        let after = rest[at + 1 + digits..].to_owned();
        let count = r - symlen + n;
        if count < 0 {
            return Err("substitution: relative import above the real package root");
        }
        let Some(Some(text)) = prefixes.get(count as usize) else {
            return Err("substitution: real module segment keeps a dot");
        };
        done.push_str(&rest[..at]);
        done.push_str(text);
        rest = if text.is_empty() && after.starts_with('.') { after[1..].to_owned() } else { after };
    }
    done.push_str(&rest);
    Ok(done)
}

fn is_ident(s: &str) -> bool {
    let mut b = s.bytes();
    matches!(b.next(), Some(c) if c == b'_' || c.is_ascii_alphabetic()) && b.all(|c| c == b'_' || c.is_ascii_alphanumeric())
}

/// `path_alt` before the s_module exclusion: the longest suffix of identifier segments of the
/// path without `.py`/`.pyi` and a trailing `/__init__`, dotted.
fn path_alt(path: &str) -> Option<String> {
    let p = path.strip_suffix(".py").or_else(|| path.strip_suffix(".pyi"))?;
    let p = if p.contains('/') { p.strip_suffix("/__init__").unwrap_or(p) } else { p };
    let segs: Vec<&str> = p.split('/').collect();
    let keep = segs.iter().rev().take_while(|s| is_ident(s)).count();
    if keep == 0 {
        return None;
    }
    Some(segs[segs.len() - keep..].join("."))
}

/// The block of `path` holding `sum`, under module context `ctx`.
pub fn block(path: &str, sum: &Summary, ctx: &ModuleCtx, it: &mut Interner) -> Block {
    let mut b = Block { in_scope: sum.in_scope, ..Default::default() };
    if !sum.in_scope {
        return b;
    }
    if sum.failure {
        b.refused = Some("per-file producer failed".into());
        return b;
    }
    // path_refusal
    let refusal = match &ctx.segments {
        None => Some("module: none (empty stem)"),
        Some(s) if ["abc", "typing", "typing_extensions", "functools"].contains(&s[0].as_str()) => {
            Some("module: named like a module native compares names with (abc, typing, typing_extensions, functools)")
        }
        Some(s) if s.len() > 17 => Some("module: deeper than the symbolic module (17 segments)"),
        Some(s) if s.iter().any(|x| x == "." || x == "..") => Some("module: a name is . or .. (ModulePath::names rejects it)"),
        _ if !path.is_ascii() => Some("path_module: non-ASCII path"),
        _ => None,
    };
    if let Some(r) = refusal {
        b.refused = Some(r.into());
        return b;
    }
    let segs = ctx.segments.as_ref().unwrap();
    let n = segs.len();
    b.module = Some(it.intern(&segs.join(".")));
    // module_prefix
    let mut prefixes: Vec<Option<String>> = vec![Some(String::new())];
    for i in 0..n {
        let s = if i == n - 1 { segs[i].as_str() } else { stem(&segs[i]) };
        let next = match &prefixes[i] {
            None => None,
            Some(_) if s.contains('.') => None,
            Some(_) if i == 0 => Some(s.to_owned()),
            Some(t) => Some(format!("{t}.{s}")),
        };
        prefixes.push(next);
    }
    let (variant, symlen) = if !ctx.package {
        (2, 1)
    } else if n > 1 && segs[n - 1] == "__init__" {
        (1, 17)
    } else {
        (0, 17)
    };
    let v: &VRows = &sum.v[variant];
    // name_sub over every exported string with a sentinel
    let mut subst: rustc_hash::FxHashMap<Sym, Sym> = Default::default();
    {
        let mut raws: Vec<Sym> = Vec::new();
        raws.extend(v.defs.iter().map(|d| d.name));
        raws.extend(v.edges.iter().map(|e| e.1));
        raws.extend(v.aliases.iter().flat_map(|a| [a.0, a.1]));
        raws.extend(v.opaque.iter().copied());
        raws.extend(v.module.iter().copied());
        raws.extend(v.base_slots.iter().flat_map(|a| [a.0, a.1]));
        for r in raws {
            if subst.contains_key(&r) || !it.str(r).contains('\x01') {
                continue;
            }
            match substitute(&it.str(r).to_owned(), n as i64, symlen, &prefixes) {
                Ok(s) => {
                    let s = it.intern(&s);
                    subst.insert(r, s);
                }
                Err(e) => {
                    b.refused = Some(e.into());
                    return b;
                }
            }
        }
    }
    b.linked = true;
    let sub = |s: Sym| *subst.get(&s).unwrap_or(&s);
    // s_def
    b.defs = v.defs.iter().map(|d: &RawDef| SDef { idx: d.idx, name: sub(d.name), start: d.start, stop: d.end, flags: d.flags }).collect();
    b.defs.sort_by_key(|d| d.idx);
    let mut edges: Vec<(u32, Sym)> = v.edges.iter().map(|&(i, nm)| (i, sub(nm))).collect();
    edges.sort_unstable();
    edges.dedup();
    b.edges = edges;
    let mut s_alias: Vec<(Sym, Sym)> = v.aliases.iter().map(|&(k, t)| (sub(k), sub(t))).collect();
    s_alias.sort_unstable();
    s_alias.dedup();
    let mut s_opaque: Vec<Sym> = v.opaque.iter().map(|&o| sub(o)).collect();
    s_opaque.sort_unstable();
    s_opaque.dedup();
    let s_module: Vec<Sym> = v.module.iter().map(|&m| sub(m)).collect();
    b.open = v.open.iter().any(|&o| o != 0);
    let mut slots: Vec<(Sym, Sym)> = v.base_slots.iter().map(|&(k, t)| (sub(k), sub(t))).collect();
    slots.sort_unstable();
    slots.dedup();
    // last_class_def: per class name, the start of the def with the largest idx
    let mut last_class: rustc_hash::FxHashMap<Sym, (u32, u32)> = Default::default();
    for d in b.defs.iter().filter(|d| d.flags & CLASS != 0) {
        let e = last_class.entry(d.name).or_insert((d.idx, d.start));
        if d.idx > e.0 {
            *e = (d.idx, d.start);
        }
    }
    // s_super
    let mut s_super: Vec<(Sym, Sym, Sym)> = Vec::new();
    for &(k, t) in &slots {
        let ks = it.str(k);
        let Some(p) = ks.find(".<base-slot>.") else { continue };
        let cls = &ks[..p];
        let after = &ks[p + 13..];
        let Some(colon) = after.find(':') else { continue };
        let start: i64 = after[..colon].parse().unwrap_or_else(|_| panic!("base-slot start {ks:?}"));
        let ord = after[colon + 1..].to_owned();
        let cls = cls.to_owned();
        let Some(cs) = it.get(&cls) else { continue };
        if last_class.get(&cs).is_some_and(|&(_, s)| s as i64 == start) {
            let ord = it.intern(&ord);
            s_super.push((cs, t, ord));
        }
    }
    // path_alt, spelling
    let alt = path_alt(path).filter(|a| !s_module.iter().any(|&m| it.str(m) == a));
    let mut spell_cache: rustc_hash::FxHashMap<Sym, Vec<Sym>> = Default::default();
    let mut spell = |name: Sym, it: &mut Interner| -> Vec<Sym> {
        if let Some(v) = spell_cache.get(&name) {
            return v.clone();
        }
        let mut out = vec![name];
        if let Some(a) = &alt {
            let ns = it.str(name).to_owned();
            for &m in &s_module {
                let ms = it.str(m);
                if ns == ms || (ns.len() > ms.len() && ns.starts_with(ms) && ns.as_bytes()[ms.len()] == b'.') {
                    let k = format!("{a}{}", &ns[ms.len()..]);
                    out.push(it.intern(&k));
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        spell_cache.insert(name, out.clone());
        out
    };
    let mut by_name = Vec::new();
    for (d, occ) in b.defs.iter().zip(occurrences(&b.defs)) {
        if d.flags & SHADOWED == 0 {
            for k in spell(d.name, it) {
                by_name.push((k, occ, d.flags, d.name));
            }
        }
    }
    by_name.sort_unstable();
    by_name.dedup();
    b.by_name = by_name;
    let mut aliases = Vec::new();
    for &(k, t) in &s_alias {
        for kk in spell(k, it) {
            aliases.push((kk, t));
        }
    }
    aliases.sort_unstable();
    aliases.dedup();
    b.aliases = aliases;
    let mut opaque = Vec::new();
    for &o in &s_opaque {
        opaque.extend(spell(o, it));
    }
    opaque.sort_unstable();
    opaque.dedup();
    b.opaque = opaque;
    let mut modules: Vec<Sym> = s_module.clone();
    if let Some(a) = &alt {
        modules.push(it.intern(a));
    }
    modules.sort_unstable();
    modules.dedup();
    b.modules = modules;
    let mut supers = Vec::new();
    for &(c, t, o) in &s_super {
        for k in spell(c, it) {
            supers.push((k, t, o));
        }
    }
    supers.sort_unstable();
    supers.dedup();
    b.supers = supers;
    b.class_defs = b.defs.iter().filter(|d| d.flags & CLASS != 0).map(|d| d.name).collect();
    b.class_defs.sort_unstable();
    let mut markers: FxHashSet<Sym> = Default::default();
    for &(_, nm) in &b.edges {
        let s = it.str(nm).to_owned();
        for tok in [".<self>.", ".<super>.", ".<field>."] {
            if let Some(p) = s.find(tok) {
                markers.insert(it.intern(&s[..p]));
            }
        }
    }
    b.markers = markers.into_iter().collect();
    b.markers.sort_unstable();
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_paths() {
        let pk = |d: &str| ["a", "a/b"].contains(&d);
        assert_eq!(module_ctx("a/b/c.py", &pk).segments.unwrap(), vec!["a", "b", "c"]);
        // the climb reaches a root '' marker: no rel, the stem fallback
        let pk0 = |d: &str| ["a", "a/b", ""].contains(&d);
        assert_eq!(module_ctx("a/b/c.py", &pk0).segments.unwrap(), vec!["c"]);
        // root '' marked: package_root '' gives no rel, so the stem fallback
        let pk2 = |d: &str| ["", "x"].contains(&d);
        let m = module_ctx("x/y.py", &pk2);
        assert_eq!((m.segments.unwrap(), m.package), (vec!["y".to_string()], false));
        let none = |_: &str| false;
        assert_eq!(module_ctx("q/r.py", &none).segments.unwrap(), vec!["r"]);
        assert_eq!(path_alt("a/b-c/d/__init__.py").as_deref(), Some("d"));
        assert_eq!(path_alt("__init__.py").as_deref(), Some("__init__"));
        assert_eq!(substitute("\x0117.x", 2, 17, &[Some("".into()), Some("a".into()), Some("a.b".into())]).unwrap(), "a.b.x");
        assert_eq!(substitute("\x0115.x", 2, 17, &[Some("".into()), Some("a".into()), Some("a.b".into())]).unwrap(), "x");
    }
}
