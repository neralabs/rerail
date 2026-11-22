//! What the projection reads of a program's rules (decision 0281's path, step R), over its version: the spans and
//! tokens whose text a rule reads (`span_text_demand`), the node kinds whose text a rule reads and the nodes it reads
//! within the bound (`text_kind_demand`), and the anchors of a phase's nodes (`node_anchors`), each read by the
//! version's handles: a variable is its slot in its rule, a string its atom's text. `projection` puts them together
//! as the facts crate's `Projection` holds them.

use rerail_blocks::vector::Vector;
use crate::code_rule_dsl::facts::vtab::syntax::{Projection, ProjectionParts, TextDemand, TextNodes};
use crate::code_rule_dsl::logic::front::ast::SpanIn;
use crate::code_rule_dsl::logic::program_image::{AtomRef, CmpOp, Lit, RuleRef, Slot, TermRef, Version};
use rustc_hash::FxHashMap as HashMap;
use rustc_hash::FxHashSet as HashSet;
use std::collections::{BTreeMap, BTreeSet};

/// A program's matchers, by relation: its matcher text and its captures' names (`preparation::text::MatcherRelations`).
pub type Matchers = [(String, String, Vec<String>)];

/// The projection of the program `v`, reading `reads`, with its long text and anchors (`projection_of`, then
/// `Projection::with_long_text_parts`).
pub fn projection(v: &Version, reads: BTreeSet<String>, matchers: &Matchers) -> Projection {
    let demand = text_kind_demand(v, matchers);
    let anchors = if std::env::var("RERAIL_NODE_ANCHORS").is_ok_and(|x| x == "off") { Err("RERAIL_NODE_ANCHORS=off".into()) } else { node_anchors(v, &v.settings().expect("a version's settings are read"), matchers) };
    projection_of(v, reads).with_long_text_parts(demand, anchors)
}

/// The projection of the program `v` reading `reads`, before its long text and anchors: `core.rerail`'s shape for a
/// program that includes `core.rerail` (declares every relation it declares as an input), T's for one that declares its
/// base relations itself (T's programs, the converter's, O74).
pub fn projection_of(v: &Version, reads: BTreeSet<String>) -> Projection {
    let (spans, token_text) = span_text_demand(v);
    let derived: BTreeSet<String> = v.rules().map(|r| v.rel_name(r.head().rel()).to_owned()).collect();
    let outputs: BTreeSet<String> = v.output_names().into_iter().collect();
    crate::code_rule_dsl::logic::profile::projection_plan_of(|| users(v), &outputs, &reads);
    let core = crate::code_rule_dsl::facts::core::core_names();
    let core_shape = !core.is_empty() && core.iter().all(|r| v.declares(r));
    let partial: BTreeSet<String> = v
        .fact_values(crate::code_rule_dsl::facts::vtab::syntax::GUEST_POLICY, &Default::default())
        .into_iter()
        .flatten()
        .filter_map(|row| match row.as_slice() {
            [crate::code_rule_dsl::logic::front::Value::Str(k), crate::code_rule_dsl::logic::front::Value::Str(v)] if v == "partial" => k.strip_prefix("errors").map(|g| g.strip_prefix(':').unwrap_or("*").to_owned()),
            _ => None,
        })
        .collect();
    let error_triple = v.columns_of("syntax_error").is_some_and(|cols| cols.len() == 3);
    let rx_spans = v.rx_spans().expect("a version's spans are read");
    let ast_patterns = v.ast_patterns().expect("a version's patterns are read");
    let parts = ProjectionParts { spans, token_text, core_shape, partial, error_triple, derived, rx_spans: rx_spans.into_iter().collect(), ast_patterns: ast_patterns.into_iter().collect() };
    Projection::of_parts(parts, reads)
}

/// Each relation's readers: the heads of the rules reading it, and the natives reading it.
fn users(v: &Version) -> BTreeMap<String, BTreeSet<String>> {
    let mut users: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for r in v.rules() {
        for a in r.body().filter_map(|l| l.atom()) {
            users.entry(v.rel_name(a.rel()).to_owned()).or_default().insert(v.rel_name(r.head().rel()).to_owned());
        }
    }
    let about = crate::code_rule_dsl::logic::program_image::About::of(v);
    for n in &about.natives {
        for &read in &n.reads {
            users.entry(v.rel_name(read).to_owned()).or_default().insert(v.rel_name(n.rel).to_owned());
        }
    }
    users
}

fn args<'v>(a: AtomRef<'v>) -> Vec<TermRef<'v>> {
    a.args().collect()
}

fn is_var(t: Option<&TermRef<'_>>, s: Slot) -> bool {
    matches!(t, Some(TermRef::Var(x)) if *x == s)
}

/// The spans whose text a rule reads (a document's source, a line), and whether a token's. Plan-time provenance, not
/// a per-file scan: only copying a span (including variable equality) carries its origin. Regex captures have their
/// own bounded ranges and do not demand the container text.
pub fn span_text_demand(v: &Version) -> (BTreeSet<SpanIn>, bool) {
    // Bit 1 the source, 2 a line, 4 a token (the second column of the four token relations).
    let mut columns: HashMap<(&str, usize), u8> = [(("source", 1), 1), (("src_line", 2), 2)]
        .into_iter()
        .chain(["first_token", "last_token", "token_before", "token_after"].map(|rel| ((rel, 1), 4)))
        .collect();
    let mut requested = 0;
    loop {
        let mut changed = false;
        for rule in v.rules() {
            let mut vars: HashMap<Slot, u8> = HashMap::default();
            for atom in rule.body().filter_map(|l| l.atom()).filter(|a| !a.neg()) {
                let rel = v.rel_name(atom.rel());
                for (column, term) in atom.args().enumerate() {
                    if let TermRef::Var(value) = term {
                        *vars.entry(value).or_default() |= columns.get(&(rel, column)).copied().unwrap_or(0);
                    }
                }
            }
            loop {
                let mut aliases_changed = false;
                for literal in rule.body() {
                    if let Lit::Cmp { l: TermRef::Var(left), op: CmpOp::Eq, r: TermRef::Var(right) } = literal.view() {
                        let mask = vars.get(&left).copied().unwrap_or(0) | vars.get(&right).copied().unwrap_or(0);
                        for value in [left, right] {
                            let entry = vars.entry(value).or_default();
                            aliases_changed |= *entry != (*entry | mask);
                            *entry |= mask;
                        }
                    }
                }
                if !aliases_changed {
                    break;
                }
            }
            let head = v.rel_name(rule.head().rel());
            for (column, term) in rule.head().args().enumerate() {
                if let TermRef::Var(value) = term {
                    let mask = vars.get(&value).copied().unwrap_or(0);
                    if mask != 0 {
                        let entry = columns.entry((head, column)).or_default();
                        changed |= *entry != (*entry | mask);
                        *entry |= mask;
                    }
                }
            }
            for atom in rule.body().filter_map(|l| l.atom()).filter(|a| v.rel_name(a.rel()) == "text") {
                if let Some(TermRef::Var(value)) = atom.args().next() {
                    requested |= vars.get(&value).copied().unwrap_or(0);
                }
            }
        }
        if !changed {
            break;
        }
    }
    ([(SpanIn::Source, 1), (SpanIn::Line, 2)].into_iter().filter_map(|(span, bit)| (requested & bit != 0).then_some(span)).collect(), requested & 4 != 0)
}

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

/// Whether `field(_, "F", v)` in `r`'s body puts `v` in a field whose every kind is a leaf (`field(x, "attribute",
/// v)`): a token's text, which `text` holds at any length (`core_rr::TEXT_INLINE`).
fn leaf_field(v: &Version, r: RuleRef<'_>, s: Slot) -> bool {
    use crate::code_rule_dsl::facts::grammar_shape::{field_kinds, leaf_kind};
    r.body().filter_map(|l| l.atom()).filter(|a| !a.neg() && v.rel_name(a.rel()) == "field").any(|a| {
        let args = args(a);
        match (args.get(1), args.get(2)) {
            (Some(TermRef::Str(f)), Some(TermRef::Var(x))) if *x == s => field_kinds(v.text(*f)).is_some_and(|k| !k.is_empty() && k.iter().all(|k| leaf_kind(k))),
            _ => false,
        }
    })
}

/// Whether `text(v, "s")` names a text within the bound: no long text equals it.
fn short_constant(v: &Version, text: AtomRef<'_>) -> bool {
    matches!(text.args().nth(1), Some(TermRef::Str(s)) if v.text(s).len() <= crate::code_rule_dsl::facts::vtab::syntax::core_rr::TEXT_INLINE)
}

/// What fixes a variable's kinds where an atom reads it.
enum Fix {
    /// A derived relation's column.
    Column(u32, usize),
    /// Kinds known whatever the round (`kind(v, "K")`, a matcher's root), or any (`None`).
    Known(Option<Vector<u32>>),
}

type Kinds = Option<Vector<u32>>;

fn intersect(a: &[u32], b: &[u32]) -> Vector<u32> {
    a.iter().copied().filter(|k| b.binary_search(k).is_ok()).collect()
}

fn union(a: &[u32], b: &[u32]) -> Vector<u32> {
    let mut u: Vector<u32> = a.iter().chain(b).copied().collect();
    u.sort_unstable();
    u.dedup();
    u
}

/// A variable's kinds under `columns`: the intersection of what fixes it (any, when nothing does).
fn fold(columns: &[Vec<Kinds>], fixes: &[Fix]) -> Kinds {
    let mut kinds: Kinds = None;
    for f in fixes {
        let fixed: Option<&[u32]> = match f {
            Fix::Column(d, c) => match columns[*d as usize].get(*c) {
                Some(k) => k.as_deref(),
                None => Some(&[]),
            },
            Fix::Known(k) => k.as_deref(),
        };
        kinds = match (kinds, fixed) {
            (None, k) => k.map(Vector::<u32>::from),
            (k, None) => k,
            (Some(a), Some(b)) => Some(intersect(&a, b)),
        };
    }
    kinds
}

/// The node kinds whose text a rule reads, and whether some rule reads the text of a node of any kind. A node's kinds
/// are followed through the relations' columns to a fixpoint: a column of a derived relation holds the kinds its
/// rules' head variables can have, each the intersection of what its body fixes (`kind(v, "K")`, a matcher atom's
/// root, a column whose kinds are known), the union over its rules. An input's column is any kind. A positive
/// `text(v, …)` then demands `v`'s kinds (`core_rr::TEXT_INLINE` holds below them only). Fixed by the same body only,
/// `text(w, t)` over `classes(c, _), child(w, c, _), kind(w, "decorated_definition")` was demanded, but a rule reading
/// `text(n, t)` of `nodes(n)` was not, and saw no text past the bound.
pub fn text_kind_demand(v: &Version, matchers: &Matchers) -> TextDemand {
    // A node's kinds to a fixpoint, on numbers: each kind name interned, a set of kinds a sorted vector of them, each
    // derived relation's columns a vector by its number, and each rule head variable's constraints found once
    // (`fixers`). Looked up by an owned `(String, usize)` key and cloned as sets of names at every step of every round,
    // the fixpoint was most of this function. `None` is any kind.
    let kind_table: std::cell::RefCell<(HashMap<String, u32>, Vec<String>)> = Default::default();
    let intern = |k: String| -> u32 {
        let mut t = kind_table.borrow_mut();
        if let Some(&n) = t.0.get(&k) {
            return n;
        }
        let n = t.1.len() as u32;
        t.1.push(k.clone());
        t.0.insert(k, n);
        n
    };
    let roots: HashMap<&str, Kinds> = matchers.iter().map(|(rel, text, _)| (rel.as_str(), root_kind(text).map(|k| rerail_blocks::vector![intern(bare(&k))]))).collect();
    let rules: Vec<RuleRef<'_>> = v.rules().collect();
    let head_of = |r: RuleRef<'_>| v.rel_name(r.head().rel());
    let derived: Vec<&str> = {
        let mut d: Vec<&str> = rules.iter().map(|&r| head_of(r)).collect();
        d.sort_unstable();
        d.dedup();
        d
    };
    let derived_id: HashMap<&str, u32> = derived.iter().enumerate().map(|(i, r)| (*r, i as u32)).collect();
    // A base relation's name the program gives rules of its own: a relation like any other there, which narrows
    // nothing by the base relation's meaning. `kind(n, k) :- node(_, n, k, _, _)` (the lowering's projection,
    // `rr::lower` `projections`, and the data-layer prelude's) is `kind` itself, not a rule of the program's.
    let projection = |r: RuleRef<'_>| {
        if head_of(r) != "kind" {
            return false;
        }
        let h = args(r.head());
        let body: Vec<_> = r.body().collect();
        let ([TermRef::Var(n), TermRef::Var(k)], [l]) = (h.as_slice(), body.as_slice()) else { return false };
        let Lit::Atom(a) = l.view() else { return false };
        !a.neg() && v.rel_name(a.rel()) == "node" && matches!(args(a).as_slice(), [_, TermRef::Var(an), TermRef::Var(ak), _, _] if an == n && ak == k)
    };
    let redefined: BTreeSet<&str> = rules.iter().filter(|r| !projection(**r)).map(|&r| head_of(r)).filter(|r| ["kind", "node", "field", "child"].contains(r)).collect();
    let fixers = |r: RuleRef<'_>, s: Slot| -> Vec<Fix> {
        let mut out = Vec::new();
        for a in r.body().filter_map(|l| l.atom()).filter(|a| !a.neg()) {
            let args = args(a);
            let rel = v.rel_name(a.rel());
            for (c, t) in args.iter().enumerate() {
                if !matches!(t, TermRef::Var(x) if *x == s) {
                    continue;
                }
                // The base relations narrow by their meaning only where the program gives no rules of its own to one
                // of that name (`redefined`): there it is a relation like any other (its columns).
                out.push(match (rel, c, args.get(1)) {
                    ("kind", 0, Some(TermRef::Str(k))) if !redefined.contains("kind") => Fix::Known(Some(rerail_blocks::vector![intern(bare(v.text(*k)))])),
                    ("kind", _, _) if !redefined.contains("kind") => Fix::Known(None),
                    // T's base `node(d, n, kind, …)` (a program that declares its base relations itself): `n` of `kind`.
                    ("node", 1, _) if !redefined.contains("node") => Fix::Known(match args.get(2) {
                        Some(TermRef::Str(k)) => Some(rerail_blocks::vector![intern(bare(v.text(*k)))]),
                        _ => None,
                    }),
                    ("node", _, _) if !redefined.contains("node") => Fix::Known(None),
                    (rel, c, _) => match derived_id.get(rel) {
                        Some(&d) => Fix::Column(d, c),
                        None if c == 0 && let Some(root) = roots.get(rel) => Fix::Known(root.clone()),
                        None => Fix::Known(None),
                    },
                });
            }
        }
        out
    };
    // Each derived relation's columns, as wide as its widest head; each rule head variable's fixers.
    let mut columns: Vec<Vec<Kinds>> = vec![Vec::new(); derived.len()];
    let mut heads: Vec<(u32, usize, Vec<Fix>)> = Vec::new();
    for &rule in &rules {
        let h = derived_id[head_of(rule)];
        let hargs = args(rule.head());
        let width = &mut columns[h as usize];
        if width.len() < hargs.len() {
            width.resize(hargs.len(), Some(rerail_blocks::vector::Vector::new()));
        }
        for (c, t) in hargs.iter().enumerate() {
            let TermRef::Var(s) = *t else { continue };
            heads.push((h, c, fixers(rule, s)));
        }
    }
    loop {
        let mut changed = false;
        for (h, c, fixes) in &heads {
            let kinds = fold(&columns, fixes);
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
        if !changed {
            break;
        }
    }
    // A text atom's variable's kinds, by name.
    let of_var = |r: RuleRef<'_>, s: Slot| -> Option<BTreeSet<String>> {
        let fixes = fixers(r, s);
        let names = kind_table.borrow();
        fold(&columns, &fixes).map(|ks| ks.into_iter().map(|k| names.1[k as usize].clone()).collect())
    };
    // A rule (by head relation) that reads `v`'s long text by `@rr_node_text(v)`: its twin (`long_text_twins`).
    // Found in one pass: each (head, variable) a rule reads so.
    let twins: HashSet<(&str, &str)> = rules
        .iter()
        .flat_map(|&r| {
            r.body().filter_map(move |l| match l.view() {
                Lit::Cmp { r: TermRef::Call(f, mut fargs), .. } if v.text(f) == "rr_node_text" => match fargs.next() {
                    Some(TermRef::Var(x)) => Some((head_of(r), v.text(r.var(x)))),
                    _ => None,
                },
                _ => None,
            })
        })
        .collect();
    let twinned = |head: &str, var: &str| twins.contains(&(head, var));
    // Whether `s` is a span (a `span` column binds it): its text is a span's (`text_spans`), not a node's.
    let span = |r: RuleRef<'_>, s: Slot| {
        r.body().filter_map(|l| l.atom()).filter(|a| !a.neg() && v.rel_name(a.rel()) != "text").any(|a| {
            let types: Option<Vec<&str>> = v.decl_of(v.rel_name(a.rel())).map(|d| d.cols().map(|(_, t)| v.text(t)).collect());
            a.args().enumerate().any(|(c, t)| matches!(t, TermRef::Var(x) if x == s) && types.as_ref().and_then(|ts| ts.get(c)).is_some_and(|ty| *ty == "span"))
        })
    };
    // A matcher capture's node: the kinds and fields its matcher gives it (`{"pattern": "$N", "field": "name"}`, or a
    // `kind` or `field` beside the pattern in an `all`). Columns past the root are the captures, in order. An
    // `inside`'s `field` is where the node it holds sits in that ancestor, not the ancestor's own.
    let captured = |r: RuleRef<'_>, s: Slot| -> (BTreeSet<String>, BTreeSet<String>) {
        let mut got: Option<(BTreeSet<String>, BTreeSet<String>)> = None;
        for a in r.body().filter_map(|l| l.atom()).filter(|a| !a.neg()) {
            let rel = v.rel_name(a.rel());
            let Some((_, text, names)) = matchers.iter().find(|(m, _, _)| m == rel) else { continue };
            for (c, t) in a.args().enumerate().skip(1) {
                if !matches!(t, TermRef::Var(x) if x == s) {
                    continue;
                }
                let Some(name) = names.get(c - 1) else { continue };
                let mut out = None;
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(text) {
                    find(&json, &format!("${name}"), false, None, &mut out);
                }
                let (kinds, fields) = out.unwrap_or_default();
                if kinds.is_empty() && fields.is_empty() {
                    continue;
                }
                // Bound by two captures: the narrower is enough, the first is kept.
                got.get_or_insert((kinds.iter().map(|k| bare(k)).collect(), fields));
            }
        }
        got.unwrap_or_default()
    };
    // The fields `s` is a node in (`field(_, "F", v)`).
    let fields_of = |r: RuleRef<'_>, s: Slot| -> BTreeSet<String> {
        r.body()
            .filter_map(|l| l.atom())
            .filter(|a| !a.neg() && v.rel_name(a.rel()) == "field" && !redefined.contains("field"))
            .filter_map(|a| {
                let args = args(a);
                match (args.get(1), args.get(2)) {
                    (Some(TermRef::Str(f)), Some(TermRef::Var(x))) if *x == s => Some(v.text(*f).to_owned()),
                    _ => None,
                }
            })
            .collect()
    };
    // What narrows the nodes `s` is a child of (`child(p, v, _)`): each such parent's kinds, or the fields it is in;
    // `None` when one is not narrowed, or `s` is no child.
    let parents_of = |r: RuleRef<'_>, s: Slot| -> Option<(BTreeSet<String>, BTreeSet<String>)> {
        let (mut kinds, mut fields) = (BTreeSet::new(), BTreeSet::new());
        let mut any = false;
        for a in r.body().filter_map(|l| l.atom()).filter(|a| !a.neg() && v.rel_name(a.rel()) == "child" && !redefined.contains("child")) {
            let args = args(a);
            if let (Some(TermRef::Var(p)), true) = (args.first(), is_var(args.get(1), s)) {
                any = true;
                // Its kinds, its fields, or a matcher capture's (a rewritten `kind`/`field` body, `rr_ast_…(i, mn)`).
                match (of_var(r, *p), fields_of(r, *p)) {
                    (Some(k), _) => kinds.extend(k.iter().map(|k| bare(k))),
                    (None, f) if !f.is_empty() => fields.extend(f),
                    (None, _) => match captured(r, *p) {
                        (k, _) if !k.is_empty() => kinds.extend(k),
                        (_, f) if !f.is_empty() => fields.extend(f),
                        _ => return None,
                    },
                }
            }
        }
        any.then_some((kinds, fields))
    };
    let (mut kinds, mut any) = (BTreeSet::new(), Vec::new());
    let mut read = Some(TextNodes::default());
    let mut wide: Option<String> = None;
    for (ri, &rule) in rules.iter().enumerate() {
        let head = head_of(rule);
        for (i, l) in rule.body().enumerate() {
            let Lit::Atom(text) = l.view() else { continue };
            if text.neg() || v.rel_name(text.rel()) != "text" {
                continue;
            }
            let Some(TermRef::Var(s)) = text.args().next() else {
                read = None;
                wide.get_or_insert_with(|| head.to_owned());
                continue;
            };
            let name = v.text(rule.var(s));
            let fixed = of_var(rule, s);
            if let Some(nodes) = &mut read {
                match &fixed {
                    Some(k) => nodes.kinds.extend(k.iter().cloned()),
                    None if span(rule, s) => {}
                    None => match (fields_of(rule, s), captured(rule, s)) {
                        (f, _) if !f.is_empty() => nodes.fields.extend(f),
                        (_, (k, _)) if !k.is_empty() => nodes.kinds.extend(k),
                        (_, (_, f)) if !f.is_empty() => nodes.fields.extend(f),
                        // Compared with a constant within the bound: only a node whose text is that constant.
                        _ if short_constant(v, text) => {
                            if let Some(TermRef::Str(c)) = text.args().nth(1) {
                                nodes.values.insert(v.text(c).to_owned());
                            }
                        }
                        // A child of a node of known kinds: only such children.
                        _ if let Some((kinds, fields)) = parents_of(rule, s) => {
                            nodes.parents.extend(kinds);
                            nodes.parent_fields.extend(fields);
                        }
                        _ => {
                            wide = Some(format!("{head}, reading text({name}, …)"));
                            read = None
                        }
                    },
                }
            }
            match fixed {
                Some(k) => kinds.extend(k),
                None if !redefined.contains("field") && leaf_field(v, rule, s) || short_constant(v, text) || twinned(head, name) => {}
                None => any.push((ri, i)),
            }
        }
    }
    TextDemand { long: kinds, any, read, wide }
}

/// The constraints on the node `$name` matches in a matcher's rule, read where its pattern sits.
type Outer<'a> = Option<(&'a serde_json::Map<String, serde_json::Value>, bool)>;

fn own(o: &serde_json::Map<String, serde_json::Value>, ancestor: bool, kinds: &mut BTreeSet<String>, fields: &mut BTreeSet<String>) {
    kinds.extend(o.get("kind").and_then(|k| k.as_str()).map(str::to_owned));
    if !ancestor {
        fields.extend(o.get("field").and_then(|k| k.as_str()).map(str::to_owned));
    }
}

fn find(x: &serde_json::Value, name: &str, ancestor: bool, outer: Outer<'_>, out: &mut Option<(BTreeSet<String>, BTreeSet<String>)>) {
    match x {
        serde_json::Value::Object(o) => {
            if o.get("pattern").and_then(|p| p.as_str()) == Some(name) {
                let (mut kinds, mut fields) = (BTreeSet::new(), BTreeSet::new());
                for (m, ancestor) in [Some((o, ancestor)), outer].into_iter().flatten() {
                    own(m, ancestor, &mut kinds, &mut fields);
                    for sib in m.get("all").and_then(|a| a.as_array()).into_iter().flatten().filter_map(|x| x.as_object()) {
                        own(sib, false, &mut kinds, &mut fields);
                    }
                }
                // Two sites of one capture: either constrains it, or none does.
                *out = Some(match out.take() {
                    None => (kinds, fields),
                    Some((k, f)) if k.is_empty() && f.is_empty() || kinds.is_empty() && fields.is_empty() => Default::default(),
                    Some((k, f)) => (k.union(&kinds).cloned().collect(), f.union(&fields).cloned().collect()),
                });
            }
            for (key, y) in o {
                if key == "all" {
                    for m in y.as_array().into_iter().flatten() {
                        find(m, name, false, Some((o, ancestor)), out);
                    }
                } else {
                    find(y, name, key == "inside", None, out);
                }
            }
        }
        serde_json::Value::Array(a) => a.iter().for_each(|y| find(y, name, false, None, out)),
        _ => {}
    }
}

/// The node kinds whose subtrees hold every node a phase's exports depend on (`Projection::node_anchors`), or why
/// every node is projected; `settings` the program's. Only a phase (`.phase`) whose exports hold no node qualifies:
/// its node ids never leave it, so a projection of fewer nodes may number them its own way.
///
/// Each rule the exports reach is checked. A variable a node column binds must be proven to lie in an anchor's
/// subtree: by `kind(v, "K")` (`K` an anchor), as a child of a proven node (`child(p, v, _)`, `field(p, _, v)`), as a
/// matcher's root or capture (`rr_ast_*`, a matcher with no `inside`, `follows` or `precedes`), or by a column of a
/// relation whose every rule proves it. Of a proven node a rule may read only what the node itself holds (its kind,
/// text, literals, place in the source, document), and its children: no parent, ancestor, sibling or neighbouring
/// token, which a narrowed projection would not have, or would have otherwise. In `sources.rerail` (phase 1): import
/// statements' subtrees.
pub fn node_anchors(v: &Version, settings: &[(String, String)], matchers: &Matchers) -> Result<BTreeSet<String>, String> {
    let setting = |key: &str| settings.iter().filter(|(k, _)| k == key).map(|(_, x)| x.trim().to_owned()).collect::<Vec<_>>();
    if setting("phase").is_empty() {
        return Err("not a phase".into());
    }
    let exports: Vec<String> = setting("export").iter().flat_map(|x| x.split(',').map(|e| e.trim().trim_end_matches('.').to_owned()).collect::<Vec<_>>()).filter(|e| !e.is_empty()).collect();
    let is_node = |t: &str| t == "node" || t == "loc";
    let ast = crate::code_rule_dsl::facts::core::ast_matches::PREFIX;
    let types = |rel: &str| -> Option<Vec<&str>> { v.decl_of(rel).map(|d| d.cols().map(|(_, t)| v.text(t)).collect()) };
    for e in &exports {
        match types(e) {
            Some(ts) if !ts.iter().any(|t| is_node(t)) => {}
            Some(_) => return Err(format!("export {e} holds a node")),
            None => return Err(format!("export {e} is not declared")),
        }
    }
    if exports.is_empty() {
        return Err("no exports".into());
    }
    let node_col = |rel: &str, c: usize| rel.starts_with(ast) || types(rel).and_then(|ts| ts.get(c).copied()).is_some_and(is_node);
    // What a node holds itself; `child` and `field` go down from their first column. `rr_long_text_node(n)` is a
    // long-text twin's (`long_text_twins`): that `n`'s text is past the inline bound, made from the projected nodes.
    const LOCAL: &[&str] = &["kind", "node", "text", "name", "str", "strprefix", "value", "static_value", "docof", "line", "col", "end_line", "intval", "op", "op_at", "comment", "rr_long_text_node"];
    let down = |rel: &str| match rel {
        "child" => Some(1),
        "field" => Some(2),
        _ => None,
    };
    let tree = |rel: &str| LOCAL.contains(&rel) || down(rel).is_some();
    // A matcher's root kind, where it reaches only below its root.
    let roots: HashMap<&str, Option<String>> = matchers
        .iter()
        .map(|(rel, text, _)| {
            let below = !["\"inside\"", "\"follows\"", "\"precedes\""].iter().any(|k| text.contains(k));
            (rel.as_str(), root_kind(text).filter(|_| below).map(|k| bare(&k)))
        })
        .collect();
    // The rules the exports reach, tree views left as the base relations they stand for.
    let mut by_head: HashMap<&str, Vec<RuleRef<'_>>> = HashMap::default();
    for r in v.rules() {
        by_head.entry(v.rel_name(r.head().rel())).or_default().push(r);
    }
    let mut reached: BTreeSet<&str> = BTreeSet::new();
    let mut stack: Vec<&str> = exports.iter().map(String::as_str).collect();
    while let Some(rel) = stack.pop() {
        if tree(rel) || !reached.insert(rel) {
            continue;
        }
        for r in by_head.get(rel).into_iter().flatten() {
            for l in r.body() {
                match l.view() {
                    Lit::Nearest { .. } => return Err(format!("{rel} reads a nearest ancestor")),
                    Lit::Ast { .. } => return Err(format!("{rel} reads an ast matcher in place")),
                    _ => {}
                }
                stack.extend(l.atom().map(|a| v.rel_name(a.rel())));
            }
        }
    }
    let rules: Vec<RuleRef<'_>> = reached.iter().flat_map(|r| by_head.get(r).into_iter().flatten().copied()).collect();
    let derived = |rel: &str| !tree(rel) && by_head.contains_key(rel);
    // Every read of a node column is of a node-local relation, a child below a named parent, a matcher, or a relation
    // the program derives.
    for r in &rules {
        for a in r.body().filter_map(|l| l.atom()) {
            let rel = v.rel_name(a.rel());
            let args = args(a);
            for (c, t) in args.iter().enumerate() {
                if !node_col(rel, c) || matches!(t, TermRef::Wild) && down(rel) != Some(c) {
                    continue;
                }
                let ok = LOCAL.contains(&rel) || rel.starts_with(ast) || derived(rel) || down(rel).is_some_and(|child| c == 0 || c == child && matches!(args.first(), Some(TermRef::Var(_))));
                if !ok {
                    return Err(format!("{} reads {rel}", v.rel_name(r.head().rel())));
                }
            }
        }
    }
    // The derived columns proven to hold anchored nodes: every node column at first, dropped where a rule cannot prove
    // its head's (the greatest such set: by induction on derivations, every row's node is in an anchor's subtree).
    let mut proven_cols: BTreeSet<(String, usize)> = rules
        .iter()
        .flat_map(|r| {
            let rel = v.rel_name(r.head().rel());
            (0..r.head().args().count()).filter(move |&c| node_col(rel, c)).map(move |c| (rel.to_owned(), c))
        })
        .collect();
    // A rule's proven variables, and the variables a kind or a matcher anchors that nothing above them reaches.
    let prove = |r: RuleRef<'_>, cols: &BTreeSet<(String, usize)>| -> (BTreeSet<Slot>, BTreeSet<Slot>, Vec<(Slot, String)>) {
        let var = |t: Option<&TermRef<'_>>| match t {
            Some(TermRef::Var(s)) => Some(*s),
            _ => None,
        };
        // Down edges (parent, child) and each variable's own grounds: a kind constant or matcher root, or a column.
        let mut edges: Vec<(Slot, Slot)> = Vec::new();
        let mut kinds: Vec<(Slot, String)> = Vec::new();
        let mut grounded: BTreeSet<Slot> = BTreeSet::new();
        for l in r.body() {
            if let Lit::Cmp { l: TermRef::Var(a), op: CmpOp::Eq, r: TermRef::Var(b) } = l.view() {
                edges.push((a, b));
                edges.push((b, a));
            }
            let Some(a) = l.atom().filter(|a| !a.neg()) else { continue };
            let rel = v.rel_name(a.rel());
            let args = args(a);
            match rel {
                "kind" => {
                    if let (Some(x), Some(TermRef::Str(k))) = (var(args.first()), args.get(1)) {
                        kinds.push((x, bare(v.text(*k))));
                    }
                }
                "child" | "field" => {
                    if let (Some(p), Some(c)) = (var(args.first()), var(args.get(down(rel).unwrap_or(1)))) {
                        edges.push((p, c));
                    }
                }
                _ if rel.starts_with(ast) => {
                    if let Some(root) = var(args.first()) {
                        if let Some(Some(k)) = roots.get(rel) {
                            kinds.push((root, k.clone()));
                        }
                        for c in args.iter().skip(1).filter_map(|t| var(Some(t))) {
                            edges.push((root, c));
                        }
                    }
                }
                _ if derived(rel) => {
                    for (c, t) in args.iter().enumerate() {
                        if let Some(x) = var(Some(t)).filter(|_| cols.contains(&(rel.to_owned(), c))) {
                            grounded.insert(x);
                        }
                    }
                }
                _ => {}
            }
        }
        let closure = |from: &BTreeSet<Slot>| {
            let mut out = from.clone();
            loop {
                let before = out.len();
                for (p, c) in &edges {
                    if out.contains(p) {
                        out.insert(*c);
                    }
                }
                if out.len() == before {
                    return out;
                }
            }
        };
        // A kind's variable is an anchor where nothing proven, nor another such variable, reaches it from above (one of
        // a cycle, where they reach each other).
        let mut anchors: BTreeSet<Slot> = BTreeSet::new();
        loop {
            let proven = closure(&grounded.iter().copied().chain(anchors.iter().copied()).collect());
            let open: Vec<Slot> = kinds.iter().map(|(x, _)| *x).filter(|x| !proven.contains(x)).collect();
            let Some(&first) = open.first() else { return (proven, anchors, kinds) };
            let tops: Vec<Slot> = open.iter().copied().filter(|x| !open.iter().any(|w| w != x && closure(&BTreeSet::from([*w])).contains(x))).collect();
            if tops.is_empty() {
                anchors.insert(first);
            } else {
                anchors.extend(tops);
            }
        }
    };
    loop {
        let mut changed = false;
        for &r in &rules {
            let (proven, _, _) = prove(r, &proven_cols);
            let rel = v.rel_name(r.head().rel());
            for (c, t) in r.head().args().enumerate() {
                let key = (rel.to_owned(), c);
                if proven_cols.contains(&key) && !matches!(t, TermRef::Var(x) if proven.contains(&x)) {
                    proven_cols.remove(&key);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut kinds_out = BTreeSet::new();
    for &r in &rules {
        let (proven, anchors, kinds) = prove(r, &proven_cols);
        for a in r.body().filter_map(|l| l.atom()) {
            let rel = v.rel_name(a.rel());
            for (c, t) in a.args().enumerate() {
                if let TermRef::Var(x) = t
                    && node_col(rel, c)
                    && !proven.contains(&x)
                {
                    return Err(format!("{}'s ${} is in no anchor's subtree", v.rel_name(r.head().rel()), v.text(r.var(x))));
                }
            }
        }
        kinds_out.extend(kinds.into_iter().filter(|(x, _)| anchors.contains(x)).map(|(_, k)| k));
    }
    Ok(kinds_out)
}
