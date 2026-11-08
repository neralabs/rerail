//! CPython 3.13's `ast` tree of a file, built from Ruff's (`ast.parse`'s, as `syntax` reads it): each node its
//! CPython class and its `_fields` in CPython's order, with the values `ast.parse` gives them, so `dump` and
//! `unparse` print it as CPython's `ast.dump` and `ast.unparse` do. Ruff's shapes become CPython's: `elif` chains are
//! nested `If`s, `is_async` and `is_star` are their own classes, a parameter list is an `arguments` with its
//! `defaults` and `kw_defaults`, a string with an f-string part is a `JoinedStr` whose adjacent literal text (and a
//! debug field's `x=`) is one `Constant`, a debug field without a conversion or format spec converts with `!r`, and a
//! `u` string's first part gives `kind='u'`. A node's span is CPython's: a def or class starts at its keyword, after
//! its decorators; a generator that is a call's only argument takes the call's parentheses.
//!
//! A lone surrogate escape, which Ruff decodes as U+FFFD, is decoded from the text (`value::string_part`,
//! `value::fstring_literal`). A value Ruff cannot give exactly (such a text with a `\N{…}` escape too; a dict
//! comprehension unpacking `**`) is `F::Unknown`: a dump or an unparse that reaches it has no text.

use super::value::{Int, PyStr, fstring_literal, join, string_part};
use ruff_python_ast::{self as rast, Expr, Stmt};
use ruff_text_size::{Ranged, TextRange};

/// A CPython `ast` node: its class, its fields in `_fields` order, and its span (UTF-8 bytes; `None` for a class
/// with no location: `arguments`, `comprehension`, `withitem`, `match_case`, operators, contexts).
#[derive(Debug)]
pub(crate) struct Node {
    pub(crate) class: &'static str,
    pub(crate) fields: Vec<(&'static str, F)>,
    pub(crate) span: Option<(u32, u32)>,
}

/// A field's value.
#[derive(Debug)]
pub(crate) enum F {
    Node(Box<Node>),
    List(Vec<F>),
    /// `None` (an optional field without a value, or a list's `None`).
    Absent,
    /// A constant (`Constant.value`, `MatchSingleton.value`).
    Const(Const),
    /// An identifier or other str field (`id`, `attr`, `name`, `module`, `kind`).
    Ident(String),
    /// An int field (`conversion`, `level`, `simple`, `is_async`).
    Int(i64),
    /// A value Ruff's tree does not give exactly.
    Unknown,
}

/// A constant's value.
#[derive(Debug)]
pub(crate) enum Const {
    None,
    Bool(bool),
    Int(Int),
    Float(f64),
    Complex(f64, f64),
    Str(PyStr),
    Bytes(Vec<u8>),
    Ellipsis,
}

impl Node {
    fn new(class: &'static str, span: Option<TextRange>, fields: Vec<(&'static str, F)>) -> Self {
        Node { class, fields, span: span.map(|r| (r.start().to_u32(), r.end().to_u32())) }
    }

    /// Field `name` (`Absent` where the class has none).
    pub(crate) fn get(&self, name: &str) -> &F {
        self.fields.iter().find(|(n, _)| *n == name).map_or(&F::Absent, |(_, f)| f)
    }

    /// Field `name`'s node, if it holds one.
    pub(crate) fn node(&self, name: &str) -> Option<&Node> {
        match self.get(name) {
            F::Node(n) => Some(n),
            _ => None,
        }
    }

    /// Field `name`'s list (empty where it holds none).
    pub(crate) fn list(&self, name: &str) -> &[F] {
        match self.get(name) {
            F::List(items) => items,
            _ => &[],
        }
    }

    /// Field `name`'s str.
    pub(crate) fn ident(&self, name: &str) -> Option<&str> {
        match self.get(name) {
            F::Ident(s) => Some(s),
            _ => None,
        }
    }

    /// Every node of the tree, this one first, then each field's in order (preorder).
    pub(crate) fn walk<'n>(&'n self, out: &mut Vec<&'n Node>) {
        fn field<'n>(f: &'n F, out: &mut Vec<&'n Node>) {
            match f {
                F::Node(n) => n.walk(out),
                F::List(items) => items.iter().for_each(|i| field(i, out)),
                _ => {}
            }
        }
        out.push(self);
        self.fields.iter().for_each(|(_, f)| field(f, out));
    }
}

fn node(n: Node) -> F {
    F::Node(Box::new(n))
}

fn opt(f: Option<F>) -> F {
    f.unwrap_or(F::Absent)
}

/// A field-less node (an operator, a context).
fn bare(class: &'static str) -> F {
    node(Node::new(class, None, Vec::new()))
}

/// The CPython tree of `source`, `None` where `ast.parse` rejects it.
pub(crate) fn module(source: &str) -> Option<Node> {
    if super::syntax::syntax_error(source).is_some() {
        return None;
    }
    let parsed = ruff_python_parser::parse_unchecked(source, ruff_python_parser::ParseOptions::from(ruff_python_parser::Mode::Module).with_target_version(super::VERSION));
    let rast::Mod::Module(m) = parsed.syntax() else { return None };
    let b = Builder { source };
    Some(Node::new("Module", None, vec![("body", b.body(&m.body)), ("type_ignores", F::List(Vec::new()))]))
}

struct Builder<'s> {
    source: &'s str,
}

impl Builder<'_> {
    fn body(&self, stmts: &[Stmt]) -> F {
        F::List(stmts.iter().map(|s| node(self.stmt(s))).collect())
    }

    fn exprs<'e>(&self, exprs: impl IntoIterator<Item = &'e Expr>) -> F {
        F::List(exprs.into_iter().map(|e| node(self.expr(e))).collect())
    }

    fn opt_expr(&self, e: Option<&Expr>) -> F {
        opt(e.map(|e| node(self.expr(e))))
    }

    fn ident(&self, i: &rast::Identifier) -> F {
        F::Ident(i.as_str().to_owned())
    }

    fn opt_ident(&self, i: Option<&rast::Identifier>) -> F {
        opt(i.map(|i| self.ident(i)))
    }

    /// Where a def or class statement starts: its `async`, `def` or `class` keyword, after the decorators (whitespace
    /// and line continuations may stand between the keywords and the name).
    fn keyword_start(&self, range: TextRange, name: &rast::Identifier, keyword: &str, is_async: bool) -> TextRange {
        fn trim(mut s: &str) -> &str {
            loop {
                s = s.trim_end();
                match s.strip_suffix('\\') {
                    Some(rest) => s = rest,
                    None => return s,
                }
            }
        }
        let before = &self.source[..name.range.start().to_usize()];
        let at = trim(before).strip_suffix(keyword).map(str::len);
        let at = match (at, is_async) {
            (Some(at), true) => trim(&self.source[..at]).strip_suffix("async").map(str::len).unwrap_or(at),
            (Some(at), false) => at,
            (None, _) => range.start().to_usize(),
        };
        TextRange::new(ruff_text_size::TextSize::try_from(at).expect("a source offset"), range.end())
    }

    fn stmt(&self, s: &Stmt) -> Node {
        let r = Some(s.range());
        match s {
            Stmt::FunctionDef(f) => {
                let class = if f.is_async { "AsyncFunctionDef" } else { "FunctionDef" };
                let span = self.keyword_start(f.range, &f.name, "def", f.is_async);
                Node::new(
                    class,
                    Some(span),
                    vec![
                        ("name", self.ident(&f.name)),
                        ("args", node(self.arguments(Some(&f.parameters)))),
                        ("body", self.body(&f.body)),
                        ("decorator_list", self.exprs(f.decorator_list.iter().map(|d| &d.expression))),
                        ("returns", self.opt_expr(f.returns.as_deref())),
                        ("type_comment", F::Absent),
                        ("type_params", self.type_params(f.type_params.as_deref())),
                    ],
                )
            }
            Stmt::ClassDef(c) => {
                let span = self.keyword_start(c.range, &c.name, "class", false);
                let (bases, keywords) = match c.arguments.as_deref() {
                    Some(a) => (self.exprs(a.args.iter()), self.keywords(&a.keywords)),
                    None => (F::List(Vec::new()), F::List(Vec::new())),
                };
                Node::new(
                    "ClassDef",
                    Some(span),
                    vec![
                        ("name", self.ident(&c.name)),
                        ("bases", bases),
                        ("keywords", keywords),
                        ("body", self.body(&c.body)),
                        ("decorator_list", self.exprs(c.decorator_list.iter().map(|d| &d.expression))),
                        ("type_params", self.type_params(c.type_params.as_deref())),
                    ],
                )
            }
            Stmt::Return(x) => Node::new("Return", r, vec![("value", self.opt_expr(x.value.as_deref()))]),
            Stmt::Delete(x) => Node::new("Delete", r, vec![("targets", self.exprs(&x.targets))]),
            Stmt::TypeAlias(x) => Node::new(
                "TypeAlias",
                r,
                vec![("name", node(self.expr(&x.name))), ("type_params", self.type_params(x.type_params.as_deref())), ("value", node(self.expr(&x.value)))],
            ),
            Stmt::Assign(x) => Node::new("Assign", r, vec![("targets", self.exprs(&x.targets)), ("value", node(self.expr(&x.value))), ("type_comment", F::Absent)]),
            Stmt::AugAssign(x) => {
                Node::new("AugAssign", r, vec![("target", node(self.expr(&x.target))), ("op", bare(operator(x.op))), ("value", node(self.expr(&x.value)))])
            }
            Stmt::AnnAssign(x) => Node::new(
                "AnnAssign",
                r,
                vec![
                    ("target", node(self.expr(&x.target))),
                    ("annotation", node(self.expr(&x.annotation))),
                    ("value", self.opt_expr(x.value.as_deref())),
                    ("simple", F::Int(i64::from(x.simple))),
                ],
            ),
            Stmt::For(x) => Node::new(
                if x.is_async { "AsyncFor" } else { "For" },
                r,
                vec![
                    ("target", node(self.expr(&x.target))),
                    ("iter", node(self.expr(&x.iter))),
                    ("body", self.body(&x.body)),
                    ("orelse", self.body(&x.orelse)),
                    ("type_comment", F::Absent),
                ],
            ),
            Stmt::While(x) => Node::new("While", r, vec![("test", node(self.expr(&x.test))), ("body", self.body(&x.body)), ("orelse", self.body(&x.orelse))]),
            Stmt::If(x) => {
                // `elif` clauses nest from the last: each an If in its predecessor's orelse; `else` the last orelse.
                let mut orelse = F::List(Vec::new());
                for (k, clause) in x.elif_else_clauses.iter().enumerate().rev() {
                    orelse = match &clause.test {
                        Some(test) => {
                            let end = x.elif_else_clauses.last().map_or(clause.range.end(), |c| c.range.end());
                            let span = TextRange::new(clause.range.start(), end);
                            let _ = k;
                            F::List(vec![node(Node::new("If", Some(span), vec![("test", node(self.expr(test))), ("body", self.body(&clause.body)), ("orelse", orelse)]))])
                        }
                        None => self.body(&clause.body),
                    };
                }
                Node::new("If", r, vec![("test", node(self.expr(&x.test))), ("body", self.body(&x.body)), ("orelse", orelse)])
            }
            Stmt::With(x) => Node::new(
                if x.is_async { "AsyncWith" } else { "With" },
                r,
                vec![
                    (
                        "items",
                        F::List(
                            x.items
                                .iter()
                                .map(|i| node(Node::new("withitem", None, vec![("context_expr", node(self.expr(&i.context_expr))), ("optional_vars", self.opt_expr(i.optional_vars.as_deref()))])))
                                .collect(),
                        ),
                    ),
                    ("body", self.body(&x.body)),
                    ("type_comment", F::Absent),
                ],
            ),
            Stmt::Match(x) => Node::new(
                "Match",
                r,
                vec![
                    ("subject", node(self.expr(&x.subject))),
                    (
                        "cases",
                        F::List(
                            x.cases
                                .iter()
                                .map(|c| {
                                    node(Node::new(
                                        "match_case",
                                        None,
                                        vec![("pattern", node(self.pattern(&c.pattern))), ("guard", self.opt_expr(c.guard.as_deref())), ("body", self.body(&c.body))],
                                    ))
                                })
                                .collect(),
                        ),
                    ),
                ],
            ),
            Stmt::Raise(x) => Node::new("Raise", r, vec![("exc", self.opt_expr(x.exc.as_deref())), ("cause", self.opt_expr(x.cause.as_deref()))]),
            Stmt::Try(x) => Node::new(
                if x.is_star { "TryStar" } else { "Try" },
                r,
                vec![
                    ("body", self.body(&x.body)),
                    (
                        "handlers",
                        F::List(
                            x.handlers
                                .iter()
                                .map(|h| {
                                    let rast::ExceptHandler::ExceptHandler(h) = h;
                                    node(Node::new(
                                        "ExceptHandler",
                                        Some(h.range),
                                        vec![("type", self.opt_expr(h.type_.as_deref())), ("name", self.opt_ident(h.name.as_ref())), ("body", self.body(&h.body))],
                                    ))
                                })
                                .collect(),
                        ),
                    ),
                    ("orelse", self.body(&x.orelse)),
                    ("finalbody", self.body(&x.finalbody)),
                ],
            ),
            Stmt::Assert(x) => Node::new("Assert", r, vec![("test", node(self.expr(&x.test))), ("msg", self.opt_expr(x.msg.as_deref()))]),
            Stmt::Import(x) => Node::new("Import", r, vec![("names", self.aliases(&x.names))]),
            Stmt::ImportFrom(x) => Node::new(
                "ImportFrom",
                r,
                vec![("module", self.opt_ident(x.module.as_ref())), ("names", self.aliases(&x.names)), ("level", F::Int(i64::from(x.level)))],
            ),
            Stmt::Global(x) => Node::new("Global", r, vec![("names", F::List(x.names.iter().map(|n| self.ident(n)).collect()))]),
            Stmt::Nonlocal(x) => Node::new("Nonlocal", r, vec![("names", F::List(x.names.iter().map(|n| self.ident(n)).collect()))]),
            Stmt::Expr(x) => Node::new("Expr", r, vec![("value", node(self.expr(&x.value)))]),
            Stmt::Pass(_) => Node::new("Pass", r, Vec::new()),
            Stmt::Break(_) => Node::new("Break", r, Vec::new()),
            Stmt::Continue(_) => Node::new("Continue", r, Vec::new()),
            Stmt::IpyEscapeCommand(_) => Node::new("Pass", r, vec![("value", F::Unknown)]),
        }
    }

    fn aliases(&self, names: &[rast::Alias]) -> F {
        F::List(names.iter().map(|a| node(Node::new("alias", Some(a.range), vec![("name", self.ident(&a.name)), ("asname", self.opt_ident(a.asname.as_ref()))]))).collect())
    }

    fn keywords(&self, keywords: &[rast::Keyword]) -> F {
        F::List(keywords.iter().map(|k| node(Node::new("keyword", Some(k.range), vec![("arg", self.opt_ident(k.arg.as_ref())), ("value", node(self.expr(&k.value)))]))).collect())
    }

    fn type_params(&self, t: Option<&rast::TypeParams>) -> F {
        let Some(t) = t else { return F::List(Vec::new()) };
        F::List(
            t.type_params
                .iter()
                .map(|p| {
                    node(match p {
                        rast::TypeParam::TypeVar(v) => Node::new(
                            "TypeVar",
                            Some(v.range),
                            vec![("name", self.ident(&v.name)), ("bound", self.opt_expr(v.bound.as_deref())), ("default_value", self.opt_expr(v.default.as_deref()))],
                        ),
                        rast::TypeParam::ParamSpec(v) => {
                            Node::new("ParamSpec", Some(v.range), vec![("name", self.ident(&v.name)), ("default_value", self.opt_expr(v.default.as_deref()))])
                        }
                        rast::TypeParam::TypeVarTuple(v) => {
                            Node::new("TypeVarTuple", Some(v.range), vec![("name", self.ident(&v.name)), ("default_value", self.opt_expr(v.default.as_deref()))])
                        }
                    })
                })
                .collect(),
        )
    }

    /// An `arg`: it starts at its name (a `*args` or `**kwargs` after its stars, which Ruff's range holds).
    fn arg(&self, p: &rast::Parameter) -> F {
        let span = TextRange::new(p.name.range.start(), p.range.end());
        node(Node::new("arg", Some(span), vec![("arg", self.ident(&p.name)), ("annotation", self.opt_expr(p.annotation.as_deref())), ("type_comment", F::Absent)]))
    }

    /// An `arguments` node: positional defaults of the last positional parameters, one `kw_defaults` entry per
    /// keyword-only parameter (`None` without a default).
    fn arguments(&self, p: Option<&rast::Parameters>) -> Node {
        let empty = || F::List(Vec::new());
        let Some(p) = p else {
            return Node::new(
                "arguments",
                None,
                vec![("posonlyargs", empty()), ("args", empty()), ("vararg", F::Absent), ("kwonlyargs", empty()), ("kw_defaults", empty()), ("kwarg", F::Absent), ("defaults", empty())],
            );
        };
        let defaults = p.posonlyargs.iter().chain(&p.args).filter_map(|a| a.default.as_deref()).map(|d| node(self.expr(d))).collect();
        Node::new(
            "arguments",
            None,
            vec![
                ("posonlyargs", F::List(p.posonlyargs.iter().map(|a| self.arg(&a.parameter)).collect())),
                ("args", F::List(p.args.iter().map(|a| self.arg(&a.parameter)).collect())),
                ("vararg", opt(p.vararg.as_deref().map(|a| self.arg(a)))),
                ("kwonlyargs", F::List(p.kwonlyargs.iter().map(|a| self.arg(&a.parameter)).collect())),
                ("kw_defaults", F::List(p.kwonlyargs.iter().map(|a| self.opt_expr(a.default.as_deref())).collect())),
                ("kwarg", opt(p.kwarg.as_deref().map(|a| self.arg(a)))),
                ("defaults", F::List(defaults)),
            ],
        )
    }

    fn comprehensions(&self, generators: &[rast::Comprehension]) -> F {
        F::List(
            generators
                .iter()
                .map(|g| {
                    node(Node::new(
                        "comprehension",
                        None,
                        vec![("target", node(self.expr(&g.target))), ("iter", node(self.expr(&g.iter))), ("ifs", self.exprs(&g.ifs)), ("is_async", F::Int(i64::from(g.is_async)))],
                    ))
                })
                .collect(),
        )
    }

    fn pattern(&self, p: &rast::Pattern) -> Node {
        use rast::Pattern as P;
        let r = Some(p.range());
        match p {
            P::MatchValue(v) => Node::new("MatchValue", r, vec![("value", node(self.expr(&v.value)))]),
            P::MatchSingleton(v) => Node::new(
                "MatchSingleton",
                r,
                vec![(
                    "value",
                    F::Const(match v.value {
                        rast::Singleton::None => Const::None,
                        rast::Singleton::True => Const::Bool(true),
                        rast::Singleton::False => Const::Bool(false),
                    }),
                )],
            ),
            P::MatchSequence(v) => Node::new("MatchSequence", r, vec![("patterns", self.patterns(&v.patterns))]),
            P::MatchMapping(v) => Node::new(
                "MatchMapping",
                r,
                vec![("keys", self.exprs(v.keys.iter())), ("patterns", self.patterns(&v.patterns)), ("rest", self.opt_ident(v.rest.as_ref()))],
            ),
            P::MatchClass(v) => Node::new(
                "MatchClass",
                r,
                vec![
                    ("cls", node(self.expr(&v.cls))),
                    ("patterns", self.patterns(&v.arguments.patterns)),
                    ("kwd_attrs", F::List(v.arguments.keywords.iter().map(|k| self.ident(&k.attr)).collect())),
                    ("kwd_patterns", F::List(v.arguments.keywords.iter().map(|k| node(self.pattern(&k.pattern))).collect())),
                ],
            ),
            P::MatchStar(v) => Node::new("MatchStar", r, vec![("name", self.opt_ident(v.name.as_ref()))]),
            P::MatchAs(v) => Node::new("MatchAs", r, vec![("pattern", opt(v.pattern.as_deref().map(|p| node(self.pattern(p))))), ("name", self.opt_ident(v.name.as_ref()))]),
            P::MatchOr(v) => Node::new("MatchOr", r, vec![("patterns", self.patterns(&v.patterns))]),
        }
    }

    fn patterns(&self, ps: &[rast::Pattern]) -> F {
        F::List(ps.iter().map(|p| node(self.pattern(p))).collect())
    }

    fn ctx(&self, c: rast::ExprContext) -> F {
        bare(match c {
            rast::ExprContext::Store => "Store",
            rast::ExprContext::Del => "Del",
            rast::ExprContext::Load | rast::ExprContext::Invalid => "Load",
        })
    }

    fn expr(&self, e: &Expr) -> Node {
        let r = Some(e.range());
        match e {
            Expr::BoolOp(x) => Node::new(
                "BoolOp",
                r,
                vec![
                    (
                        "op",
                        bare(match x.op {
                            rast::BoolOp::And => "And",
                            rast::BoolOp::Or => "Or",
                        }),
                    ),
                    ("values", self.exprs(&x.values)),
                ],
            ),
            Expr::Named(x) => Node::new("NamedExpr", r, vec![("target", node(self.expr(&x.target))), ("value", node(self.expr(&x.value)))]),
            Expr::BinOp(x) => Node::new("BinOp", r, vec![("left", node(self.expr(&x.left))), ("op", bare(operator(x.op))), ("right", node(self.expr(&x.right)))]),
            Expr::UnaryOp(x) => Node::new(
                "UnaryOp",
                r,
                vec![
                    (
                        "op",
                        bare(match x.op {
                            rast::UnaryOp::Invert => "Invert",
                            rast::UnaryOp::Not => "Not",
                            rast::UnaryOp::UAdd => "UAdd",
                            rast::UnaryOp::USub => "USub",
                        }),
                    ),
                    ("operand", node(self.expr(&x.operand))),
                ],
            ),
            Expr::Lambda(x) => Node::new("Lambda", r, vec![("args", node(self.arguments(x.parameters.as_deref()))), ("body", node(self.expr(&x.body)))]),
            Expr::If(x) => Node::new("IfExp", r, vec![("test", node(self.expr(&x.test))), ("body", node(self.expr(&x.body))), ("orelse", node(self.expr(&x.orelse)))]),
            Expr::Dict(x) => Node::new(
                "Dict",
                r,
                vec![("keys", F::List(x.items.iter().map(|i| self.opt_expr(i.key.as_ref())).collect())), ("values", self.exprs(x.items.iter().map(|i| &i.value)))],
            ),
            Expr::Set(x) => Node::new("Set", r, vec![("elts", self.exprs(&x.elts))]),
            Expr::ListComp(x) => Node::new("ListComp", r, vec![("elt", node(self.expr(&x.elt))), ("generators", self.comprehensions(&x.generators))]),
            Expr::SetComp(x) => Node::new("SetComp", r, vec![("elt", node(self.expr(&x.elt))), ("generators", self.comprehensions(&x.generators))]),
            Expr::DictComp(x) => Node::new(
                "DictComp",
                r,
                vec![
                    ("key", x.key.as_deref().map_or(F::Unknown, |k| node(self.expr(k)))),
                    ("value", node(self.expr(&x.value))),
                    ("generators", self.comprehensions(&x.generators)),
                ],
            ),
            Expr::Generator(x) => Node::new("GeneratorExp", r, vec![("elt", node(self.expr(&x.elt))), ("generators", self.comprehensions(&x.generators))]),
            Expr::Await(x) => Node::new("Await", r, vec![("value", node(self.expr(&x.value)))]),
            Expr::Yield(x) => Node::new("Yield", r, vec![("value", self.opt_expr(x.value.as_deref()))]),
            Expr::YieldFrom(x) => Node::new("YieldFrom", r, vec![("value", node(self.expr(&x.value)))]),
            Expr::Compare(x) => Node::new(
                "Compare",
                r,
                vec![
                    ("left", node(self.expr(&x.left))),
                    ("ops", F::List(x.ops.iter().map(|o| bare(cmpop(*o))).collect())),
                    ("comparators", self.exprs(x.comparators.iter())),
                ],
            ),
            Expr::Call(x) => {
                let args = x.arguments.args.iter().map(|a| {
                    let mut n = self.expr(a);
                    // A generator that is the only argument takes the call's parentheses.
                    if let Expr::Generator(g) = a
                        && !g.parenthesized
                        && x.arguments.args.len() == 1
                        && x.arguments.keywords.is_empty()
                    {
                        let range = x.arguments.range;
                        n.span = Some((range.start().to_u32(), range.end().to_u32()));
                    }
                    node(n)
                });
                Node::new("Call", r, vec![("func", node(self.expr(&x.func))), ("args", F::List(args.collect())), ("keywords", self.keywords(&x.arguments.keywords))])
            }
            Expr::FString(x) => self.joined(x, r),
            Expr::TString(_) => Node::new("TemplateStr", r, vec![("values", F::Unknown)]),
            Expr::StringLiteral(x) => {
                let parts: Option<Vec<PyStr>> = x.value.as_slice().iter().map(|p| string_part(&p.value, &self.source[p.range()])).collect();
                let kind = x.value.as_slice().first().is_some_and(|p| p.flags.prefix().is_unicode());
                Node::new(
                    "Constant",
                    r,
                    vec![("value", parts.map_or(F::Unknown, |p| F::Const(Const::Str(join(p))))), ("kind", if kind { F::Ident("u".into()) } else { F::Absent })],
                )
            }
            Expr::BytesLiteral(x) => {
                let bytes: Vec<u8> = x.value.as_slice().iter().flat_map(|p| p.value.iter().copied()).collect();
                Node::new("Constant", r, vec![("value", F::Const(Const::Bytes(bytes))), ("kind", F::Absent)])
            }
            Expr::NumberLiteral(x) => {
                let value = match &x.value {
                    rast::Number::Int(_) => Int::parse(&self.source[x.range()]).map_or(F::Unknown, |i| F::Const(Const::Int(i))),
                    rast::Number::Float(f) => F::Const(Const::Float(*f)),
                    rast::Number::Complex { real, imag } => F::Const(Const::Complex(*real, *imag)),
                };
                Node::new("Constant", r, vec![("value", value), ("kind", F::Absent)])
            }
            Expr::BooleanLiteral(x) => Node::new("Constant", r, vec![("value", F::Const(Const::Bool(x.value))), ("kind", F::Absent)]),
            Expr::NoneLiteral(_) => Node::new("Constant", r, vec![("value", F::Const(Const::None)), ("kind", F::Absent)]),
            Expr::EllipsisLiteral(_) => Node::new("Constant", r, vec![("value", F::Const(Const::Ellipsis)), ("kind", F::Absent)]),
            Expr::Attribute(x) => Node::new("Attribute", r, vec![("value", node(self.expr(&x.value))), ("attr", self.ident(&x.attr)), ("ctx", self.ctx(x.ctx))]),
            Expr::Subscript(x) => Node::new("Subscript", r, vec![("value", node(self.expr(&x.value))), ("slice", node(self.expr(&x.slice))), ("ctx", self.ctx(x.ctx))]),
            Expr::Starred(x) => Node::new("Starred", r, vec![("value", node(self.expr(&x.value))), ("ctx", self.ctx(x.ctx))]),
            Expr::Name(x) => Node::new("Name", r, vec![("id", F::Ident(x.id.as_str().to_owned())), ("ctx", self.ctx(x.ctx))]),
            Expr::List(x) => Node::new("List", r, vec![("elts", self.exprs(&x.elts)), ("ctx", self.ctx(x.ctx))]),
            Expr::Tuple(x) => Node::new("Tuple", r, vec![("elts", self.exprs(&x.elts)), ("ctx", self.ctx(x.ctx))]),
            Expr::Slice(x) => Node::new(
                "Slice",
                r,
                vec![("lower", self.opt_expr(x.lower.as_deref())), ("upper", self.opt_expr(x.upper.as_deref())), ("step", self.opt_expr(x.step.as_deref()))],
            ),
            Expr::IpyEscapeCommand(_) => Node::new("Name", r, vec![("id", F::Unknown)]),
        }
    }

    /// A string with an f-string part: CPython's `JoinedStr`, its literal text (plain parts, f-strings' literal
    /// pieces and debug fields' text) joined into one `Constant` between fields, empty text left out.
    fn joined(&self, x: &rast::ExprFString, r: Option<TextRange>) -> Node {
        let mut values = Vec::new();
        let mut text = Vec::new();
        let mut exact = true;
        for part in x.value.as_slice() {
            match part {
                rast::FStringPart::Literal(lit) => match string_part(&lit.value, &self.source[lit.range()]) {
                    Some(s) => text.push(s),
                    None => exact = false,
                },
                rast::FStringPart::FString(f) => {
                    let raw = f.flags.prefix().is_raw();
                    let escapes = !raw && self.source[f.range()].contains(['\\']);
                    self.elements(&f.elements, escapes, &mut values, &mut text, &mut exact);
                }
            }
        }
        flush(&mut values, &mut text);
        Node::new("JoinedStr", r, vec![("values", if exact { F::List(values) } else { F::Unknown })])
    }

    /// An f-string's (or a format spec's) elements into `values`, literal text gathered in `text`.
    fn elements(&self, elements: &[rast::InterpolatedStringElement], escapes: bool, values: &mut Vec<F>, text: &mut Vec<PyStr>, exact: &mut bool) {
        for e in elements {
            match e {
                rast::InterpolatedStringElement::Literal(l) => {
                    // Ruff decodes a lone surrogate escape as U+FFFD: the piece is decoded from its text.
                    let piece = if escapes { fstring_literal(&l.value, &self.source[l.range()]) } else { Some(PyStr::Text(l.value.to_string())) };
                    match piece {
                        Some(s) => text.push(s),
                        None => *exact = false,
                    }
                }
                rast::InterpolatedStringElement::Interpolation(i) => {
                    if let Some(debug) = &i.debug_text {
                        text.push(PyStr::Text(debug.as_str().to_owned()));
                    }
                    flush(values, text);
                    let conversion = match i.conversion.to_byte() {
                        Some(b) => i64::from(b),
                        None if i.debug_text.is_some() && i.format_spec.is_none() => i64::from(b'r'),
                        None => -1,
                    };
                    let spec = i.format_spec.as_deref().map(|spec| {
                        let (mut inner, mut inner_text) = (Vec::new(), Vec::new());
                        self.elements(&spec.elements, escapes, &mut inner, &mut inner_text, exact);
                        flush(&mut inner, &mut inner_text);
                        node(Node::new("JoinedStr", Some(spec.range), vec![("values", F::List(inner))]))
                    });
                    values.push(node(Node::new(
                        "FormattedValue",
                        Some(i.range),
                        vec![("value", node(self.expr(&i.expression))), ("conversion", F::Int(conversion)), ("format_spec", opt(spec))],
                    )));
                }
            }
        }
    }
}

/// The text gathered so far as a `Constant`, if any.
fn flush(values: &mut Vec<F>, text: &mut Vec<PyStr>) {
    let s = join(std::mem::take(text));
    if s.points().next().is_some() {
        values.push(node(Node::new("Constant", None, vec![("value", F::Const(Const::Str(s))), ("kind", F::Absent)])));
    }
}

pub(crate) fn operator(op: rast::Operator) -> &'static str {
    use rast::Operator as O;
    match op {
        O::Add => "Add",
        O::Sub => "Sub",
        O::Mult => "Mult",
        O::MatMult => "MatMult",
        O::Div => "Div",
        O::Mod => "Mod",
        O::Pow => "Pow",
        O::LShift => "LShift",
        O::RShift => "RShift",
        O::BitOr => "BitOr",
        O::BitXor => "BitXor",
        O::BitAnd => "BitAnd",
        O::FloorDiv => "FloorDiv",
    }
}

fn cmpop(op: rast::CmpOp) -> &'static str {
    use rast::CmpOp as C;
    match op {
        C::Eq => "Eq",
        C::NotEq => "NotEq",
        C::Lt => "Lt",
        C::LtE => "LtE",
        C::Gt => "Gt",
        C::GtE => "GtE",
        C::Is => "Is",
        C::IsNot => "IsNot",
        C::In => "In",
        C::NotIn => "NotIn",
    }
}
