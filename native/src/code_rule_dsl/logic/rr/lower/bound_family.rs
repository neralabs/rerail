//! A library can declare a finite request domain and the evidence needed to negate a member.
use super::{Atom, BTreeMap, BTreeSet, Error, Literal, Program, Rule, Term, grounds};

pub(super) fn specs(prog: &Program) -> impl Iterator<Item = &str> {
    prog.settings
        .iter()
        .filter(|(key, _)| key == "pragma")
        .filter_map(|(_, text)| text.strip_prefix("\"bound-family\" \"")?.strip_suffix('"'))
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

pub(super) fn lower(
    prog: &mut Program,
    inherited: &[String],
    dependencies: &BTreeMap<String, BTreeSet<String>>,
) -> Result<(), Error> {
    let mut members = BTreeMap::new();
    for spec in inherited.iter().map(String::as_str).chain(specs(prog)) {
        let names: Vec<_> = spec.split_whitespace().collect();
        let [request, complete, rest @ ..] = names.as_slice() else {
            continue;
        };
        let columns = |name: &str| {
            prog.decls
                .get(name)
                .ok_or_else(|| Error::Syntax(format!("bound-family: undeclared relation {name}")))
        };
        let key = columns(request)?;
        for name in std::iter::once(complete).chain(rest) {
            let cols = columns(name)?;
            if key.is_empty()
                || (*name == *complete && cols.len() != key.len())
                || cols.len() < key.len()
                || !key.iter().zip(cols.iter()).all(|((_, a), (_, b))| a == b)
            {
                return Err(Error::Syntax(format!(
                    "bound-family: {name} needs the key columns of {request}"
                )));
            }
        }
        for member in rest {
            let mode = (request.to_string(), complete.to_string(), key.len());
            if members
                .insert(member.to_string(), mode.clone())
                .is_some_and(|old| old != mode)
            {
                return Err(Error::Syntax(format!("bound-family: conflicting declarations for {member}")));
            }
        }
    }
    if members.is_empty() {
        return Ok(());
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
    let mut generated = Vec::new();
    for rule in &mut prog.rules {
        let mut guards = Vec::new();
        for (i, lit) in rule.body.iter().enumerate() {
            let Some(atom) = lit.atom() else { continue };
            let Some((request, complete, arity)) = members.get(atom.rel.as_str()) else {
                continue;
            };
            let Some(keys) = atom.args.get(..*arity) else { continue };
            let keyed = |name: &str| Atom {
                rel: name.into(),
                args: keys.iter().cloned().collect(),
                neg: false,
                gated: false,
            };
            // A negative test may witness absence only for a complete key, including an explicitly requested key.
            if atom.neg {
                let guard = Literal::Atom(keyed(complete));
                if !rule.body.contains(&guard) && !guards.contains(&guard) {
                    guards.push(guard);
                }
            }
            if rule.head.rel == request.as_str() || keys.iter().any(|k| matches!(k, Term::Wild | Term::Top)) {
                continue;
            }
            let head = keyed(request);
            let Some(body) = request_domain(rule, i, &head, complete, &depends_on_answer) else {
                continue;
            };
            let ask = Rule { head, body: body.into() };
            if !generated.contains(&ask) {
                generated.push(ask);
            }
        }
        if !guards.is_empty() {
            rule.body = rule.body.iter().cloned().chain(guards).collect();
        }
    }
    for ask in generated {
        if !prog.rules.contains(&ask) {
            prog.rules.push(ask);
        }
    }
    Ok(())
}

fn request_domain(rule: &Rule, reader: usize, head: &Atom, complete: &str, dependent: &BTreeSet<String>) -> Option<Vec<Literal>> {
    let keys = head.vars();
    let vars: Vec<_> = keys.iter().cloned().collect();
    let mut body: Vec<_> = rule
        .body
        .iter()
        .enumerate()
        .filter(|(i, l)| *i != reader && matches!(l, Literal::Atom(a) if !a.neg && a.rel != complete && a.rel != head.rel))
        .map(|(_, l)| l.clone())
        .collect();
    // Prefer independent binders. A preceding lookup can supply a key such as a parent's ID.
    body.sort_by_key(|l| l.atom().is_some_and(|a| dependent.contains(a.rel.as_str())));
    body.extend(
        rule.body
            .iter()
            .filter(|l| matches!(l, Literal::Cmp { op: "=", .. }))
            .cloned(),
    );
    if !grounds(&body, &vars) {
        return None;
    }
    for i in (0..body.len()).rev() {
        let removed = body.remove(i);
        if !grounds(&body, &vars) {
            body.insert(i, removed);
        }
    }
    // Keep joins that select these bindings. A batch(label, id) joined to chosen(label)
    // requests only that label's IDs, even though batch alone would bind id.
    let mut joined = keys;
    joined.extend(body.iter().flat_map(Literal::vars));
    loop {
        let before = body.len();
        for (i, lit) in rule.body.iter().enumerate() {
            if i == reader || body.contains(lit) {
                continue;
            }
            let used = lit.vars();
            let select = match lit {
                Literal::Atom(a) if !dependent.contains(a.rel.as_str()) => {
                    if a.neg {
                        used.is_subset(&joined)
                    } else {
                        !used.is_disjoint(&joined)
                    }
                }
                Literal::Cmp { .. } => used.is_subset(&joined),
                _ => false,
            };
            if select {
                joined.extend(used);
                body.push(lit.clone());
            }
        }
        if before == body.len() {
            return Some(body);
        }
    }
}
