//! A rule's parts as text, read by their handles: the text the tree writes (`front`'s `Display`), so a message, an
//! EXPLAIN record or a ledger line reads the same whichever form made it. Variables are named by their rule.
use super::handles::{AggRef, AtomRef, Lit, LitRef, RuleRef, TermRef};
use super::version::Version;
use std::fmt::Write as _;

impl Version {
    /// Term `t` of rule `r`, as `front::Term` writes it.
    pub fn show_term(&self, r: RuleRef<'_>, t: TermRef<'_>) -> String {
        let mut s = String::new();
        self.write_term(&mut s, r, t);
        s
    }
    /// Atom `a` of rule `r`, as `front::Atom` writes it.
    pub fn show_atom(&self, r: RuleRef<'_>, a: AtomRef<'_>) -> String {
        let mut s = String::new();
        self.write_atom(&mut s, r, a);
        s
    }
    /// Literal `l` of rule `r`, as `front::Literal` writes it.
    pub fn show_lit(&self, r: RuleRef<'_>, l: LitRef<'_>) -> String {
        let mut s = String::new();
        self.write_lit(&mut s, r, l);
        s
    }
    /// Rule `r`, as `front::Rule` writes it.
    pub fn show_rule(&self, r: RuleRef<'_>) -> String {
        let mut s = String::new();
        self.write_atom(&mut s, r, r.head());
        s.push_str(" :- ");
        for (i, l) in r.body().enumerate() {
            if i > 0 {
                s.push_str(", ");
            }
            self.write_lit(&mut s, r, l);
        }
        s
    }

    fn var_name(&self, r: RuleRef<'_>, s: super::Slot) -> &str {
        self.text(r.var(s))
    }
    fn write_term(&self, out: &mut String, r: RuleRef<'_>, t: TermRef<'_>) {
        match t {
            TermRef::Var(s) => {
                out.push('$');
                out.push_str(self.var_name(r, s));
            }
            TermRef::Wild => out.push('_'),
            TermRef::Int(n) => {
                let _ = write!(out, "{n}");
            }
            TermRef::Str(a) => out.push_str(&serde_json::to_string(self.text(a)).expect("a string serializes")),
            TermRef::Top => out.push_str("top"),
            TermRef::Call(f, args) => {
                out.push('@');
                out.push_str(self.text(f));
                out.push('(');
                self.write_terms(out, r, args);
                out.push(')');
            }
        }
    }
    fn write_terms<'t>(&self, out: &mut String, r: RuleRef<'_>, ts: impl Iterator<Item = TermRef<'t>>) {
        for (i, t) in ts.enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            self.write_term(out, r, t);
        }
    }
    fn write_atom(&self, out: &mut String, r: RuleRef<'_>, a: AtomRef<'_>) {
        if a.neg() {
            out.push('!');
        }
        out.push_str(self.rel_name(a.rel()));
        out.push('(');
        self.write_terms(out, r, a.args());
        out.push(')');
    }
    fn write_lit(&self, out: &mut String, r: RuleRef<'_>, l: LitRef<'_>) {
        let var = |s| self.var_name(r, s);
        match l.view() {
            Lit::Atom(a) => self.write_atom(out, r, a),
            Lit::Cmp { l, op, r: rt } => {
                self.write_term(out, r, l);
                let _ = write!(out, " {} ", op.as_str());
                self.write_term(out, r, rt);
            }
            Lit::Agg { var: v, op, over, atom } => {
                let over = over.map_or("", var);
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
                }
                self.write_atom(out, r, atom);
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
