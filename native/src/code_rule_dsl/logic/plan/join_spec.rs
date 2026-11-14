//! A planned rule as a native join spec (sqlite-ext `setfix.rs`, "native join probes", T's `join_spec`), read by
//! handles (decision 0173, t36 item 4): the rule's `Body` and its steps, in plan order, each step a `JoinItem` over
//! numbered slots, then the head. A literal with no native step (an aggregate other than `count`, `sum`, `min`, `max`
//! and `join`, a descent, a document range, top) has no spec: that rule stays SQL.
use super::body::{Body, VarSet};
use super::{Mode, Run, Step, prog_types, sym_char};
use super::super::native::component::spec::{Arg, Extreme, HeadTok, InputKey, JoinItem, JoinSpec, Source as JoinSource, Tok};
use super::super::program_image::{AggRef, CmpOp, Lit, Slot, TermRef};
use super::super::strings;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

/// What a spec's slot holds: a rule's variable, or a value the lowering makes (a computed term's, `rr_fn<step>_<k>`; a
/// head `@cat`'s, `rr_head_cat<k>`).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Var(Slot),
    Fn(usize, usize),
    HeadCat(usize),
}

/// The slots, numbered as they are first met.
#[derive(Default)]
struct Slots(HashMap<Key, usize>);

impl Slots {
    fn has(&self, k: Key) -> bool {
        self.0.contains_key(&k)
    }
    fn bound(&self, t: TermRef<'_>) -> bool {
        matches!(t, TermRef::Var(v) if self.has(Key::Var(v)))
    }
    fn slot(&mut self, k: Key) -> usize {
        let len = self.0.len();
        *self.0.entry(k).or_insert(len)
    }
}

/// A value a step reads: a term of the rule, a made slot, or a made constant string (a set's patterns joined).
#[derive(Clone)]
enum Val<'v> {
    T(TermRef<'v>),
    Made(Key),
    Str(String),
}

impl Val<'_> {
    /// Whether the value is known when the step runs: a bound variable, a constant or a made value.
    fn known(&self, slots: &Slots) -> bool {
        match self {
            Val::T(TermRef::Var(v)) => slots.has(Key::Var(*v)),
            Val::T(TermRef::Int(_) | TermRef::Str(_)) | Val::Made(_) | Val::Str(_) => true,
            Val::T(_) => false,
        }
    }
}

/// A constant term's token.
fn const_tok(b: &Body<'_>, t: TermRef<'_>) -> Option<Tok> {
    match t {
        TermRef::Int(i) => Some(Tok::Int(i)),
        TermRef::Str(s) => Some(Tok::Sym(b.v.text(s).to_owned())),
        _ => None,
    }
}

/// A value's token: `_`, a slot (numbered as first met) or a constant.
fn tok(b: &Body<'_>, v: &Val<'_>, slots: &mut Slots) -> Option<Tok> {
    match v {
        Val::T(TermRef::Wild) => Some(Tok::Wild),
        Val::T(TermRef::Var(s)) => Some(Tok::Slot(slots.slot(Key::Var(*s)))),
        Val::T(t) => const_tok(b, *t),
        Val::Made(k) => Some(Tok::Slot(slots.slot(*k))),
        Val::Str(s) => Some(Tok::Sym(s.clone())),
    }
}

fn t_tok(b: &Body<'_>, t: TermRef<'_>, slots: &mut Slots) -> Option<Tok> {
    tok(b, &Val::T(t), slots)
}

fn var_tok(s: Slot, slots: &mut Slots) -> Tok {
    Tok::Slot(slots.slot(Key::Var(s)))
}

fn is_const(t: TermRef<'_>) -> bool {
    matches!(t, TermRef::Int(_) | TermRef::Str(_))
}

/// The native function of call `f` with `n` arguments (`@concat` is `str_concat`).
fn call_name(f: &str, n: usize) -> Option<&'static str> {
    if f == "concat" {
        return (n == 2).then_some("str_concat");
    }
    let (name, kinds, _) = strings::functions::native_call(f)?;
    (kinds.len() == n).then_some(name)
}

/// Whether a computed term's value is a number (a function's result kind), not a symbol.
fn numeric_call(b: &Body<'_>, t: TermRef<'_>) -> bool {
    matches!(t, TermRef::Call(f, _) if b.v.text(f) != "concat"
        && strings::functions::native_call(b.v.text(f)).is_some_and(|(_, _, k)| k == strings::functions::Kind::Number))
}

/// A test's patterns: one string, or a set `{a, b}` of strings (`strings::patterns`).
fn patterns(b: &Body<'_>, t: TermRef<'_>) -> Option<Vec<String>> {
    match t {
        TermRef::Str(s) => Some(vec![b.v.text(s).to_owned()]),
        TermRef::Call(f, args) if b.v.text(f) == strings::SET => {
            args.map(|a| if let TermRef::Str(s) = a { Some(b.v.text(s).to_owned()) } else { None }).collect()
        }
        _ => None,
    }
}

/// A computed term (`@concat`, the text and number functions `strings::functions::native_call` names) of bound
/// arguments, nested on either side, as `func` steps, inner ones first, each into a slot of its own (`rr_fn<step>_<k>`):
/// the value of it. A bound variable, a made value or a constant is itself; anything else (an unbound variable, a call
/// with no native function) has none.
fn expr<'v>(b: &Body<'v>, v: &Val<'v>, idx: usize, slots: &mut Slots, lines: &mut Vec<JoinItem>) -> Option<Val<'v>> {
    let t = match v {
        Val::T(t) => *t,
        made => return Some(made.clone()),
    };
    match t {
        TermRef::Var(s) => slots.has(Key::Var(s)).then(|| v.clone()),
        TermRef::Int(_) | TermRef::Str(_) => Some(v.clone()),
        TermRef::Call(f, args) => {
            let f = b.v.text(f);
            let args: Vec<TermRef<'v>> = args.collect();
            // `@cat(a, b)` (what `@dot` lowers to) inside another term: the head's `Cat`, made into a slot first
            // (`sym(f, @concat(@cat(q, x), …))`).
            if f == "cat" && args.len() == 2 {
                let made: Vec<Val<'v>> = args.iter().map(|&a| expr(b, &Val::T(a), idx, slots, lines)).collect::<Option<_>>()?;
                let (x, y) = (tok(b, &made[0], slots)?, tok(b, &made[1], slots)?);
                let out = Val::Made(Key::Fn(idx, lines.len()));
                lines.push(JoinItem::Cat { out: tok(b, &out, slots)?, a: x, b: y });
                return Some(out);
            }
            let name = call_name(f, args.len())?;
            let made: Vec<Val<'v>> = args.iter().map(|&a| expr(b, &Val::T(a), idx, slots, lines)).collect::<Option<_>>()?;
            let args: Vec<Tok> = made.iter().map(|a| tok(b, a, slots)).collect::<Option<_>>()?;
            let out = Val::Made(Key::Fn(idx, lines.len()));
            lines.push(JoinItem::Func { name: name.to_owned(), out: tok(b, &out, slots)?, args });
            Some(out)
        }
        _ => None,
    }
}

impl<'a> Run<'a, '_> {
    /// `join_spec` for a whole rule: `None` too when a head variable is bound by none of its steps (the native join
    /// would read past its slots). A prefix of a body (the probe for how far a body runs natively) is not checked.
    pub(super) fn whole_join_spec(
        &self,
        b: &Body<'_>,
        steps: &[Step],
        at: Option<usize>,
        ri: &BTreeMap<String, usize>,
        scc: &BTreeSet<String>,
        native_rels: &BTreeSet<String>,
    ) -> Option<Arc<JoinSpec>> {
        let mut bound = VarSet::new();
        for s in steps {
            let mut add = |t: TermRef<'_>| {
                if let TermRef::Var(v) = t {
                    bound.insert(v);
                }
            };
            match b.view(s.at) {
                Lit::Atom(l) if !l.neg() => l.args().for_each(&mut add),
                // `$s = @cat($m, $x)` with s bound splits s: m and x are bound too.
                Lit::Cmp { l, op: CmpOp::Eq, r: TermRef::Call(f, args) } if b.v.text(f) == "cat" => {
                    add(l);
                    args.for_each(&mut add);
                }
                Lit::Cmp { l, op: CmpOp::Eq, r } => {
                    add(l);
                    add(r);
                }
                Lit::Nearest { n, a, .. } => {
                    bound.insert(n);
                    bound.insert(a);
                }
                Lit::Agg { var, .. } => {
                    bound.insert(var);
                }
                _ => {}
            }
        }
        let mut head_vars = Vec::new();
        for t in b.head().args() {
            match t {
                TermRef::Call(_, args) => head_vars.extend(args),
                t => head_vars.push(t),
            }
        }
        if head_vars.into_iter().filter_map(TermRef::var).any(|v| !bound.contains(v)) {
            return None;
        }
        self.join_spec(b, steps, false, at, ri, scc, native_rels)
    }

    /// A rule's native join spec (`join_spec_made`), kept for the run by a digest of every input it reads
    /// (`logic::derived`): each chunk's program is new, and most of its strata plan the same orders over the same
    /// sources. `bare`: the spec of the steps alone, under a head of no terms (a prefix, probed for how far it runs).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn join_spec(
        &self,
        b: &Body<'_>,
        steps: &[Step],
        bare: bool,
        at: Option<usize>,
        ri: &BTreeMap<String, usize>,
        scc: &BTreeSet<String>,
        native_rels: &BTreeSet<String>,
    ) -> Option<Arc<JoinSpec>> {
        let prog = self.prog;
        let inputs = |d: &mut super::super::derived::Inputs| {
            if bare {
                // The tree's head of no terms, as `digest_atom` writes it.
                d.str(b.rel(b.head()));
                d.u64(0);
                d.u64(0);
            } else {
                b.digest_atom(d, b.head());
            }
            d.u64(at.map_or(u64::MAX, |a| a as u64));
            match self.model.inputs.atoms {
                Some(ops) => ops.iter().for_each(|&o| d.u64(o as u64)),
                None => d.u64(u64::MAX),
            }
            let read = |rel: &str, d: &mut super::super::derived::Inputs| {
                d.str(rel);
                if let Some(i) = ri.get(rel).filter(|_| scc.contains(rel)) {
                    d.u64(0);
                    d.u64(*i as u64);
                } else if native_rels.contains(rel) {
                    d.u64(1);
                } else {
                    d.u64(2);
                    d.str(&self.model.relation(prog, rel, false).render());
                    d.u64(u64::from(self.inputs.contains(rel)) | u64::from(self.model.derived.contains(rel)) << 1 | u64::from(self.projections.contains_key(rel)) << 2);
                    match self.model.view_src.get(rel) {
                        Some((base, cols)) => {
                            d.str(base);
                            d.u64(cols.len() as u64);
                            for c in cols {
                                d.str(c.as_deref().unwrap_or("\u{0}"));
                            }
                        }
                        None => d.u64(u64::MAX),
                    }
                }
                let decl = prog.decls.get(rel).map_or(&[][..], |c| c.as_slice());
                d.u64(decl.len() as u64);
                for (_, t) in decl {
                    d.str(t);
                }
            };
            for s in steps {
                b.digest_lit(d, b.lit(s.at));
                d.u64(match s.mode {
                    None => 0,
                    Some(Mode::Range) => 1,
                    Some(Mode::Climb) => 2,
                    Some(Mode::Descend) => 3,
                });
                match b.view(s.at) {
                    Lit::Atom(l) | Lit::Agg { atom: l, .. } => read(b.rel(l), d),
                    Lit::Nearest { kindset, .. } => {
                        let kinds = prog.kindsets.get(b.v.text(kindset)).map_or(&[][..], Vec::as_slice);
                        d.u64(kinds.len() as u64);
                        kinds.iter().for_each(|k| d.str(k));
                    }
                    _ => {}
                }
            }
        };
        super::super::derived::value("join spec", inputs, || self.join_spec_made(b, steps, bare, at, ri, scc, native_rels).map(Arc::new))
    }

    #[allow(clippy::too_many_arguments)]
    fn join_spec_made(
        &self,
        b: &Body<'_>,
        steps: &[Step],
        bare: bool,
        at: Option<usize>,
        ri: &BTreeMap<String, usize>,
        scc: &BTreeSet<String>,
        native_rels: &BTreeSet<String>,
    ) -> Option<JoinSpec> {
        let prog = self.prog;
        let atoms = self.model.inputs.atoms;
        let mut slots = Slots::default();
        // A variable's type: its column's in a positive atom; a count's, a sum's or a number function's value is a
        // number. A variable equal to another (`$m = $a`, a capture's alias) has the other's type.
        let typ = |t: TermRef<'_>| -> Option<String> {
            match t {
                TermRef::Int(_) => Some("int".to_owned()),
                TermRef::Str(_) => Some("sym".to_owned()),
                TermRef::Var(n) => {
                    let mut same: Vec<Slot> = vec![n];
                    let mut i = 0;
                    while i < same.len() {
                        for s in steps {
                            if let Lit::Cmp { l: TermRef::Var(x), op: CmpOp::Eq, r: TermRef::Var(y) } = b.view(s.at) {
                                for (x, y) in [(x, y), (y, x)] {
                                    if x == same[i] && !same.contains(&y) {
                                        same.push(y);
                                    }
                                }
                            }
                        }
                        i += 1;
                    }
                    same.iter().find_map(|&n| {
                        steps.iter().find_map(|s| match b.view(s.at) {
                            Lit::Atom(l) if !l.neg() => b
                                .args(s.at)
                                .iter()
                                .position(|x| x.var() == Some(n))
                                .and_then(|c| Some(prog.decls.get(b.rel(l))?.get(c)?.1.to_string())),
                            Lit::Agg { var, op: AggRef::Count | AggRef::Sum | AggRef::Product, .. } if var == n => Some("int".to_owned()),
                            Lit::Cmp { l: TermRef::Var(v), op: CmpOp::Eq, r: r @ TermRef::Call(..) } if v == n && numeric_call(b, r) => Some("int".to_owned()),
                            // A variable equal to a constant (`0 = $r`, an inlined `fneed(f, 0)`) has the constant's type.
                            Lit::Cmp { l: TermRef::Var(v), op: CmpOp::Eq, r: TermRef::Int(_) } | Lit::Cmp { l: TermRef::Int(_), op: CmpOp::Eq, r: TermRef::Var(v) }
                                if v == n =>
                            {
                                Some("int".to_owned())
                            }
                            Lit::Cmp { l: TermRef::Var(v), op: CmpOp::Eq, r: TermRef::Str(_) } | Lit::Cmp { l: TermRef::Str(_), op: CmpOp::Eq, r: TermRef::Var(v) }
                                if v == n =>
                            {
                                Some("sym".to_owned())
                            }
                            _ => None,
                        })
                    })
                }
                _ => None,
            }
        };
        let arg_toks = |args: &[TermRef<'_>], slots: &mut Slots| args.iter().map(|&t| t_tok(b, t, slots).map(Arg::Tok)).collect::<Option<Vec<_>>>();
        // The atom's source and tokens. An input relation is loaded with its constant columns as a filter and
        // projected away (`field($n, "path", $p)` loads the `path` edges only), so it is read in the size it is used.
        let atom = |rel: &str, args: &[TermRef<'_>], idx: Option<usize>, slots: &mut Slots| -> Option<(JoinSource, Vec<Arg>)> {
            if scc.contains(rel) {
                let src = if idx.is_some() && idx == at { JoinSource::Delta(ri[rel]) } else { JoinSource::Full(ri[rel]) };
                return Some((src, arg_toks(args, slots)?));
            }
            if native_rels.contains(rel) {
                return Some((JoinSource::Native(rel.to_string()), arg_toks(args, slots)?));
            }
            let keep: Vec<usize> = (0..args.len()).filter(|&c| !is_const(args[c])).collect();
            // A view that only projects an input (`kind($n, $k) :- node(_, $n, $k, _, _)`) is read from the input,
            // in place: its columns are the input's.
            let projected = self.model.view_src.get(rel).filter(|(_, vc)| vc.iter().all(Option::is_some)).filter(|_| self.projections.contains_key(rel));
            let sql_col = |c: usize| match projected {
                Some((_, vc)) => vc[c].clone().unwrap(),
                None => self.model.col(rel, c),
            };
            let cols = keep.iter().map(|&c| sql_col(c)).collect::<Vec<_>>().join(", ");
            let cols = if cols.is_empty() { "1".to_owned() } else { cols };
            let mut wheres = Vec::new();
            // The view's own filters, on its input's columns.
            if let Some((base, _)) = projected {
                for &(c, v) in &self.projections[rel] {
                    wheres.push(format!("{} = {v}", self.model.col(base, c)));
                }
            }
            for (c, &t) in args.iter().enumerate() {
                let value = match t {
                    TermRef::Int(i) => super::integer(i),
                    TermRef::Str(s) => super::string(b.v.text(s)),
                    _ => continue,
                };
                wheres.push(format!("{} = {}", sql_col(c), value.render()));
            }
            // A relation this program part does not declare (an input view of the whole) is not joined natively: SQL
            // reads it.
            let decl = prog.decls.get(rel)?;
            let types: String = keep
                .iter()
                .map(|&c| sym_char(&self.model.inputs, &decl.get(c).unwrap_or_else(|| panic!("{rel} is declared with {} columns and read with {}", decl.len(), args.len())).1))
                .collect();
            let types = if types.is_empty() { "i".to_owned() } else { types };
            let mut sel = self.model.relation(prog, projected.map_or(rel, |(base, _)| base), false).render();
            if !wheres.is_empty() {
                sel += &format!(" WHERE {}", wheres.join(" AND "));
            }
            let mut toks: Vec<Arg> = keep.iter().map(|&c| t_tok(b, args[c], slots).map(Arg::Tok)).collect::<Option<_>>()?;
            if toks.is_empty() {
                toks.push(Arg::Tok(Tok::Wild));
            }
            let base = self.inputs.contains(rel) || self.model.view_src.contains_key(rel);
            Some((JoinSource::Input(InputKey { base, sql: sel, cols, types }), toks))
        };
        // An input is read without its constant columns (`atom`): a column's place among those read.
        let read_col = |rel: &str, args: &[TermRef<'_>], c: usize| {
            if scc.contains(rel) || native_rels.contains(rel) { c } else { c - args[..c].iter().filter(|&&t| is_const(t)).count() }
        };
        let kinds = |kset| prog.kindsets[b.v.text(kset)].join(",");
        let call_of = |t: TermRef<'_>, f: &str| matches!(t, TermRef::Call(g, _) if b.v.text(g) == f);
        let mut lines = Vec::new();
        for (idx, s) in steps.iter().enumerate() {
            match b.view(s.at) {
                Lit::Atom(l) => {
                    if s.mode == Some(Mode::Range) {
                        return None;
                    }
                    let (src, args) = atom(b.rel(l), b.args(s.at), Some(idx), &mut slots)?;
                    lines.push(JoinItem::Atom { neg: l.neg(), src, args });
                }
                Lit::Cmp { l: TermRef::Call(fl, xl), op: op @ (CmpOp::Eq | CmpOp::Ne), r: TermRef::Call(fr, xr) }
                    if b.v.text(fl) == "doc" && b.v.text(fr) == "doc" && xl.clone().count() == 1 && xr.clone().count() == 1 =>
                {
                    // `@doc($a) = @doc($b)` (cross_rounds' hops): both nodes bound.
                    let (x, y) = (xl.clone().next().expect("one"), xr.clone().next().expect("one"));
                    if !slots.bound(x) || !slots.bound(y) {
                        return None;
                    }
                    let (a, c) = (t_tok(b, x, &mut slots)?, t_tok(b, y, &mut slots)?);
                    lines.push(JoinItem::Doc { a, b: c, eq: op == CmpOp::Eq });
                }
                Lit::Cmp { l: TermRef::Var(d), op: CmpOp::Eq, r: TermRef::Call(f, args) } if b.v.text(f) == "doc" && args.clone().count() == 1 => {
                    // `$d = @doc($n)` (the cross-file memo's guard, decision 0076): n bound; d bound or tested.
                    let x = args.clone().next().expect("one");
                    if !slots.bound(x) {
                        return None;
                    }
                    let n = t_tok(b, x, &mut slots)?;
                    lines.push(JoinItem::DocOf { out: var_tok(d, &mut slots), n });
                }
                Lit::Cmp { l: TermRef::Call(f, args), op: CmpOp::Eq, r: TermRef::Int(want @ (0 | 1)) }
                    if let f = b.v.text(f)
                        && (f == "in" || f == "equals" || strings::is_test(f))
                        && let [x, set @ TermRef::Call(_, members)] = args.collect::<Vec<_>>()[..]
                        && call_of(set, strings::SET)
                        && members.clone().all(|m| matches!(m, TermRef::Int(_)))
                        && slots.bound(x) =>
                {
                    let members: Vec<Tok> = members.map(|m| t_tok(b, m, &mut slots)).collect::<Option<_>>()?;
                    let a = t_tok(b, x, &mut slots)?;
                    lines.push(if f == "in" || f == "equals" {
                        // `@equals($q, {"a", "b"}) = 1` once lowered (the strings are symbols): membership, natively.
                        JoinItem::In { a, want: want == 1, set: members }
                    } else {
                        // `@contains($w, {"dolt_", "dolt log"}) = 1` once lowered (the patterns are symbols): one
                        // matcher of the symbols' names, made with the spec.
                        JoinItem::TestSet { op: f.to_string(), s: a, want: want == 1, members }
                    });
                }
                Lit::Cmp { l: TermRef::Call(f, args), op: CmpOp::Eq, r: TermRef::Int(want @ (0 | 1)) } if strings::is_test(b.v.text(f)) && args.clone().count() == 2 => {
                    // `@prefix($s, p) = 1` (or `= 0`): a text test of a string no text relation gives (`strings::as_test`
                    // takes the others to the vocabulary), its string and pattern bound; a set's patterns as one string.
                    let args: Vec<TermRef<'_>> = args.collect();
                    let pattern = match patterns(b, args[1]) {
                        Some(ps) => Val::Str(ps.join(&strings::SET_SEP.to_string())),
                        // A computed pattern (`@suffix($s, @concat(".", $x))`): made first, into its own slot.
                        None if matches!(args[1], TermRef::Call(..)) => expr(b, &Val::T(args[1]), idx, &mut slots, &mut lines)?,
                        None => Val::T(args[1]),
                    };
                    // A constant string is a symbol (`Int`) once lowered; a computed string (`@lower($s)`) is made first.
                    let tested = expr(b, &Val::T(args[0]), idx, &mut slots, &mut lines)?;
                    if !tested.known(&slots) || !pattern.known(&slots) {
                        return None;
                    }
                    let (ts, tp) = (tok(b, &tested, &mut slots)?, tok(b, &pattern, &mut slots)?);
                    lines.push(JoinItem::Test { op: b.v.text(f).to_string(), s: ts, p: tp, want: want == 1 });
                }
                Lit::Cmp { l, op: op @ (CmpOp::Eq | CmpOp::Ne), r } if !call_of(r, "cat") => {
                    // `$v = @f(…)` (a text or number function, `@concat`, nested calls): the call's value straight into
                    // `$v`, bound or tested; any other `=` or `!=`, of variables, constants or computed terms, as a test
                    // (or `$v = <bound term>`, binding `$v`). Calls on either side are made first, inner ones first.
                    let unbound = |t: TermRef<'_>, slots: &Slots| matches!(t, TermRef::Var(v) if !slots.has(Key::Var(v)));
                    let (l, r) = if matches!(r, TermRef::Var(_)) && matches!(l, TermRef::Call(..)) { (r, l) } else { (l, r) };
                    if op == CmpOp::Eq
                        && let (TermRef::Var(_), TermRef::Call(f, args)) = (l, r)
                    {
                        let args: Vec<TermRef<'_>> = args.collect();
                        let name = call_name(b.v.text(f), args.len())?;
                        let made: Vec<Val<'_>> = args.iter().map(|&a| expr(b, &Val::T(a), idx, &mut slots, &mut lines)).collect::<Option<_>>()?;
                        let args: Vec<Tok> = made.iter().map(|a| tok(b, a, &mut slots)).collect::<Option<_>>()?;
                        lines.push(JoinItem::Func { name: name.to_owned(), out: t_tok(b, l, &mut slots)?, args });
                    } else {
                        // The bound side goes right.
                        let (x, y) = if unbound(r, &slots) { (r, l) } else { (l, r) };
                        if unbound(y, &slots) || (op == CmpOp::Ne && unbound(x, &slots)) {
                            return None;
                        }
                        let y = expr(b, &Val::T(y), idx, &mut slots, &mut lines)?;
                        let x = if unbound(x, &slots) { Val::T(x) } else { expr(b, &Val::T(x), idx, &mut slots, &mut lines)? };
                        let (a, c) = (tok(b, &x, &mut slots)?, tok(b, &y, &mut slots)?);
                        lines.push(JoinItem::Eq { a, b: c, ne: op != CmpOp::Eq });
                    }
                }
                Lit::Cmp { l, op: CmpOp::Eq, r: TermRef::Call(_, mut args) } => {
                    // `$s = @cat($m, $x)` (the arm above takes every other `=`).
                    if !matches!(l, TermRef::Var(_)) {
                        return None;
                    }
                    let (x, y) = (args.next().expect("two"), args.next().expect("two"));
                    let (out, a) = (t_tok(b, l, &mut slots)?, t_tok(b, x, &mut slots)?);
                    lines.push(JoinItem::Cat { out, a, b: t_tok(b, y, &mut slots)? });
                }
                Lit::Cmp { l, op: op @ (CmpOp::Lt | CmpOp::Le | CmpOp::Gt | CmpOp::Ge), r } => {
                    // Integers (and nodes) only: symbols are numbered in the order they are met. A number function's
                    // value (`@strlen($s) > 1`, `@add($i, $j) <= 2`) is made first.
                    let bad = |t: TermRef<'_>, slots: &Slots| {
                        if matches!(t, TermRef::Call(..)) {
                            return !numeric_call(b, t);
                        }
                        matches!(t, TermRef::Var(v) if !slots.has(Key::Var(v))) || matches!(typ(t).as_deref(), None | Some("sym"))
                    };
                    if bad(l, &slots) || bad(r, &slots) {
                        return None;
                    }
                    let (l, r) = (expr(b, &Val::T(l), idx, &mut slots, &mut lines)?, expr(b, &Val::T(r), idx, &mut slots, &mut lines)?);
                    let (x, y) = if matches!(op, CmpOp::Lt | CmpOp::Le) { (&l, &r) } else { (&r, &l) };
                    let a = tok(b, x, &mut slots)?;
                    lines.push(JoinItem::Lt { a, b: tok(b, y, &mut slots)?, or_eq: !matches!(op, CmpOp::Lt | CmpOp::Gt) });
                }
                Lit::Agg { var, op: op @ (AggRef::Count | AggRef::Sum | AggRef::Min | AggRef::Max), over, atom: a } => {
                    let rel = b.rel(a);
                    let mut args: Vec<TermRef<'_>> = b.args(s.at).to_vec();
                    // The over-variable belongs to the bag even if an outer step bound the same name.
                    let local: Vec<usize> = args.iter().enumerate().filter_map(|(c, t)| (over.is_some() && t.var() == over).then_some(c)).collect();
                    let count = matches!(op, AggRef::Count);
                    if atoms.is_none() && (!count || local.len() > 1) {
                        return None;
                    }
                    let col = over.and_then(|o| args.iter().position(|t| t.var() == Some(o)));
                    if !count && col.is_none() {
                        return None;
                    }
                    if let Some(c) = col {
                        let typ = prog_types(prog, rel).get(c).copied();
                        if matches!(op, AggRef::Sum) && typ == Some("sym") {
                            return None;
                        }
                        // Text extrema compare text unless this program holds symbols as atoms on both paths.
                        if !count && atoms.is_none() && typ == Some("sym") {
                            return None;
                        }
                    }
                    for t in &mut args {
                        if over.is_some() && t.var() == over {
                            *t = TermRef::Wild;
                        }
                    }
                    if args.iter().any(|&t| matches!(t, TermRef::Var(_)) && !slots.bound(t)) {
                        return None;
                    }
                    // Input constant filters are projected out by `atom`; its value column uses that projection too.
                    let value_col = col.map(|c| read_col(rel, &args, c));
                    let (src, mut toks) = atom(rel, &args, None, &mut slots)?;
                    if local.len() > 1 {
                        for c in local {
                            toks[read_col(rel, &args, c)] = Arg::Local("0".into());
                        }
                    }
                    let out = var_tok(var, &mut slots);
                    lines.push(match op {
                        AggRef::Count => JoinItem::Count { src, args: toks, out },
                        _ => JoinItem::Extreme {
                            op: match op {
                                AggRef::Sum => Extreme::Sum,
                                AggRef::Min => Extreme::Min,
                                _ => Extreme::Max,
                            },
                            src,
                            args: toks,
                            col: value_col?,
                            out,
                        },
                    });
                }
                Lit::Agg { var, op: AggRef::Join { by, sep }, over: Some(o), atom: a } => {
                    // `$s = join $x by ($k…) sep "…" : { atom }`: x and the unbound k are the atom's own columns, one
                    // each; every other variable is bound. A bound k is the same for every row, so it orders nothing.
                    // Symbols order by their names natively, as SQL orders their text.
                    let rel = b.rel(a);
                    let mut args: Vec<TermRef<'_>> = b.args(s.at).to_vec();
                    let column = |v: Slot| args.iter().position(|t| t.var() == Some(v));
                    let uses = |v: Slot| args.iter().filter(|t| t.var() == Some(v)).count();
                    let local: Vec<Slot> = std::iter::once(o).chain(by.filter(|v| !slots.has(Key::Var(*v)))).collect();
                    if slots.has(Key::Var(o)) || local.iter().any(|&v| uses(v) != 1) {
                        return None;
                    }
                    if args.iter().any(|t| t.var().is_some_and(|v| !slots.has(Key::Var(v)) && !local.contains(&v))) {
                        return None;
                    }
                    let sym = |c: usize| prog_types(prog, rel).get(c).copied() == Some("sym");
                    let xc = column(o)?;
                    let mut bycols = Vec::new();
                    for v in by.filter(|v| !slots.has(Key::Var(*v))) {
                        let c = column(v)?;
                        bycols.push((read_col(rel, &args, c), sym(c)));
                    }
                    let x = (read_col(rel, &args, xc), sym(xc));
                    let wild: Vec<usize> = local.iter().map(|&v| column(v)).collect::<Option<_>>()?;
                    for c in wild {
                        args[c] = TermRef::Wild;
                    }
                    let (src, toks) = atom(rel, &args, None, &mut slots)?;
                    let sep = Tok::Sym(b.v.text(sep).to_owned());
                    let out = var_tok(var, &mut slots);
                    lines.push(JoinItem::StrJoin { src, args: toks, x, by: bycols, sep, out });
                }
                Lit::Nearest { n, a, kindset } if s.mode == Some(Mode::Climb) => {
                    let tn = var_tok(n, &mut slots);
                    lines.push(JoinItem::Near { down: false, n: tn, a: var_tok(a, &mut slots), kinds: kinds(kindset) });
                }
                Lit::Nearest { n, a, kindset } if slots.has(Key::Var(a)) && !slots.has(Key::Var(n)) => {
                    let tn = var_tok(n, &mut slots);
                    lines.push(JoinItem::Near { down: true, n: tn, a: var_tok(a, &mut slots), kinds: kinds(kindset) });
                }
                _ => return None,
            }
        }
        let mut head = Vec::new();
        let head_args: Vec<TermRef<'_>> = if bare { Vec::new() } else { b.head().args().collect() };
        for (k, t) in head_args.into_iter().enumerate() {
            match t {
                TermRef::Call(_, args) if call_of(t, "cat") && args.clone().count() == 2 && args.clone().all(|x| slots.bound(x)) => {
                    let mut args = args;
                    let (x, y) = (args.next().expect("two"), args.next().expect("two"));
                    let (Some(Tok::Slot(a)), Some(Tok::Slot(c))) = (t_tok(b, x, &mut slots), t_tok(b, y, &mut slots)) else { return None };
                    head.push(HeadTok::Cat(a, c));
                }
                // `sym_at($e, @cat(1539, $x))`: a qualified name of a constant scope (the planner folds a scope's name
                // to its symbol) and a bound part, or two constants: the name made into a slot of its own first.
                TermRef::Call(_, args) if call_of(t, "cat") && args.clone().count() == 2 => {
                    if !args.clone().all(|x| is_const(x) || slots.bound(x)) {
                        return None;
                    }
                    let mut args = args;
                    let (x, y) = (args.next().expect("two"), args.next().expect("two"));
                    let (a, c) = (t_tok(b, x, &mut slots)?, t_tok(b, y, &mut slots)?);
                    let out = tok(b, &Val::Made(Key::HeadCat(k)), &mut slots)?;
                    lines.push(JoinItem::Cat { out: out.clone(), a, b: c });
                    head.push(HeadTok::Tok(out));
                }
                // `q(…, @concat(m, ".", a))`, `q(…, @add($i, 1))`: the computed term made into a slot before the head
                // reads it.
                TermRef::Call(..) => {
                    let made = expr(b, &Val::T(t), steps.len(), &mut slots, &mut lines)?;
                    head.push(HeadTok::Tok(tok(b, &made, &mut slots)?));
                }
                _ => head.push(HeadTok::Tok(t_tok(b, t, &mut slots)?)),
            }
        }
        lines.push(JoinItem::Head(head));
        Some(JoinSpec { items: lines })
    }
}
