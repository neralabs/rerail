//! Lossless public findings, lowered through the same relational planner as the rule. The existing
//! five-column `violation` output is an index, not the full diagnostic. Each `.violation` (and each error
//! directive) has its own relation of typed columns (decision 0249 §6, `diagnostic_name`): a row describes itself
//! (policy, clause, code, path, line, at, message, its tag and each parameter's name beside its value), so tabs,
//! newlines, numeric parameters, tags and distinct witnesses are kept as values, and no reader parses a text.
use super::*;
use crate::code_rule_dsl::logic::program_image::fields::{FieldsMut, ProgramFields};

/// The relations' name's start: `rr_diagnostic_<digest>`, one relation for each finding and form (a node's or a
/// place's).
pub const PREFIX: &str = "rr_diagnostic_";
pub const COMMIT_PREFIX: &str = "rr_diagnostic_commit_";

#[derive(Clone, Copy, PartialEq)]
enum Location { Document, File, Commit }

/// The columns before a finding's parameters: its policy, clause, code, path, line, place (`at`), message, and
/// whether it has a tag (`tagged`, then `tag`).
pub const HEAD: [&str; 8] = ["policy", "clause", "code", "path", "line", "at", "message", "tagged"];

/// The start of a hole column's name (`hole0`, …): a message placeholder's value, last in a finding's relation. The
/// writer composes `message` from them and the template and writes the row without them (decision 0225 §3).
pub const HOLE: &str = "hole";

/// The index of `message` in a finding's row (`HEAD`).
pub const MESSAGE_COLUMN: usize = 6;

/// Decision 0431 R9: the clauses whose findings are placed at a node no file holds (`corpus::refuse_unplaced`).
pub const UNPLACED: &str = "rr_unplaced";
/// The tape values' nodes a string of a file places (`guest_place` at a string whose document is a file's): a node of
/// a string inside another value stands in that value's text, which is no file's (decision 0431 §3, R9).
pub const PLACED_AT: &str = "rr_placed_at";

/// Whether `rel` is a finding's relation.
pub fn is_diagnostic(rel: &str) -> bool {
    rel.starts_with(PREFIX)
}

/// A finding's relation, by its policy, clause and form: the same in every program of a run, and another for each
/// policy, so a pack's programs share them as they share `violation` (`corpus::own_names`).
fn diagnostic_name(policy: &str, clause: &str, location: Location) -> String {
    use sha2::Digest as _;
    let mut h = sha2::Sha256::new();
    h.update(policy.as_bytes());
    h.update([0]);
    h.update(clause.as_bytes());
    h.update([0, location as u8]);
    let d: [u8; 32] = h.finalize().into();
    let prefix = if location == Location::Commit { COMMIT_PREFIX } else { PREFIX };
    format!("{prefix}{}", d[..8].iter().map(|b| format!("{b:02x}")).collect::<String>())
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
    while let Some(start) = rest.find(['{', '}']) {
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

/// A finding column's type as its relation's column holds it: a text, a node, or the column's own.
fn column_type(ty: &str) -> &str {
    match ty {
        "symbol" | "sym" => "sym",
        "node" | "span" | "loc" => "node",
        "number" | "int" => "int",
        other => other,
    }
}

/// The findings of a program's `.violation`s and `.on … report` settings (`Findings::findings`), written onto `b`: its
/// fields, and its rules term by term (`Builder::open_rule`, decision 0276), after the rules it holds.
pub(super) struct Findings<'a, 'p> {
    b: &'a mut crate::code_rule_dsl::logic::program_image::Builder<'p>,
    next: usize,
    made: Vec<(String, usize)>,
}

type Open = crate::code_rule_dsl::logic::program_image::Open;

/// The `.on syntax_error report CODE [at line 1]` and `.on input_error report CODE` settings (`core.rerail`): what, its
/// code, and whether at line 1.
fn on_reports(settings: &[(String, String)]) -> Vec<(String, String, bool)> {
    settings
        .iter()
        .filter(|(k, _)| k == "on")
        .filter_map(|(_, text)| {
            let words: Vec<&str> = text.split_whitespace().collect();
            match words.as_slice() {
                [what @ ("syntax_error" | "input_error"), "report", code, rest @ ..] => Some(((*what).to_owned(), (*code).to_owned(), rest == ["at", "line", "1"])),
                _ => None,
            }
        })
        .collect()
}

/// Decision 0055's settings (`Findings::guest_policy`): the `guest_policy` facts they state, the guest errors they
/// report (`(grammar, code)`), and whether one is a warning. `.guests` and `.scope embedded` are refused (decision
/// 0523): `.embedded` is the one setting.
fn guest_settings(settings: &[(String, String)]) -> Result<(Vec<(String, String)>, Vec<(Option<String>, String)>, bool), Error> {
    let mut facts = Vec::new();
    let mut findings = Vec::new();
    let mut warn = false;
    for (key, text) in settings {
        let words: Vec<&str> = text.split_whitespace().collect();
        if key == "guests" || (key == "scope" && words.first() == Some(&"embedded")) {
            return Err(Error::Syntax(format!(
                ".{key} {text}: retired (decision 0523): a run reads other languages' files for their programs only under `.embedded on`, and a rule reaches a string's code through `parsed`"
            )));
        }
        if key == "scope" {
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
    Ok((facts, findings, warn))
}

/// An entry of a `.scope` glob from a policy option (`front::OPTION_GLOB`): its form and the option's name.
fn option_entry(g: &str) -> Option<(String, String)> {
    g.strip_prefix(crate::code_rule_dsl::logic::front::OPTION_GLOB).and_then(|r| r.split_once(':')).map(|(f, n)| (f.to_owned(), n.to_owned()))
}

/// What a message's placeholder reads beside the finding's body (`Findings::diagnostic_rule`): a column's line, its
/// `name` field's text, a relation of it, or its text, into a fresh variable (and the field's node).
enum Reads {
    Nothing,
    Line(usize, String),
    Name(usize, String, String),
    Relation(String, usize, String),
    Text(usize, String),
}

impl<'p> Findings<'_, 'p> {
    /// A new name, none the program declares (`Lowerer::fresh`).
    fn fresh(&mut self, stem: &str) -> String {
        let b = &*self.b;
        super::fresh_name(&mut self.next, &mut self.made, |n| b.declares(n), stem)
    }

    /// Variable `name`, a builder term.
    fn v(&mut self, name: &str) -> Tm {
        Tm::VarAtom(self.b.atom(name))
    }

    /// Text `text`, a builder term.
    fn s(&mut self, text: &str) -> Tm {
        Tm::StrAtom(self.b.atom(text))
    }

    /// `f(args)`, a builder term.
    fn call(&mut self, f: &str, args: Vec<Tm>) -> Tm {
        Tm::CallAtom(self.b.atom(f), args)
    }

    /// `value`'s text.
    fn number(&mut self, value: Tm) -> Tm {
        self.call("to_string", vec![value])
    }

    /// Opens a rule of head `rel(args)`.
    fn open(&mut self, rel: &str, args: &[Tm]) -> Open {
        let mut o = self.b.open_rule();
        self.b.put_head(&mut o, rel, args);
        o
    }

    /// Atom `rel(args)` (negated: `neg`) of open rule `o`.
    fn put(&mut self, o: &mut Open, rel: &str, args: &[Tm], neg: bool) {
        self.b.put_atom(o, rel, args, neg, false);
    }

    /// `l op r` in open rule `o`.
    fn put_cmp(&mut self, o: &mut Open, l: Tm, op: &str, r: Tm) {
        let op = crate::code_rule_dsl::logic::program_image::CmpOp::of(op).expect("a comparison");
        self.b.put_cmp(o, l, op, r);
    }

    /// Ends open rule `o`.
    fn close(&mut self, o: Open) -> Result<(), Error> {
        self.b.close_rule(o).map(drop).map_err(Error::Syntax)
    }

    /// The test of a finding's path `$rr_p` against the `.scope` settings, in open rule `o`, where `scoped`
    /// (`scope_filter`): `!rr_scope_drop(rr_p)`.
    fn scope(&mut self, o: &mut Open, scoped: bool) {
        if scoped {
            let p = self.v("rr_p");
            self.put(o, "rr_scope_drop", &[p], true);
        }
    }

    /// The findings of the `.violation`s: `violation(policy, V, code, path, line)` for each row of V, at its first
    /// column's line (the engine's findings relation, as the converter's programs give it). A node's document is
    /// `node`'s column; a span's is `docof` (`core.rerail`), and so is its line. A `@place(path, line)` row is read from
    /// V's place relation; a span or loc `at` that holds a `@place` value made elsewhere (`PLACE`) is found at its
    /// path and line. Whether it wrote them (then it wrote a rule at least).
    fn findings(&mut self, mut violations: Vec<Violation>, seen: RulesSeen) -> Result<bool, Error> {
        // `.on syntax_error report CODE [at line 1]` and `.on input_error report CODE` (`core.rerail`).
        let on = on_reports(&self.b.settings());
        let guest_errors = self.guest_policy()?;
        self.references_setting()?;
        if violations.is_empty() && on.is_empty() && guest_errors.is_empty() {
            return Ok(false);
        }
        let policy = self
            .b
            .settings()
            .into_iter()
            .find(|(k, _)| k == "policy")
            .and_then(|(_, text)| text.split('"').nth(1).map(str::to_owned))
            .unwrap_or_default();
        let scoped = self.scope_filter()?;
        // A finding is its rule, clause, code, path, line and column (the anchor's 0-based byte column): two findings
        // on one line are two rows. A finding with no column (a place, an error of a file or input) has column 0.
        self.b.fields().declare("violation", &[("rule", "sym"), ("clause", "sym"), ("code", "sym"), ("path", "sym"), ("line", "int"), ("col", "int")]);
        // A program that derives `violation` itself, in the older five columns, is read at column 0: the caller's
        // (`lowering_tail`'s findings step), for this sees no rules.
        // Where it is already (a printed program's `.output`), else last.
        self.b.fields().add_output("violation");
        // Whether a rule makes a `@place` value: only then can a span or loc `at` hold one.
        let place_values = seen.place_values;
        let (p, l, c, d, v0) = (self.v("rr_p"), self.v("rr_l"), self.v("rr_c"), self.v("rr_d"), self.v("rr_v0"));
        let w = || Tm::Wild;
        let inputs = self.b.input_names();
        for f in std::mem::take(&mut violations) {
            // A violation made of direct `@place` heads alone has no document's rows: its document reads stay out of
            // the plan.
            // Nor does one made of `@git_commit_location` heads alone (its rows are at a commit, in the relation
            // `commit_location_relation` names).
            let commit_locations = self.b.columns_of(&commit_location_relation(&f.name)).is_some();
            let at_document = (!f.places && !commit_locations) || seen.heads.contains(&f.name) || self.b.states_facts(&f.name) || inputs.contains(&f.name);
            // A finding at a tape value's node stands at each string that holds it, in that string's file (decision
            // 0431 §3: "a finding at s by n, placed through s"): C408's `dict()` inside `a: "dict({})"`.
            let through = at_document && f.node && seen.reads_guests;
            if through {
                self.placed_at()?;
            }
            self.diagnostics(&policy, &f, at_document, commit_locations, through)?;
            let code_col = match f.code {
                FindingCode::Column(i) => Some(i),
                _ => None,
            };
            let code = match &f.code {
                FindingCode::Name => self.s(&f.name),
                FindingCode::Column(i) => self.v(&format!("rr_v{i}")),
                FindingCode::Constant(c) => self.s(c),
            };
            let head = vec![self.s(&policy), self.s(&f.name), code, p.clone(), l.clone(), c.clone()];
            let cols: Vec<Tm> = (0..f.arity).map(|i| if i == 0 || Some(i) == code_col { self.v(&format!("rr_v{i}")) } else { Tm::Wild }).collect();
            if at_document {
                let mut o = self.open("violation", &head);
                self.put(&mut o, &f.name, &cols, false);
                if f.node {
                    self.put(&mut o, "node", &[d.clone(), v0.clone(), w(), w(), w()], false);
                } else {
                    self.put(&mut o, "docof", &[v0.clone(), d.clone()], false);
                }
                self.put(&mut o, "doc", &[d.clone(), p.clone(), w()], false);
                self.put(&mut o, "line", &[v0.clone(), l.clone()], false);
                self.put(&mut o, "col", &[v0.clone(), c.clone()], false);
                self.scope(&mut o, scoped);
                self.close(o)?;
                if through {
                    let s = self.v("rr_s");
                    let mut o = self.open("violation", &head);
                    self.put(&mut o, &f.name, &cols, false);
                    self.put(&mut o, "guest_place", &[v0.clone(), s.clone(), l.clone(), c.clone()], false);
                    self.put(&mut o, "node", &[d.clone(), s, w(), w(), w()], false);
                    self.put(&mut o, "doc", &[d.clone(), p.clone(), w()], false);
                    self.scope(&mut o, scoped);
                    self.close(o)?;
                }
                // Decision 0431 R9: a finding at a node whose document is no file's (a tape value's), and that no
                // string places (a value no file's string holds), is refused by the run.
                self.b.fields().declare_absent(UNPLACED, &[("clause", "sym")]);
                self.b.fields().add_output(UNPLACED);
                let clause = self.s(&f.name);
                let mut o = self.open(UNPLACED, &[clause]);
                self.put(&mut o, &f.name, &cols, false);
                if f.node {
                    self.put(&mut o, "node", &[d.clone(), v0.clone(), w(), w(), w()], false);
                } else {
                    self.put(&mut o, "docof", &[v0.clone(), d.clone()], false);
                }
                self.put(&mut o, "doc", &[d.clone(), w(), w()], true);
                if through {
                    // The node's document is a tape value's (`tape_of`, an entry: hiding leaves `d` unbounded, so a
                    // value in the hidden range is refused too), and no file's string places it.
                    self.put(&mut o, "tape_of", &[d.clone(), w()], false);
                    self.put(&mut o, PLACED_AT, std::slice::from_ref(&v0), true);
                }
                self.close(o)?;
            }
            if at_document && !f.node && place_values {
                let path = self.call(PLACE_PATH, vec![v0.clone()]);
                let line = self.call(PLACE_LINE, vec![v0.clone()]);
                let mut found = head.clone();
                found[3] = path.clone();
                found[4] = line;
                found[5] = Tm::Int(0);
                let mut o = self.open("violation", &found);
                self.put(&mut o, &f.name, &cols, false);
                self.put_cmp(&mut o, v0.clone(), "<", Tm::Int(UNSET));
                // The scope tests the place's path, bound for it here.
                if scoped {
                    self.put_cmp(&mut o, p.clone(), "=", path);
                    self.scope(&mut o, true);
                }
                self.close(o)?;
            }
            if f.places {
                let mut args = vec![p.clone(), l.clone()];
                args.extend(cols.into_iter().skip(1));
                let mut head = head;
                head[5] = Tm::Int(0);
                let mut o = self.open("violation", &head);
                self.put(&mut o, &place_relation(&f.name), &args, false);
                self.scope(&mut o, scoped);
                self.close(o)?;
            }
        }
        // A syntax error at its line (or line 1), in its document; an input error at line 1 of the input's path.
        for (what, code, line_one) in on {
            let head = vec![self.s(&policy), self.s(&what), self.s(&code), p.clone(), l.clone(), Tm::Int(0)];
            for rel in [what.as_str(), "doc"] {
                if self.b.declares(rel) {
                    self.b.fields().add_input(rel);
                }
            }
            let syntax = what == "syntax_error";
            let body = |me: &mut Self, o: &mut Open| {
                let (p, l, d) = (me.v("rr_p"), me.v("rr_l"), me.v("rr_d"));
                if syntax {
                    let line = if line_one { Tm::Wild } else { l.clone() };
                    me.put(o, "syntax_error", &[d.clone(), line, Tm::Wild], false);
                    me.put(o, "doc", &[d.clone(), p, Tm::Wild], false);
                    // A file's own error: a program's is `.on guest_error`'s.
                    me.put(o, "rr_program", &[d, Tm::Wild, Tm::Wild], true);
                    me.scope(o, scoped);
                    if line_one {
                        me.put_cmp(o, l, "=", Tm::Int(1));
                    }
                } else {
                    me.put(o, "input_error", &[p, Tm::Wild, Tm::Wild], false);
                    me.put_cmp(o, l, "=", Tm::Int(1));
                }
            };
            self.error_diagnostic(&policy, &what, &code, &body)?;
            if syntax {
                self.guest_input();
            }
            let mut o = self.open("violation", &head);
            body(self, &mut o);
            self.close(o)?;
        }
        // A guest that does not parse, reported at its host's line (`syntax_error` of a guest is a file line).
        for (grammar, code) in guest_errors {
            let head = vec![self.s(&policy), self.s("guest_error"), self.s(&code), p.clone(), l.clone(), Tm::Int(0)];
            for (rel, cols) in [
                ("syntax_error", &[("d", "int"), ("l", "int"), ("msg", "sym")][..]),
                ("doc", &[("d", "int"), ("path", "sym"), ("module", "sym")][..]),
            ] {
                self.b.fields().declare_absent(rel, cols);
                self.b.fields().add_input(rel);
            }
            self.guest_input();
            let body = |me: &mut Self, o: &mut Open| {
                let (p, l, d) = (me.v("rr_p"), me.v("rr_l"), me.v("rr_d"));
                let g = grammar.as_deref().map_or(Tm::Wild, |g| me.s(g));
                me.put(o, "syntax_error", &[d.clone(), l, Tm::Wild], false);
                me.put(o, "rr_program", &[d.clone(), Tm::Wild, g], false);
                me.put(o, "doc", &[d, p, Tm::Wild], false);
            };
            self.error_diagnostic(&policy, "guest_error", &code, &body)?;
            let mut o = self.open("violation", &head);
            body(self, &mut o);
            self.close(o)?;
            // A string a rule parses is a tape value (decision 0431): no file's, its lines its own text's. It is
            // reported at each string holding its text, at the string's line in the string's file. Where the
            // program has tape values (`tape_of`, `sub_tape`).
            if ["tape_of", "sub_tape"].iter().all(|r| self.b.declares(r)) {
                for rel in ["tape_of", "sub_tape"] {
                    self.b.fields().add_input(rel);
                }
                let at_string = |me: &mut Self, o: &mut Open| {
                    let (p, l, d) = (me.v("rr_p"), me.v("rr_l"), me.v("rr_d"));
                    let (t, s, h) = (me.v("rr_t"), me.v("rr_s"), me.v("rr_h"));
                    let g = grammar.as_deref().map_or(Tm::Wild, |g| me.s(g));
                    me.put(o, "syntax_error", &[d.clone(), Tm::Wild, Tm::Wild], false);
                    me.put(o, "tape_of", &[d, t.clone()], false);
                    me.put(o, "sub_tape", &[s.clone(), g, t], false);
                    me.put(o, "docof", &[s.clone(), h.clone()], false);
                    me.put(o, "doc", &[h, p, Tm::Wild], false);
                    me.put(o, "line", &[s, l], false);
                };
                self.error_diagnostic(&policy, "guest_error", &code, &at_string)?;
                let mut o = self.open("violation", &head);
                at_string(self, &mut o);
                self.close(o)?;
            }
        }
        Ok(true)
    }

    /// The `.scope` settings as a test of a finding's path `$rr_p` (`core.rerail`): a document's path is in scope in one
    /// of `dirs` (each a directory or a glob of one, at the root) or `include`'s globs when there are any, in none
    /// of `exclude`'s, and with `tests false` under no `test` or `tests` path part. Findings in other documents are
    /// dropped; one at a path that is no document (a budget file) is kept. Facts still come from every file. A
    /// `.scope given` program reads the files it is given, all in scope. Whether there is a test (`scope`).
    fn scope_filter(&mut self) -> Result<bool, Error> {
        let settings = self.b.settings();
        let report = scope_report_globs(&settings);
        let given = settings.iter().any(|(k, t)| k == "scope" && t.trim() == "given");
        let (within, out) = match scope_globs(&settings) {
            Some(globs) => globs,
            None if !report.is_empty() && !given => (Vec::new(), Vec::new()),
            None => return Ok(false),
        };
        // The corpus reads only these documents (`Field::DocScopes`): their facts are the policy's, as the
        // scripts' discovery has them; the drop below still holds for a combined run that reads more.
        if !within.is_empty() || !out.is_empty() {
            self.b.fields().set_field(crate::code_rule_dsl::logic::program_facts::Field::DocScopes(Some(&[(within.clone(), out.clone())])));
        }
        if within.iter().chain(&out).chain(&report).any(|g| option_entry(g).is_some()) {
            self.b.fields().declare_absent("option_item", &[("name", "sym"), ("i", "int"), ("v", "sym")]);
            self.b.fields().add_input("option_item");
        }
        self.b.fields().declare_absent("doc", &[("d", "int"), ("path", "sym"), ("module", "sym")]);
        self.b.fields().add_input("doc");
        // rr_scope_drop(p): a document's path out of the scope.
        self.b.fields().declare("rr_scope_drop", &[("p", "sym")]);
        let (d, p) = (self.v("d"), self.v("p"));
        if !within.is_empty() {
            self.b.fields().declare("rr_scope_in", &[("d", "int")]);
            for g in &within {
                let mut o = self.open("rr_scope_in", std::slice::from_ref(&d));
                self.scope_doc(&mut o);
                self.scope_test(&mut o, g);
                self.close(o)?;
            }
            let mut o = self.open("rr_scope_drop", std::slice::from_ref(&p));
            self.scope_doc(&mut o);
            self.put(&mut o, "rr_scope_in", std::slice::from_ref(&d), true);
            self.close(o)?;
        }
        for g in &out {
            let mut o = self.open("rr_scope_drop", std::slice::from_ref(&p));
            self.scope_doc(&mut o);
            self.scope_test(&mut o, g);
            self.close(o)?;
        }
        // rr_scope_not(d): a document out of the scope, which a combined run's guards negate (decision 0091): in a
        // pack that reads more documents than this policy's scope, the policy's rules see only the others' rows.
        if !within.is_empty() || !out.is_empty() {
            self.b.fields().declare(SCOPE_NOT, &[("d", "int")]);
            if !within.is_empty() {
                let mut o = self.open(SCOPE_NOT, std::slice::from_ref(&d));
                self.scope_doc(&mut o);
                self.put(&mut o, "rr_scope_in", std::slice::from_ref(&d), true);
                self.close(o)?;
            }
            for g in &out {
                let mut o = self.open(SCOPE_NOT, std::slice::from_ref(&d));
                self.scope_doc(&mut o);
                self.scope_test(&mut o, g);
                self.close(o)?;
            }
        }
        // `.scope report dirs`: a finding in a document under none of them is dropped too.
        if !report.is_empty() {
            self.b.fields().declare("rr_report_in", &[("d", "int")]);
            for g in &report {
                let mut o = self.open("rr_report_in", std::slice::from_ref(&d));
                self.scope_doc(&mut o);
                self.scope_glob(&mut o, g);
                self.close(o)?;
            }
            let mut o = self.open("rr_scope_drop", std::slice::from_ref(&p));
            self.scope_doc(&mut o);
            self.put(&mut o, "rr_report_in", std::slice::from_ref(&d), true);
            self.close(o)?;
        }
        Ok(true)
    }

    /// `doc(d, p, _)` in open rule `o`: a document `d` at path `p` (`scope_filter`).
    fn scope_doc(&mut self, o: &mut Open) {
        let (d, p) = (self.v("d"), self.v("p"));
        self.put(o, "doc", &[d, p, Tm::Wild], false);
    }

    /// Path `p` matches glob `g`, in open rule `o`.
    fn scope_glob(&mut self, o: &mut Open, g: &str) {
        let (p, g) = (self.v("p"), self.s(g));
        let glob = self.call("glob", vec![p, g]);
        self.put_cmp(o, glob, "=", Tm::Int(1));
    }

    /// Path `p` in scope entry `g`, in open rule `o`: its glob, or an entry from a policy option (`option_entry`): the
    /// option's values, as `option_item` holds them; a directory's documents are those under it.
    fn scope_test(&mut self, o: &mut Open, g: &str) {
        let Some((form, name)) = option_entry(g) else { return self.scope_glob(o, g) };
        let (p, item, name) = (self.v("p"), self.v("rr_o"), self.s(&name));
        self.put(o, "option_item", &[name, Tm::Wild, item.clone()], false);
        let test = if form == "dirs" {
            let slash = self.s("/");
            let under = self.call("concat", vec![item, slash]);
            self.call("prefix", vec![p, under])
        } else {
            self.call("glob", vec![p, item])
        };
        self.put_cmp(o, test, "=", Tm::Int(1));
    }

    /// `rr_program(d, file, grammar)` as an input: the document underneath a file's program (decision 0431 §4), which
    /// no library a rule reads declares (a rule names a program by its file and place).
    fn guest_input(&mut self) {
        self.b.fields().declare_absent("rr_program", &[("d", "int"), ("file", "int"), ("grammar", "sym")]);
        self.b.fields().add_input("rr_program");
    }

    /// The `.references` setting (W's: `siblingLinks`, `constructorFields`, `scope`) as the facts
    /// `python/references.rerail` reads (decision 0483 R5), in a program that includes it: `references_given(1)`, the
    /// scope's globs (`**` when it names none; none when it includes nothing), each sibling link, and the constructor
    /// fields' renaming. Each renaming is W's `Regex::replace`, the first match only: it is given as a regex whose
    /// every match is that one (`first_match`), for `@rx_replace`. No setting, no facts.
    fn references_setting(&mut self) -> Result<(), Error> {
        let settings: Vec<String> = self.b.settings().into_iter().filter(|(k, _)| k == "references").map(|(_, v)| v).collect();
        if !self.b.declares(REFERENCE_SETTING[0]) {
            return Ok(());
        }
        // No setting: the relations are stated, empty. They are the run's constants either way, so the rules
        // reading them stay file-local (t50 step 1); a relation with no stated facts has no location to the split.
        if settings.is_empty() {
            for rel in REFERENCE_SETTING {
                if self.b.declares(rel) && self.b.fact_rows(rel).is_empty() {
                    self.b.fields().set_facts(rel, &[]);
                }
            }
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
            self.b.fields().add_facts(rel, &[crate::code_rule_dsl::logic::front::List::from(row)]);
        }
        // A part the setting leaves out (no `siblingLinks`, no `constructorFields`) is stated, empty.
        for rel in REFERENCE_SETTING {
            if self.b.declares(rel) && self.b.fact_rows(rel).is_empty() {
                self.b.fields().set_facts(rel, &[]);
            }
        }
        Ok(())
    }

    /// Decision 0055's settings as `guest_policy` facts, which the corpus runner and the batch read:
    /// - `.guests on | off` and `.scope embedded true | false` are refused (decision 0523): `.embedded` says what a
    ///   run reads of other languages' files, and a program shows files and the run language's code guests always;
    /// - `.on guest_error [GRAMMAR] skip | partial [warn | report CODE]`, or `.on guest_error [GRAMMAR] warn | report
    ///   CODE` (skip): `guest_policy("errors" | "errors:GRAMMAR", "skip" | "partial")`. A skipped guest is
    ///   withheld; a partial one keeps the tree it has. `warn` and `report` both give a finding, `warn`'s of severity
    ///   warning (`.severity guest_error warning`); the findings are returned, `(grammar, code)`, for `findings`.
    fn guest_policy(&mut self) -> Result<Vec<(Option<String>, String)>, Error> {
        let settings = self.b.settings();
        let (facts, findings, warn) = guest_settings(&settings)?;
        if warn {
            let mut all = settings.clone();
            all.push(("severity".to_owned(), "guest_error warning".to_owned()));
            self.b.fields().set_field(crate::code_rule_dsl::logic::program_facts::Field::Settings(&all));
        }
        if !facts.is_empty() {
            self.b.fields().declare(GUEST_POLICY, &[("k", "sym"), ("v", "sym")]);
            let rows: Vec<crate::code_rule_dsl::logic::front::List<Value>> = facts.into_iter().map(|(k, v)| [Value::Str(k.into()), Value::Str(v.into())].into()).collect();
            self.b.fields().add_facts(GUEST_POLICY, &rows);
        }
        Ok(findings)
    }

    /// Error directives use the same public diagnostic transport as ordinary violations: `body` writes the error's
    /// literals.
    fn error_diagnostic(&mut self, policy: &str, clause: &str, code: &str, body: &dyn Fn(&mut Self, &mut Open)) -> Result<(), Error> {
        let finding = Violation {
            name: clause.to_owned(), arity: 1, code: FindingCode::Constant(code.to_owned()), node: false,
            places: true, columns: vec![("at".to_owned(), "span".to_owned())], message: String::new(), tag: None,
        };
        self.diagnostic_rule(policy, &finding, &[Tm::Int(0)], Tm::Int(0), Location::File, body)
    }

    fn diagnostics(&mut self, policy: &str, finding: &Violation, at_document: bool, commit_locations: bool, through: bool) -> Result<(), Error> {
        let cols: Vec<Tm> = (0..finding.arity).map(|i| self.v(&format!("rr_v{i}"))).collect();
        let at = cols[0].clone();
        if at_document {
            self.diagnostic_rule(policy, finding, &cols, at.clone(), Location::Document, &|me, o| {
                let (d, p, l) = (me.v("rr_d"), me.v("rr_p"), me.v("rr_l"));
                me.put(o, &finding.name, &cols, false);
                if finding.node {
                    me.put(o, "node", &[d.clone(), cols[0].clone(), Tm::Wild, Tm::Wild, Tm::Wild], false);
                } else {
                    me.put(o, "docof", &[cols[0].clone(), d.clone()], false);
                }
                me.put(o, "doc", &[d, p, Tm::Wild], false);
                me.put(o, "line", &[cols[0].clone(), l], false);
            })?;
        }
        // At a tape value's node: at each file's string that holds it, at the node's line there (`guest_place`).
        if through {
            self.diagnostic_rule(policy, finding, &cols, at, Location::Document, &|me, o| {
                let (d, p, l, s) = (me.v("rr_d"), me.v("rr_p"), me.v("rr_l"), me.v("rr_s"));
                me.put(o, &finding.name, &cols, false);
                me.put(o, "guest_place", &[cols[0].clone(), s.clone(), l, Tm::Wild], false);
                me.put(o, "node", &[d.clone(), s, Tm::Wild, Tm::Wild, Tm::Wild], false);
                me.put(o, "doc", &[d, p, Tm::Wild], false);
            })?;
        }
        if finding.places {
            let mut args = vec![self.v("rr_p"), self.v("rr_l")];
            args.extend(cols.iter().skip(1).cloned());
            let place = place_relation(&finding.name);
            self.diagnostic_rule(policy, finding, &cols, Tm::Int(0), Location::File, &|me, o| me.put(o, &place, &args, false))?;
        }
        // `V(@git_commit_location(repo, commit[, field]), …)` rows: at a commit, no file's line.
        if commit_locations {
            let mut args = vec![self.v("rr_repo"), self.v("rr_commit"), self.v("rr_has_field"), self.v("rr_field")];
            args.extend(cols.iter().skip(1).cloned());
            let commit = commit_location_relation(&finding.name);
            self.diagnostic_rule(policy, finding, &cols, Tm::Int(0), Location::Commit, &|me, o| me.put(o, &commit, &args, false))?;
        }
        Ok(())
    }

    /// `PLACED_AT`'s rule, once a program: a tape value's node `n`, placed by `guest_place` at a string `s` whose document
    /// is a file's.
    fn placed_at(&mut self) -> Result<(), Error> {
        if self.b.declares(PLACED_AT) {
            return Ok(());
        }
        self.b.fields().declare(PLACED_AT, &[("n", "node")]);
        let (n, s, d) = (self.v("rr_n"), self.v("rr_s"), self.v("rr_d"));
        let mut o = self.open(PLACED_AT, std::slice::from_ref(&n));
        self.put(&mut o, "guest_place", &[n, s.clone(), Tm::Wild, Tm::Wild], false);
        self.put(&mut o, "node", &[d.clone(), s, Tm::Wild, Tm::Wild, Tm::Wild], false);
        self.put(&mut o, "doc", &[d, Tm::Wild, Tm::Wild], false);
        self.close(o)
    }

    /// The rule of `finding`'s relation (`diagnostic_name`): its row of the finding's columns `cols`, at `at` (a place's
    /// at a file or a commit, `location`), its body `body`'s literals, then what its message's placeholders read
    /// (`Reads`).
    fn diagnostic_rule(&mut self, policy: &str, finding: &Violation, cols: &[Tm], at: Tm, location: Location, body: &dyn Fn(&mut Self, &mut Open)) -> Result<(), Error> {
        let place = location != Location::Document;
        let column = |key: &str| finding.columns.iter().position(|(name, _)| name == key)
            .ok_or_else(|| Error::Syntax(format!(".violation {} message: no column {key}", finding.name)));
        let string_column = |me: &mut Self, i: usize| if matches!(finding.columns[i].1.as_str(), "symbol" | "sym") {
            cols[i].clone()
        } else { me.number(if i == 0 && place { at.clone() } else { cols[i].clone() }) };
        // The message's placeholders' values, in order; its texts are not the program's (decision 0225 §3): the
        // writer puts them in from the template (`message_of`), so a message's edit is no edit of the rules.
        let (_, keys) = message_parts(&finding.message).map_err(|e| Error::Syntax(format!(".violation {} message: {e}", finding.name)))?;
        let mut holes = Vec::new();
        let mut reads = Vec::new();
        for key in &keys {
            let value = self.fresh("message");
            let (term, read) = if let Some((node, property)) = key.split_once('.') {
                let i = column(node)?;
                match property {
                    "line" if i == 0 && location == Location::Commit => return Err(Error::Syntax(format!(".violation {}: a commit location has no source line", finding.name))),
                    "line" if i == 0 => {
                        let l = self.v("rr_l");
                        (self.number(l), Reads::Nothing)
                    }
                    "line" => {
                        let x = self.v(&value);
                        (self.number(x), Reads::Line(i, value))
                    }
                    "name" => {
                        if i == 0 && place { return Err(Error::Syntax(format!(".violation {} message: a place has no name", finding.name))); }
                        let child = self.fresh("message_name");
                        (self.v(&value), Reads::Name(i, child, value))
                    }
                    _ => return Err(Error::Syntax(format!(".violation {} message: unsupported property {key}", finding.name))),
                }
            } else if let Some((relation, argument)) = key.strip_suffix(')').and_then(|key| key.split_once('(')) {
                let i = column(argument)?;
                if !self.b.declares(relation) {
                    return Err(Error::Syntax(format!(".violation {} message: no relation {relation}", finding.name)));
                }
                (self.v(&value), Reads::Relation(relation.to_owned(), i, value))
            } else {
                let i = column(key)?;
                if matches!(finding.columns[i].1.as_str(), "node" | "span" | "loc") {
                    if i == 0 && place { (self.s(""), Reads::Nothing) } else { (self.v(&value), Reads::Text(i, value)) }
                } else { (string_column(self, i), Reads::Nothing) }
            };
            holes.push(term);
            reads.push(read);
        }
        // The template beside the program, for `rr::print` and the writer (`MESSAGE`).
        let setting = (MESSAGE.to_owned(), format!("{} {}", finding.name, rerail_rr_syntax::syntax::quote(&finding.message)));
        if !finding.message.is_empty() {
            let mut all = self.b.settings();
            if !all.contains(&setting) {
                all.push(setting);
                self.b.fields().set_field(crate::code_rule_dsl::logic::program_facts::Field::Settings(&all));
            }
        }
        // A code from a column of another type than text: its text, as a code is.
        let code = match &finding.code {
            FindingCode::Name => self.s(&finding.name),
            FindingCode::Constant(code) => self.s(code),
            FindingCode::Column(i) if matches!(finding.columns[*i].1.as_str(), "symbol" | "sym") => cols[*i].clone(),
            FindingCode::Column(i) => self.number(cols[*i].clone()),
        };
        // The row: the head (`HEAD`), the tag when there is one, each parameter's name beside its value, then the
        // message's placeholders' values (`HOLE`), a text each. The rule leaves `message` empty: the writer makes it
        // from the template and the holes, and writes the row without them (`compose_message`, decision 0225 §3).
        let at_type = if place { "int" } else { column_type(&finding.columns[0].1) };
        let mut decl: Vec<(String, String)> = HEAD.iter().zip(["sym", "sym", "sym", "sym", "int", at_type, "sym", "int"]).map(|(c, t)| ((*c).to_owned(), t.to_owned())).collect();
        let (path, line) = if location == Location::Commit { (self.s(""), Tm::Int(0)) } else { (self.v("rr_p"), self.v("rr_l")) };
        let mut args = vec![self.s(policy), self.s(&finding.name), code, path, line, at, self.s(""), Tm::Int(i64::from(finding.tag.is_some()))];
        if let Some(i) = finding.tag {
            decl.push(("tag".to_owned(), column_type(&finding.columns[i].1).to_owned()));
            args.push(cols[i].clone());
        }
        if location == Location::Commit {
            for (column, ty, var) in [("repository","sym","rr_repo"), ("commit","sym","rr_commit"), ("has_field","int","rr_has_field"), ("field","sym","rr_field")] {
                decl.push((column.to_owned(), ty.to_owned()));
                args.push(self.v(var));
            }
        }
        for (i, (name, ty)) in finding.columns.iter().enumerate().skip(1) {
            decl.push((format!("name{i}"), "sym".to_owned()));
            decl.push((format!("value{i}"), column_type(ty).to_owned()));
            args.push(self.s(name));
            args.push(cols[i].clone());
        }
        for (j, hole) in holes.into_iter().enumerate() {
            decl.push((format!("{HOLE}{j}"), "sym".to_owned()));
            args.push(hole);
        }
        let rel = diagnostic_name(policy, &finding.name, location);
        let pairs: Vec<(&str, &str)> = decl.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
        self.b.fields().declare(&rel, &pairs);
        self.b.fields().add_output(&rel);
        let mut o = self.open(&rel, &args);
        body(self, &mut o);
        for read in reads {
            match read {
                Reads::Nothing => {}
                Reads::Line(i, value) => {
                    let x = self.v(&value);
                    self.put(&mut o, "line", &[cols[i].clone(), x], false);
                }
                Reads::Name(i, child, value) => {
                    let (name, child, x) = (self.s("name"), self.v(&child), self.v(&value));
                    self.put(&mut o, "field", &[cols[i].clone(), name, child.clone()], false);
                    self.put(&mut o, "text", &[child, x], false);
                }
                Reads::Relation(relation, i, value) => {
                    let x = self.v(&value);
                    self.put(&mut o, &relation, &[cols[i].clone(), x], false);
                }
                Reads::Text(i, value) => {
                    let x = self.v(&value);
                    self.put(&mut o, "text", &[cols[i].clone(), x], false);
                }
            }
        }
        self.close(o)
    }
}

/// Whether the findings (`Findings`) of `pending` over `prog`'s fields write rules: a `.violation`, an `.on … report`
/// setting or a guest error reported. Only then does the caller derive a version for them (`findings_on`).
pub fn findings_write(prog: &(impl crate::code_rule_dsl::logic::program_image::fields::ProgramFields + ?Sized), pending: &PendingFindings) -> bool {
    let settings = prog.settings();
    !pending.violations.is_empty() || !on_reports(&settings).is_empty() || guest_settings(&settings).map_or(true, |g| !g.1.is_empty())
}

/// The findings (`Findings`) of `pending` written onto `b`: its fields, and its rules after those it holds (where
/// `findings_write`). Whether they wrote the findings (then the caller widens the program's own `violation` heads of
/// five columns, `lowering_tail`). A `.scope tool_calls` policy's are lowered first (`tool_call_findings`, main's
/// #866).
pub fn findings_on(b: &mut crate::code_rule_dsl::logic::program_image::Builder<'_>, pending: &PendingFindings, seen: RulesSeen) -> Result<bool, Error> {
    let mut f = Findings { b, next: pending.next, made: Vec::new() };
    let files = f.tool_call_findings(pending.violations.clone(), &seen)?;
    // A tool-call policy's findings were written there (it leaves none in files).
    let at_calls = files.len() < pending.violations.len();
    Ok(f.findings(files, seen)? || at_calls)
}

/// A `.scope tool_calls` policy's findings (`docs/rule-syntax/tool-calls.md`): each at a tool call, its `at` the call's
/// id (`at: symbol`), with no path and line 0. `findings` lowers the rest, those in files.
impl Findings<'_, '_> {
    /// Refuses a policy that reads tool calls without `.scope tool_calls` (as `rules.rs` refuses a JSON rule that reads
    /// `input` and samples no tool call), and lowers a tool-call policy's findings. Gives the findings left, those in
    /// files.
    fn tool_call_findings(&mut self, violations: Vec<Violation>, seen: &RulesSeen) -> Result<Vec<Violation>, Error> {
        let settings = self.b.settings();
        let scoped = reads_tool_calls(&settings);
        refuse_misread_tool_calls(&settings, seen.reads_tool_call, &violations)?;
        if !scoped {
            return Ok(violations);
        }
        let policy = settings.iter().find(|(k, _)| k == "policy").and_then(|(_, text)| text.split('"').nth(1)).unwrap_or_default().to_owned();
        for f in &violations {
            self.call_finding(&policy, f)?;
        }
        Ok(Vec::new())
    }

    /// The finding of a `.violation` at a tool call: its row (`diagnostic_rule`) at no path and line 0, its `at` the
    /// call's id, and its `violation` row. The message names the call's id as `{at}`: a call has no line or name.
    fn call_finding(&mut self, policy: &str, f: &Violation) -> Result<(), Error> {
        let (_, keys) = message_parts(&f.message).map_err(|e| Error::Syntax(format!(".violation {} message: {e}", f.name)))?;
        if let Some(key) = keys.iter().find(|k| k.strip_prefix(f.columns[0].0.as_str()).is_some_and(|p| p.starts_with('.'))) {
            return Err(Error::Syntax(format!(".violation {} message: {{{key}}}: a tool call has only its id", f.name)));
        }
        let cols: Vec<Tm> = (0..f.arity).map(|i| self.v(&format!("rr_v{i}"))).collect();
        let at = cols[0].clone();
        let at_call = |me: &mut Self, o: &mut Open| {
            me.put(o, &f.name, &cols, false);
            me.put(o, TOOL_CALLS[0], &[cols[0].clone(), Tm::Wild], false);
        };
        self.diagnostic_rule(policy, f, &cols, at, Location::Document, &|me, o| {
            at_call(me, o);
            let (p, l, empty) = (me.v("rr_p"), me.v("rr_l"), me.s(""));
            me.put_cmp(o, p, "=", empty);
            me.put_cmp(o, l, "=", Tm::Int(0));
        })?;
        // The `violation(rule, clause, code, path, line, col)` index, an output, as `findings` declares it for those in
        // files.
        self.b.fields().declare("violation", &[("rule", "sym"), ("clause", "sym"), ("code", "sym"), ("path", "sym"), ("line", "int"), ("col", "int")]);
        self.b.fields().add_output("violation");
        let code = match &f.code {
            FindingCode::Name => self.s(&f.name),
            FindingCode::Column(i) => self.v(&format!("rr_v{i}")),
            FindingCode::Constant(c) => self.s(c),
        };
        let head = [self.s(policy), self.s(&f.name), code, self.s(""), Tm::Int(0), Tm::Int(0)];
        let mut o = self.open("violation", &head);
        at_call(self, &mut o);
        self.close(o)
    }
}

/// A tool call's relations read without `.scope tool_calls` (`read`, `RulesSeen::reads_tool_call`), another `.scope`
/// beside it (it reads no file), or a finding at a tool call (`at: symbol`) in one policy and not the other.
fn refuse_misread_tool_calls(settings: &[(String, String)], read: Option<&str>, violations: &[Violation]) -> Result<(), Error> {
    let scoped = reads_tool_calls(settings);
    let other_scope = settings.iter().find(|(k, t)| k == "scope" && t.trim() != "tool_calls");
    match (read, other_scope, violations.iter().find(|v| v.call() != scoped)) {
        (Some(rel), _, _) if !scoped => Err(Error::Syntax(format!("{rel} is a tool call's relation: a policy that reads tool calls says `.scope tool_calls`"))),
        (_, Some((_, other)), _) if scoped => Err(Error::Syntax(format!("`.scope tool_calls` reads no file: no `.scope {}` beside it", other.trim()))),
        (_, _, Some(v)) => Err(Error::Syntax(format!(".violation {}: {}", v.name, call_at(scoped)))),
        _ => Ok(()),
    }
}

/// Why a `.violation`'s `at` does not fit its policy: a tool call's in a `.scope tool_calls` policy, a node, span or loc
/// in any other.
fn call_at(scoped: bool) -> &'static str {
    if scoped {
        "a `.scope tool_calls` policy's findings are at a tool call, `at: symbol`"
    } else {
        "`at` is a tool call (a symbol) only in a `.scope tool_calls` policy"
    }
}

/// The facts the `.references` setting gives `python/references.rerail` (`Findings::references_setting`): none without
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
