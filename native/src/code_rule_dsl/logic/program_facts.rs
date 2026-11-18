//! A rule program as facts (decision 0074): every rule, literal, atom and term, every declaration and setting, as a
//! row of a fixed schema, so a rule can read a program as data. A program written as facts reads back as the same
//! program (`program`).
//!
//! These rows are not how a program is kept (decision 0155): a kept program is its `.rr` text (`program_text`). The
//! rows are made from a program when asked for, and the text's `.about` part is the rows of what has no syntax
//! (`write_about`).
//!
//! The schema, one program `p` among several in a document:
//!
//! - **Rules:** `rule(p, r, head)` and `body(p, r, k, lit)`: rule `r`'s head atom and its `k`-th body literal.
//! - **Literals:** `lit_atom(p, l, a)`, `lit_cmp(p, l, left, op, right)`, `lit_agg(p, l, var, op, a)` (with
//!   `agg_over`, `agg_join`, `agg_by`), `lit_nearest(p, l, n, a, set)`, `lit_ast(p, l, doc, node, matcher)` (with
//!   `ast_capture`).
//! - **Atoms and terms:** `atom(p, a, rel, neg, gated)`, `arg(p, a, i, t)`, `term(p, t, kind, n, s)` (kind `var`,
//!   `wild`, `int`, `str`, `top` or `call`) and `call_arg(p, t, i, u)`.
//! - **Declarations:** `relation(p, rel, arity)`, `decl(p, rel, i, col, ty)`, `nullary(p, rel)`, `lattice(p, rel,
//!   keys, ty)` (with `lattice_merge`, `lattice_widen`), `input(p, k, rel)`, `output(p, k, rel)`, `kindset(p, set)`,
//!   `kindset_kind(p, set, k, kind)` and `qualifier(p, rel, word)` (`.decl r(…) inline` and its kin).
//! - **Facts:** `fact_row(p, rel, row, width)` and `fact(p, rel, row, i, kind, n, s)`.
//! - **What preparing made:** `demand`, `demand_dropped`, `magic_extras`, `magic_report` (with `report_list`),
//!   `gate` (with `gate_src`, `gate_quiet`, `gate_dep`), `native_dfs` (with `native_roots`), `native_match` (with
//!   `native_group`), `native_text_test` (with `native_text_pattern`), `native_components`, `native_reach`, `native_reach_through`, `rx_span`, `policy` and `setting`.
//!
//! Each list is indexed (`k`, `i`, `row`), so a row's place is in the row itself: the rows are a set, and any order
//! of them reads back as the same program. Ids (`r`, `l`, `a`, `t`) are numbered per program.

use super::front::name::Name;
use super::front::ast::{AggOp, Atom, LatticeDecl, Literal, Native, Program, Rule, Shape, ShapeWait, SpanIn, Term, Value};
use super::front::ast::{Gate, Report};
use std::collections::{BTreeMap, BTreeSet};

/// Rows by relation, as a program's `facts` holds them.
pub type Facts = BTreeMap<String, Vec<crate::code_rule_dsl::logic::front::name::List<Value>>>;

/// A value of a fact row, as a reader takes it.
pub trait Cell {
    fn int(&self) -> Result<i64, String>;
    fn str(&self) -> Result<&str, String>;
}

/// A branded atom's class as the program's facts write it (`term`, `fact` rows of kind `atom`, decision 0270).
fn brand_tag(b: crate::code_rule_dsl::logic::front::ast::Branded) -> &'static str {
    match b.tag() {
        'n' => "n",
        'k' => "k",
        _ => "f",
    }
}

fn branded(tag: &str, atom: i64) -> Result<crate::code_rule_dsl::logic::front::ast::Branded, String> {
    let atom = u32::try_from(atom).map_err(|e| bad(e))?;
    tag.chars().next().and_then(|c| crate::code_rule_dsl::logic::front::ast::Branded::of_tag(c, atom)).ok_or_else(|| bad(format!("no atom class {tag:?}")))
}

impl Cell for Value {
    fn int(&self) -> Result<i64, String> {
        match self {
            Value::Int(i) => Ok(*i),
            Value::Branded(b) => Ok(b.value()),
            Value::Str(x) => Err(bad(format!("{x:?} where a number goes"))),
        }
    }
    fn str(&self) -> Result<&str, String> {
        match self {
            Value::Str(x) => Ok(x),
            Value::Int(i) => Err(bad(format!("{i} where a string goes"))),
            Value::Branded(b) => Err(bad(format!("the atom {b} where a string goes"))),
        }
    }
}

/// Each relation of the schema and its columns (`int` or `sym`).
pub fn schema() -> BTreeMap<Name, crate::code_rule_dsl::logic::front::name::List<(Name, Name)>> {
    const SCHEMA: &[(&str, &[&str])] = &[
        ("program", &["p:sym"]),
        ("rule", &["p:sym", "r:int", "head:int"]),
        ("body", &["p:sym", "r:int", "k:int", "lit:int"]),
        ("lit_atom", &["p:sym", "l:int", "a:int"]),
        ("lit_cmp", &["p:sym", "l:int", "left:int", "op:sym", "right:int"]),
        ("lit_agg", &["p:sym", "l:int", "var:sym", "op:sym", "a:int"]),
        ("agg_over", &["p:sym", "l:int", "var:sym"]),
        ("agg_join", &["p:sym", "l:int", "sep:sym"]),
        ("agg_by", &["p:sym", "l:int", "i:int", "var:sym"]),
        ("lit_nearest", &["p:sym", "l:int", "n:sym", "a:sym", "set:sym"]),
        ("lit_ast", &["p:sym", "l:int", "doc:sym", "node:sym", "matcher:sym"]),
        ("ast_capture", &["p:sym", "l:int", "i:int", "name:sym", "var:sym"]),
        ("atom", &["p:sym", "a:int", "rel:sym", "neg:int", "gated:int"]),
        ("arg", &["p:sym", "a:int", "i:int", "t:int"]),
        ("term", &["p:sym", "t:int", "kind:sym", "n:int", "s:sym"]),
        ("call_arg", &["p:sym", "t:int", "i:int", "u:int"]),
        ("relation", &["p:sym", "rel:sym", "arity:int"]),
        ("decl", &["p:sym", "rel:sym", "i:int", "col:sym", "ty:sym"]),
        ("nullary", &["p:sym", "rel:sym"]),
        ("lattice", &["p:sym", "rel:sym", "keys:int", "ty:sym"]),
        ("lattice_merge", &["p:sym", "rel:sym", "f:sym"]),
        ("lattice_widen", &["p:sym", "rel:sym", "k:int"]),
        ("input", &["p:sym", "k:int", "rel:sym"]),
        ("output", &["p:sym", "k:int", "rel:sym"]),
        ("kindset", &["p:sym", "set:sym"]),
        ("kindset_kind", &["p:sym", "set:sym", "k:int", "kind:sym"]),
        // A relation of facts, with or without rows: one with none (a constant folded empty) reads back as present.
        ("fact_rel", &["p:sym", "rel:sym"]),
        ("fact_row", &["p:sym", "rel:sym", "row:int", "width:int"]),
        ("fact", &["p:sym", "rel:sym", "row:int", "i:int", "kind:sym", "n:int", "s:sym"]),
        ("demand", &["p:sym", "k:int", "rel:sym"]),
        ("demand_dropped", &["p:sym", "k:int", "a:sym", "b:sym"]),
        ("magic_extras", &["p:sym", "n:int"]),
        ("magic_report", &["p:sym", "magic_rules:int", "extras_kept:int", "gates:int"]),
        ("report_list", &["p:sym", "list:sym", "k:int", "s:sym"]),
        ("gate", &["p:sym", "k:int", "key:sym", "rel:sym", "level:int"]),
        ("gate_src", &["p:sym", "k:int", "src:sym"]),
        ("gate_quiet", &["p:sym", "k:int", "i:int", "rel:sym"]),
        ("gate_dep", &["p:sym", "k:int", "i:int", "rel:sym"]),
        ("native_dfs", &["p:sym", "rel:sym", "edge:sym", "target:sym"]),
        ("native_roots", &["p:sym", "rel:sym", "roots:sym"]),
        ("native_match", &["p:sym", "rel:sym", "input:sym", "pattern:sym", "positions:int"]),
        ("native_group", &["p:sym", "rel:sym", "i:int", "group:sym"]),
        ("native_text_test", &["p:sym", "rel:sym", "input:sym", "column:int", "op:sym"]),
        ("native_text_pattern", &["p:sym", "rel:sym", "i:int", "pattern:sym"]),
        ("native_python_candidates", &["p:sym", "rel:sym", "input:sym", "targets:sym", "context:sym", "member:sym"]),
        ("native_python", &["p:sym", "rel:sym", "kind:sym", "input:sym", "targets:sym"]),
        ("native_components", &["p:sym", "rel:sym", "edge:sym", "own:sym"]),
        ("native_reach", &["p:sym", "rel:sym", "seed:sym", "edge:sym"]),
        ("native_reach_through", &["p:sym", "rel:sym", "reach:sym", "map:sym"]),
        ("rx_span", &["p:sym", "span:sym", "s:sym"]),
        // `ast_patterns`: the AST patterns the program matches (`n ~ `…``), each by its text.
        ("ast_pattern", &["p:sym", "s:sym"]),
        ("policy", &["p:sym", "k:int", "json:sym"]),
        ("setting", &["p:sym", "k:int", "key:sym", "text:sym"]),
        // `doc_scopes`: present when the program reads only some documents; each entry k's globs, side 0 the
        // paths it reads and side 1 those it leaves out.
        ("doc_scoped", &["p:sym"]),
        ("doc_scope", &["p:sym", "k:int"]),
        ("doc_scope_glob", &["p:sym", "k:int", "side:int", "i:int", "glob:sym"]),
        // `text_globs`: `.scope text`'s, in order.
        ("text_glob", &["p:sym", "i:int", "glob:sym"]),
        // `answers`: each request relation's answering programs, in order.
        ("answer", &["p:sym", "req:sym", "i:int", "by:sym"]),
        // `asked_by`: the k-th rule of each request relation, one row for each relation whose readers it serves
        // (decision 0174).
        ("asked", &["p:sym", "req:sym", "k:int", "by:sym"]),
        // `qualifiers`: `.decl r(…) inline` and its kin (decision 0099).
        ("qualifier", &["p:sym", "rel:sym", "word:sym"]),
        // `tapes`: the columns declared `tape` (decision 0103 §3).
        ("tape_column", &["p:sym", "rel:sym", "i:int"]),
        // `shapes`: the join shape directives, in order: each one's head, literal and text, and when its literal
        // joins (after a relation, or once its variables are bound).
        ("shape", &["p:sym", "k:int", "head:sym", "lit:sym", "text:sym"]),
        ("shape_after", &["p:sym", "k:int", "rel:sym"]),
        ("shape_bound", &["p:sym", "k:int", "i:int", "var:sym"]),
    ];
    SCHEMA
        .iter()
        .map(|(rel, cols)| {
            let cols = cols.iter().map(|c| c.split_once(':').expect("name:type")).map(|(n, t)| (n.into(), t.into())).collect();
            ((*rel).into(), cols)
        })
        .collect()
}

fn s(x: &str) -> Value {
    Value::Str(x.to_owned().into())
}

fn n(x: impl TryInto<i64>) -> Value {
    Value::Int(x.try_into().unwrap_or(i64::MAX))
}

/// Writes programs as facts.
struct Writer<'a> {
    facts: &'a mut Facts,
    p: String,
    terms: i64,
    atoms: i64,
    literals: i64,
    /// Whether the parts `.rr` syntax writes are written too (rules, declarations, facts, inputs, outputs), or only
    /// the rest (`write_about`, decision 0155).
    syntax: bool,
}

impl Writer<'_> {
    fn row(&mut self, rel: &str, values: Vec<Value>) {
        // Made at its length: an `insert(0, …)` doubled each full row's capacity (0.2 GB of the Python pack's program).
        let mut row = Vec::with_capacity(values.len() + 1);
        row.push(s(&self.p));
        row.extend(values);
        match self.facts.get_mut(rel) {
            Some(rows) => rows.push(row.into()),
            None => {
                self.facts.insert(rel.to_owned(), vec![row.into()]);
            }
        }
    }

    fn term(&mut self, term: &Term) -> i64 {
        let t = self.terms;
        self.terms += 1;
        let (kind, num, text) = match term {
            Term::Var(v) => ("var", 0, v.as_str()),
            Term::Wild => ("wild", 0, ""),
            Term::Int(i) => ("int", *i, ""),
            Term::Branded(b) => ("atom", b.value(), brand_tag(*b)),
            Term::Str(x) => ("str", 0, x.as_str()),
            Term::Top => ("top", 0, ""),
            Term::Call(f, _) => ("call", 0, f.as_str()),
        };
        self.row("term", vec![n(t), s(kind), n(num), s(text)]);
        if let Term::Call(_, args) = term {
            for (i, arg) in args.iter().enumerate() {
                let u = self.term(arg);
                self.row("call_arg", vec![n(t), n(i), n(u)]);
            }
        }
        t
    }

    fn atom(&mut self, atom: &Atom) -> i64 {
        let a = self.atoms;
        self.atoms += 1;
        self.row("atom", vec![n(a), s(&atom.rel), n(atom.neg as i64), n(atom.gated as i64)]);
        for (i, arg) in atom.args.iter().enumerate() {
            let t = self.term(arg);
            self.row("arg", vec![n(a), n(i), n(t)]);
        }
        a
    }

    fn literal(&mut self, literal: &Literal) -> i64 {
        let l = self.literals;
        self.literals += 1;
        match literal {
            Literal::Atom(atom) => {
                let a = self.atom(atom);
                self.row("lit_atom", vec![n(l), n(a)]);
            }
            Literal::Cmp { l: left, op, r: right } => {
                let (left, right) = (self.term(left), self.term(right));
                self.row("lit_cmp", vec![n(l), n(left), s(op), n(right)]);
            }
            Literal::Agg { var, op, over, atom } => {
                let a = self.atom(atom);
                let name = match op {
                    AggOp::Count => "count",
                    AggOp::Min => "min",
                    AggOp::Max => "max",
                    AggOp::Sum => "sum",
                    AggOp::Product => "product",
                    AggOp::Join { .. } => "join",
                };
                self.row("lit_agg", vec![n(l), s(var), s(name), n(a)]);
                if let Some(over) = over {
                    self.row("agg_over", vec![n(l), s(over)]);
                }
                if let AggOp::Join { by, sep } = op {
                    self.row("agg_join", vec![n(l), s(sep)]);
                    for (i, v) in by.iter().enumerate() {
                        self.row("agg_by", vec![n(l), n(i), s(v)]);
                    }
                }
            }
            Literal::Nearest { n: node, a, kindset } => self.row("lit_nearest", vec![n(l), s(node), s(a), s(kindset)]),
            Literal::Ast { document, node, matcher, captures } => {
                self.row("lit_ast", vec![n(l), s(document), s(node), s(matcher)]);
                for (i, (name, v)) in captures.iter().enumerate() {
                    self.row("ast_capture", vec![n(l), n(i), s(name), s(v)]);
                }
            }
        }
        l
    }

    fn program(&mut self, prog: &Program) {
        self.row("program", vec![]);
        if self.syntax {
            self.syntax_parts(prog);
        }
        self.rest(prog);
    }

    /// What `.rr` syntax writes of `prog`: its rules, declarations, facts' rows, inputs and outputs.
    fn syntax_parts(&mut self, prog: &Program) {
        for (r, rule) in prog.rules.iter().enumerate() {
            let head = self.atom(&rule.head);
            self.row("rule", vec![n(r), n(head)]);
            for (k, literal) in rule.body.iter().enumerate() {
                let l = self.literal(literal);
                self.row("body", vec![n(r), n(k), n(l)]);
            }
        }
        for (rel, cols) in &prog.decls {
            self.row("relation", vec![s(rel), n(cols.len())]);
            for (i, (col, ty)) in cols.iter().enumerate() {
                self.row("decl", vec![s(rel), n(i), s(col), s(ty)]);
            }
        }
        for (rel, rows) in &prog.facts {
            for (row, values) in rows.iter().enumerate() {
                self.row("fact_row", vec![s(rel), n(row), n(values.len())]);
                for (i, v) in values.iter().enumerate() {
                    let (kind, num, text) = match v {
                        Value::Int(x) => ("int", *x, ""),
                        Value::Branded(b) => ("atom", b.value(), brand_tag(*b)),
                        Value::Str(x) => ("str", 0, x.as_str()),
                    };
                    self.row("fact", vec![s(rel), n(row), n(i), s(kind), n(num), s(text)]);
                }
            }
        }
        for (k, rel) in prog.inputs.iter().enumerate() {
            self.row("input", vec![n(k), s(rel)]);
        }
        for (k, rel) in prog.outputs.iter().enumerate() {
            self.row("output", vec![n(k), s(rel)]);
        }
    }

    /// The rest: what `.rr` syntax has no form for, and each relation of facts by name (one may have no rows).
    fn rest(&mut self, prog: &Program) {
        for f in Field::all_of(prog) {
            self.field(&f);
        }
        for rel in prog.facts.keys() {
            self.row("fact_rel", vec![s(rel)]);
        }
    }

    /// One field's rows (`Field::relations` names the relations they are rows of).
    fn field(&mut self, f: &Field<'_>) {
        match *f {
            Field::Nullary(set) => {
                for rel in set {
                    self.row("nullary", vec![s(rel)]);
                }
            }
            Field::Qualifiers(map) => {
                for (rel, words) in map {
                    for w in words {
                        self.row("qualifier", vec![s(rel), s(w)]);
                    }
                }
            }
            Field::Tapes(map) => {
                for (rel, cols) in map {
                    for i in cols {
                        self.row("tape_column", vec![s(rel), n(*i)]);
                    }
                }
            }
            Field::Shapes(shapes) => {
                for (k, shape) in shapes.iter().enumerate() {
                    self.row("shape", vec![n(k), s(&shape.head), s(&shape.lit), s(&shape.text)]);
                    match &shape.wait {
                        ShapeWait::After(a) => self.row("shape_after", vec![n(k), s(a)]),
                        ShapeWait::Bound(vars) => {
                            for (i, x) in vars.iter().enumerate() {
                                self.row("shape_bound", vec![n(k), n(i), s(x)]);
                            }
                        }
                    }
                }
            }
            Field::Lattice(map) => {
                for (rel, l) in map {
                    self.row("lattice", vec![s(rel), n(l.keys), s(&l.ty)]);
                    if let Some(f) = &l.merge {
                        self.row("lattice_merge", vec![s(rel), s(f)]);
                    }
                    if let Some(k) = l.widen {
                        self.row("lattice_widen", vec![s(rel), n(k)]);
                    }
                }
            }
            Field::Kindsets(map) => {
                for (set, kinds) in map {
                    self.row("kindset", vec![s(set)]);
                    for (k, kind) in kinds.iter().enumerate() {
                        self.row("kindset_kind", vec![s(set), n(k), s(kind)]);
                    }
                }
            }
            Field::Demand(list) => {
                for (k, rel) in list.iter().enumerate() {
                    self.row("demand", vec![n(k), s(rel)]);
                }
            }
            Field::DemandDropped(list) => {
                for (k, (a, b)) in list.iter().enumerate() {
                    self.row("demand_dropped", vec![n(k), s(a), s(b)]);
                }
            }
            Field::MagicExtras(extras) => {
                if let Some(extras) = extras {
                    self.row("magic_extras", vec![n(extras)]);
                }
            }
            Field::MagicReport(report) => {
                if let Some(report) = report {
                    self.row("magic_report", vec![n(report.magic_rules), n(report.extras_kept), n(report.gates)]);
                    for (list, items) in [("adornments", &report.adornments), ("full", &report.full), ("full_for_negation", &report.full_for_negation)] {
                        for (k, item) in items.iter().enumerate() {
                            self.row("report_list", vec![s(list), n(k), s(item)]);
                        }
                    }
                }
            }
            Field::Gates(gates) => {
                for (k, (key, gate)) in gates.iter().enumerate() {
                    self.row("gate", vec![n(k), s(key), s(&gate.rel), n(gate.level)]);
                    if let Some(src) = &gate.src {
                        self.row("gate_src", vec![n(k), s(src)]);
                    }
                    for (i, rel) in gate.quiet.iter().enumerate() {
                        self.row("gate_quiet", vec![n(k), n(i), s(rel)]);
                    }
                    for (i, rel) in gate.deps.iter().enumerate() {
                        self.row("gate_dep", vec![n(k), n(i), s(rel)]);
                    }
                }
            }
            Field::Policies(policies) => {
                for (k, policy) in policies.iter().enumerate() {
                    self.row("policy", vec![n(k), s(&policy.to_string())]);
                }
            }
            Field::Settings(settings) => {
                for (k, (key, text)) in settings.iter().enumerate() {
                    self.row("setting", vec![n(k), s(key), s(text)]);
                }
            }
            Field::Natives(natives) => {
                for (rel, native) in natives {
                    self.native(rel, native);
                }
            }
            Field::DocScopes(scopes) => {
                if let Some(scopes) = scopes {
                    self.row("doc_scoped", vec![]);
                    for (k, (within, out)) in scopes.iter().enumerate() {
                        self.row("doc_scope", vec![n(k)]);
                        for (side, globs) in [within, out].into_iter().enumerate() {
                            for (i, g) in globs.iter().enumerate() {
                                self.row("doc_scope_glob", vec![n(k), n(side), n(i), s(g)]);
                            }
                        }
                    }
                }
            }
            Field::TextGlobs(globs) => {
                for (i, g) in globs.iter().enumerate() {
                    self.row("text_glob", vec![n(i), s(g)]);
                }
            }
            Field::AstPatterns(patterns) => {
                for pattern in patterns {
                    self.row("ast_pattern", vec![s(pattern)]);
                }
            }
            Field::Answers(answers) => {
                for (req, by) in answers {
                    for (i, b) in by.iter().enumerate() {
                        self.row("answer", vec![s(req), n(i), s(b)]);
                    }
                }
            }
            Field::Asked(asked) => {
                for (req, rules) in asked {
                    for (k, readers) in rules.iter().enumerate() {
                        for b in readers {
                            self.row("asked", vec![s(req), n(k), s(b)]);
                        }
                    }
                }
            }
            Field::RxSpans(spans) => {
                for (span, text) in spans {
                    // Kinds and captures carry their names after a unit separator.
                    let span = match span {
                        SpanIn::Source => "source".to_owned(),
                        SpanIn::Line => "line".to_owned(),
                        SpanIn::Comment => "comment".to_owned(),
                        SpanIn::Kind(k) => format!("kind:{k}"),
                        SpanIn::ChildOf(k) => format!("child_of:{k}"),
                        SpanIn::Capture(p, g) => format!("capture:{g}:{p}"),
                    };
                    self.row("rx_span", vec![s(&span), s(text)]);
                }
            }
        }
    }

    /// A native relation's rows.
    fn native(&mut self, rel: &str, native: &Native) {
        match native {
            Native::DfsFirst { edge, target, roots } => {
                self.row("native_dfs", vec![s(rel), s(edge), s(target)]);
                if let Some(roots) = roots {
                    self.row("native_roots", vec![s(rel), s(roots)]);
                }
            }
            Native::MatchAll { input, pattern, groups, positions } => {
                self.row("native_match", vec![s(rel), s(input), s(pattern), n(*positions as i64)]);
                for (i, group) in groups.iter().enumerate() {
                    self.row("native_group", vec![s(rel), n(i), s(group)]);
                }
            }
            Native::TextTest { input, column, op, patterns } => {
                self.row("native_text_test", vec![s(rel), s(input), n(*column), s(op)]);
                for (i, pattern) in patterns.iter().enumerate() {
                    self.row("native_text_pattern", vec![s(rel), n(i), s(pattern)]);
                }
            }
            Native::PythonCandidates { input, targets, context, member } => self.row("native_python_candidates", vec![s(rel), s(input), s(targets), s(context), s(member)]),
            Native::Python { kind, input, targets } => self.row("native_python", vec![s(rel), s(kind), s(input), s(targets.as_deref().unwrap_or(""))]),
            Native::Components { edge, own } => self.row("native_components", vec![s(rel), s(edge), s(own)]),
            Native::Reach { seed, edge } => self.row("native_reach", vec![s(rel), s(seed), s(edge)]),
            Native::ReachThrough { reach, map } => self.row("native_reach_through", vec![s(rel), s(reach), s(map)]),
        }
    }
}

/// One of a program's fields besides its rules, declarations, facts, inputs and outputs: what `write_about` writes,
/// a field at a time, so a version's field is stated where it is (`Builder::set_field`, decision 0276 §3) and read
/// where it is (`Version`'s readers in `program_image::fields`), with no program decoded.
#[derive(Clone, Copy)]
pub enum Field<'a> {
    Nullary(&'a BTreeSet<String>),
    Qualifiers(&'a BTreeMap<String, BTreeSet<String>>),
    Tapes(&'a BTreeMap<String, BTreeSet<usize>>),
    Shapes(&'a [Shape]),
    Lattice(&'a BTreeMap<String, LatticeDecl>),
    Kindsets(&'a BTreeMap<String, Vec<String>>),
    Demand(&'a [String]),
    DemandDropped(&'a [(String, String)]),
    MagicExtras(Option<usize>),
    MagicReport(Option<&'a Report>),
    Gates(&'a [(String, Gate)]),
    Policies(&'a [serde_json::Value]),
    Settings(&'a [(String, String)]),
    Natives(&'a BTreeMap<String, Native>),
    DocScopes(Option<&'a [(Vec<String>, Vec<String>)]>),
    TextGlobs(&'a [String]),
    AstPatterns(&'a BTreeSet<String>),
    Answers(&'a BTreeMap<String, Vec<String>>),
    Asked(&'a BTreeMap<String, Vec<BTreeSet<String>>>),
    RxSpans(&'a BTreeSet<(SpanIn, String)>),
}

/// The native relations' about relations: each names its relation in its first column after the program's.
pub const NATIVE_RELATIONS: &[&str] = &["native_dfs", "native_roots", "native_match", "native_group", "native_text_test", "native_text_pattern", "native_python_candidates", "native_python", "native_components", "native_reach", "native_reach_through"];

impl<'a> Field<'a> {
    /// The relations this field's rows are rows of: stating the field replaces all of theirs.
    pub fn relations(&self) -> &'static [&'static str] {
        match self {
            Field::Nullary(_) => &["nullary"],
            Field::Qualifiers(_) => &["qualifier"],
            Field::Tapes(_) => &["tape_column"],
            Field::Shapes(_) => &["shape", "shape_after", "shape_bound"],
            Field::Lattice(_) => &["lattice", "lattice_merge", "lattice_widen"],
            Field::Kindsets(_) => &["kindset", "kindset_kind"],
            Field::Demand(_) => &["demand"],
            Field::DemandDropped(_) => &["demand_dropped"],
            Field::MagicExtras(_) => &["magic_extras"],
            Field::MagicReport(_) => &["magic_report", "report_list"],
            Field::Gates(_) => &["gate", "gate_src", "gate_quiet", "gate_dep"],
            Field::Policies(_) => &["policy"],
            Field::Settings(_) => &["setting"],
            Field::Natives(_) => NATIVE_RELATIONS,
            Field::DocScopes(_) => &["doc_scoped", "doc_scope", "doc_scope_glob"],
            Field::TextGlobs(_) => &["text_glob"],
            Field::AstPatterns(_) => &["ast_pattern"],
            Field::Answers(_) => &["answer"],
            Field::Asked(_) => &["asked"],
            Field::RxSpans(_) => &["rx_span"],
        }
    }

    /// Every field of `prog`, in the order `write_about` writes them.
    pub fn all_of(prog: &'a Program) -> [Field<'a>; 20] {
        [
            Field::Nullary(&prog.nullary),
            Field::Qualifiers(&prog.qualifiers),
            Field::Tapes(&prog.tapes),
            Field::Shapes(&prog.shapes),
            Field::Lattice(&prog.lattice),
            Field::Kindsets(&prog.kindsets),
            Field::Demand(&prog.demand),
            Field::DemandDropped(&prog.demand_dropped),
            Field::MagicExtras(prog.magic_extras),
            Field::MagicReport(prog.magic_report.as_ref()),
            Field::Gates(&prog.gates),
            Field::Policies(&prog.policies),
            Field::Settings(&prog.settings),
            Field::Natives(&prog.natives),
            Field::DocScopes(prog.doc_scopes.as_deref()),
            Field::TextGlobs(&prog.text_globs),
            Field::AstPatterns(&prog.ast_patterns),
            Field::Answers(&prog.answers),
            Field::Asked(&prog.asked_by),
            Field::RxSpans(&prog.rx_spans),
        ]
    }
}

/// Field `f`'s rows, as facts of program `p` (each of `f.relations()` that has rows).
pub fn write_field(facts: &mut Facts, p: &str, f: &Field<'_>) {
    Writer { facts, p: p.to_owned(), terms: 0, atoms: 0, literals: 0, syntax: false }.field(f);
}

/// `prog` as facts of program `p`, added to `facts`.
pub fn write(facts: &mut Facts, p: &str, prog: &Program) {
    Writer { facts, p: p.to_owned(), terms: 0, atoms: 0, literals: 0, syntax: true }.program(prog);
}

/// What of `prog` its `.rr` text has no syntax for (`program_text`, decision 0155), as facts of program `p`: `program`
/// reads them back as a program of those parts alone.
pub fn write_about(facts: &mut Facts, p: &str, prog: &Program) {
    Writer { facts, p: p.to_owned(), terms: 0, atoms: 0, literals: 0, syntax: false }.program(prog);
}

/// Why facts do not read back as a program.
fn bad(what: impl std::fmt::Display) -> String {
    format!("program facts: {what}")
}

/// One program's rows of the schema, by relation, and the ids they index.
struct Reader<'a, C: Cell> {
    rows: BTreeMap<&'a str, Vec<&'a [C]>>,
    terms: Dense<&'a [C]>,
    call_args: Kids,
    atoms: Dense<&'a [C]>,
    args: Kids,
    /// Each rule's body literals, in order.
    bodies: Kids,
}

/// Rows by id: a vector when the ids are dense (as `Writer` numbers them), else a map. Any numbering reads back.
enum Dense<T> {
    Vec(Vec<Option<T>>),
    Map(rustc_hash::FxHashMap<i64, T>),
}

impl<T> Dense<T> {
    fn of(items: Vec<(i64, T)>) -> Self {
        let n = items.len();
        match items.iter().map(|(k, _)| *k).max() {
            Some(max) if items.iter().all(|(k, _)| *k >= 0) && (max as usize) < 2 * n + 64 => {
                let mut v: Vec<Option<T>> = (0..=max as usize).map(|_| None).collect();
                for (k, t) in items {
                    v[k as usize] = Some(t);
                }
                Dense::Vec(v)
            }
            None => Dense::Vec(Vec::new()),
            _ => Dense::Map(items.into_iter().collect()),
        }
    }

    fn get(&self, k: i64) -> Option<&T> {
        match self {
            Dense::Vec(v) => usize::try_from(k).ok().and_then(|k| v.get(k)).and_then(Option::as_ref),
            Dense::Map(m) => m.get(&k),
        }
    }

    fn take(&mut self, k: i64) -> Option<T> {
        match self {
            Dense::Vec(v) => usize::try_from(k).ok().and_then(|k| v.get_mut(k)).and_then(Option::take),
            Dense::Map(m) => m.remove(&k),
        }
    }
}

/// An owner's children (`arg(a, i, t)`, `call_arg(t, i, u)`, `body(r, k, l)`): each owner's, ordered by index, as
/// one sorted list and the owner's range of it.
struct Kids {
    children: Vec<i64>,
    at: Dense<(u32, u32)>,
}

impl Kids {
    fn of<C: Cell>(rows: &[&[C]]) -> Result<Self, String> {
        let mut all: Vec<(i64, i64, i64)> = rows.iter().map(|r| Ok((int(&r[0])?, int(&r[1])?, int(&r[2])?))).collect::<Result<_, String>>()?;
        all.sort_unstable();
        let mut at: Vec<(i64, (u32, u32))> = Vec::new();
        for (i, (owner, _, _)) in all.iter().enumerate() {
            match at.last_mut() {
                Some((o, (_, n))) if o == owner => *n += 1,
                _ => at.push((*owner, (i as u32, 1))),
            }
        }
        Ok(Kids { children: all.into_iter().map(|(_, _, c)| c).collect(), at: Dense::of(at) })
    }

    fn of_owner(&self, owner: i64) -> &[i64] {
        self.at.get(owner).map_or(&[], |&(s, n)| &self.children[s as usize..(s + n) as usize])
    }
}

fn int<C: Cell>(v: &C) -> Result<i64, String> {
    v.int()
}

/// A string value, borrowed: for a kind matched on, not kept.
fn str_of<C: Cell>(v: &C) -> Result<&str, String> {
    v.str()
}

fn text<C: Cell>(v: &C) -> Result<String, String> {
    v.str().map(str::to_owned)
}

/// `items` by their index column, in order.
fn ordered<T>(mut items: Vec<(i64, T)>) -> Vec<T> {
    items.sort_by_key(|(k, _)| *k);
    items.into_iter().map(|(_, t)| t).collect()
}

impl<'a> Reader<'a, Value> {
    fn of(facts: &'a Facts, p: &str) -> Result<Self, String> {
        let mut rows: BTreeMap<&str, Vec<&[Value]>> = BTreeMap::new();
        for (rel, all) in facts {
            for row in all {
                if matches!(row.first(), Some(Value::Str(q)) if q == p) {
                    rows.entry(rel.as_str()).or_default().push(&row[1..]);
                }
            }
        }
        Self::from_rows(rows, p)
    }
}

impl<'a, C: Cell> Reader<'a, C> {
    fn from_rows(rows: BTreeMap<&'a str, Vec<&'a [C]>>, p: &str) -> Result<Self, String> {
        if !rows.contains_key("program") {
            return Err(bad(format!("no program {p:?}")));
        }
        let get = |rel: &str| rows.get(rel).map_or(&[][..], Vec::as_slice);
        let terms = Dense::of(get("term").iter().map(|row| Ok((int(&row[0])?, *row))).collect::<Result<Vec<_>, String>>()?);
        let atoms = Dense::of(get("atom").iter().map(|row| Ok((int(&row[0])?, *row))).collect::<Result<Vec<_>, String>>()?);
        let call_args = Kids::of(get("call_arg"))?;
        let args = Kids::of(get("arg"))?;
        let bodies = Kids::of(get("body"))?;
        Ok(Reader { rows, terms, call_args, atoms, args, bodies })
    }

    fn get(&self, rel: &str) -> &[&'a [C]] {
        self.rows.get(rel).map_or(&[], Vec::as_slice)
    }

    fn term(&self, t: i64) -> Result<Term, String> {
        let row = self.terms.get(t).ok_or_else(|| bad(format!("no term {t}")))?;
        Ok(match str_of(&row[1])? {
            "var" => Term::Var(text(&row[3])?.into()),
            "wild" => Term::Wild,
            "int" => Term::Int(int(&row[2])?),
            "atom" => Term::Branded(branded(text(&row[3])?.as_str(), int(&row[2])?)?),
            "str" => Term::Str(text(&row[3])?.into()),
            "top" => Term::Top,
            "call" => {
                let args = self.call_args.of_owner(t).iter().map(|u| self.term(*u)).collect::<Result<Vec<_>, _>>()?;
                Term::Call(text(&row[3])?.into(), args.into())
            }
            other => return Err(bad(format!("no term kind {other}"))),
        })
    }

    fn atom(&self, a: i64) -> Result<Atom, String> {
        let row = self.atoms.get(a).ok_or_else(|| bad(format!("no atom {a}")))?;
        let args = self.args.of_owner(a).iter().map(|t| self.term(*t)).collect::<Result<Vec<_>, _>>()?;
        Ok(Atom { rel: text(&row[1])?.into(), args: args.into(), neg: int(&row[2])? != 0, gated: int(&row[3])? != 0 })
    }

    /// Rows of `rel` keyed by their first column (an id), each row less that column.
    fn by_id(&self, rel: &str) -> Result<BTreeMap<i64, Vec<&'a [C]>>, String> {
        let mut out: BTreeMap<i64, Vec<&[C]>> = BTreeMap::new();
        for row in self.get(rel) {
            out.entry(int(&row[0])?).or_default().push(&row[1..]);
        }
        Ok(out)
    }

    fn literals(&self) -> Result<Dense<Literal>, String> {
        let mut out: Vec<(i64, Literal)> = Vec::new();
        for row in self.get("lit_atom") {
            out.push((int(&row[0])?, Literal::Atom(self.atom(int(&row[1])?)?)));
        }
        for row in self.get("lit_cmp") {
            let op = text(&row[2])?;
            let op = super::front::ast::operator(&op).ok_or_else(|| bad(format!("no operator {op:?}")))?;
            out.push((int(&row[0])?, Literal::Cmp { l: self.term(int(&row[1])?)?, op, r: self.term(int(&row[3])?)? }));
        }
        let over = self.by_id("agg_over")?;
        let join = self.by_id("agg_join")?;
        let by = self.by_id("agg_by")?;
        for row in self.get("lit_agg") {
            let l = int(&row[0])?;
            let op = match str_of(&row[2])? {
                "count" => AggOp::Count,
                "min" => AggOp::Min,
                "max" => AggOp::Max,
                "sum" => AggOp::Sum,
                "product" => AggOp::Product,
                "join" => {
                    let sep = join.get(&l).and_then(|r| r.first()).ok_or_else(|| bad(format!("join {l} without its separator")))?;
                    let by = ordered(by.get(&l).into_iter().flatten().map(|r| Ok((int(&r[0])?, text(&r[1])?))).collect::<Result<Vec<_>, String>>()?);
                    AggOp::Join { by, sep: text(&sep[0])? }
                }
                other => return Err(bad(format!("no aggregate {other}"))),
            };
            let over = over.get(&l).and_then(|r| r.first()).map(|r| text(&r[0])).transpose()?;
            out.push((l, Literal::Agg { var: text(&row[1])?, op, over, atom: self.atom(int(&row[3])?)? }));
        }
        for row in self.get("lit_nearest") {
            out.push((int(&row[0])?, Literal::Nearest { n: text(&row[1])?, a: text(&row[2])?, kindset: text(&row[3])? }));
        }
        let captures = self.by_id("ast_capture")?;
        for row in self.get("lit_ast") {
            let l = int(&row[0])?;
            let captures = ordered(captures.get(&l).into_iter().flatten().map(|r| Ok((int(&r[0])?, (text(&r[1])?, text(&r[2])?)))).collect::<Result<Vec<_>, String>>()?);
            out.push((l, Literal::Ast { document: text(&row[1])?, node: text(&row[2])?, matcher: text(&row[3])?, captures }));
        }
        Ok(Dense::of(out))
    }

    fn program(&self) -> Result<Program, String> {
        let mut prog = Program::default();
        let mut literals = self.literals()?;
        let mut rules = Vec::new();
        for row in self.get("rule") {
            let r = int(&row[0])?;
            let body = self.bodies.of_owner(r).iter().map(|l| literals.take(*l).ok_or_else(|| bad(format!("rule {r}: a literal twice or none")))).collect::<Result<Vec<_>, String>>()?;
            rules.push((r, Rule { head: self.atom(int(&row[1])?)?, body: body.into() }));
        }
        prog.rules = ordered(rules);
        let decls = self.by_id_text("decl")?;
        for row in self.get("relation") {
            let rel = text(&row[0])?;
            let cols = ordered(decls.get(&rel).into_iter().flatten().map(|d| Ok((int(&d[0])?, (text(&d[1])?, text(&d[2])?)))).collect::<Result<Vec<_>, String>>()?);
            if cols.len() as i64 != int(&row[1])? {
                return Err(bad(format!("{rel} has {} of its {} columns", cols.len(), int(&row[1])?)));
            }
            prog.decls.insert(rel.into(), cols.iter().map(|(c, t)| (c.into(), t.into())).collect());
        }
        prog.nullary = self.nullary()?;
        prog.qualifiers = self.qualifiers()?;
        prog.tapes = self.tapes()?;
        prog.lattice = self.lattice()?;
        // Keyed by the relation's name as the rows hold it, borrowed.
        let mut values: rustc_hash::FxHashMap<(&str, i64), Vec<(i64, Value)>> = Default::default();
        for row in self.get("fact") {
            let v = match str_of(&row[3])? {
                "int" => Value::Int(int(&row[4])?),
                "atom" => Value::Branded(branded(text(&row[5])?.as_str(), int(&row[4])?)?),
                _ => Value::Str(text(&row[5])?.into()),
            };
            values.entry((str_of(&row[0])?, int(&row[1])?)).or_default().push((int(&row[2])?, v));
        }
        let mut facts: BTreeMap<String, Vec<(i64, Vec<Value>)>> = BTreeMap::new();
        for row in self.get("fact_rel") {
            facts.entry(text(&row[0])?).or_default();
        }
        for row in self.get("fact_row") {
            let (rel, k) = (text(&row[0])?, int(&row[1])?);
            let row_values = ordered(values.remove(&(str_of(&row[0])?, k)).unwrap_or_default());
            if row_values.len() as i64 != int(&row[2])? {
                return Err(bad(format!("{rel} row {k} has {} of its {} values", row_values.len(), int(&row[2])?)));
            }
            facts.entry(rel).or_default().push((k, row_values));
        }
        prog.facts = facts.into_iter().map(|(rel, rows)| (rel, ordered(rows).into_iter().map(crate::code_rule_dsl::logic::front::name::List::from).collect())).collect();
        let list = |rel: &str| -> Result<Vec<String>, String> { Ok(ordered(self.get(rel).iter().map(|r| Ok((int(&r[0])?, text(&r[1])?))).collect::<Result<Vec<_>, String>>()?)) };
        prog.inputs = list("input")?;
        prog.outputs = list("output")?;
        prog.demand = self.demand()?;
        prog.kindsets = self.kindsets()?;
        prog.demand_dropped = self.demand_dropped()?;
        prog.magic_extras = self.magic_extras()?;
        prog.magic_report = self.magic_report()?;
        prog.gates = self.gates()?;
        prog.shapes = self.shapes()?;
        prog.policies = self.policies()?;
        prog.settings = self.settings()?;
        prog.natives = self.natives()?;
        prog.rx_spans = self.rx_spans()?;
        prog.ast_patterns = self.ast_patterns()?;
        prog.doc_scopes = self.doc_scopes()?;
        prog.text_globs = self.text_globs()?;
        prog.answers = self.answers()?;
        prog.asked_by = self.asked()?;
        Ok(prog)
    }

    // -- the fields besides rules, declarations, facts, inputs and outputs, each read alone (`Field`) --

    fn nullary(&self) -> Result<BTreeSet<String>, String> {
        self.get("nullary").iter().map(|r| text(&r[0])).collect()
    }
    fn qualifiers(&self) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
        let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for row in self.get("qualifier") {
            out.entry(text(&row[0])?).or_default().insert(text(&row[1])?);
        }
        Ok(out)
    }
    fn tapes(&self) -> Result<BTreeMap<String, BTreeSet<usize>>, String> {
        let mut out: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
        for row in self.get("tape_column") {
            out.entry(text(&row[0])?).or_default().insert(int(&row[1])? as usize);
        }
        Ok(out)
    }
    fn lattice(&self) -> Result<BTreeMap<String, LatticeDecl>, String> {
        let merges = self.by_id_text("lattice_merge")?;
        let widens = self.by_id_text("lattice_widen")?;
        let mut out = BTreeMap::new();
        for row in self.get("lattice") {
            let rel = text(&row[0])?;
            let merge = merges.get(&rel).and_then(|r| r.first()).map(|r| text(&r[0])).transpose()?;
            let widen = widens.get(&rel).and_then(|r| r.first()).map(|r| int(&r[0]).map(|k| k as u32)).transpose()?;
            out.insert(rel, LatticeDecl { keys: int(&row[1])? as usize, ty: text(&row[2])?, merge, widen });
        }
        Ok(out)
    }
    fn demand(&self) -> Result<Vec<String>, String> {
        Ok(ordered(self.get("demand").iter().map(|r| Ok((int(&r[0])?, text(&r[1])?))).collect::<Result<Vec<_>, String>>()?))
    }
    fn kindsets(&self) -> Result<BTreeMap<String, Vec<String>>, String> {
        let kinds = self.by_id_text("kindset_kind")?;
        let mut out = BTreeMap::new();
        for row in self.get("kindset") {
            let set = text(&row[0])?;
            let members = ordered(kinds.get(&set).into_iter().flatten().map(|r| Ok((int(&r[0])?, text(&r[1])?))).collect::<Result<Vec<_>, String>>()?);
            out.insert(set, members);
        }
        Ok(out)
    }
    fn demand_dropped(&self) -> Result<Vec<(String, String)>, String> {
        Ok(ordered(self.get("demand_dropped").iter().map(|r| Ok((int(&r[0])?, (text(&r[1])?, text(&r[2])?)))).collect::<Result<Vec<_>, String>>()?))
    }
    fn magic_extras(&self) -> Result<Option<usize>, String> {
        self.get("magic_extras").first().map(|r| int(&r[0]).map(|x| x as usize)).transpose()
    }
    fn magic_report(&self) -> Result<Option<Report>, String> {
        let Some(row) = self.get("magic_report").first() else { return Ok(None) };
        let mut lists: BTreeMap<String, Vec<(i64, String)>> = BTreeMap::new();
        for r in self.get("report_list") {
            lists.entry(text(&r[0])?).or_default().push((int(&r[1])?, text(&r[2])?));
        }
        let mut take = |name: &str| ordered(lists.remove(name).unwrap_or_default());
        Ok(Some(Report {
            adornments: take("adornments"),
            magic_rules: int(&row[0])? as usize,
            full: take("full"),
            extras_kept: int(&row[1])? as usize,
            gates: int(&row[2])? as usize,
            full_for_negation: take("full_for_negation"),
        }))
    }
    fn indexed(rows: Option<&Vec<&[C]>>) -> Result<Vec<String>, String> {
        Ok(ordered(rows.into_iter().flatten().map(|r| Ok((int(&r[0])?, text(&r[1])?))).collect::<Result<Vec<_>, String>>()?))
    }
    fn gates(&self) -> Result<Vec<(String, Gate)>, String> {
        let srcs = self.by_id("gate_src")?;
        let quiet = self.by_id("gate_quiet")?;
        let deps = self.by_id("gate_dep")?;
        let mut gates = Vec::new();
        for row in self.get("gate") {
            let k = int(&row[0])?;
            let src = srcs.get(&k).and_then(|r| r.first()).map(|r| text(&r[0])).transpose()?;
            gates.push((k, (text(&row[1])?, Gate { src, rel: text(&row[2])?, level: int(&row[3])? as usize, quiet: Self::indexed(quiet.get(&k))?, deps: Self::indexed(deps.get(&k))? })));
        }
        Ok(ordered(gates))
    }
    fn shapes(&self) -> Result<Vec<Shape>, String> {
        let after = self.by_id("shape_after")?;
        let bound = self.by_id("shape_bound")?;
        let mut shapes = Vec::new();
        for row in self.get("shape") {
            let k = int(&row[0])?;
            let wait = match after.get(&k).and_then(|r| r.first()) {
                Some(r) => ShapeWait::After(text(&r[0])?),
                None => ShapeWait::Bound(Self::indexed(bound.get(&k))?),
            };
            shapes.push((k, Shape { head: text(&row[1])?, lit: text(&row[2])?, wait, text: text(&row[3])? }));
        }
        Ok(ordered(shapes))
    }
    fn policies(&self) -> Result<Vec<serde_json::Value>, String> {
        Ok(ordered(self.get("policy").iter().map(|r| Ok((int(&r[0])?, serde_json::from_str(&text(&r[1])?).map_err(|e| bad(e))?))).collect::<Result<Vec<_>, String>>()?))
    }
    fn settings(&self) -> Result<Vec<(String, String)>, String> {
        Ok(ordered(self.get("setting").iter().map(|r| Ok((int(&r[0])?, (text(&r[1])?, text(&r[2])?)))).collect::<Result<Vec<_>, String>>()?))
    }
    fn natives(&self) -> Result<BTreeMap<String, Native>, String> {
        let mut out = BTreeMap::new();
        let roots = self.by_id_text("native_roots")?;
        for row in self.get("native_dfs") {
            let rel = text(&row[0])?;
            let roots = roots.get(&rel).and_then(|r| r.first()).map(|r| text(&r[0])).transpose()?;
            out.insert(rel, Native::DfsFirst { edge: text(&row[1])?, target: text(&row[2])?, roots });
        }
        let groups = self.by_id_text("native_group")?;
        for row in self.get("native_match") {
            let rel = text(&row[0])?;
            let groups = ordered(groups.get(&rel).into_iter().flatten().map(|r| Ok((int(&r[0])?, text(&r[1])?))).collect::<Result<Vec<_>, String>>()?);
            out.insert(rel, Native::MatchAll { input: text(&row[1])?, pattern: text(&row[2])?, groups, positions: int(&row[3])? != 0 });
        }
        let patterns = self.by_id_text("native_text_pattern")?;
        for row in self.get("native_text_test") {
            let rel = text(&row[0])?;
            let patterns = ordered(patterns.get(&rel).into_iter().flatten().map(|r| Ok((int(&r[0])?, text(&r[1])?))).collect::<Result<Vec<_>, String>>()?);
            out.insert(rel, Native::TextTest { input: text(&row[1])?, column: int(&row[2])? as usize, op: text(&row[3])?, patterns });
        }
        for row in self.get("native_python_candidates") {
            out.insert(text(&row[0])?, Native::PythonCandidates { input: text(&row[1])?, targets: text(&row[2])?, context: text(&row[3])?, member: text(&row[4])? });
        }
        for row in self.get("native_python") {
            let targets = text(&row[3])?;
            out.insert(text(&row[0])?, Native::Python { kind: text(&row[1])?, input: text(&row[2])?, targets: (!targets.is_empty()).then_some(targets) });
        }
        for row in self.get("native_components") {
            out.insert(text(&row[0])?, Native::Components { edge: text(&row[1])?, own: text(&row[2])? });
        }
        for row in self.get("native_reach") {
            out.insert(text(&row[0])?, Native::Reach { seed: text(&row[1])?, edge: text(&row[2])? });
        }
        for row in self.get("native_reach_through") {
            out.insert(text(&row[0])?, Native::ReachThrough { reach: text(&row[1])?, map: text(&row[2])? });
        }
        Ok(out)
    }
    fn rx_spans(&self) -> Result<BTreeSet<(SpanIn, String)>, String> {
        let mut out = BTreeSet::new();
        for row in self.get("rx_span") {
            // `kind:K`, and `capture:G:P` (a group is a name, so the pattern is what follows its colon).
            let named = text(&row[0])?;
            let span = match named.split_once(':') {
                None if named == "source" => SpanIn::Source,
                None if named == "line" => SpanIn::Line,
                None if named == "comment" => SpanIn::Comment,
                Some(("kind", k)) => SpanIn::Kind(k.to_owned()),
                Some(("child_of", k)) => SpanIn::ChildOf(k.to_owned()),
                Some(("capture", rest)) => match rest.split_once(':') {
                    Some((g, p)) => SpanIn::Capture(p.to_owned(), g.to_owned()),
                    None => return Err(bad(format!("no span {named:?}"))),
                },
                _ => return Err(bad(format!("no span {named:?}"))),
            };
            out.insert((span, text(&row[1])?));
        }
        Ok(out)
    }
    fn ast_patterns(&self) -> Result<BTreeSet<String>, String> {
        self.get("ast_pattern").iter().map(|r| text(&r[0])).collect()
    }
    fn doc_scopes(&self) -> Result<Option<Vec<(Vec<String>, Vec<String>)>>, String> {
        if self.get("doc_scoped").is_empty() {
            return Ok(None);
        }
        let mut scopes: BTreeMap<i64, (Vec<(i64, String)>, Vec<(i64, String)>)> = self.get("doc_scope").iter().map(|r| Ok((int(&r[0])?, Default::default()))).collect::<Result<_, String>>()?;
        for row in self.get("doc_scope_glob") {
            let entry = scopes.get_mut(&int(&row[0])?).ok_or_else(|| bad("a scope glob of no entry"))?;
            let side = if int(&row[1])? == 0 { &mut entry.0 } else { &mut entry.1 };
            side.push((int(&row[2])?, text(&row[3])?));
        }
        let ordered_globs = |mut g: Vec<(i64, String)>| -> Vec<String> {
            g.sort();
            g.into_iter().map(|(_, s)| s).collect()
        };
        Ok(Some(scopes.into_values().map(|(w, o)| (ordered_globs(w), ordered_globs(o))).collect()))
    }
    fn text_globs(&self) -> Result<Vec<String>, String> {
        let mut text_globs = self.get("text_glob").iter().map(|r| Ok((int(&r[0])?, text(&r[1])?))).collect::<Result<Vec<_>, String>>()?;
        text_globs.sort();
        Ok(text_globs.into_iter().map(|(_, g)| g).collect())
    }
    fn answers(&self) -> Result<BTreeMap<String, Vec<String>>, String> {
        let mut answers: BTreeMap<String, Vec<(i64, String)>> = BTreeMap::new();
        for row in self.get("answer") {
            answers.entry(text(&row[0])?).or_default().push((int(&row[1])?, text(&row[2])?));
        }
        Ok(answers.into_iter().map(|(req, by)| (req, ordered(by))).collect())
    }
    fn asked(&self) -> Result<BTreeMap<String, Vec<BTreeSet<String>>>, String> {
        let mut out: BTreeMap<String, Vec<BTreeSet<String>>> = BTreeMap::new();
        for row in self.get("asked") {
            let (req, k, by) = (text(&row[0])?, int(&row[1])?, text(&row[2])?);
            let k = usize::try_from(k).map_err(|_| format!("asked: rule {k}"))?;
            let rules = out.entry(req).or_default();
            if rules.len() <= k {
                rules.resize_with(k + 1, BTreeSet::new);
            }
            rules[k].insert(by);
        }
        Ok(out)
    }

    /// Rows of `rel` keyed by their first column (a name), each row less that column.
    fn by_id_text(&self, rel: &str) -> Result<BTreeMap<String, Vec<&'a [C]>>, String> {
        let mut out: BTreeMap<String, Vec<&[C]>> = BTreeMap::new();
        for row in self.get(rel) {
            out.entry(text(&row[0])?).or_default().push(&row[1..]);
        }
        Ok(out)
    }
}

/// The fields of program `p` of `facts` besides its rules, each read alone (`Field`'s rows): what a version's readers
/// read (`program_image::fields`), no program assembled. `facts` need hold only the relations of the fields read.
pub struct Fields<'a>(Reader<'a, Value>);

/// `p`'s fields in `facts` (`Fields`).
pub fn fields<'a>(facts: &'a Facts, p: &str) -> Result<Fields<'a>, String> {
    Reader::of(facts, p).map(Fields)
}

impl Fields<'_> {
    pub fn nullary(&self) -> Result<BTreeSet<String>, String> { self.0.nullary() }
    pub fn qualifiers(&self) -> Result<BTreeMap<String, BTreeSet<String>>, String> { self.0.qualifiers() }
    pub fn tapes(&self) -> Result<BTreeMap<String, BTreeSet<usize>>, String> { self.0.tapes() }
    pub fn lattice(&self) -> Result<BTreeMap<String, LatticeDecl>, String> { self.0.lattice() }
    pub fn demand(&self) -> Result<Vec<String>, String> { self.0.demand() }
    pub fn kindsets(&self) -> Result<BTreeMap<String, Vec<String>>, String> { self.0.kindsets() }
    pub fn demand_dropped(&self) -> Result<Vec<(String, String)>, String> { self.0.demand_dropped() }
    pub fn magic_extras(&self) -> Result<Option<usize>, String> { self.0.magic_extras() }
    pub fn magic_report(&self) -> Result<Option<Report>, String> { self.0.magic_report() }
    pub fn gates(&self) -> Result<Vec<(String, Gate)>, String> { self.0.gates() }
    pub fn shapes(&self) -> Result<Vec<Shape>, String> { self.0.shapes() }
    pub fn policies(&self) -> Result<Vec<serde_json::Value>, String> { self.0.policies() }
    pub fn settings(&self) -> Result<Vec<(String, String)>, String> { self.0.settings() }
    pub fn natives(&self) -> Result<BTreeMap<String, Native>, String> { self.0.natives() }
    pub fn rx_spans(&self) -> Result<BTreeSet<(SpanIn, String)>, String> { self.0.rx_spans() }
    pub fn ast_patterns(&self) -> Result<BTreeSet<String>, String> { self.0.ast_patterns() }
    pub fn doc_scopes(&self) -> Result<Option<Vec<(Vec<String>, Vec<String>)>>, String> { self.0.doc_scopes() }
    pub fn text_globs(&self) -> Result<Vec<String>, String> { self.0.text_globs() }
    pub fn answers(&self) -> Result<BTreeMap<String, Vec<String>>, String> { self.0.answers() }
    pub fn asked(&self) -> Result<BTreeMap<String, Vec<BTreeSet<String>>>, String> { self.0.asked() }
}

/// Program `p` of `facts`, as it was written.
pub fn program(facts: &Facts, p: &str) -> Result<Program, String> {
    Reader::of(facts, p)?.program()
}

/// `facts` as bytes: a compact encoding for the facts memo (strings in a table, each value a tag and a number).
pub fn encode(facts: &Facts) -> Vec<u8> {
    // One pass: each string's id by first use (relation, then its rows' values, in order), the rows written as they
    // are met; the table, which comes first, written after.
    let mut strings: rustc_hash::FxHashMap<&str, u32> = Default::default();
    let mut table: Vec<&str> = Vec::new();
    let mut body = Vec::new();
    let u = |out: &mut Vec<u8>, x: u64| out.extend_from_slice(&x.to_le_bytes());
    u(&mut body, facts.len() as u64);
    for (rel, rows) in facts {
        let next = table.len() as u32;
        let k = *strings.entry(rel.as_str()).or_insert_with(|| {
            table.push(rel.as_str());
            next
        });
        u(&mut body, u64::from(k));
        u(&mut body, rows.len() as u64);
        for row in rows {
            u(&mut body, row.len() as u64);
            for v in row {
                match v {
                    Value::Int(i) => {
                        body.push(0);
                        body.extend_from_slice(&i.to_le_bytes());
                    }
                    Value::Branded(b) => {
                        body.push(2 + b.class as u8);
                        body.extend_from_slice(&u64::from(b.atom).to_le_bytes());
                    }
                    Value::Str(x) => {
                        let next = table.len() as u32;
                        let k = *strings.entry(x.as_str()).or_insert_with(|| {
                            table.push(x.as_str());
                            next
                        });
                        body.push(1);
                        u(&mut body, u64::from(k));
                    }
                }
            }
        }
    }
    let mut out = Vec::with_capacity(body.len() + 8 + table.iter().map(|x| x.len() + 8).sum::<usize>());
    u(&mut out, table.len() as u64);
    for x in &table {
        u(&mut out, x.len() as u64);
        out.extend_from_slice(x.as_bytes());
    }
    out.extend_from_slice(&body);
    out
}

/// Bytes `encode` made, as facts.
pub fn decode(bytes: &[u8]) -> Result<Facts, String> {
    struct At<'a>(&'a [u8], usize);
    impl At<'_> {
        fn take(&mut self, n: usize) -> Result<&[u8], String> {
            let out = self.0.get(self.1..self.1 + n).ok_or_else(|| bad("truncated"))?;
            self.1 += n;
            Ok(out)
        }
        fn u64(&mut self) -> Result<u64, String> {
            Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("eight bytes")))
        }
        fn len(&mut self) -> Result<usize, String> {
            usize::try_from(self.u64()?).map_err(|e| bad(e))
        }
    }
    let mut at = At(bytes, 0);
    let count = at.len()?;
    let mut table = Vec::with_capacity(count.min(1 << 20));
    for _ in 0..count {
        let n = at.len()?;
        table.push(std::str::from_utf8(at.take(n)?).map_err(|e| bad(e))?.to_owned());
    }
    let string = |k: u64| table.get(k as usize).cloned().ok_or_else(|| bad("no such string"));
    let mut facts = Facts::new();
    for _ in 0..at.len()? {
        let rel = string(at.u64()?)?;
        let rows_n = at.len()?;
        let mut rows = Vec::with_capacity(rows_n.min(1 << 20));
        for _ in 0..rows_n {
            let width = at.len()?;
            let mut row = Vec::with_capacity(width.min(64));
            for _ in 0..width {
                row.push(match at.take(1)?[0] {
                    0 => Value::Int(i64::from_le_bytes(at.take(8)?.try_into().expect("eight bytes"))),
                    1 => Value::Str(string(at.u64()?)?.into()),
                    tag @ 2..=4 => Value::Branded(crate::code_rule_dsl::logic::front::ast::Branded {
                        class: [crate::code_rule_dsl::logic::front::ast::Class::Name, crate::code_rule_dsl::logic::front::ast::Class::Kind, crate::code_rule_dsl::logic::front::ast::Class::Field][usize::from(tag - 2)],
                        atom: u32::try_from(at.u64()?).map_err(|e| bad(e))?,
                    }),
                    tag => return Err(bad(format!("no value tag {tag}"))),
                });
            }
            rows.push(row.into());
        }
        facts.insert(rel, rows);
    }
    if at.1 != bytes.len() {
        return Err(bad("bytes after the facts"));
    }
    Ok(facts)
}

/// A digest of `value`'s `Debug` text, made as it is written: two programs compared by it without either's text in
/// memory (a 16 k-rule program's is tens of megabytes).
pub fn debug_digest(value: &impl std::fmt::Debug) -> [u8; 32] {
    use sha2::Digest as _;
    struct Hashing(sha2::Sha256);
    impl std::fmt::Write for Hashing {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            self.0.update(s.as_bytes());
            Ok(())
        }
    }
    let mut out = Hashing(sha2::Sha256::new());
    let _ = std::fmt::write(&mut out, format_args!("{value:?}"));
    out.0.finalize().into()
}
