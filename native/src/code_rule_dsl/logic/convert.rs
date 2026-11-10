//! Policy matchers as Datalog rules (list step 30, decision 0036): a one-time converter from a clause's
//! compiled matcher (`matcher::AstMatcher`, options resolved) to rules over the base relations
//! (`node`, `child`, `field`, `text`, `op`, `intval`) and the tree axes (`inside`, `has`).
//!
//! Each sub-rule is a conjunction of literals over one node variable. `any` is a relation with one rule per
//! branch, `not` its negation, a utility a relation of its own (recursive where it reaches itself). Regexes become string tests or
//! name sets, never regexes. What it cannot say is a gap, named; what it says only approximately is noted.
//! The clause is converted whole or not at all: a clause that fails leaves nothing it emitted (`Emitter::clause`),
//! and the program keeps only what `violation` reaches (`reachable`).
//!
//! The relations, per node `n`:
//! - `inside` (to the end): `inside($n, $a, set)`, the set being the kinds the ancestor's rule allows;
//! - `inside` with a stop rule: the same, less ancestors past the first stop node (`blocked`);
//! - `has` likewise, read from the ancestor; from a node of unknown kinds, `within(m, n)` (the tree); with a
//!   field, the descendant is within that field's child (byte spans, both nodes bound);
//! - neighbours: `child(p, c, i)` or `field(p, f, c)`.
//!
//! # The authored overlay
//!
//! What the converter cannot say is written by hand, one `.rr` file per rule (decision 0043), and merged over the
//! converted program (`overlay`):
//! - the file is `gate/data-layer/packs/data-layer/candidates/datalog/authored/<rule-id>.rr`, the rule id as in the
//!   policy (`data-layer-lazy-architecture`), read whenever the rule is converted (no rebuild);
//! - it declares every relation it reads or defines but core.rr's, which are the engine's (decision 0047:
//!   `rr::read_file` refuses a file that declares or derives one): core.rr's declarations are its prelude
//!   (`rr::lower::core`), in the column types of the converter's (`BASE`): `doc(d, path, module)`,
//!   `node(d, n, kind, start, end)`, `child(p, c, i)` (named children), `field(p, f, c)`, `text(n, s)`,
//!   `name(n, s)`, `str(n, v)`, `strprefix(n, p)`, `intval(n, v)`, `op(n, s)`, `line(n, l)`, and those the README
//!   lists (the source text, `docof`, `op_at`, `prev_token`, `token`, `visit_index`, `path_part`, `file_name`, `pkg`,
//!   W's values and f-string trees: `fstring_piece`, `fstring_template`, whose marker is a constant; Ruff's string
//!   annotations and their trees, `string_annotation(s)` and `parsed(s, grammar, root)`, each tree a document of its
//!   own; W's pythonAst `literal_json(n, v)`, `dump_sha256(n, h)`, `body_sha256(f, h)` and `signature_sha256(f, h)`).
//!   Not core.rr's: core.rr's `@py_unparse(n)` and `@ast_dump(n)` as `cpython_unparse(n: node, s: symbol)` and
//!   `cpython_ast_dump(n: node, s: symbol)`, and the two syntax verdicts: CPython's, core.rr's `syntax_error(d, l,
//!   msg)`, as `cpython_syntax_error(d: number, l: number, msg: symbol)`, and the tape's (tree-sitter's ERROR and
//!   MISSING nodes, the converter's `syntax_error(d: number)`) as `tape_syntax_error(d: number)`, given (below).
//!   The base relations it reads are inputs, `.input` or not. It may `#include` the libraries (`python/ast.rr`):
//!   their `.output`s are theirs. The scope, `in_scope(d: number)` and `out_scope(d: number)`, is the converter's
//!   (the policy's, or the replay's per event): read, never derived. So is the tape's verdict, `tape_syntax_error`
//!   (decision 0047). So is the report gate, `report_when_gt(k: number)`: one row
//!   `k` where the policy (as the run configures it) has `report_when: {count: {gt: k}}`, the only form the compiler
//!   accepts (once per policy, on a define clause): the rule reports nothing unless its findings, all files and
//!   clauses together, number more than `k`; no row where it has none. The tree: `inside(n, a, set)` and
//!   `has(a, n, set)` (`a` a strict ancestor of `n` of a kind in `set`), `nearest(n, a, set)` (the nearest one),
//!   `set` a relation of kind facts the file declares (`.decl calls(kind: symbol)`, `calls("call").`). The
//!   converter does not gate its own clauses: in a policy with a report gate they are gaps, for the file to give.
//!   The options, as the run resolves them (the policy's config, else each option's default; the replay's per
//!   event), are given too, read, never derived: `option_on(name: symbol)`, a row per boolean option that is true;
//!   `option_item(name: symbol, i: number, v: symbol)`, a row per item of an array option, i from 0 (core.rr's; strings
//!   as they are, others as JSON); `option_value(name: symbol, v: symbol)`, a row per string, number or object
//!   option, as text;
//!   `option_json(name: symbol, pointer: symbol, v: symbol)`, a row per leaf of a JSON option (an object or array,
//!   or a string holding one, as `parameters` does) at its JSON pointer (`/lineCounts/a~1b.py`), as W's
//!   `json_extract` reads it, the leaf as `option_value` renders it. A clause
//!   whose matchers carry `when: {option: …}` converts for the run's options only (its matchers that do not apply
//!   give nothing); coverage marks it: an authored clause, gated on these facts, follows every configuration.
//!   `match_all("re", sp, m)` is a built-in, never declared: `m` each match of regex `re` in span `sp`'s text; so is
//!   `match_group("re", sp, m, g, gsp)`: `gsp` the span of the named group `g` in that match;
//! - each clause it gives derives `violation("<rule-id>", "<clause-id>", "<code>", p, l)`, declared
//!   `.decl violation(rule: symbol, clause: symbol, code: symbol, path: symbol, line: number)`, the rule and clause
//!   constant and the clause one of the policy's;
//! - its helper relations and kind sets start with a prefix of their clause's. A name the converter makes is an
//!   error (`m`, `u`, `t`, `k` or `blocked` and digits; `never`, `in_scope`, `out_scope`, `report_when_gt`,
//!   `option_on`, `option_item`, `option_value`, `option_json`, `tape_syntax_error`; `strtest_*`, `matchall_*`,
//!   `matchgroup_*`, `magic_*`,
//!   `done_*`, `inside_*`, `nearest_*`), and so are a relation both derive and one name declared with other columns;
//! - a clause it derives is authored, whatever the converter made of it: its rules replace the converter's. A
//!   clause the run leaves out (the replay's `keep`) is dropped from both. The merge, less what `violation` does not
//!   reach, is printed with `rr::print`.

#[cfg(test)]
use super::front::{self, Literal, Program, Term};
use crate::code_rule_dsl::language::{GrammarDialect, KindCatalog};
use crate::code_rule_dsl::matcher::{AstMatcher, Axis, Semantic, StopBy};
use ast_grep_core::matcher::{Pattern, PatternNode};
use ast_grep_core::meta_var::MetaVariable;
use regex_syntax::hir::{Class, Hir, HirKind, Look};
use std::collections::{BTreeMap, BTreeSet};

/// Why a clause did not convert.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::code_rule_dsl) struct Gap(pub(in crate::code_rule_dsl) String);

type Res<T> = Result<T, Gap>;

fn gap<T>(what: impl Into<String>) -> Res<T> {
    Err(Gap(what.into()))
}

/// The rules one or more clauses need, and the shared declarations.
#[derive(Clone)]
pub(in crate::code_rule_dsl) struct Emitter {
    kinds: KindCatalog,
    /// Relation declarations, by name: `(name, columns)`.
    decls: BTreeMap<String, String>,
    rules: Vec<String>,
    facts: Vec<String>,
    kindsets: BTreeMap<String, BTreeSet<String>>,
    next: usize,
    /// What was said only approximately, by what.
    pub(in crate::code_rule_dsl) approx: BTreeSet<String>,
    /// The relation of each recursive utility, by its id in the tree being converted.
    utils: BTreeMap<(usize, usize), String>,
    /// The pattern captures of the rule being converted: each metavariable's variable.
    pub(in crate::code_rule_dsl) captures: BTreeMap<String, String>,
    /// Whether the empty relation `never` is used.
    never: bool,
    /// The node an enclosing `all` tests and the kinds it can have (`has` bounds its ancestor by them).
    node_kinds: Option<(String, BTreeSet<u16>)>,
}

/// A conjunction over one node variable: the literals, as text.
type Body = Vec<String>;

impl Emitter {
    pub(in crate::code_rule_dsl) fn new(dialect: GrammarDialect) -> Self {
        Self {
            kinds: dialect.kinds(),
            decls: BTreeMap::new(),
            rules: Vec::new(),
            facts: Vec::new(),
            kindsets: BTreeMap::new(),
            next: 0,
            approx: BTreeSet::new(),
            utils: BTreeMap::new(),
            captures: BTreeMap::new(),
            never: false,
            node_kinds: None,
        }
    }

    /// Converts one clause with `f`. Where it fails, all it emitted is taken back (rules, declarations, kind sets,
    /// utilities, approximations): a clause is converted whole or leaves nothing.
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn clause(&mut self, f: impl FnOnce(&mut Self) -> Res<()>) -> Res<()> {
        let saved = self.clone();
        let out = f(self);
        if out.is_err() {
            *self = saved;
        }
        out
    }

    fn fresh(&mut self, stem: &str) -> String {
        self.next += 1;
        format!("{stem}{}", self.next)
    }

    fn var(&mut self) -> String {
        self.fresh("$v")
    }

    fn kind(&self, id: u16) -> Res<String> {
        match self.kinds.kind_name(id) {
            Some(name) => Ok(name.to_owned()),
            None => gap(format!("kind id {id}")),
        }
    }

    fn field(&self, id: u16) -> Res<String> {
        match self.kinds.field_name(id) {
            Some(name) => Ok(name.to_owned()),
            None => gap(format!("field id {id}")),
        }
    }

    /// A kind set for `kinds`, by its members; its name.
    fn kindset(&mut self, kinds: &BTreeSet<u16>) -> Res<String> {
        let names: BTreeSet<String> = kinds.iter().map(|&k| self.kind(k)).collect::<Res<_>>()?;
        if names.iter().any(|k| k == "comment") {
            return gap("comment nodes (not in node)");
        }
        if let Some((name, _)) = self.kindsets.iter().find(|(_, members)| **members == names) {
            return Ok(name.clone());
        }
        let name = format!("k{}", self.kindsets.len());
        self.kindsets.insert(name.clone(), names);
        Ok(name)
    }

    /// A relation `p(n, …)` holding where `bodies` (over `$n`) hold; its atom at `n`. The variables the bodies share
    /// with the rule around them (made before `mark`: the metavariables it bound) are columns, passed in: a
    /// variable of a rule of its own is that rule's alone, so one left out would match anything.
    fn relation(&mut self, bodies: Vec<Body>, mark: usize, n: &str) -> String {
        let outer = outer_vars(&bodies, mark);
        let name = self.fresh("m");
        let columns: Vec<String> = std::iter::once("n: node".to_owned()).chain(outer.iter().map(|v| format!("{}: node", &v[1..]))).collect();
        self.decls.insert(name.clone(), columns.join(", "));
        let args = |first: &str| std::iter::once(first.to_owned()).chain(outer.iter().cloned()).collect::<Vec<_>>().join(", ");
        for body in bodies {
            // Each column bound in the body: one only negated reads every node first, as `$n` does.
            let body = outer.iter().fold(nonempty(body, "$n"), |body, v| nonempty(body, v));
            self.rules.push(format!("{name}({}) :- {}.", args("$n"), body.join(", ")));
        }
        format!("{name}({})", args(n))
    }

    /// The literals under which `m` holds at `n`.
    pub(in crate::code_rule_dsl) fn body(&mut self, m: &AstMatcher, n: &str) -> Res<Body> {
        Ok(match m {
            AstMatcher::Kind(k) => {
                let ids = kind_ids(k);
                let [k] = ids.as_slice() else { return gap("kind of no single id") };
                let k = self.kind(*k)?;
                if k == "comment" {
                    return gap("comment nodes (not in node)");
                }
                vec![format!("node(_, {n}, {}, _, _)", quote(&k))]
            }
            AstMatcher::Supertype(st) => {
                // Membership is contextual (decision 0025); the kinds a member can have stand for it.
                self.approx.insert("supertype as its kinds".into());
                let set = self.kindset(&st.kinds)?;
                let bodies = self.kindsets[&set].iter().map(|k| vec![format!("node(_, $n, {}, _, _)", quote(k))]).collect();
                vec![self.relation(bodies, 0, n)]
            }
            AstMatcher::Pattern(p) => self.pattern(p, n)?,
            AstMatcher::Constrained(c) => {
                let mut body = self.body(&c.rule, n)?;
                for (name, rule) in &c.constraints {
                    let Some(v) = self.captures.get(name).cloned() else { return gap(format!("constraint on ${name}, not captured")) };
                    body.extend(self.at_node(&v, &v, |e| e.body(rule, &v))?);
                }
                body
            }
            AstMatcher::Regex(r) => self.regex(r.as_str(), n, None)?,
            AstMatcher::All(ms) => {
                let kinds = kinds(m, 0);
                let saved = self.node_kinds.clone();
                if let Some(ks) = kinds.as_ref().filter(|ks| !ks.is_empty()) {
                    self.node_kinds = Some((n.to_owned(), ks.clone()));
                }
                let mut body = Vec::new();
                let mut emit = || -> Res<()> {
                    for t in ms {
                        match t {
                            AstMatcher::Regex(r) => body.extend(self.regex(r.as_str(), n, kinds.as_ref())?),
                            _ => body.extend(self.body(t, n)?),
                        }
                    }
                    Ok(())
                };
                let done = emit();
                self.node_kinds = saved;
                done?;
                body
            }
            AstMatcher::Any(ms) => {
                let mark = self.next;
                // A token alternative holds at no token here (`no_token`): the others are the `any`.
                let mut kept = Vec::with_capacity(ms.len());
                for t in ms {
                    match t {
                        AstMatcher::Semantic(Semantic::Tokens(test)) if token_alternative(test) && ms.len() > 1 => self.no_token(test)?,
                        _ => kept.push(t),
                    }
                }
                let bodies = kept.into_iter().map(|t| self.at_node(n, "$n", |e| e.scoped(|e| e.body(t, "$n")))).collect::<Res<Vec<_>>>()?;
                if bodies.iter().any(Vec::is_empty) {
                    return Ok(Vec::new());
                }
                if bodies.len() == 1 {
                    return Ok(rename(&bodies[0], "$n", n));
                }
                vec![self.relation(bodies, mark, n)]
            }
            AstMatcher::Not(t) => {
                let mark = self.next;
                let body = self.at_node(n, "$n", |e| e.scoped(|e| e.body(t, "$n")))?;
                if body.is_empty() {
                    return Ok(self.never(n));
                }
                // One positive literal over `$n` alone is negated as written.
                let r = match body.as_slice() {
                    [one] if single(one) && !one.contains("$v") => rename(std::slice::from_ref(one), "$n", n).remove(0),
                    _ => self.relation(vec![body], mark, n),
                };
                vec![format!("!{r}")]
            }
            AstMatcher::Inside(related, stop) => self.walk(Axis::Inside, None, related.rule(), stop, n)?,
            AstMatcher::Has(related, stop) => self.walk(Axis::Has, None, related.rule(), stop, n)?,
            AstMatcher::Precedes(related, stop) => self.walk(Axis::Precedes, None, related.rule(), stop, n)?,
            AstMatcher::Follows(related, stop) => self.walk(Axis::Follows, None, related.rule(), stop, n)?,
            AstMatcher::Field(f) => {
                let axis = if f.inside { Axis::Inside } else { Axis::Has };
                self.walk(axis, Some(f.field), &f.rule, &f.stop, n)?
            }
            AstMatcher::Util(u) => {
                let key = (u.table.as_ptr() as usize, u.index);
                let name = match self.utils.get(&key) {
                    Some(name) => name.clone(),
                    None => {
                        let name = self.fresh("u");
                        self.utils.insert(key, name.clone());
                        self.decls.insert(name.clone(), "n: node".to_owned());
                        // A utility's relation serves every caller: no caller's node kinds hold in it, and it reads no
                        // metavariable a caller bound.
                        let saved = self.node_kinds.take();
                        let mark = self.next;
                        let body = u.with_target(|rule| self.scoped(|e| e.body(rule, "$n"))).ok_or_else(|| Gap("utility unavailable".into()));
                        self.node_kinds = saved;
                        let body = body??;
                        if !outer_vars(std::slice::from_ref(&body), mark).is_empty() {
                            return gap("a utility reading a metavariable bound outside it");
                        }
                        self.rules.push(format!("{name}($n) :- {}.", nonempty(body, "$n").join(", ")));
                        name
                    }
                };
                vec![format!("{name}({n})")]
            }
            AstMatcher::NthChild(c) => {
                if c.of_rule.is_some() || c.reverse || c.position.step != 0 || c.position.offset < 1 {
                    return gap("nthChild other than a fixed position");
                }
                self.approx.insert("nthChild among named children".into());
                vec![format!("child(_, {n}, {})", c.position.offset - 1)]
            }
            AstMatcher::Semantic(Semantic::FilePath(condition)) => {
                let (d, p) = (self.var(), self.var());
                let mut body = vec![format!("node({d}, {n}, _, _, _)"), format!("doc({d}, {p}, _)")];
                body.extend(self.condition(condition, &p)?);
                body
            }
            // A neighbour's "no token beside it" (`TokenTest::none`, RER-131): siblings are read among
            // named children already, which is that approximation.
            AstMatcher::Semantic(Semantic::Tokens(test))
                if test.tests.is_empty() && test.counts.is_empty() && test.rule.is_none() && test.capture.is_none() && !test.none.is_empty() =>
            {
                self.approx.insert("siblings among named children".into());
                Vec::new()
            }
            // A token test (RER-131: a relation to a regex reaches tokens through one, `any: [named,
            // tokens]`). As before tokens left the records, a regex is read on names, the only text
            // the relations hold: one that names an operator or keyword is a gap, any other holds
            // at no token, and the named alternative is the relation.
            AstMatcher::Semantic(Semantic::Tokens(test)) if token_alternative(test) => {
                self.no_token(test)?;
                self.never(n)
            }
            AstMatcher::Semantic(other) => return gap(format!("semantic {}", semantic_name(other))),
            AstMatcher::Call(_) => return gap("utility call"),
            AstMatcher::Compare(_) => return gap("compare"),
            AstMatcher::Count(_) => return gap("count"),
            AstMatcher::Never => self.never(n),
            #[cfg(test)]
            AstMatcher::TestUnknown(_) => return gap("three-valued"),
        })
    }

    /// A token test read as before tokens left the records (RER-131): a regex is read on names, the
    /// only text the relations hold, so one that names an operator or keyword is a gap, and any
    /// other holds at no token.
    fn no_token(&mut self, test: &crate::code_rule_dsl::matcher::TokenTest) -> Res<()> {
        for (_, regex) in &test.tests {
            let hir = regex_syntax::ParserBuilder::new().build().parse(regex.as_str()).map_err(|e| Gap(format!("regex {}: {e}", regex.as_str())))?;
            if let Some(Shape::Names(set)) = shape(&hir)
                && set.iter().any(|w| !w.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') || KEYWORDS.contains(&w.as_str()))
            {
                return gap("a regex on an anonymous token");
            }
        }
        self.approx.insert("a regex on a node of any kind as a name".into());
        Ok(())
    }

    fn never(&mut self, n: &str) -> Body {
        self.never = true;
        vec![format!("never({n})")]
    }

    /// Runs `f` with the captures as they are now, restoring them after: a sub-rule in a relation of its own
    /// does not share its variables with the rule that reads it.
    fn scoped<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let saved = self.captures.clone();
        let out = f(self);
        self.captures = saved;
        out
    }

    /// Runs `f` converting a rule at the node named `inner`, the node `n` here (a helper relation's `$n`) or another
    /// (`n` = `inner`): what the enclosing `all` says of the node's kinds holds there only if that node is `n`.
    fn at_node<T>(&mut self, n: &str, inner: &str, f: impl FnOnce(&mut Self) -> T) -> T {
        let saved = self.node_kinds.clone();
        self.node_kinds = saved.clone().filter(|(node, _)| node == n).map(|(_, kinds)| (inner.to_owned(), kinds));
        let out = f(self);
        self.node_kinds = saved;
        out
    }

    /// A text condition (`regex`, `notRegex`) over the text in `s`.
    pub(in crate::code_rule_dsl) fn condition(&mut self, c: &crate::code_rule_dsl::language::TextCondition, s: &str) -> Res<Body> {
        let mut body = Vec::new();
        if let Some(r) = &c.regex {
            body.extend(self.tests(r.as_str(), s, &DOC, Text::Path)?);
        }
        if let Some(r) = &c.not_regex {
            let t = self.tests(r.as_str(), "$s", &DOC, Text::Path)?;
            let rel = self.fresh("t");
            self.decls.insert(rel.clone(), "s: sym".to_owned());
            // The texts it holds for are those of `doc`'s paths and modules.
            for column in DOC {
                let mut b = vec![column.to_owned()];
                b.extend(t.clone());
                self.rules.push(format!("{rel}($s) :- {}.", b.join(", ")));
            }
            body.push(format!("!{rel}({s})"));
        }
        Ok(body)
    }

    /// `inside`, `has`, `precedes`, `follows`.
    fn walk(&mut self, axis: Axis, field: Option<u16>, rule: &AstMatcher, stop: &StopBy, n: &str) -> Res<Body> {
        let m = self.var();
        let mut body = Vec::new();
        let field = field.map(|f| self.field(f)).transpose()?;
        match (axis, stop) {
            (Axis::Inside, StopBy::Neighbor) => match &field {
                Some(f) => body.push(format!("field({m}, {}, {n})", quote(f))),
                None => body.push(format!("child({m}, {n}, _)")),
            },
            (Axis::Has, StopBy::Neighbor) => match &field {
                Some(f) => body.push(format!("field({n}, {}, {m})", quote(f))),
                None => body.push(format!("child({n}, {m}, _)")),
            },
            (Axis::Inside | Axis::Has, StopBy::End | StopBy::Rule(_)) => {
                let Some(ks) = kinds(rule, 0) else { return gap(format!("{axis:?} to a rule of any kind")) };
                let set = self.kindset(&ks)?;
                let (low, high) = if axis == Axis::Inside { (n.to_owned(), m.clone()) } else { (m.clone(), n.to_owned()) };
                // The stop rule's kinds, where it tests only kinds: a stop is then a node of those kinds.
                let stop_kinds = match stop {
                    StopBy::Rule(r) if kind_only(r, 0) => kinds(r, 0),
                    _ => None,
                };
                // `inside` stopping at the first node of kinds that include the target's is the nearest stop
                // (decision 0038): a target further up has that stop between, so only the nearest can be one, and the
                // rule's own test (below) says whether it is. A built-in over the tape, not `inside` less a `blocked`
                // relation of all pairs with one between.
                let nearest = axis == Axis::Inside && stop_kinds.as_ref().is_some_and(|stops| ks.is_subset(stops));
                // `has` from a node of known kinds (its enclosing `all`'s): the ancestor is one of those kinds over
                // the descendant, from the tape. Not every node within it, nor a set on the ancestor of the
                // descendant's kinds, which the rule's own test (below) checks.
                let ancestors = match &self.node_kinds {
                    Some((node, kinds)) if axis == Axis::Has && node == n => self.kindset(&kinds.clone()).ok(),
                    _ => None,
                };
                if nearest {
                    let stops = self.kindset(stop_kinds.as_ref().expect("kind-only stops"))?;
                    body.push(format!("nearest({low}, {high}, {stops})"));
                } else if axis == Axis::Inside {
                    body.push(format!("inside({low}, {high}, {set})"));
                } else if let Some(ancestors) = &ancestors {
                    body.push(format!("inside({low}, {high}, {ancestors})"));
                } else {
                    // `has` from a node of unknown kinds: the descendant is any node within it.
                    body.push(within_strict(&low, &high));
                }
                if let Some(f) = &field {
                    // The path enters `high` through `f`: `low` lies within that field's child.
                    let c = self.var();
                    body.push(format!("field({high}, {}, {c})", quote(f)));
                    body.extend(within(&low, &c, self));
                }
                if let StopBy::Rule(stop) = stop
                    && !nearest
                {
                    let Some(stops) = kinds(stop, 0) else { return gap("a stop rule of any kind") };
                    let stops = self.kindset(&stops)?;

                    let blocked = self.fresh("blocked");
                    self.decls.insert(blocked.clone(), "low: node, high: node".to_owned());
                    // A stop node strictly between: over `low`, under `high`.
                    let mut b = Vec::new();
                    if kind_only(stop, 0) {
                        // A stop is a node of the stop kinds: one lies between where the nearest one over `low`
                        // lies under `high` (any other is further up). One row a `low` from the tape's nearest
                        // ancestors (decision 0038), not every pair of a stop and an ancestor.
                        b.push(format!("nearest($low, $s, {stops})"));
                    } else {
                        b.push(format!("inside($low, $s, {stops})"));
                        let mark = self.next;
                        let is_stop = self.at_node("$s", "$s", |e| e.scoped(|e| e.body(stop, "$s")))?;
                        if !outer_vars(std::slice::from_ref(&is_stop), mark).is_empty() {
                            return gap("a stop rule reading a metavariable bound outside it");
                        }
                        b.extend(is_stop);
                    }
                    // Under `high`: for `inside` an ancestor of the target's kinds over the stop; for `has` an ancestor
                    // of the node's kinds where known (bounded by the tape), else any node the stop lies within.
                    match (axis, &ancestors) {
                        (Axis::Inside, _) => b.push(format!("inside($s, $high, {set})")),
                        (_, Some(ancestors)) => b.push(format!("inside($s, $high, {ancestors})")),
                        _ => b.push(within_strict("$s", "$high")),
                    }
                    self.rules.push(format!("{blocked}($low, $high) :- {}.", b.join(", ")));
                    body.push(format!("!{blocked}({low}, {high})"));
                }
            }
            (Axis::Precedes | Axis::Follows, StopBy::Neighbor | StopBy::End) => {
                // Named siblings only: anonymous ones are not in `child`.
                self.approx.insert("siblings among named children".into());
                let (p, i, j) = (self.var(), self.var(), self.var());
                body.push(format!("child({p}, {n}, {i})"));
                body.push(format!("child({p}, {m}, {j})"));
                let (before, after) = if axis == Axis::Precedes { (&i, &j) } else { (&j, &i) };
                match stop {
                    StopBy::Neighbor => body.push(format!("{after} = @add({before}, 1)")),
                    _ => body.push(format!("{before} < {after}")),
                }
            }
            (Axis::Precedes | Axis::Follows, StopBy::Rule(_)) => return gap("sibling walk with a stop rule"),
        }
        if matches!(axis, Axis::Has | Axis::Inside) && matches!(stop, StopBy::Neighbor) {
            self.approx.insert("neighbours among named children".into());
        }
        body.extend(self.at_node(&m, &m, |e| e.body(rule, &m))?);
        Ok(body)
    }

    /// A pattern at `n`: its kinds and named children by position, its identifiers' text, its captures.
    fn pattern(&mut self, p: &Pattern, n: &str) -> Res<Body> {
        let mut body = Vec::new();
        let mut captures = std::mem::take(&mut self.captures);
        let r = self.pattern_node(&p.node, n, &mut captures, &mut body);
        self.captures = captures;
        r?;
        Ok(body)
    }

    fn pattern_node(&mut self, p: &PatternNode, n: &str, captures: &mut BTreeMap<String, String>, body: &mut Body) -> Res<()> {
        match p {
            PatternNode::MetaVar { meta_var } => match meta_var {
                MetaVariable::Capture(name, _) => match captures.get(name).cloned() {
                    // Bound before: the same name, as ast-grep compares their text.
                    Some(prior) if prior != n => {
                        self.approx.insert("a metavariable twice: equal names".into());
                        let t = self.var();
                        body.push(format!("text({prior}, {t})"));
                        body.push(format!("text({n}, {t})"));
                    }
                    Some(_) => {}
                    None => {
                        captures.insert(name.clone(), n.to_owned());
                    }
                },
                MetaVariable::Dropped(_) => {}
                MetaVariable::Multiple | MetaVariable::MultiCapture(_) => return gap("an ellipsis outside a list"),
            },
            PatternNode::Terminal { text, is_named, kind_id } => {
                if !is_named {
                    return gap("a pattern of one anonymous token");
                }
                let kind = self.kind(*kind_id)?;
                body.push(format!("node(_, {n}, {}, _, _)", quote(&kind)));
                match kind.as_str() {
                    "identifier" | "dotted_name" | "type_identifier" | "field_identifier" => body.push(format!("text({n}, {})", quote(text))),
                    "integer" => match text.replace('_', "").parse::<i64>() {
                        Ok(v) => body.push(format!("intval({n}, {v})")),
                        Err(_) => return gap("an integer pattern"),
                    },
                    "true" | "false" | "none" | "ellipsis" => {}
                    other => return gap(format!("a {other} literal in a pattern")),
                }
            }
            PatternNode::Internal { kind_id, children } => {
                let kind = self.kind(*kind_id)?;
                if kind == "string" || kind == "concatenated_string" {
                    return gap("a string literal in a pattern");
                }
                body.push(format!("node(_, {n}, {}, _, _)", quote(&kind)));
                // Operators are the node's `op`; other anonymous tokens follow from the kinds.
                for c in children {
                    if let PatternNode::Terminal { text, is_named: false, .. } = c
                        && matches!(kind.as_str(), "binary_operator" | "augmented_assignment" | "comparison_operator" | "boolean_operator" | "unary_operator")
                    {
                        body.push(format!("op({n}, {})", quote(text)));
                    }
                }
                let named: Vec<&PatternNode> = children.iter().filter(|c| !matches!(c, PatternNode::Terminal { is_named: false, .. })).collect();
                let ellipsis: Vec<usize> = named
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| matches!(c, PatternNode::MetaVar { meta_var: MetaVariable::Multiple | MetaVariable::MultiCapture(_) }))
                    .map(|(i, _)| i)
                    .collect();
                let fixed = match ellipsis.as_slice() {
                    [] => named.len(),
                    [at] if *at + 1 == named.len() => *at,
                    _ => return gap("an ellipsis before other children"),
                };
                for (i, c) in named.iter().take(fixed).enumerate() {
                    let v = self.var();
                    body.push(format!("child({n}, {v}, {i})"));
                    self.pattern_node(c, &v, captures, body)?;
                }
                if ellipsis.is_empty() {
                    body.push(format!("!child({n}, _, {fixed})"));
                }
                self.approx.insert("patterns by named children".into());
            }
        }
        Ok(())
    }

    /// A regex on a node's text as string tests. `kinds` are the node's kinds where its rule names them.
    fn regex(&mut self, source: &str, n: &str, kinds: Option<&BTreeSet<u16>>) -> Res<Body> {
        let hir = regex_syntax::ParserBuilder::new().build().parse(source).map_err(|e| Gap(format!("regex {source}: {e}")))?;
        let names: Option<BTreeSet<String>> = kinds.map(|k| k.iter().filter_map(|&k| self.kinds.kind_name(k).map(str::to_owned)).collect());
        let text_kinds = names.as_ref().is_some_and(|k| !k.is_empty() && k.iter().all(|k| matches!(k.as_str(), "identifier" | "dotted_name" | "type_identifier" | "field_identifier")));
        match shape(&hir) {
            Some(Shape::Anything) => return Ok(Vec::new()),
            Some(Shape::Nothing) => return Ok(self.never(n)),
            _ => {}
        }
        // `async def`: the definition's modifier.
        if names.as_ref().is_some_and(|k| k.len() == 1 && k.contains("function_definition")) {
            match source {
                r"^async\b" | r"^async\s" => return Ok(vec![format!("op({n}, \"async\")")]),
                r"^def\b" | r"^def\s" => return Ok(vec![format!("!op({n}, \"async\")")]),
                _ => {}
            }
        }
        // A string literal: its prefix and content. A rule that names no kind but whose regex reads a string's
        // prefix and quotes is about strings too.
        if names.as_ref().is_some_and(|k| k.len() == 1 && k.contains("string")) {
            return self.string_regex(&hir, source, n);
        }
        if names.is_none()
            && let Ok(body) = self.string_regex(&hir, source, n)
        {
            return Ok(body);
        }
        // A name: an identifier's text or a dotted attribute's (`name`). A rule that names no kind is read
        // as testing a name, the only text the relations hold.
        let name_kinds = names.as_ref().is_none_or(|k| k.iter().all(|k| matches!(k.as_str(), "identifier" | "attribute" | "dotted_name" | "type_identifier" | "field_identifier")));
        if !text_kinds && name_kinds {
            // Operators and keywords are anonymous tokens, which no relation holds as a node.
            if names.is_none()
                && let Some(Shape::Names(set)) = shape(&hir)
                && set.iter().any(|w| !w.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') || KEYWORDS.contains(&w.as_str()))
            {
                return gap("a regex on an anonymous token");
            }
            if names.is_none() {
                self.approx.insert("a regex on a node of any kind as a name".into());
            }
            let s = self.var();
            let mut body = vec![format!("name({n}, {s})")];
            body.extend(self.tests(source, &s, &["name(_, $s)"], Text::Name)?);
            return Ok(body);
        }
        // A module's text, exactly: W's regex searches the file's text (from its first token, comments and strings
        // included), which `node_text` holds; `rx` is that search.
        if names.as_ref().is_some_and(|k| k.len() == 1 && k.contains("module")) {
            let s = self.var();
            return Ok(vec![format!("node_text({n}, {s})"), format!("@rx({s}, {})", quote(source))]);
        }
        if !text_kinds {
            return gap(format!("a regex on the text of {} ({source})", names.map_or("any node".to_owned(), |k| k.into_iter().collect::<Vec<_>>().join("|"))));
        }
        let s = self.var();
        let mut body = vec![format!("text({n}, {s})")];
        body.extend(self.tests(source, &s, &["text(_, $s)"], Text::Name)?);
        Ok(body)
    }

    /// A regex over the text in `s` as text tests (decision 0037): a set (`$s in {…}`), prefix, suffix and
    /// substring tests, or globs, exact for the kind of text. `domain`: the literals that give the strings tested
    /// (each over `$s`), for a relation of alternatives.
    fn tests(&mut self, source: &str, s: &str, domain: &[&str], text: Text) -> Res<Body> {
        let hir = regex_syntax::ParserBuilder::new().build().parse(source).map_err(|e| Gap(format!("regex {source}: {e}")))?;
        self.tests_hir(&hir, source, s, domain, text)
    }

    fn tests_hir(&mut self, hir: &Hir, source: &str, s: &str, domain: &[&str], text: Text) -> Res<Body> {
        let mut body = Vec::new();
        match shape(hir) {
            Some(Shape::Anything) => {}
            Some(Shape::Nothing) => return Ok(self.never("_")),
            Some(Shape::Names(set)) => body.push(member(s, set)),
            Some(Shape::Tests(tests)) => {
                // One test per kind of test, each over all its literals.
                let mut by_op: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
                for (op, lit) in tests {
                    by_op.entry(op).or_default().insert(lit);
                }
                let literal = |op: &str, set: BTreeSet<String>, s: &str| {
                    if op == "equals" { member(s, set) } else { format!("@{op}({s}, {})", set_term(set)) }
                };
                if by_op.len() == 1 {
                    let (op, set) = by_op.into_iter().next().expect("one test");
                    body.push(literal(op, set, s));
                } else {
                    // A relation over the strings the rules read (names, paths, modules), one rule per kind of test.
                    let r = self.fresh("t");
                    self.decls.insert(r.clone(), "s: sym".to_owned());
                    for (op, set) in by_op {
                        for column in domain {
                            self.rules.push(format!("{r}($s) :- {column}, {}.", literal(op, set.clone(), "$s")));
                        }
                    }
                    body.push(format!("{r}({s})"));
                }
            }
            None => match globs(hir, text) {
                Some((globs, fold, approx)) => {
                    if let Some(a) = approx {
                        self.approx.insert(a.to_owned());
                    }
                    body.push(format!("@{}glob({s}, {})", if fold { "i" } else { "" }, set_term(globs)));
                }
                None => return gap(format!("regex {source}")),
            },
        }
        Ok(body)
    }

    /// A regex on a string literal's source text (quotes and prefix included) as tests on its prefix
    /// (`strprefix`) and content (`str`): `^[rRuU]*(?:'(?:a|b)'|"(?:a|b)")$`, `^[rub]*["']{1,3}x`, `(?i)^[rub]*f`.
    fn string_regex(&mut self, hir: &Hir, source: &str, n: &str) -> Res<Body> {
        let fail = || gap(format!("a regex on the text of string ({source})"));
        let parts: Vec<Hir> = match hir.kind() {
            HirKind::Concat(parts) => parts.clone(),
            _ => return fail(),
        };
        if !matches!(parts.first().map(Hir::kind), Some(HirKind::Look(Look::Start))) {
            return fail();
        }
        let mut i = 1;
        // The prefix letters the regex allows (Python's are r, u, b, f; the relation holds them lowercased).
        let mut allowed = BTreeSet::new();
        if let Some(HirKind::Repetition(r)) = parts.get(i).map(Hir::kind)
            && r.min == 0
            && let HirKind::Class(Class::Unicode(c)) = r.sub.kind()
        {
            for range in c.ranges() {
                for ch in range.start()..=range.end() {
                    if !"rubfRUBF".contains(ch) {
                        return fail();
                    }
                    allowed.insert(ch.to_ascii_lowercase());
                }
            }
            i += 1;
        }
        let p = self.var();
        let mut body = vec![format!("strprefix({n}, {p})")];
        let rest = &parts[i..];
        // `(?i)^[rub]*f`: an f-string, whatever else its prefix holds.
        if let [h] = rest
            && let Some(f) = letter(h)
        {
            let mut with = allowed.clone();
            with.insert(f);
            if "rubf".chars().all(|c| with.contains(&c)) {
                body.push(format!("@contains({p}, {})", quote(&f.to_string())));
                return Ok(body);
            }
            return fail();
        }
        for letter in "rubf".chars().filter(|c| !allowed.contains(c)) {
            body.push(format!("!@contains({p}, {})", quote(&letter.to_string())));
        }
        if allowed.contains(&'f') {
            self.approx.insert("an interpolated string has no content".into());
        }
        const QUOTES: [&str; 4] = ["'''", "\"\"\"", "'", "\""];
        let content: Hir = match rest {
            // `(?:'X'|"X"|…)$`: the content is X, whole.
            [alt, end] if matches!(end.kind(), HirKind::Look(Look::End)) => {
                let mut xs = Vec::new();
                for b in alternatives(alt) {
                    let HirKind::Concat(bp) = b.kind() else { return fail() };
                    let (Some(open), Some(close)) = (bp.first().and_then(lit), bp.last().and_then(lit)) else { return fail() };
                    if open != close || !QUOTES.contains(&open.as_str()) || bp.len() < 2 {
                        return fail();
                    }
                    xs.push(Hir::concat(bp[1..bp.len() - 1].to_vec()));
                }
                Hir::concat(vec![Hir::look(Look::Start), Hir::alternation(xs), Hir::look(Look::End)])
            }
            // `["']{1,3}Y`: the content starts with Y.
            [q, tail @ ..] if is_quote_class(q) => {
                let mut v = vec![Hir::look(Look::Start)];
                match tail {
                    // `["']X["']$`: the content is X, whole.
                    [x @ .., close, end] if is_quote_class(close) && matches!(end.kind(), HirKind::Look(Look::End)) => {
                        v.extend(x.iter().cloned());
                        v.push(Hir::look(Look::End));
                    }
                    _ => v.extend(tail.iter().cloned()),
                }
                Hir::concat(v)
            }
            _ => return fail(),
        };
        let v = self.var();
        body.push(format!("str({n}, {v})"));
        body.extend(self.tests_hir(&content, source, &v, &["str(_, $s)"], Text::Content)?);
        Ok(body)
    }

    /// Adds a rule `head :- body` and its head's declaration.
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn rule(&mut self, head: &str, columns: &str, body: Body) {
        let name = head.split('(').next().unwrap_or(head).to_owned();
        self.decls.insert(name, columns.to_owned());
        let rule = format!("{head} :- {}.", body.join(", "));
        if !self.rules.contains(&rule) {
            self.rules.push(rule);
        }
    }

    /// The program: declarations, kind sets, facts, rules.
    pub(in crate::code_rule_dsl) fn program(&self, outputs: &[&str]) -> String {
        let mut out = String::new();
        // The base relations the rules read: one declared and not read has no statistics.
        let text: String = self.rules.join("\n");
        let read: Vec<&str> = BASE_RELATIONS.iter().copied().filter(|r| text.contains(&format!("{r}("))).collect();
        for line in BASE.lines() {
            if let Some(rel) = line.strip_prefix(".decl ").and_then(|d| d.split('(').next())
                && read.contains(&rel)
            {
                out.push_str(line);
                out.push('\n');
            }
        }
        if !read.is_empty() {
            out.push_str(&format!(".input {}\n", read.join(", ")));
        }
        if self.never {
            out.push_str(".decl never(n: node)\n");
        }
        for (name, kinds) in &self.kindsets {
            out.push_str(&format!(".kindset {name} = {}\n", kinds.iter().map(|k| quote(k)).collect::<Vec<_>>().join(" | ")));
        }
        for (name, columns) in &self.decls {
            out.push_str(&format!(".decl {name}({columns})\n"));
        }
        for f in &self.facts {
            out.push_str(f);
            out.push('\n');
        }
        for r in &self.rules {
            out.push_str(r);
            out.push('\n');
        }
        if !outputs.is_empty() {
            out.push_str(&format!(".output {}\n", outputs.join(", ")));
        }
        out
    }
}

/// The base relations every converted program reads, and those the corpus runner binds for an authored file beyond
/// them (core.rr's, `facts::vtab::syntax::Extra`): the source text, its spans and node columns T had not; and W's
/// Python reference graph (`logic::plan::corpus::references`).
pub(in crate::code_rule_dsl) const BASE: &str = "\
.decl doc(d: int, path: sym, module: sym)
.decl node(d: int, n: node, kind: sym, start: int, end: int)
.decl child(p: node, c: node, i: int)
.decl field(p: node, f: sym, c: node)
.decl text(n: node, s: sym)
.decl name(n: node, s: sym)
.decl str(n: node, s: sym)
.decl strprefix(n: node, s: sym)
.decl intval(n: node, v: int)
.decl op(n: node, s: sym)
.decl line(n: node, l: int)
.decl syntax_error(d: int)
.decl source(d: int, sp: node)
.decl src_line(d: int, l: int, sp: node)
.decl src_line_lf(d: int, l: int, sp: node)
.decl tape_line(d: int, l: int, sp: node)
.decl line_count(d: int, k: int)
.decl end_line(n: node, l: int)
.decl col(n: node, c: int)
.decl node_text(n: node, s: sym)
.decl pkg(d: int, p: sym)
.decl docof(n: node, d: int)
.decl op_at(n: node, i: int, s: sym)
.decl prev_token(n: node, s: sym)
.decl token(p: node, i: int, s: sym, start: int, end: int)
.decl visit_index(n: node, i: int)
.decl path_part(d: int, s: sym)
.decl file_name(d: int, s: sym)
.decl value(n: node, v: sym)
.decl static_value(n: node, v: sym)
.decl value_type_of(n: node, t: sym)
.decl value_type(v: sym, t: sym)
.decl comment(n: node, s: sym)
.decl fstring_piece(n: node, i: int, kind: sym, v: sym)
.decl fstring_template(n: node, marker: sym, v: sym)
.decl cpython_syntax_error(d: int, l: int, msg: sym)
.decl string_annotation(s: node)
.decl parsed(s: node, grammar: sym, root: node)
.decl literal_json(n: node, v: sym)
.decl cpython_unparse(n: node, s: sym)
.decl cpython_ast_dump(n: node, s: sym)
.decl dump_sha256(n: node, h: sym)
.decl body_sha256(f: node, h: sym)
.decl signature_sha256(f: node, h: sym)
.decl reference_edge(src: node, dst: node, via: sym)
.decl reference_open(n: node)
";

const BASE_RELATIONS: [&str; 13] = ["doc", "node", "child", "field", "text", "name", "str", "strprefix", "intval", "op", "line", "syntax_error", "node_text"];

/// Python's keywords: tokens, never names.
const KEYWORDS: [&str; 35] = [
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except",
    "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try",
    "while", "with", "yield",
];

/// The strings a path or module condition tests.
const DOC: [&str; 2] = ["doc(_, $s, _)", "doc(_, _, $s)"];

/// `prog` less what its outputs do not reach: the rules (`front::prune`), then the declarations, inputs, facts and
/// kind sets no rule left reads or derives.
#[cfg(test)]
pub(in crate::code_rule_dsl) fn reachable(prog: Program) -> Program {
    let mut prog = front::prune(prog);
    let mut used: BTreeSet<String> = prog.outputs.iter().cloned().collect();
    let mut sets = BTreeSet::new();
    for r in &prog.rules {
        used.insert(r.head.rel.clone());
        for l in &r.body {
            if let Some(a) = l.atom() {
                used.insert(a.rel.clone());
                if let Some((set, _)) = front::axis(&a.rel) {
                    sets.insert(set.to_owned());
                }
            }
            if let Literal::Nearest { kindset, .. } = l {
                sets.insert(kindset.clone());
            }
        }
    }
    prog.decls.retain(|r, _| used.contains(r));
    prog.inputs.retain(|r| used.contains(r));
    prog.facts.retain(|r, _| used.contains(r));
    prog.lattice.retain(|r, _| used.contains(r));
    prog.kindsets.retain(|k, _| sets.contains(k));
    prog
}

/// Where a clause's rules come from.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) enum Origin {
    /// The converter's, from the clause's matcher.
    Converted,
    /// The rule's authored file (the overlay).
    Authored,
}

/// The columns of `violation(rule, clause, code, path, line)`.
#[cfg(test)]
const VIOLATION: [&str; 5] = ["sym", "sym", "sym", "sym", "int"];

/// What the converter gives an authored file to read: the scope, the report gate, the options and the tape's verdict.
#[cfg(test)]
pub(in crate::code_rule_dsl) use super::plan::corpus::GIVEN;

/// The column types of what the converter gives (`GIVEN`), where `rel` is one.
#[cfg(test)]
fn given_types(rel: &str) -> Option<Vec<String>> {
    let types: &[&str] = match rel {
        "in_scope" | "out_scope" | "report_when_gt" | "tape_syntax_error" => &["int"],
        "option_on" => &["sym"],
        "option_item" => &["sym", "int", "sym"],
        "option_value" => &["sym", "sym"],
        "option_json" => &["sym", "sym", "sym"],
        _ => return None,
    };
    Some(types.iter().map(|t| (*t).to_owned()).collect())
}

/// A base relation's column types (`BASE`), where `rel` is one.
#[cfg(test)]
fn base_types(rel: &str) -> Option<Vec<String>> {
    BASE.lines().find_map(|line| {
        let columns = line.strip_prefix(".decl ")?.strip_prefix(rel)?.strip_prefix('(')?.strip_suffix(')')?;
        Some(columns.split(", ").map(|c| c.split(": ").nth(1).unwrap_or_default().to_owned()).collect())
    })
}

/// The names the converter makes: an authored file declares none of them, but for reading the scope.
#[cfg(test)]
fn converter_name(rel: &str) -> bool {
    let numbered = |stem: &str| rel.strip_prefix(stem).is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    ["m", "u", "t", "k", "blocked"].into_iter().any(numbered)
        || ["never"].contains(&rel)
        || GIVEN.contains(&rel)
        || ["strtest_", "matchall_", "matchgroup_", "magic_", "done_", "inside_", "nearest_"].iter().any(|p| rel.starts_with(p))
}

#[cfg(test)]
fn types(columns: &[(String, String)]) -> Vec<&str> {
    columns.iter().map(|(_, t)| t.as_str()).collect()
}

/// Column types in `.rr` spellings, for the errors.
#[cfg(test)]
fn spelled<'a>(types: impl IntoIterator<Item = &'a str>) -> String {
    let spell = |t: &'a str| match t {
        "int" => "number",
        "sym" => "symbol",
        other => other,
    };
    types.into_iter().map(spell).collect::<Vec<_>>().join(", ")
}

/// The rule and clause a `violation` rule derives, where both are constants.
#[cfg(test)]
fn violation_of(r: &front::Rule) -> Option<(&str, &str)> {
    match r.head.args.as_slice() {
        [Term::Str(rule), Term::Str(clause), ..] if r.head.rel == "violation" => Some((rule, clause)),
        _ => None,
    }
}

#[cfg(test)]
fn push_new(list: &mut Vec<String>, items: impl IntoIterator<Item = String>) {
    for item in items {
        if !list.contains(&item) {
            list.push(item);
        }
    }
}

/// The authored overlay (module doc): rule `rule`'s authored program over its converted one. `clauses`: the policy's
/// clause ids; `keep`: the clauses this run keeps (the replay's, per event). The merged program, with only what
/// `violation` reaches, and the clauses the authored program derives (kept or not).
#[cfg(test)]
pub(in crate::code_rule_dsl) fn overlay(mut converted: Program, mut authored: Program, rule: &str, clauses: &[String], keep: &dyn Fn(&str) -> bool) -> Result<(Program, BTreeSet<String>), String> {
    let err = |m: String| Err(format!("authored {rule}: {m}"));
    // The prelude's declarations the file does not use are core.rr's, not the file's (`rr::drop_unread_core`).
    super::rr::drop_unread_core(&mut authored);
    // `.global library …` is what reading the file wrote of the libraries it includes (decision 0091), not a
    // directive of its own: it goes with the merged program.
    let (library, settings): (Vec<_>, Vec<_>) = std::mem::take(&mut authored.settings).into_iter().partition(|(key, _)| key == front::GLOBAL);
    if !authored.policies.is_empty() || !settings.is_empty() {
        return err("`.policy` and setting directives are the policy's, not the overlay's".into());
    }
    converted.settings.extend(library);
    if let Some(o) = authored.outputs.iter().find(|o| *o != "violation") {
        return err(format!(".output {o}: the only output is violation"));
    }
    let axis = |rel: &str| front::axis(rel).is_some_and(|(set, _)| authored.kindsets.contains_key(set));
    // Declarations: the base relations, `violation` and the scope as the converter declares them, and no name the
    // converter makes.
    for (name, columns) in &authored.decls {
        if axis(name) {
            continue;
        }
        if matches!(name.as_str(), "match_all" | "match_group") {
            return err(format!("{name} is a built-in constraint (match_all(\"re\", sp, m), match_group(\"re\", sp, m, g, gsp)): it is not declared"));
        }
        let want = match name.as_str() {
            "violation" => Some(VIOLATION.map(str::to_owned).to_vec()),
            _ => given_types(name).or_else(|| base_types(name)),
        };
        match want {
            Some(want) if types(columns) != want => {
                return err(format!("{name}({}) is declared with other columns than the converter's ({})", spelled(types(columns)), spelled(want.iter().map(String::as_str))));
            }
            None if converter_name(name) => {
                return err(format!("{name} is a name the converter makes (m, u, t, k or blocked and digits; never, strtest_*, matchall_*, magic_*, done_*, inside_*, nearest_*): name it after its clause"));
            }
            _ => {}
        }
    }
    if let Some(set) = authored.kindsets.keys().find(|k| converter_name(k) || base_types(k).is_some()) {
        return err(format!("kind set {set} has a name the converter makes: name it after its clause"));
    }
    // What it derives: never a base relation or the scope; each relation declared, each one read given by
    // something; each `violation` at a constant clause of this rule.
    let derived: BTreeSet<&str> = authored.rules.iter().map(|r| r.head.rel.as_str()).chain(authored.facts.keys().map(String::as_str)).collect();
    if let Some(rel) = derived.iter().find(|r| base_types(r).is_some() || GIVEN.contains(*r)) {
        return err(format!("{rel} is derived here, but the corpus or the converter gives it: read it only"));
    }
    if let Some(rel) = derived.iter().find(|r| !authored.decls.contains_key(**r)) {
        return err(format!("{rel} is derived and not declared"));
    }
    let mut authored_clauses = BTreeSet::new();
    for r in &authored.rules {
        // `match_all("re", sp, m)` and `match_group("re", sp, m, g, gsp)` are built-ins, which the runner makes
        // relations of (`strings::as_relations`).
        for a in r.body.iter().filter_map(Literal::atom).filter(|a| !matches!(a.rel.as_str(), "match_all" | "match_group")) {
            let rel = a.rel.as_str();
            if !authored.decls.contains_key(rel) {
                return err(format!("{rel} is read and not declared"));
            }
            // A bridged relation (`typing_<program>__<output>`, the library's calls) is the runner's to give.
            if !(derived.contains(rel) || base_types(rel).is_some() || GIVEN.contains(&rel) || authored.inputs.iter().any(|i| i == rel) || super::plan::corpus::bridge::is_bridged(rel)) {
                return err(format!("{rel} is read, and nothing derives or inputs it"));
            }
        }
        if r.head.rel == "violation" {
            let Some((of, clause)) = violation_of(r) else { return err(format!("a violation whose rule and clause are not constants: {r}")) };
            if of != rule {
                return err(format!("a violation of rule {of:?}"));
            }
            if !clauses.iter().any(|c| c == clause) {
                return err(format!("a violation of clause {clause:?}, which the policy has not"));
            }
            authored_clauses.insert(clause.to_owned());
        }
    }
    // The converter's rules for the authored clauses give way; nothing else may be derived by both.
    converted.rules.retain(|r| violation_of(r).is_none_or(|(_, c)| !authored_clauses.contains(c)));
    let both = {
        let defined: BTreeSet<&str> = converted.rules.iter().map(|r| r.head.rel.as_str()).chain(converted.facts.keys().map(String::as_str)).collect();
        derived.iter().find(|r| **r != "violation" && defined.contains(**r)).map(|r| (*r).to_owned())
    };
    if let Some(rel) = both {
        return err(format!("{rel} is derived by both the converter and the overlay"));
    }
    // The base relations it reads are inputs, declared `.input` or not.
    let reads_base: Vec<String> = authored.rules.iter().flat_map(|r| r.body.iter().filter_map(Literal::atom)).map(|a| a.rel.clone()).filter(|r| base_types(r).is_some()).collect();
    let Program { decls, lattice, facts, rules, inputs, outputs, kindsets, demand, .. } = authored;
    for (name, columns) in decls {
        match converted.decls.get(&name) {
            Some(c) if types(c) != types(&columns) => {
                return err(format!("{name} is declared as ({}) here and as ({}) by the converter", spelled(types(&columns)), spelled(types(c))));
            }
            Some(_) => {}
            None => {
                converted.decls.insert(name, columns);
            }
        }
    }
    for (name, kinds) in kindsets {
        match converted.kindsets.get(&name) {
            Some(k) if *k != kinds => return err(format!("kind set {name} has other kinds than the converter's")),
            Some(_) => {}
            None => {
                converted.kindsets.insert(name, kinds);
            }
        }
    }
    for (name, l) in lattice {
        match converted.lattice.get(&name) {
            Some(c) if *c != l => return err(format!("{name}'s lattice differs from the converter's")),
            _ => {
                converted.lattice.insert(name, l);
            }
        }
    }
    for (rel, rows) in facts {
        converted.facts.entry(rel).or_default().extend(rows);
    }
    // core.rr's `.input` of a relation the converter gives (`option_item`) is the converter's facts, not an input.
    push_new(&mut converted.inputs, inputs.into_iter().filter(|i| !GIVEN.contains(&i.as_str())).chain(reads_base));
    push_new(&mut converted.outputs, outputs.into_iter().chain(["violation".to_owned()]));
    push_new(&mut converted.demand, demand);
    converted.rules.extend(rules.into_iter().filter(|r| violation_of(r).is_none_or(|(_, c)| keep(c))));
    Ok((reachable(converted), authored_clauses))
}

fn quote(s: &str) -> String {
    serde_json::to_string(s).expect("a string")
}

/// A body that binds `n`: one with no positive atom over `n` reads every node first.
fn nonempty(body: Body, n: &str) -> Body {
    let binds = body.iter().any(|l| single(l) && replace_var(l, n, "\u{0}") != *l);
    if binds {
        body
    } else {
        std::iter::once(format!("node(_, {n}, _, _, _)")).chain(body).collect()
    }
}

/// The kind ids a `kind` matcher names.
fn kind_ids(k: &ast_grep_core::matcher::KindMatcher) -> Vec<u16> {
    use ast_grep_core::matcher::Matcher;
    k.potential_kinds().map(|set| set.iter().filter_map(|k| u16::try_from(k).ok()).collect()).unwrap_or_default()
}

/// The kinds a node must have for `m` to match it, where they are known.
/// Whether `m` tests only a node's kind (a kind, a supertype-free any-of of kinds, or a utility that is one).
fn kind_only(m: &AstMatcher, depth: usize) -> bool {
    if depth > 32 {
        return false;
    }
    match m {
        AstMatcher::Kind(_) => true,
        AstMatcher::Any(ms) => ms.iter().all(|t| kind_only(t, depth + 1)),
        AstMatcher::Util(u) => u.with_target(|rule| kind_only(rule, depth + 1)).unwrap_or(false),
        _ => false,
    }
}

fn kinds(m: &AstMatcher, depth: usize) -> Option<BTreeSet<u16>> {
    use ast_grep_core::matcher::Matcher;
    if depth > 32 {
        return None;
    }
    match m {
        AstMatcher::Kind(k) => Some(kind_ids(k).into_iter().collect()),
        AstMatcher::Supertype(st) => Some(st.kinds.as_ref().clone()),
        AstMatcher::Pattern(p) => p.potential_kinds().map(|ks| ks.iter().filter_map(|k| u16::try_from(k).ok()).collect()),
        AstMatcher::Constrained(c) => kinds(&c.rule, depth + 1),
        AstMatcher::All(ms) => ms.iter().filter_map(|t| kinds(t, depth + 1)).reduce(|a, b| a.intersection(&b).copied().collect()),
        AstMatcher::Any(ms) => ms.iter().map(|t| kinds(t, depth + 1)).reduce(|a, b| Some(a?.union(&b?).copied().collect())).unwrap_or_else(|| Some(BTreeSet::new())),
        AstMatcher::Util(u) => u.with_target(|rule| kinds(rule, depth + 1)).flatten(),
        AstMatcher::Never => Some(BTreeSet::new()),
        _ => None,
    }
}

/// A token test a relation to a regex reaches tokens through (`any: [named, tokens]`, RER-131),
/// with a regex tested at each token, and nothing bound or counted.
fn token_alternative(test: &crate::code_rule_dsl::matcher::TokenTest) -> bool {
    test.capture.is_none() && test.counts.is_empty() && test.none.is_empty() && !test.tests.is_empty()
}

/// A semantic predicate's name, for the gaps.
fn semantic_name(s: &Semantic) -> &'static str {
    match s {
        Semantic::Resolves(_) => "resolves",
        Semantic::Binding(_) => "resolves.binding",
        Semantic::Binds(_) => "binds",
        Semantic::Value(_) => "value",
        Semantic::Arguments(_) => "arguments",
        Semantic::Parse(_) => "parse",
        Semantic::Host(_) => "host",
        Semantic::Embedded(_) => "embedded",
        Semantic::Capture(_) => "capture",
        Semantic::FormatArgs(_) => "formatArgs",
        Semantic::Layout(_) => "layout",
        Semantic::Tokens(_) => "tokens",
        Semantic::FilePath(_) => "file path",
        Semantic::Calls(_) => "calls",
        Semantic::ImportTarget(_) => "importTarget",
    }
}

/// Whether a literal is one positive atom (negatable as written).
fn single(literal: &str) -> bool {
    !literal.starts_with('!') && !literal.starts_with('@') && literal.contains('(') && !literal.contains(" = ") && !literal.contains(" < ")
}

fn rename(body: &[String], from: &str, to: &str) -> Body {
    body.iter().map(|l| replace_var(l, from, to)).collect()
}

/// The variables of `bodies` made before `mark` (`Emitter::fresh` numbers them in order): those of the rule around.
fn outer_vars(bodies: &[Body], mark: usize) -> Vec<String> {
    let mut out = BTreeSet::new();
    for literal in bodies.iter().flatten() {
        let mut rest = literal.as_str();
        while let Some(at) = rest.find("$v") {
            let digits: String = rest[at + 2..].chars().take_while(char::is_ascii_digit).collect();
            rest = &rest[at + 2 + digits.len()..];
            if let Ok(k) = digits.parse::<usize>()
                && k <= mark
            {
                out.insert(k);
            }
        }
    }
    out.into_iter().map(|k| format!("$v{k}")).collect()
}

/// Replaces the variable `from` (whole names only) in a literal.
fn replace_var(literal: &str, from: &str, to: &str) -> String {
    let mut out = String::new();
    let mut rest = literal;
    while let Some(at) = rest.find(from) {
        let after = &rest[at + from.len()..];
        let whole = !after.starts_with(|c: char| c.is_alphanumeric() || c == '_');
        out.push_str(&rest[..at]);
        out.push_str(if whole { to } else { from });
        rest = after;
    }
    out.push_str(rest);
    out
}

/// `a` strictly below `b` in the tree: the language's `within` (`core.rr`), which the engine reads off the tree
/// (decision 0050).
fn within_strict(a: &str, b: &str) -> String {
    format!("within({a}, {b})")
}

/// `a`'s span within `b`'s (or equal). Used where `a` is already below `b`'s parent and `b` is one of its children,
/// whose spans don't overlap: then it says "`a` is `b` or below it", which the front syntax has no alternative
/// for. The engine makes it `span_within` (decision 0050).
fn within(a: &str, b: &str, e: &mut Emitter) -> Body {
    let (d, s, t, u, w) = (e.var(), e.var(), e.var(), e.var(), e.var());
    vec![format!("node({d}, {a}, _, {s}, {t})"), format!("node({d}, {b}, _, {u}, {w})"), format!("{u} <= {s}"), format!("{t} <= {w}")]
}

enum Shape {
    Anything,
    Nothing,
    /// Whole-text alternatives of literals.
    Names(BTreeSet<String>),
    /// Alternatives each a prefix, suffix or substring test.
    Tests(Vec<(&'static str, String)>),
}

/// What a regex says about a name, where it is one of the shapes rules need.
fn shape(hir: &Hir) -> Option<Shape> {
    if is_anything(hir) {
        return Some(Shape::Anything);
    }
    if let HirKind::Concat(parts) = hir.kind()
        && parts.len() == 2
        && matches!(parts[0].kind(), HirKind::Look(Look::WordAscii | Look::WordUnicode))
        && matches!(parts[1].kind(), HirKind::Look(Look::WordAsciiNegate | Look::WordUnicodeNegate))
    {
        return Some(Shape::Nothing);
    }
    let branches = alternatives(hir);
    let mut names = BTreeSet::new();
    let mut tests = Vec::new();
    for b in &branches {
        let (start, strings, end, ci) = anchored(b)?;
        for lit in strings {
            let op = match (start, end, ci) {
                (true, true, false) => {
                    names.insert(lit);
                    continue;
                }
                (true, true, true) => "iequals",
                (true, false, false) => "prefix",
                (true, false, true) => "iprefix",
                (false, true, false) => "suffix",
                (false, true, true) => "isuffix",
                (false, false, false) => "contains",
                (false, false, true) => "icontains",
            };
            tests.push((op, lit));
        }
    }
    match (names.is_empty(), tests.is_empty()) {
        (false, true) => Some(Shape::Names(names)),
        (true, false) => Some(Shape::Tests(tests)),
        (false, false) => {
            tests.extend(names.into_iter().map(|n| ("equals", n)));
            Some(Shape::Tests(tests))
        }
        (true, true) => None,
    }
}

fn is_anything(hir: &Hir) -> bool {
    match hir.kind() {
        HirKind::Empty => true,
        HirKind::Repetition(r) => {
            r.min == 0
                && r.max.is_none()
                && matches!(r.sub.kind(), HirKind::Class(Class::Unicode(c))
                    if c.ranges().first().is_some_and(|r| r.start() == '\0') && c.ranges().last().is_some_and(|r| r.end() == char::MAX))
        }
        HirKind::Capture(c) => is_anything(&c.sub),
        _ => false,
    }
}

/// The top-level alternatives of a regex, through groups and a shared anchor (`^(?:a|b)$`).
fn alternatives(hir: &Hir) -> Vec<Hir> {
    match hir.kind() {
        HirKind::Alternation(bs) => bs.iter().flat_map(alternatives).collect(),
        HirKind::Capture(c) => alternatives(&c.sub),
        HirKind::Concat(parts) => {
            // `^ (a|b) $`, `^ (a|b)`, `(a|b) $`: distribute the anchors over the branches.
            let start = matches!(parts.first().map(Hir::kind), Some(HirKind::Look(Look::Start)));
            let end = matches!(parts.last().map(Hir::kind), Some(HirKind::Look(Look::End)));
            let inner = &parts[usize::from(start)..parts.len() - usize::from(end)];
            if inner.len() == 1 {
                let branches = alternatives(&inner[0]);
                if branches.len() > 1 {
                    return branches
                        .into_iter()
                        .map(|b| {
                            let mut v = Vec::new();
                            if start {
                                v.push(Hir::look(Look::Start));
                            }
                            v.push(b);
                            if end {
                                v.push(Hir::look(Look::End));
                            }
                            Hir::concat(v)
                        })
                        .collect();
                }
            }
            vec![hir.clone()]
        }
        _ => vec![hir.clone()],
    }
}

/// `(^?) language ($?)`: whether anchored at the start and the end, the strings, and whether they ignore case.
/// The language is finite: literals, alternations, optional parts, and one letter's cases.
fn anchored(hir: &Hir) -> Option<(bool, Vec<String>, bool, bool)> {
    let parts: Vec<&Hir> = match hir.kind() {
        HirKind::Concat(parts) => parts.iter().collect(),
        _ => vec![hir],
    };
    let start = matches!(parts.first().map(|h| h.kind()), Some(HirKind::Look(Look::Start)));
    let end = parts.len() > usize::from(start) && matches!(parts.last().map(|h| h.kind()), Some(HirKind::Look(Look::End)));
    let inner = &parts[usize::from(start)..parts.len() - usize::from(end)];
    let mut ci = false;
    let mut strings = vec![String::new()];
    for p in inner {
        strings = product(&strings, &language(p, &mut ci)?)?;
    }
    if strings.iter().any(String::is_empty) && !(start && end) {
        return None;
    }
    Some((start, strings, end, ci))
}

/// At most this many strings in a regex's language.
const MAX_STRINGS: usize = 256;

fn product(left: &[String], right: &[String]) -> Option<Vec<String>> {
    if left.len() * right.len() > MAX_STRINGS {
        return None;
    }
    Some(left.iter().flat_map(|l| right.iter().map(move |r| format!("{l}{r}"))).collect())
}

/// The finite language of `hir`, where it has one; a class of one letter's cases (`(?i)`) is that letter,
/// lowercased, and sets `ci`.
fn language(hir: &Hir, ci: &mut bool) -> Option<Vec<String>> {
    match hir.kind() {
        HirKind::Empty => Some(vec![String::new()]),
        HirKind::Literal(l) => Some(vec![std::str::from_utf8(&l.0).ok()?.to_owned()]),
        HirKind::Capture(c) => language(&c.sub, ci),
        HirKind::Concat(parts) => {
            let mut out = vec![String::new()];
            for p in parts {
                out = product(&out, &language(p, ci)?)?;
            }
            Some(out)
        }
        HirKind::Alternation(bs) => {
            let mut out = Vec::new();
            for b in bs {
                out.extend(language(b, ci)?);
            }
            (out.len() <= MAX_STRINGS).then_some(out)
        }
        HirKind::Repetition(r) if r.min == 0 && r.max == Some(1) => {
            let mut out = vec![String::new()];
            out.extend(language(&r.sub, ci)?);
            Some(out)
        }
        HirKind::Class(Class::Unicode(c)) => {
            let l = case_letter(c)?;
            *ci = true;
            Some(vec![l.to_string()])
        }
        _ => None,
    }
}

/// The kind of text a test reads (decision 0037), which decides when a regex is exactly a glob.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Text {
    /// A name: word characters (and the dots of a dotted name). `\w` is any of its characters.
    Name,
    /// A file path or module: a glob's `*` stays in a segment, `**` crosses them.
    Path,
    /// A string's content: any characters.
    Content,
}

/// `$s` is one of `set`: `$s = "a"`, or `$s in {"a", "b"}`.
fn member(s: &str, set: BTreeSet<String>) -> String {
    if set.len() == 1 { format!("{s} = {}", quote(set.first().expect("one"))) } else { format!("{s} in {}", set_term(set)) }
}

/// A set of strings as a term: one string, or `{a, b}`.
fn set_term(set: impl IntoIterator<Item = String>) -> String {
    let items: Vec<String> = set.into_iter().map(|s| quote(&s)).collect();
    if items.len() == 1 { items.into_iter().next().expect("one") } else { format!("{{{}}}", items.join(", ")) }
}

/// A literal's text.
fn lit(h: &Hir) -> Option<String> {
    match h.kind() {
        HirKind::Literal(l) => std::str::from_utf8(&l.0).ok().map(str::to_owned),
        HirKind::Capture(c) => lit(&c.sub),
        _ => None,
    }
}

/// One letter, either case (`f`, `[fF]`), lowercased.
fn letter(h: &Hir) -> Option<char> {
    match h.kind() {
        HirKind::Literal(_) => {
            let t = lit(h)?;
            let mut cs = t.chars();
            let c = cs.next()?;
            (cs.next().is_none() && c.is_ascii_alphabetic()).then(|| c.to_ascii_lowercase())
        }
        HirKind::Class(Class::Unicode(c)) => case_letter(c),
        _ => None,
    }
}

/// A class of one ASCII letter's cases (`(?i)s` is `[sSſ]`, `(?i)k` is `[kKK]`), lowercased.
fn case_letter(c: &regex_syntax::hir::ClassUnicode) -> Option<char> {
    let chars: Vec<char> = c.ranges().iter().flat_map(|r| r.start()..=r.end()).take(4).collect();
    let ascii: BTreeSet<char> = chars.iter().filter(|c| c.is_ascii()).copied().collect();
    let lower = ascii.first()?.to_ascii_lowercase();
    let ok = lower.is_ascii_alphabetic()
        && ascii == BTreeSet::from([lower, lower.to_ascii_uppercase()])
        && chars.iter().all(|c| c.is_ascii() || matches!(c, '\u{17f}' | '\u{212a}'));
    ok.then_some(lower)
}

/// `["']{1,3}`, `["']`: a run of quotes.
fn is_quote_class(h: &Hir) -> bool {
    let quotes = |h: &Hir| matches!(h.kind(), HirKind::Class(Class::Unicode(c)) if c.ranges().iter().all(|r| (r.start()..=r.end()).all(|ch| ch == '"' || ch == '\'')));
    match h.kind() {
        HirKind::Repetition(r) => r.min >= 1 && r.max.is_some_and(|m| m <= 3) && quotes(&r.sub),
        _ => quotes(h),
    }
}

/// At most this many globs for one regex.
const MAX_GLOBS: usize = 512;

/// A regex as globs over `text`, exactly, and whether they ignore case; `None` where no glob set says it.
/// The third value is what the globs say only approximately, if anything.
fn globs(hir: &Hir, text: Text) -> Option<(Vec<String>, bool, Option<&'static str>)> {
    let mut g = Globber { text, fold: false, cased: false, approx: None };
    let mut out = BTreeSet::new();
    let boundary = |h: Option<&Hir>| matches!(h.map(Hir::kind), Some(HirKind::Look(Look::WordAscii | Look::WordUnicode)));
    for (mut start, mut parts, mut end) in edges(hir) {
        // A word boundary at a name's edge: the name's edge or a dot (`\bx` is `x…` or `….x…`).
        let mut frags = vec![String::new()];
        let mut tails = vec![String::new()];
        if text == Text::Name && !start && boundary(parts.first()) {
            parts.remove(0);
            start = true;
            frags.push("*.".to_owned());
        }
        if text == Text::Name && !end && boundary(parts.last()) {
            parts.pop();
            end = true;
            tails.push(".*".to_owned());
        }
        for p in &parts {
            frags = glob_product(&frags, &g.frag(p)?)?;
        }
        frags = glob_product(&frags, &tails)?;
        for f in frags {
            for f in g.anchor(&f, start, end) {
                out.insert(f);
            }
        }
        if out.len() > MAX_GLOBS {
            return None;
        }
    }
    if g.fold && g.cased {
        return None;
    }
    // In a path, `**/?*/x` and `x` are `**/x`; `x` is dropped where `**/x` is kept.
    for g in out.clone() {
        if let Some(rest) = g.strip_prefix("**/?*/")
            && out.contains(rest)
        {
            out.remove(&g);
            out.remove(rest);
            out.insert(format!("**/{rest}"));
        }
    }
    let kept: Vec<String> = out.iter().filter(|g| !out.contains(&format!("**/{g}"))).cloned().collect();
    (!kept.is_empty()).then_some((kept, g.fold, g.approx))
}

fn glob_product(left: &[String], right: &[String]) -> Option<Vec<String>> {
    if left.len() * right.len() > MAX_GLOBS {
        return None;
    }
    Some(left.iter().flat_map(|l| right.iter().map(move |r| format!("{l}{r}"))).collect())
}

/// A regex's branches with their anchors: `(start, parts, end)`. An alternation at either edge is split into its
/// branches, recursively, so anchors inside it (`(?:^|/)x`, `x(?:/|$)`, `^(?:a/|b$)`) reach the edge.
fn edges(hir: &Hir) -> Vec<(bool, Vec<Hir>, bool)> {
    fn flat(h: &Hir) -> Vec<Hir> {
        match h.kind() {
            HirKind::Concat(ps) => ps.iter().flat_map(flat).collect(),
            HirKind::Capture(c) => flat(&c.sub),
            _ => vec![h.clone()],
        }
    }
    fn branches(h: &Hir) -> Option<Vec<Hir>> {
        match h.kind() {
            HirKind::Alternation(bs) => Some(bs.clone()),
            HirKind::Capture(c) => branches(&c.sub),
            _ => None,
        }
    }
    fn expand(parts: Vec<Hir>, out: &mut Vec<Vec<Hir>>) {
        if out.len() > MAX_GLOBS {
            return;
        }
        if let Some(bs) = parts.first().and_then(branches) {
            for b in bs {
                let mut v = flat(&b);
                v.extend(parts[1..].iter().cloned());
                expand(v, out);
            }
        } else if let Some(bs) = parts.last().and_then(branches) {
            for b in bs {
                let mut v = parts[..parts.len() - 1].to_vec();
                v.extend(flat(&b));
                expand(v, out);
            }
        } else {
            out.push(parts);
        }
    }
    let mut all = Vec::new();
    expand(flat(hir), &mut all);
    all.into_iter()
        .map(|mut parts| {
            let start = matches!(parts.first().map(Hir::kind), Some(HirKind::Look(Look::Start)));
            if start {
                parts.remove(0);
            }
            let end = matches!(parts.last().map(Hir::kind), Some(HirKind::Look(Look::End)));
            if end {
                parts.pop();
            }
            (start, parts, end)
        })
        .collect()
}

struct Globber {
    text: Text,
    /// A class of one letter's two cases was read as the letter: the globs ignore case.
    fold: bool,
    /// A literal holds a letter: with `fold`, case would be lost.
    cased: bool,
    approx: Option<&'static str>,
}

/// What a class is, for globs.
enum ClassIs {
    /// Any character the text holds, but `/` in a path.
    Segment,
    /// Any character at all, `/` included, in a path.
    Any,
    Other(String),
}

impl Globber {
    fn escape(&mut self, s: &str) -> String {
        let mut out = String::new();
        for c in s.chars() {
            if c.is_alphabetic() && c.to_lowercase().ne(c.to_uppercase()) {
                self.cased = true;
            }
            if matches!(c, '*' | '?' | '[' | ']' | '{' | '}' | '\\' | ',') {
                out.push('\\');
            }
            out.push(c);
        }
        out
    }

    fn class(&mut self, c: &regex_syntax::hir::ClassUnicode) -> Option<ClassIs> {
        let ranges = c.ranges();
        let contains = |ch: char| ranges.iter().any(|r| r.start() <= ch && ch <= r.end());
        let count: u32 = ranges.iter().map(|r| r.end() as u32 - r.start() as u32 + 1).sum();
        if count == 1 {
            return Some(ClassIs::Other(self.escape(&ranges[0].start().to_string())));
        }
        // One letter's cases.
        if let Some(l) = case_letter(c) {
            self.fold = true;
            return Some(ClassIs::Other(l.to_string()));
        }
        let word = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_".chars().all(contains);
        let everything = contains('\u{0}') && contains(char::MAX) && "\n/ .a".chars().filter(|&c| c != '\n').all(contains);
        match self.text {
            // Names hold word characters (and dots): `\w` and `.` are any of them.
            Text::Name if word => return Some(ClassIs::Segment),
            Text::Path if everything && contains('/') => return Some(ClassIs::Any),
            Text::Path if !contains('/') && contains(char::MAX) && contains('a') => return Some(ClassIs::Segment),
            Text::Content if everything => return Some(ClassIs::Segment),
            Text::Content if word => {
                self.approx = Some("\\w in a string's content read as any character");
                return Some(ClassIs::Segment);
            }
            _ => {}
        }
        // A small ASCII class, or the complement of one.
        let render = |rs: &[(char, char)]| -> Option<String> {
            let mut s = String::new();
            for &(a, b) in rs {
                if !a.is_ascii_alphanumeric() && a != '_' || !b.is_ascii_alphanumeric() && b != '_' {
                    if a == b && matches!(a, '.' | '/' | ' ' | '"' | '\'' | '%' | '@' | ':' | '(' | ')') && !(self.text == Text::Path && a == '/') {
                        s.push(a);
                        continue;
                    }
                    return None;
                }
                s.push(a);
                if b != a {
                    s.push('-');
                    s.push(b);
                }
            }
            Some(s)
        };
        let rs: Vec<(char, char)> = ranges.iter().map(|r| (r.start(), r.end())).collect();
        if rs.len() <= 8 && rs.iter().all(|(_, b)| b.is_ascii()) {
            return render(&rs).map(|s| ClassIs::Other(format!("[{s}]")));
        }
        if contains(char::MAX) && contains('\u{0}') {
            // The complement within ASCII.
            let mut comp = Vec::new();
            let mut at = 0u32;
            for &(a, b) in &rs {
                if a as u32 > at {
                    comp.push((char::from_u32(at)?, char::from_u32(a as u32 - 1)?));
                }
                at = b as u32 + 1;
                if at > 0x7f {
                    break;
                }
            }
            if comp.len() <= 4 && comp.iter().all(|(_, b)| b.is_ascii()) && !(self.text == Text::Path && !comp.iter().any(|&(a, b)| a <= '/' && '/' <= b)) {
                return render(&comp).map(|s| ClassIs::Other(format!("[!{s}]")));
            }
        }
        None
    }

    /// A part's glob fragments (alternatives).
    fn frag(&mut self, h: &Hir) -> Option<Vec<String>> {
        Some(match h.kind() {
            HirKind::Empty => vec![String::new()],
            HirKind::Literal(l) => vec![self.escape(std::str::from_utf8(&l.0).ok()?)],
            HirKind::Capture(c) => self.frag(&c.sub)?,
            HirKind::Concat(ps) => {
                let mut out = vec![String::new()];
                for p in ps {
                    out = glob_product(&out, &self.frag(p)?)?;
                }
                out
            }
            HirKind::Alternation(bs) => {
                let mut out = Vec::new();
                for b in bs {
                    out.extend(self.frag(b)?);
                }
                (out.len() <= MAX_GLOBS).then_some(out)?
            }
            HirKind::Class(Class::Unicode(c)) => match self.class(c)? {
                ClassIs::Segment => vec!["?".to_owned()],
                ClassIs::Any => return None,
                ClassIs::Other(s) => vec![s],
            },
            HirKind::Repetition(r) => {
                // Names hold no spaces: `\s*` there is nothing, `\s+` never matches.
                if self.text == Text::Name
                    && let HirKind::Class(Class::Unicode(c)) = r.sub.kind()
                    && c.ranges().iter().all(|r| (r.start()..=r.end()).all(char::is_whitespace))
                {
                    return Some(if r.min == 0 { vec![String::new()] } else { Vec::new() });
                }
                // A run of one character (`_*`): at most three of it.
                if r.max.is_none()
                    && let Some(one) = lit(&r.sub).filter(|l| l.chars().count() == 1)
                {
                    self.approx = Some("a run of one character read as at most three");
                    let one = self.escape(&one);
                    return Some((r.min as usize..=3.max(r.min as usize)).map(|k| one.repeat(k)).collect());
                }
                if let HirKind::Class(Class::Unicode(c)) = r.sub.kind() {
                    match self.class(c)? {
                        ClassIs::Segment if r.max.is_none() => return Some(vec![if r.min == 0 { "*" } else { "?*" }.to_owned()]),
                        ClassIs::Any if r.max.is_none() => {
                            return Some(if r.min == 0 { vec!["*".to_owned(), "*/**/*".to_owned()] } else { vec!["?*".to_owned(), "*/**/*".to_owned()] });
                        }
                        ClassIs::Any => return None,
                        _ => {}
                    }
                }
                let max = r.max?;
                if max > 4 {
                    return None;
                }
                let one = self.frag(&r.sub)?;
                let mut out = Vec::new();
                let mut at = vec![String::new()];
                for k in 0..=max {
                    if k >= r.min {
                        out.extend(at.iter().cloned());
                    }
                    at = glob_product(&at, &one)?;
                }
                out
            }
            _ => return None,
        })
    }

    /// A fragment with the regex's anchors: unanchored ends match anything there.
    fn anchor(&self, f: &str, start: bool, end: bool) -> Vec<String> {
        let mut heads = vec![f.to_owned()];
        if !start {
            heads = match self.text {
                Text::Path if f.starts_with('/') => vec![format!("**/?*{f}")],
                Text::Path if f.starts_with("**") => vec![f.to_owned()],
                Text::Path => vec![format!("**/*{f}")],
                _ if f.starts_with('*') => vec![f.to_owned()],
                _ => vec![format!("*{f}")],
            };
        }
        if end {
            return heads;
        }
        heads
            .into_iter()
            .flat_map(|h| match self.text {
                Text::Path if h.ends_with('/') => vec![format!("{h}**")],
                Text::Path if h.ends_with("**") => vec![h],
                Text::Path if h.ends_with('*') => vec![h.clone(), format!("{h}/**")],
                Text::Path => vec![format!("{h}*"), format!("{h}*/**")],
                _ if h.ends_with('*') => vec![h],
                _ => vec![format!("{h}*")],
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;

mod policy;
pub(in crate::code_rule_dsl) use policy::policy;
