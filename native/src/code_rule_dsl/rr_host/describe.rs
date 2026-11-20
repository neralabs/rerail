//! `describeRr`: what a `.rerail` rule file states, as the engine reads it (its settings, its findings' codes, its
//! tests and the files it read), once it reads and checks as a run would (`tools::check_rr`).

use crate::code_rule_dsl::logic::front::Program;
use crate::code_rule_dsl::logic::front::ast::{Atom, Term};
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
    Ok(json!({
        "path": path, "ok": true, "id": header.id, "revision": header.revision, "description": header.description,
        "mode": header.mode, "language": header.language,
        "scope": {"include": header.include, "exclude": header.exclude, "toolCalls": header.tool_calls},
        "codes": codes(&program, &header.id).map_err(|e| format!("{}: {e}", path.display()))?,
        "tests": tests(path)?,
        "suppresses": program.settings.iter().any(|(key, _)| key == "suppress"),
        "fixes": program.settings.iter().any(|(key, _)| key == "fix"),
        "files": read_files(path, &files)?,
    }))
}

/// The file read and checked as `check-rr` checks it (its reader, then its types): the error otherwise.
fn checked(path: &Path) -> Result<(), String> {
    let row = crate::tools::check_rr(&[path.to_path_buf()]).as_array().and_then(|rows| rows.first().cloned()).unwrap_or_default();
    match row["status"].as_str() {
        Some("reader-passed") => Ok(()),
        _ => Err(format!("{}: {}", path.display(), row["error"].as_str().unwrap_or("the file does not check"))),
    }
}

/// The settings a rule states of itself.
struct Header {
    id: String,
    revision: i64,
    description: Option<String>,
    mode: String,
    language: Option<String>,
    include: Vec<String>,
    exclude: Vec<String>,
    tool_calls: bool,
}

impl Header {
    fn of(settings: &[(String, String)]) -> Result<Header, String> {
        let (id, revision) = policy(setting(settings, "policy").ok_or("no `.policy \"id\" revision N`")?)?;
        let mut header = Header {
            id,
            revision,
            description: setting(settings, "description").map(first_text).transpose()?.flatten(),
            mode: mode(settings)?,
            language: setting(settings, "language").map(first_text).transpose()?.flatten(),
            include: Vec::new(),
            exclude: Vec::new(),
            tool_calls: false,
        };
        for (_, text) in settings.iter().filter(|(key, _)| key == "scope") {
            header.scope(text)?;
        }
        Ok(header)
    }

    /// One `.scope` line: its globs, or `tool_calls`. The other forms (`dirs`, `text`, `tests`, …) narrow the run
    /// natively and are not described.
    fn scope(&mut self, text: &str) -> Result<(), String> {
        let words = words(text)?;
        let globs = words.iter().skip(1).cloned();
        match words.first().map(String::as_str) {
            Some("include") => self.include.extend(globs),
            Some("exclude") => self.exclude.extend(globs),
            Some("tool_calls") => self.tool_calls = true,
            _ => {}
        }
        Ok(())
    }
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
