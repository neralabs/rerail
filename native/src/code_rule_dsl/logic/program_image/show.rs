//! A rule's parts as text, read by their handles: the text the tree writes (`front`'s `Display`), so a message, an
//! EXPLAIN record or a ledger line reads the same whichever form made it. Variables are named by their rule.
use super::handles::{AggRef, AtomRef, Lit, LitRef, RuleRef, TermRef};
use super::version::Version;
use std::fmt::Write as _;

/// A naming of a rule's variables, or `None` for their own names.
type Nm<'a> = Option<&'a dyn Fn(super::Slot) -> String>;

impl Version {
    /// Term `t` of rule `r`, as `front::ast::Term` writes it.
    pub fn show_term(&self, r: RuleRef<'_>, t: TermRef<'_>) -> String {
        let mut s = String::new();
        self.write_term(&mut s, r, t, None);
        s
    }
    /// Atom `a` of rule `r`, as `front::ast::Atom` writes it.
    pub fn show_atom(&self, r: RuleRef<'_>, a: AtomRef<'_>) -> String {
        let mut s = String::new();
        self.write_atom(&mut s, r, a, None);
        s
    }
    /// Literal `l` of rule `r`, as `front::ast::Literal` writes it.
    pub fn show_lit(&self, r: RuleRef<'_>, l: LitRef<'_>) -> String {
        let mut s = String::new();
        self.write_lit(&mut s, r, l, None);
        s
    }
    /// Term `t` of rule `r` with its variables named by `name` (a rule about to be written renamed: what tells two
    /// such rules apart).
    pub fn show_term_named(&self, r: RuleRef<'_>, t: TermRef<'_>, name: &dyn Fn(super::Slot) -> String) -> String {
        let mut s = String::new();
        self.write_term(&mut s, r, t, Some(name));
        s
    }
    /// Literal `l` of rule `r` with its variables named by `name`.
    pub fn show_lit_named(&self, r: RuleRef<'_>, l: LitRef<'_>, name: &dyn Fn(super::Slot) -> String) -> String {
        let mut s = String::new();
        self.write_lit(&mut s, r, l, Some(name));
        s
    }
    /// The whole program, for a measurement aid (`version_trace`, 0276 §6): each declaration, the inputs and outputs,
    /// then each rule, a line each, in the version's order.
    pub fn show(&self) -> String {
        let mut s = String::new();
        for d in self.decls() {
            let cols: Vec<String> = d.cols().map(|(n, t)| format!("{}: {}", self.text(n), self.text(t))).collect();
            let _ = writeln!(s, ".decl {}({})", self.text(d.name()), cols.join(", "));
        }
        for i in self.inputs() {
            let _ = writeln!(s, ".input {}", self.text(i));
        }
        for o in self.outputs() {
            let _ = writeln!(s, ".output {}", self.text(o));
        }
        for r in self.rules() {
            let _ = writeln!(s, "{}.", self.show_rule(r));
        }
        s
    }
    /// Each rule's head relation and the relations its body reads, each with whether it is negated, in order: what a
    /// test asks of a program's shape.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn heads_and_reads(&self) -> Vec<(String, Vec<(bool, String)>)> {
        self.rules().map(|r| (self.rel_name(r.head().rel()).to_owned(), r.reads().map(|a| (a.neg(), self.rel_name(a.rel()).to_owned())).collect())).collect()
    }
    /// Rule `r`, as `front::ast::Rule` writes it.
    pub fn show_rule(&self, r: RuleRef<'_>) -> String {
        let mut s = String::new();
        self.write_atom(&mut s, r, r.head(), None);
        s.push_str(" :- ");
        for (i, l) in r.body().enumerate() {
            if i > 0 {
                s.push_str(", ");
            }
            self.write_lit(&mut s, r, l, None);
        }
        s
    }

    fn var_name(&self, r: RuleRef<'_>, s: super::Slot, nm: Nm<'_>) -> std::borrow::Cow<'_, str> {
        match nm {
            Some(name) => name(s).into(),
            None => self.text(r.var(s)).into(),
        }
    }
    fn write_term(&self, out: &mut String, r: RuleRef<'_>, t: TermRef<'_>, nm: Nm<'_>) {
        match t {
            TermRef::Var(s) => {
                out.push('$');
                out.push_str(&self.var_name(r, s, nm));
            }
            TermRef::Wild => out.push('_'),
            TermRef::Int(n) => {
                let _ = write!(out, "{n}");
            }
            TermRef::Str(a) => out.push_str(&serde_json::to_string(self.text(a)).expect("a string serializes")),
            TermRef::Branded(b) => {
                let _ = write!(out, "{b}");
            }
            TermRef::Top => out.push_str("top"),
            TermRef::Call(f, args) => {
                out.push('@');
                out.push_str(self.text(f));
                out.push('(');
                self.write_terms(out, r, args, nm);
                out.push(')');
            }
        }
    }
    fn write_terms<'t>(&self, out: &mut String, r: RuleRef<'_>, ts: impl Iterator<Item = TermRef<'t>>, nm: Nm<'_>) {
        for (i, t) in ts.enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            self.write_term(out, r, t, nm);
        }
    }
    fn write_atom(&self, out: &mut String, r: RuleRef<'_>, a: AtomRef<'_>, nm: Nm<'_>) {
        if a.neg() {
            out.push('!');
        }
        out.push_str(self.rel_name(a.rel()));
        out.push('(');
        self.write_terms(out, r, a.args(), nm);
        out.push(')');
    }
    fn write_lit(&self, out: &mut String, r: RuleRef<'_>, l: LitRef<'_>, nm: Nm<'_>) {
        let var = |s| self.var_name(r, s, nm);
        match l.view() {
            Lit::Atom(a) => self.write_atom(out, r, a, nm),
            Lit::Cmp { l, op, r: rt } => {
                self.write_term(out, r, l, nm);
                let _ = write!(out, " {} ", op.as_str());
                self.write_term(out, r, rt, nm);
            }
            Lit::Agg { var: v, op, over, atom } => {
                let over = over.map_or(std::borrow::Cow::Borrowed(""), var);
                match op {
                    AggRef::Count => {
                        let _ = write!(out, "${} = count {{ ", var(v));
                    }
                    AggRef::Join { by, sep } => {
                        let by: Vec<String> = by.map(|b| format!("${}", var(b))).collect();
                        let sep = serde_json::to_string(self.text(sep)).expect("a string serializes");
                        let _ = write!(out, "${} = join ${over} by ({}) sep {sep} : {{ ", var(v), by.join(", "));
                    }
                    AggRef::Min | AggRef::Max | AggRef::Sum | AggRef::Product => {
                        let op = match op {
                            AggRef::Min => "min",
                            AggRef::Sum => "sum",
                            AggRef::Product => "product",
                            _ => "max",
                        };
                        let _ = write!(out, "${} = {op} ${over} : {{ ", var(v));
                    }
                    AggRef::Bounded { max, .. } => {
                        let _ = write!(out, "${} = {} ${over} : {{ ", var(v), if max { "max" } else { "min" });
                    }
                }
                self.write_atom(out, r, atom, nm);
                if let AggRef::Bounded { max, bound, strict } = op {
                    let _ = write!(out, ", ${over} {} ${}", crate::code_rule_dsl::logic::front::ast::AggOp::bound_cmp(max, strict), var(bound));
                }
                out.push_str(" }");
            }
            Lit::Nearest { n, a, kindset } => {
                let _ = write!(out, "nearest(${}, ${}, {})", var(n), var(a), self.text(kindset));
            }
            Lit::Ast { document, node, matcher, captures } => {
                let _ = write!(out, "ast(${}, ${}, {}", var(document), var(node), self.text(matcher));
                for (name, v) in captures {
                    let _ = write!(out, ", {}: ${}", self.text(name), var(v));
                }
                out.push(')');
            }
        }
    }
}
