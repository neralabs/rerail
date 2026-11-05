//! Policy matchers as Datalog rules (list step 30, decision 0036): a one-time converter from a clause's
//! compiled matcher (`matcher::AstMatcher`, options resolved) to rules over the base relations
//! (`node`, `child`, `field`, `text`, `op`, `intval`) and the tree axes (`inside`, `has`).
//!
//! Each sub-rule is a conjunction of literals over one node variable. `any` is a relation with one rule per
//! branch, `not` its negation, a utility a relation of its own (recursive where it reaches itself). Regexes become string tests or
//! name sets, never regexes. What it cannot say is a gap, named; what it says only approximately is noted.
//! The clause is converted whole or not at all.
//!
//! The relations, per node `n`:
//! - `inside` (to the end): `inside($n, $a, set)`, the set being the kinds the ancestor's rule allows;
//! - `inside` with a stop rule: the same, less ancestors past the first stop node (`blocked`);
//! - `has` likewise, read from the ancestor; with a field, the descendant is within that field's child
//!   (byte spans, both nodes bound);
//! - neighbours: `child(p, c, i)` or `field(p, f, c)`.

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
pub(in crate::code_rule_dsl) struct Emitter {
    kinds: KindCatalog,
    /// Relation declarations, by name: `(name, columns)`.
    decls: BTreeMap<String, String>,
    rules: Vec<String>,
    facts: Vec<String>,
    kindsets: BTreeMap<String, BTreeSet<String>>,
    /// Name sets by their members, so equal sets are one relation.
    names: BTreeMap<BTreeSet<String>, String>,
    next: usize,
    /// What was said only approximately, by what.
    pub(in crate::code_rule_dsl) approx: BTreeSet<String>,
    /// The relation of each recursive utility, by its id in the tree being converted.
    utils: BTreeMap<(usize, usize), String>,
    /// The pattern captures of the rule being converted: each metavariable's variable.
    pub(in crate::code_rule_dsl) captures: BTreeMap<String, String>,
    /// Whether the empty relation `never` is used.
    never: bool,
    /// Whether the derived relation `name` (identifiers and dotted attributes) is used.
    uses_name: bool,
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
            names: BTreeMap::new(),
            next: 0,
            approx: BTreeSet::new(),
            utils: BTreeMap::new(),
            captures: BTreeMap::new(),
            never: false,
            uses_name: false,
        }
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

    /// A relation `p(n)` holding where `body` (over `$n`) holds; its name.
    fn relation(&mut self, bodies: Vec<Body>) -> String {
        let name = self.fresh("m");
        self.decls.insert(name.clone(), "n: node".to_owned());
        for body in bodies {
            self.rules.push(format!("{name}($n) :- {}.", nonempty(body, "$n").join(", ")));
        }
        name
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
                let r = self.relation(bodies);
                vec![format!("{r}({n})")]
            }
            AstMatcher::Pattern(p) => self.pattern(p, n)?,
            AstMatcher::Constrained(c) => {
                let mut body = self.body(&c.rule, n)?;
                for (name, rule) in &c.constraints {
                    let Some(v) = self.captures.get(name).cloned() else { return gap(format!("constraint on ${name}, not captured")) };
                    body.extend(self.body(rule, &v)?);
                }
                body
            }
            AstMatcher::Regex(r) => self.regex(r.as_str(), n, None)?,
            AstMatcher::All(ms) => {
                let kinds = kinds(m, 0);
                let mut body = Vec::new();
                for t in ms {
                    match t {
                        AstMatcher::Regex(r) => body.extend(self.regex(r.as_str(), n, kinds.as_ref())?),
                        _ => body.extend(self.body(t, n)?),
                    }
                }
                body
            }
            AstMatcher::Any(ms) => {
                let bodies = ms.iter().map(|t| self.scoped(|e| e.body(t, "$n"))).collect::<Res<Vec<_>>>()?;
                if bodies.iter().any(Vec::is_empty) {
                    return Ok(Vec::new());
                }
                if bodies.len() == 1 {
                    return Ok(rename(&bodies[0], "$n", n));
                }
                let r = self.relation(bodies);
                vec![format!("{r}({n})")]
            }
            AstMatcher::Not(t) => {
                let body = self.scoped(|e| e.body(t, "$n"))?;
                if body.is_empty() {
                    return Ok(self.never(n));
                }
                // One positive literal over `$n` alone is negated as written.
                let r = match body.as_slice() {
                    [one] if single(one) && !one.contains("$v") => one.clone(),
                    _ => format!("{}($n)", self.relation(vec![body])),
                };
                vec![format!("!{}", rename(&[r], "$n", n)[0])]
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
                        let body = u.with_target(|rule| self.scoped(|e| e.body(rule, "$n"))).ok_or_else(|| Gap("utility unavailable".into()))??;
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
            AstMatcher::Semantic(other) => return gap(format!("semantic {}", semantic_name(other))),
            AstMatcher::Call(_) => return gap("utility call"),
            AstMatcher::Compare(_) => return gap("compare"),
            AstMatcher::Count(_) => return gap("count"),
            AstMatcher::Never => self.never(n),
            #[cfg(test)]
            AstMatcher::TestUnknown(_) => return gap("three-valued"),
        })
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

    /// A text condition (`regex`, `notRegex`) over the text in `s`.
    pub(in crate::code_rule_dsl) fn condition(&mut self, c: &crate::code_rule_dsl::language::TextCondition, s: &str) -> Res<Body> {
        let mut body = Vec::new();
        if let Some(r) = &c.regex {
            body.extend(self.tests(r.as_str(), s, &DOC)?);
        }
        if let Some(r) = &c.not_regex {
            let t = self.tests(r.as_str(), "$s", &DOC)?;
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
                body.push(format!("inside({low}, {high}, {set})"));
                if let Some(f) = &field {
                    // The path enters `high` through `f`: `low` lies within that field's child.
                    let c = self.var();
                    body.push(format!("field({high}, {}, {c})", quote(f)));
                    body.extend(within(&low, &c, self));
                }
                if let StopBy::Rule(stop) = stop {
                    let Some(stops) = kinds(stop, 0) else { return gap("a stop rule of any kind") };
                    let stops = self.kindset(&stops)?;
                    let is_stop = self.scoped(|e| e.body(stop, "$s"))?;
                    let blocked = self.fresh("blocked");
                    self.decls.insert(blocked.clone(), "low: node, high: node".to_owned());
                    // A stop node strictly between: over `low`, under `high`.
                    let mut b = Vec::new();
                    if axis == Axis::Has {
                        b.push(format!("inside($low, $high, {set})"));
                    }
                    b.push(format!("inside($low, $s, {stops})"));
                    b.extend(is_stop);
                    if axis == Axis::Inside {
                        b.push(format!("inside($s, $high, {set})"));
                    } else {
                        b.extend(within_strict("$s", "$high", self));
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
        body.extend(self.body(rule, &m)?);
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
            self.uses_name = true;
            let s = self.var();
            let mut body = vec![format!("name({n}, {s})")];
            body.extend(self.tests(source, &s, &["name(_, $s)"])?);
            return Ok(body);
        }
        if !text_kinds {
            return gap(format!("a regex on the text of {} ({source})", names.map_or("any node".to_owned(), |k| k.into_iter().collect::<Vec<_>>().join("|"))));
        }
        let s = self.var();
        let mut body = vec![format!("text({n}, {s})")];
        body.extend(self.tests(source, &s, &["text(_, $s)"])?);
        Ok(body)
    }

    /// A regex over the text in `s` as string tests: a name set, or prefix, suffix and substring tests.
    /// `domain`: the literals that give the strings tested (each over `$s`), for a relation of alternatives.
    fn tests(&mut self, source: &str, s: &str, domain: &[&str]) -> Res<Body> {
        let hir = regex_syntax::ParserBuilder::new().build().parse(source).map_err(|e| Gap(format!("regex {source}: {e}")))?;
        let mut body = Vec::new();
        match shape(&hir) {
            Some(Shape::Anything) => {}
            Some(Shape::Nothing) => return Ok(self.never("_")),
            Some(Shape::Names(set)) => {
                let r = self.name_set(set);
                body.push(format!("{r}({s})"));
            }
            Some(Shape::Tests(tests)) if tests.len() == 1 => {
                let (op, lit) = &tests[0];
                body.push(format!("@{op}({s}, {})", quote(lit)));
            }
            Some(Shape::Tests(tests)) => {
                // One branch per test: a relation over the strings the rules read (names, paths, modules).
                let r = self.fresh("t");
                self.decls.insert(r.clone(), "s: sym".to_owned());
                for (op, lit) in tests {
                    if op == "equals" {
                        self.facts.push(format!("{r}({}).", quote(&lit)));
                        continue;
                    }
                    for column in domain {
                        self.rules.push(format!("{r}($s) :- {column}, @{op}($s, {}).", quote(&lit)));
                    }
                }
                let _ = &mut body;
                body.push(format!("{r}({s})"));
            }
            None => return gap(format!("regex {source}")),
        }
        Ok(body)
    }

    fn name_set(&mut self, set: BTreeSet<String>) -> String {
        if let Some(r) = self.names.get(&set) {
            return r.clone();
        }
        let r = format!("names{}", self.names.len());
        self.decls.insert(r.clone(), "s: sym".to_owned());
        for s in &set {
            self.facts.push(format!("{r}({}).", quote(s)));
        }
        self.names.insert(set, r.clone());
        r
    }

    /// Adds a rule `head :- body` and its head's declaration.
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
        let mut text: String = self.rules.join("\n");
        if self.uses_name {
            text.push_str(NAME);
        }
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
        if self.uses_name {
            out.push_str(NAME);
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

/// The base relations every converted program reads.
pub(in crate::code_rule_dsl) const BASE: &str = "\
.decl doc(d: int, path: sym, module: sym)
.decl node(d: int, n: node, kind: sym, start: int, end: int)
.decl child(p: node, c: node, i: int)
.decl field(p: node, f: sym, c: node)
.decl text(n: node, s: sym)
.decl intval(n: node, v: int)
.decl op(n: node, s: sym)
.decl line(n: node, l: int)
.decl syntax_error(d: int)
";

const BASE_RELATIONS: [&str; 9] = ["doc", "node", "child", "field", "text", "intval", "op", "line", "syntax_error"];

/// Python's keywords: tokens, never names.
const KEYWORDS: [&str; 35] = [
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except",
    "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try",
    "while", "with", "yield",
];

/// The strings a path or module condition tests.
const DOC: [&str; 2] = ["doc(_, $s, _)", "doc(_, _, $s)"];

/// A node's name: an identifier's text, or an attribute's object name and attribute joined by a dot.
const NAME: &str = "\
.decl name(n: node, s: sym)
name($n, $s) :- text($n, $s).
name($n, @cat($o, $a)) :- node(_, $n, \"attribute\", _, _), field($n, \"object\", $x), name($x, $o), field($n, \"attribute\", $y), text($y, $a).
";

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
        Semantic::FilePath(_) => "file path",
        Semantic::Calls(_) => "calls",
    }
}

/// Whether a literal is one positive atom (negatable as written).
fn single(literal: &str) -> bool {
    !literal.starts_with('!') && !literal.starts_with('@') && literal.contains('(') && !literal.contains(" = ") && !literal.contains(" < ")
}

fn rename(body: &[String], from: &str, to: &str) -> Body {
    body.iter().map(|l| replace_var(l, from, to)).collect()
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

/// `a`'s span within `b`'s (or equal): both bound.
fn within(a: &str, b: &str, e: &mut Emitter) -> Body {
    let (s, t, u, w) = (e.var(), e.var(), e.var(), e.var());
    vec![format!("node(_, {a}, _, {s}, {t})"), format!("node(_, {b}, _, {u}, {w})"), format!("{u} <= {s}"), format!("{t} <= {w}")]
}

/// `a` strictly within `b`: contained and not `b`.
fn within_strict(a: &str, b: &str, e: &mut Emitter) -> Body {
    let mut body = within(a, b, e);
    body.push(format!("{a} != {b}"));
    body
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
            let chars: Vec<char> = c.ranges().iter().flat_map(|r| r.start()..=r.end()).take(5).collect();
            let lower: BTreeSet<String> = chars.iter().map(|ch| ch.to_lowercase().collect()).collect();
            if chars.len() > 4 || lower.len() != 1 || chars.len() < 2 {
                return None;
            }
            *ci = true;
            Some(vec![lower.first()?.clone()])
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;
