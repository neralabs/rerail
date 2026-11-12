//! SQLite's physical sentinel column for logical zero-column relations and zero-key lattices.
use crate::code_rule_dsl::logic::front;
use napi::Result;
use std::collections::{BTreeMap, BTreeSet};
fn error(message: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(message.to_string())
}

pub fn logical_arity(program:&front::Program,name:&str)->usize {
    if program.nullary.contains(name){0}else{program.decls[name].len()}
}

pub fn physical(program: &mut front::Program) {
    let sentinel: BTreeSet<String> = program
        .decls
        .iter()
        .filter(|(name, cols)| {
            cols.is_empty() || program.lattice.get(*name).is_some_and(|l| l.keys == 0)
        })
        .map(|(name, _)| name.clone())
        .collect();
    program.nullary.extend(program.decls.iter().filter(|(_,cols)|cols.is_empty()).map(|(name,_)|name.clone()));
    for name in &sentinel {
        program
            .decls
            .get_mut(name)
            .unwrap()
            .insert(0, ("__presence".into(), "int".into()));
        if let Some(lattice) = program.lattice.get_mut(name) {
            lattice.keys += 1;
        }
        for row in program.facts.get_mut(name).into_iter().flatten() {
            row.insert(0, front::Value::Int(1));
        }
    }
    let atom = |a: &mut front::Atom| {
        if sentinel.contains(a.rel.as_str()) {
            a.args.insert(0, front::Term::Int(1));
        }
    };
    for rule in &mut program.rules {
        atom(&mut rule.head);
        for lit in &mut rule.body {
            match lit {
                front::Literal::Atom(a) | front::Literal::Agg { atom: a, .. } => atom(a),
                _ => {}
            }
        }
    }
}

/// The `top` keyword belongs to a lattice, rather than the flat lattice's numeric sentinel.
pub fn tops(
    program: &mut front::Program,
    lattices: &BTreeMap<String, (usize, i64)>,
) -> Result<()> {
    fn term(t: &mut front::Term, top: i64) {
        match t {
            front::Term::Top => *t = front::Term::Int(top),
            front::Term::Call(_, args) => {
                for a in args {
                    term(a, top)
                }
            }
            _ => {}
        }
    }
    for rule in &mut program.rules {
        let default = lattices
            .get(rule.head.rel.as_str())
            .map_or(crate::code_rule_dsl::logic::native::TOP, |(_, top)| *top);
        let mut known = BTreeMap::<String, BTreeSet<i64>>::new();
        for atom in rule
            .body
            .iter()
            .filter_map(front::Literal::atom)
            .chain([&rule.head])
        {
            if let Some((col, top)) = lattices.get(atom.rel.as_str())
                && let Some(front::Term::Var(v)) = atom.args.get(*col)
            {
                known.entry(v.clone()).or_default().insert(*top);
            }
        }
        loop {
            let before: usize = known.values().map(BTreeSet::len).sum();
            for lit in &rule.body {
                if let front::Literal::Cmp { l, r, op } = lit
                    && *op == "="
                {
                    for (target, source) in [(l, r), (r, l)] {
                        if let front::Term::Var(target) = target {
                            let mut vars = BTreeSet::new();
                            source.vars(&mut vars);
                            let values: BTreeSet<_> = vars
                                .iter()
                                .filter_map(|v| known.get(v))
                                .flatten()
                                .copied()
                                .collect();
                            if !values.is_empty() {
                                known.entry(target.clone()).or_default().extend(values);
                            }
                        }
                    }
                }
            }
            if before == known.values().map(BTreeSet::len).sum::<usize>() {
                break;
            }
        }
        let atom = |a: &mut front::Atom| {
            let top = lattices.get(a.rel.as_str()).map_or(default, |(_, top)| *top);
            for t in &mut a.args {
                term(t, top);
            }
        };
        atom(&mut rule.head);
        for lit in &mut rule.body {
            match lit {
                front::Literal::Atom(a) => atom(a),
                front::Literal::Agg { atom: a, .. } => {
                    atom(a);
                }
                front::Literal::Cmp { l, r, .. } => {
                    let mut variables = BTreeSet::new();
                    l.vars(&mut variables);
                    r.vars(&mut variables);
                    let inferred: BTreeSet<i64> = variables
                        .into_iter()
                        .filter_map(|v| known.get(&v))
                        .flatten()
                        .copied()
                        .collect();
                    if inferred.len() > 1
                        && (matches!(l, front::Term::Top) || matches!(r, front::Term::Top))
                    {
                        return Err(error("top is ambiguous between different lattice domains"));
                    }
                    let top = inferred.first().copied().unwrap_or(default);
                    term(l, top);
                    term(r, top);
                }
                _ => {}
            }
        }
    }
    let mut rules = Vec::new();
    for rule in std::mem::take(&mut program.rules) {
        let row = rule
            .head
            .args
            .iter()
            .map(|t| match t {
                front::Term::Int(n) => Some(front::Value::Int(*n)),
                front::Term::Str(s) => Some(front::Value::Str(s.clone())),
                _ => None,
            })
            .collect::<Option<Vec<_>>>();
        if rule.body.is_empty()
            && let Some(row) = row
        {
            program.facts.entry(rule.head.rel.to_string()).or_default().push(row);
        } else {
            rules.push(rule);
        }
    }
    program.rules = rules;
    Ok(())
}
