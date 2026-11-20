//! Lossless public findings, lowered through the same relational planner as the rule. The existing
//! five-column `violation` output is an index, not the full diagnostic. Each `.violation` (and each error
//! directive) has its own relation of typed columns (decision 0249 §6, `diagnostic_name`): a row describes itself
//! (policy, clause, code, path, line, at, message, its tag and each parameter's name beside its value), so tabs,
//! newlines, numeric parameters, tags and distinct witnesses are kept as values, and no reader parses a text.
use super::*;
use crate::code_rule_dsl::logic::program_image::fields::FieldsMut;

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

/// The findings of a program's `.violation`s and `.on … report` settings (`Findings::findings`), written over its
/// fields (`FieldsMut`): a version's edit (`lowering_tail::Findings`) or, until the tree goes, a parsed program's.
/// The rules it makes are collected (`rules`), for the caller to place.
pub(super) struct Findings<'a, P: FieldsMut + ?Sized> {
    prog: &'a mut P,
    next: usize,
    made: Vec<(String, usize)>,
    rules: Vec<Rule>,
}

impl<P: FieldsMut + ?Sized> Findings<'_, P> {
    /// A new name, none the program declares (`Lowerer::fresh`).
    fn fresh(&mut self, stem: &str) -> String {
        let prog = &*self.prog;
        super::fresh_name(&mut self.next, &mut self.made, |n| prog.declares(n), stem)
    }

    fn write(&mut self, rule: Rule) {
        self.rules.push(rule);
    }

    /// The findings of the `.violation`s: `violation(policy, V, code, path, line)` for each row of V, at its first
    /// column's line (the engine's findings relation, as the converter's programs give it). A node's document is
    /// `node`'s column; a span's is `docof` (`core.rr`), and so is its line. A `@place(path, line)` row is read from
    /// V's place relation; a span or loc `at` that holds a `@place` value made elsewhere (`PLACE`) is found at its
    /// path and line.
    fn findings(&mut self, mut violations: Vec<Violation>, seen: RulesSeen) -> Result<bool, Error> {
        // `.on syntax_error report CODE [at line 1]` and `.on input_error report CODE` (`core.rr`).
        let on: Vec<(String, String, bool)> = self
            .prog
            .settings()
            .into_iter()
            .filter(|(k, _)| k == "on")
            .filter_map(|(_, text)| {
                let words: Vec<&str> = text.split_whitespace().collect();
                match words.as_slice() {
                    [what @ ("syntax_error" | "input_error"), "report", code, rest @ ..] => Some(((*what).to_owned(), (*code).to_owned(), rest == ["at", "line", "1"])),
                    _ => None,
                }
            })
            .collect();
        let guest_errors = self.guest_policy(seen.reads_guest_policy);
        self.references_setting()?;
        if violations.is_empty() && on.is_empty() && guest_errors.is_empty() {
            return Ok(false);
        }
        let policy = self
            .prog
            .settings()
            .into_iter()
            .find(|(k, _)| k == "policy")
            .and_then(|(_, text)| text.split('"').nth(1).map(str::to_owned))
            .unwrap_or_default();
        let scoped = self.scope_filter();
        // A finding is its rule, clause, code, path, line and column (the anchor's 0-based byte column): two findings
        // on one line are two rows. A finding with no column (a place, an error of a file or input) has column 0.
        self.prog.declare("violation", &[("rule", "sym"), ("clause", "sym"), ("code", "sym"), ("path", "sym"), ("line", "int"), ("col", "int")]);
        // A program that derives `violation` itself, in the older five columns, is read at column 0: the caller's
        // (`widen_violation_heads`), for this sees no rules.
        // Where it is already (a printed program's `.output`), else last.
        self.prog.add_output("violation");
        let v = |s: &str| Term::Var(s.to_owned().into());
        let atom = |rel: &str, args: Vec<Term>| Literal::Atom(Atom { rel: rel.to_owned().into(), args: args.into(), neg: false, gated: false });
        // Whether a rule makes a `@place` value: only then can a span or loc `at` hold one.
        let place_values = seen.place_values;
        for f in std::mem::take(&mut violations) {
            self.diagnostics(&policy, &f)?;
            let code_col = match f.code {
                FindingCode::Column(i) => Some(i),
                _ => None,
            };
            let code = match &f.code {
                FindingCode::Name => Term::Str(f.name.clone().into()),
                FindingCode::Column(i) => v(&format!("rr_v{i}")),
                FindingCode::Constant(c) => Term::Str(c.clone().into()),
            };
            let head = Atom { rel: "violation".into(), args: [Term::Str(policy.clone().into()), Term::Str(f.name.clone().into()), code, v("rr_p"), v("rr_l"), v("rr_c")].into(), neg: false, gated: false };
            let cols: Vec<Term> = (0..f.arity).map(|i| if i == 0 || Some(i) == code_col { v(&format!("rr_v{i}")) } else { Term::Wild }).collect();
            let at = if f.node {
                vec![atom("node", vec![v("rr_d"), v("rr_v0"), Term::Wild, Term::Wild, Term::Wild]), atom("doc", vec![v("rr_d"), v("rr_p"), Term::Wild]), atom("line", vec![v("rr_v0"), v("rr_l")]), atom("col", vec![v("rr_v0"), v("rr_c")])]
            } else {
                vec![atom("docof", vec![v("rr_v0"), v("rr_d")]), atom("doc", vec![v("rr_d"), v("rr_p"), Term::Wild]), atom("line", vec![v("rr_v0"), v("rr_l")]), atom("col", vec![v("rr_v0"), v("rr_c")])]
            };
            self.write(Rule { head: head.clone(), body: std::iter::once(atom(&f.name, cols.clone())).chain(at).chain(scoped.iter().cloned()).collect() });
            if !f.node && place_values {
                let of = |g: &str| Term::Call(g.to_owned().into(), [v("rr_v0")].into());
                let mut found = head.clone();
                found.args.make_mut()[3] = of(PLACE_PATH);
                found.args.make_mut()[4] = of(PLACE_LINE);
                found.args.make_mut()[5] = Term::Int(0);
                let mut body = vec![atom(&f.name, cols.clone()), Literal::Cmp { l: v("rr_v0"), op: "<", r: Term::Int(UNSET) }];
                // The scope tests the place's path, bound for it here.
                if !scoped.is_empty() {
                    body.push(Literal::Cmp { l: v("rr_p"), op: "=", r: of(PLACE_PATH) });
                    body.extend(scoped.iter().cloned());
                }
                self.write(Rule { head: found, body: body.into() });
            }
            if f.places {
                let mut args = vec![v("rr_p"), v("rr_l")];
                args.extend(cols.into_iter().skip(1));
                let mut head = head;
                head.args.make_mut()[5] = Term::Int(0);
                self.write(Rule { head, body: std::iter::once(atom(&place_relation(&f.name), args)).chain(scoped.iter().cloned()).collect() });
            }
        }
        // A syntax error at its line (or line 1), in its document; an input error at line 1 of the input's path.
        for (what, code, line_one) in on {
            let head = Atom { rel: "violation".into(), args: [Term::Str(policy.clone().into()), Term::Str(what.clone().into()), Term::Str(code.clone().into()), v("rr_p"), v("rr_l"), Term::Int(0)].into(), neg: false, gated: false };
            let body = if what == "syntax_error" {
                let line = if line_one { Term::Wild } else { v("rr_l") };
                // A file's own error: a guest's is `.on guest_error`'s.
                let not_guest = Literal::Atom(Atom { rel: "guest".into(), args: [v("rr_d"), Term::Wild, Term::Wild, Term::Wild].into(), neg: true, gated: false });
                let mut body = vec![atom("syntax_error", vec![v("rr_d"), line, Term::Wild]), atom("doc", vec![v("rr_d"), v("rr_p"), Term::Wild]), not_guest];
                body.extend(scoped.iter().cloned());
                if line_one {
                    body.push(Literal::Cmp { l: v("rr_l"), op: "=", r: Term::Int(1) });
                }
                body
            } else {
                vec![atom("input_error", vec![v("rr_p"), Term::Wild, Term::Wild]), Literal::Cmp { l: v("rr_l"), op: "=", r: Term::Int(1) }]
            };
            for rel in [what.as_str(), "doc"] {
                if self.prog.declares(rel) {
                    self.prog.add_input(rel);
                }
            }
            self.error_diagnostic(&policy, &what, &code, body.clone())?;
            if what == "syntax_error" {
                self.guest_input();
            }
            self.write(Rule { head, body: body.into() });
        }
        // A guest that does not parse, reported at its host's line (`syntax_error` of a guest is a file line).
        for (grammar, code) in guest_errors {
            let head = Atom { rel: "violation".into(), args: [Term::Str(policy.clone().into()), Term::Str("guest_error".into()), Term::Str(code.clone().into()), v("rr_p"), v("rr_l"), Term::Int(0)].into(), neg: false, gated: false };
            let g = grammar.map_or(Term::Wild, |g| Term::Str(g.into()));
            let body = vec![atom("syntax_error", vec![v("rr_d"), v("rr_l"), Term::Wild]), atom("guest", vec![v("rr_d"), Term::Wild, Term::Wild, g]), atom("doc", vec![v("rr_d"), v("rr_p"), Term::Wild])];
            for (rel, cols) in [
                ("syntax_error", &[("d", "int"), ("l", "int"), ("msg", "sym")][..]),
                ("doc", &[("d", "int"), ("path", "sym"), ("module", "sym")][..]),
            ] {
                self.prog.declare_absent(rel, cols);
                self.prog.add_input(rel);
            }
            self.guest_input();
            self.error_diagnostic(&policy, "guest_error", &code, body.clone())?;
            self.write(Rule { head, body: body.into() });
        }
        Ok(true)
    }

    /// The `.scope` settings as a test of a finding's path `$rr_p` (`core.rr`): a document's path is in scope in one
    /// of `dirs` (each a directory or a glob of one, at the root) or `include`'s globs when there are any, in none
    /// of `exclude`'s, and with `tests false` under no `test` or `tests` path part. Findings in other documents are
    /// dropped; one at a path that is no document (a budget file) is kept. Facts still come from every file. A
    /// `.scope given` program reads the files it is given, all in scope.
    fn scope_filter(&mut self) -> Vec<Literal> {
        let settings = self.prog.settings();
        let report = scope_report_globs(&settings);
        let given = settings.iter().any(|(k, t)| k == "scope" && t.trim() == "given");
        let (within, out) = match scope_globs(&settings) {
            Some(globs) => globs,
            None if !report.is_empty() && !given => (Vec::new(), Vec::new()),
            None => return Vec::new(),
        };
        // The corpus reads only these documents (`front::Program::doc_scopes`): their facts are the policy's, as the
        // scripts' discovery has them; the drop below still holds for a combined run that reads more.
        if !within.is_empty() || !out.is_empty() {
            self.prog.set_field(crate::code_rule_dsl::logic::program_facts::Field::DocScopes(Some(&[(within.clone(), out.clone())])));
        }
        let v = |s: &str| Term::Var(s.to_owned().into());
        let doc = Literal::Atom(Atom { rel: "doc".into(), args: [v("d"), v("p"), Term::Wild].into(), neg: false, gated: false });
        let glob = |g: &str| Literal::Cmp { l: Term::Call("glob".into(), [v("p"), Term::Str(g.to_owned().into())].into()), op: "=", r: Term::Int(1) };
        // An entry from a policy option (`front::OPTION_GLOB`): the option's values, as `option_item` holds them; a
        // directory's documents are those under it.
        let option_entry = |g: &str| g.strip_prefix(crate::code_rule_dsl::logic::front::OPTION_GLOB).and_then(|r| r.split_once(':')).map(|(f, n)| (f.to_owned(), n.to_owned()));
        let by_option = |form: &str, name: &str| -> Vec<Literal> {
            let item = Literal::Atom(Atom { rel: "option_item".into(), args: [Term::Str(name.to_owned().into()), Term::Wild, v("rr_o")].into(), neg: false, gated: false });
            let test = if form == "dirs" {
                Literal::Cmp { l: Term::Call("prefix".into(), [v("p"), Term::Call("concat".into(), [v("rr_o"), Term::Str("/".into())].into())].into()), op: "=", r: Term::Int(1) }
            } else {
                Literal::Cmp { l: Term::Call("glob".into(), [v("p"), v("rr_o")].into()), op: "=", r: Term::Int(1) }
            };
            vec![item, test]
        };
        if within.iter().chain(&out).chain(&report).any(|g| option_entry(g).is_some()) {
            self.prog.declare_absent("option_item", &[("name", "sym"), ("i", "int"), ("v", "sym")]);
            self.prog.add_input("option_item");
        }
        let test = |g: &str| -> Vec<Literal> {
            match option_entry(g) {
                Some((form, name)) => by_option(&form, &name),
                None => vec![glob(g)],
            }
        };
        let atom = |rel: &str, args: Vec<Term>, neg: bool| Literal::Atom(Atom { rel: rel.to_owned().into(), args: args.into(), neg, gated: false });
        let head = |rel: &str, var: &str| Atom { rel: rel.to_owned().into(), args: [v(var)].into(), neg: false, gated: false };
        self.prog.declare_absent("doc", &[("d", "int"), ("path", "sym"), ("module", "sym")]);
        self.prog.add_input("doc");
        // rr_scope_drop(p): a document's path out of the scope.
        self.prog.declare("rr_scope_drop", &[("p", "sym")]);
        if !within.is_empty() {
            self.prog.declare("rr_scope_in", &[("d", "int")]);
            for g in &within {
                self.write(Rule { head: head("rr_scope_in", "d"), body: std::iter::once(doc.clone()).chain(test(g)).collect() });
            }
            self.write(Rule { head: head("rr_scope_drop", "p"), body: [doc.clone(), atom("rr_scope_in", vec![v("d")], true)].into() });
        }
        for g in &out {
            self.write(Rule { head: head("rr_scope_drop", "p"), body: std::iter::once(doc.clone()).chain(test(g)).collect() });
        }
        // rr_scope_not(d): a document out of the scope, which a combined run's guards negate (decision 0091): in a
        // pack that reads more documents than this policy's scope, the policy's rules see only the others' rows.
        if !within.is_empty() || !out.is_empty() {
            self.prog.declare(SCOPE_NOT, &[("d", "int")]);
            if !within.is_empty() {
                self.write(Rule { head: head(SCOPE_NOT, "d"), body: [doc.clone(), atom("rr_scope_in", vec![v("d")], true)].into() });
            }
            for g in &out {
                self.write(Rule { head: head(SCOPE_NOT, "d"), body: std::iter::once(doc.clone()).chain(test(g)).collect() });
            }
        }
        // `.scope report dirs`: a finding in a document under none of them is dropped too.
        if !report.is_empty() {
            self.prog.declare("rr_report_in", &[("d", "int")]);
            for g in &report {
                self.write(Rule { head: head("rr_report_in", "d"), body: [doc.clone(), glob(g)].into() });
            }
            self.write(Rule { head: head("rr_scope_drop", "p"), body: [doc.clone(), atom("rr_report_in", vec![v("d")], true)].into() });
        }
        vec![atom("rr_scope_drop", vec![v("rr_p")], true)]
    }

    /// `guest(g, host, at, grammar)` as an input (`core.rr` declares it; a program without it has it here).
    fn guest_input(&mut self) {
        self.prog.declare_absent("guest", &[("g", "int"), ("host", "int"), ("at", "node"), ("grammar", "sym")]);
        self.prog.add_input("guest");
    }

    /// The `.references` setting (W's: `siblingLinks`, `constructorFields`, `scope`) as the facts
    /// `python/references.rr` reads (decision 0476 R5), in a program that includes it: `references_given(1)`, the
    /// scope's globs (`**` when it names none; none when it includes nothing), each sibling link, and the constructor
    /// fields' renaming. Each renaming is W's `Regex::replace`, the first match only: it is given as a regex whose
    /// every match is that one (`first_match`), for `@rx_replace`. No setting, no facts.
    fn references_setting(&mut self) -> Result<(), Error> {
        let settings: Vec<String> = self.prog.settings().into_iter().filter(|(k, _)| k == "references").map(|(_, v)| v).collect();
        if settings.is_empty() || !self.prog.declares(REFERENCE_SETTING[0]) {
            return Ok(());
        }
        let bad = |m: String| Error::Syntax(format!(".references: {m}"));
        let [text] = settings.as_slice() else { return Err(bad("more than one setting".into())) };
        let value: serde_json::Value = serde_json::from_str(text.trim()).map_err(|e| bad(e.to_string()))?;
        let object = value.as_object().ok_or_else(|| bad("not a JSON object".into()))?;
        if let Some(key) = object.keys().find(|k| !["siblingLinks", "constructorFields", "scope"].contains(&k.as_str())) {
            return Err(bad(format!("unknown key {key}")));
        }
        let str_at = |v: &serde_json::Value, key: &str, at: &str| -> Result<String, Error> {
            v.get(key).and_then(serde_json::Value::as_str).map(str::to_owned).ok_or_else(|| bad(format!("{at}/{key} must be a string")))
        };
        let regex = |re: &str, at: &str| regex::Regex::new(re).map(drop).map_err(|e| bad(format!("{at}: {e}")));
        let s = |t: &str| Value::Str(t.into());
        let mut rows: Vec<(&str, Vec<Value>)> = vec![("references_given", vec![Value::Int(1)])];
        let scope = object.get("scope");
        let globs = |key: &str| -> Result<Vec<String>, Error> {
            // No scope is every file; a scope names both arrays (`compile::compile_scope`).
            match scope.map(|s| s.get(key)) {
                None => Ok(if key == "include" { vec!["**".to_owned()] } else { Vec::new() }),
                Some(None) => Err(bad(format!("/scope/{key} must be an array"))),
                Some(Some(v)) => v
                    .as_array()
                    .ok_or_else(|| bad(format!("/scope/{key} must be an array")))?
                    .iter()
                    .map(|g| g.as_str().map(str::to_owned).ok_or_else(|| bad(format!("/scope/{key} holds a non-string"))))
                    .collect(),
            }
        };
        rows.extend(globs("include")?.iter().map(|g| ("references_include", vec![s(g)])));
        rows.extend(globs("exclude")?.iter().map(|g| ("references_exclude", vec![s(g)])));
        if let Some(links) = object.get("siblingLinks") {
            let links = links.as_array().filter(|l| (1..=32).contains(&l.len())).ok_or_else(|| bad("siblingLinks must hold 1 to 32 links".into()))?;
            for (i, link) in links.iter().enumerate() {
                let at = format!("/siblingLinks/{i}");
                let decorator = str_at(link, "decorator", &at)?;
                let name = link.get("name").ok_or_else(|| bad(format!("{at}/name is required")))?;
                let (replace, by) = (str_at(name, "replace", &format!("{at}/name"))?, str_at(name, "by", &format!("{at}/name"))?);
                regex(&decorator, &format!("{at}/decorator"))?;
                regex(&replace, &format!("{at}/name/replace"))?;
                let (re, by) = first_match(&replace, &by);
                rows.push(("references_sibling", vec![Value::Int(i as i64), s(&decorator), s(&replace), s(&re), s(&by)]));
            }
        }
        if let Some(fields) = object.get("constructorFields") {
            let at = "/constructorFields/fieldToParameter";
            let spec = fields.get("fieldToParameter").ok_or_else(|| bad(format!("{at} is required")))?;
            let (replace, by) = (str_at(spec, "replace", at)?, str_at(spec, "by", at)?);
            regex(&replace, at)?;
            let (re, by) = first_match(&replace, &by);
            rows.push(("references_field", vec![s(&re), s(&by)]));
        }
        for (rel, row) in rows {
            self.prog.add_facts(rel, &[crate::code_rule_dsl::logic::front::List::from(row)]);
        }
        Ok(())
    }

    /// Decision 0055's settings as `guest_policy` facts, which the corpus runner and the batch read:
    /// - `.guests on | off`: `guest_policy("library", …)`, whether the libraries' `embed`s are made (on by default);
    /// - `.on guest_error [GRAMMAR] skip | partial [warn | report CODE]`, or `.on guest_error [GRAMMAR] warn | report
    ///   CODE` (skip): `guest_policy("errors" | "errors:GRAMMAR", "skip" | "partial")`. A skipped guest is
    ///   withheld; a partial one keeps the tree it has. `warn` and `report` both give a finding, `warn`'s of severity
    ///   warning (`.severity guest_error warning`); the findings are returned, `(grammar, code)`, for `findings`.
    fn guest_policy(&mut self, reads: bool) -> Vec<(Option<String>, String)> {
        let mut facts = Vec::new();
        let mut findings = Vec::new();
        let mut warn = false;
        let settings = self.prog.settings();
        for (key, text) in &settings {
            let words: Vec<&str> = text.split_whitespace().collect();
            if key == "guests" {
                if let [v @ ("on" | "off")] = words.as_slice() {
                    facts.push(("library".to_owned(), (*v).to_owned()));
                }
                continue;
            }
            // `.scope embedded false` keeps the policy to primary documents: `.guests off` (`core.rr`).
            if key == "scope" {
                if let ["embedded", v @ ("true" | "false")] = words.as_slice() {
                    facts.push(("library".to_owned(), if *v == "true" { "on" } else { "off" }.to_owned()));
                }
                continue;
            }
            let ["guest_error", rest @ ..] = words.as_slice() else { continue };
            if key != "on" {
                continue;
            }
            let (grammar, rest) = match rest {
                [g, rest @ ..] if !matches!(*g, "skip" | "partial" | "warn" | "report") => (Some((*g).to_owned()), rest),
                _ => (None, rest),
            };
            let (mode, rest) = match rest {
                [m @ ("skip" | "partial"), rest @ ..] => (*m, rest),
                _ => ("skip", rest),
            };
            facts.push((grammar.as_ref().map_or("errors".to_owned(), |g| format!("errors:{g}")), mode.to_owned()));
            if let [how @ ("warn" | "report"), code] = rest {
                warn |= *how == "warn";
                findings.push((grammar, (*code).to_owned()));
            }
        }
        if warn {
            let mut all = settings.clone();
            all.push(("severity".to_owned(), "guest_error warning".to_owned()));
            self.prog.set_field(crate::code_rule_dsl::logic::program_facts::Field::Settings(&all));
        }
        // The libraries' `embed`s are made by default: a program that reads the setting and sets none reads
        // `guest_policy("library", "on")`, a stated fact, so `!guest_policy("library", "off")` is decided before the
        // run (`simplify`) instead of reading a relation nothing gives.
        // A program printed with the fact states it already.
        let stated = self.prog.fact_rows(GUEST_POLICY).iter().any(|r| matches!(r.first(), Some(Value::Str(k)) if k == "library"));
        if reads && !stated && !facts.iter().any(|(k, _)| k == "library") {
            facts.push(("library".to_owned(), "on".to_owned()));
        }
        if !facts.is_empty() {
            self.prog.declare(GUEST_POLICY, &[("k", "sym"), ("v", "sym")]);
            let rows: Vec<crate::code_rule_dsl::logic::front::List<Value>> = facts.into_iter().map(|(k, v)| [Value::Str(k.into()), Value::Str(v.into())].into()).collect();
            self.prog.add_facts(GUEST_POLICY, &rows);
        }
        findings
    }

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
                if !self.prog.declares(relation) {
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
        if !finding.message.is_empty() {
            let mut all = self.prog.settings();
            if !all.contains(&setting) {
                all.push(setting);
                self.prog.set_field(crate::code_rule_dsl::logic::program_facts::Field::Settings(&all));
            }
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
        let pairs: Vec<(&str, &str)> = decl.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
        self.prog.declare(&rel, &pairs);
        self.prog.add_output(&rel);
        self.write(Rule { head: Atom { rel: rel.into(), args: args.into(), neg: false, gated: false }, body: body.into() });
        Ok(())
    }
}

/// The findings (`Findings`) of `pending` over `prog`'s fields: the rules they make, and whether they wrote the
/// findings (then the program's own `violation` heads of five columns are widened, `widen_violation_heads`).
pub fn findings_on(prog: &mut (impl FieldsMut + ?Sized), pending: &PendingFindings, seen: RulesSeen) -> Result<(Vec<Rule>, bool), Error> {
    let mut f = Findings { prog, next: pending.next, made: Vec::new(), rules: Vec::new() };
    let wrote = f.findings(pending.violations.clone(), seen)?;
    Ok((f.rules, wrote))
}

/// The facts the `.references` setting gives `python/references.rr` (`Findings::references_setting`): none without
/// the setting.
pub const REFERENCE_SETTING: [&str; 5] = ["references_given", "references_include", "references_exclude", "references_sibling", "references_field"];

/// `re` and `by` rewritten so that `@rx_replace`'s every match (the regex crate's `replace_all`) is `Regex::replace`'s
/// one, the first: the regex is anchored at the start and takes the text before that match as group 1, the match as
/// group 2. `by`'s group references move along (`$n` and `${n}` to `${n+2}`, `$0` to `${2}`; names stay), and the
/// text before the match is put back first.
pub fn first_match(re: &str, by: &str) -> (String, String) {
    let mut out = String::from("${1}");
    let mut rest = by;
    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        if let Some(tail) = after.strip_prefix('$') {
            out.push_str("$$");
            rest = tail;
            continue;
        }
        let (name, tail) = if let Some(inner) = after.strip_prefix('{') {
            match inner.find('}') {
                Some(end) => (&inner[..end], &inner[end + 1..]),
                None => ("", after),
            }
        } else {
            let end = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(after.len());
            (&after[..end], &after[end..])
        };
        match name.parse::<usize>() {
            _ if name.is_empty() => out.push('$'),
            Ok(n) => out.push_str(&format!("${{{}}}", n + 2)),
            Err(_) => out.push_str(&format!("${{{name}}}")),
        }
        rest = tail;
    }
    out.push_str(rest);
    (format!("^(?s:(.*?))({re})"), out)
}

#[cfg(test)]
mod first_match_tests {
    /// Each rewritten pair replaces as W's `Regex::replace` does: the first match only, groups and all.
    #[test]
    fn a_rewritten_regex_replaces_the_first_match_only() {
        for (s, re, by) in [
            ("a_b_c", "_", "-"),
            ("foo_async", "(?:_async)?$", "_lazy"),
            ("foo", "$", "_lazy"),
            ("__x", "^_+", ""),
            ("ab12cd34", "([a-z]+)([0-9]+)", "$2$1"),
            ("ab12", "(?P<w>[a-z]+)", "<${w}>"),
            ("ab12", "[a-z]+", "[$0]$$"),
            ("none", "z", "y"),
        ] {
            let want = regex::Regex::new(re).unwrap().replace(s, by).into_owned();
            let (re2, by2) = super::first_match(re, by);
            let got = regex::Regex::new(&re2).unwrap().replace_all(s, by2.as_str()).into_owned();
            assert_eq!(got, want, "{s:?} {re:?} {by:?} as {re2:?} {by2:?}");
        }
    }
}
