//! The text passes on the version (decision 0281's path, step 6c), each the tree's function of the same name ported
//! onto the version's rules:
//!
//! - `Absorb` (`matchers::absorb`): a Python program's tree-shaped parts as `ast` literals (decision 0106 §4);
//! - `MatcherRelations` (`matchers::lower`): each `ast` literal an atom of its matcher's relation (decision 0106);
//! - `NodeSubjects` (`strings::node_subject_tests`): a text test on a node tests the node's text;
//! - `Twins` (`syntax::long_text_twins`): twin rules reading text past the inline bound (decision 0109);
//! - `ImpliedSets` (`strings::implied_set_tests`): a test over a constant table's values gains the whole table's;
//! - `TextRelations` (`strings::as_relations`): text tests over text relations as the vocabulary's relations
//!   (decision 0037), `match_all` and `match_group` as their regexes' relations, and the `fstring_template` markers.
//!
//! The names they make follow content (the tests', the regexes', the matchers'), and their fresh variables are
//! counted per rule, as the tree's are: a program's names do not depend on the order of its rules.

use crate::code_rule_dsl::logic::plan::corpus::matchers::{MOST_CAPTURES, PREFIX, columns, single_field};
use crate::code_rule_dsl::logic::plan::text_demand;
use crate::code_rule_dsl::logic::program_image::manager::{Cx, Pipeline, RuleOut, RulePass, Select, Step};
use crate::code_rule_dsl::logic::program_image::{AtomRef, Builder, Cell, CmpOp, Lit, LitRef, Rewrite, RuleRef, Slot, T, TermRef, Version, t_of};
use crate::code_rule_dsl::logic::strings::{DURING, GROUPS, MATCHES, Matcher, RELATION, SET, TEXT_COLUMNS, Vocabulary, is_test};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

type Result<T> = std::result::Result<T, String>;

/// A program's matchers, by relation: its matcher text and its captures' names.
pub type Matchers = Vec<(String, String, Vec<String>)>;

/// What the text passes found: the matchers and the vocabulary's tests.
pub struct Found {
    pub matchers: Matchers,
    pub vocabulary: Vocabulary,
}

/// The text passes on `p`, for a program of `dialect`: as the tree's run in `plan::corpus`'s preparation.
pub fn passes(p: &mut Pipeline<'_>, dialect: crate::code_rule_dsl::language::GrammarDialect) -> Result<Found> {
    if dialect == crate::code_rule_dsl::language::GrammarDialect::Python && !std::env::var("RERAIL_AST_PARTS").is_ok_and(|v| v == "off") {
        p.rules(&mut Absorb { capture: !std::env::var("RERAIL_AST_PART_CAPTURES").is_ok_and(|v| v == "off"), rewritten: 0 });
    }
    let mut lowering = MatcherRelations::default();
    p.rules(&mut lowering);
    let matchers = lowering.out;
    let text = p.current()?.decl_of("text").is_some();
    if text {
        p.rules(&mut NodeSubjects);
        p.rules(&mut Twins { matchers: matchers.clone(), by_rule: HashMap::new(), made: 0 });
        // `as_relations` runs it again: on the twins.
        p.rules(&mut NodeSubjects);
    }
    p.rules(&mut ImpliedSets { added: 0 });
    let mut relations = TextRelations::default();
    p.rules(&mut relations);
    let vocabulary = Vocabulary::from_parts(relations.tests, relations.specs, relations.matches, relations.markers);
    Ok(Found { matchers, vocabulary })
}

fn name<'v>(v: &'v Version, r: RuleRef<'v>, s: Slot) -> &'v str {
    v.text(r.var(s))
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

/// The variables literal `l` names (`front::Literal::vars`).
fn lit_vars(l: LitRef<'_>) -> BTreeSet<Slot> {
    let mut out = BTreeSet::new();
    match l.view() {
        Lit::Atom(a) => a.args().for_each(|t| term_vars(t, &mut out)),
        Lit::Cmp { l, r, .. } => {
            term_vars(l, &mut out);
            term_vars(r, &mut out);
        }
        Lit::Agg { var, op, over, atom } => {
            atom.args().for_each(|t| term_vars(t, &mut out));
            out.insert(var);
            out.extend(over);
            out.extend(op.bound());
        }
        Lit::Nearest { n, a, .. } => out.extend([n, a]),
        Lit::Ast { document, node, captures, .. } => {
            out.extend([document, node]);
            out.extend(captures.map(|(_, s)| s));
        }
    }
    out
}

/// The head's variables, each a whole argument (`front::Atom`'s `Term::Var` arguments).
fn head_vars(r: RuleRef<'_>) -> BTreeSet<Slot> {
    r.head().args().filter_map(TermRef::var).collect()
}

fn positive<'v>(v: &Version, l: LitRef<'v>, rel: &str) -> Option<AtomRef<'v>> {
    match l.view() {
        Lit::Atom(a) if !a.neg() && v.rel_name(a.rel()) == rel => Some(a),
        _ => None,
    }
}

fn args<'v>(a: AtomRef<'v>) -> Vec<TermRef<'v>> {
    a.args().collect()
}

/// The next name `{stem}{n}` (`n` counting on from `count`) the rule does not hold (`held`, its names).
fn fresh(held: &mut HashSet<String>, count: &mut usize, stem: &str) -> String {
    loop {
        *count += 1;
        let t = format!("{stem}{count}");
        if held.insert(t.clone()) {
            return t;
        }
    }
}

// ---- matchers::absorb ----

struct Absorb {
    capture: bool,
    rewritten: usize,
}

impl RulePass for Absorb {
    const NAME: &'static str = "tree patterns as matchers";
    fn settings(&self) -> Vec<u8> {
        vec![self.capture as u8]
    }
    fn rule(&mut self, cx: &Cx<'_>, r: RuleRef<'_>, out: &mut RuleOut<'_, '_>) -> Result<Step> {
        let v = cx.v();
        let str_of = |t: Option<&TermRef<'_>>| match t {
            Some(TermRef::Str(s)) => Some(v.text(*s).strip_prefix("python:").unwrap_or(v.text(*s)).to_owned()),
            _ => None,
        };
        let var_of = |t: Option<&TermRef<'_>>| t.and_then(|t| t.var());
        let body: Vec<LitRef<'_>> = r.body().collect();
        // How many literals (and the head) read each variable.
        let mut uses: HashMap<Slot, usize> = HashMap::new();
        for s in r.head().args().filter_map(TermRef::var) {
            *uses.entry(s).or_default() += 1;
        }
        for l in &body {
            for s in lit_vars(*l) {
                *uses.entry(s).or_default() += 1;
            }
        }
        let mut drop: BTreeSet<usize> = BTreeSet::new();
        let mut added: Vec<(Slot, String, Vec<(String, Slot)>)> = Vec::new();
        for (ri, l) in body.iter().enumerate() {
            let Some(root) = positive(v, *l, "kind") else { continue };
            let root = args(root);
            let (Some(rv), Some(k)) = (var_of(root.first()), str_of(root.get(1))) else { continue };
            if drop.contains(&ri) {
                continue;
            }
            let mut parts = vec![serde_json::json!({"kind": k})];
            let mut captures: Vec<(String, Slot)> = Vec::new();
            let mut taken = vec![ri];
            for (fi, f) in body.iter().enumerate() {
                let Some(field) = positive(v, *f, "field") else { continue };
                let field = args(field);
                if drop.contains(&fi) || field.len() != 3 || var_of(field.first()) != Some(rv) {
                    continue;
                }
                let (Some(fname), Some(x)) = (str_of(field.get(1)), var_of(field.get(2))) else { continue };
                let read_again = uses.get(&x).is_some_and(|&n| n > 2);
                if uses.get(&x).is_none_or(|&n| n < 2) || (read_again && !self.capture) || x == rv {
                    continue;
                }
                let kind = body.iter().enumerate().find_map(|(ki, kl)| {
                    let a = args(positive(v, *kl, "kind")?);
                    (var_of(a.first()) == Some(x)).then(|| str_of(a.get(1)).map(|k2| (ki, k2)))?
                });
                let Some((ki, k2)) = kind else { continue };
                if !single_field(&k, &fname) {
                    continue;
                }
                if read_again {
                    let name_of = format!("C{}", captures.len() + 1);
                    parts.push(serde_json::json!({"has": {"field": fname, "kind": k2, "pattern": format!("${name_of}"), "stopBy": "neighbor"}}));
                    captures.push((name_of, x));
                } else {
                    parts.push(serde_json::json!({"has": {"field": fname, "kind": k2, "stopBy": "neighbor"}}));
                }
                taken.extend([fi, ki]);
            }
            if parts.len() < 2 || captures.len() > MOST_CAPTURES {
                continue;
            }
            drop.extend(taken);
            let matcher = serde_json::json!({"language": "python", "rule": {"all": parts}}).to_string();
            added.push((rv, matcher, captures));
        }
        if added.is_empty() {
            return Ok(Step::Keep);
        }
        self.rewritten += 1;
        let documents: Vec<String> = (1..=added.len()).map(|n| format!("rr_ast_d{n}")).collect();
        let mut w = out.rule().head_from(v, r, r.head());
        for ((node, matcher, captures), document) in added.iter().zip(&documents) {
            let caps: Vec<(&str, T<'_>)> = captures.iter().map(|(n, s)| (n.as_str(), T::Var(name(v, r, *s)))).collect();
            w = w.ast(T::Var(document), T::Var(name(v, r, *node)), matcher, &caps);
        }
        for (i, l) in body.iter().enumerate() {
            if !drop.contains(&i) {
                w = w.lit(v, r, *l);
            }
        }
        w.finish()?;
        Ok(Step::Written)
    }
    fn finish(&mut self, cx: &Cx<'_>, _b: &mut Builder<'_>) -> Result<()> {
        let n = self.rewritten;
        if n > 0 {
            cx.decide("tree patterns as matchers (decision 0106)", || format!("{n} rules' kind and field parts run as the transducer's matchers"));
        }
        Ok(())
    }
}

// ---- matchers::lower ----

#[derive(Default)]
struct MatcherRelations {
    /// Each distinct matcher (text, captured names), in its content's order.
    texts: Vec<(String, Vec<String>)>,
    /// Whether a written rule reads `docof`.
    docof: bool,
    out: Matchers,
}

fn relation(k: usize, m: usize) -> String {
    if m == 0 { format!("{PREFIX}{k}") } else { format!("{PREFIX}{k}_{m}") }
}

/// The names an `ast` literal's matcher text makes in its `transform`: they bind text, not nodes.
fn transformed(text: &str) -> BTreeSet<String> {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v.get("transform")?.as_object().map(|t| t.keys().cloned().collect()))
        .unwrap_or_default()
}

impl RulePass for MatcherRelations {
    const NAME: &'static str = "ast literals as matcher relations";
    fn begin(&mut self, cx: &Cx<'_>) -> Result<Select> {
        let v = cx.v();
        let mut texts: BTreeSet<(String, Vec<String>)> = BTreeSet::new();
        let mut set = crate::code_rule_dsl::logic::program_image::manager::RuleSet::new();
        for r in v.rules() {
            for l in r.body() {
                if let Lit::Ast { matcher, captures, .. } = l.view()
                    && captures.clone().count() <= MOST_CAPTURES
                {
                    texts.insert((v.text(matcher).to_owned(), captures.map(|(n, _)| v.text(n).to_owned()).collect()));
                    set.insert(r.id());
                }
            }
        }
        self.texts = texts.into_iter().collect();
        Ok(Select::Rules(set))
    }
    fn rule(&mut self, cx: &Cx<'_>, r: RuleRef<'_>, out: &mut RuleOut<'_, '_>) -> Result<Step> {
        let v = cx.v();
        let head = head_vars(r);
        let body: Vec<LitRef<'_>> = r.body().collect();
        let reads: Vec<BTreeSet<Slot>> = body.iter().map(|l| lit_vars(*l)).collect();
        // Each literal's relation, named before the rule is written.
        let mut rels = Vec::new();
        for l in &body {
            if let Lit::Ast { matcher, captures, .. } = l.view()
                && captures.clone().count() <= MOST_CAPTURES
            {
                let names: Vec<String> = captures.map(|(n, _)| v.text(n).to_owned()).collect();
                let k = self.texts.iter().position(|(t, n)| t == v.text(matcher) && *n == names).expect("numbered in begin");
                rels.push(out.builder().rel(&relation(k, names.len())));
            }
        }
        let mut rels = rels.into_iter();
        let mut w = out.rule().head_from(v, r, r.head());
        for (i, l) in body.iter().enumerate() {
            match l.view() {
                Lit::Ast { document, node, captures, .. } if captures.clone().count() <= MOST_CAPTURES => {
                    let mut a = vec![T::Var(name(v, r, node))];
                    a.extend(captures.map(|(_, s)| T::Var(name(v, r, s))));
                    w = w.atom(rels.next().expect("named above"), &a);
                    let elsewhere = head.contains(&document) || reads.iter().enumerate().any(|(j, x)| j != i && x.contains(&document));
                    if elsewhere {
                        self.docof = true;
                        w = w.atom("docof", &[T::Var(name(v, r, node)), T::Var(name(v, r, document))]);
                    }
                }
                _ => w = w.lit(v, r, *l),
            }
        }
        w.finish()?;
        Ok(Step::Written)
    }
    fn finish(&mut self, cx: &Cx<'_>, b: &mut Builder<'_>) -> Result<()> {
        let v = cx.v();
        let mut inputs: Vec<String> = v.inputs().map(|a| v.text(a).to_owned()).collect();
        let before = inputs.len();
        for (k, (text, names)) in self.texts.iter().enumerate() {
            let rel = relation(k, names.len());
            let made = transformed(text);
            let types = std::iter::once("node").chain(names.iter().map(|n| if made.contains(n) { "sym" } else { "node" }));
            let cols: Vec<(&str, &str)> = columns(&rel).iter().copied().zip(types).collect();
            b.declare(&rel, &cols);
            if !inputs.contains(&rel) {
                inputs.push(rel.clone());
            }
            self.out.push((rel, text.clone(), names.clone()));
        }
        // `docof` is an input unless the program derives it already (`hiding` writes it as a view where documents
        // are hidden).
        let derived = v.rel("docof").is_some_and(|d| v.heads(d).next().is_some());
        let read = self.docof || v.rules().any(|r| r.body().filter_map(|l| l.atom()).any(|a| v.rel_name(a.rel()) == "docof"));
        if !self.out.is_empty() && !derived && !inputs.iter().any(|r| r == "docof") && read {
            if v.decl_of("docof").is_none() {
                b.declare("docof", &[("n", "loc"), ("d", "number")]);
            }
            inputs.push("docof".to_owned());
        }
        if inputs.len() != before {
            b.set_inputs(&inputs);
        }
        Ok(())
    }
}

// ---- strings::node_subject_tests ----

struct NodeSubjects;

/// A text test: `@op(s, patterns) = 0|1`.
fn test_of<'v>(v: &Version, l: LitRef<'v>) -> Option<(crate::code_rule_dsl::logic::program_image::AtomId, Vec<TermRef<'v>>, i64)> {
    let Lit::Cmp { l: TermRef::Call(op, a), op: CmpOp::Eq, r: TermRef::Int(x @ (0 | 1)) } = l.view() else { return None };
    is_test(v.text(op)).then(|| (op, a.collect(), x))
}

impl RulePass for NodeSubjects {
    const NAME: &'static str = "text tests on nodes test their text";
    fn rule(&mut self, cx: &Cx<'_>, r: RuleRef<'_>, out: &mut RuleOut<'_, '_>) -> Result<Step> {
        let v = cx.v();
        let mut nodes: HashSet<Slot> = HashSet::new();
        for l in r.body() {
            let Lit::Atom(a) = l.view() else { continue };
            if a.neg() {
                continue;
            }
            let types: Option<Vec<&str>> = v.decl_of(v.rel_name(a.rel())).map(|d| d.cols().map(|(_, t)| v.text(t)).collect());
            for (c, t) in a.args().enumerate() {
                if let (TermRef::Var(s), Some(ty)) = (t, types.as_ref().and_then(|ts| ts.get(c))) {
                    if *ty == "node" || *ty == "loc" {
                        nodes.insert(s);
                    }
                }
            }
        }
        let body: Vec<LitRef<'_>> = r.body().collect();
        let rewritten: Vec<usize> = body.iter().enumerate().filter(|(_, l)| test_of(v, **l).is_some_and(|(_, a, _)| matches!(a.first(), Some(TermRef::Var(s)) if nodes.contains(s)))).map(|(i, _)| i).collect();
        if rewritten.is_empty() {
            return Ok(Step::Keep);
        }
        let mut held: HashSet<String> = r.vars().map(|a| v.text(a).to_owned()).collect();
        let mut count = 0;
        let fresh_names: Vec<String> = rewritten.iter().map(|_| fresh(&mut held, &mut count, "rr_node_text")).collect();
        let mut w = out.rule().head_from(v, r, r.head());
        let mut added: Vec<(Slot, &str)> = Vec::new();
        for (i, l) in body.iter().enumerate() {
            match rewritten.iter().position(|&x| x == i) {
                Some(k) => {
                    let (op, a, x) = test_of(v, *l).expect("a test");
                    let TermRef::Var(s) = a[0] else { unreachable!("a node") };
                    let t = fresh_names[k].as_str();
                    let call: Vec<T<'_>> = std::iter::once(T::Var(t)).chain(a[1..].iter().map(|x| t_of(v, r, *x))).collect();
                    w = w.cmp(T::CallAtom(op, call), CmpOp::Eq, T::Int(x));
                    added.push((s, t));
                }
                None => w = w.lit(v, r, *l),
            }
        }
        for (s, t) in added {
            w = w.atom("text", &[T::Var(name(v, r, s)), T::Var(t)]);
        }
        w.finish()?;
        Ok(Step::Written)
    }
}

// ---- syntax::long_text_twins ----

struct Twins {
    matchers: Matchers,
    /// The rules twinned, by id: their `text` atoms' places.
    by_rule: HashMap<crate::code_rule_dsl::logic::program_image::RuleId, Vec<usize>>,
    made: usize,
}

impl RulePass for Twins {
    const NAME: &'static str = "long text twins";
    fn begin(&mut self, cx: &Cx<'_>) -> Result<Select> {
        const MAX: usize = 3;
        let v = cx.v();
        let demand = text_demand::text_kind_demand(v, &self.matchers);
        let rules: Vec<RuleRef<'_>> = v.rules().collect();
        // A twin binds `v` by the rest of its body: an atom whose `text` alone binds `v` is not twinned.
        let bound = |r: RuleRef<'_>, i: usize| {
            let body: Vec<LitRef<'_>> = r.body().collect();
            let Some(Lit::Atom(text)) = body.get(i).map(|l| l.view()) else { return false };
            let Some(TermRef::Var(x)) = text.args().next() else { return false };
            body.iter().enumerate().filter(|(j, _)| *j != i).filter_map(|(_, l)| l.atom()).filter(|a| !a.neg() && v.rel_name(a.rel()) != "text").any(|a| a.args().any(|t| t.var() == Some(x)))
        };
        let mut by_rule: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (ri, i) in demand.any.into_iter().filter(|&(ri, i)| bound(rules[ri], i)) {
            by_rule.entry(ri).or_default().push(i);
        }
        let mut set = crate::code_rule_dsl::logic::program_image::manager::RuleSet::new();
        for (ri, atoms) in by_rule.into_iter().filter(|(_, a)| a.len() <= MAX) {
            set.insert(rules[ri].id());
            self.by_rule.insert(rules[ri].id(), atoms);
        }
        Ok(Select::Rules(set))
    }
    fn rule(&mut self, cx: &Cx<'_>, r: RuleRef<'_>, out: &mut RuleOut<'_, '_>) -> Result<Step> {
        let v = cx.v();
        let atoms = self.by_rule.remove(&r.id()).expect("selected in begin");
        let body: Vec<LitRef<'_>> = r.body().collect();
        // The rule as it was, then its twins.
        let mut w = out.rule().head_from(v, r, r.head());
        for l in &body {
            w = w.lit(v, r, *l);
        }
        w.finish()?;
        let mut count = 0usize;
        for set in 1..(1u32 << atoms.len()) {
            // Each twinned `text(v, t)`: `t = @rr_node_text(v)`, and `rr_long_text_node(v)` after the body.
            let mut swapped: Vec<(usize, Slot, String)> = Vec::new();
            for (bit, &i) in atoms.iter().enumerate().filter(|(b, _)| set >> b & 1 == 1) {
                let Lit::Atom(text) = body[i].view() else { continue };
                let a = args(text);
                let Some(TermRef::Var(node)) = a.first().copied() else { continue };
                let t = match a.get(1) {
                    Some(TermRef::Var(t)) => name(v, r, *t).to_owned(),
                    _ => {
                        count += 1;
                        format!("rr_long_text{count}_{bit}")
                    }
                };
                swapped.push((i, node, t));
            }
            let mut w = out.rule().head_from(v, r, r.head());
            for (i, l) in body.iter().enumerate() {
                match swapped.iter().find(|(j, _, _)| *j == i) {
                    Some((_, node, t)) => w = w.cmp(T::Var(t), CmpOp::Eq, T::Call("rr_node_text", vec![T::Var(name(v, r, *node))])),
                    None => w = w.lit(v, r, *l),
                }
            }
            for (_, node, _) in &swapped {
                w = w.atom("rr_long_text_node", &[T::Var(name(v, r, *node))]);
            }
            w.finish()?;
            self.made += 1;
        }
        Ok(Step::Written)
    }
    fn finish(&mut self, cx: &Cx<'_>, b: &mut Builder<'_>) -> Result<()> {
        let made = self.made;
        if made == 0 {
            return Ok(());
        }
        let v = cx.v();
        b.declare("rr_long_text_node", &[("n", "node")]);
        let mut inputs: Vec<String> = v.inputs().map(|a| v.text(a).to_owned()).collect();
        if !inputs.iter().any(|r| r == "rr_long_text_node") {
            inputs.push("rr_long_text_node".into());
            b.set_inputs(&inputs);
        }
        cx.decide("long text twins (decision 0109)", || format!("{made} rules read text past the inline bound for the nodes they bind"));
        Ok(())
    }
}

// ---- strings::implied_set_tests ----

/// Most patterns an implied set test takes: a bigger table is tested as written.
const IMPLIED_SET_MAX: usize = 4096;

struct ImpliedSets {
    added: usize,
}

/// Whether `rel` is an input of `v`.
fn is_input(v: &Version, rel: &str) -> bool {
    v.inputs().any(|a| v.text(a) == rel)
}

/// `rel`'s facts (not its about rows), each row's cells.
fn fact_rows(v: &Version, rel: &str) -> Option<Vec<Vec<Cell>>> {
    let f = v.facts(v.find(rel)?)?;
    Some(f.rows().map(|row| row.collect()).collect())
}

/// The values column `col` of `rel` can hold when they come from the program's constants alone
/// (`strings::constant_domain`).
fn constant_domain(v: &Version, rel: &str, col: usize, visiting: &mut Vec<(String, usize)>) -> Option<BTreeSet<String>> {
    if visiting.iter().any(|(r, c)| r == rel && *c == col) || visiting.len() > 16 {
        return None;
    }
    let str_cell = |c: Option<&Cell>| match c {
        Some(Cell::Str(a)) => Some(v.text(*a).to_owned()),
        _ => None,
    };
    let rules: Vec<RuleRef<'_>> = v.rel(rel).map(|id| v.heads(id).collect()).unwrap_or_default();
    if rules.is_empty() {
        if is_input(v, rel) {
            return None;
        }
        let rows = fact_rows(v, rel)?;
        return rows.iter().map(|row| str_cell(row.get(col))).collect();
    }
    visiting.push((rel.to_owned(), col));
    let mut out = BTreeSet::new();
    for row in fact_rows(v, rel).into_iter().flatten() {
        let Some(s) = str_cell(row.get(col)) else {
            visiting.pop();
            return None;
        };
        out.insert(s);
    }
    for rule in rules {
        let got = match rule.head().args().nth(col) {
            Some(TermRef::Str(s)) => Some(BTreeSet::from([v.text(s).to_owned()])),
            Some(TermRef::Var(x)) => rule.body().find_map(|l| {
                let Lit::Atom(a) = l.view() else { return None };
                if a.neg() {
                    return None;
                }
                let c = a.args().position(|t| t.var() == Some(x))?;
                let arel = v.rel_name(a.rel());
                // A fact relation read with constants: only the rows that hold them.
                if v.heads(a.rel()).next().is_none()
                    && let Some(rows) = fact_rows(v, arel)
                {
                    if is_input(v, arel) {
                        return None;
                    }
                    let terms = args(a);
                    let matches = |row: &[Cell]| {
                        terms.iter().zip(row).all(|(t, c)| match (t, c) {
                            (TermRef::Str(x), Cell::Str(y)) => v.text(*x) == v.text(*y),
                            (TermRef::Int(x), Cell::Int(y)) => x == y,
                            (TermRef::Str(_) | TermRef::Int(_), _) => false,
                            _ => true,
                        })
                    };
                    return rows.iter().filter(|r| matches(r)).map(|row| str_cell(row.get(c))).collect();
                }
                constant_domain(v, arel, c, visiting)
            }),
            _ => None,
        };
        match got {
            Some(values) => out.extend(values),
            None => {
                visiting.pop();
                return None;
            }
        }
    }
    visiting.pop();
    Some(out)
}

impl RulePass for ImpliedSets {
    const NAME: &'static str = "implied set tests";
    fn rule(&mut self, cx: &Cx<'_>, r: RuleRef<'_>, out: &mut RuleOut<'_, '_>) -> Result<Step> {
        let v = cx.v();
        let mut extra: Vec<(crate::code_rule_dsl::logic::program_image::AtomId, Slot, BTreeSet<String>)> = Vec::new();
        for l in r.body() {
            let Some((op, a, 1)) = test_of(v, l) else { continue };
            let [TermRef::Var(s), TermRef::Var(p)] = a.as_slice() else { continue };
            let domain = r.body().find_map(|b| match b.view() {
                Lit::Atom(at) if !at.neg() => {
                    let c = at.args().position(|t| t.var() == Some(*p))?;
                    constant_domain(v, v.rel_name(at.rel()), c, &mut Vec::new())
                }
                _ => None,
            });
            if let Some(values) = domain.filter(|d| !d.is_empty() && d.len() <= IMPLIED_SET_MAX) {
                extra.push((op, *s, values));
            }
        }
        if extra.is_empty() {
            return Ok(Step::Keep);
        }
        self.added += extra.len();
        let mut w = out.rule().head_from(v, r, r.head());
        for (op, s, values) in &extra {
            let set = T::Call(SET, values.iter().map(|x| T::Str(x)).collect());
            w = w.cmp(T::CallAtom(*op, vec![T::Var(name(v, r, *s)), set]), CmpOp::Eq, T::Int(1));
        }
        for l in r.body() {
            w = w.lit(v, r, l);
        }
        w.finish()?;
        Ok(Step::Written)
    }
    fn finish(&mut self, cx: &Cx<'_>, _b: &mut Builder<'_>) -> Result<()> {
        let added = self.added;
        if added > 0 {
            cx.decide("implied set tests", || format!("{added} text tests of a constant table's values each gained the test of the whole table as one set"));
        }
        Ok(())
    }
}

// ---- strings::as_relations ----

/// A test's patterns: one string, or a set `{a, b}` (`strings::patterns`).
fn patterns(v: &Version, t: TermRef<'_>) -> Option<Vec<String>> {
    match t {
        TermRef::Str(s) => Some(vec![v.text(s).to_owned()]),
        TermRef::Call(f, a) if v.text(f) == SET => a.map(|x| if let TermRef::Str(s) = x { Some(v.text(s).to_owned()) } else { None }).collect(),
        _ => None,
    }
}

/// A text test: its operator, its string's variable, its patterns, and whether it holds (`strings::as_test`).
fn as_test(v: &Version, l: LitRef<'_>) -> Option<(String, Slot, Vec<String>, bool)> {
    let (op, a, x) = test_of(v, l)?;
    let [TermRef::Var(s), p] = a.as_slice() else { return None };
    Some((v.text(op).to_owned(), *s, patterns(v, *p)?, x == 1))
}

fn same_term(a: TermRef<'_>, b: TermRef<'_>) -> bool {
    match (a, b) {
        (TermRef::Var(x), TermRef::Var(y)) => x == y,
        (TermRef::Wild, TermRef::Wild) | (TermRef::Top, TermRef::Top) => true,
        (TermRef::Int(x), TermRef::Int(y)) => x == y,
        (TermRef::Str(x), TermRef::Str(y)) => x == y,
        (TermRef::Call(f, xs), TermRef::Call(g, ys)) => f == g && xs.clone().count() == ys.clone().count() && xs.zip(ys).all(|(x, y)| same_term(x, y)),
        _ => false,
    }
}

/// Whether the rule (head `head`, `width` literals, one `atom` and a holding test of its string) is a native test's
/// definition (`strings::definition`).
fn definition(head: AtomRef<'_>, atom: AtomRef<'_>, width: usize, holds: bool) -> bool {
    let h = args(head);
    let vars: HashSet<Slot> = h.iter().filter_map(|t| t.var()).collect();
    holds && width == 2 && !head.neg() && h.len() == atom.args().count() && h.iter().zip(atom.args()).all(|(x, y)| same_term(*x, y)) && vars.len() == h.len()
}

enum How {
    Input { s: Slot },
    During { input: String, column: usize, at: usize },
    Defined { input: String, column: usize },
}

struct Plan {
    place: usize,
    key: (String, Vec<String>),
    holds: bool,
    how: How,
}

#[derive(Default)]
struct TextRelations {
    plans: HashMap<crate::code_rule_dsl::logic::program_image::RuleId, Vec<Plan>>,
    ids: BTreeMap<(String, Vec<String>), usize>,
    during: BTreeMap<(usize, String, usize), String>,
    defined: Vec<(String, String, usize, String, Vec<String>)>,
    regexes: BTreeMap<String, usize>,
    made: BTreeMap<String, bool>,
    tests: Vec<(String, Arc<Matcher>)>,
    specs: Vec<(String, Vec<String>)>,
    matches: Vec<Arc<regex::Regex>>,
    markers: Vec<String>,
}

/// The text columns of the program's relations, and of those the columns answered during the run
/// (`strings::vocabulary_columns`).
fn vocabulary_columns(v: &Version) -> (HashMap<String, HashSet<usize>>, HashSet<(String, usize)>) {
    let derives = |rel: &str| v.rel(rel).is_some_and(|id| v.heads(id).next().is_some());
    let mut columns: HashMap<String, HashSet<usize>> = TEXT_COLUMNS.iter().map(|(n, c)| ((*n).to_owned(), c.iter().copied().collect())).collect();
    let mut answered: HashSet<(String, usize)> = TEXT_COLUMNS.iter().filter(|(n, _)| derives(n)).flat_map(|(n, c)| c.iter().map(|&c| ((*n).to_owned(), c))).collect();
    let inputs: HashSet<&str> = v.inputs().map(|a| v.text(a)).collect();
    let decls = v.decls();
    loop {
        let mut found = Vec::new();
        for d in &decls {
            let relation = v.text(d.name());
            if fact_rows(v, relation).is_some_and(|rows| !rows.is_empty()) || (inputs.contains(relation) && !columns.contains_key(relation)) {
                continue;
            }
            let writers: Vec<RuleRef<'_>> = v.rel(relation).map(|id| v.heads(id).collect()).unwrap_or_default();
            if writers.is_empty() {
                continue;
            }
            for (column, (_, ty)) in d.cols().enumerate() {
                if v.text(ty) != "sym" || columns.get(relation).is_some_and(|cols| cols.contains(&column)) {
                    continue;
                }
                let source = |rule: &RuleRef<'_>| -> Option<bool> {
                    let Some(TermRef::Var(value)) = rule.head().args().nth(column) else { return None };
                    let mut from = rule
                        .body()
                        .filter_map(|l| l.atom())
                        .filter(|a| !a.neg())
                        .flat_map(|atom| {
                            let rel = v.rel_name(atom.rel());
                            let a = args(atom);
                            columns.get(rel).into_iter().flatten().filter(move |&&col| a.get(col).is_some_and(|t| t.var() == Some(value))).map(|&col| answered.contains(&(rel.to_owned(), col))).collect::<Vec<_>>()
                        })
                        .peekable();
                    from.peek()?;
                    Some(from.all(|a| a))
                };
                let sources: Option<Vec<bool>> = writers.iter().map(source).collect();
                if let Some(sources) = sources {
                    found.push((relation.to_owned(), column, sources.iter().any(|&a| a)));
                }
            }
        }
        if found.is_empty() {
            return (columns, answered);
        }
        for (relation, column, during) in found {
            if during {
                answered.insert((relation.clone(), column));
            }
            columns.entry(relation).or_default().insert(column);
        }
    }
}

/// `match_all` and `match_group` atoms as their regex's relation, the regex left out.
struct Regexes<'a> {
    v: &'a Version,
    numbers: &'a BTreeMap<String, usize>,
}

impl Regexes<'_> {
    /// The relation a `match_all` or `match_group` atom reads: its regex's, and whether it is a group's.
    fn of(&self, a: AtomRef<'_>) -> Option<(String, bool)> {
        let group = match self.v.rel_name(a.rel()) {
            "match_all" => false,
            "match_group" => true,
            _ => return None,
        };
        let Some(TermRef::Str(re)) = a.args().next() else { return None };
        let k = self.numbers[self.v.text(re)];
        Some((if group { format!("{GROUPS}{k}") } else { format!("{MATCHES}{k}") }, group))
    }
}

impl Rewrite for Regexes<'_> {
    fn atom_as(&mut self, a: AtomRef<'_>) -> Option<(String, bool, bool)> {
        self.of(a).map(|(rel, _)| (rel, a.neg(), a.gated()))
    }
    fn drop_arg(&mut self, a: AtomRef<'_>, c: usize) -> bool {
        c == 0 && self.of(a).is_some()
    }
}

impl RulePass for TextRelations {
    const NAME: &'static str = "text tests as relations";
    fn begin(&mut self, cx: &Cx<'_>) -> Result<Select> {
        let v = cx.v();
        let (vocabulary_columns, answered) = vocabulary_columns(v);
        let about = crate::code_rule_dsl::logic::program_image::About::of(v);
        let other_native: HashSet<&str> = about.natives.iter().filter(|n| n.kind != crate::code_rule_dsl::logic::program_image::NativeKind::TextTest).map(|n| v.rel_name(n.rel)).collect();
        let outputs: HashSet<&str> = v.outputs().map(|a| v.text(a)).collect();
        let mut heads: HashMap<&str, usize> = HashMap::new();
        for r in v.rules() {
            *heads.entry(v.rel_name(r.head().rel())).or_default() += 1;
        }
        let alone: HashSet<&str> = heads.into_iter().filter(|(h, n)| *n == 1 && !outputs.contains(h) && !is_input(v, h) && fact_rows(v, h).is_none() && !other_native.contains(h)).map(|(h, _)| h).collect();
        let row_mode = std::env::var("RERAIL_TEXT_TESTS").is_ok_and(|x| x == "row");
        let mut set = crate::code_rule_dsl::logic::program_image::manager::RuleSet::new();
        let mut markers: BTreeSet<String> = BTreeSet::new();
        for r in v.rules() {
            let body: Vec<LitRef<'_>> = r.body().collect();
            let mut texts: HashMap<Slot, Option<(usize, usize)>> = HashMap::new();
            for (at, l) in body.iter().enumerate() {
                let Lit::Atom(a) = l.view() else { continue };
                if a.neg() {
                    continue;
                }
                let rel = v.rel_name(a.rel());
                let a = args(a);
                for &c in vocabulary_columns.get(rel).into_iter().flatten() {
                    let Some(TermRef::Var(x)) = a.get(c) else { continue };
                    if !answered.contains(&(rel.to_owned(), c)) {
                        texts.insert(*x, None);
                    } else if TEXT_COLUMNS.iter().any(|(t, _)| *t == rel) {
                        texts.entry(*x).or_insert(Some((c, at)));
                    }
                }
            }
            let mut plan = Vec::new();
            for (place, l) in body.iter().enumerate() {
                let Some((op, s, mut pats, holds)) = as_test(v, *l) else { continue };
                let Some(from) = texts.get(&s) else { continue };
                if from.is_some() && row_mode {
                    continue;
                }
                pats.sort();
                pats.dedup();
                let key = (op, pats);
                let how = match *from {
                    Some((column, at)) => {
                        let Lit::Atom(atom) = body[at].view() else { unreachable!("a text atom") };
                        let input = v.rel_name(atom.rel()).to_owned();
                        if alone.contains(v.rel_name(r.head().rel())) && definition(r.head(), atom, body.len(), holds) {
                            How::Defined { input, column }
                        } else {
                            How::During { input, column, at }
                        }
                    }
                    None => How::Input { s },
                };
                plan.push(Plan { place, key, holds, how });
            }
            let mut matching = false;
            for a in body.iter().filter_map(|l| l.atom()) {
                let rel = v.rel_name(a.rel());
                let t = args(a);
                match (rel, t.as_slice()) {
                    ("fstring_template", [_, TermRef::Str(m), _]) => {
                        markers.insert(v.text(*m).to_owned());
                    }
                    ("fstring_template", _) => return Err(format!("fstring_template takes a constant marker: fstring_template(n, \"{{...}}\", v), not {}", v.show_atom(r, a))),
                    ("match_all", [TermRef::Str(re), _, _]) | ("match_group", [TermRef::Str(re), _, _, _, _]) => {
                        self.regexes.insert(v.text(*re).to_owned(), 0);
                        matching = true;
                    }
                    ("match_all", _) => return Err(format!("match_all takes a constant regex, a span and its match: match_all(\"re\", sp, m), not {}", v.show_atom(r, a))),
                    ("match_group", _) => {
                        return Err(format!("match_group takes a constant regex, a span, its match, a group name and its span: match_group(\"re\", sp, m, g, gsp), not {}", v.show_atom(r, a)));
                    }
                    _ => {}
                }
            }
            if !plan.is_empty() || matching {
                set.insert(r.id());
            }
            if !plan.is_empty() {
                self.plans.insert(r.id(), plan);
            }
        }
        self.markers = markers.into_iter().collect();
        let keys: BTreeSet<(String, Vec<String>)> = self.plans.values().flatten().map(|p| p.key.clone()).collect();
        self.ids = keys.into_iter().enumerate().map(|(k, key)| (key, k)).collect();
        for (key, &k) in &self.ids {
            let rel = format!("{RELATION}{k}");
            let matcher = Matcher::new(&key.0, &key.1)?;
            cx.decide("text tests (decision 0037: a relation per distinct test, each string tested once)", || {
                let shown: Vec<String> = key.1.iter().take(6).map(|p| format!("{p:?}")).collect();
                let more = if key.1.len() > 6 { format!(", … {} in all", key.1.len()) } else { String::new() };
                format!("{rel} = @{}({{{}{more}}}): {}", key.0, shown.join(", "), matcher.describe())
            });
            self.tests.push((rel, Arc::new(matcher)));
            self.specs.push(key.clone());
        }
        let during: BTreeSet<(usize, String, usize)> = self
            .plans
            .values()
            .flatten()
            .filter_map(|p| match &p.how {
                How::During { input, column, .. } => Some((self.ids[&p.key], input.clone(), *column)),
                _ => None,
            })
            .collect();
        self.during = during.into_iter().enumerate().map(|(n, key)| {
            let native = format!("{DURING}{}_{n}", key.0);
            (key, native)
        }).collect();
        for (k, (re, at)) in self.regexes.iter_mut().enumerate() {
            let rel = v.rules().flat_map(|r| r.body().filter_map(|l| l.atom()).collect::<Vec<_>>()).find(|a| matches!(a.args().next(), Some(TermRef::Str(x)) if v.text(x) == re.as_str()) && matches!(v.rel_name(a.rel()), "match_all" | "match_group")).map_or("match_all", |a| v.rel_name(a.rel()));
            self.matches.push(Arc::new(regex::Regex::new(re).map_err(|e| format!("{rel}({re:?}, …): {e}"))?));
            *at = k;
        }
        Ok(Select::Rules(set))
    }
    fn rule(&mut self, cx: &Cx<'_>, r: RuleRef<'_>, out: &mut RuleOut<'_, '_>) -> Result<Step> {
        let v = cx.v();
        let plan = self.plans.remove(&r.id()).unwrap_or_default();
        let body: Vec<LitRef<'_>> = r.body().collect();
        // Each literal's replacement: an atom (relation, the terms of literal `from`'s atom or the test's string,
        // negated or not), or none (dropped).
        let mut put: BTreeMap<usize, Option<(String, From, bool)>> = BTreeMap::new();
        let mut replaced: Vec<usize> = Vec::new();
        for Plan { place, key, holds, how } in plan {
            let k = self.ids[&key];
            match how {
                How::Defined { input, column } => self.defined.push((v.rel_name(r.head().rel()).to_owned(), input, column, key.0, key.1)),
                How::Input { s } => {
                    put.insert(place, Some((format!("{RELATION}{k}"), From::Var(s), !holds)));
                }
                How::During { input, column, at } => {
                    let native = self.during[&(k, input, column)].clone();
                    if holds && !replaced.contains(&at) {
                        replaced.push(at);
                        put.insert(at, Some((native, From::Atom(at), false)));
                        put.insert(place, None);
                    } else {
                        put.insert(place, Some((native, From::Atom(at), !holds)));
                    }
                }
            }
        }
        // Relations named before the rule is written.
        let rels: BTreeMap<usize, crate::code_rule_dsl::logic::program_image::RelId> = put.iter().filter_map(|(i, p)| p.as_ref().map(|(rel, _, _)| (*i, out.builder().rel(rel)))).collect();
        let mut rename = Regexes { v, numbers: &self.regexes };
        let mut w = out.rule().head_from(v, r, r.head());
        for (i, l) in body.iter().enumerate() {
            match put.get(&i) {
                Some(None) => {}
                Some(Some((_, from, neg))) => {
                    let terms: Vec<T<'_>> = match from {
                        From::Var(s) => vec![T::Var(name(v, r, *s))],
                        From::Atom(at) => body[*at].atom().map(|a| a.args().map(|t| t_of(v, r, t)).collect()).unwrap_or_default(),
                    };
                    w = w.atom_with(rels[&i], &terms, *neg, false);
                }
                None => w = w.lit_with(v, r, *l, &mut rename),
            }
        }
        w.finish()?;
        let made: Vec<(String, bool)> = body.iter().filter_map(|l| l.atom()).filter_map(|a| rename.of(a)).collect();
        self.made.extend(made);
        Ok(Step::Written)
    }
    fn finish(&mut self, cx: &Cx<'_>, b: &mut Builder<'_>) -> Result<()> {
        let v = cx.v();
        let mut inputs: Vec<String> = v.inputs().map(|a| v.text(a).to_owned()).collect();
        let before = inputs.len();
        let mut natives: Vec<(String, String, usize, String, Vec<String>)> = Vec::new();
        // Each native test's one rule orders it after its input, and is the test row by row.
        for ((k, input, column), native) in std::mem::take(&mut self.during) {
            let (op, pats) = self.specs[k].clone();
            let decl: Vec<(String, String)> = v.decl_of(&input).ok_or_else(|| format!("{input} is not declared"))?.cols().map(|(n, t)| (v.text(n).to_owned(), v.text(t).to_owned())).collect();
            let xs: Vec<String> = (0..decl.len()).map(|i| format!("x{i}")).collect();
            let a: Vec<T<'_>> = xs.iter().map(|x| T::Var(x)).collect();
            let set = T::Call(SET, pats.iter().map(|p| T::Str(p)).collect());
            b.rule().head(native.as_str(), &a).atom(input.as_str(), &a).cmp(T::Call(&op, vec![T::Var(&xs[column]), set]), CmpOp::Eq, T::Int(1)).finish()?;
            let cols: Vec<(&str, &str)> = decl.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
            b.declare(&native, &cols);
            natives.push((native, input, column, op, pats));
        }
        natives.extend(std::mem::take(&mut self.defined));
        if !natives.is_empty() {
            // As `program_facts::write_about` writes them: by the native's name, each one's patterns in order.
            let mut tests = b.about_rows("native_text_test");
            let mut pats = b.about_rows("native_text_pattern");
            let p = tests.first().or(pats.first()).map(|r| r[0]).unwrap_or_else(|| Cell::Str(b.atom(crate::code_rule_dsl::logic::program_image::ABOUT)));
            for (native, input, column, op, patterns) in natives {
                let rel = b.atom(&native);
                tests.retain(|r| r.get(1) != Some(&Cell::Str(rel)));
                pats.retain(|r| r.get(1) != Some(&Cell::Str(rel)));
                let (i, o) = (b.atom(&input), b.atom(&op));
                tests.push(vec![p, Cell::Str(rel), Cell::Str(i), Cell::Int(column as i64), Cell::Str(o)]);
                for (n, pattern) in patterns.iter().enumerate() {
                    let pa = b.atom(pattern);
                    pats.push(vec![p, Cell::Str(rel), Cell::Int(n as i64), Cell::Str(pa)]);
                }
            }
            let key = |b: &Builder<'_>, r: &Vec<Cell>| match (r.get(1), r.get(2)) {
                (Some(Cell::Str(a)), Some(Cell::Int(n))) => (b.text(*a).to_owned(), *n),
                (Some(Cell::Str(a)), _) => (b.text(*a).to_owned(), 0),
                _ => (String::new(), 0),
            };
            tests.sort_by_cached_key(|r| key(b, r).0);
            pats.sort_by_cached_key(|r| key(b, r));
            b.set_about_rows("native_text_test", tests);
            b.set_about_rows("native_text_pattern", pats);
        }
        for (rel, _) in &self.tests {
            b.declare(rel, &[("s", "sym")]);
            if !inputs.contains(rel) {
                inputs.push(rel.clone());
            }
        }
        for (rel, group) in std::mem::take(&mut self.made) {
            if group {
                b.declare(&rel, &[("sp", "node"), ("m", "node"), ("g", "sym"), ("gsp", "node")]);
            } else {
                b.declare(&rel, &[("sp", "node"), ("m", "node")]);
            }
            if !inputs.contains(&rel) {
                inputs.push(rel);
            }
        }
        if inputs.len() != before {
            b.set_inputs(&inputs);
        }
        Ok(())
    }
}

/// What a test's replacement atom reads: the test's string, or the terms of the rule's atom at a place.
enum From {
    Var(Slot),
    Atom(usize),
}
