//! `EXPLAIN` for compiled plans: stable text (no addresses, no hash-map order), identical for
//! every file of a rule since plans are compiled once per rule. This prints the logical plan;
//! the physical plan's printer joins it once S2's operator set is fixed.
use crate::ir::{AggFn, Binding, Column, Expr, FileSource, Lit, Plan, Program, Rel, RepoSource, Sink, StageDef, Total, WindowKind};
use std::fmt::Write;

pub trait SourceName {
    fn name(&self) -> String;
}
impl SourceName for FileSource {
    fn name(&self) -> String {
        match self {
            FileSource::Input(i) => format!("input:{i}"),
            FileSource::Syntax(t) => format!("syntax:{t:?}"),
            FileSource::Stage(s) => format!("stage:{s}"),
            FileSource::Core(c) => format!("core:{}", c.name()),
        }
    }
}
impl SourceName for RepoSource {
    fn name(&self) -> String {
        match self {
            RepoSource::Exported(e) => format!("exported:{e}"),
            RepoSource::Stage(s) => format!("stage:{s}"),
        }
    }
}

fn total(t: Total) -> &'static str {
    match t {
        Total::InputOrder => "input-order",
        Total::Proven => "proven",
        Total::Checked => "checked",
    }
}

fn cols(s: &[Column]) -> String {
    s.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ")
}

fn lit(l: &Lit) -> String {
    match l {
        Lit::Null => "NULL".into(),
        Lit::Int(i) => i.to_string(),
        Lit::Real(r) => format!("{r:?}"),
        Lit::Text(t) => format!("'{}'", t.replace('\'', "''")),
    }
}

struct Printer {
    out: String,
    subs: Vec<String>,
}

impl Printer {
    fn expr<S: SourceName>(&mut self, e: &Expr<S>, indent: usize) -> String {
        let mut x = |e: &Expr<S>| self.expr(e, indent);
        match e {
            Expr::Lit(l) => lit(l),
            Expr::Col { depth: 0, col } => format!("#{col}"),
            Expr::Col { depth, col } => format!("^{depth}#{col}"),
            Expr::Not(a) => format!("NOT {}", x(a)),
            Expr::Neg(a) => format!("-{}", x(a)),
            Expr::Plus(a) => format!("+{}", x(a)),
            Expr::Arith(o, a, b) => format!("({} {:?} {})", x(a), o, x(b)),
            Expr::Concat(a, b) => format!("({} || {})", x(a), x(b)),
            Expr::And(a, b) => format!("({} AND {})", x(a), x(b)),
            Expr::Or(a, b) => format!("({} OR {})", x(a), x(b)),
            Expr::Cmp(o, a, b, (la, ra)) => format!("({} {:?}[{la:?},{ra:?}] {})", x(a), o, x(b)),
            Expr::Is { left, right, negated } => format!("({} IS{} {})", x(left), if *negated { " NOT" } else { "" }, x(right)),
            Expr::IsNull { e, negated } => format!("({} IS{} NULL)", x(e), if *negated { " NOT" } else { "" }),
            Expr::Like { text, pattern, negated } => format!("({}{} LIKE {})", x(text), if *negated { " NOT" } else { "" }, x(pattern)),
            Expr::Func(f, args) => format!("{f:?}({})", args.iter().map(&mut x).collect::<Vec<_>>().join(", ")),
            Expr::Case { operand, whens, else_ } => {
                let mut s = String::from("CASE");
                if let Some(o) = operand {
                    s += &format!(" {}", x(o));
                }
                for (w, t, (la, ra)) in whens {
                    s += &format!(" WHEN[{la:?},{ra:?}] {} THEN {}", x(w), x(t));
                }
                if let Some(e) = else_ {
                    s += &format!(" ELSE {}", x(e));
                }
                s + " END"
            }
            Expr::Cast(a, aff) => format!("CAST({} AS {aff:?})", x(a)),
            Expr::InList { e, list, negated } => format!("({}{} IN ({}))", x(e), if *negated { " NOT" } else { "" }, list.iter().map(|(v, _)| x(v)).collect::<Vec<_>>().join(", ")),
            Expr::InRel { e, rel, negated, .. } => {
                let n = self.sub(rel, indent);
                format!("({}{} IN $sub{n})", self.expr(e, indent), if *negated { " NOT" } else { "" })
            }
            Expr::Exists { rel, negated } => {
                let n = self.sub(rel, indent);
                format!("{}EXISTS $sub{n}", if *negated { "NOT " } else { "" })
            }
            Expr::Scalar(rel) => format!("$sub{}", self.sub(rel, indent)),
        }
    }

    fn sub<S: SourceName>(&mut self, r: &Rel<S>, indent: usize) -> usize {
        let mut p = Printer { out: String::new(), subs: vec![] };
        p.rel(r, indent + 2);
        self.subs.push(p.out);
        self.subs.extend(p.subs);
        self.subs.len() - 1
    }

    fn line(&mut self, indent: usize, s: &str) {
        let _ = writeln!(self.out, "{:indent$}{s}", "");
    }

    fn rel<S: SourceName>(&mut self, r: &Rel<S>, indent: usize) {
        let mark = self.subs.len();
        let head = match r {
            Rel::Scan { src, schema } => format!("Scan {} [{}]", src.name(), cols(schema)),
            Rel::Local { id, schema } => format!("Local {id} [{}]", cols(schema)),
            Rel::Unit => "Unit".into(),
            Rel::JsonEach { arg, on, outer, .. } => format!("JsonEach{} arg {}{}", if *outer { " outer" } else { "" }, self.expr(arg, indent), on.as_ref().map(|o| format!(" on {}", self.expr(o, indent))).unwrap_or_default()),
            Rel::Join { kind, on, .. } => format!("Join {kind:?}{}", on.as_ref().map(|o| format!(" on {}", self.expr(o, indent))).unwrap_or_default()),
            Rel::Filter { pred, .. } => format!("Filter {}", self.expr(pred, indent)),
            Rel::Project { exprs, schema, .. } => {
                let es: Vec<String> = exprs.iter().map(|e| self.expr(e, indent)).collect();
                format!("Project [{}] ({})", cols(schema), es.join(", "))
            }
            Rel::Aggregate { keys, aggs, .. } => {
                let ks: Vec<String> = keys.iter().map(|e| self.expr(e, indent)).collect();
                let ags: Vec<String> = aggs
                    .iter()
                    .map(|a| {
                        let arg = a.arg.as_ref().map(|e| self.expr(e, indent)).unwrap_or_else(|| if a.f == AggFn::CountStar { "*".into() } else { String::new() });
                        let t = if matches!(a.f, AggFn::GroupConcat | AggFn::JsonGroupArray) { format!(" total: {}", total(a.total)) } else { String::new() };
                        format!("{:?}({}{arg}){t}", a.f, if a.distinct { "DISTINCT " } else { "" })
                    })
                    .collect();
                format!("Aggregate keys ({}) aggs ({})", ks.join(", "), ags.join(", "))
            }
            Rel::Window { funcs, .. } => {
                let fs: Vec<String> = funcs
                    .iter()
                    .map(|w| {
                        let k = match &w.kind {
                            WindowKind::RowNumber => "row_number()".to_string(),
                            WindowKind::Min(e) => format!("min({})", self.expr(e, indent)),
                            WindowKind::Max(e) => format!("max({})", self.expr(e, indent)),
                        };
                        let p: Vec<String> = w.partition.iter().map(|e| self.expr(e, indent)).collect();
                        let o: Vec<String> = w.order.iter().map(|(e, d)| format!("{}{}", self.expr(e, indent), if *d { " DESC" } else { "" })).collect();
                        let t = if matches!(w.kind, WindowKind::RowNumber) { format!(" total: {}", total(w.total)) } else { String::new() };
                        format!("{k} partition ({}) order ({}){t}", p.join(", "), o.join(", "))
                    })
                    .collect();
                format!("Window {}", fs.join("; "))
            }
            Rel::Distinct { prefix, .. } => format!("Distinct prefix {prefix}"),
            Rel::Union { all, .. } => format!("Union{}", if *all { " all" } else { "" }),
            Rel::Sort { keys, limit, offset, total: t, .. } => format!(
                "Sort ({}){}{}{}",
                keys.iter().map(|(c, d)| format!("#{c}{}", if *d { " DESC" } else { "" })).collect::<Vec<_>>().join(", "),
                limit.as_ref().map(|e| format!(" limit {}", self.expr(e, indent))).unwrap_or_default(),
                offset.as_ref().map(|e| format!(" offset {}", self.expr(e, indent))).unwrap_or_default(),
                if limit.is_some() || offset.is_some() { format!(" total: {}", total(*t)) } else { String::new() }
            ),
            Rel::With { bindings, .. } => format!("With {}", bindings.iter().map(|(id, b)| format!("{id}{}", if matches!(b, Binding::Fix { set: true, .. }) { ":fix-set" } else if matches!(b, Binding::Fix { .. }) { ":fix-bag" } else { "" })).collect::<Vec<_>>().join(", ")),
        };
        self.line(indent, &head);
        let new_subs: Vec<String> = self.subs.drain(mark..).collect();
        for (k, s) in new_subs.iter().enumerate() {
            self.line(indent + 2, &format!("$sub{}:", mark + k));
            self.out.push_str(s);
        }
        match r {
            Rel::JsonEach { input, .. } | Rel::Filter { input, .. } | Rel::Project { input, .. } | Rel::Aggregate { input, .. } | Rel::Window { input, .. } | Rel::Distinct { input, .. } | Rel::Sort { input, .. } => self.rel(input, indent + 2),
            Rel::Join { left, right, .. } | Rel::Union { left, right, .. } => {
                self.rel(left, indent + 2);
                self.rel(right, indent + 2);
            }
            Rel::With { bindings, body } => {
                for (id, b) in bindings {
                    match b {
                        Binding::Plain(r) => {
                            self.line(indent + 2, &format!("binding {id}:"));
                            self.rel(r, indent + 4);
                        }
                        Binding::Fix { init, step, schema, .. } => {
                            self.line(indent + 2, &format!("binding {id} [{}] init:", cols(schema)));
                            init.iter().for_each(|r| self.rel(r, indent + 4));
                            self.line(indent + 2, &format!("binding {id} step:"));
                            step.iter().for_each(|r| self.rel(r, indent + 4));
                        }
                    }
                }
                self.line(indent + 2, "body:");
                self.rel(body, indent + 4);
            }
            _ => {}
        }
    }

    fn plan<S: SourceName>(&mut self, kind: &str, p: &Plan<S>) {
        self.line(0, &format!("{kind} plan `{}`{}", p.clause, if p.syntax.is_some() { " (syntax spec)" } else { "" }));
        for (i, st) in p.stages.iter().enumerate() {
            match &st.def {
                StageDef::Query { rel } => {
                    self.line(2, &format!("stage:{i} {} [{}]", st.name, cols(&st.schema)));
                    self.rel(rel, 4);
                }
                StageDef::Reach { roots, edges, graph } => {
                    self.line(2, &format!("stage:{i} {} reach {graph:?}", st.name));
                    self.line(4, "roots:");
                    self.rel(roots, 6);
                    if let Some(e) = edges {
                        self.line(4, "edges:");
                        self.rel(e, 6);
                    }
                }
            }
        }
        match &p.sink {
            Sink::Witnesses { input, violation } => self.line(2, &format!("witnesses of input:{input} code {}", violation.code)),
            Sink::Budget { rel, group_by, lte, closure, .. } => {
                self.line(2, &format!("budget group by ({}) count > {lte}{}", group_by.join(", "), if closure.is_empty() { String::new() } else { format!(" closure {}", closure.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ")) }));
                self.rel(rel, 4);
            }
            Sink::Findings { rel, key, code, .. } => {
                self.line(2, &format!("findings key ({}){}", key.join(", "), code.as_ref().map(|c| format!(" code column {c}")).unwrap_or_default()));
                self.rel(rel, 4);
            }
            Sink::Export(list) => {
                for (id, rel) in list {
                    self.line(2, &format!("export:{id}"));
                    self.rel(rel, 4);
                }
            }
        }
    }
}

/// The program's logical plans as stable text.
pub fn explain(p: &Program) -> String {
    let mut pr = Printer { out: String::new(), subs: vec![] };
    pr.line(0, &format!("rule `{}`", p.rule));
    for (i, input) in p.inputs.iter().enumerate() {
        pr.line(2, &format!("input:{i} {} [{}]", input.name, cols(&input.schema)));
    }
    for e in &p.exported {
        pr.line(2, &format!("exported:{} {} [{}]{}{} from per-file plan {}", e.id, e.name, cols(&e.schema), if e.distinct { " distinct" } else { "" }, if e.ordered { " ordered" } else { "" }, e.producer));
    }
    for plan in &p.per_file {
        pr.plan("per-file", plan);
    }
    for plan in &p.repository {
        pr.plan("repository", plan);
    }
    if let Some(n) = p.report_when {
        pr.line(0, &format!("report when findings > {n}"));
    }
    pr.out
}
