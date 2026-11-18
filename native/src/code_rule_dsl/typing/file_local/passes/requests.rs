//! `file_local::requests_file_local` on the version (decision 0281's path, step 7b; work queue Q149): each request of
//! decision 0174 (`rr_<a>_req(n)`) that the split finds cross-file narrowed to literals over the asked node's own
//! document, one for one with the tree's `Narrower`. See there for what a narrowed request keeps and why.
//!
//! A narrowed body is made of pieces: literals of the request's own rule, and of the rules of each cross-file
//! relation it unfolds, each of those renamed apart (`rq<k>_<x>`, its column's variable the reader's). A piece is
//! its source rule's literal and its renames, written through the builder (`RuleBuilder::lit_with`), so a rule is
//! written from several rules with no tree.

use super::super::{ALTERNATIVES, FileLocal, UNFOLD, close_under};
use crate::code_rule_dsl::logic::program_image::build::{Rewrite, T};
use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::{AtomId, CmpOp, Lit as L, LitRef, RuleId, RuleRef, Slot, TermRef, Version};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

/// `requests_file_local` of the version, for the requests `requests` names (those the program declares).
pub struct RequestsFileLocal {
    pub requests: Vec<&'static str>,
    /// Each narrowed request and how (the tree's return value, for the profile's decisions).
    pub narrowed: Vec<(String, String)>,
}

/// A literal of a narrowed body: source rule `src`'s literal at `node`, its variables renamed by `names` (none: their
/// own names).
#[derive(Clone)]
struct Piece {
    src: RuleId,
    node: u32,
    names: Option<Arc<BTreeMap<Slot, String>>>,
}

impl Piece {
    fn rule<'v>(&self, v: &'v Version) -> RuleRef<'v> {
        v.rule(self.src).expect("a rule of the input")
    }
    fn lit<'v>(&self, v: &'v Version) -> LitRef<'v> {
        let r = self.rule(v);
        r.body().find(|l| l.node() == self.node).expect("a literal of the rule")
    }
    fn name(&self, v: &Version, s: Slot) -> String {
        match self.names.as_ref().and_then(|n| n.get(&s)) {
            Some(n) => n.clone(),
            None => v.text(self.rule(v).var(s)).to_owned(),
        }
    }
    fn vars(&self, v: &Version) -> BTreeSet<String> {
        crate::code_rule_dsl::logic::plan::body::lit_vars(self.lit(v)).iter().map(|s| self.name(v, s)).collect()
    }
    /// The term as text, its variables by their names here: what tells two narrowed rules apart (`Rule`'s equality).
    fn show_term(&self, v: &Version, t: TermRef<'_>) -> String {
        match t {
            TermRef::Var(s) => self.name(v, s),
            TermRef::Call(f, args) => format!("@{}({})", v.text(f), args.map(|a| self.show_term(v, a)).collect::<Vec<_>>().join(", ")),
            t => v.show_term(self.rule(v), t),
        }
    }
    fn show(&self, v: &Version) -> String {
        match self.lit(v).view() {
            L::Atom(a) => format!("{}{}({})", if a.neg() { "!" } else { "" }, v.rel_name(a.rel()), a.args().map(|t| self.show_term(v, t)).collect::<Vec<_>>().join(", ")),
            L::Cmp { l, op, r } => format!("{} {} {}", self.show_term(v, l), op.as_str(), self.show_term(v, r)),
            L::Nearest { n, a, kindset } => format!("nearest({}, {}, {})", self.name(v, n), self.name(v, a), v.text(kindset)),
            _ => v.show_lit(self.rule(v), self.lit(v)),
        }
    }
}

/// The renames of a piece as it is written.
struct Renames<'a>(&'a BTreeMap<Slot, AtomId>);

impl Rewrite for Renames<'_> {
    fn var(&mut self, s: Slot) -> Option<AtomId> {
        self.0.get(&s).copied()
    }
}

/// `Narrower` over the version.
struct Narrower<'a> {
    v: &'a Version,
    same: &'a BTreeMap<String, Vec<usize>>,
    local: &'a BTreeSet<String>,
    fresh: std::cell::Cell<usize>,
}

/// `var_equalities` of pieces: `a = b`, `a = @doc(b)` and `@doc(a) = @doc(b)`, by name.
fn equalities(v: &Version, body: &[Piece]) -> Vec<(String, String)> {
    let doc_of = |p: &Piece, t: TermRef<'_>| match t {
        TermRef::Call(f, mut xs) if v.text(f) == "doc" => match (xs.next(), xs.next()) {
            (Some(TermRef::Var(b)), None) => Some(p.name(v, b)),
            _ => None,
        },
        _ => None,
    };
    body.iter()
        .filter_map(|p| match p.lit(v).view() {
            L::Cmp { l: TermRef::Var(a), op: CmpOp::Eq, r: TermRef::Var(b) } => Some((p.name(v, a), p.name(v, b))),
            L::Cmp { l: TermRef::Var(a), op: CmpOp::Eq, r } => doc_of(p, r).map(|b| (p.name(v, a), b)),
            L::Cmp { l, op: CmpOp::Eq, r } => doc_of(p, l).zip(doc_of(p, r)),
            _ => None,
        })
        .collect()
}

impl Narrower<'_> {
    /// `Narrower::narrow`: the bodies, over `n`'s document only, whose union holds every `n` that `body` binds; `None`
    /// when that takes every node.
    fn narrow(&self, body: &[Piece], n: &str, depth: usize) -> Option<Vec<Vec<Piece>>> {
        let v = self.v;
        let equal = equalities(v, body);
        // Each atom over locations, with the variables in its location columns.
        let located: Vec<(usize, BTreeSet<String>)> = body
            .iter()
            .enumerate()
            .filter_map(|(k, p)| match p.lit(v).view() {
                L::Atom(a) if !a.neg() => self.same.get(v.rel_name(a.rel())).map(|cols| {
                    let vars = cols.iter().filter_map(|&c| match a.args().nth(c) {
                        Some(TermRef::Var(s)) => Some(p.name(v, s)),
                        _ => None,
                    });
                    (k, vars.collect())
                }),
                L::Nearest { n, a, .. } => Some((k, BTreeSet::from([p.name(v, n), p.name(v, a)]))),
                _ => None,
            })
            .collect();
        let mut document = BTreeSet::from([n.to_owned()]);
        close_under(&mut document, &equal);
        let mut keep: BTreeSet<usize> = BTreeSet::new();
        loop {
            let before = keep.len();
            for (k, vars) in &located {
                if !keep.contains(k) && !vars.is_disjoint(&document) {
                    keep.insert(*k);
                    document.extend(vars.iter().cloned());
                    close_under(&mut document, &equal);
                }
            }
            if keep.len() == before {
                break;
            }
        }
        let mut bound: BTreeSet<String> = BTreeSet::new();
        for &k in &keep {
            bound.extend(body[k].vars(v));
        }
        // `m = x` binds m where x is bound.
        let same_value: Vec<(String, String)> = body
            .iter()
            .filter_map(|p| match p.lit(v).view() {
                L::Cmp { l: TermRef::Var(a), op: CmpOp::Eq, r: TermRef::Var(b) } => Some((p.name(v, a), p.name(v, b))),
                _ => None,
            })
            .collect();
        close_under(&mut bound, &same_value);
        for (k, p) in body.iter().enumerate() {
            if matches!(p.lit(v).view(), L::Cmp { .. }) && p.vars(v).is_subset(&bound) {
                keep.insert(k);
            }
        }
        let mut bodies: Vec<Vec<Piece>> = vec![body.iter().enumerate().filter(|(k, _)| keep.contains(k)).map(|(_, p)| p.clone()).collect()];
        let mut places = bound.contains(n);
        // Each cross-file atom joined to the document by a variable is unfolded.
        if depth < UNFOLD {
            for p in body {
                let L::Atom(a) = p.lit(v).view() else { continue };
                let rel = v.rel_name(a.rel());
                if a.neg() || self.local.contains(rel) || v.heads(a.rel()).next().is_none() {
                    continue;
                }
                let Some((column, var)) = a.args().enumerate().find_map(|(c, t)| match t {
                    TermRef::Var(s) if document.contains(&p.name(v, s)) => Some((c, p.name(v, s))),
                    _ => None,
                }) else {
                    continue;
                };
                let Some(alternatives) = self.unfold(a.rel(), column, &var, depth) else { continue };
                if alternatives.len() * bodies.len() > ALTERNATIVES {
                    continue;
                }
                places |= var == n;
                bodies = bodies.iter().flat_map(|b| alternatives.iter().map(move |alt| b.iter().chain(alt).cloned().collect())).collect();
            }
        }
        places.then_some(bodies)
    }

    /// `Narrower::unfold`: `rel`'s rules, each narrowed to the document of its column `column`, named `var`.
    fn unfold(&self, rel: crate::code_rule_dsl::logic::program_image::RelId, column: usize, var: &str, depth: usize) -> Option<Vec<Vec<Piece>>> {
        let v = self.v;
        let mut bodies = Vec::new();
        for rule in v.heads(rel) {
            let Some(TermRef::Var(h)) = rule.head().args().nth(column) else { return None };
            // A recursive rule that passes the column through gives it no value the other rules do not.
            let passes = rule.body().any(|l| matches!(l.view(), L::Atom(a) if !a.neg() && a.rel() == rule.head().rel() && a.args().nth(column).and_then(TermRef::var) == Some(h)));
            if passes {
                continue;
            }
            let k = self.fresh.get();
            self.fresh.set(k + 1);
            // The rule's variables apart from the reader's, its head's at `column` named `var`.
            let mut names: BTreeMap<Slot, String> = BTreeMap::new();
            let all = rule.body().flat_map(|l| crate::code_rule_dsl::logic::plan::body::lit_vars(l).iter().collect::<Vec<_>>());
            for s in all {
                names.entry(s).or_insert_with(|| if s == h { var.to_owned() } else { format!("rq{k}_{}", v.text(rule.var(s))) });
            }
            let names = Arc::new(names);
            // The literals a narrowed request may keep: positive atoms, nearest, comparisons (`renamed`).
            let renamed: Vec<Piece> = rule
                .body()
                .filter(|l| matches!(l.view(), L::Atom(a) if !a.neg()) || matches!(l.view(), L::Nearest { .. } | L::Cmp { .. }))
                .map(|l| Piece { src: rule.id(), node: l.node(), names: Some(Arc::clone(&names)) })
                .collect();
            bodies.extend(self.narrow(&renamed, var, depth + 1)?);
            if bodies.len() > ALTERNATIVES {
                return None;
            }
        }
        (!bodies.is_empty()).then_some(bodies)
    }
}

/// What a request rule becomes.
enum Made {
    Keep,
    Bodies(Vec<Vec<Piece>>),
    /// `head :- node(_, n, _, …)`: every node.
    EveryNode(Slot, usize),
}

impl ProgramPass for RequestsFileLocal {
    const NAME: &'static str = "on request: the requests narrowed to the node's document";
    fn settings(&self) -> Vec<u8> {
        self.requests.join("\n").into_bytes()
    }
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let v = cx.v();
        let is_request = |r: &RuleRef<'_>| self.requests.contains(&v.rel_name(r.head().rel()));
        if !v.rules().any(|r| is_request(&r)) {
            return Ok(());
        }
        let split = cx.get::<FileLocal>()?;
        let header = v.header(&Default::default())?;
        let node_arity = header.decls.get("node").map(|cols| cols.len());
        let narrower = Narrower { v, same: &split.same_doc, local: &split.local, fresh: std::cell::Cell::new(0) };
        let own = |r: RuleRef<'_>| -> Vec<Piece> { r.body().map(|l| Piece { src: r.id(), node: l.node(), names: None }).collect() };
        let shown = |pieces: &[Piece]| pieces.iter().map(|p| p.show(v)).collect::<Vec<_>>().join(", ");
        let head_shown = |r: RuleRef<'_>| {
            let p = Piece { src: r.id(), node: 0, names: None };
            format!("{}({})", v.rel_name(r.head().rel()), r.head().args().map(|t| p.show_term(v, t)).collect::<Vec<_>>().join(", "))
        };
        let mut plan: Vec<(RuleRef<'_>, Made)> = Vec::new();
        let mut every_node = false;
        let mut changed = false;
        for rule in v.rules() {
            let req = v.rel_name(rule.head().rel());
            let n = match rule.head().args().next() {
                Some(TermRef::Var(n)) if self.requests.contains(&req) && !split.local.contains(req) => n,
                _ => {
                    plan.push((rule, Made::Keep));
                    continue;
                }
            };
            let body = own(rule);
            let name = v.text(rule.var(n)).to_owned();
            let made = match (narrower.narrow(&body, &name, 0), node_arity) {
                (Some(bodies), _) => {
                    if let [one] = bodies.as_slice()
                        && shown(one) == shown(&body)
                    {
                        Made::Keep
                    } else {
                        let alts: Vec<String> = bodies.iter().map(|b| shown(b)).collect();
                        self.narrowed.push((req.to_owned(), format!("`{}` asks {}", shown(&body), alts.join(" ; "))));
                        Made::Bodies(bodies)
                    }
                }
                (None, Some(arity)) => {
                    every_node = true;
                    self.narrowed.push((req.to_owned(), format!("`{}` asks node(_, {name}, …)", shown(&body))));
                    Made::EveryNode(n, arity)
                }
                (None, None) => Made::Keep,
            };
            changed |= !matches!(made, Made::Keep);
            plan.push((rule, made));
        }
        if !changed {
            return Ok(());
        }
        let input = Arc::clone(w.input());
        let b = w.derive()?;
        if every_node && !header.inputs.iter().any(|i| i == "node") {
            let mut with = header.clone();
            with.inputs.push("node".to_owned());
            b.set_header(&header, &with);
        }
        // Two rules narrowed to one body are one rule (a request's, the tree's `seen`).
        let mut seen: HashSet<String> = HashSet::new();
        for (rule, made) in plan {
            let request = is_request(&rule);
            match made {
                Made::Keep => {
                    if !request || seen.insert(format!("{} :- {}", head_shown(rule), shown(&own(rule)))) {
                        b.keep(&input, rule);
                    }
                }
                Made::EveryNode(n, arity) => {
                    let key = format!("{} :- node(_, {}, …)", head_shown(rule), v.text(rule.var(n)));
                    if seen.insert(key) {
                        let mut args = vec![T::Wild; arity];
                        args[1] = T::VarAtom(rule.var(n));
                        b.rule().source(rule.id()).head_from(&input, rule, rule.head()).atom("node", &args).finish()?;
                    }
                }
                Made::Bodies(bodies) => {
                    for body in bodies {
                        if !seen.insert(format!("{} :- {}", head_shown(rule), shown(&body))) {
                            continue;
                        }
                        // Each renamed piece's names, interned before the rule is opened.
                        let renames: Vec<Option<BTreeMap<Slot, AtomId>>> = body
                            .iter()
                            .map(|p| p.names.as_ref().map(|names| names.iter().map(|(s, n)| (*s, b.atom(n))).collect()))
                            .collect();
                        let mut rb = b.rule().source(rule.id()).head_from(&input, rule, rule.head());
                        for (p, names) in body.iter().zip(&renames) {
                            let (r, l) = (p.rule(&input), p.lit(&input));
                            rb = match names {
                                None => rb.lit(&input, r, l),
                                Some(names) => rb.lit_with(&input, r, l, &mut Renames(names)),
                            };
                        }
                        rb.finish()?;
                    }
                }
            }
        }
        Ok(())
    }
}
