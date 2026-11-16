//! Exact maintenance without delete-and-rederive, a session's default (`RERAIL_MAINTAIN=dred` compares with
//! delete-and-rederive): counting, held as support sets, because the engine holds sets.
//!
//! Each rule of a non-recursive stratum gets its own relation of derivations: the head's columns, then every other
//! variable its positive atoms bind (a wildcard there becomes a variable). A row of it has exactly one derivation,
//! so its deletes are exact and it needs no rederivation (`maintain` reads `__sup` relations so). The head is their
//! projection, and a head row is gone when its last support is: one probe, a count reaching zero (`maintain`: a
//! head row that keeps a support is neither deleted nor derived again).
//!
//! ```text
//! h(x) :- p(x, y), q(y, _).      becomes    h__sup0(x, y, w0) :- p(x, y), q(y, w0).
//!                                           h(x) :- h__sup0(x, _, _).
//! ```
//!
//! A recursive stratum keeps its rules and is maintained by delete-and-rederive (`maintain`): counting over cycles
//! needs a witness of each row's level (decision 0090's levels spike), not built yet. A rule with an aggregate,
//! `nearest`, a matcher, or a variable no positive atom binds keeps its form; its stratum is maintained as before.

use super::lower;
use crate::code_rule_dsl::logic::plan::fields::{Fields, FieldsData};
use crate::code_rule_dsl::logic::program_image::analyses;
use crate::code_rule_dsl::logic::program_image::manager::{Cx, Pipeline, RuleOut, RulePass, Select, Step};
use crate::code_rule_dsl::logic::program_image::{Lit, RuleRef, T, TermRef, Version};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Whether a session maintains by counting: the default. `RERAIL_MAINTAIN=dred` asks for delete-and-rederive, to
/// compare with.
pub(in crate::code_rule_dsl) fn counting() -> bool {
    std::env::var("RERAIL_MAINTAIN").map_or(true, |v| v != "dred")
}

/// A support relation of `rel`'s: a row of it is one derivation.
pub(in crate::code_rule_dsl) fn is_support(rel: &str) -> bool {
    rel.contains("__sup")
}

/// `cross` with each rule of a non-recursive stratum through its own support relation; with how many rules were
/// so, and how many kept their form. A rule pass over `cross`'s version (`Supports`, decision 0281): a derivation,
/// each rule that keeps its form kept by its handle, in order, and each made one written from it. A program the
/// image has no fields for (its rules alone), or whose components are not found, keeps its form.
pub(in crate::code_rule_dsl) fn supports(cross: &lower::Lowered) -> (lower::Lowered, usize, usize) {
    let Ok(p) = cross.placed() else { return (cross.clone(), 0, 0) };
    if !p.whole {
        return (cross.clone(), 0, 0);
    }
    let mut pass = Supports { domains: &cross.domains, together: p.together(), fields: None, recursive: BTreeSet::new(), n: BTreeMap::new(), made: BTreeMap::new(), through: 0, kept: 0, skipped: false };
    let mut pipe = Pipeline::new("supports", p.v.clone());
    pipe.rules(&mut pass);
    let made = pipe.finish().ok().map(|(v, _)| v);
    if pass.skipped {
        return (cross.clone(), 0, 0);
    }
    let Some(made) = super::made_from(true, made, "the supports") else { return (cross.clone(), 0, 0) };
    let mut out = cross.clone();
    out.version = made;
    for (name, d) in pass.made {
        out.domains.entry(name).or_insert(d);
    }
    (out, pass.through, pass.kept)
}

/// `supports` as a rule pass: which strata are recursive is found once (`begin`); each rule of a non-recursive one
/// is written as its support relation and the head's projection of it, each recording the rule as its source.
struct Supports<'d> {
    /// The domains of `cross`'s relations.
    domains: &'d lower::Domains,
    /// The sets a maintained program runs as one component.
    together: Vec<BTreeSet<String>>,
    fields: Option<Arc<FieldsData>>,
    recursive: BTreeSet<String>,
    /// How many support relations each head has.
    n: BTreeMap<String, usize>,
    /// Each support relation's columns' domains.
    made: lower::Domains,
    through: usize,
    kept: usize,
    /// Whether the components were not found, so every rule keeps its form.
    skipped: bool,
}

impl RulePass for Supports<'_> {
    const NAME: &'static str = "supports";
    fn begin(&mut self, cx: &Cx<'_>) -> Result<Select, String> {
        let v = cx.v();
        let f = cx.get::<FieldsData>()?;
        // The version's components where nothing runs together (most programs), else its own with the sets merged.
        let sccs = if self.together.is_empty() { cx.get::<analyses::Components>().map(|c| c.0.clone()) } else { analyses::components_by_name(v, &f.about, &self.together).map_err(|e| e.to_string()) };
        let Ok(sccs) = sccs else {
            self.skipped = true;
            cx.decide("skipped", || "its components are not found: every rule keeps its form".into());
            return Ok(Select::Rules(Default::default()));
        };
        // A stratum of one relation is recursive when a rule of it reads it: found in one pass over the rules (a pass
        // per stratum was quadratic, thousands of strata by tens of thousands of rules).
        let reads_itself: BTreeSet<&str> = v.rules().filter(|r| r.reads().any(|a| a.rel() == r.head().rel())).map(|r| v.rel_name(r.head().rel())).collect();
        self.recursive = sccs.iter().filter(|scc| scc.len() > 1 || scc.iter().any(|r| reads_itself.contains(r.as_str()))).flatten().cloned().collect();
        self.fields = Some(f);
        Ok(Select::All)
    }
    fn rule(&mut self, cx: &Cx<'_>, rule: RuleRef<'_>, out: &mut RuleOut<'_, '_>) -> Result<Step, String> {
        let v: &Version = cx.v();
        let f = Fields::over(v, self.fields.as_ref().expect("begun"));
        let h = v.rel_name(rule.head().rel());
        let eligible = !self.recursive.contains(h) && !f.gate(h) && f.native(h).is_none() && !f.lattice(h) && !f.nullary(h);
        let Some(s) = eligible.then(|| support(&f, v, rule, self.domains)).flatten() else {
            self.kept += 1;
            return Ok(Step::Keep);
        };
        let i = self.n.entry(h.to_owned()).or_default();
        let name = format!("{h}__sup{i}");
        *i += 1;
        let cols: Vec<(&str, &str)> = s.decl.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
        out.builder().declare(&name, &cols);
        // Every column's domain: the head's, then each bound variable's in the rule (a kind's is no atom, decision
        // 0170).
        let d = s.domains(self.domains.get(h).map_or(&[][..], Vec::as_slice));
        self.made.entry(name.clone()).or_insert(d);
        s.write(out, v, rule, &name)?;
        self.through += 1;
        Ok(Step::Written)
    }
}

/// A rule of a head as its support relation (`support`): the relation's columns, the fresh variable each wildcard of
/// a positive atom becomes (by the literal and argument), and the variables past the head's, in order.
struct Support {
    decl: Vec<(String, String)>,
    wild: BTreeMap<(usize, usize), String>,
    extra: Vec<String>,
    /// Each column's variable (`None` for a constant of the head), and each variable's domain from the first
    /// positive atom column with one that binds it.
    vars: Vec<Option<String>>,
    domain_of: BTreeMap<String, lower::Domain>,
}

impl Support {
    /// The domains of the relation's columns past `known` (its head's), by `lower::column_domain` (decision 0170:
    /// given the head's domains alone, every symbol column after them was taken for an atom, a kind's too).
    fn domains(&self, known: &[lower::Domain]) -> Vec<lower::Domain> {
        self.decl
            .iter()
            .enumerate()
            .map(|(c, (_, t))| lower::column_domain(c, t, known, self.vars.get(c).and_then(Option::as_ref).and_then(|x| self.domain_of.get(x).copied())))
            .collect()
    }
}

/// A term's variables (a call's arguments' too), by name.
fn term_vars(v: &Version, r: RuleRef<'_>, t: TermRef<'_>, out: &mut Vec<String>) {
    match t {
        TermRef::Var(s) => out.push(v.text(r.var(s)).to_owned()),
        TermRef::Call(_, args) => args.for_each(|a| term_vars(v, r, a, out)),
        _ => {}
    }
}

/// `rule` (of `v`, whose fields are `f`) as a support relation: `None` where it keeps its form (a head that is not
/// of variables and constants, an aggregate, `nearest`, a matcher, or a variable no positive atom binds).
fn support(f: &Fields<'_>, v: &Version, rule: RuleRef<'_>, domains: &lower::Domains) -> Option<Support> {
    let head = rule.head();
    let hdecl = f.columns(v.rel_name(head.rel()))?;
    if !head.args().all(|t| matches!(t, TermRef::Var(_) | TermRef::Int(_) | TermRef::Str(_))) {
        return None;
    }
    // Each variable's type, from the first positive atom column binding it; wildcards there become variables.
    let mut types: BTreeMap<String, String> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut wild = BTreeMap::new();
    let mut domain_of: BTreeMap<String, lower::Domain> = BTreeMap::new();
    let mut fresh = 0;
    let mut used = Vec::new();
    for (i, l) in rule.body().enumerate() {
        match l.view() {
            Lit::Atom(a) if !a.neg() => {
                let decl = f.columns(v.rel_name(a.rel()))?;
                let doms = domains.get(v.rel_name(a.rel()));
                for (c, t) in a.args().enumerate() {
                    let name = match t {
                        TermRef::Wild => {
                            fresh += 1;
                            let w = format!("rr_w{fresh}");
                            wild.insert((i, c), w.clone());
                            w
                        }
                        TermRef::Var(s) => v.text(rule.var(s)).to_owned(),
                        t => {
                            term_vars(v, rule, t, &mut used);
                            continue;
                        }
                    };
                    if let Some(&d) = doms.and_then(|ds| ds.get(c)) {
                        domain_of.entry(name.clone()).or_insert(d);
                    }
                    if !types.contains_key(&name) {
                        types.insert(name.clone(), decl.get(c)?.1.to_owned());
                        order.push(name);
                    }
                }
            }
            Lit::Atom(a) => a.args().for_each(|t| term_vars(v, rule, t, &mut used)),
            Lit::Cmp { l, r, .. } => {
                term_vars(v, rule, l, &mut used);
                term_vars(v, rule, r, &mut used);
            }
            _ => return None,
        }
    }
    // Every variable is bound by a positive atom (a comparison's `$x = e` would bind one without a type).
    if !used.iter().all(|x| types.contains_key(x)) {
        return None;
    }
    let mut head_vars = Vec::new();
    head.args().for_each(|t| term_vars(v, rule, t, &mut head_vars));
    let extra: Vec<String> = order.into_iter().filter(|x| !head_vars.contains(x)).collect();
    let mut decl: Vec<(String, String)> = hdecl.iter().map(|(c, t)| ((*c).to_owned(), (*t).to_owned())).collect();
    decl.extend(extra.iter().map(|x| (x.clone(), types[x].clone())));
    let vars = head.args().map(|t| match t { TermRef::Var(s) => Some(v.text(rule.var(s)).to_owned()), _ => None }).chain(extra.iter().cloned().map(Some)).collect();
    Some(Support { decl, wild, extra, vars, domain_of })
}

/// A term of `v`'s rule `r`, to write into a builder under `v`'s ids.
fn t_of<'v>(v: &'v Version, r: RuleRef<'v>, t: TermRef<'v>) -> T<'v> {
    match t {
        TermRef::Var(s) => T::VarAtom(r.var(s)),
        TermRef::Wild => T::Wild,
        TermRef::Top => T::Top,
        TermRef::Int(i) => T::Int(i),
        TermRef::Str(a) => T::StrAtom(a),
        TermRef::Call(f, args) => T::Call(v.text(f), args.map(|a| t_of(v, r, a)).collect()),
    }
}

impl Support {
    /// Writes `rule` through support relation `name` (`rule`'s replacement, in a derivation of `v`):
    /// `name(head, extra) :- body`, each positive atom's wildcard its variable, then `head :- name(head, _, …)`.
    fn write<'v>(&self, out: &mut RuleOut<'_, '_>, v: &'v Version, rule: RuleRef<'v>, name: &str) -> Result<(), String> {
        let head: Vec<T<'v>> = rule.head().args().map(|t| t_of(v, rule, t)).collect();
        let mut args = head.clone();
        args.extend(self.extra.iter().map(|x| T::Var(x.as_str())));
        let mut rb = out.rule().head(name, &args);
        for (i, l) in rule.body().enumerate() {
            rb = match l.view() {
                Lit::Atom(a) if !a.neg() => {
                    let terms: Vec<T<'_>> = a.args().enumerate().map(|(c, t)| match self.wild.get(&(i, c)) {
                        Some(w) => T::Var(w.as_str()),
                        None => t_of(v, rule, t),
                    }).collect();
                    rb.atom_with(a.rel(), &terms, false, a.gated())
                }
                _ => rb.lit(v, rule, l),
            };
        }
        rb.finish()?;
        let mut over = head;
        over.extend(self.extra.iter().map(|_| T::Wild));
        out.rule().head_from(v, rule, rule.head()).atom(name, &over).finish()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The module's example, a negation and a recursive stratum: each non-recursive rule through its support, made on
    /// the version (its tree read only here, to check it).
    #[test]
    fn each_non_recursive_rule_is_derived_through_its_support() {
        let prog = crate::code_rule_dsl::logic::rr::parse(
            ".decl p(x: number, y: number)\n.decl q(y: number, z: number)\n.decl h(x: number)\n.decl g(x: number)\n.decl t(x: number, y: number)\n\
             h(x) :- p(x, y), q(y, _).\ng(x) :- p(x, _), !q(x, _).\nt(x, y) :- p(x, y).\nt(x, z) :- t(x, y), p(y, z).\n",
        )
        .unwrap();
        let cross = lower::Given { program: prog, domains: Default::default(), ops: [0; 3], dialect: crate::code_rule_dsl::language::GrammarDialect::Python }.image().unwrap();
        let (out, through, kept) = supports(&cross);
        assert_eq!((through, kept), (2, 2));
        let v = out.version.clone();
        let lists = crate::code_rule_dsl::logic::front::Lists::default();
        let tree = crate::code_rule_dsl::logic::plan::body::Placed::whole(v).program(&lists).unwrap();
        let rules: Vec<String> = tree.rules.iter().map(|r| r.to_string()).collect();
        assert_eq!(
            rules,
            [
                "h__sup0($x, $y, $rr_w1) :- p($x, $y), q($y, $rr_w1)",
                "h($x) :- h__sup0($x, _, _)",
                "g__sup0($x, $rr_w1) :- p($x, $rr_w1), !q($x, _)",
                "g($x) :- g__sup0($x, _)",
                "t($x, $y) :- p($x, $y)",
                "t($x, $z) :- t($x, $y), p($y, $z)",
            ]
        );
        let cols = |rel: &str| tree.decls[rel].iter().map(|(c, t)| format!("{c}:{t}")).collect::<Vec<_>>().join(" ");
        assert_eq!((cols("h__sup0"), cols("g__sup0")), ("x:int y:int rr_w1:int".to_owned(), "x:int rr_w1:int".to_owned()));
    }

    /// A support relation's columns past its head's take their domains from its rule: a kind bound in the body is
    /// a kind id, not an atom (`lower::atom_columns`, decision 0170). Given the head's domains alone, every symbol
    /// column after them was taken for an atom; its frame named kind ids as atoms, and the next run on the store read
    /// SQL inside its waves to place them (no such atom), or a forked process mapped them.
    #[test]
    fn a_support_relation_keeps_its_kind_columns_out_of_its_atoms() {
        let prog = crate::code_rule_dsl::logic::front::parse(
            r#"
            .decl node(d: int, n: node, kind: sym, start: int, end: int)
            .decl text(n: node, s: sym)
            .decl child(p: node, n: node)
            .input node
            .input text
            .input child
            .decl h(n: node)
            .output h
            h($n) :- child($p, $n), node(_, $p, $k, _, _), text($p, $s).
            "#,
        )
        .unwrap();
        let domains = lower::domains(&prog).unwrap();
        let cross = lower::Given { program: prog, domains, ops: [0; 3], dialect: crate::code_rule_dsl::language::GrammarDialect::Python }.image().unwrap();
        let (out, through, _) = supports(&cross);
        assert_eq!(through, 1);
        let v = out.version.clone();
        let lists = crate::code_rule_dsl::logic::front::Lists::default();
        let tree = crate::code_rule_dsl::logic::plan::body::Placed::whole(v).program(&lists).unwrap();
        let sup = tree.decls.keys().find(|r| is_support(r)).expect("a support relation").to_string();
        let decl = &tree.decls[sup.as_str()];
        let types: Vec<&str> = decl.iter().map(|(_, t)| t.as_str()).collect();
        let atoms = lower::atom_columns(&types, &out.domains, &sup);
        let col = |name: &str| decl.iter().position(|(c, _)| c.as_str().trim_start_matches('$') == name).unwrap_or_else(|| panic!("{name} in {decl:?}"));
        assert!(!atoms[col("k")], "a kind is no atom: {decl:?} {atoms:?}");
        assert!(atoms[col("s")], "a text is: {decl:?} {atoms:?}");
    }
}
