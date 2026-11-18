//! Lossless public findings, lowered through the same relational planner as the rule. The existing
//! five-column `violation` output is an index, not the full diagnostic. Each `.violation` (and each error
//! directive) has its own relation of typed columns (decision 0249 §6, `diagnostic_name`): a row describes itself
//! (policy, clause, code, path, line, at, message, its tag and each parameter's name beside its value), so tabs,
//! newlines, numeric parameters, tags and distinct witnesses are kept as values, and no reader parses a text.
use super::*;

/// The relations' name's start: `rr_diagnostic_<digest>`, one relation for each finding and form (a node's or a
/// place's).
pub const PREFIX: &str = "rr_diagnostic_";

/// The columns before a finding's parameters: its policy, clause, code, path, line, place (`at`), message, and
/// whether it has a tag (`tagged`, then `tag`).
pub const HEAD: [&str; 8] = ["policy", "clause", "code", "path", "line", "at", "message", "tagged"];

/// The start of a hole column's name (`hole0`, …): a message placeholder's value, last in a finding's relation. The
/// writer composes `message` from them and the template and writes the row without them (decision 0225 §3).
pub const HOLE: &str = "hole";

/// The index of `message` in a finding's row (`HEAD`).
pub const MESSAGE_COLUMN: usize = 6;

/// Whether `rel` is a finding's relation.
pub fn is_diagnostic(rel: &str) -> bool {
    rel.starts_with(PREFIX)
}

/// A finding's relation, by its policy, clause and form: the same in every program of a run, and another for each
/// policy, so a pack's programs share them as they share `violation` (`corpus::own_names`).
fn diagnostic_name(policy: &str, clause: &str, place: bool) -> String {
    use sha2::Digest as _;
    let mut h = sha2::Sha256::new();
    h.update(policy.as_bytes());
    h.update([0]);
    h.update(clause.as_bytes());
    h.update([0, u8::from(place)]);
    let d: [u8; 32] = h.finalize().into();
    format!("{PREFIX}{}", d[..8].iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// The setting that keeps a finding's message template beside the program (`.message B101 "…"`): what `rr::print`
/// prints and the writer reads where the program's own texts do not hold the `.violation` (decision 0225 §3).
pub const MESSAGE: &str = "message";

/// A message template's texts and placeholders: the texts between the placeholders (one more than the placeholders),
/// and each placeholder's key (`a`, `a.line`, `rel(a)`). `{{` and `}}` are a brace of the text (O111), as the converter
/// writes a literal one (`convert::policy`), and a lone `}` is text.
pub fn message_parts(template: &str) -> Result<(Vec<String>, Vec<String>), String> {
    let (mut texts, mut keys) = (Vec::new(), Vec::new());
    let mut rest = template;
    let mut text = String::new();
    loop {
        let Some(start) = rest.find(['{', '}']) else { break };
        text.push_str(&rest[..start]);
        let brace = &rest[start..start + 1];
        if rest[start + 1..].starts_with(brace) {
            text.push_str(brace);
            rest = &rest[start + 2..];
            continue;
        }
        if brace == "}" {
            text.push('}');
            rest = &rest[start + 1..];
            continue;
        }
        texts.push(std::mem::take(&mut text));
        let Some(end) = rest[start + 1..].find('}').map(|end| end + start + 1) else {
            return Err("unclosed placeholder".to_owned());
        };
        keys.push(rest[start + 1..end].to_owned());
        rest = &rest[end + 1..];
    }
    text.push_str(rest);
    texts.push(text);
    Ok((texts, keys))
}

/// A template's placeholders alone, as a template (`{a}{a.line}`): what lowering reads of it, so what a program's key
/// holds of it (`rr::message_masked`).
pub fn message_skeleton(template: &str) -> String {
    match message_parts(template) {
        Ok((_, keys)) => keys.iter().map(|k| format!("{{{k}}}")).collect(),
        Err(_) => template.to_owned(),
    }
}

/// A message from its template and its placeholders' values, as the rule made them (`diagnostic_rule`).
pub fn message_of(template: &str, values: &[String]) -> String {
    let Ok((texts, _)) = message_parts(template) else { return values.concat() };
    let mut out = String::new();
    for (i, text) in texts.iter().enumerate() {
        out.push_str(text);
        if let Some(v) = values.get(i) {
            out.push_str(v);
        }
    }
    out
}

fn v(name: &str) -> Term { Term::Var(name.to_owned().into()) }
fn s(text: &str) -> Term { Term::Str(text.to_owned().into()) }
fn call(name: &str, args: Vec<Term>) -> Term { Term::Call(name.to_owned().into(), args.into()) }
fn atom(rel: &str, args: Vec<Term>) -> Literal {
    Literal::Atom(Atom { rel: rel.to_owned().into(), args: args.into(), neg: false, gated: false })
}
fn concat(terms: Vec<Term>) -> Term {
    terms.into_iter().reduce(|a, b| call("concat", vec![a, b])).unwrap_or_else(|| s(""))
}
fn number(value: Term) -> Term { call("to_string", vec![value]) }
/// A finding column's type as its relation's column holds it: a text, a node, or the column's own.
fn column_type(ty: &str) -> &str {
    match ty {
        "symbol" | "sym" => "sym",
        "node" | "span" | "loc" => "node",
        "number" | "int" => "int",
        other => other,
    }
}

impl Lowerer {
    /// Error directives use the same public diagnostic transport as ordinary violations.
    pub(super) fn error_diagnostic(&mut self, policy: &str, clause: &str, code: &str, body: Vec<Literal>) -> Result<(), Error> {
        let finding = Violation {
            name: clause.to_owned(), arity: 1, code: FindingCode::Constant(code.to_owned()), node: false,
            places: true, columns: vec![("at".to_owned(), "span".to_owned())], message: String::new(), tag: None,
        };
        self.diagnostic_rule(policy, &finding, &[Term::Int(0)], Term::Int(0), body, true)
    }

    pub(super) fn diagnostics(&mut self, policy: &str, finding: &Violation) -> Result<(), Error> {
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
        // The message's placeholders' values, in order; its texts are not the program's (decision 0225 §3): the
        // writer puts them in from the template (`message_of`), so a message's edit is no edit of the rules.
        let (_, keys) = message_parts(&finding.message).map_err(|e| Error::Syntax(format!(".violation {} message: {e}", finding.name)))?;
        let mut holes = Vec::new();
        for key in &keys {
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
            holes.push(term);
        }
        // The template beside the program, for `rr::print` and the writer (`MESSAGE`).
        let setting = (MESSAGE.to_owned(), format!("{} {}", finding.name, rerail_rr_syntax::syntax::quote(&finding.message)));
        if !finding.message.is_empty() && !self.prog.settings.contains(&setting) {
            self.prog.settings.push(setting);
        }
        // A code from a column of another type than text: its text, as a code is.
        let code = match &finding.code {
            FindingCode::Name => s(&finding.name),
            FindingCode::Constant(code) => s(code),
            FindingCode::Column(i) if matches!(finding.columns[*i].1.as_str(), "symbol" | "sym") => cols[*i].clone(),
            FindingCode::Column(i) => number(cols[*i].clone()),
        };
        // The row: the head (`HEAD`), the tag when there is one, each parameter's name beside its value, then the
        // message's placeholders' values (`HOLE`), a text each. The rule leaves `message` empty: the writer makes it
        // from the template and the holes, and writes the row without them (`compose_message`, decision 0225 §3).
        let at_type = if place { "int" } else { column_type(&finding.columns[0].1) };
        let mut decl: Vec<(String, String)> = HEAD.iter().zip(["sym", "sym", "sym", "sym", "int", at_type, "sym", "int"]).map(|(c, t)| ((*c).to_owned(), t.to_owned())).collect();
        let mut args = vec![s(policy), s(&finding.name), code, v("rr_p"), v("rr_l"), at, s(""), Term::Int(i64::from(finding.tag.is_some()))];
        if let Some(i) = finding.tag {
            decl.push(("tag".to_owned(), column_type(&finding.columns[i].1).to_owned()));
            args.push(cols[i].clone());
        }
        for (i, (name, ty)) in finding.columns.iter().enumerate().skip(1) {
            decl.push((format!("name{i}"), "sym".to_owned()));
            decl.push((format!("value{i}"), column_type(ty).to_owned()));
            args.push(s(name));
            args.push(cols[i].clone());
        }
        for (j, hole) in holes.into_iter().enumerate() {
            decl.push((format!("{HOLE}{j}"), "sym".to_owned()));
            args.push(hole);
        }
        let rel = diagnostic_name(policy, &finding.name, place);
        self.prog.decls.insert(rel.clone().into(), decl.into_iter().map(|(c, t)| (c.into(), t.into())).collect::<Vec<_>>().into());
        if !self.prog.outputs.iter().any(|name| *name == rel) {
            self.prog.outputs.push(rel.clone());
        }
        self.write(Rule { head: Atom { rel: rel.into(), args: args.into(), neg: false, gated: false }, body: body.into() });
        Ok(())
    }
}
