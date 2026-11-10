//! The relations of core.rr about the tree that a batch makes only for a program that reads them (`Tree::new`'s
//! `reads`), beside `Extra`'s text (a document's `path_part` and `file_name` are `Batch::build`'s, made for every one):
//!
//! ```text
//! op_at(n, i, s)               n's operator tokens in order, from 0: its tape children in field `operator` or
//!                              `operators` (a comparison's), each its token: "<", "not in", "is not"
//! prev_token(n, s)             the token right before n among its parent's tape children, anonymous ones and comments
//!                              included (W's `follows` at a neighbour): its text, which a `regex` there reads: ","
//!                              or "import" (an anonymous token's text is its kind)
//! token(p, i, s, start, end)   p's tape children in order, anonymous ones and comments included (W's `precedes` and
//!                              `follows` at a neighbour): each one's text and byte range
//! visit_index(n, i)            n's place in CPython's `ast` visit order, from 0 per document (Python documents)
//! ```
//!
//! The visit order is `ast.NodeVisitor`'s: a node, then its fields in their `_fields` order. Where tree-sitter's
//! children are in another order, they are taken in CPython's (`Tree::order`): a def's name, parameters, body, its
//! decorators (children of the `decorated_definition` around it), return annotation and type parameters; a class's
//! bases and keywords then body, decorators, type parameters; an argument list's positional arguments before its
//! keywords; a parameter list's defaults after all parameters (keyword-only defaults before `**kwargs`);
//! a dictionary's keys before its values; `a if t else b`'s test first. Nodes with no `ast` node of their own
//! (wrappers) are numbered where they open. Rows hold string indexes, as `Batch::build`'s do.

use super::{Dialect, Projected, Relation};
use crate::code_rule_dsl::facts::core::{Facts, NONE, Part};
use std::collections::BTreeSet;

#[derive(Default)]
pub(super) struct Tree {
    op_at: bool,
    prev_token: bool,
    token: bool,
    visit_index: bool,
    op_ats: Vec<[i64; 3]>,
    prev_tokens: Vec<[i64; 2]>,
    tokens: Vec<[i64; 5]>,
    visit_indexes: Vec<[i64; 2]>,
}

impl Tree {
    pub(super) fn new(reads: &BTreeSet<String>) -> Self {
        let read = |rel: &str| reads.contains(rel);
        Self {
            op_at: read("op_at"),
            prev_token: read("prev_token"),
            token: read("token"),
            visit_index: read("visit_index"),
            ..Self::default()
        }
    }

    /// Document `p`'s rows (`python`: it is Python's). A text document (no `dialect`) has none.
    pub(super) fn document(&mut self, p: &Projected, dialect: Option<&Dialect>, python: bool, string: &mut dyn FnMut(&str) -> usize) {
        let f = &*p.facts;
        let Some(dialect) = dialect else { return };
        // A node's children in source order, the tokens its records leave out included (RER-131).
        let field = |part: &Part| match part {
            Part::Node(c) => f.field[*c as usize],
            Part::Token(t) => t.field,
        };
        let span = |part: &Part| match part {
            Part::Node(c) => (f.start[*c as usize], f.end[*c as usize]),
            Part::Token(t) => (t.start, t.end),
        };
        if self.op_at {
            let fields = [dialect.operator, dialect.operators];
            for (local, &id) in p.core.iter().enumerate() {
                let n = p.global(local as u32);
                let parts = f.parts(id);
                let operators = parts.iter().filter(|part| field(part) != 0 && fields.contains(&Some(field(part))));
                for (i, part) in operators.enumerate() {
                    self.op_ats.push([n, i as i64, string(dialect.part_name(f, part)) as i64]);
                }
            }
        }
        if self.prev_token || self.token {
            for (local, &id) in p.core.iter().enumerate() {
                let n = p.global(local as u32);
                let parts = f.parts(id);
                let text = |part: &Part| {
                    let (start, end) = span(part);
                    &f.source[start as usize..end as usize]
                };
                if self.token {
                    for (i, part) in parts.iter().enumerate() {
                        let (start, end) = span(part);
                        self.tokens.push([n, i as i64, string(text(part)) as i64, i64::from(start), i64::from(end)]);
                    }
                }
                if self.prev_token {
                    for pair in parts.windows(2) {
                        let Part::Node(after) = pair[1] else { continue };
                        let after = p.local[after as usize];
                        if after != NONE {
                            self.prev_tokens.push([p.global(after), string(text(&pair[0])) as i64]);
                        }
                    }
                }
            }
        }
        if self.visit_index && python && let Some(&root) = p.core.first() {
            Order { f, dialect, p, next: 0, out: &mut self.visit_indexes }.run(root);
        }
    }

    /// The relations read, their strings as atoms (`atom` of a string index).
    pub(super) fn relations(self, atom: &dyn Fn(i64) -> i64) -> Vec<(&'static str, Relation)> {
        let relation = |columns: &[&str], rows: &mut dyn Iterator<Item = Vec<i64>>| {
            let mut r = Relation::new(columns);
            rows.for_each(|row| r.push(&row));
            r
        };
        let mut out = Vec::new();
        if self.op_at {
            out.push(("op_at", relation(&["n", "i", "s"], &mut self.op_ats.iter().map(|&[n, i, s]| vec![n, i, atom(s)]))));
        }
        if self.prev_token {
            out.push(("prev_token", relation(&["n", "s"], &mut self.prev_tokens.iter().map(|&[n, s]| vec![n, atom(s)]))));
        }
        if self.token {
            out.push(("token", relation(&["p", "i", "s", "start", "end_"], &mut self.tokens.iter().map(|&[p, i, s, a, b]| vec![p, i, atom(s), a, b]))));
        }
        if self.visit_index {
            out.push(("visit_index", relation(&["n", "i"], &mut self.visit_indexes.iter().map(|r| r.to_vec()))));
        }
        out
    }
}

/// CPython's visit order over one Python document's tape (the module doc), with a stack rather than recursion: a
/// chain of 10,000 `+` is that deep.
struct Order<'a> {
    f: &'a Facts,
    dialect: &'a Dialect,
    p: &'a Projected,
    next: i64,
    out: &'a mut Vec<[i64; 2]>,
}

/// What the order does next: number a node and take its children (with a definition's decorators), or number it only.
enum Step {
    Visit(u32, Vec<u32>),
    Number(u32),
}

fn visit(id: u32) -> Step {
    Step::Visit(id, Vec::new())
}

impl Order<'_> {
    fn kind(&self, id: u32) -> &'static str {
        self.dialect.name(self.f, id)
    }

    fn field(&self, id: u32) -> &'static str {
        self.dialect.catalog.field_name(self.f.field[id as usize]).unwrap_or("")
    }

    fn run(&mut self, root: u32) {
        let mut stack = vec![visit(root)];
        while let Some(step) = stack.pop() {
            let (id, decorators) = match step {
                Step::Visit(id, decorators) => (id, Some(decorators)),
                Step::Number(id) => (id, None),
            };
            let local = self.p.local[id as usize];
            if local != NONE {
                self.out.push([self.p.global(local), self.next]);
                self.next += 1;
            }
            if let Some(decorators) = decorators {
                stack.extend(self.children(id, decorators).into_iter().rev());
            }
        }
    }

    /// `id`'s children in CPython's order; `decorators`: a definition's, from the `decorated_definition` around it.
    fn children(&self, id: u32, decorators: Vec<u32>) -> Vec<Step> {
        let children = self.f.children(id);
        let with_field = |names: &[&str]| children.iter().copied().filter(|&c| names.contains(&self.field(c))).map(visit).collect::<Vec<_>>();
        match self.kind(id) {
            "decorated_definition" => {
                let (definition, decorators): (Vec<u32>, Vec<u32>) = children.iter().copied().filter(|&c| self.f.named(c)).partition(|&c| self.field(c) == "definition");
                // Anything else (tokens) has no `ast` place: it goes first.
                let mut out: Vec<Step> = children.iter().copied().filter(|c| !definition.contains(c) && !decorators.contains(c)).map(visit).collect();
                out.extend(definition.into_iter().map(|d| Step::Visit(d, decorators.clone())));
                out
            }
            kind @ ("function_definition" | "class_definition") => {
                let head: &[&str] = if kind == "function_definition" { &["name", "parameters"] } else { &["name", "superclasses"] };
                let tail: &[&str] = if kind == "function_definition" { &["return_type", "type_parameters"] } else { &["type_parameters"] };
                let placed: Vec<&str> = head.iter().chain(tail).chain(&["body"]).copied().collect();
                let mut out: Vec<Step> = children.iter().copied().filter(|&c| !placed.contains(&self.field(c))).map(visit).collect();
                out.extend(with_field(head));
                out.extend(with_field(&["body"]));
                out.extend(decorators.into_iter().map(visit));
                for name in tail {
                    out.extend(with_field(&[name]));
                }
                out
            }
            "argument_list" => {
                let keyword = |c: &u32| matches!(self.kind(*c), "keyword_argument" | "dictionary_splat");
                let (keywords, positional): (Vec<u32>, Vec<u32>) = children.iter().copied().partition(keyword);
                positional.into_iter().chain(keywords).map(visit).collect()
            }
            "parameters" | "lambda_parameters" => self.parameters(children),
            "dictionary" => {
                let part = |c: u32, value: bool| self.f.children(c).iter().copied().filter(move |&k| (self.field(k) == "value") == value).map(visit);
                let mut out = Vec::new();
                for &c in children {
                    match self.kind(c) {
                        "pair" => {
                            out.push(Step::Number(c));
                            out.extend(part(c, false));
                        }
                        "dictionary_splat" => {}
                        _ => out.push(visit(c)),
                    }
                }
                for &c in children {
                    match self.kind(c) {
                        "pair" => out.extend(part(c, true)),
                        "dictionary_splat" => out.push(visit(c)),
                        _ => {}
                    }
                }
                out
            }
            "conditional_expression" => {
                let named: Vec<u32> = children.iter().copied().filter(|&c| self.f.named(c)).collect();
                match named.as_slice() {
                    [body, test, orelse] => vec![visit(*test), visit(*body), visit(*orelse)],
                    _ => named.into_iter().map(visit).collect(),
                }
            }
            _ => children.iter().copied().map(visit).collect(),
        }
    }

    /// A parameter list as `ast.arguments` visits it: posonlyargs, args, vararg, kwonlyargs, kw_defaults, kwarg,
    /// defaults. The parameters come in that order already; the defaults wait.
    fn parameters(&self, children: &[u32]) -> Vec<Step> {
        let (mut out, mut defaults, mut kw_defaults) = (Vec::new(), Vec::new(), Vec::new());
        let mut keyword_only = false;
        let splat = |c: u32, kind: &str| self.kind(c) == kind || self.kind(c) == "typed_parameter" && self.f.children(c).iter().any(|&s| self.kind(s) == kind);
        for &c in children {
            if splat(c, "dictionary_splat_pattern") {
                out.append(&mut kw_defaults);
            }
            match self.kind(c) {
                "default_parameter" | "typed_default_parameter" => {
                    out.push(Step::Number(c));
                    let (values, rest): (Vec<u32>, Vec<u32>) = self.f.children(c).iter().copied().partition(|&k| self.field(k) == "value");
                    out.extend(rest.into_iter().map(visit));
                    if keyword_only { &mut kw_defaults } else { &mut defaults }.extend(values.into_iter().map(visit));
                }
                _ => out.push(visit(c)),
            }
            if self.kind(c) == "keyword_separator" || splat(c, "list_splat_pattern") {
                keyword_only = true;
            }
        }
        out.append(&mut kw_defaults);
        out.append(&mut defaults);
        out
    }
}
