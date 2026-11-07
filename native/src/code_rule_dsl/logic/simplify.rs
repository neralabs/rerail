//! Rule bodies simplified before planning (decision 0038), so a long or repeated rule costs what it says:
//!
//! - **Implied atoms drop.** A positive atom goes when another positive atom of the same relation has, at every
//!   position, the same term, or any term where it has `_`: `doc($d, $p, _)` beside `doc($d, $p, $m)`.
//! - **A kind test on a set member drops.** `node(_, $a, "k", _, _)` goes where `$a` is the ancestor of a
//!   `nearest($n, $a, set)` or `inside_<set>($n, $a)` of the same rule and the set is exactly `{k}`.
//! - **A variable equal to a constant is the constant.** `text($n, $v), $v = "get"` is `text($n, "get")`: the
//!   planner then reads the atom by its constant column (a lookup by string), where a separate `$v = "get"` binds
//!   `$v` only after the atom is scanned some other way. A variable the head names stays (a head column filled from a
//!   text column keeps its relation a string filter, decision 0037), as does one an aggregate, `nearest` or `ast` names.
//! - **A test of one node read only positively is inlined.** `m($n) :- node(_, $n, _, _, _), !k($n)` (a complement:
//!   the converter's `not`) read as `m($t)` becomes the reader's `node(_, $t, _, _, _), !k($t)`: where the reader has
//!   bound `$t`, a probe of `k`, not a relation of every node less `k`'s. Such a relation has one rule, over its one
//!   node variable alone and not recursive, and no negated or aggregated reader; kept relations (below) stay. (One
//!   rule: a relation of several would copy each reader once per rule, its whole join run again.)
//! - **A variable named once is `_`.** In a positive atom, `text($v122, $s)` where `$v122` appears nowhere else is
//!   `text(_, $s)`; it is then implied by any `text($n, $s)` beside it and goes.
//! - **Two atoms of one keyed row are one.** `node($d, $n, _, _, _), node(_, $n, "call", _, _)` is
//!   `node($d, $n, "call", _, _)`: a node's row is one (`KEYS`), so atoms with the same key term read the same row.
//!   Columns merge where one is `_` or both are the same; two variables in one column are one variable.
//! - **A part of a body that meets the rest only at a document is a relation of documents.**
//!   `m($n) :- node($d, $n, "module", _, _), node($d, $y, _, _, _), name($y, $s), @contains($s, "cache")` asks
//!   whether the file names something with `cache`, once per such name: every match multiplies the rest's rows. The
//!   part `node($d, $y, …), name($y, $s), @contains(…)` shares only `$d` with the rest and the head, so it becomes
//!   `exists_k($d) :- node($d, $y, _, _, _), name($y, $s), @contains($s, "cache")`, one row per file, and the rule reads
//!   `exists_k($d)` (a semi-join). The same set of rows: the part's other variables appear nowhere else. A part moves
//!   when it has two literals or more, a variable of its own, no aggregate, `nearest` or `ast`, and every variable
//!   it names bound by a positive atom of its own. A document variable is one in `node`'s or `doc`'s first column.
//! - **Relations no output reads go.** A derived relation that no output, demanded or kept relation reads,
//!   directly or through others, is not computed.
//! - **Identical relations merge.** Derived relations whose rules are equal once their heads and variables are
//!   renamed in order are one relation; readers read the first. Repeated to a fixpoint, so relations over merged
//!   ones merge too. Outputs, inputs, lattices, relations with facts and demanded relations are kept as they are.

use super::front::{Atom, Literal, Program, Rule, Term};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// What was simplified.
#[derive(Debug, Default, PartialEq)]
pub(in crate::code_rule_dsl) struct Report {
    pub(in crate::code_rule_dsl) implied: usize,
    pub(in crate::code_rule_dsl) kind_tests: usize,
    /// Variables equal to a constant, replaced by it.
    pub(in crate::code_rule_dsl) constants: usize,
    /// Merged relations: (kept, merged into it).
    pub(in crate::code_rule_dsl) merged: Vec<(String, String)>,
    /// Relations inlined into their readers.
    pub(in crate::code_rule_dsl) inlined: Vec<String>,
    /// Variables named once, made `_`.
    pub(in crate::code_rule_dsl) singletons: usize,
    /// Atoms merged into another of the same keyed row.
    pub(in crate::code_rule_dsl) keyed: usize,
    /// Relations nothing kept reads, dropped.
    pub(in crate::code_rule_dsl) unread: Vec<String>,
    /// Parts of bodies that meet the rest only at a document, made relations of documents (`exists_k`).
    pub(in crate::code_rule_dsl) existentials: Vec<String>,
    /// Span comparisons of two nodes of one document, made `span_within(inner, outer)` (decision 0050).
    pub(in crate::code_rule_dsl) within: usize,
}

pub(in crate::code_rule_dsl) fn simplify(prog: &mut Program) -> Report {
    let mut report = Report::default();
    report.inlined = inline_node_tests(prog);
    for rule in &mut prog.rules {
        report.constants += fold_constants(rule);
        report.within += span_containment(rule);
        report.keyed += merge_keyed(rule);
        report.singletons += wild_singletons(rule);
        report.kind_tests += drop_kind_tests(rule, &prog.kindsets, &prog.decls);
        report.implied += drop_implied(rule);
    }
    if report.within > 0 && !prog.decls.contains_key(SPAN_WITHIN) {
        prog.decls.insert(SPAN_WITHIN.to_owned(), vec![("inner".to_owned(), "node".to_owned()), ("outer".to_owned(), "node".to_owned())]);
        prog.inputs.push(SPAN_WITHIN.to_owned());
    }
    report.existentials = project_existentials(prog);
    if merge(prog, &mut report) {
        // A merge can leave a literal twice (`!m1($n), !m2($n)` became `!m1($n), !m1($n)`).
        for rule in &mut prog.rules {
            report.implied += drop_implied(rule);
        }
    }
    report.unread = drop_unread(prog);
    if super::profile::on() {
        let (i, k, c, m, n) = (report.implied, report.kind_tests, report.constants, report.merged.len(), report.inlined.len());
        let (w, y, u, x, s) = (report.singletons, report.keyed, report.unread.len(), report.existentials.len(), report.within);
        super::profile::decision("simplified bodies (decision 0038)", || {
            format!("{n} tests of one node inlined into their readers; {i} implied atoms and {k} kind tests on set members dropped; {c} variables equal to a constant replaced by it; {m} relations merged into an identical one; {w} variables named once made `_`; {y} atoms merged into another of their keyed row; {x} parts of bodies that meet the rest only at a document made relations of documents; {s} span comparisons of two nodes made `span_within`; {u} relations no output reads dropped")
        });
    }
    report
}

const SPAN_WITHIN: &str = crate::code_rule_dsl::facts::vtab::syntax::SPAN_WITHIN;

/// Replaces each pair `node(d, a, _, sa, ea), node(d, b, _, sb, eb), sb <= sa, ea <= eb` (one document variable
/// `d`, either side of each comparison) by `span_within(a, b)`: the comparisons, not the atoms, which may bind or test
/// more. A join of every pair of a file's nodes becomes a walk of the tree (`syntax::within`). Returns the pairs
/// replaced.
fn span_containment(rule: &mut Rule) -> usize {
    let var = |t: &Term| if let Term::Var(v) = t { Some(v.clone()) } else { None };
    // (d, n, start, end) of each positive node atom with variables there.
    let nodes: Vec<(String, String, String, String)> = rule
        .body
        .iter()
        .filter_map(|l| match l {
            Literal::Atom(a) if !a.neg && a.rel == "node" && a.args.len() == 5 => {
                Some((var(&a.args[0])?, var(&a.args[1])?, var(&a.args[3])?, var(&a.args[4])?))
            }
            _ => None,
        })
        .collect();
    // `x <= y` as written, either way round.
    let le = |l: &Literal| -> Option<(String, String)> {
        match l {
            Literal::Cmp { l, op: "<=", r } => Some((var(l)?, var(r)?)),
            Literal::Cmp { l, op: ">=", r } => Some((var(r)?, var(l)?)),
            _ => None,
        }
    };
    let mut made = 0;
    'pairs: for (i, inner) in nodes.iter().enumerate() {
        for (j, outer) in nodes.iter().enumerate() {
            if i == j || inner.0 != outer.0 || inner.1 == outer.1 {
                continue;
            }
            let starts = rule.body.iter().position(|l| le(l) == Some((outer.2.clone(), inner.2.clone())));
            let ends = rule.body.iter().position(|l| le(l) == Some((inner.3.clone(), outer.3.clone())));
            let (Some(starts), Some(ends)) = (starts, ends) else { continue };
            let within = Literal::Atom(Atom { rel: SPAN_WITHIN.to_owned(), args: vec![Term::Var(inner.1.clone()), Term::Var(outer.1.clone())], neg: false, gated: false });
            rule.body[starts.min(ends)] = within;
            rule.body.remove(starts.max(ends));
            made += 1;
            // The rule changed: look again for another pair.
            made += span_containment(rule);
            break 'pairs;
        }
    }
    made
}

/// Moves each part of a body that meets the rest only at a document variable into a relation of documents
/// (`exists_<k>`, the module's list); returns the relations made.
fn project_existentials(prog: &mut Program) -> Vec<String> {
    let mut made = Vec::new();
    let mut at = 0;
    while at < prog.rules.len() {
        let mut next = made.len();
        let name = loop {
            let name = format!("exists_{next}");
            if !prog.decls.contains_key(&name) {
                break name;
            }
            next += 1;
        };
        match existential(&mut prog.rules[at], &name, &prog.decls) {
            // The same rule again: it may have another such part.
            Some((rule, columns)) => {
                prog.decls.insert(name.clone(), columns);
                prog.rules.push(rule);
                made.push(name);
            }
            None => at += 1,
        }
    }
    made
}

/// The first part of `rule`'s body that meets the rest and the head only at one document variable, replaced by
/// `name($d)`: that relation's rule and columns.
fn existential(rule: &mut Rule, name: &str, decls: &BTreeMap<String, Vec<(String, String)>>) -> Option<(Rule, Vec<(String, String)>)> {
    let mut head = BTreeSet::new();
    rule.head.args.iter().for_each(|t| t.vars(&mut head));
    let docs: BTreeSet<String> = rule
        .body
        .iter()
        .filter_map(|l| match l {
            Literal::Atom(a) if (a.rel == "node" && a.args.len() == 5) || a.rel == "doc" => match a.args.first() {
                Some(Term::Var(d)) => Some(d.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    let vars: Vec<BTreeSet<String>> = rule.body.iter().map(Literal::vars).collect();
    for d in &docs {
        // Parts: literals joined by a shared variable other than `$d`.
        let mut part: Vec<usize> = (0..rule.body.len()).collect();
        fn root(part: &mut [usize], i: usize) -> usize {
            let mut r = i;
            while part[r] != r {
                r = part[r];
            }
            part[i] = r;
            r
        }
        let mut first: HashMap<&str, usize> = HashMap::new();
        for (i, vs) in vars.iter().enumerate() {
            for v in vs.iter().filter(|v| *v != d) {
                match first.get(v.as_str()) {
                    Some(&j) => {
                        let (a, b) = (root(&mut part, i), root(&mut part, j));
                        part[a] = b;
                    }
                    None => {
                        first.insert(v, i);
                    }
                }
            }
        }
        let roots: BTreeSet<usize> = (0..rule.body.len()).map(|i| root(&mut part, i)).collect();
        for r in roots {
            let members: Vec<usize> = (0..rule.body.len()).filter(|&i| root(&mut part, i) == r).collect();
            if members.len() < 2 || members.len() == rule.body.len() {
                continue;
            }
            let own: BTreeSet<String> = members.iter().flat_map(|&i| vars[i].iter().cloned()).collect();
            // Only `$d` may be shared with the rest or the head (it is: other shared variables join parts).
            if own.iter().any(|v| v != d && head.contains(v)) || !own.contains(d) || own.len() < 2 {
                continue;
            }
            let shares = head.contains(d) || (0..rule.body.len()).any(|i| !members.contains(&i) && vars[i].contains(d));
            if !shares {
                continue;
            }
            let lits: Vec<&Literal> = members.iter().map(|&i| &rule.body[i]).collect();
            if lits.iter().any(|l| matches!(l, Literal::Agg { .. } | Literal::Nearest { .. } | Literal::Ast { .. })) {
                continue;
            }
            let bound: BTreeSet<String> = lits.iter().filter_map(|l| match l {
                Literal::Atom(a) if !a.neg => Some(a.vars()),
                _ => None,
            }).flatten().collect();
            if !own.is_subset(&bound) {
                continue;
            }
            // `$d`'s column type, from a positive atom of the part that names it.
            let ty = lits.iter().find_map(|l| match l {
                Literal::Atom(a) if !a.neg => a.args.iter().position(|t| matches!(t, Term::Var(v) if v == d)).and_then(|c| Some(decls.get(&a.rel)?.get(c)?.1.clone())),
                _ => None,
            })?;
            let atom = |neg| Atom { rel: name.to_owned(), args: vec![Term::Var(d.clone())], neg, gated: false };
            let body: Vec<Literal> = lits.into_iter().cloned().collect();
            let at = members[0];
            let mut kept = Vec::with_capacity(rule.body.len() - members.len() + 1);
            for (i, l) in std::mem::take(&mut rule.body).into_iter().enumerate() {
                if i == at {
                    kept.push(Literal::Atom(atom(false)));
                }
                if !members.contains(&i) {
                    kept.push(l);
                }
            }
            rule.body = kept;
            return Some((Rule { head: atom(false), body }, vec![("d".to_owned(), ty)]));
        }
    }
    None
}

/// Replaces each variable the body equates to a constant (`$v = "s"`, `$v = 3`) by the constant, in the head and
/// every literal, and drops the equation. Returns how many variables went.
fn fold_constants(rule: &mut Rule) -> usize {
    let constant = |t: &Term| matches!(t, Term::Str(_) | Term::Int(_));
    // Variables the head names, or a literal other than as a term (an aggregate's result, `nearest`, `ast`), stay
    // variables. A head column the rule fills from a text column is what keeps its relation a string filter, run per
    // chunk (`typing::file_local::split`); `t($s) :- doc(_, $s, _), $s = "a.py"` stays as it is.
    let mut named = rule.head.vars();
    for l in &rule.body {
        match l {
            Literal::Agg { var, over, .. } => {
                named.insert(var.clone());
                named.extend(over.iter().cloned());
            }
            Literal::Nearest { n, a, .. } => {
                named.insert(n.clone());
                named.insert(a.clone());
            }
            Literal::Ast { document, node, captures, .. } => {
                named.insert(document.clone());
                named.insert(node.clone());
                named.extend(captures.iter().map(|(_, v)| v.clone()));
            }
            _ => {}
        }
    }
    let mut folded = 0;
    loop {
        let found = rule.body.iter().position(|l| match l {
            Literal::Cmp { l: Term::Var(v), op: "=", r } | Literal::Cmp { l: r, op: "=", r: Term::Var(v) } => constant(r) && !named.contains(v),
            _ => false,
        });
        let Some(at) = found else { return folded };
        let (var, value) = match rule.body.remove(at) {
            Literal::Cmp { l: Term::Var(v), r, .. } if constant(&r) => (v, r),
            Literal::Cmp { l, r: Term::Var(v), .. } => (v, l),
            _ => unreachable!("an equation of a variable and a constant"),
        };
        fn put(t: &mut Term, var: &str, value: &Term) {
            match t {
                Term::Var(v) if v == var => *t = value.clone(),
                Term::Call(_, args) => args.iter_mut().for_each(|a| put(a, var, value)),
                _ => {}
            }
        }
        for l in &mut rule.body {
            match l {
                Literal::Atom(a) | Literal::Agg { atom: a, .. } => a.args.iter_mut().for_each(|t| put(t, &var, &value)),
                Literal::Cmp { l, r, .. } => {
                    put(l, &var, &value);
                    put(r, &var, &value);
                }
                Literal::Nearest { .. } | Literal::Ast { .. } => {}
            }
        }
        folded += 1;
    }
}

/// Whether a relation's rules may not change: outputs, inputs, lattices, relations with facts, demanded relations and
/// gates.
fn kept(prog: &Program, rel: &str) -> bool {
    prog.outputs.iter().any(|o| o == rel)
        || prog.inputs.iter().any(|i| i == rel)
        || prog.lattice.contains_key(rel)
        || prog.facts.get(rel).is_some_and(|f| !f.is_empty())
        || prog.demand.iter().any(|d| d == rel)
        || prog.gates.iter().any(|(g, _)| g == rel)
}

/// Inlines each one-rule test of one node read only positively (the module's list); returns the relations inlined.
fn inline_node_tests(prog: &mut Program) -> Vec<String> {
    join_complements(prog);
    // Each candidate's bodies, over the variable `$h`.
    let mut bodies: BTreeMap<String, Vec<Vec<Literal>>> = BTreeMap::new();
    let mut refused: BTreeSet<String> = BTreeSet::new();
    for rule in &prog.rules {
        let rel = &rule.head.rel;
        let one_node = match rule.head.args.as_slice() {
            [Term::Var(h)] => {
                let tests_node = rule.body.iter().any(|l| matches!(l, Literal::Atom(a) if !a.neg && a.rel == "node" && a.args.get(1) == Some(&Term::Var(h.clone()))));
                let alone = rule.body.iter().all(|l| matches!(l, Literal::Atom(_) | Literal::Cmp { .. }) && l.vars().iter().all(|v| v == h));
                let recursive = rule.body.iter().any(|l| matches!(l, Literal::Atom(a) if a.rel == *rel));
                (tests_node && alone && !recursive).then(|| rule.body.iter().map(|l| rename_var(l, h, "$h")).collect::<Vec<_>>())
            }
            _ => None,
        };
        match one_node {
            Some(body) if !kept(prog, rel) => bodies.entry(rel.clone()).or_default().push(body),
            _ => {
                refused.insert(rel.clone());
            }
        }
    }
    // Read only as a positive atom over a variable, by rules of other relations.
    for rule in &prog.rules {
        for l in &rule.body {
            match l {
                Literal::Atom(a) if !a.neg && matches!(a.args.as_slice(), [Term::Var(_)]) => {}
                Literal::Atom(a) | Literal::Agg { atom: a, .. } => {
                    refused.insert(a.rel.clone());
                }
                _ => {}
            }
        }
    }
    bodies.retain(|rel, rules| !refused.contains(rel) && rules.len() == 1);
    if bodies.is_empty() {
        return Vec::new();
    }
    // A candidate's body may read another: inline readers in turn until none reads one (each pass removes a level).
    let mut rules = prog.rules.clone();
    for _ in 0..8 {
        let mut changed = false;
        let mut next = Vec::with_capacity(rules.len());
        for mut rule in rules {
            // Each read in turn: an inlined body may read another candidate, read on the next pass.
            let mut at = 0;
            while at < rule.body.len() {
                let candidate = match &rule.body[at] {
                    Literal::Atom(a) if !a.neg && bodies.contains_key(&a.rel) => match &a.args[0] {
                        Term::Var(t) => Some((a.rel.clone(), t.clone())),
                        _ => None,
                    },
                    _ => None,
                };
                let Some((rel, t)) = candidate else {
                    at += 1;
                    continue;
                };
                changed = true;
                let body: Vec<Literal> = bodies[&rel][0].iter().map(|l| rename_var(l, "$h", &t)).collect();
                let len = body.len();
                rule.body.splice(at..=at, body);
                at += len;
            }
            next.push(rule);
        }
        rules = next;
        if !changed {
            break;
        }
    }
    // Unfinished (a chain of reads deeper than the passes): nothing is inlined.
    let reads = |r: &Rule| r.body.iter().any(|l| matches!(l, Literal::Atom(a) if bodies.contains_key(&a.rel)));
    if rules.iter().any(|r| !bodies.contains_key(&r.head.rel) && reads(r)) {
        return Vec::new();
    }
    rules.retain(|r| !bodies.contains_key(&r.head.rel));
    prog.rules = rules;
    for rel in bodies.keys() {
        prog.decls.remove(rel);
    }
    bodies.into_keys().collect()
}

/// A relation of several rules each `node(_, $n, _, _, _), !x_i($n)` (the converter's `not` of an `all`: every node
/// failing one of its parts) is one rule, `node(_, $n, _, _, _), !r_all($n)`, with `r_all($n) :- x_1($n), …, x_k($n)`
/// (De Morgan): a test of one node, then inlined. `r_all` holds the nodes passing every part, a join of relations
/// that are each a test already, not every node of the chunk.
fn join_complements(prog: &mut Program) {
    let mut by_rel: BTreeMap<String, Vec<Option<String>>> = BTreeMap::new();
    for rule in &prog.rules {
        let negated = match (rule.head.args.as_slice(), rule.body.as_slice()) {
            ([Term::Var(h)], [Literal::Atom(node), Literal::Atom(x)])
                if !node.neg
                    && node.rel == "node"
                    && node.args.len() == 5
                    && node.args.iter().enumerate().all(|(i, t)| if i == 1 { *t == Term::Var(h.clone()) } else { *t == Term::Wild })
                    && x.neg
                    && !x.gated
                    && x.rel != rule.head.rel
                    && x.args == [Term::Var(h.clone())] =>
            {
                Some(x.rel.clone())
            }
            _ => None,
        };
        by_rel.entry(rule.head.rel.clone()).or_default().push(negated);
    }
    for (rel, parts) in by_rel {
        if parts.len() < 2 || kept(prog, &rel) {
            continue;
        }
        let Some(parts) = parts.into_iter().collect::<Option<Vec<String>>>() else { continue };
        let all = format!("{rel}_all");
        if prog.decls.contains_key(&all) {
            continue;
        }
        let h = || Term::Var("n".to_owned());
        let atom = |rel: &str, neg: bool| Atom { rel: rel.to_owned(), args: vec![h()], neg, gated: false };
        prog.rules.retain(|r| r.head.rel != rel);
        prog.rules.push(Rule { head: atom(&all, false), body: parts.iter().map(|p| Literal::Atom(atom(p, false))).collect() });
        let node = Atom { rel: "node".to_owned(), args: vec![Term::Wild, h(), Term::Wild, Term::Wild, Term::Wild], neg: false, gated: false };
        prog.rules.push(Rule { head: atom(&rel, false), body: vec![Literal::Atom(node), Literal::Atom(atom(&all, true))] });
        prog.decls.insert(all, vec![("n".to_owned(), "node".to_owned())]);
    }
}

/// The base relations whose rows are keyed: (relation, arity, key column). A node has one row of `node` (its
/// document, kind and span), one kind, text, name, start line and document; a document one row of `doc`.
const KEYS: &[(&str, usize, usize)] =
    &[("node", 5, 1), ("doc", 3, 0), ("kind", 2, 0), ("text", 2, 0), ("name", 2, 0), ("line", 2, 0), ("docof", 2, 0)];

/// How many times each variable appears in `rule`, head included.
fn occurrences(rule: &Rule) -> HashMap<String, usize> {
    fn term(t: &Term, n: &mut HashMap<String, usize>) {
        match t {
            Term::Var(v) => *n.entry(v.clone()).or_default() += 1,
            Term::Call(_, args) => args.iter().for_each(|a| term(a, n)),
            _ => {}
        }
    }
    let mut n = HashMap::new();
    rule.head.args.iter().for_each(|t| term(t, &mut n));
    for l in &rule.body {
        match l {
            Literal::Atom(a) => a.args.iter().for_each(|t| term(t, &mut n)),
            Literal::Cmp { l, r, .. } => {
                term(l, &mut n);
                term(r, &mut n);
            }
            Literal::Agg { var, over, atom, .. } => {
                atom.args.iter().for_each(|t| term(t, &mut n));
                *n.entry(var.clone()).or_default() += 1;
                if let Some(o) = over {
                    *n.entry(o.clone()).or_default() += 1;
                }
            }
            Literal::Nearest { n: x, a, .. } => {
                *n.entry(x.clone()).or_default() += 1;
                *n.entry(a.clone()).or_default() += 1;
            }
            Literal::Ast { document, node, captures, .. } => {
                for v in [document, node].into_iter().chain(captures.iter().map(|(_, v)| v)) {
                    *n.entry(v.clone()).or_default() += 1;
                }
            }
        }
    }
    n
}

/// Makes `_` each variable of a positive atom that appears nowhere else in `rule`; returns how many.
fn wild_singletons(rule: &mut Rule) -> usize {
    let once: BTreeSet<String> = occurrences(rule).into_iter().filter(|(_, k)| *k == 1).map(|(v, _)| v).collect();
    let mut made = 0;
    for l in &mut rule.body {
        if let Literal::Atom(a) = l
            && !a.neg
        {
            for t in &mut a.args {
                if matches!(t, Term::Var(v) if once.contains(v)) {
                    *t = Term::Wild;
                    made += 1;
                }
            }
        }
    }
    made
}

/// Merges positive atoms of a keyed relation (`KEYS`) with the same key term into one; returns how many went.
/// Two columns merge where they are equal or one is `_`; two different variables merge by renaming the later one
/// (one the head, atoms and comparisons name, so `rename_var` reaches every use). Anything else stays two atoms.
fn merge_keyed(rule: &mut Rule) -> usize {
    let mut merged = 0;
    'again: loop {
        let special: BTreeSet<String> = rule
            .body
            .iter()
            .filter(|l| matches!(l, Literal::Agg { .. } | Literal::Nearest { .. } | Literal::Ast { .. }))
            .flat_map(|l| l.vars())
            .collect();
        fn keyed(l: &Literal) -> Option<(&Atom, usize)> {
            match l {
                Literal::Atom(a) if !a.neg && !a.gated => KEYS.iter().find(|(r, arity, _)| *r == a.rel && *arity == a.args.len()).map(|&(_, _, k)| (a, k)),
                _ => None,
            }
        }
        for i in 0..rule.body.len() {
            let Some((a, k)) = keyed(&rule.body[i]) else { continue };
            if matches!(a.args[k], Term::Wild | Term::Call(..)) {
                continue;
            }
            for j in i + 1..rule.body.len() {
                let Some((b, _)) = keyed(&rule.body[j]) else { continue };
                if b.rel != a.rel || b.args[k] != a.args[k] {
                    continue;
                }
                let mut args = Vec::with_capacity(a.args.len());
                let mut renames = Vec::new();
                for (x, y) in a.args.iter().zip(&b.args) {
                    match (x, y) {
                        _ if x == y => args.push(x.clone()),
                        (Term::Wild, y) => args.push(y.clone()),
                        (x, Term::Wild) => args.push(x.clone()),
                        (Term::Var(x), Term::Var(y)) if !special.contains(y) => {
                            args.push(Term::Var(x.clone()));
                            renames.push((y.clone(), x.clone()));
                        }
                        _ => {
                            args.clear();
                            break;
                        }
                    }
                }
                if args.is_empty() {
                    continue;
                }
                let one = Literal::Atom(Atom { args, ..a.clone() });
                rule.body[i] = one;
                rule.body.remove(j);
                for (from, to) in renames {
                    rule.body = rule.body.iter().map(|l| rename_var(l, &from, &to)).collect();
                    for t in &mut rule.head.args {
                        if matches!(t, Term::Var(v) if *v == from) {
                            *t = Term::Var(to.clone());
                        }
                    }
                }
                merged += 1;
                continue 'again;
            }
        }
        return merged;
    }
}

/// Drops the rules of derived relations that no kept relation (`kept`: outputs, demanded relations, gates, …)
/// reads, directly or through others; returns those relations.
fn drop_unread(prog: &mut Program) -> Vec<String> {
    let mut reads: HashMap<&str, BTreeSet<&str>> = HashMap::new();
    for rule in &prog.rules {
        reads.entry(rule.head.rel.as_str()).or_default().extend(rule.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.as_str()));
    }
    let mut live: BTreeSet<&str> = BTreeSet::new();
    let mut todo: Vec<&str> = reads.keys().copied().filter(|r| kept(prog, r)).collect();
    while let Some(r) = todo.pop() {
        if live.insert(r) {
            todo.extend(reads.get(r).into_iter().flatten().copied());
        }
    }
    let unread: Vec<String> = reads.keys().filter(|r| !live.contains(**r)).map(|r| (*r).to_owned()).collect::<BTreeSet<_>>().into_iter().collect();
    prog.rules.retain(|r| !unread.contains(&r.head.rel));
    unread
}

/// `l` with the variable `from` renamed `to`.
fn rename_var(l: &Literal, from: &str, to: &str) -> Literal {
    fn term(t: &Term, from: &str, to: &str) -> Term {
        match t {
            Term::Var(v) if v == from => Term::Var(to.to_owned()),
            Term::Call(f, args) => Term::Call(f.clone(), args.iter().map(|a| term(a, from, to)).collect()),
            t => t.clone(),
        }
    }
    match l {
        Literal::Atom(a) => Literal::Atom(Atom { args: a.args.iter().map(|t| term(t, from, to)).collect(), ..a.clone() }),
        Literal::Cmp { l, op, r } => Literal::Cmp { l: term(l, from, to), op: *op, r: term(r, from, to) },
        other => other.clone(),
    }
}

/// Whether `b` implies `a`: the same relation, both positive, and at each position the same term or `_` in `a`.
fn implies(b: &Atom, a: &Atom) -> bool {
    b.rel == a.rel && !a.neg && !b.neg && a.args.len() == b.args.len() && a.args.iter().zip(&b.args).all(|(x, y)| x == y || *x == Term::Wild)
}

fn drop_implied(rule: &mut Rule) -> usize {
    let atoms: Vec<Option<&Atom>> = rule.body.iter().map(|l| if let Literal::Atom(a) = l { Some(a) } else { None }).collect();
    // A dropped atom is implied by one that stays: by a different atom, or by an equal one earlier. Any literal
    // equal to an earlier one goes too.
    let drop: BTreeSet<usize> = (0..atoms.len())
        .filter(|&i| {
            if rule.body[..i].contains(&rule.body[i]) {
                return true;
            }
            let Some(a) = atoms[i] else { return false };
            (0..atoms.len()).any(|j| j != i && atoms[j].is_some_and(|b| implies(b, a) && b != a))
        })
        .collect();
    let n = drop.len();
    let mut at = 0;
    rule.body.retain(|_| {
        at += 1;
        !drop.contains(&(at - 1))
    });
    n
}

fn drop_kind_tests(rule: &mut Rule, kindsets: &BTreeMap<String, Vec<String>>, decls: &BTreeMap<String, Vec<(String, String)>>) -> usize {
    // `node`'s columns as declared: the node and its kind.
    let Some(cols) = decls.get("node") else { return 0 };
    let (Some(n), Some(k)) = (cols.iter().position(|(c, _)| c == "n"), cols.iter().position(|(c, _)| c == "kind")) else { return 0 };
    // The ancestors of a single-kind set, by variable.
    let mut member: HashMap<String, &str> = HashMap::new();
    for l in &rule.body {
        let (a, set) = match l {
            Literal::Nearest { a, kindset, .. } => (a.clone(), kindset.as_str()),
            Literal::Atom(x) if !x.neg && x.args.len() == 2 => match (x.rel.strip_prefix(super::front::AXIS), &x.args[1]) {
                (Some(set), Term::Var(a)) => (a.clone(), set),
                _ => continue,
            },
            _ => continue,
        };
        if let Some([kind]) = kindsets.get(set).map(Vec::as_slice) {
            member.insert(a, kind.as_str());
        }
    }
    if member.is_empty() {
        return 0;
    }
    let before = rule.body.len();
    rule.body.retain(|l| {
        let Literal::Atom(x) = l else { return true };
        if x.rel != "node" || x.neg || x.args.len() != cols.len() {
            return true;
        }
        let only = x.args.iter().enumerate().all(|(c, t)| c == n || c == k || *t == Term::Wild);
        let tested = match (&x.args[n], &x.args[k]) {
            (Term::Var(a), Term::Str(kind)) => member.get(a) == Some(&kind.as_str()),
            _ => false,
        };
        !(only && tested)
    });
    before - rule.body.len()
}

/// A rule with its head named `@` and its variables numbered in order of first appearance.
fn canonical(rule: &Rule) -> String {
    let mut names: HashMap<String, String> = HashMap::new();
    let mut name = |v: &str| -> String {
        let k = names.len();
        names.entry(v.to_owned()).or_insert_with(|| format!("v{k}")).clone()
    };
    fn term(t: &Term, name: &mut dyn FnMut(&str) -> String) -> Term {
        match t {
            Term::Var(v) => Term::Var(name(v)),
            Term::Call(f, args) => Term::Call(f.clone(), args.iter().map(|a| term(a, name)).collect()),
            t => t.clone(),
        }
    }
    let atom = |a: &Atom, name: &mut dyn FnMut(&str) -> String| Atom { args: a.args.iter().map(|t| term(t, name)).collect(), ..a.clone() };
    let mut head = atom(&rule.head, &mut name);
    let own = rule.head.rel.clone();
    head.rel = "@".into();
    let body: Vec<Literal> = rule
        .body
        .iter()
        .map(|l| match l {
            Literal::Atom(a) => {
                let mut a = atom(a, &mut name);
                if a.rel == own {
                    a.rel = "@".into();
                }
                Literal::Atom(a)
            }
            Literal::Cmp { l, op, r } => Literal::Cmp { l: term(l, &mut name), op, r: term(r, &mut name) },
            Literal::Agg { var, op, over, atom: a } => {
                let var = name(var);
                let over = over.as_ref().map(|o| name(o));
                Literal::Agg { var, op: op.clone(), over, atom: atom(a, &mut name) }
            }
            Literal::Nearest { n, a, kindset } => Literal::Nearest { n: name(n), a: name(a), kindset: kindset.clone() },
            Literal::Ast { document, node, matcher, captures } => Literal::Ast {
                document: name(document),
                node: name(node),
                matcher: matcher.clone(),
                captures: captures.iter().map(|(c, v)| (c.clone(), name(v))).collect(),
            },
        })
        .collect();
    Rule { head, body }.to_string()
}

/// Whether anything merged.
fn merge(prog: &mut Program, report: &mut Report) -> bool {
    let mut any = false;
    loop {
        // Each mergeable relation's rules, canonical and sorted, with its declared types.
        let mut by_rel: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for r in &prog.rules {
            by_rel.entry(r.head.rel.as_str()).or_default().push(canonical(r));
        }
        let keep = |rel: &str| kept(prog, rel);
        let mut first: HashMap<(Vec<String>, Vec<String>), String> = HashMap::new();
        let mut rename: BTreeMap<String, String> = BTreeMap::new();
        for (rel, mut rules) in by_rel {
            if keep(rel) {
                continue;
            }
            rules.sort();
            let types: Vec<String> = prog.decls.get(rel).map(|c| c.iter().map(|(_, t)| t.clone()).collect()).unwrap_or_default();
            match first.get(&(rules.clone(), types.clone())) {
                Some(kept) => {
                    rename.insert(rel.to_owned(), kept.clone());
                }
                None => {
                    first.insert((rules, types), rel.to_owned());
                }
            }
        }
        if rename.is_empty() {
            return any;
        }
        any = true;
        prog.rules.retain(|r| !rename.contains_key(&r.head.rel));
        for rule in &mut prog.rules {
            for l in &mut rule.body {
                if let Literal::Atom(a) | Literal::Agg { atom: a, .. } = l
                    && let Some(to) = rename.get(&a.rel)
                {
                    a.rel = to.clone();
                }
            }
        }
        for (from, to) in rename {
            prog.decls.remove(&from);
            report.merged.push((to, from));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implied_atoms_kind_tests_and_identical_relations_go() {
        let mut prog = super::super::rr::parse(
            r#"
            .decl node(d: number, n: node, kind: symbol, start: number, end: number)
            .decl doc(d: number, path: symbol, module: symbol)
            .input node
            .input doc
            .decl k0(kind: symbol)
            k0("function_definition").
            .decl b1(l: node, h: node)
            .decl b2(l: node, h: node)
            .decl m1(n: node)
            .decl m2(n: node)
            .decl out(n: node, p: symbol)
            .output out
            b1(low, high) :- inside(low, s, k0), node(_, s, "function_definition", _, _), inside(s, high, k0).
            b2(x, y) :- inside(x, z, k0), node(_, z, "function_definition", _, _), inside(z, y, k0).
            m1(n) :- inside(n, v, k0), !b1(n, v).
            m2(q) :- inside(q, w, k0), !b2(q, w).
            out(n, p) :- node(d, n, "call", _, _), !m1(n), !m2(n), doc(d, p, m), doc(d, p, _).
            "#,
        )
        .unwrap();
        let report = simplify(&mut prog);
        // The second `!m1($n)` the merge made. `doc($d, $p, _)` beside `doc($d, $p, $m)` is one row of `doc` (its key
        // `$d`): merged, and `$m`, named once, is `_`.
        assert_eq!(report.implied, 1);
        assert_eq!(report.keyed, 1);
        // The kind test on a member of k0 = {function_definition}, in b1 and b2.
        assert_eq!(report.kind_tests, 2);
        // b2 merges into b1, then m2 (now equal to m1) into m1.
        assert_eq!(report.merged, vec![("b1".to_owned(), "b2".to_owned()), ("m1".to_owned(), "m2".to_owned())]);
        let out: Vec<String> = prog.rules.iter().filter(|r| r.head.rel == "out").map(|r| r.to_string()).collect();
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("!m1($n), doc($d, $p, _)") && !out[0].contains("m2") && out[0].matches("doc(").count() == 1, "{}", out[0]);
        assert!(!prog.decls.contains_key("b2") && !prog.decls.contains_key("m2"));
    }

    #[test]
    fn a_variable_equal_to_a_constant_is_the_constant() {
        let mut prog = super::super::front::parse(
            r#"
            .decl node(d: int, n: node, kind: sym, start: int, end: int)
            .decl text(n: node, s: sym)
            .input node
            .input text
            .decl m(n: node)
            .output m
            m($n) :- node(_, $n, "identifier", _, _), text($n, $v), $v = "get".
            m($n) :- node(_, $n, "identifier", _, _), text($n, $v), "set" = $v.
            .decl t(s: sym)
            .decl doc(d: int, path: sym, module: sym)
            .input doc
            .output t
            t($s) :- doc(_, $s, _), $s = "a.py".
            "#,
        )
        .unwrap();
        let report = simplify(&mut prog);
        assert_eq!(report.constants, 2);
        let rules: Vec<String> = prog.rules.iter().map(|r| r.to_string()).collect();
        assert!(rules[0].contains(r#"text($n, "get")"#) && !rules[0].contains("$v"), "{}", rules[0]);
        assert!(rules[1].contains(r#"text($n, "set")"#) && !rules[1].contains("$v"), "{}", rules[1]);
        // A head variable stays: the head is filled from a text column.
        assert!(rules[2].contains(r#"doc(_, $s, _), $s = "a.py""#), "{}", rules[2]);
    }

    #[test]
    fn singletons_keyed_rows_and_unread_relations() {
        let mut prog = super::super::front::parse(
            r#"
            .decl node(d: int, n: node, kind: sym, start: int, end: int)
            .decl text(n: node, s: sym)
            .input node
            .input text
            .decl named(n: node)
            .output named
            named($n) :- node($d, $n, _, _, _), node(_, $n, "identifier", $s, _), text($n, $v), text($w, $v), $s > 3.
            .decl two(n: node, m: node)
            .output two
            two($n, $m) :- node(_, $n, "call", _, _), node(_, $m, "call", _, _), node(_, $n, _, $a, _), node(_, $m, _, $a, _).
            .decl dead(n: node)
            .decl deader(n: node)
            dead($n) :- node(_, $n, "call", _, _), !deader($n).
            deader($n) :- node(_, $n, "name", _, _).
            "#,
        )
        .unwrap();
        let report = simplify(&mut prog);
        let rule = |head: &str| prog.rules.iter().find(|r| r.head.rel == head).map(|r| r.to_string()).unwrap_or_default();
        // One row of `node` for `$n` (its key), `$d` named once; `text($w, $v)`, `$w` named once, is implied.
        let named = rule("named");
        assert!(named.contains(r#"node(_, $n, "identifier", $s, _)"#) && named.matches("node(").count() == 1, "{named}");
        assert!(named.matches("text(").count() == 1, "{named}");
        // Atoms with different keys stay apart: `$n` and `$m` are two nodes with one start (each its two atoms merged).
        assert_eq!(rule("two").matches("node(").count(), 2, "{}", rule("two"));
        // `dead`, a test of one node that nothing reads, is inlined into its (no) readers; `deader`, read only by it,
        // is then read by nothing.
        assert_eq!(report.unread, vec!["deader".to_owned()]);
        assert!(prog.rules.iter().all(|r| r.head.rel != "dead" && r.head.rel != "deader"));
    }

    #[test]
    fn a_part_meeting_the_rest_only_at_a_document_is_a_relation_of_documents() {
        let mut prog = super::super::front::parse(
            r#"
            .decl node(d: int, n: node, kind: sym, start: int, end: int)
            .decl name(n: node, s: sym)
            .input node
            .input name
            .decl cached(n: node)
            .output cached
            cached($n) :- node(_, $n, "module", _, _), node($d, $n, _, _, _), node($d, $y, _, _, _), name($y, $s), @contains($s, "cache").
            cached($n) :- node(_, $n, "class", _, _), node($e, $n, _, _, _), node($e, $z, _, _, _), name($z, $t), @contains($t, "cache").
            .decl near(n: node)
            .output near
            near($n) :- node($d, $n, "call", _, _), node($d, $y, _, _, _), name($y, $s), name($n, $s).
            "#,
        )
        .unwrap();
        let report = simplify(&mut prog);
        // One relation per rule, the second merged into the first (equal once renamed).
        assert_eq!(report.existentials, vec!["exists_0".to_owned(), "exists_1".to_owned()]);
        assert_eq!(report.merged, vec![("exists_0".to_owned(), "exists_1".to_owned())]);
        let rules = |head: &str| prog.rules.iter().filter(|r| r.head.rel == head).map(|r| r.to_string()).collect::<Vec<_>>();
        assert_eq!(rules("exists_0").len(), 1);
        let exists = &rules("exists_0")[0];
        assert!(exists.contains("name(") && exists.contains("contains") && !exists.contains("module"), "{exists}");
        for cached in rules("cached") {
            assert!(cached.contains("exists_0(") && !cached.contains("name("), "{cached}");
        }
        // `$s` joins the part to the head's node: it stays.
        assert!(rules("near")[0].contains("name($y, $s)"), "{:?}", rules("near"));
    }

    #[test]
    fn two_nodes_of_a_document_compared_by_span_are_within() {
        let mut prog = super::super::front::parse(
            r#"
            .decl node(d: int, n: node, kind: sym, start: int, end: int)
            .input node
            .decl inside(n: node, b: node)
            .output inside
            inside($n, $b) :- node($d, $n, "call", $s, $e), node($d, $b, "try_statement", $bs, $be), $bs <= $s, $be >= $e.
            .decl other(n: node)
            .output other
            other($n) :- node($d, $n, "call", $s, $e), node($d2, $b, "try_statement", $bs, $be), $bs <= $s, $e <= $be.
            "#,
        )
        .unwrap();
        let report = simplify(&mut prog);
        assert_eq!(report.within, 1);
        assert!(prog.inputs.iter().any(|i| i == "span_within") && prog.decls.contains_key("span_within"));
        let rules = |head: &str| prog.rules.iter().filter(|r| r.head.rel == head).map(|r| r.to_string()).collect::<Vec<_>>();
        let inside = &rules("inside")[0];
        assert!(inside.contains("span_within($n, $b)") && !inside.contains("<="), "{inside}");
        // Two documents: not a containment of one document's nodes.
        assert!(!rules("other")[0].contains("within"), "{:?}", rules("other"));
    }

    #[test]
    fn a_test_of_one_node_read_positively_is_inlined() {
        let mut prog = super::super::front::parse(
            r#"
            .decl node(d: int, n: node, kind: sym, start: int, end: int)
            .decl text(n: node, s: sym)
            .input node
            .input text
            .decl k(n: node)
            .decl not_k(n: node)
            .decl named(n: node)
            .decl read_negated(n: node)
            .decl j(n: node)
            .decl not_both(n: node)
            .decl out(n: node)
            .output out
            k($n) :- text($n, "x").
            j($n) :- text($n, "z").
            not_both($n) :- node(_, $n, _, _, _), !k($n).
            not_both($n) :- node(_, $n, _, _, _), !j($n).
            not_k($m) :- node(_, $m, _, _, _), !k($m).
            named($n) :- node(_, $n, "identifier", _, _), not_k($n).
            read_negated($n) :- node(_, $n, _, _, _), !k($n), text($n, "y").
            out($a) :- node(_, $a, "call", _, _), named($a), !read_negated($a), not_both($a).
            "#,
        )
        .unwrap();
        let report = simplify(&mut prog);
        // `not_k`, then `named` (now `node(...), !k($n)` over `$n` alone); `read_negated` is read negated, and `k`
        // has no node test.
        // `not_both` is first one rule, `!not_both_all($n)` with `not_both_all($n) :- k($n), j($n)` (De Morgan).
        assert_eq!(report.inlined, vec!["named".to_owned(), "not_both".to_owned(), "not_k".to_owned()]);
        let rules: Vec<String> = prog.rules.iter().map(|r| r.to_string()).collect();
        let out = rules.iter().find(|r| r.starts_with("out(")).unwrap();
        assert!(out.contains("!k($a)") && out.contains("!not_both_all($a)") && !out.contains("named") && !out.contains("not_k"), "{out}");
        assert!(rules.iter().any(|r| r == "not_both_all($n) :- k($n), j($n)"), "{rules:?}");
        assert!(!prog.decls.contains_key("not_k") && !prog.decls.contains_key("named") && prog.decls.contains_key("read_negated"));
    }
}
