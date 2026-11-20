//! What a program's `text(v, …)` atoms read (`super::text_kind_demand`): the kinds a node variable can have, to a
//! fixpoint over the program's derived relations (`NodeKinds`), and what else narrows the nodes it can be: the field
//! it is in, a matcher capture's constraints, the constant it is compared with, its parent (`TextReads`).

use super::{TextDemand, TextNodes, leaf_field, short_constant};
use crate::code_rule_dsl::logic::front::ast::{Atom, Literal, Program, Rule, Term};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};

/// The kinds a node can have and the fields of its parent it can be in.
type Constraints = (BTreeSet<String>, BTreeSet<String>);
/// Kinds by number (`NodeKinds::intern`), sorted; `None` is any kind.
type KindSet = Option<Vec<u32>>;
/// The program's `ast` relations, by relation, as `matchers::lower` gives them: relation, matcher, capture names.
type Matchers = [(String, String, Vec<String>)];
/// The `all` a matcher's object sits in, and whether that one is an `inside`'s.
type Outer<'a> = Option<(&'a serde_json::Map<String, serde_json::Value>, bool)>;

/// A kind as the program holds it (`python:call`) or as a grammar names it (`call`).
fn bare(k: &str) -> String {
    k.rsplit(':').next().unwrap_or(k).to_owned()
}

/// A matcher's root kind: its rule's own `kind`, or one of its top-level `all`'s.
fn root_kind(text: &str) -> Option<String> {
    let rule = serde_json::from_str::<serde_json::Value>(text).ok()?.get("rule")?.clone();
    if let Some(k) = rule.get("kind").and_then(|k| k.as_str()) {
        return Some(k.to_owned());
    }
    rule.get("all")?.as_array()?.iter().find_map(|r| r.get("kind").and_then(|k| k.as_str()).map(str::to_owned))
}

/// Whether `r` is `node`'s projection `kind(n, k) :- node(_, n, k, _, _)` (the lowering's, `rr::lower` `projections`,
/// and the data-layer prelude's): `kind` itself, not a rule of the program's.
fn is_kind_projection(r: &Rule) -> bool {
    r.head.rel == "kind"
        && matches!((r.head.args.as_ref(), r.body.as_ref()),
            ([Term::Var(n), Term::Var(k)], [Literal::Atom(a)])
                if !a.neg && a.rel == "node" && matches!(a.args.as_ref(), [_, Term::Var(an), Term::Var(ak), _, _] if an == n && ak == k))
}

/// What fixes a variable's kinds where an atom reads it.
enum Fix {
    /// A derived relation's column.
    Column(u32, usize),
    /// Kinds known whatever the round (`kind(v, "K")`, a matcher's root), or any (`None`).
    Known(KindSet),
}

/// A rule head's column (its relation's number and the column), and what fixes the head variable there.
type Head = (u32, usize, Vec<Fix>);

fn intersect(a: &[u32], b: &[u32]) -> Vec<u32> {
    a.iter().copied().filter(|k| b.binary_search(k).is_ok()).collect()
}

fn union(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut u: Vec<u32> = a.iter().chain(b).copied().collect();
    u.sort_unstable();
    u.dedup();
    u
}

/// A node's kinds followed through the relations' columns to a fixpoint, on numbers: each kind name interned
/// (`names`), a set of kinds a sorted vector of them, each derived relation's columns a vector by its number, and each
/// rule head variable's constraints found once (`heads`). Looked up by an owned `(String, usize)` key and cloned as
/// sets of names at every step of every round, the fixpoint was most of `text_kind_demand`.
struct NodeKinds<'p> {
    /// Each kind's number, and each number's kind.
    names: RefCell<(HashMap<String, u32>, Vec<String>)>,
    /// Each matcher relation's root kind.
    roots: HashMap<&'p str, KindSet>,
    /// Each derived relation's number.
    derived: HashMap<&'p str, u32>,
    /// A base relation's name the program gives rules of its own: a relation like any other there, which narrows
    /// nothing by the base relation's meaning. `kind(n, k) :- node(_, n, k, _, _)` is `kind` itself
    /// (`is_kind_projection`).
    redefined: BTreeSet<&'p str>,
    /// Each derived relation's columns' kinds, by its number.
    columns: Vec<Vec<KindSet>>,
}

impl<'p> NodeKinds<'p> {
    fn of(program: &'p Program, matchers: &'p Matchers) -> Self {
        let mut derived: Vec<&str> = program.rules.iter().map(|r| r.head.rel.as_str()).collect();
        derived.sort_unstable();
        derived.dedup();
        let redefined = program.rules.iter().filter(|r| !is_kind_projection(r)).map(|r| r.head.rel.as_str()).filter(|r| ["kind", "node", "field", "child"].contains(r)).collect();
        let mut kinds = NodeKinds { names: RefCell::default(), roots: HashMap::new(), derived: derived.iter().enumerate().map(|(i, r)| (*r, i as u32)).collect(), redefined, columns: Vec::new() };
        let roots = matchers.iter().map(|(rel, text, _)| (rel.as_str(), root_kind(text).map(|k| vec![kinds.intern(bare(&k))]))).collect();
        kinds.roots = roots;
        let columns = kinds.solve(program);
        kinds.columns = columns;
        kinds
    }

    /// Kind `k`'s number.
    fn intern(&self, k: String) -> u32 {
        let mut t = self.names.borrow_mut();
        if let Some(&n) = t.0.get(&k) {
            return n;
        }
        let n = t.1.len() as u32;
        t.1.push(k.clone());
        t.0.insert(k, n);
        n
    }

    /// What fixes `v`'s kinds in `body`: each column of a positive atom that reads it.
    fn fixers(&self, body: &[Literal], v: &str) -> Vec<Fix> {
        let mut out = Vec::new();
        for a in body.iter().filter_map(Literal::atom).filter(|a| !a.neg) {
            for (c, _) in a.args.iter().enumerate().filter(|(_, t)| matches!(t, Term::Var(x) if x == v)) {
                out.push(self.fix(a, c));
            }
        }
        out
    }

    /// What column `c` of atom `a` fixes. The base relations narrow by their meaning only where the program gives no
    /// rules of its own to one of that name (`redefined`): there it is a relation like any other (its columns).
    fn fix(&self, a: &Atom, c: usize) -> Fix {
        let base = !self.redefined.contains(a.rel.as_str());
        match a.rel.as_str() {
            "kind" if base => Fix::Known(self.constant(c == 0, a.args.get(1))),
            // T's base `node(d, n, kind, …)` (a program that declares its base relations itself): `n` of `kind`.
            "node" if base => Fix::Known(self.constant(c == 1, a.args.get(2))),
            rel => self.column(rel, c),
        }
    }

    /// The kind constant `t` names, where the column read is the atom's node (`at`); else any kind.
    fn constant(&self, at: bool, t: Option<&Term>) -> KindSet {
        match t {
            Some(Term::Str(k)) if at => Some(vec![self.intern(bare(k))]),
            _ => None,
        }
    }

    /// Column `c` of relation `rel`: a derived relation's, a matcher's root, or any other's (any kind).
    fn column(&self, rel: &str, c: usize) -> Fix {
        match self.derived.get(rel) {
            Some(&d) => Fix::Column(d, c),
            None if c == 0 && let Some(root) = self.roots.get(rel) => Fix::Known(root.clone()),
            None => Fix::Known(None),
        }
    }

    /// A variable's kinds under `columns`: the intersection of what fixes it (any, when nothing does).
    fn fold(columns: &[Vec<KindSet>], fixes: &[Fix]) -> KindSet {
        let mut kinds: KindSet = None;
        for f in fixes {
            let fixed: Option<&[u32]> = match f {
                Fix::Column(d, c) => match columns[*d as usize].get(*c) {
                    Some(k) => k.as_deref(),
                    None => Some(&[]),
                },
                Fix::Known(k) => k.as_deref(),
            };
            kinds = match (kinds, fixed) {
                (None, k) => k.map(<[u32]>::to_vec),
                (k, None) => k,
                (Some(a), Some(b)) => Some(intersect(&a, b)),
            };
        }
        kinds
    }

    /// Each derived relation's columns, as wide as its widest head, of no kind yet; each rule head variable's fixers.
    fn heads(&self, program: &Program) -> (Vec<Vec<KindSet>>, Vec<Head>) {
        let mut columns: Vec<Vec<KindSet>> = vec![Vec::new(); self.derived.len()];
        let mut heads = Vec::new();
        for rule in &program.rules {
            let h = self.derived[rule.head.rel.as_str()];
            let width = &mut columns[h as usize];
            if width.len() < rule.head.args.len() {
                width.resize(rule.head.args.len(), Some(Vec::new()));
            }
            for (c, t) in rule.head.args.iter().enumerate() {
                let Term::Var(v) = t else { continue };
                heads.push((h, c, self.fixers(&rule.body, v)));
            }
        }
        (columns, heads)
    }

    /// The derived relations' columns to a fixpoint: each the union, over its rules, of what fixes their head variable.
    fn solve(&self, program: &Program) -> Vec<Vec<KindSet>> {
        let (mut columns, heads) = self.heads(program);
        while Self::round(&mut columns, &heads) {}
        columns
    }

    /// One round of the fixpoint: each head column joined with what its rule's body fixes now. Whether one changed.
    fn round(columns: &mut [Vec<KindSet>], heads: &[Head]) -> bool {
        let mut changed = false;
        for (h, c, fixes) in heads {
            let kinds = Self::fold(columns, fixes);
            let entry = &mut columns[*h as usize][*c];
            let joined = match (&*entry, kinds) {
                (None, _) | (_, None) => None,
                (Some(a), Some(b)) => Some(union(a, &b)),
            };
            if *entry != joined {
                *entry = joined;
                changed = true;
            }
        }
        changed
    }

    /// `v`'s kinds in `body`, by name; `None` any.
    fn of_var(&self, body: &[Literal], v: &str) -> Option<BTreeSet<String>> {
        let fixes = self.fixers(body, v);
        let names = self.names.borrow();
        Self::fold(&self.columns, &fixes).map(|ks| ks.into_iter().map(|k| names.1[k as usize].clone()).collect())
    }
}

/// `(head, v)` where literal `l` of a rule of `head` reads `v`'s long text by `@rr_node_text(v)`: its twin
/// (`long_text_twins`).
fn twin_read<'p>(head: &'p str, l: &'p Literal) -> Option<(&'p str, &'p str)> {
    match l {
        Literal::Cmp { r: Term::Call(f, args), .. } if f == "rr_node_text" => match args.first() {
            Some(Term::Var(x)) => Some((head, x.as_str())),
            _ => None,
        },
        _ => None,
    }
}

/// The constraints matcher `text` puts on the node its capture `name` matches; none where it puts none.
fn capture_constraints(text: &str, name: &str) -> Constraints {
    let mut out = None;
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(text) {
        sites(&json, &format!("${name}"), false, None, &mut out);
    }
    out.unwrap_or_default()
}

/// The constraints on the node `$name` matches, read where its pattern sits, merged over its sites into `out`.
fn sites(v: &serde_json::Value, name: &str, ancestor: bool, outer: Outer<'_>, out: &mut Option<Constraints>) {
    match v {
        serde_json::Value::Object(o) => sites_in(o, name, ancestor, outer, out),
        serde_json::Value::Array(a) => a.iter().for_each(|x| sites(x, name, false, None, out)),
        _ => {}
    }
}

/// `sites` in object `o`: its own pattern's, and those below it. An `inside`'s `field` is where the node it holds
/// sits in that ancestor, not the ancestor's own.
fn sites_in(o: &serde_json::Map<String, serde_json::Value>, name: &str, ancestor: bool, outer: Outer<'_>, out: &mut Option<Constraints>) {
    if o.get("pattern").and_then(|p| p.as_str()) == Some(name) {
        merge(out, site(o, ancestor, outer));
    }
    for (key, x) in o {
        if key == "all" {
            for m in x.as_array().into_iter().flatten() {
                sites(m, name, false, Some((o, ancestor)), out);
            }
        } else {
            sites(x, name, key == "inside", None, out);
        }
    }
}

/// The constraints at one site of a pattern, object `o`: its own and its `all`'s, and those of the `all` it is in
/// (`outer`) and that one's siblings.
fn site(o: &serde_json::Map<String, serde_json::Value>, ancestor: bool, outer: Outer<'_>) -> Constraints {
    let mut got = Constraints::default();
    for (m, ancestor) in [Some((o, ancestor)), outer].into_iter().flatten() {
        own(m, ancestor, &mut got);
        for sib in m.get("all").and_then(|a| a.as_array()).into_iter().flatten().filter_map(|x| x.as_object()) {
            own(sib, false, &mut got);
        }
    }
    got
}

/// Object `m`'s own `kind`, and its `field` unless it is an `inside`'s (`ancestor`).
fn own(m: &serde_json::Map<String, serde_json::Value>, ancestor: bool, (kinds, fields): &mut Constraints) {
    kinds.extend(m.get("kind").and_then(|k| k.as_str()).map(str::to_owned));
    if !ancestor {
        fields.extend(m.get("field").and_then(|k| k.as_str()).map(str::to_owned));
    }
}

/// Two sites of one capture: either constrains it, or none does.
fn merge(out: &mut Option<Constraints>, got: Constraints) {
    *out = Some(match out.take() {
        None => got,
        Some(prev) if unconstrained(&prev) || unconstrained(&got) => Default::default(),
        Some((k, f)) => (k.union(&got.0).cloned().collect(), f.union(&got.1).cloned().collect()),
    });
}

fn unconstrained((kinds, fields): &Constraints) -> bool {
    kinds.is_empty() && fields.is_empty()
}

/// What narrows the nodes a program's `text(v, …)` atoms read, and the demand each makes (`super::text_kind_demand`).
pub(super) struct TextReads<'p> {
    program: &'p Program,
    matchers: &'p Matchers,
    kinds: NodeKinds<'p>,
    /// Each (rule head, variable) a rule reads the long text of by `@rr_node_text(v)` (`twin_read`).
    twins: HashSet<(&'p str, &'p str)>,
}

impl<'p> TextReads<'p> {
    pub(super) fn of(program: &'p Program, matchers: &'p Matchers) -> Self {
        let twins = program.rules.iter().flat_map(|r| r.body.iter().filter_map(move |l| twin_read(r.head.rel.as_str(), l))).collect();
        TextReads { program, matchers, kinds: NodeKinds::of(program, matchers), twins }
    }

    /// The demand of `text`, literal `at` of `rule`, added to `d`. A negated read demands as a positive one does: a
    /// node whose row is not made has no text to test, and its negation held whatever its text.
    pub(super) fn demand(&self, d: &mut TextDemand, at: (usize, usize), rule: &Rule, text: &Atom) {
        let Some(Term::Var(v)) = text.args.first() else {
            d.read = None;
            d.wide.get_or_insert_with(|| rule.head.rel.to_string());
            return;
        };
        let fixed = self.kinds.of_var(&rule.body, v);
        if let Some(nodes) = &mut d.read
            && !self.narrow(nodes, &rule.body, v, fixed.as_ref(), text)
        {
            d.wide = Some(format!("{}, reading text({v}, …)", rule.head.rel));
            d.read = None;
        }
        self.long(d, at, rule, v, fixed, text);
    }

    /// Adds the nodes `v` can be to `nodes`: those of its kinds where they are `fixed`. `false` where nothing narrows
    /// it, and every node's text is read.
    fn narrow(&self, nodes: &mut TextNodes, body: &[Literal], v: &str, fixed: Option<&BTreeSet<String>>, text: &Atom) -> bool {
        if let Some(k) = fixed {
            nodes.kinds.extend(k.iter().cloned());
            return true;
        }
        // A span's text is a span's (`text_spans`), not a node's.
        self.span(body, v) || self.narrow_unfixed(nodes, body, v, text)
    }

    /// `narrow` of a node whose kinds no body fixes: by the fields it is in, or a matcher capture's kinds or field.
    fn narrow_unfixed(&self, nodes: &mut TextNodes, body: &[Literal], v: &str, text: &Atom) -> bool {
        match (self.fields_of(body, v), self.captured(body, v)) {
            (f, _) if !f.is_empty() => nodes.fields.extend(f),
            (_, (k, _)) if !k.is_empty() => nodes.kinds.extend(k),
            (_, (_, f)) if !f.is_empty() => nodes.fields.extend(f),
            _ => return self.narrow_by_value_or_parent(nodes, body, v, text),
        }
        true
    }

    /// `narrow_unfixed` of a node in no field and no capture: by the constant its text is compared with, or its parent.
    fn narrow_by_value_or_parent(&self, nodes: &mut TextNodes, body: &[Literal], v: &str, text: &Atom) -> bool {
        // Compared with a constant within the bound: only a node whose text is that constant.
        if short_constant(text) {
            if let Some(Term::Str(c)) = text.args.get(1) {
                nodes.values.insert(c.to_string());
            }
            return true;
        }
        // A child of a node of known kinds: only such children.
        let Some((kinds, fields)) = self.parents_of(body, v) else { return false };
        nodes.parents.extend(kinds);
        nodes.parent_fields.extend(fields);
        true
    }

    /// The text past the inline bound that `text` of `v` reads. None for a negation of a constant within the bound: a
    /// longer text is not that constant, row or no row. `v`'s kinds where they are `fixed`; else `long_unfixed`'s.
    fn long(&self, d: &mut TextDemand, at: (usize, usize), rule: &Rule, v: &str, fixed: Option<BTreeSet<String>>, text: &Atom) {
        match fixed {
            _ if text.neg && short_constant(text) => {}
            Some(k) => d.long.extend(k),
            None => self.long_unfixed(d, at, rule, v, text),
        }
    }

    /// `long` of a node whose kinds no body fixes: none for a token's text, a constant within the bound, or a rule
    /// whose twin reads `v`'s; else every node's: positive, by a twin (`any`), negated, made at any length
    /// (`negated_any`), since a twin cannot stand in for a negation.
    fn long_unfixed(&self, d: &mut TextDemand, at: (usize, usize), rule: &Rule, v: &str, text: &Atom) {
        if self.within_bound(&rule.body, v, text) {
            return;
        }
        if text.neg {
            d.negated_any += 1;
        } else if !self.twinned(rule.head.rel.as_str(), v) {
            d.any.push(at);
        }
    }

    /// Whether `text` of `v`, a node whose kinds no body fixes, is within the bound: a token's (`leaf_field`), or a
    /// constant within it.
    fn within_bound(&self, body: &[Literal], v: &str, text: &Atom) -> bool {
        !self.kinds.redefined.contains("field") && leaf_field(body, v) || short_constant(text)
    }

    /// Whether a rule of `head` reads `v`'s long text by `@rr_node_text(v)`: its twin (`long_text_twins`).
    fn twinned(&self, head: &str, v: &str) -> bool {
        self.twins.contains(&(head, v))
    }

    /// Whether `v` is a span (a `span` column binds it): its text is a span's (`text_spans`), not a node's.
    fn span(&self, body: &[Literal], v: &str) -> bool {
        body.iter().filter_map(Literal::atom).filter(|a| !a.neg && a.rel != "text").any(|a| {
            let types = self.program.decls.get(a.rel.as_str());
            a.args.iter().enumerate().any(|(c, t)| matches!(t, Term::Var(x) if x == v) && types.and_then(|ts| ts.get(c)).is_some_and(|(_, ty)| ty == "span"))
        })
    }

    /// A matcher capture's node: the kinds and fields its matcher gives it (`{"pattern": "$N", "field": "name"}`, or a
    /// `kind` or `field` beside the pattern in an `all`). Columns past the root are the captures, in order. Bound by
    /// two captures: the narrower is enough, the first is kept.
    fn captured(&self, body: &[Literal], v: &str) -> Constraints {
        for a in body.iter().filter_map(Literal::atom).filter(|a| !a.neg) {
            let Some((_, text, names)) = self.matchers.iter().find(|(rel, _, _)| *rel == a.rel) else { continue };
            for (c, _) in a.args.iter().enumerate().skip(1).filter(|(_, t)| matches!(t, Term::Var(x) if x == v)) {
                let Some(name) = names.get(c - 1) else { continue };
                let got = capture_constraints(text, name);
                if !unconstrained(&got) {
                    return (got.0.iter().map(|k| bare(k)).collect(), got.1);
                }
            }
        }
        Constraints::default()
    }

    /// The fields `v` is a node in (`field(_, "F", v)`).
    fn fields_of(&self, body: &[Literal], v: &str) -> BTreeSet<String> {
        let base = !self.kinds.redefined.contains("field");
        body.iter().filter_map(Literal::atom).filter(|a| base && !a.neg && a.rel == "field").filter_map(|a| match (a.args.get(1), a.args.get(2)) {
            (Some(Term::Str(f)), Some(Term::Var(x))) if x == v => Some(f.clone()),
            _ => None,
        }).map(String::from).collect()
    }

    /// What narrows the nodes `v` is a child of (`child(p, v, _)`): each such parent's kinds, or the fields it is in;
    /// `None` when one is not narrowed, or `v` is no child.
    fn parents_of(&self, body: &[Literal], v: &str) -> Option<Constraints> {
        let parents = self.parent_vars(body, v);
        let (mut kinds, mut fields) = Constraints::default();
        for p in &parents {
            let (k, f) = self.parent_constraints(body, p)?;
            kinds.extend(k);
            fields.extend(f);
        }
        (!parents.is_empty()).then_some((kinds, fields))
    }

    /// Each `p` of a `child(p, v, _)` in `body`.
    fn parent_vars<'b>(&self, body: &'b [Literal], v: &str) -> Vec<&'b str> {
        let base = !self.kinds.redefined.contains("child");
        body.iter().filter_map(Literal::atom).filter(|a| base && !a.neg && a.rel == "child").filter_map(|a| match (a.args.first(), a.args.get(1)) {
            (Some(Term::Var(p)), Some(Term::Var(x))) if x == v => Some(p.as_str()),
            _ => None,
        }).collect()
    }

    /// What narrows parent `p`: its kinds, the fields it is in, or a matcher capture's kinds or fields (a rewritten
    /// `kind`/`field` body, `rr_ast_…(i, mn)`); `None` when nothing does.
    fn parent_constraints(&self, body: &[Literal], p: &str) -> Option<Constraints> {
        if let Some(k) = self.kinds.of_var(body, p) {
            return Some((k.iter().map(|k| bare(k)).collect(), BTreeSet::new()));
        }
        let fields = self.fields_of(body, p);
        if !fields.is_empty() {
            return Some((BTreeSet::new(), fields));
        }
        let (kinds, fields) = self.captured(body, p);
        if !kinds.is_empty() {
            return Some((kinds, BTreeSet::new()));
        }
        (!fields.is_empty()).then_some((BTreeSet::new(), fields))
    }
}
