//! The relations of core.rerail about the tree that a batch makes only for a program that reads them (`Tree::new`'s
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
//! ```
//!
//! A named-only tape (RER-131) stores no token its grammar implies, and these relations hold none: a rule that asks
//! for one is refused when it is compiled.
//!
//! ```text
//! visit_index(n, i)            n's place in the language's visit order, a key per document that compares in it
//!                              (`keys_by_run`): CPython's `ast` order in a Python document, the tape's elsewhere
//! ```
//!
//! Outside Python the order is the tape's: a node, then its children in order, so start offset, an ancestor before
//! the nodes it starts with (core.rerail's `earlier`). Before, only Python documents had rows, and `earlier` held of no
//! other language's nodes: a Vue component's script (decision 0597's context `script`) found no repeated key.
//!
//! The visit order is `ast.NodeVisitor`'s: a node, then its fields in their `_fields` order. Where tree-sitter's
//! children are in another order, they are taken in CPython's (`Tree::order`): a def's name, parameters, body, its
//! decorators (children of the `decorated_definition` around it), return annotation and type parameters; a class's
//! bases and keywords then body, decorators, type parameters; an argument list's positional arguments before its
//! keywords; a parameter list's defaults after all parameters (keyword-only defaults before `**kwargs`);
//! a dictionary's keys before its values; `a if t else b`'s test first. Nodes with no `ast` node of their own
//! (wrappers) are numbered where they open. Rows hold string indexes, as `Batch::build`'s do.

use crate::code_rule_dsl::facts::core::FactsIn;
use rerail_blocks::vector::Vector;
use super::{Dialect, Projected, Relation};
use crate::code_rule_dsl::facts::core::{Facts, NONE};
use std::collections::BTreeSet;

/// Tape node `x`'s operators, `op_at`'s in order: its children in field `operator` or `operators`, each its token
/// ("<", "not in"). What `op_at`'s rows hold and `@rr_node_op_at` reads (decision 0353 step 3).
pub(super) fn node_operators<'f>(f: FactsIn<'f>, dialect: &'f Dialect, x: u32) -> impl Iterator<Item = &'f str> + 'f {
    let fields = [dialect.operator, dialect.operators];
    f.children(x).into_iter().filter(move |&c| f.field(c) != 0 && fields.contains(&Some(f.field(c)))).map(move |c| dialect.name(f, c))
}

#[derive(Default)]
pub(super) struct Tree {
    op_at: bool,
    prev_token: bool,
    token: bool,
    /// `token` answered on request (decision 0174): its anonymous children's rows (`rr_token_anon`, their text their
    /// kind). Its named children's (`rr_token_named`) are a view of the tape (`tape_rows::NamedChild`).
    token_parts: bool,
    op_ats: Vector<[i64; 3]>,
    prev_tokens: Vector<[i64; 2]>,
    tokens: Vector<[i64; 5]>,
    anon_tokens: Vector<[i64; 5]>,
}

impl Tree {
    pub(super) fn new(reads: &BTreeSet<String>) -> Self {
        let read = |rel: &str| reads.contains(rel);
        Self {
            op_at: read("op_at"),
            prev_token: read("prev_token"),
            token: read("token"),
            token_parts: read("rr_token_anon"),
            ..Self::default()
        }
    }

    /// `op_at` of `p`'s nodes (`node_operators`). A guest's (a string annotation's comparison, `x: "123 is not y"`) are made too: this
    /// `op_at` stands in for `core_rr`'s, which holds every node's, the guests' included.
    pub(super) fn operators(&mut self, p: &Projected, dialect: &Dialect, string: &mut dyn FnMut(&str) -> usize) {
        if !self.op_at {
            return;
        }
        let f = p.f();
        for (_, id, n) in p.node_ids() {
            for (i, t) in node_operators(f, dialect, id).enumerate() {
                self.op_ats.push([n, i as i64, string(t) as i64]);
            }
        }
    }

    /// Document `p`'s rows. A text document (no `dialect`) has none.
    pub(super) fn document(&mut self, p: &Projected, dialect: Option<&Dialect>, string: &mut dyn FnMut(&str) -> usize) {
        let f = p.f();
        let Some(dialect) = dialect else { return };
        self.operators(p, dialect, string);
        if self.token_parts {
            for (_, id, n) in p.node_ids() {
                for (i, &c) in f.children(id).iter().enumerate() {
                    let (a, b) = (i64::from(f.start(c)), i64::from(f.end(c)));
                    // A named child a node is: `rr_token_named`'s, a view (`tape_rows::NamedChild`).
                    if !(f.named(c) && p.local_of_node(c) != NONE) {
                        // A named child no node is (a comment, a withheld node) keeps its text here.
                        let text = if f.named(c) { f.text(c) } else { dialect.name(f, c) };
                        self.anon_tokens.push([n, i as i64, string(text) as i64, a, b]);
                    }
                }
            }
        }
        if self.prev_token || self.token {
            for (_, id, n) in p.node_ids() {
                let children = f.children(id);
                // An anonymous token's text is its kind.
                let text = |c: u32| if f.named(c) { f.text(c) } else { dialect.name(f, c) };
                if self.token {
                    for (i, &c) in children.iter().enumerate() {
                        self.tokens.push([n, i as i64, string(text(c)) as i64, i64::from(f.start(c)), i64::from(f.end(c))]);
                    }
                }
                if self.prev_token {
                    for pair in children.windows(2) {
                        let after = p.local_of_node(pair[1]);
                        if after != NONE {
                            self.prev_tokens.push([p.global(after), string(text(pair[0])) as i64]);
                        }
                    }
                }
            }
        }
    }

    /// The relations read, their strings as atoms (`atom` of a string index).
    pub(super) fn relations(self, atom: &dyn Fn(i64) -> i64, relation: super::Make) -> Vec<(&'static str, Relation)> {
        let mut out = Vec::new();
        if self.op_at {
            out.push(("op_at", relation(&["n", "i", "s"], &mut self.op_ats.iter().map(|&[n, i, s]| rerail_facts_base::row![n, i, atom(s)])).with_atoms(&[false, false, true])));
        }
        if self.prev_token {
            out.push(("prev_token", relation(&["n", "s"], &mut self.prev_tokens.iter().map(|&[n, s]| rerail_facts_base::row![n, atom(s)])).with_atoms(&[false, true])));
        }
        if self.token {
            out.push(("token", relation(&["p", "i", "s", "start", "end_"], &mut self.tokens.iter().map(|&[p, i, s, a, b]| rerail_facts_base::row![p, i, atom(s), a, b])).with_atoms(&[false, false, true, false, false])));
        }
        if self.token_parts {
            out.push(("rr_token_anon", relation(&["p", "i", "s", "start", "end_"], &mut self.anon_tokens.iter().map(|&[p, i, s, a, b]| rerail_facts_base::row![p, i, atom(s), a, b])).with_atoms(&[false, false, true, false, false])));
        }
        out
    }
}

/// `visit_index`'s values for one document, `order` its nodes' local ids in visit order and `out` each one's place
/// from 0 (as `visit_order` gives them): the rank of its *run*, a stretch of nodes consecutive in the visit order that
/// are one granule's (its id's slot, `slot`), then its index among its granule's nodes in that order (decision 0121's
/// key, in the language's visit order rather than preorder). An edit inside a granule moves no other granule's
/// values; one that adds or removes a granule moves the ranks after it. Past the key's bits (more runs, or a
/// granule's more nodes), the places from 0 stay: one scheme a document, so its values still compare.
pub fn keys_by_run(order: &[u32], slot: impl Fn(u32) -> u32, out: &mut [u32]) {
    use rerail_clif::ids::ORDER_INDEX_BITS;
    let mut index: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    let (mut rank, mut last) = (0u32, None);
    for (k, &l) in order.iter().enumerate() {
        let s = slot(l);
        if last.is_some_and(|x| x != s) {
            rank += 1;
        }
        last = Some(s);
        let i = index.entry(s).or_default();
        if rank >> (32 - ORDER_INDEX_BITS) != 0 || *i >> ORDER_INDEX_BITS != 0 {
            // Past the key's bits: every place from 0 again.
            for (k, &l) in order[..k].iter().enumerate() {
                out[l as usize] = k as u32;
            }
            return;
        }
        out[l as usize] = rank << ORDER_INDEX_BITS | *i;
        *i += 1;
    }
}

/// The visit order over one document's tape (CPython's in a Python document), with a stack rather than recursion: a
/// chain of 10,000 `+` is that deep.
struct Order<'a> {
    f: FactsIn<'a>,
    dialect: &'a Dialect,
    p: &'a Projected,
    /// CPython's order; else the tape's.
    python: bool,
    next: u32,
    each: &'a mut dyn FnMut(u32, u32),
}

/// `visit_index`'s numbering of a document `p`, a file's own or a guest's (a Vue component's script, decision 0597):
/// each node of the projection with its place in the visit order (CPython's where `python`, else the tape's),
/// `(local, i)`, in that order (`tape_rows::VisitIndex` reads it).
pub fn visit_order(p: &Projected, dialect: &Dialect, python: bool, each: &mut dyn FnMut(u32, u32)) {
    if let Some((_, root)) = p.nodes().next() {
        Order { f: p.f(), dialect, p, python, next: 0, each }.run(root);
    }
}

/// What the order does next: number a node and take its children (with a definition's decorators), or number it only.
enum Step {
    Visit(u32, Vector<u32>),
    Number(u32),
}

fn visit(id: u32) -> Step {
    Step::Visit(id, rerail_blocks::vector::Vector::new())
}

impl Order<'_> {
    fn kind(&self, id: u32) -> &'static str {
        self.dialect.name(self.f, id)
    }

    fn field(&self, id: u32) -> &'static str {
        self.dialect.catalog.field_name(self.f.field(id)).unwrap_or("")
    }

    fn run(&mut self, root: u32) {
        let mut stack = vec![visit(root)];
        while let Some(step) = stack.pop() {
            let (id, decorators) = match step {
                Step::Visit(id, decorators) => (id, Some(decorators)),
                Step::Number(id) => (id, None),
            };
            let local = self.p.local_of_node(id);
            if local != NONE {
                (self.each)(local, self.next);
                self.next += 1;
            }
            if let Some(decorators) = decorators {
                stack.extend(self.children(id, decorators).into_iter().rev());
            }
        }
    }

    /// `id`'s children in CPython's order; `decorators`: a definition's, from the `decorated_definition` around it.
    fn children(&self, id: u32, decorators: Vector<u32>) -> Vec<Step> {
        let children = self.f.children(id);
        if !self.python {
            return children.iter().copied().map(visit).collect();
        }
        let with_field = |names: &[&str]| children.iter().copied().filter(|&c| names.contains(&self.field(c))).map(visit).collect::<Vec<_>>();
        match self.kind(id) {
            "decorated_definition" => {
                let (definition, decorators): (Vector<u32>, Vector<u32>) = children.iter().copied().filter(|&c| self.f.named(c)).partition(|&c| self.field(c) == "definition");
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
                let (keywords, positional): (Vector<u32>, Vector<u32>) = children.iter().copied().partition(keyword);
                positional.into_iter().chain(keywords).map(visit).collect()
            }
            "parameters" | "lambda_parameters" => self.parameters(&children),
            "dictionary" => {
                let part = |c: u32, value: bool| self.f.children(c).into_iter().filter(move |&k| (self.field(k) == "value") == value).map(visit);
                let mut out = Vec::new();
                for &c in &children {
                    match self.kind(c) {
                        "pair" => {
                            out.push(Step::Number(c));
                            out.extend(part(c, false));
                        }
                        "dictionary_splat" => {}
                        _ => out.push(visit(c)),
                    }
                }
                for &c in &children {
                    match self.kind(c) {
                        "pair" => out.extend(part(c, true)),
                        "dictionary_splat" => out.push(visit(c)),
                        _ => {}
                    }
                }
                out
            }
            "conditional_expression" => {
                let named: Vector<u32> = children.iter().copied().filter(|&c| self.f.named(c)).collect();
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
                    let (values, rest): (Vector<u32>, Vector<u32>) = self.f.children(c).iter().copied().partition(|&k| self.field(k) == "value");
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

