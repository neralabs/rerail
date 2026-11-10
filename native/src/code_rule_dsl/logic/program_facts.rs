//! A rule program as facts (decision 0074): every rule, literal, atom and term, every declaration and setting, as a
//! row of a fixed schema. A program is then data a rule can read, a document the store keeps as a tape, and rows the
//! facts memo keeps. A program written as facts reads back as the same program (`program`).
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
//!   `native_group`), `native_components`, `rx_span`, `policy` and `setting`.
//!
//! Each list is indexed (`k`, `i`, `row`), so a row's place is in the row itself: the rows are a set, and any order
//! of them reads back as the same program. Ids (`r`, `l`, `a`, `t`) are numbered per program.

use super::front::{AggOp, Atom, LatticeDecl, Literal, Native, Program, Rule, SpanIn, Term, Value};
use super::magic::{Gate, Report};
use std::collections::BTreeMap;

/// Rows by relation, as a program's `facts` holds them.
pub(in crate::code_rule_dsl) type Facts = BTreeMap<String, Vec<Vec<Value>>>;

/// Each relation of the schema and its columns (`int` or `sym`).
pub(in crate::code_rule_dsl) fn schema() -> BTreeMap<String, Vec<(String, String)>> {
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
        ("native_components", &["p:sym", "rel:sym", "edge:sym", "own:sym"]),
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
        // `ast_patterns`: the tree patterns the program's `ast` atoms match.
        ("ast_pattern", &["p:sym", "pattern:sym"]),
        // `answers`: each request relation's answering programs, in order.
        ("answer", &["p:sym", "req:sym", "i:int", "by:sym"]),
        // `qualifiers`: `.decl r(…) inline` and its kin (decision 0099).
        ("qualifier", &["p:sym", "rel:sym", "word:sym"]),
    ];
    SCHEMA
        .iter()
        .map(|(rel, cols)| {
            let cols = cols.iter().map(|c| c.split_once(':').expect("name:type")).map(|(n, t)| (n.to_owned(), t.to_owned())).collect();
            ((*rel).to_owned(), cols)
        })
        .collect()
}

fn s(x: &str) -> Value {
    Value::Str(x.to_owned())
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
}

impl Writer<'_> {
    fn row(&mut self, rel: &str, mut values: Vec<Value>) {
        values.insert(0, s(&self.p));
        self.facts.entry(rel.to_owned()).or_default().push(values);
    }

    fn term(&mut self, term: &Term) -> i64 {
        let t = self.terms;
        self.terms += 1;
        let (kind, num, text) = match term {
            Term::Var(v) => ("var", 0, v.as_str()),
            Term::Wild => ("wild", 0, ""),
            Term::Int(i) => ("int", *i, ""),
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
        for rel in &prog.nullary {
            self.row("nullary", vec![s(rel)]);
        }
        for (rel, words) in &prog.qualifiers {
            for w in words {
                self.row("qualifier", vec![s(rel), s(w)]);
            }
        }
        for (rel, l) in &prog.lattice {
            self.row("lattice", vec![s(rel), n(l.keys), s(&l.ty)]);
            if let Some(f) = &l.merge {
                self.row("lattice_merge", vec![s(rel), s(f)]);
            }
            if let Some(k) = l.widen {
                self.row("lattice_widen", vec![s(rel), n(k)]);
            }
        }
        for (rel, rows) in &prog.facts {
            for (row, values) in rows.iter().enumerate() {
                self.row("fact_row", vec![s(rel), n(row), n(values.len())]);
                for (i, v) in values.iter().enumerate() {
                    let (kind, num, text) = match v {
                        Value::Int(x) => ("int", *x, ""),
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
        for (set, kinds) in &prog.kindsets {
            self.row("kindset", vec![s(set)]);
            for (k, kind) in kinds.iter().enumerate() {
                self.row("kindset_kind", vec![s(set), n(k), s(kind)]);
            }
        }
        for (k, rel) in prog.demand.iter().enumerate() {
            self.row("demand", vec![n(k), s(rel)]);
        }
        for (k, (a, b)) in prog.demand_dropped.iter().enumerate() {
            self.row("demand_dropped", vec![n(k), s(a), s(b)]);
        }
        if let Some(extras) = prog.magic_extras {
            self.row("magic_extras", vec![n(extras)]);
        }
        if let Some(report) = &prog.magic_report {
            self.row("magic_report", vec![n(report.magic_rules), n(report.extras_kept), n(report.gates)]);
            for (list, items) in [("adornments", &report.adornments), ("full", &report.full), ("full_for_negation", &report.full_for_negation)] {
                for (k, item) in items.iter().enumerate() {
                    self.row("report_list", vec![s(list), n(k), s(item)]);
                }
            }
        }
        for (k, (key, gate)) in prog.gates.iter().enumerate() {
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
        for (k, policy) in prog.policies.iter().enumerate() {
            self.row("policy", vec![n(k), s(&policy.to_string())]);
        }
        for (k, (key, text)) in prog.settings.iter().enumerate() {
            self.row("setting", vec![n(k), s(key), s(text)]);
        }
        for (rel, native) in &prog.natives {
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
                Native::Components { edge, own } => self.row("native_components", vec![s(rel), s(edge), s(own)]),
            }
        }
        if let Some(scopes) = &prog.doc_scopes {
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
        for (i, g) in prog.text_globs.iter().enumerate() {
            self.row("text_glob", vec![n(i), s(g)]);
        }
        for pattern in &prog.ast_patterns {
            self.row("ast_pattern", vec![s(pattern)]);
        }
        for (req, by) in &prog.answers {
            for (i, b) in by.iter().enumerate() {
                self.row("answer", vec![s(req), n(i), s(b)]);
            }
        }
        for (span, text) in &prog.rx_spans {
            // Kinds and captures carry their names after a unit separator.
            let span = match span {
                SpanIn::Source => "source".to_owned(),
                SpanIn::Line => "line".to_owned(),
                SpanIn::Comment => "comment".to_owned(),
                SpanIn::Kind(k) => format!("kind:{k}"),
                SpanIn::Capture(p, g) => format!("capture:{g}:{p}"),
            };
            self.row("rx_span", vec![s(&span), s(text)]);
        }
        for text in &prog.ast_patterns {
            self.row("ast_pattern", vec![s(text)]);
        }
    }
}

/// `prog` as facts of program `p`, added to `facts`.
pub(in crate::code_rule_dsl) fn write(facts: &mut Facts, p: &str, prog: &Program) {
    Writer { facts, p: p.to_owned(), terms: 0, atoms: 0, literals: 0 }.program(prog);
}

/// Why facts do not read back as a program.
fn bad(what: impl std::fmt::Display) -> String {
    format!("program facts: {what}")
}

/// One program's rows of the schema, by relation, and the ids they index.
struct Reader<'a> {
    rows: BTreeMap<&'a str, Vec<&'a [Value]>>,
    terms: BTreeMap<i64, &'a [Value]>,
    call_args: BTreeMap<i64, BTreeMap<i64, i64>>,
    atoms: BTreeMap<i64, &'a [Value]>,
    args: BTreeMap<i64, BTreeMap<i64, i64>>,
}

fn int(v: &Value) -> Result<i64, String> {
    match v {
        Value::Int(i) => Ok(*i),
        Value::Str(x) => Err(bad(format!("{x:?} where a number goes"))),
    }
}

fn text(v: &Value) -> Result<String, String> {
    match v {
        Value::Str(x) => Ok(x.clone()),
        Value::Int(i) => Err(bad(format!("{i} where a string goes"))),
    }
}

/// `items` by their index column, in order.
fn ordered<T>(mut items: Vec<(i64, T)>) -> Vec<T> {
    items.sort_by_key(|(k, _)| *k);
    items.into_iter().map(|(_, t)| t).collect()
}

impl<'a> Reader<'a> {
    fn of(facts: &'a Facts, p: &str) -> Result<Self, String> {
        let mut rows: BTreeMap<&str, Vec<&[Value]>> = BTreeMap::new();
        for (rel, all) in facts {
            for row in all {
                if matches!(row.first(), Some(Value::Str(q)) if q == p) {
                    rows.entry(rel.as_str()).or_default().push(&row[1..]);
                }
            }
        }
        if !rows.contains_key("program") {
            return Err(bad(format!("no program {p:?}")));
        }
        let mut reader = Reader { rows, terms: BTreeMap::new(), call_args: BTreeMap::new(), atoms: BTreeMap::new(), args: BTreeMap::new() };
        for row in reader.get("term") {
            reader.terms.insert(int(&row[0])?, row);
        }
        for row in reader.get("call_arg") {
            reader.call_args.entry(int(&row[0])?).or_default().insert(int(&row[1])?, int(&row[2])?);
        }
        for row in reader.get("atom") {
            reader.atoms.insert(int(&row[0])?, row);
        }
        for row in reader.get("arg") {
            reader.args.entry(int(&row[0])?).or_default().insert(int(&row[1])?, int(&row[2])?);
        }
        Ok(reader)
    }

    fn get(&self, rel: &str) -> Vec<&'a [Value]> {
        self.rows.get(rel).cloned().unwrap_or_default()
    }

    fn term(&self, t: i64) -> Result<Term, String> {
        let row = self.terms.get(&t).ok_or_else(|| bad(format!("no term {t}")))?;
        Ok(match text(&row[1])?.as_str() {
            "var" => Term::Var(text(&row[3])?),
            "wild" => Term::Wild,
            "int" => Term::Int(int(&row[2])?),
            "str" => Term::Str(text(&row[3])?),
            "top" => Term::Top,
            "call" => {
                let args = self.call_args.get(&t).map(|a| a.values().map(|u| self.term(*u)).collect::<Result<Vec<_>, _>>()).transpose()?.unwrap_or_default();
                Term::Call(text(&row[3])?, args)
            }
            other => return Err(bad(format!("no term kind {other}"))),
        })
    }

    fn atom(&self, a: i64) -> Result<Atom, String> {
        let row = self.atoms.get(&a).ok_or_else(|| bad(format!("no atom {a}")))?;
        let args = self.args.get(&a).map(|args| args.values().map(|t| self.term(*t)).collect::<Result<Vec<_>, _>>()).transpose()?.unwrap_or_default();
        Ok(Atom { rel: text(&row[1])?, args, neg: int(&row[2])? != 0, gated: int(&row[3])? != 0 })
    }

    /// Rows of `rel` keyed by their first column (an id), each row less that column.
    fn by_id(&self, rel: &str) -> Result<BTreeMap<i64, Vec<&'a [Value]>>, String> {
        let mut out: BTreeMap<i64, Vec<&[Value]>> = BTreeMap::new();
        for row in self.get(rel) {
            out.entry(int(&row[0])?).or_default().push(&row[1..]);
        }
        Ok(out)
    }

    fn literals(&self) -> Result<BTreeMap<i64, Literal>, String> {
        let mut out = BTreeMap::new();
        for row in self.get("lit_atom") {
            out.insert(int(&row[0])?, Literal::Atom(self.atom(int(&row[1])?)?));
        }
        for row in self.get("lit_cmp") {
            let op = text(&row[2])?;
            let op = super::front::operator(&op).ok_or_else(|| bad(format!("no operator {op:?}")))?;
            out.insert(int(&row[0])?, Literal::Cmp { l: self.term(int(&row[1])?)?, op, r: self.term(int(&row[3])?)? });
        }
        let over = self.by_id("agg_over")?;
        let join = self.by_id("agg_join")?;
        let by = self.by_id("agg_by")?;
        for row in self.get("lit_agg") {
            let l = int(&row[0])?;
            let op = match text(&row[2])?.as_str() {
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
            out.insert(l, Literal::Agg { var: text(&row[1])?, op, over, atom: self.atom(int(&row[3])?)? });
        }
        for row in self.get("lit_nearest") {
            out.insert(int(&row[0])?, Literal::Nearest { n: text(&row[1])?, a: text(&row[2])?, kindset: text(&row[3])? });
        }
        let captures = self.by_id("ast_capture")?;
        for row in self.get("lit_ast") {
            let l = int(&row[0])?;
            let captures = ordered(captures.get(&l).into_iter().flatten().map(|r| Ok((int(&r[0])?, (text(&r[1])?, text(&r[2])?)))).collect::<Result<Vec<_>, String>>()?);
            out.insert(l, Literal::Ast { document: text(&row[1])?, node: text(&row[2])?, matcher: text(&row[3])?, captures });
        }
        Ok(out)
    }

    fn program(&self) -> Result<Program, String> {
        let mut prog = Program::default();
        let mut literals = self.literals()?;
        let bodies = self.by_id("body")?;
        let mut rules = Vec::new();
        for row in self.get("rule") {
            let r = int(&row[0])?;
            let body = ordered(
                bodies.get(&r).into_iter().flatten().map(|b| Ok((int(&b[0])?, literals.remove(&int(&b[1])?).ok_or_else(|| bad(format!("rule {r}: a literal twice or none")))?))).collect::<Result<Vec<_>, String>>()?,
            );
            rules.push((r, Rule { head: self.atom(int(&row[1])?)?, body }));
        }
        prog.rules = ordered(rules);
        let decls = self.by_id_text("decl")?;
        for row in self.get("relation") {
            let rel = text(&row[0])?;
            let cols = ordered(decls.get(&rel).into_iter().flatten().map(|d| Ok((int(&d[0])?, (text(&d[1])?, text(&d[2])?)))).collect::<Result<Vec<_>, String>>()?);
            if cols.len() as i64 != int(&row[1])? {
                return Err(bad(format!("{rel} has {} of its {} columns", cols.len(), int(&row[1])?)));
            }
            prog.decls.insert(rel, cols);
        }
        prog.nullary = self.get("nullary").iter().map(|r| text(&r[0])).collect::<Result<_, _>>()?;
        for row in self.get("qualifier") {
            prog.qualifiers.entry(text(&row[0])?).or_default().insert(text(&row[1])?);
        }
        let merges = self.by_id_text("lattice_merge")?;
        let widens = self.by_id_text("lattice_widen")?;
        for row in self.get("lattice") {
            let rel = text(&row[0])?;
            let merge = merges.get(&rel).and_then(|r| r.first()).map(|r| text(&r[0])).transpose()?;
            let widen = widens.get(&rel).and_then(|r| r.first()).map(|r| int(&r[0]).map(|k| k as u32)).transpose()?;
            prog.lattice.insert(rel, LatticeDecl { keys: int(&row[1])? as usize, ty: text(&row[2])?, merge, widen });
        }
        let mut values: BTreeMap<(String, i64), Vec<(i64, Value)>> = BTreeMap::new();
        for row in self.get("fact") {
            let v = match text(&row[3])?.as_str() {
                "int" => Value::Int(int(&row[4])?),
                _ => Value::Str(text(&row[5])?),
            };
            values.entry((text(&row[0])?, int(&row[1])?)).or_default().push((int(&row[2])?, v));
        }
        let mut facts: BTreeMap<String, Vec<(i64, Vec<Value>)>> = BTreeMap::new();
        for row in self.get("fact_row") {
            let (rel, k) = (text(&row[0])?, int(&row[1])?);
            let row_values = ordered(values.remove(&(rel.clone(), k)).unwrap_or_default());
            if row_values.len() as i64 != int(&row[2])? {
                return Err(bad(format!("{rel} row {k} has {} of its {} values", row_values.len(), int(&row[2])?)));
            }
            facts.entry(rel).or_default().push((k, row_values));
        }
        prog.facts = facts.into_iter().map(|(rel, rows)| (rel, ordered(rows))).collect();
        let list = |rel: &str| -> Result<Vec<String>, String> { Ok(ordered(self.get(rel).iter().map(|r| Ok((int(&r[0])?, text(&r[1])?))).collect::<Result<Vec<_>, String>>()?)) };
        prog.inputs = list("input")?;
        prog.outputs = list("output")?;
        prog.demand = list("demand")?;
        let kinds = self.by_id_text("kindset_kind")?;
        for row in self.get("kindset") {
            let set = text(&row[0])?;
            let members = ordered(kinds.get(&set).into_iter().flatten().map(|r| Ok((int(&r[0])?, text(&r[1])?))).collect::<Result<Vec<_>, String>>()?);
            prog.kindsets.insert(set, members);
        }
        prog.demand_dropped = ordered(self.get("demand_dropped").iter().map(|r| Ok((int(&r[0])?, (text(&r[1])?, text(&r[2])?)))).collect::<Result<Vec<_>, String>>()?);
        prog.magic_extras = self.get("magic_extras").first().map(|r| int(&r[0]).map(|x| x as usize)).transpose()?;
        if let Some(row) = self.get("magic_report").first() {
            let mut lists: BTreeMap<String, Vec<(i64, String)>> = BTreeMap::new();
            for r in self.get("report_list") {
                lists.entry(text(&r[0])?).or_default().push((int(&r[1])?, text(&r[2])?));
            }
            let mut take = |name: &str| ordered(lists.remove(name).unwrap_or_default());
            prog.magic_report = Some(Report {
                adornments: take("adornments"),
                magic_rules: int(&row[0])? as usize,
                full: take("full"),
                extras_kept: int(&row[1])? as usize,
                gates: int(&row[2])? as usize,
                full_for_negation: take("full_for_negation"),
            });
        }
        let srcs = self.by_id("gate_src")?;
        let quiet = self.by_id("gate_quiet")?;
        let deps = self.by_id("gate_dep")?;
        let indexed = |rows: Option<&Vec<&[Value]>>| -> Result<Vec<String>, String> { Ok(ordered(rows.into_iter().flatten().map(|r| Ok((int(&r[0])?, text(&r[1])?))).collect::<Result<Vec<_>, String>>()?)) };
        let mut gates = Vec::new();
        for row in self.get("gate") {
            let k = int(&row[0])?;
            let src = srcs.get(&k).and_then(|r| r.first()).map(|r| text(&r[0])).transpose()?;
            gates.push((k, (text(&row[1])?, Gate { src, rel: text(&row[2])?, level: int(&row[3])? as usize, quiet: indexed(quiet.get(&k))?, deps: indexed(deps.get(&k))? })));
        }
        prog.gates = ordered(gates);
        prog.policies = ordered(self.get("policy").iter().map(|r| Ok((int(&r[0])?, serde_json::from_str(&text(&r[1])?).map_err(|e| bad(e))?))).collect::<Result<Vec<_>, String>>()?);
        prog.settings = ordered(self.get("setting").iter().map(|r| Ok((int(&r[0])?, (text(&r[1])?, text(&r[2])?)))).collect::<Result<Vec<_>, String>>()?);
        let roots = self.by_id_text("native_roots")?;
        for row in self.get("native_dfs") {
            let rel = text(&row[0])?;
            let roots = roots.get(&rel).and_then(|r| r.first()).map(|r| text(&r[0])).transpose()?;
            prog.natives.insert(rel, Native::DfsFirst { edge: text(&row[1])?, target: text(&row[2])?, roots });
        }
        let groups = self.by_id_text("native_group")?;
        for row in self.get("native_match") {
            let rel = text(&row[0])?;
            let groups = ordered(groups.get(&rel).into_iter().flatten().map(|r| Ok((int(&r[0])?, text(&r[1])?))).collect::<Result<Vec<_>, String>>()?);
            prog.natives.insert(rel, Native::MatchAll { input: text(&row[1])?, pattern: text(&row[2])?, groups, positions: int(&row[3])? != 0 });
        }
        for row in self.get("native_components") {
            prog.natives.insert(text(&row[0])?, Native::Components { edge: text(&row[1])?, own: text(&row[2])? });
        }
        for row in self.get("rx_span") {
            // `kind:K`, and `capture:G:P` (a group is a name, so the pattern is what follows its colon).
            let named = text(&row[0])?;
            let span = match named.split_once(':') {
                None if named == "source" => SpanIn::Source,
                None if named == "line" => SpanIn::Line,
                None if named == "comment" => SpanIn::Comment,
                Some(("kind", k)) => SpanIn::Kind(k.to_owned()),
                Some(("capture", rest)) => match rest.split_once(':') {
                    Some((g, p)) => SpanIn::Capture(p.to_owned(), g.to_owned()),
                    None => return Err(bad(format!("no span {named:?}"))),
                },
                _ => return Err(bad(format!("no span {named:?}"))),
            };
            prog.rx_spans.insert((span, text(&row[1])?));
        }
        for row in self.get("ast_pattern") {
            prog.ast_patterns.insert(text(&row[0])?);
        }
        if !self.get("doc_scoped").is_empty() {
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
            prog.doc_scopes = Some(scopes.into_values().map(|(w, o)| (ordered_globs(w), ordered_globs(o))).collect());
        }
        let mut text_globs = self.get("text_glob").iter().map(|r| Ok((int(&r[0])?, text(&r[1])?))).collect::<Result<Vec<_>, String>>()?;
        text_globs.sort();
        prog.text_globs = text_globs.into_iter().map(|(_, g)| g).collect();
        prog.ast_patterns = self.get("ast_pattern").iter().map(|r| text(&r[0])).collect::<Result<_, _>>()?;
        let mut answers: BTreeMap<String, Vec<(i64, String)>> = BTreeMap::new();
        for row in self.get("answer") {
            answers.entry(text(&row[0])?).or_default().push((int(&row[1])?, text(&row[2])?));
        }
        prog.answers = answers.into_iter().map(|(req, by)| (req, ordered(by))).collect();
        Ok(prog)
    }

    /// Rows of `rel` keyed by their first column (a name), each row less that column.
    fn by_id_text(&self, rel: &str) -> Result<BTreeMap<String, Vec<&'a [Value]>>, String> {
        let mut out: BTreeMap<String, Vec<&[Value]>> = BTreeMap::new();
        for row in self.get(rel) {
            out.entry(text(&row[0])?).or_default().push(&row[1..]);
        }
        Ok(out)
    }
}

/// Program `p` of `facts`, as it was written.
pub(in crate::code_rule_dsl) fn program(facts: &Facts, p: &str) -> Result<Program, String> {
    Reader::of(facts, p)?.program()
}

/// `facts` as a `.rr` document: the schema's declarations and every row as a fact. `rr::parse` reads it back as the
/// same rows (`document_facts`).
pub(in crate::code_rule_dsl) fn document(facts: &Facts, extra: &BTreeMap<String, Vec<(String, String)>>) -> Result<String, String> {
    let mut decls = schema();
    decls.extend(extra.iter().map(|(k, v)| (k.clone(), v.clone())));
    let prog = Program { decls, facts: facts.clone(), ..Program::default() };
    super::rr::print(&prog).map_err(|e| bad(format!("{e:?}")))
}

/// A document's rows, as `rr::parse` reads them.
pub(in crate::code_rule_dsl) fn document_facts(text: &str) -> Result<Facts, String> {
    let prog = super::rr::parse(text).map_err(|e| bad(format!("{e:?}")))?;
    Ok(prog.facts)
}

/// `facts` as bytes: a compact encoding for the facts memo (strings in a table, each value a tag and a number).
pub(in crate::code_rule_dsl) fn encode(facts: &Facts) -> Vec<u8> {
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
pub(in crate::code_rule_dsl) fn decode(bytes: &[u8]) -> Result<Facts, String> {
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
                    1 => Value::Str(string(at.u64()?)?),
                    tag => return Err(bad(format!("no value tag {tag}"))),
                });
            }
            rows.push(row);
        }
        facts.insert(rel, rows);
    }
    if at.1 != bytes.len() {
        return Err(bad("bytes after the facts"));
    }
    Ok(facts)
}

#[cfg(test)]
mod tests {
    /// The encoding names each string by its first use and puts the table first: the bytes the two-pass writer made
    /// (strings counted, then the rows written), and they decode to the facts.
    #[test]
    fn facts_encode_as_two_passes_did_and_decode_back() {
        use super::{Facts, Value};
        let mut facts = Facts::new();
        facts.insert("b".into(), vec![vec![Value::Str("x".into()), Value::Int(-3)], vec![Value::Str("b".into()), Value::Str("y".into())]]);
        facts.insert("a".into(), vec![vec![Value::Str("y".into()), Value::Str("z".into()), Value::Int(7)]]);
        let mut want = Vec::new();
        let u = |out: &mut Vec<u8>, x: u64| out.extend_from_slice(&x.to_le_bytes());
        // Strings by first use: a, y, z, b, x.
        let table = ["a", "y", "z", "b", "x"];
        u(&mut want, 5);
        for x in table {
            u(&mut want, x.len() as u64);
            want.extend_from_slice(x.as_bytes());
        }
        u(&mut want, 2);
        let id = |x: &str| table.iter().position(|t| *t == x).unwrap() as u64;
        for (rel, rows) in &facts {
            u(&mut want, id(rel));
            u(&mut want, rows.len() as u64);
            for row in rows {
                u(&mut want, row.len() as u64);
                for v in row {
                    match v {
                        Value::Int(i) => {
                            want.push(0);
                            want.extend_from_slice(&i.to_le_bytes());
                        }
                        Value::Str(x) => {
                            want.push(1);
                            u(&mut want, id(x));
                        }
                    }
                }
            }
        }
        let got = super::encode(&facts);
        assert_eq!(got, want);
        assert_eq!(super::decode(&got).unwrap(), facts);
    }

    /// The documents a program reads (`doc_scopes`) survive its facts: a stored program read back reads the same
    /// documents, not all of them.
    #[test]
    fn document_scopes_read_back_as_written() {
        let mut prog = crate::code_rule_dsl::logic::rr::parse(".decl a(n: node)\n.output a\n").unwrap();
        for scopes in [None, Some(vec![(vec!["impl/**".to_owned(), "api/**".to_owned()], vec!["**/tests/**".to_owned()]), (vec![], vec![])])] {
            prog.doc_scopes = scopes.clone();
            prog.text_globs = if scopes.is_some() { vec!["**/*.md".to_owned(), "docs/*.rst".to_owned()] } else { Vec::new() };
            let mut facts = super::Facts::new();
            super::write(&mut facts, "p", &prog);
            let back = super::program(&facts, "p").unwrap();
            assert_eq!(back.doc_scopes, scopes);
            assert_eq!(back.text_globs, prog.text_globs);
        }
    }

    /// The AST patterns a program matches survive its facts: a stored program read back (a prepared program,
    /// O-stored-patterns) still asks the batch for `ast_match` rows. Without them, a run that reuses it finds no
    /// pattern matches.
    #[test]
    fn ast_patterns_read_back_as_written() {
        let mut prog = crate::code_rule_dsl::logic::rr::parse(".decl a(n: node)\n.output a\n").unwrap();
        prog.ast_patterns = ["$O[:-$K]".to_owned(), "print($$$A)".to_owned()].into();
        let mut facts = super::Facts::new();
        super::write(&mut facts, "p", &prog);
        assert_eq!(super::program(&facts, "p").unwrap().ast_patterns, prog.ast_patterns);
    }
}
