//! A library can declare a finite request domain and the evidence needed to negate a member
//! (`.pragma "bound-family" "request complete member…"`): a rule that reads a member asks `request` for the member's
//! key, bound as the rule binds it, and a negated member also reads `complete` at that key, so its absence is a
//! witness only for a key whose family is complete. Over the module's version (decision 0276 stage 6): its rules,
//! read and rewritten through the builder.
use super::{BTreeMap, BTreeSet, Error};
use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
use crate::code_rule_dsl::logic::program_image::{Builder, CmpOp, Lit, LitRef, Rewrite, RuleRef, Slot, T, TermRef, Version, t_of};
use std::sync::Arc;

/// The bound families a program declares: each pragma's text, `request complete member…`.
pub(super) fn specs(prog: &(impl ProgramFields + ?Sized)) -> Vec<String> {
    prog.settings()
        .into_iter()
        .filter(|(key, _)| key == "pragma")
        .filter_map(|(_, text)| Some(text.strip_prefix("\"bound-family\" \"")?.strip_suffix('"')?.to_owned()))
        .collect()
}

pub(super) fn check_names(spec: &str) -> Result<(), Error> {
    if spec.split_whitespace().count() >= 3
        && spec.split_whitespace().all(|name| {
            name.starts_with(|c: char| c.is_ascii_alphabetic()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
    {
        return Ok(());
    }
    Err(Error::Syntax(
        "bound-family expects a request, a completeness relation and at least one member".into(),
    ))
}

/// A rule copied as it is (the guarded rule, before its guards).
struct Same;
impl Rewrite for Same {}

/// `v` with each bound family's readers' requests and negations' guards (`inherited`: the families its includes
/// declared; `dependencies`: each relation's reads, by head): `None` where nothing changes.
pub(super) fn lower(v: &Arc<Version>, inherited: &[String], dependencies: &BTreeMap<String, BTreeSet<String>>) -> Result<Option<Arc<Version>>, Error> {
    let mut members: BTreeMap<String, (String, String, usize)> = BTreeMap::new();
    for spec in inherited.iter().cloned().chain(specs(&**v)) {
        let names: Vec<_> = spec.split_whitespace().collect();
        let [request, complete, rest @ ..] = names.as_slice() else {
            continue;
        };
        let columns = |name: &str| v.columns_of(name).ok_or_else(|| Error::Syntax(format!("bound-family: undeclared relation {name}")));
        let key = columns(request)?;
        for name in std::iter::once(complete).chain(rest) {
            let cols = columns(name)?;
            if key.is_empty()
                || (*name == *complete && cols.len() != key.len())
                || cols.len() < key.len()
                || !key.iter().zip(cols.iter()).all(|((_, a), (_, b))| a == b)
            {
                return Err(Error::Syntax(format!("bound-family: {name} needs the key columns of {request}")));
            }
        }
        for member in rest {
            let mode = (request.to_string(), complete.to_string(), key.len());
            if members.insert(member.to_string(), mode.clone()).is_some_and(|old| old != mode) {
                return Err(Error::Syntax(format!("bound-family: conflicting declarations for {member}")));
            }
        }
    }
    if members.is_empty() {
        return Ok(None);
    }
    // A restriction derived from the answer cannot decide whether to request that answer.
    let mut depends_on_answer: BTreeSet<String> = members.keys().cloned().collect();
    for (request, complete, _) in members.values() {
        depends_on_answer.extend([request.clone(), complete.clone()]);
    }
    loop {
        let previous = depends_on_answer.len();
        for (head, reads) in dependencies {
            if !reads.is_disjoint(&depends_on_answer) {
                depends_on_answer.insert(head.clone());
            }
        }
        if previous == depends_on_answer.len() {
            break;
        }
    }
    // The rules as written, by their shape (`shown`): a request already stated is not stated again.
    let mut stated: BTreeSet<String> = v.rules().map(|r| shown_rule(v, r)).collect();
    let mut b = Builder::on(v);
    let mut changed = false;
    for r in v.rules() {
        let head = v.rel_name(r.head().rel()).to_owned();
        let body: Vec<LitRef<'_>> = r.body().collect();
        let mut guards: Vec<(String, Vec<TermRef<'_>>)> = Vec::new();
        let mut asks: Vec<(String, Vec<TermRef<'_>>, Vec<LitRef<'_>>)> = Vec::new();
        for (i, lit) in body.iter().enumerate() {
            let Lit::Atom(atom) = lit.view() else { continue };
            let Some((request, complete, arity)) = members.get(v.rel_name(atom.rel())) else { continue };
            let keys: Vec<TermRef<'_>> = atom.args().take(*arity).collect();
            if keys.len() < *arity {
                continue;
            }
            // A negative test may witness absence only for a complete key, including an explicitly requested key.
            if atom.neg() {
                let guard = shown_atom(v, r, complete, false, &keys);
                let present = body.iter().any(|l| shown_lit(v, r, *l) == guard) || guards.iter().any(|(rel, k)| shown_atom(v, r, rel, false, k) == guard);
                if !present {
                    guards.push((complete.clone(), keys.clone()));
                }
            }
            if head == *request || keys.iter().any(|k| matches!(k, TermRef::Wild | TermRef::Top)) {
                continue;
            }
            let Some(domain) = request_domain(v, &body, i, &keys, request, complete, &depends_on_answer) else { continue };
            asks.push((request.clone(), keys, domain));
        }
        for (request, keys, domain) in asks {
            let shown = format!("{} :- {}", shown_atom(v, r, &request, false, &keys), domain.iter().map(|l| shown_lit(v, r, *l)).collect::<Vec<_>>().join(", "));
            if !stated.insert(shown) {
                continue;
            }
            let ks: Vec<T<'_>> = keys.iter().map(|k| t_of(r, *k)).collect();
            let mut ask = b.rule().head(request.as_str(), &ks);
            for l in domain {
                ask = ask.lit(v, r, l);
            }
            ask.finish().map_err(Error::Syntax)?;
            changed = true;
        }
        if !guards.is_empty() {
            b.remove(r.id());
            let mut guarded = b.rewrite(v, r, &mut Same);
            for (complete, keys) in &guards {
                let ks: Vec<T<'_>> = keys.iter().map(|k| t_of(r, *k)).collect();
                guarded = guarded.atom(complete.as_str(), &ks);
            }
            guarded.finish().map_err(Error::Syntax)?;
            changed = true;
        }
    }
    if !changed {
        return Ok(None);
    }
    b.finish().map(|(v, _)| Some(v)).map_err(Error::Syntax)
}

/// The literals of rule `r` that bind the keys of its `reader`th literal's request: the fewest of its positive atoms
/// (independent ones first) and equalities that ground them, then the literals that select among those bindings.
fn request_domain<'v>(
    v: &'v Version,
    rule: &[LitRef<'v>],
    reader: usize,
    keys: &[TermRef<'v>],
    request: &str,
    complete: &str,
    dependent: &BTreeSet<String>,
) -> Option<Vec<LitRef<'v>>> {
    let mut key_vars = BTreeSet::new();
    keys.iter().for_each(|k| term_vars(*k, &mut key_vars));
    let rel_of = |l: &LitRef<'v>| match l.view() {
        Lit::Atom(a) => Some((v.rel_name(a.rel()), a.neg())),
        _ => None,
    };
    let mut body: Vec<LitRef<'v>> = rule
        .iter()
        .enumerate()
        .filter(|(i, l)| *i != reader && rel_of(l).is_some_and(|(rel, neg)| !neg && rel != complete && rel != request))
        .map(|(_, l)| *l)
        .collect();
    // Prefer independent binders. A preceding lookup can supply a key such as a parent's ID.
    body.sort_by_key(|l| rel_of(l).is_some_and(|(rel, _)| dependent.contains(rel)));
    body.extend(rule.iter().filter(|l| matches!(l.view(), Lit::Cmp { op: CmpOp::Eq, .. })).copied());
    if !grounds(&body, &key_vars) {
        return None;
    }
    for i in (0..body.len()).rev() {
        let removed = body.remove(i);
        if !grounds(&body, &key_vars) {
            body.insert(i, removed);
        }
    }
    // Keep joins that select these bindings. A batch(label, id) joined to chosen(label)
    // requests only that label's IDs, even though batch alone would bind id.
    let mut joined = key_vars;
    body.iter().for_each(|l| lit_vars(*l, &mut joined));
    let offsets = |body: &[LitRef<'v>]| body.iter().map(|l| l.offset()).collect::<BTreeSet<u32>>();
    loop {
        let before = body.len();
        for (i, lit) in rule.iter().enumerate() {
            if i == reader || offsets(&body).contains(&lit.offset()) {
                continue;
            }
            let mut used = BTreeSet::new();
            lit_vars(*lit, &mut used);
            let select = match lit.view() {
                Lit::Atom(a) if !dependent.contains(v.rel_name(a.rel())) => {
                    if a.neg() { used.is_subset(&joined) } else { !used.is_disjoint(&joined) }
                }
                Lit::Cmp { .. } => used.is_subset(&joined),
                _ => false,
            };
            if select {
                joined.extend(used);
                body.push(*lit);
            }
        }
        if before == body.len() {
            return Some(body);
        }
    }
}

/// Whether `body` binds every one of `vars`: a positive atom binds its variables, `x = t` binds `x` once `t`'s are,
/// and `nearest(n, a)` binds `a` once `n` is.
fn grounds(body: &[LitRef<'_>], vars: &BTreeSet<Slot>) -> bool {
    let mut bound: BTreeSet<Slot> = BTreeSet::new();
    for l in body {
        if let Lit::Atom(a) = l.view()
            && !a.neg()
        {
            a.args().for_each(|t| term_vars(t, &mut bound));
        }
    }
    loop {
        let before = bound.len();
        for l in body {
            match l.view() {
                Lit::Cmp { l: TermRef::Var(x), op: CmpOp::Eq, r } | Lit::Cmp { l: r, op: CmpOp::Eq, r: TermRef::Var(x) } if !bound.contains(&x) => {
                    let mut vs = BTreeSet::new();
                    term_vars(r, &mut vs);
                    if vs.iter().all(|v| bound.contains(v)) {
                        bound.insert(x);
                    }
                }
                Lit::Nearest { n, a, .. } if bound.contains(&n) => {
                    bound.insert(a);
                }
                _ => {}
            }
        }
        if bound.len() == before {
            break;
        }
    }
    vars.iter().all(|v| bound.contains(v))
}

fn term_vars(t: TermRef<'_>, out: &mut BTreeSet<Slot>) {
    match t {
        TermRef::Var(s) => {
            out.insert(s);
        }
        TermRef::Call(_, args) => args.for_each(|a| term_vars(a, out)),
        _ => {}
    }
}

/// The variables literal `l` names (an atom's, a comparison's, `nearest`'s).
fn lit_vars(l: LitRef<'_>, out: &mut BTreeSet<Slot>) {
    match l.view() {
        Lit::Atom(a) => a.args().for_each(|t| term_vars(t, out)),
        Lit::Cmp { l, r, .. } => {
            term_vars(l, out);
            term_vars(r, out);
        }
        Lit::Nearest { n, a, .. } => {
            out.extend([n, a]);
        }
        Lit::Agg { var, over, .. } => {
            out.insert(var);
            out.extend(over);
        }
        Lit::Ast { document, node, .. } => {
            out.extend([document, node]);
        }
    }
}

/// A term of rule `r`, as text: what two literals are compared by (the same rule's, or another's by its names).
fn shown_term(v: &Version, r: RuleRef<'_>, t: TermRef<'_>) -> String {
    match t {
        TermRef::Var(s) => v.text(r.var(s)).to_owned(),
        TermRef::Wild => "_".to_owned(),
        TermRef::Top => "top".to_owned(),
        TermRef::Int(i) => i.to_string(),
        TermRef::Str(a) => format!("{:?}", v.text(a)),
        TermRef::Branded(b) => format!("{b:?}"),
        TermRef::Call(f, args) => format!("@{}({})", v.text(f), args.map(|a| shown_term(v, r, a)).collect::<Vec<_>>().join(", ")),
    }
}

fn shown_atom(v: &Version, r: RuleRef<'_>, rel: &str, neg: bool, args: &[TermRef<'_>]) -> String {
    format!("{}{rel}({})", if neg { "!" } else { "" }, args.iter().map(|t| shown_term(v, r, *t)).collect::<Vec<_>>().join(", "))
}

fn shown_lit(v: &Version, r: RuleRef<'_>, l: LitRef<'_>) -> String {
    match l.view() {
        Lit::Atom(a) => shown_atom(v, r, v.rel_name(a.rel()), a.neg(), &a.args().collect::<Vec<_>>()),
        Lit::Cmp { l, op, r: x } => format!("{} {} {}", shown_term(v, r, l), op.as_str(), shown_term(v, r, x)),
        // Not compared: another kind of literal is no request's domain, nor a guard.
        _ => format!("#{}", l.offset()),
    }
}

fn shown_rule(v: &Version, r: RuleRef<'_>) -> String {
    let h = r.head();
    let head = shown_atom(v, r, v.rel_name(h.rel()), false, &h.args().collect::<Vec<_>>());
    format!("{head} :- {}", r.body().map(|l| shown_lit(v, r, l)).collect::<Vec<_>>().join(", "))
}
