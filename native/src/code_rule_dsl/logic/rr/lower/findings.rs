//! Lossless public findings, lowered through the same relational planner as the rule. The existing
//! five-column `violation` output is an index, not the full diagnostic. One JSON symbol per diagnostic
//! preserves tabs, newlines, numeric parameters, tags and distinct witnesses through the CSV transport.
use super::*;

const OUTPUT: &str = "rr_diagnostic";

fn v(name: &str) -> Term { Term::Var(name.to_owned()) }
fn s(text: &str) -> Term { Term::Str(text.to_owned()) }
fn call(name: &str, args: Vec<Term>) -> Term { Term::Call(name.to_owned(), args) }
fn atom(rel: &str, args: Vec<Term>) -> Literal {
    Literal::Atom(Atom { rel: rel.to_owned(), args, neg: false, gated: false })
}
fn concat(terms: Vec<Term>) -> Term {
    terms.into_iter().reduce(|a, b| call("concat", vec![a, b])).unwrap_or_else(|| s(""))
}
fn json(value: Term) -> Term { call("json_str", vec![value]) }
fn number(value: Term) -> Term { call("to_string", vec![value]) }

impl Lowerer {
    /// Error directives use the same public diagnostic transport as ordinary violations.
    pub(super) fn error_diagnostic(&mut self, policy: &str, clause: &str, code: &str, body: Vec<Literal>) -> Result<(), Error> {
        self.prog.decls.insert(OUTPUT.to_owned(), vec![("json".to_owned(), "sym".to_owned())]);
        if !self.prog.outputs.iter().any(|name| name == OUTPUT) { self.prog.outputs.push(OUTPUT.to_owned()); }
        let finding = Violation {
            name: clause.to_owned(), arity: 1, code: FindingCode::Constant(code.to_owned()), node: false,
            places: true, columns: vec![("at".to_owned(), "span".to_owned())], message: String::new(), tag: None,
        };
        self.diagnostic_rule(policy, &finding, &[Term::Int(0)], Term::Int(0), body, true)
    }

    pub(super) fn diagnostics(&mut self, policy: &str, finding: &Violation) -> Result<(), Error> {
        self.prog.decls.insert(OUTPUT.to_owned(), vec![("json".to_owned(), "sym".to_owned())]);
        if !self.prog.outputs.iter().any(|name| name == OUTPUT) {
            self.prog.outputs.push(OUTPUT.to_owned());
        }
        let cols: Vec<Term> = (0..finding.arity).map(|i| v(&format!("rr_v{i}"))).collect();
        let mut body = vec![atom(&finding.name, cols.clone())];
        if finding.node {
            body.push(atom("node", vec![v("rr_d"), cols[0].clone(), Term::Wild, Term::Wild, Term::Wild]));
        } else {
            body.push(atom("docof", vec![cols[0].clone(), v("rr_d")]));
        }
        body.push(atom("doc", vec![v("rr_d"), v("rr_p"), Term::Wild]));
        body.push(atom("line", vec![cols[0].clone(), v("rr_l")]));
        self.diagnostic_rule(policy, finding, &cols, cols[0].clone(), body, false)?;
        if finding.places {
            let mut args = vec![v("rr_p"), v("rr_l")];
            args.extend(cols.iter().skip(1).cloned());
            let body = vec![atom(&place_relation(&finding.name), args)];
            self.diagnostic_rule(policy, finding, &cols, Term::Int(0), body, true)?;
        }
        Ok(())
    }

    fn diagnostic_rule(&mut self, policy: &str, finding: &Violation, cols: &[Term], at: Term, mut body: Vec<Literal>, place: bool) -> Result<(), Error> {
        let column = |key: &str| finding.columns.iter().position(|(name, _)| name == key)
            .ok_or_else(|| Error::Syntax(format!(".violation {} message: no column {key}", finding.name)));
        let string_column = |i: usize| if matches!(finding.columns[i].1.as_str(), "symbol" | "sym") {
            cols[i].clone()
        } else { number(if i == 0 && place { at.clone() } else { cols[i].clone() }) };
        let mut pieces = Vec::new();
        let mut rest = finding.message.as_str();
        while let Some(start) = rest.find('{') {
            pieces.push(s(&rest[..start]));
            let Some(end) = rest[start + 1..].find('}').map(|end| end + start + 1) else {
                return Err(Error::Syntax(format!(".violation {} message: unclosed placeholder", finding.name)));
            };
            let key = &rest[start + 1..end];
            let value = self.fresh("message");
            let term = if let Some((node, property)) = key.split_once('.') {
                let i = column(node)?;
                match property {
                    "line" if i == 0 => number(v("rr_l")),
                    "line" => { body.push(atom("line", vec![cols[i].clone(), v(&value)])); number(v(&value)) }
                    "name" => {
                        if i == 0 && place { return Err(Error::Syntax(format!(".violation {} message: a place has no name", finding.name))); }
                        let child = self.fresh("message_name");
                        body.push(atom("field", vec![cols[i].clone(), s("name"), v(&child)]));
                        body.push(atom("text", vec![v(&child), v(&value)]));
                        v(&value)
                    }
                    _ => return Err(Error::Syntax(format!(".violation {} message: unsupported property {key}", finding.name))),
                }
            } else if let Some((relation, argument)) = key.strip_suffix(')').and_then(|key| key.split_once('(')) {
                let i = column(argument)?;
                if !self.prog.decls.contains_key(relation) {
                    return Err(Error::Syntax(format!(".violation {} message: no relation {relation}", finding.name)));
                }
                body.push(atom(relation, vec![cols[i].clone(), v(&value)]));
                v(&value)
            } else {
                let i = column(key)?;
                if matches!(finding.columns[i].1.as_str(), "node" | "span" | "loc") {
                    if i == 0 && place { s("") } else {
                        body.push(atom("text", vec![cols[i].clone(), v(&value)]));
                        v(&value)
                    }
                } else { string_column(i) }
            };
            pieces.push(term);
            rest = &rest[end + 1..];
        }
        pieces.push(s(rest));
        let message = concat(pieces);
        let code = match &finding.code {
            FindingCode::Name => s(&finding.name),
            FindingCode::Constant(code) => s(code),
            FindingCode::Column(i) => cols[*i].clone(),
        };
        let tag = finding.tag.map(|i| {
            if matches!(finding.columns[i].1.as_str(), "symbol" | "sym") { json(cols[i].clone()) }
            else { number(cols[i].clone()) }
        }).unwrap_or_else(|| s("null"));
        let mut terms = vec![s("{\"policy\":"), json(s(policy)), s(",\"clause\":"), json(s(&finding.name)),
                             s(",\"code\":"), json(code), s(",\"path\":"), json(v("rr_p")),
                             s(",\"line\":"), number(v("rr_l")), s(",\"at\":"), number(at),
                             s(",\"tag\":"), tag, s(",\"message\":"), json(message), s(",\"parameters\":{")];
        for (i, (name, ty)) in finding.columns.iter().enumerate().skip(1) {
            if i > 1 { terms.push(s(",")); }
            terms.push(s(&format!("{}:", super::super::syntax::quote(name))));
            terms.push(if matches!(ty.as_str(), "symbol" | "sym") { json(cols[i].clone()) } else { number(cols[i].clone()) });
        }
        terms.push(s("}}"));
        body.push(Literal::Cmp { l: v("rr_json"), op: "=", r: concat(terms) });
        self.prog.rules.push(Rule { head: Atom { rel: OUTPUT.to_owned(), args: vec![v("rr_json")], neg: false, gated: false }, body });
        Ok(())
    }
}
