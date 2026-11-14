//! The membership-dependent stages of `reference_edges.sql` as keyed queries. Each query
//! evaluates its SQL stage(s) for one key, reading the keyed tables and lower-level queries
//! through `Ctx` (which records every probe). Stage names are cited per query; the semantics are
//! the SQL's, with two documented exceptions in `link_result`: no `link_reach` demand cut (a
//! link is evaluated where a caller needs it, at the caller's hops) and hop sets instead of
//! `[lo, hi]` tuples (equal while no range has lo < 0 ≤ hi; counted in `Anomalies`).
use super::engine::*;
use super::intern::Sym;
use super::path::{last, parent, CLASS};
use std::cmp::Ordering;

type Rows<'a> = &'a [(Row, u32)];

fn cls_flag(r: &(Row, u32)) -> bool {
    r.0[2] & CLASS != 0
}

fn class_key(rows: Rows) -> bool {
    !rows.is_empty() && rows.iter().all(cls_flag)
}

/// Distinct targets of alias rows (path, target).
fn targets(rows: Rows) -> Vec<Sym> {
    let mut v: Vec<Sym> = rows.iter().map(|r| r.0[1]).collect();
    v.sort_unstable();
    v.dedup();
    v
}

fn cmp_s(cx: &Ctx, a: Sym, b: Sym) -> Ordering {
    cx.it.str(a).cmp(cx.it.str(b))
}

fn min_s(cx: &Ctx, xs: impl IntoIterator<Item = Sym>) -> Option<Sym> {
    xs.into_iter().min_by(|&a, &b| cmp_s(cx, a, b))
}

fn intern(cx: &mut Ctx, s: &str) -> Sym {
    cx.it.intern(s)
}

fn s(cx: &Ctx, x: Sym) -> String {
    cx.it.str(x).to_owned()
}

/// `uniform_alias`: the target, when every entry of the key agrees.
fn uniform_alias(cx: &mut Ctx, k: &str) -> Option<Sym> {
    let t = targets(cx.rows(T_ALIAS, k));
    (t.len() == 1).then(|| t[0])
}

/// `named_prefix`.
fn named_prefix(cx: &mut Ctx, k: &str) -> bool {
    if let Some(t) = uniform_alias(cx, k) {
        if cx.it.str(t) != k {
            return true;
        }
    }
    !cx.rows(T_BYNAME, k).is_empty() || !cx.rows(T_OPAQUE, k).is_empty() || !cx.rows(T_MODULES, k).is_empty()
}

/// `prefix_alias`.
fn prefix_alias(cx: &mut Ctx, k: &str) -> bool {
    for t in targets(cx.rows(T_ALIAS, k)) {
        let ts = s(cx, t);
        if ts != k && (cx.rows(T_MODULES, &ts).is_empty() || !cx.rows(T_BYNAME, &ts).is_empty()) {
            return true;
        }
    }
    false
}

/// `own_name`: (min name, distinct names) of a by_name key.
fn own_name(cx: &Ctx, rows: Rows) -> Option<(Sym, usize)> {
    let mut names: Vec<Sym> = rows.iter().map(|r| r.0[3]).collect();
    names.sort_unstable();
    names.dedup();
    let n = names.len();
    min_s(cx, names).map(|m| (m, n))
}

/// `reference_class`: the one class def of that short name.
fn reference_class(cx: &mut Ctx, short: &str) -> Option<Sym> {
    let rows = cx.rows(T_CLASSSHORT, short);
    let total: u32 = rows.iter().map(|r| r.1).sum();
    (total == 1).then(|| rows[0].0[0])
}

/// `method_def` rows of key `k`: (path, idx) of the non-class by_name entries.
fn method_defs(cx: &mut Ctx, k: &str) -> Vec<u32> {
    let rows = cx.rows(T_BYNAME, k);
    let mut out = Vec::new();
    for r in rows.iter().filter(|r| !cls_flag(r)) {
        out.push(cx.defs.id(r.0[0], r.0[3], r.0[1]));
    }
    out
}

fn has_method(cx: &mut Ctx, k: &str) -> bool {
    cx.rows(T_BYNAME, k).iter().any(|r| !cls_flag(r))
}

fn q(kind: u8, a: u32, b: u32) -> Q {
    Q { kind, a, b }
}

pub fn exec(cx: &mut Ctx, qq: Q) -> V {
    match qq.kind {
        K_BASECLASS => base_class(cx, qq.a),
        K_FQ => fq_class(cx, qq.a),
        K_BASES => bases(cx, qq.a),
        K_BASETARGETS => base_targets(cx, qq.a),
        K_TYPEKEY => type_key(cx, qq.a),
        K_CLASSOF => class_of(cx, qq.a),
        K_CLASSROOT => class_root(cx, qq.a),
        K_SUBCLASS => subclass(cx, qq.a),
        K_ORDER => order(cx, qq.a),
        K_OWNREF => own_refusal(cx, qq.a),
        K_MF0 | K_MF1 => method_first(cx, qq.a, qq.b, qq.kind == K_MF1),
        K_FIELD => field_class(cx, qq.a, qq.b),
        K_REFSUP => supers_refusal(cx, qq.a),
        K_COMPOSE => compose(cx, qq.a, qq.b),
        K_BODY => body(cx, qq.a),
        K_LINK => link(cx, qq.a, qq.b),
        _ => unreachable!(),
    }
}

/// `base_target` + `base_class` for one `<base>` target of `supers`.
fn base_class(cx: &mut Ctx, t: Sym) -> V {
    if cx.rows_sym(T_SUPERS_T, t).is_empty() {
        return V::BaseClass(None);
    }
    let ts = s(cx, t);
    let Some(p) = ts.find(".<base>.") else { return V::BaseClass(None) };
    let (module, short) = (&ts[..p], &ts[p + 8..]);
    for kind in ["<imported>", "<class>"] {
        let mut found: Vec<Sym> = Vec::new();
        for tg in targets(cx.rows(T_ALIAS, &format!("{module}.{kind}.{short}"))) {
            let tgs = s(cx, tg);
            if class_key(cx.rows(T_BYNAME, &tgs)) {
                found.push(tg);
            }
        }
        if !found.is_empty() {
            found.sort_unstable();
            found.dedup();
            let n = found.len() as u32;
            return V::BaseClass(Some((min_s(cx, found), n)));
        }
    }
    V::BaseClass(Some((reference_class(cx, short), 0)))
}

fn base_class_of(cx: &mut Ctx, t: Sym) -> Option<(Option<Sym>, u32)> {
    match &*cx.get(q(K_BASECLASS, t, 0)) {
        V::BaseClass(b) => *b,
        _ => unreachable!(),
    }
}

/// `bases`: a class's `<base>` supers resolved to one workspace class, (ord, base).
fn bases(cx: &mut Ctx, c: Sym) -> V {
    let rows = cx.rows_sym(T_SUPERS, c);
    let mut out = Vec::new();
    for r in rows {
        let (target, ord) = (r.0[1], r.0[2]);
        if !cx.it.str(target).contains(".<base>.") {
            continue;
        }
        if let Some((Some(b), several)) = base_class_of(cx, target) {
            if several <= 1 {
                out.push((ord, b));
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    V::Pairs(out)
}

fn bases_of(cx: &mut Ctx, c: Sym) -> Vec<(Sym, Sym)> {
    match &*cx.get(q(K_BASES, c, 0)) {
        V::Pairs(v) => v.clone(),
        _ => unreachable!(),
    }
}

/// The `supers` targets whose base_class is `b` (any `several`): the reverse of base_class,
/// through the base-class alias index (`T_ALIAS_T`: aliases by target, only keys some `supers`
/// target resolves through) and the `<base>` targets by short name.
fn base_targets(cx: &mut Ctx, b: Sym) -> V {
    let mut cand: Vec<Sym> = Vec::new();
    let alias_rows = cx.rows_sym(T_ALIAS_T, b);
    let mut keys: Vec<Sym> = alias_rows.iter().map(|r| r.0[1]).collect();
    keys.sort_unstable();
    keys.dedup();
    for k in keys {
        let ks = s(cx, k);
        for t in super::world::base_targets_of_key(&ks) {
            if !cx.rows(T_SUPERS_T, &t).is_empty() {
                cand.push(cx.it.get(&t).unwrap());
            }
        }
    }
    let bs = s(cx, b);
    for r in cx.rows(T_SUPERS_S, last(&bs)) {
        cand.push(r.0[1]);
    }
    cand.sort_unstable();
    cand.dedup();
    let mut out = Vec::new();
    for t in cand {
        if let Some((Some(c), several)) = base_class_of(cx, t) {
            if c == b {
                out.push((t, several));
            }
        }
    }
    V::Targets(out)
}

fn base_targets_of(cx: &mut Ctx, b: Sym) -> Vec<(Sym, u32)> {
    match &*cx.get(q(K_BASETARGETS, b, 0)) {
        V::Targets(v) => v.clone(),
        _ => unreachable!(),
    }
}

/// `class_root`.
fn class_root(cx: &mut Ctx, c: Sym) -> V {
    let cs = s(cx, c);
    let yes = class_key(cx.rows_sym(T_BYNAME, c))
        || !cx.rows_sym(T_SUPERS, c).is_empty()
        || !cx.rows_sym(T_MARKER, c).is_empty()
        || reference_class(cx, last(&cs)) == Some(c)
        || base_targets_of(cx, c).iter().any(|&(_, several)| several <= 1);
    V::Bool(yes)
}

/// `subclass` by class: the children whose workspace base has own name `a`.
fn subclass(cx: &mut Ctx, a: Sym) -> V {
    let mut keys: Vec<Sym> = cx.rows_sym(T_BYNAME_D, a).iter().map(|r| r.0[1]).collect();
    keys.sort_unstable();
    keys.dedup();
    let mut out = Vec::new();
    for b in keys {
        let rows = cx.rows_sym(T_BYNAME, b);
        if own_name(cx, rows).map(|o| o.0) != Some(a) {
            continue;
        }
        for (t, several) in base_targets_of(cx, b) {
            if several > 1 {
                continue;
            }
            let srows = cx.rows_sym(T_SUPERS_T, t);
            for r in srows {
                let child = r.0[1];
                let crow = cx.rows_sym(T_BYNAME, child);
                if own_name(cx, crow).map(|o| o.0) == Some(child) {
                    out.push(child);
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    V::Syms(out)
}

/// `order_dist` + `reference_order` for one root: BFS positions (dist, least ordinal sequence).
fn order(cx: &mut Ctx, root: Sym) -> V {
    if !matches!(*cx.get(q(K_CLASSROOT, root, 0)), V::Bool(true)) {
        return V::Order(Vec::new(), false);
    }
    let mut seen: rustc_hash::FxHashMap<Sym, (u8, String)> = Default::default();
    seen.insert(root, (0, String::new()));
    let mut layer: Vec<(Sym, String)> = vec![(root, String::new())];
    let mut deep = false;
    let mut ord_len: Option<usize> = None;
    for d in 0..64u8 {
        let mut next: rustc_hash::FxHashMap<Sym, String> = Default::default();
        for (c, seq) in &layer {
            for (ord, base) in bases_of(cx, *c) {
                if seen.contains_key(&base) {
                    continue;
                }
                let o = cx.it.str(ord);
                match ord_len {
                    None => ord_len = Some(o.len()),
                    Some(l) if l != o.len() => cx.anomalies.ord_lengths += 1,
                    _ => {}
                }
                let cand = format!("{seq}{o}");
                match next.get_mut(&base) {
                    Some(cur) if *cur <= cand => {}
                    Some(cur) => *cur = cand,
                    None => {
                        next.insert(base, cand);
                    }
                }
            }
        }
        if next.is_empty() {
            break;
        }
        if d + 1 >= 64 {
            deep = true;
        }
        layer = next.into_iter().collect();
        for (c, sq) in &layer {
            seen.insert(*c, (d + 1, sq.clone()));
        }
    }
    let mut out: Vec<(Sym, u8, Sym)> = seen.into_iter().map(|(c, (d, sq))| (c, d, cx.it.intern(&sq))).collect();
    out.sort_unstable();
    V::Order(out, deep)
}

/// reference_order of `root` with each seq as text (compared as text).
fn order_of(cx: &mut Ctx, root: Sym) -> Vec<(Sym, u8, Box<str>)> {
    match &*cx.get(q(K_ORDER, root, 0)) {
        V::Order(v, _) => v.iter().map(|&(c, d, s)| (c, d, cx.it.str(s).into())).collect(),
        _ => unreachable!(),
    }
}

/// `imported_alias` + `fq_class` for one name: classes reachable through `<imported>` aliases,
/// with the least hop count `k` (≤ 17).
fn fq_class(cx: &mut Ctx, n: Sym) -> V {
    let mut seen: rustc_hash::FxHashSet<Sym> = Default::default();
    seen.insert(n);
    let mut layer = vec![n];
    let mut out = Vec::new();
    for k in 0..=17i32 {
        let mut next = Vec::new();
        for &x in &layer {
            let rows = cx.rows_sym(T_BYNAME, x);
            if class_key(rows) {
                out.push((x, k));
                continue;
            }
            if !rows.is_empty() || k == 17 {
                continue;
            }
            let xs = s(cx, x);
            let Some(par) = parent(&xs) else { continue };
            let key = format!("{par}.<imported>.{}", last(&xs));
            if key.find(".<imported>.") != Some(par.len()) {
                continue;
            }
            for t in targets(cx.rows(T_ALIAS, &key)) {
                if seen.insert(t) {
                    next.push(t);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        layer = next;
    }
    out.sort_unstable();
    V::SymK(out)
}

/// `type_key` for one alias key (`.<imported>.` or `.<class>.`).
fn type_key(cx: &mut Ctx, key: Sym) -> V {
    let ks = s(cx, key);
    if !ks.contains(".<imported>.") && !ks.contains(".<class>.") {
        return V::TypeKey(None);
    }
    let mut cls: Vec<Sym> = Vec::new();
    let mut kmin = i32::MAX;
    for t in targets(cx.rows_sym(T_ALIAS, key)) {
        if let V::SymK(v) = &*cx.get(q(K_FQ, t, 0)) {
            for &(c, k) in v {
                cls.push(c);
                kmin = kmin.min(k);
            }
        }
    }
    if cls.is_empty() {
        return V::TypeKey(None);
    }
    cls.sort_unstable();
    cls.dedup();
    let n = cls.len() as u32;
    V::TypeKey(Some((min_s(cx, cls).unwrap(), kmin, n)))
}

/// type_key at a key text: absent aliases (probed here) give none without a query.
fn type_key_at(cx: &mut Ctx, key: &str) -> Option<(Sym, i32, u32)> {
    if cx.rows(T_ALIAS, key).is_empty() {
        return None;
    }
    let k = cx.it.get(key).unwrap();
    match &*cx.get(q(K_TYPEKEY, k, 0)) {
        V::TypeKey(t) => *t,
        _ => unreachable!(),
    }
}

/// `class_of_walk` + `class_of` for one start: every class the walk passes, with its hops.
fn class_of(cx: &mut Ctx, start: Sym) -> V {
    let st = s(cx, start);
    if st.starts_with("<annotation>.") {
        return V::SymK(Vec::new());
    }
    // rows (cur, k, p)
    let mut seen: rustc_hash::FxHashSet<(String, i32, Option<String>)> = Default::default();
    let mut work: Vec<(String, i32, Option<String>)> = vec![(st, 0, None)];
    let mut names: Vec<(String, i32)> = Vec::new();
    while let Some(row) = work.pop() {
        if !seen.insert(row.clone()) {
            continue;
        }
        let (cur, k, p) = row;
        match p {
            None => {
                names.push((cur.clone(), k));
                if k < 17 && !cur.contains(".<base>.") && cx.rows(T_BYNAME, &cur).is_empty() && cx.rows(T_OPAQUE, &cur).is_empty() {
                    if let Some(t) = uniform_alias(cx, &cur) {
                        work.push((s(cx, t), k + 1, None));
                    }
                    if cur.contains('.') && cx.rows(T_ALIAS, &cur).is_empty() {
                        let par = parent(&cur).unwrap().to_owned();
                        work.push((cur, k, Some(par)));
                    }
                }
            }
            Some(p) => {
                if p.contains('.') && (p.len() + 1 >= cur.len() || !named_prefix(cx, &p)) {
                    work.push((cur.clone(), k, Some(parent(&p).unwrap().to_owned())));
                }
                if p.len() + 1 < cur.len() {
                    if let Some(t) = uniform_alias(cx, &p) {
                        let ts = s(cx, t);
                        if ts != p {
                            work.push((format!("{ts}{}", &cur[p.len()..]), k + 1, None));
                        }
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    for (cur, k) in names {
        if cur.contains(".<base>.") {
            if cx.rows(T_SUPERS_T, &cur).is_empty() {
                continue;
            }
            let t = cx.it.get(&cur).unwrap();
            if let Some((Some(b), several)) = base_class_of(cx, t) {
                if several <= 1 {
                    out.push((b, k));
                }
            }
        } else if class_key(cx.rows(T_BYNAME, &cur)) {
            out.push((cx.it.get(&cur).unwrap(), k));
        }
    }
    out.sort_unstable();
    out.dedup();
    V::SymK(out)
}

/// `method_candidate` + `method_first`: the first classes in BFS order with a function `attr`.
fn method_first(cx: &mut Ctx, root: Sym, attr: Sym, skip: bool) -> V {
    let order = order_of(cx, root);
    let at = s(cx, attr);
    let mut best: Option<(u8, Box<str>)> = None;
    let mut out = Vec::new();
    for (c, d, seq) in order {
        if skip && d == 0 {
            continue;
        }
        let key = format!("{}.{at}", cx.it.str(c));
        if !has_method(cx, &key) {
            continue;
        }
        let pos = (d, seq);
        match &best {
            Some(b) if *b < pos => {}
            Some(b) if *b == pos => out.push(c),
            _ => {
                best = Some(pos);
                out.clear();
                out.push(c);
            }
        }
    }
    out.sort_unstable();
    V::Syms(out)
}

fn syms(v: &V) -> Vec<Sym> {
    match v {
        V::Syms(x) => x.clone(),
        _ => unreachable!(),
    }
}

/// `field_candidate` + `field_hit` + `field_class` for (class, field).
fn field_class(cx: &mut Ctx, cls: Sym, field: Sym) -> V {
    let order = order_of(cx, cls);
    let f = s(cx, field);
    let fl = f.trim_start_matches('_').to_owned();
    let mut best: Option<(u8, Box<str>)> = None;
    let mut hits: Vec<String> = Vec::new();
    for (c, d, seq) in order {
        let cs = s(cx, c);
        let ip = format!("{cs}.<init-parameter>.{f}");
        let plain = format!("{cs}.{f}");
        let ip_rows = cx.rows(T_ALIAS, &ip);
        let hit = if !ip_rows.is_empty() {
            if ip_rows.iter().any(|r| cx.it.str(r.0[1]) != "<unannotated>") {
                Some(ip)
            } else if !cx.rows(T_ALIAS, &plain).is_empty() {
                Some(plain)
            } else {
                None
            }
        } else {
            let ip2 = format!("{cs}.<init-parameter>.{fl}");
            let ip2_rows = if fl != f { cx.rows(T_ALIAS, &ip2) } else { &[] };
            if fl != f && !ip2_rows.is_empty() {
                if ip2_rows.iter().any(|r| cx.it.str(r.0[1]) != "<unannotated>") {
                    Some(ip2)
                } else if !cx.rows(T_ALIAS, &plain).is_empty() {
                    Some(plain)
                } else {
                    None
                }
            } else if !cx.rows(T_ALIAS, &plain).is_empty() {
                Some(plain)
            } else {
                None
            }
        };
        let Some(hit) = hit else { continue };
        let pos = (d, seq);
        match &best {
            Some(b) if *b < pos => {}
            Some(b) if *b == pos => hits.push(hit),
            _ => {
                best = Some(pos);
                hits.clear();
                hits.push(hit);
            }
        }
    }
    let cls_s = s(cx, cls);
    let par = parent(&cls_s).map(str::to_owned);
    let mut out: Vec<(Sym, i32, i32)> = Vec::new();
    for h in hits {
        for t in targets(cx.rows(T_ALIAS, &h)) {
            let ts = s(cx, t);
            if let Some(x) = ts.strip_prefix("<annotation>.") {
                let Some(par) = &par else { continue };
                let t0 = type_key_at(cx, &format!("{par}.<imported>.{x}"));
                if let Some((c, k, 1)) = t0 {
                    out.push((c, 0, 14 - k));
                }
                if let Some((c, k1, 1)) = type_key_at(cx, &format!("{par}.<class>.{x}")) {
                    let lo = match t0 {
                        Some((_, k0, _)) => 15 - k0,
                        None => 0,
                    };
                    out.push((c, lo, 14 - k1));
                }
            } else if let V::SymK(v) = &*cx.get(q(K_CLASSOF, t, 0)) {
                for &(c, k) in v {
                    out.push((c, 0, 14 - k));
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    V::Field(out)
}

/// `compose_candidate` + `compose_first` for `<self>` (class `a`, attribute `b`).
fn compose(cx: &mut Ctx, a: Sym, b: Sym) -> V {
    let children = syms(&cx.get(q(K_SUBCLASS, a, 0)));
    let bs = s(cx, b);
    // (child, ord, dist, seq, cls)
    let mut cands: Vec<(Sym, Sym, u8, Box<str>, Sym)> = Vec::new();
    for child in children {
        for (ord, base) in bases_of(cx, child) {
            if base == a {
                continue;
            }
            for (c, d, seq) in order_of(cx, base) {
                let key = format!("{}.{bs}", cx.it.str(c));
                if has_method(cx, &key) {
                    cands.push((child, ord, d, seq, c));
                }
            }
        }
    }
    // the least (child, ord, dist, seq), compared as text where the SQL compares text
    let mut best: Vec<(Sym, Sym, u8, Box<str>, Sym)> = Vec::new();
    for c in cands {
        let ord = match best.first() {
            None => Ordering::Less,
            Some(x) => cmp_s(cx, c.0, x.0).then_with(|| cmp_s(cx, c.1, x.1)).then(c.2.cmp(&x.2)).then_with(|| c.3.cmp(&x.3)),
        };
        match ord {
            Ordering::Less => {
                best.clear();
                best.push(c);
            }
            Ordering::Equal => best.push(c),
            Ordering::Greater => {}
        }
    }
    let mut out: Vec<Sym> = best.into_iter().map(|x| x.4).collect();
    out.sort_unstable();
    out.dedup();
    V::Syms(out)
}

/// Refusals of one `supers` class key: a base target with several first-rank classes, bases
/// from several files, a hierarchy deeper than 64.
fn supers_refusal(cx: &mut Ctx, c: Sym) -> V {
    let rows = cx.rows_sym(T_SUPERS, c);
    if rows.is_empty() {
        return V::Bool(false);
    }
    let mut paths: Vec<u32> = rows.iter().map(|r| r.0[0]).collect();
    paths.sort_unstable();
    paths.dedup();
    if paths.len() > 1 {
        return V::Bool(true);
    }
    let ts: Vec<Sym> = rows.iter().map(|r| r.0[1]).collect();
    for t in ts {
        if cx.it.str(t).contains(".<base>.") {
            if let Some((_, several)) = base_class_of(cx, t) {
                if several > 1 {
                    return V::Bool(true);
                }
            }
        }
    }
    match &*cx.get(q(K_ORDER, c, 0)) {
        V::Order(_, deep) => V::Bool(*deep),
        _ => unreachable!(),
    }
}

/// own_name refusal of one key with several def names: used as a class (supers or a base).
fn own_refusal(cx: &mut Ctx, key: Sym) -> V {
    let rows = cx.rows_sym(T_BYNAME, key);
    if own_name(cx, rows).map_or(true, |o| o.1 <= 1) {
        return V::Bool(false);
    }
    V::Bool(!cx.rows_sym(T_SUPERS, key).is_empty() || !base_targets_of(cx, key).is_empty())
}

fn push_term(cx: &mut Ctx, b: &mut Body, d: u32, lo: i32, hi: i32) {
    if lo < 0 && hi >= 0 {
        cx.anomalies.negative_lo += 1;
    }
    b.terms.push((d, lo, hi));
}

fn push_kid(cx: &mut Ctx, b: &mut Body, kid: &str, delta: u8, chk: bool, lo: i32, hi: i32) {
    if lo < 0 && hi >= 0 {
        cx.anomalies.negative_lo += 1;
    }
    let k = intern(cx, kid);
    b.kids.push((k, delta, chk, lo, hi));
}

fn method_terms(cx: &mut Ctx, b: &mut Body, classes: &[Sym], attr: &str, lo: i32, hi: i32) {
    for &c in classes {
        let key = format!("{}.{attr}", cx.it.str(c));
        for d in method_defs(cx, &key) {
            push_term(cx, b, d, lo, hi);
        }
    }
}

fn byname_terms(cx: &mut Ctx, b: &mut Body, key: &str) {
    let rows = cx.rows(T_BYNAME, key);
    for r in rows {
        let d = cx.defs.id(r.0[0], r.0[3], r.0[1]);
        push_term(cx, b, d, 0, 99);
    }
}

/// `link_form`, `link_plain`, `prefix_*`, `field_*`, `type_demand`, `method_*`, `compose_*`,
/// `link_term`, `link_kid` for one name: its direct definitions and the names it unions.
fn body(cx: &mut Ctx, n: Sym) -> V {
    let name = s(cx, n);
    let find = |t: &str| name.find(t);
    let (pt, pf, pb, pu, ps, pp) = (find(".<type>."), find(".<field>."), find(".<bare>."), find(".<unqualified>."), find(".<self>."), find(".<super>."));
    let has_after = |p: Option<usize>, len: usize, pat: &str| p.is_some_and(|p| name[p + len..].contains(pat));
    let form = if has_after(pt, 8, ".") {
        1
    } else if has_after(pf, 9, ".") {
        2
    } else if pu.is_none() && has_after(pb, 8, "=>") {
        3
    } else if has_after(pu, 15, "=>") {
        4
    } else if ps.is_some() {
        5
    } else if pp.is_some() {
        6
    } else if name.ends_with('^') {
        7
    } else {
        8
    };
    let mut b = Body::default();
    let split = |at: usize, len: usize, sep: &str| -> (String, String, String) {
        let rest = &name[at + len..];
        let i = rest.find(sep).unwrap();
        (name[..at].to_owned(), rest[..i].to_owned(), rest[i + sep.len()..].to_owned())
    };
    match form {
        1 => {
            let (a, bb, c) = split(pt.unwrap(), 8, ".");
            let t0 = type_key_at(cx, &format!("{a}.<imported>.{bb}"));
            let mut demand: Vec<(Sym, i32, i32)> = Vec::new();
            if let Some((cl, k, 1)) = t0 {
                demand.push((cl, 0, 15 - k));
            }
            if let Some((cl, k1, 1)) = type_key_at(cx, &format!("{a}.<class>.{bb}")) {
                let lo = t0.map_or(0, |(_, k0, _)| 16 - k0);
                demand.push((cl, lo, 15 - k1));
            }
            let attr = intern(cx, &c);
            for (cl, lo, hi) in demand {
                let f = syms(&cx.get(q(K_MF0, cl, attr)));
                method_terms(cx, &mut b, &f, &c, lo, hi);
            }
        }
        2 => {
            let (a, bb, c) = split(pf.unwrap(), 9, ".");
            let (ai, bi) = (intern(cx, &a), intern(cx, &bb));
            b.root_refusal = !matches!(*cx.get(q(K_CLASSROOT, ai, 0)), V::Bool(true));
            if let V::Field(v) = &*cx.get(q(K_FIELD, ai, bi)) {
                for &(mc, lo, hi) in v {
                    let kid = format!("{}.{c}", cx.it.str(mc));
                    push_kid(cx, &mut b, &kid, 3, true, lo, hi);
                }
            }
        }
        3 => {
            let (a, bb, c) = split(pb.unwrap(), 8, "=>");
            let decl = targets(cx.rows(T_ALIAS, &format!("{a}.<declared>.{bb}")));
            let imp = if decl.is_empty() { targets(cx.rows(T_ALIAS, &format!("{a}.<imported>.{bb}"))) } else { Vec::new() };
            if !decl.is_empty() || !imp.is_empty() {
                for t in decl.into_iter().chain(imp) {
                    let ts = s(cx, t);
                    push_kid(cx, &mut b, &ts, 2, true, 0, 99);
                }
            } else if !c.is_empty() {
                push_kid(cx, &mut b, &c, 1, false, 0, 99);
            }
        }
        4 => {
            let (a, bb, c) = split(pu.unwrap(), 15, "=>");
            let own = format!("{a}.{bb}");
            if has_method(cx, &own) {
                byname_terms(cx, &mut b, &own);
                for t in targets(cx.rows(T_ALIAS, &own)) {
                    let ts = s(cx, t);
                    push_kid(cx, &mut b, &ts, 2, true, 0, 99);
                }
            } else if !c.is_empty() {
                push_kid(cx, &mut b, &c, 1, false, 0, 99);
            }
        }
        5 | 6 => {
            let (at, len) = if form == 5 { (ps.unwrap(), 8) } else { (pp.unwrap(), 9) };
            let a = name[..at].to_owned();
            let bb = name[at + len..].to_owned();
            let (ai, bi) = (intern(cx, &a), intern(cx, &bb));
            b.root_refusal = !matches!(*cx.get(q(K_CLASSROOT, ai, 0)), V::Bool(true));
            let f = syms(&cx.get(q(if form == 5 { K_MF0 } else { K_MF1 }, ai, bi)));
            if !f.is_empty() {
                method_terms(cx, &mut b, &f, &bb, 0, 99);
            } else if form == 5 {
                let c = syms(&cx.get(q(K_COMPOSE, ai, bi)));
                method_terms(cx, &mut b, &c, &bb, 0, 99);
            }
        }
        7 => {
            let a = name.trim_end_matches('^').trim_end_matches('.').to_owned();
            push_kid(cx, &mut b, &a, 1, false, 0, 99);
        }
        _ => {
            let a = parent(&name).map(str::to_owned);
            let bb = last(&name).to_owned();
            if !cx.rows(T_BYNAME, &name).is_empty() {
                byname_terms(cx, &mut b, &name);
            } else if let Some(imp) = a.as_ref().map(|a| targets(cx.rows(T_ALIAS, &format!("{a}.<imported>.{bb}")))).filter(|t| !t.is_empty()) {
                for t in imp {
                    let ts = s(cx, t);
                    push_kid(cx, &mut b, &ts, 2, true, 0, 99);
                }
            } else if !cx.rows(T_ALIAS, &name).is_empty() || !cx.rows(T_OPAQUE, &name).is_empty() {
                for t in targets(cx.rows(T_ALIAS, &name)) {
                    let ts = s(cx, t);
                    push_kid(cx, &mut b, &ts, 1, true, 0, 99);
                }
            } else {
                prefix_scan(cx, &mut b, &name);
            }
        }
    }
    b.terms.sort_unstable();
    b.terms.dedup();
    b.kids.sort_unstable();
    b.kids.dedup();
    V::Body(Box::new(b))
}

/// Step 11: `plain_prefix` + `prefix_match` + `prefix_first`, then its kids and terms.
fn prefix_scan(cx: &mut Ctx, b: &mut Body, name: &str) {
    let mut p = parent(name);
    while let Some(pre) = p {
        if pre.len() + 1 < name.len() {
            let kind = if prefix_alias(cx, pre) {
                1
            } else if !cx.rows(T_BYNAME, pre).is_empty() {
                2
            } else if !cx.rows(T_OPAQUE, pre).is_empty() {
                3
            } else if !cx.rows(T_MODULES, pre).is_empty() {
                4
            } else {
                0
            };
            if kind != 0 {
                let rest = &name[pre.len()..];
                match kind {
                    1 => {
                        for t in targets(cx.rows(T_ALIAS, pre)) {
                            let ts = s(cx, t);
                            if ts != pre && (cx.rows(T_MODULES, &ts).is_empty() || !cx.rows(T_BYNAME, &ts).is_empty()) {
                                push_kid(cx, b, &format!("{ts}{rest}"), 1, true, 0, 99);
                            }
                        }
                    }
                    2 if class_key(cx.rows(T_BYNAME, pre)) => {
                        let r2 = &rest[1..];
                        let pi = intern(cx, pre);
                        if let Some(dot) = r2.find('.') {
                            let (field, member) = (&r2[..dot], &r2[dot + 1..]);
                            let fi = intern(cx, field);
                            if let V::Field(v) = &*cx.get(q(K_FIELD, pi, fi)) {
                                for &(mc, lo, hi) in v {
                                    let kid = format!("{}.{member}", cx.it.str(mc));
                                    push_kid(cx, b, &kid, 3, true, lo, hi);
                                }
                            }
                        } else {
                            let ai = intern(cx, r2);
                            let f = syms(&cx.get(q(K_MF0, pi, ai)));
                            method_terms(cx, b, &f, r2, 0, 99);
                        }
                    }
                    _ => {}
                }
                return;
            }
        }
        p = parent(pre);
    }
}

/// `link_result` at hops `h`: the name's terms holding at `h` and its kids' links at `h + delta`
/// (a checked kid needs `h + delta ≤ 16`).
fn link(cx: &mut Ctx, n: Sym, h: u32) -> V {
    let body = cx.get(q(K_BODY, n, 0));
    let V::Body(body) = &*body else { unreachable!() };
    let hi32 = h as i32;
    let mut out: Vec<u32> = body.terms.iter().filter(|t| t.1 <= hi32 && hi32 <= t.2).map(|t| t.0).collect();
    for &(kid, delta, chk, lo, hi) in &body.kids {
        if !(lo <= hi32 && hi32 <= hi) {
            continue;
        }
        let h2 = h + delta as u32;
        if (chk && h2 > 16) || h2 > HMAX {
            continue;
        }
        if let V::Link(v) = &*cx.get(q(K_LINK, kid, h2)) {
            out.extend_from_slice(v);
        }
    }
    out.sort_unstable();
    out.dedup();
    if h >= 19 && !out.is_empty() {
        cx.anomalies.deep_nonempty += 1;
    }
    V::Link(out.into_boxed_slice())
}
