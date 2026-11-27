//! `describeRr`: what a `.rerail` rule file states, as the engine reads it (its settings, its findings' codes, its
//! tests and the files it read), once it reads and checks as a run would (`tools::check_rr`).

use crate::code_rule_dsl::logic::front::{OPTION_GLOB, Program};
use crate::code_rule_dsl::logic::front::ast::{Atom, Literal, Rule, Term, Value as FactValue};
use crate::code_rule_dsl::logic::rr;
use rerail_rr_syntax::syntax::{Item, Tok, tokens};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// One row per path, in order: the file described, or why it is not.
pub fn describe(paths: &[PathBuf]) -> Vec<Value> {
    paths.iter().map(|path| described(path).unwrap_or_else(|error| json!({"path": path, "ok": false, "error": error}))).collect()
}

fn described(path: &Path) -> Result<Value, String> {
    checked(path)?;
    let (program, files) = rr::read_file(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let header = Header::of(&program.settings).map_err(|e| format!("{}: {e}", path.display()))?;
    let options = Options::of(&program).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(json!({
        "path": path, "ok": true, "id": header.id, "revision": header.revision, "description": header.description,
        "mode": header.mode, "language": header.language,
        "scope": scope(&program, &header),
        "codes": codes(&program, &header.id).map_err(|e| format!("{}: {e}", path.display()))?,
        "tests": tests(path)?,
        "violations": violations(&program, &header.id),
        "options": options.names, "anyOption": options.any,
        "files": read_files(path, &files)?,
    }))
}

/// The policy's `.violation`s, each once in the order its `violation` rules come (`finding_of`), with the `suppress`
/// and `fix` lines lowering kept as settings of the findings layer (`suppress V tool "rule", … [over {c}]`,
/// `fix V [safe] F "title"`): `{name, suppress: [{tool, rules}], fixes: [F]}`.
fn violations(program: &Program, policy: &str) -> Vec<Value> {
    let mut names: Vec<&str> = Vec::new();
    let clauses = program.rules.iter().map(|rule| &rule.head).filter(|head| finding_of(head, policy)).filter_map(|head| match head.args.get(1) {
        Some(Term::Str(clause)) => Some(clause.as_str()),
        _ => None,
    });
    for clause in clauses {
        if !names.contains(&clause) {
            names.push(clause);
        }
    }
    let lines = |key: &'static str| program.settings.iter().filter(move |(k, _)| k == key).filter_map(|(_, text)| tokens(text).ok());
    names
        .into_iter()
        .map(|name| {
            let suppress: Vec<Value> = lines("suppress").filter_map(|line| suppress_line(name, &line)).collect();
            let fixes: Vec<Value> = lines("fix").filter_map(|line| fix_line(name, &line)).collect();
            json!({"name": name, "suppress": suppress, "fixes": fixes})
        })
        .collect()
}

/// A `suppress V tool "rule", … [over {c}]` setting's tool and rules, where it is violation `name`'s.
fn suppress_line(name: &str, line: &[(Tok<'_>, usize, usize)]) -> Option<Value> {
    let [(Tok::Id(v), ..), (Tok::Id(tool), ..), rest @ ..] = line else { return None };
    let texts = rest.iter().take_while(|(tok, ..)| !matches!(tok, Tok::Id("over")));
    let rules: Vec<String> = texts
        .filter_map(|(tok, ..)| match tok {
            Tok::Str(rule) => Some(rule.to_string()),
            _ => None,
        })
        .collect();
    (*v == name).then(|| json!({"tool": tool, "rules": rules}))
}

/// A `fix V [safe] F "title"` setting's fix relation F, where it is violation `name`'s.
fn fix_line(name: &str, line: &[(Tok<'_>, usize, usize)]) -> Option<Value> {
    let [(Tok::Id(v), ..), rest @ ..] = line else { return None };
    let fix = rest.iter().rev().find_map(|(tok, ..)| match tok {
        Tok::Id(f) => Some(*f),
        _ => None,
    })?;
    (*v == name).then(|| json!(fix))
}

/// The options the policy takes: the names its rules, and those of the files it includes, read `option` or
/// `option_item` by, sorted, each once. Nothing declares them: a rule says what an option left unset means
/// (`!option("max", _)`). `any`: some read's name is a variable the rule's own facts do not give (`names_of`), so the
/// names cannot be listed, and the policy may read any.
struct Options {
    names: Vec<String>,
    any: bool,
}

impl Options {
    /// The policy's options. An error for a policy that carries the converter's `.options {…}` declaration, which a
    /// rule does not state, or that reads an option named as a run's own setting (`run::RUN_SETTINGS`): the two share
    /// the object a run's option values come in.
    fn of(program: &Program) -> Result<Options, String> {
        if program.settings.iter().any(|(key, _)| key == "options") {
            return Err("it carries an `.options {…}` declaration; a rule declares no options: its rules say what an option left unset means".to_owned());
        }
        let (mut names, mut any) = (std::collections::BTreeSet::new(), false);
        for rule in &program.rules {
            for atom in rule.body.iter().filter_map(Literal::atom).filter(|atom| reads_option(atom)) {
                match names_of(program, rule, atom) {
                    Some(read) => names.extend(read),
                    None => any = true,
                }
            }
        }
        if let Some(name) = names.iter().find(|name| super::run::RUN_SETTINGS.contains(&name.as_str())) {
            return Err(format!("it reads an option named {name}, the name of a run's own setting; give the option another name"));
        }
        Ok(Options { names: names.into_iter().collect(), any })
    }
}

/// Whether `atom` reads a policy's options.
fn reads_option(atom: &Atom) -> bool {
    matches!(atom.rel.as_str(), "option" | "option_item")
}

/// The names `atom`, a read of `option` or `option_item` in `rule`, reads: the text it is written with, or, for a
/// variable, the texts the rule's facts give it (`limit(o, k) :- limit_rule(o, d), option(o, v)` beside
/// `limit_rule("max", 3).`). `None` where they cannot be listed.
fn names_of(program: &Program, rule: &Rule, atom: &Atom) -> Option<Vec<String>> {
    match atom.args.first()? {
        Term::Str(name) => Some(vec![name.as_str().to_owned()]),
        Term::Var(variable) => {
            let beside = rule.body.iter().filter_map(Literal::atom).filter(|other| !other.neg && !reads_option(other));
            beside.into_iter().find_map(|other| fact_texts(program, other, variable.as_str()))
        }
        _ => None,
    }
}

/// The texts `variable` holds where `atom` reads it: its column's values, where the atom's relation is facts alone
/// (no rule derives it and no run gives it; none, for a library's relation this policy states no fact of) and each
/// is a text. `None` where the atom does not read the variable, or reads it otherwise.
fn fact_texts(program: &Program, atom: &Atom, variable: &str) -> Option<Vec<String>> {
    let column = atom.args.iter().position(|term| matches!(term, Term::Var(name) if name.as_str() == variable))?;
    let rel = atom.rel.as_str();
    if program.inputs.iter().any(|input| input == rel) || program.rules.iter().any(|rule| rule.head.rel.as_str() == rel) {
        return None;
    }
    let texts = program.facts.get(rel).into_iter().flatten().map(|row| match row.get(column) {
        Some(FactValue::Str(text)) => Some(text.as_str().to_owned()),
        _ => None,
    });
    texts.collect()
}

/// The file read and checked as `check-rr` checks it (its reader, then its types): the error otherwise, naming the
/// file once (the reader's errors name it already, the type check's do not).
fn checked(path: &Path) -> Result<(), String> {
    let row = crate::tools::check_rr(&[path.to_path_buf()]).as_array().and_then(|rows| rows.first().cloned()).unwrap_or_default();
    let error = match row["status"].as_str() {
        Some("reader-passed") => return Ok(()),
        _ => row["error"].as_str().unwrap_or("the file does not check"),
    };
    let named = path.display().to_string();
    Err(if error.starts_with(&named) { error.to_owned() } else { format!("{named}: {error}") })
}

/// The settings a rule states of itself.
struct Header {
    id: String,
    revision: i64,
    description: Option<String>,
    mode: String,
    language: Option<String>,
    tool_calls: bool,
}

impl Header {
    fn of(settings: &[(String, String)]) -> Result<Header, String> {
        let (id, revision) = policy(setting(settings, "policy").ok_or("no `.policy \"id\" revision N`")?)?;
        Ok(Header {
            id,
            revision,
            description: setting(settings, "description").map(first_text).transpose()?.flatten(),
            mode: mode(settings)?,
            language: setting(settings, "language").map(first_text).transpose()?.flatten(),
            tool_calls: settings.iter().any(|(key, text)| key == "scope" && text.trim() == "tool_calls"),
        })
    }
}

/// The files the rule reads, as the run selects them (`corpus::scoped`): `include`, a glob of which a file matches (any
/// file, when none), its `dirs` and `include` forms; `exclude`, its `exclude` forms and `tests false`; `text`, the files
/// it reads as text, whatever their language (`.scope text`). An `include` taken from a policy option is known only at
/// the run, so a rule with one takes any file; an `exclude` from one is left out, which takes more files, never fewer.
fn scope(program: &Program, header: &Header) -> Value {
    let (within, out) = program.doc_scopes.iter().flatten().next().cloned().unwrap_or_default();
    let known = |globs: Vec<String>| globs.into_iter().filter(|g| !g.starts_with(OPTION_GLOB)).collect::<Vec<_>>();
    let include = if within.iter().any(|g| g.starts_with(OPTION_GLOB)) { Vec::new() } else { within };
    json!({"include": include, "exclude": known(out), "text": program.text_globs, "toolCalls": header.tool_calls})
}

/// The first setting named `key`, as written.
fn setting<'s>(settings: &'s [(String, String)], key: &str) -> Option<&'s str> {
    settings.iter().find(|(k, _)| k == key).map(|(_, text)| text.as_str())
}

/// A setting's words and texts, in order (`include "a", "b"`: `include`, `a`, `b`).
fn words(text: &str) -> Result<Vec<String>, String> {
    let tokens = tokens(text).map_err(syntax_error)?;
    Ok(tokens
        .into_iter()
        .filter_map(|(tok, _, _)| match tok {
            Tok::Id(word) => Some(word.to_owned()),
            Tok::Str(text) => Some(text.into_owned()),
            _ => None,
        })
        .collect())
}

/// A setting's first word or text (`.language python`, `.description "…"`).
fn first_text(text: &str) -> Result<Option<String>, String> {
    Ok(words(text)?.into_iter().next())
}

/// `.policy "id" revision N`.
fn policy(text: &str) -> Result<(String, i64), String> {
    let tokens = tokens(text).map_err(syntax_error)?;
    let Some((Tok::Str(id), _, _)) = tokens.first() else { return Err("`.policy` names no quoted id".to_owned()) };
    let revision = tokens.windows(2).find_map(|pair| match (&pair[0].0, &pair[1].0) {
        (Tok::Id("revision"), Tok::Num(n)) => Some(n.to_owned()),
        _ => None,
    });
    Ok((id.to_string(), revision.ok_or("`.policy` states no revision")?))
}

/// A setting's text that does not read, as the reader says it.
fn syntax_error(error: rerail_rr_syntax::Error) -> String {
    rr::Error::from(error).to_string()
}

/// The policy's mode: its one `.mode`. A mode for some codes alone (`.mode log CODE, …`) is not described.
fn mode(settings: &[(String, String)]) -> Result<String, String> {
    let mut modes = settings.iter().filter(|(key, _)| key == "mode").map(|(_, text)| words(text));
    let (Some(only), None) = (modes.next().transpose()?, modes.next()) else {
        return Err("a rule states one mode, `.mode block|steer|log`".to_owned());
    };
    match only.as_slice() {
        [mode] if ["block", "steer", "log"].contains(&mode.as_str()) => Ok(mode.clone()),
        _ => Err(format!("`.mode {}`: a mode for some codes is not described; state one mode", only.join(" "))),
    }
}

/// The codes the policy's findings can have, sorted, each once: each `.violation`'s and each `.on … report CODE`'s,
/// as its `violation` rules hold them. A code a finding takes from a column cannot be listed.
fn codes(program: &Program, policy: &str) -> Result<Vec<String>, String> {
    let heads = program.rules.iter().map(|rule| &rule.head).filter(|head| finding_of(head, policy));
    let mut codes = heads.map(code).collect::<Result<Vec<_>, _>>()?;
    codes.sort();
    codes.dedup();
    Ok(codes)
}

/// Whether `head` is one of `policy`'s `violation` rules (`rr::lower`'s findings: policy, clause, code, …).
fn finding_of(head: &Atom, policy: &str) -> bool {
    head.rel.as_str() == "violation" && matches!(head.args.first(), Some(Term::Str(p)) if p.as_str() == policy)
}

/// A `violation` rule's code: a text, or an error naming the clause whose code is a column.
fn code(head: &Atom) -> Result<String, String> {
    match (head.args.get(1), head.args.get(2)) {
        (_, Some(Term::Str(code))) => Ok(code.as_str().to_owned()),
        (Some(Term::Str(clause)), _) => Err(format!(".violation {}: its code is a column; its codes cannot be listed", clause.as_str())),
        _ => Err("a finding's code is a column; its codes cannot be listed".to_owned()),
    }
}

/// The number of `.test` blocks the file holds.
fn tests(path: &Path) -> Result<usize, String> {
    let (statements, _) = rr::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(statements.iter().filter(|(item, _)| matches!(item, Item::Test { .. })).count())
}

/// The rule file, then each file it includes that the engine read, each by its canonical path and its bytes' SHA-256.
fn read_files(path: &Path, included: &[rr::LibraryFile]) -> Result<Vec<Value>, String> {
    let own = dunce::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let bytes = std::fs::read(&own).map_err(|e| format!("{}: {e}", own.display()))?;
    let mut files = vec![json!({"path": own, "sha256": hex(&Sha256::digest(&bytes))})];
    for file in included {
        let canonical = dunce::canonicalize(&file.path).unwrap_or_else(|_| file.path.clone());
        if let (Some(sha), true) = (file.sha256, canonical != own) {
            files.push(json!({"path": canonical, "sha256": hex(&sha)}));
        }
    }
    Ok(files)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
