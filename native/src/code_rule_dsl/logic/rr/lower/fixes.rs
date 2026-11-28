//! A `.violation`'s fixes, run (decision 0720). Each `fix [safe] F "title"` line names F, whose rules
//! `F(cols…) :- body => edits` say how a finding of V is fixed. Each edit of each rule becomes an output relation of
//! its own, `rr_fixedit_<digest>`, joined with V's rows: per finding the rule fixes, the finding's columns, the edit's
//! target and the values of its text's placeholders, after constant columns that say which fix, rule and edit it is and
//! the edit's text as written (`HEAD`). The host (`rr_host::fixes`) reads the rows back, fills each edit's text from
//! the file and composes a finding's fix as one replacement.
//!
//! An edit `replace(target, `text`)` is run, its text's placeholders `$x` each a variable of the rule. Anything else
//! is not run, and the rule then fixes nothing: `delete`, `insert_before` and `insert_after` (their separators and
//! lines are the applier's, not built yet), a library's `.edit` relation, a list placeholder (`$$$X`), and a
//! placeholder or target that no positive atom of the rule types. A fix whose rule does not lower is left out with it;
//! the violation's own rules are lowered as before.

use super::*;

/// The start of a fix edit's relation's name.
pub const PREFIX: &str = "rr_fixedit_";

/// The constant columns before a fix edit's values: the policy, the violation, the fix (its `fix` line's index among
/// the violation's), the rule (the index of F's rule), the edit (its index in the rule) and how many edits the rule
/// has, the edit's text as written, and the names of the violation's columns and of the text's placeholders, each
/// comma-joined. Then come the violation's columns, the target, and the placeholders' values.
pub const HEAD: [&str; 9] = ["policy", "violation", "fix", "rule", "edit", "edits", "template", "columns", "placeholders"];

/// Whether `rel` is a fix edit's relation, as written or as a pack's programs prefix their own (`p<i>_`).
pub fn is_fix_edit(rel: &str) -> bool {
    rel.starts_with(PREFIX) || rel.split_once('_').is_some_and(|(p, rest)| p.starts_with('p') && rest.starts_with(PREFIX))
}

/// A piece of an edit's text: text as written, or a placeholder's value.
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Text(String),
    Hole(String),
}

/// An edit's text as pieces: `$name` is a placeholder (letters, digits and `_`), any other `$` is text. `None` for a
/// list placeholder, `$$$X`, which is not run.
pub fn template_parts(template: &str) -> Option<Vec<Part>> {
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut rest = template;
    while let Some(at) = rest.find('$') {
        text.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        if after.starts_with('$') {
            return None;
        }
        let name: String = after.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        if name.is_empty() {
            text.push('$');
        } else {
            parts.push(Part::Text(std::mem::take(&mut text)));
            parts.push(Part::Hole(name.clone()));
        }
        rest = &after[name.len()..];
    }
    text.push_str(rest);
    parts.push(Part::Text(text));
    Some(parts.into_iter().filter(|part| *part != Part::Text(String::new())).collect())
}

/// Which edit of which fix a relation holds.
#[derive(Clone, Copy)]
struct EditAt {
    fix: usize,
    rule: usize,
    edit: usize,
    edits: usize,
}

/// A fix edit's relation, by its policy, violation, fix, rule and edit: another for each.
fn edit_relation(policy: &str, violation: &str, at: EditAt) -> String {
    use sha2::Digest as _;
    let mut h = sha2::Sha256::new();
    h.update(format!("{policy}\0{violation}\0{}\0{}\0{}", at.fix, at.rule, at.edit).as_bytes());
    let d: [u8; 32] = h.finalize().into();
    format!("{PREFIX}{}", d[..8].iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// `replace(target, text)`'s target variable and text as written: a backtick text, a quoted one with no `$`, or a
/// variable's value. `None` for any other edit.
fn replace_edit(term: &STerm) -> Option<(String, String)> {
    let STerm::Call { name, functor: false, args } = term else { return None };
    let [STerm::Var(target), text] = args.as_slice() else { return None };
    let text = match text {
        STerm::Tick(text) => text.clone(),
        STerm::Str(text) if !text.contains('$') => text.clone(),
        STerm::Var(v) => format!("${v}"),
        _ => return None,
    };
    (name == "replace").then(|| (target.clone(), text))
}

/// The `.policy` id the module's settings state.
fn policy_id(settings: &[(String, String)]) -> Option<String> {
    settings.iter().find(|(key, _)| key == "policy").and_then(|(_, text)| text.split('"').nth(1)).map(str::to_owned)
}

/// The `fix V [safe] F "title"` settings, as (V, F) in order (`Lowerer::finding_settings`).
fn fix_lines(settings: &[(String, String)]) -> Vec<(String, String)> {
    let words = |text: &str| text.split_whitespace().take_while(|word| !word.starts_with('"')).map(str::to_owned).collect::<Vec<_>>();
    settings.iter().filter(|(key, _)| key == "fix").filter_map(|(_, text)| {
        let words = words(text);
        Some((words.first()?.clone(), words.last()?.clone()))
    }).collect()
}

impl Lowerer {
    /// Each edit of each rule of each fix the module's `.violation`s offer, as its relation (the module doc). `fixes`
    /// are the module's rules that end in edits, as read.
    pub(super) fn fix_outputs(&mut self, fixes: &[ReadClause]) {
        let Some(policy) = policy_id(&self.prog.settings) else { return };
        let lines = fix_lines(&self.prog.settings);
        for violation in self.violations.clone() {
            let offered = lines.iter().filter(|(v, _)| *v == violation.name).map(|(_, f)| f);
            for (fix, f) in offered.enumerate() {
                let rules = fixes.iter().filter(|(heads, ..)| matches!(heads.as_slice(), [head] if head.rel == *f));
                for (rule, (heads, body, edits, _)) in rules.enumerate() {
                    for (edit, term) in edits.iter().enumerate() {
                        let at = EditAt { fix, rule, edit, edits: edits.len() };
                        self.fix_edit(&policy, &violation, at, &heads[0], body, term);
                    }
                }
            }
        }
    }

    /// One edit's relation; nothing where the edit is not run or its rule does not lower (the module doc).
    fn fix_edit(&mut self, policy: &str, violation: &Violation, at: EditAt, head: &Head, body: &[Lit], term: &STerm) {
        let Some((target, template)) = replace_edit(term) else { return };
        let Some(parts) = template_parts(&template) else { return };
        if head.args.len() != violation.arity {
            return;
        }
        let saved = self.clone();
        if self.edit_rule(policy, violation, at, head, body, (&target, &template, &parts)).is_none() {
            *self = saved;
        }
    }

    /// `edit_relation(…)(constants…, V's columns…, target, placeholders…) :- V(F's head…), F's body.`, declared and
    /// an output; `None` where a column is not typed by the rule or the rule does not lower.
    fn edit_rule(&mut self, policy: &str, violation: &Violation, at: EditAt, head: &Head, body: &[Lit], (target, template, parts): (&str, &str, &[Part])) -> Option<()> {
        // V's columns are F's head: each `_` a variable of its own.
        let columns: Vec<STerm> = head.args.iter().enumerate().map(|(k, a)| if *a == STerm::Wild { STerm::Var(format!("rr_fix_{k}")) } else { a.clone() }).collect();
        let mut lits = vec![Lit::Atom { rel: violation.name.clone(), args: columns.clone(), neg: false }];
        lits.extend(body.iter().cloned());
        let mut holes: Vec<String> = Vec::new();
        for part in parts {
            if let Part::Hole(name) = part && !holes.contains(name) {
                holes.push(name.clone());
            }
        }
        let target_type = self.var_type(target, &lits).filter(|t| t == "node")?;
        let hole_types = holes.iter().map(|hole| self.var_type(hole, &lits)).collect::<Option<Vec<String>>>()?;
        let name = edit_relation(policy, &violation.name, at);
        let names = |names: &mut dyn Iterator<Item = &String>| names.map(String::as_str).collect::<Vec<_>>().join(",");
        let constants = [
            STerm::Str(policy.to_owned()), STerm::Str(violation.name.clone()), STerm::Num(at.fix as i64), STerm::Num(at.rule as i64),
            STerm::Num(at.edit as i64), STerm::Num(at.edits as i64), STerm::Str(template.to_owned()),
            STerm::Str(names(&mut violation.columns.iter().map(|(c, _)| c))), STerm::Str(names(&mut holes.iter())),
        ];
        let types = ["symbol", "symbol", "number", "number", "number", "number", "symbol", "symbol", "symbol"];
        let mut decl: Vec<(String, String)> = HEAD.iter().zip(types).map(|(c, t)| ((*c).to_owned(), t.to_owned())).collect();
        decl.extend(violation.columns.iter().map(|(c, t)| (format!("v_{c}"), t.clone())));
        decl.push(("target".to_owned(), target_type));
        decl.extend(holes.iter().zip(hole_types).map(|(hole, t)| (format!("p_{hole}"), t)));
        let mut args: Vec<STerm> = constants.into_iter().chain(columns).collect();
        args.push(STerm::Var(target.to_owned()));
        args.extend(holes.iter().map(|hole| STerm::Var(hole.clone())));
        self.decl(name.clone(), decl, &[]).ok()?;
        self.clause(&[Head { modal: None, rel: name.clone(), args }], &lits).ok()?;
        self.prog.outputs.push(name);
        Some(())
    }

    /// The type of variable `v` as the declared column of a positive atom of `lits` binding it holds it (in
    /// alternatives, as the first does); `None` where no such atom binds it.
    fn var_type(&self, v: &str, lits: &[Lit]) -> Option<String> {
        lits.iter().find_map(|l| match l {
            Lit::Atom { rel, args, neg: false } => {
                let cols = self.prog.decls.get(rel.as_str())?;
                args.iter().zip(cols).find_map(|(a, (_, t))| matches!(a, STerm::Var(x) if x == v).then(|| t.to_string()))
            }
            Lit::Group { alts, neg: false } => alts.first().and_then(|alt| self.var_type(v, alt)),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_template_is_text_and_placeholders() {
        let parts = template_parts("!($a $o $r)").unwrap();
        let text = |t: &str| Part::Text(t.to_owned());
        let hole = |h: &str| Part::Hole(h.to_owned());
        assert_eq!(parts, [text("!("), hole("a"), text(" "), hole("o"), text(" "), hole("r"), text(")")]);
        assert_eq!(template_parts("{$b}").unwrap(), [text("{"), hole("b"), text("}")]);
        assert_eq!(template_parts("`${x}` costs $"), Some(vec![text("`${x}` costs $")]));
        assert_eq!(template_parts("f($$$ARGS)"), None);
    }

    #[test]
    fn a_fix_edit_relation_is_known_in_a_pack_too() {
        let name = edit_relation("p", "V", EditAt { fix: 0, rule: 0, edit: 0, edits: 1 });
        assert!(is_fix_edit(&name) && is_fix_edit(&format!("p3_{name}")) && !is_fix_edit("rr_diagnostic_00"));
    }
}
