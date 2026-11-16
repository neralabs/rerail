//! Rules in `.rr`, the engine's rule language: Soufflé Datalog with the additions of
//! `docs/rule-syntax/lib/core.rr` (decision 0043). Every rule the engine runs is read here.
//!
//! - `syntax`: tokens and statements as written;
//! - `lower`: statements to the engine's `Program`, the form the planner, magic sets and the backends take;
//! - `print`: a `Program` as `.rr` text, read back as the same program;
//! - `check`: the checks of `docs/rule-syntax/site/check_rr.py` (grounded, declared once, stratified);
//! - `translate`: the earlier `.rdl` dialect's text in `.rr`, comments kept, for the move and for the converter.
//!
//! `#include "x.rr"` reads `docs/rule-syntax/lib/x.rr`, or else `x.rr` beside the including file; each file once.
//! A file read with `parse_file` has `core.rr` before it, included or not. An included file's `.output`s are its own,
//! not the program's: they are left out (`python/calls.rr`'s).

pub use rerail_rr::code_rule_dsl::logic::rr::check;
#[cfg_attr(any(test, feature = "test-hooks"), path = "rr/aggregate_demand")]
pub mod aggregate_demand {
    pub use rerail_rr::code_rule_dsl::logic::rr::aggregate_demand::*;
    #[cfg(any(test, feature = "test-hooks"))]
    pub mod tests;
    #[allow(unused_imports)]
    use crate::code_rule_dsl::logic::front::{self, AggOp, Atom, Literal, Program, Rule, Term};
    #[allow(unused_imports)]
    use std::collections::{BTreeMap, BTreeSet};
}
pub mod from_tape;
pub mod images;
pub mod module_image;
pub use rerail_rr::code_rule_dsl::logic::rr::lower;
/// The text tests `.rr` names, and the engine's test each lowers to (the coverage gate probes each).
pub use rerail_rr::code_rule_dsl::logic::rr::lower::TESTS as RR_TESTS;
pub use rerail_rr::code_rule_dsl::logic::rr::print::{self};
// The parser and the tape's tree, the `.rr` parser crate's (`rerail_rr_syntax`).
use rerail_rr_syntax::syntax;
pub use rerail_rr_syntax::tape;

/// `src`'s tape as nodes: the lean parser's image (decision 0140) read back (`tape::nodes`), what
/// the statements are read off as a stored tape's are (decision 0062).
pub fn nodes(src: &str) -> Vec<tape::Node> {
    tape::nodes(&tape(src)).expect("the lean parser's image decodes")
}
/// `src` with each `.violation`'s message reduced to its placeholders (`lower::findings::message_skeleton`): the
/// text a program's keys are made of (`corpus::texts_digest`). Lowering reads a message's placeholders alone, and the
/// writer puts its texts in (decision 0225 §3), so an edit of a message's words is no edit of the program.
pub fn message_masked(src: &str) -> std::borrow::Cow<'_, str> {
    if !src.contains(".violation") {
        return std::borrow::Cow::Borrowed(src);
    }
    let message = tape::Field::Message as u16;
    let mut spans: Vec<(usize, usize)> = nodes(src).iter().filter(|n| n.field == message && !n.is_error()).map(|n| (n.start as usize, n.end as usize)).collect();
    spans.sort();
    let mut out = String::with_capacity(src.len());
    let mut at = 0;
    for (start, end) in spans {
        let Some(literal) = src.get(start..end).filter(|l| start >= at && l.len() >= 2 && l.starts_with('"') && l.ends_with('"')) else { continue };
        let Ok(template) = syntax::unescape(&literal[1..literal.len() - 1]) else { continue };
        out.push_str(&src[at..start]);
        out.push_str(&quote(&lower::findings::message_skeleton(&template)));
        at = end;
    }
    out.push_str(&src[at..]);
    std::borrow::Cow::Owned(out)
}

/// The message templates a text gives its findings, by (policy, clause): its `.violation`s', and the `.message`
/// settings a printed program keeps them in (`lower::findings::MESSAGE`). The policy is the `.policy` setting's name,
/// as lowering names it in each finding.
pub fn message_templates(src: &str) -> Vec<(String, String, String)> {
    if !src.contains(".violation") && !src.contains(".message") {
        return Vec::new();
    }
    let Ok(statements) = from_tape::statements(src, &nodes(src)) else { return Vec::new() };
    let policy = statements
        .iter()
        .find_map(|(item, _)| match item {
            syntax::Item::Setting { key, text } if key == "policy" => text.split('"').nth(1).map(str::to_owned),
            _ => None,
        })
        .unwrap_or_default();
    statements
        .iter()
        .filter_map(|(item, _)| match item {
            syntax::Item::Violation { name, message, .. } => Some((name.clone(), message.clone())),
            syntax::Item::Setting { key, text } if key == lower::findings::MESSAGE => setting_template(text),
            _ => None,
        })
        .map(|(clause, template)| (policy.clone(), clause, template))
        .collect()
}

/// A `.message` setting's clause and template (`B101 "…"`).
pub fn setting_template(text: &str) -> Option<(String, String)> {
    let (clause, literal) = text.trim().split_once(' ')?;
    let literal = literal.trim();
    let body = literal.strip_prefix('"')?.strip_suffix('"')?;
    Some((clause.to_owned(), syntax::unescape(body).ok()?))
}

#[cfg_attr(any(test, feature = "test-hooks"), path = "rr")]
pub mod tape_type {
    pub use rerail_rr::code_rule_dsl::logic::rr::tape_type::*;
    #[cfg(any(test, feature = "test-hooks"))]
    #[path = "tape_type_tests.rs"]
    pub mod tests;
    #[allow(unused_imports)]
    use std::collections::{BTreeMap, BTreeSet};
    #[allow(unused_imports)]
    use rerail_rr_syntax::syntax::{Head, Item, Lit, Statement, Term};
    #[allow(unused_imports)]
    use crate::code_rule_dsl::logic::front::Error;
}
pub mod token_reads;
pub use rerail_rr::code_rule_dsl::logic::rr::translate::{self};

// cratify: `tape_tests` is rerail-engine's (it mounts it)
// cratify: `tests` is rerail-engine's (it mounts it)

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub use super::front::Error;
use super::front::Program;
#[cfg(any(test, feature = "test-hooks"))]
pub use lower::drop_unread_core;
pub use aggregate_demand::retype_helpers;
pub use lower::is_core;
pub use lower::findings::{is_diagnostic, HEAD as DIAGNOSTIC_HEAD};
pub use lower::WORLD;
pub use print::print;
pub use syntax::quote;
pub use translate::translate;

/// The relations `core.rr` declares: the engine's names (decision 0141). Empty when `core.rr` does not read.
pub fn core_names() -> &'static std::collections::BTreeSet<String> {
    static CORE: std::sync::OnceLock<std::collections::BTreeSet<String>> = std::sync::OnceLock::new();
    CORE.get_or_init(|| {
        std::fs::read_to_string(library().join("core.rr")).ok().and_then(|src| parse(&src).ok()).map(|p| p.decls.into_keys().map(String::from).collect()).unwrap_or_default()
    })
}

/// The library `#include` reads from.
pub fn library() -> PathBuf {
    // `RERAIL_RR_LIB` names another copy: the browser build (native/wasm) mounts the library in its own filesystem.
    std::env::var_os("RERAIL_RR_LIB")
        .map(PathBuf::from)
        // From the checkout's root, with no `..` in the path: a path the engine reads is named one way, so the file
        // observer keeps one answer and one journal root for each directory (`native/../docs` and `docs` were two).
        .unwrap_or_else(|| rerail_store::source_tree::root().map_or_else(|| PathBuf::from("docs/rule-syntax/lib"), |root| root.join("docs/rule-syntax/lib")))
}

/// A program's `.rr` text. Its `#include`s name library files (decision 0089), read as `parse_file` reads them:
/// `core.rr` first unless the text is in T's shape, each file once, includes first. A text that includes nothing is
/// read as it is, without `core.rr`.
pub fn parse(src: &str) -> Result<Program, Error> {
    parse_with(src, true)
}

/// A program's `.rr` text as `parse` reads it, but with only its libraries' declarations, not their clauses: what a
/// run that holds the libraries once (decision 0089 §5) needs of a program that adds to no library relation. Its own
/// rules are lowered against the library's names; the library's rules are the run's, lowered once.
pub fn parse_own(src: &str) -> Result<Program, Error> {
    parse_with(src, false)
}

// A run handed a program it read is handed the program itself (`corpus::run_read`), not its print to lower again.
fn parse_with(src: &str, clauses: bool) -> Result<Program, Error> {
    read_text(src)?.lower(clauses)
}

/// A program text read once (its tape, its statements, its includes resolved), for what a run needs of it: its
/// library modules (`modules`) and its program (`lower`). A run of a pack asks both of every program; each read of
/// a text makes its tape and resolves its includes again.
pub struct Text(Read);

enum Read {
    /// A text that includes nothing: read as it is.
    Plain(Vec<syntax::Statement>),
    /// A text's library files (includes first, without their includes: canonical path, bytes' digest, statements as
    /// `read` keeps them) and its own statements.
    Modular { files: Vec<(PathBuf, [u8; 32], std::sync::Arc<Vec<syntax::Statement>>)>, own: Vec<syntax::Statement> },
}

/// `src` read once, its includes resolved (`Text`).
pub fn read_text(src: &str) -> Result<Text, Error> {
    read_parsed(from_tape::statements(src, &nodes(src))?)
}

/// `read_text` of statements already parsed.
pub(in crate::code_rule_dsl) fn read_parsed(statements: Vec<syntax::Statement>) -> Result<Text, Error> {
    if !statements.iter().any(|(item, _)| matches!(item, syntax::Item::Include(_))) {
        refuse_ungrounded(&statements, &BTreeSet::new())?;
        return Ok(Text(Read::Plain(statements)));
    }
    let (files, own) = resolve_shared(statements)?;
    let given: BTreeSet<String> = files.iter().flat_map(|(_, _, s)| check::fix_relations(s)).collect();
    refuse_ungrounded(&own, &given)?;
    Ok(Text(Read::Modular { files, own }))
}

/// A program's own rules must be grounded (`check::grounded`, which the library's files pass by test): every head
/// variable, and every variable a negation, comparison or aggregate reads from outside, bound by a positive literal
/// of the body. An ungrounded rule otherwise fails late and far from its cause — "unplaced literals", "$x is unbound
/// in SQL", a statement over SQLite's 65535 references — or, when no rule reads the unbound column, is silently
/// projected away.
fn refuse_ungrounded(statements: &[syntax::Statement], given: &BTreeSet<String>) -> Result<(), Error> {
    // A program in T's shape (it declares `doc` itself: a lowered program printed and read again, `combine`) holds
    // the lowering's own rules, which are not source rules: only a source text is checked.
    if statements.iter().any(|(item, _)| matches!(item, syntax::Item::Decl { name, .. } if name == "doc")) {
        return Ok(());
    }
    let bad = check::grounded(statements, given);
    if bad.is_empty() {
        return Ok(());
    }
    let lines: Vec<String> = bad.iter().take(8).map(|(line, vars)| format!("line {line}: {}", vars.join(", "))).collect();
    Err(Error::Syntax(format!(
        "{} ungrounded rule{} (a variable no positive body literal binds): {}{}",
        bad.len(),
        if bad.len() == 1 { "" } else { "s" },
        lines.join("; "),
        if bad.len() > 8 { "; …" } else { "" }
    )))
}

impl Text {
    /// The text's library modules (`modules`).
    pub fn modules(&self) -> Option<Modules> {
        let Read::Modular { files, own } = &self.0 else { return None };
        let declares = |statements: &[syntax::Statement]| -> std::collections::BTreeSet<String> {
            statements
                .iter()
                .filter_map(|(item, _)| match item {
                    syntax::Item::Decl { name, .. } | syntax::Item::Violation { name, .. } => Some(name.clone()),
                    _ => None,
                })
                .collect()
        };
        let heads = own
            .iter()
            .flat_map(|(item, _)| match item {
                syntax::Item::Clause { heads, edits, .. } if edits.is_empty() => heads.iter().map(|h| h.rel.clone()).collect(),
                _ => Vec::new(),
            })
            .collect();
        Some(Modules { files: files.iter().map(|(path, digest, statements)| (path.clone(), *digest, declares(statements))).collect(), heads, declared: declares(own) })
    }

    /// The text's header: its program without a clause of its own or of its libraries (their declarations, its
    /// directives, its findings' declarations), for what a batch reads of a policy before its run (decision 0168):
    /// its settings, `.policy`, `.language` and outputs, as its whole program has them, less `.global library` (which
    /// `read_file` writes) and the settings a clause makes. Lowering only that is a fraction of lowering its rules.
    pub fn header(self) -> Result<Program, Error> {
        let header = |statements: Vec<syntax::Statement>| -> Vec<syntax::Statement> {
            statements.into_iter().filter(|(item, _)| !matches!(item, syntax::Item::Clause { .. } | syntax::Item::Include(_))).collect()
        };
        match self.0 {
            Read::Plain(statements) => lower::lower(header(statements)),
            Read::Modular { files, own } => {
                let mut statements: Vec<syntax::Statement> = files.iter().flat_map(|(_, _, s)| s.iter().filter(|(item, _)| !matches!(item, syntax::Item::Clause { .. } | syntax::Item::Include(_))).cloned()).collect();
                statements.extend(header(own));
                lower::lower(statements)
            }
        }
    }

    /// The text's own program, lowered as a module against the library's modules its includes reach and linked in
    /// their scope (`lower::link_in`, decision 0246 §2): what `lower(false)` gives less what `own_part` takes out,
    /// without lowering the library's declarations again. A text that asks of the library has its request's part
    /// copied from the linked library (`with_cone`). `None` for a text that includes nothing. `store` holds it as a program
    /// image (decision 0246 §6) under its text `src` and its scope's keys: a store that holds it reads it, and the
    /// text is not lowered.
    pub fn lower_in(self, src: &str, library: &Library, scoped: &Scoped, store: Option<&crate::content_store::ContentStore>) -> Result<Option<Program>, Error> {
        let Read::Modular { files, own } = self.0 else { return Ok(None) };
        let key = program_key(src, &scoped.keys);
        if let Some((bytes, id)) = images::read(store, key)
            && let Ok(program) = super::program_image::Version::open(&bytes).and_then(|v| v.program(&Default::default()))
        {
            if let (Some(store), Some(id)) = (store, id) {
                renew_image(store, id);
            }
            crate::code_rule_dsl::prof::count("rr.program_image_read", 1);
            return Ok(Some(program));
        }
        let checked: Vec<syntax::Statement> = own.iter().filter(|(item, _)| !matches!(item, syntax::Item::Clause { body, .. } if body.is_empty())).cloned().collect();
        let reads_core = files.first().is_some_and(|(path, _, _)| path.ends_with("core.rr"));
        check_names(&vec![(PathBuf::from("<program>"), checked)], reads_core)?;
        closed_program(&files, &own)?;
        let own: Vec<syntax::Statement> = own.into_iter().filter(|(item, _)| !matches!(item, syntax::Item::Include(_))).collect();
        let m = lower::lower_module_seeded(own, &scoped.seeds)?;
        let mut program = lower::link_seeded(m, &scoped.seeds, core_names())?;
        // A text that adds to library relations asks of the library (decision 0089 §3): its request's cone, the library
        // relations that read what it adds, and the helpers they read, are copied from the linked library, made once
        // (decision 0246 §2: a cone is made at link time). The run's `own_part` then holds them as the request.
        let declared: BTreeSet<String> = files.iter().flat_map(|(_, _, statements)| statements.iter()).filter_map(|(item, _)| match item {
            syntax::Item::Decl { name, .. } | syntax::Item::Violation { name, .. } => Some(name.clone()),
            _ => None,
        }).collect();
        let asked: BTreeSet<String> = program.rules.iter().map(|r| r.head.rel.to_string()).filter(|h| declared.contains(h) && !core_names().contains(h)).collect();
        if !asked.is_empty() {
            with_cone(&mut program, &library.program, &asked, &declared);
        }
        if let Ok(bytes) = encode_program(&program)
            && keep_image(store, "a program image", key, bytes)
        {
            crate::code_rule_dsl::prof::count("rr.program_image_written", 1);
        }
        Ok(Some(program))
    }

    /// The text's program, as `parse` (`clauses`) or `parse_own` reads it.
    pub fn lower(self, clauses: bool) -> Result<Program, Error> {
        let (files, own) = match self.0 {
            Read::Plain(statements) => return lower::lower(statements),
            Read::Modular { files, own } => (files, own),
        };
        // The text's own rules and declarations; its facts may be the runner's (`env`, decision 0089 §1).
        let checked: Vec<syntax::Statement> = own.iter().filter(|(item, _)| !matches!(item, syntax::Item::Clause { body, .. } if body.is_empty())).cloned().collect();
        // `core.rr`'s names, unless the text is in T's shape and reads no `core.rr` (`resolve`).
        let reads_core = files.first().is_some_and(|(path, _, _)| path.ends_with("core.rr"));
        check_names(&vec![(PathBuf::from("<program>"), checked)], reads_core)?;
        closed_program(&files, &own)?;
        // Lowering reads one kind of clause statically: a kind set's (`fscope("module")`, `lfscope(k) :- fscope(k)`),
        // whose relation has one column. Those are kept; the rest are the run's. Only what is kept is copied.
        let unary: std::collections::BTreeSet<&str> = if clauses {
            Default::default()
        } else {
            files
                .iter()
                .flat_map(|(_, _, statements)| statements.iter())
                .filter_map(|(item, _)| match item {
                    syntax::Item::Decl { name, cols, .. } if cols.len() == 1 => Some(name.as_str()),
                    _ => None,
                })
                .collect()
        };
        let kept = |item: &syntax::Item| match item {
            syntax::Item::Include(_) => false,
            syntax::Item::Clause { heads, .. } => clauses || heads.iter().all(|h| unary.contains(h.rel.as_str())),
            _ => true,
        };
        let mut statements: Vec<syntax::Statement> = files.iter().flat_map(|(_, _, statements)| statements.iter().filter(|(item, _)| kept(item)).cloned()).collect();
        statements.extend(own);
        lower::lower(statements)
    }
}

/// A program text's library modules (decision 0089): the files its `#include`s name, as `parse` reads them, and
/// what the text itself declares and derives. `None` for a text that includes nothing (a printed program).
#[derive(Clone, Debug)]
pub struct Modules {
    /// Each library file, includes first: its canonical path, its bytes' digest and the relations it declares.
    pub files: Vec<(PathBuf, [u8; 32], std::collections::BTreeSet<String>)>,
    /// The relations the text's own clauses derive or state facts of.
    pub heads: std::collections::BTreeSet<String>,
    /// The relations the text itself declares.
    pub declared: std::collections::BTreeSet<String>,
}

pub fn modules(src: &str) -> Result<Option<Modules>, Error> {
    Ok(read_text(src)?.modules())
}

/// A run's library (decision 0089): its modules, each with its files (canonical paths), and the program linked from
/// them. A program that asks nothing of the library is lowered against the modules its includes reach
/// (`Text::lower_in`), and the run holds the linked program once.
pub struct Library {
    pub modules: Vec<lower::Module>,
    pub files: Vec<Vec<PathBuf>>,
    /// Each module's key in the store (`module_key`).
    pub keys: Vec<crate::content_store::HashKey>,
    pub program: Program,
}

impl Text {
    /// The library files a modular text includes, as `Library::scope` reads them; `None` for a plain one.
    pub fn files(&self) -> Option<Vec<PathBuf>> {
        match &self.0 {
            Read::Modular { files, .. } => Some(files.iter().map(|(p, _, _)| p.clone()).collect()),
            Read::Plain(_) => None,
        }
    }
}

/// A scope of a library, seeded once for every program lowered in it (`Text::lower_in`): its modules' keys.
pub struct Scoped {
    pub keys: Vec<crate::content_store::HashKey>,
    seeds: lower::Seeds,
}

impl Scoped {
    /// The alias a program of text `src` lowered in this scope is kept under as its image.
    pub fn program_image(&self, src: &str) -> crate::content_store::HashKey {
        program_key(src, &self.keys)
    }
}

impl Library {
    /// The aliases the library's images are kept under: each module's, and the linked library's.
    pub fn image_keys(&self) -> Vec<crate::content_store::HashKey> {
        if self.modules.is_empty() {
            return Vec::new();
        }
        self.keys.iter().copied().chain([linked_key(&self.keys)]).collect()
    }

    /// The scope of a program whose library files are `files`, seeded.
    pub fn scoped(&self, files: &[PathBuf]) -> Scoped {
        let (scope, keys) = self.scope(files);
        Scoped { keys, seeds: lower::seeds(&scope) }
    }

    /// The modules a program whose library files are `files` reads: those all of whose files it includes, in order,
    /// with their keys.
    pub fn scope(&self, files: &[PathBuf]) -> (Vec<&lower::Module>, Vec<crate::content_store::HashKey>) {
        self.modules.iter().zip(&self.files).zip(&self.keys).filter(|((_, fs), _)| fs.iter().all(|f| files.contains(f))).map(|((m, _), k)| (m, *k)).unzip()
    }
}

/// The library files `paths` (canonical, includes first, as `Modules::files` lists them) as one program, lowered
/// once for every program of a run that includes them (decision 0089). Kept for the process by the files' digests.
pub(in crate::code_rule_dsl) fn lower_library(paths: &[PathBuf], store: Option<&crate::content_store::ContentStore>) -> Result<std::sync::Arc<Library>, Error> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static LIBRARIES: OnceLock<Mutex<HashMap<Vec<(PathBuf, [u8; 32])>, Arc<Library>>>> = OnceLock::new();
    let mut files = Vec::new();
    let mut key = Vec::new();
    for path in paths {
        let (statements, file) = read(path)?;
        key.push((path.clone(), file.sha256.unwrap_or_default()));
        files.push((path.clone(), statements.into_iter().filter(|(item, _)| !matches!(item, syntax::Item::Include(_))).collect()));
    }
    let cache = LIBRARIES.get_or_init(Default::default);
    if let Some(library) = cache.lock().expect("the lowered libraries").get(&key) {
        crate::code_rule_dsl::prof::count("rr.library_reused", 1);
        return Ok(Arc::clone(library));
    }
    check_names(&files, false)?;
    let library = if std::env::var("RERAIL_LIBRARY_LOWERING").is_ok_and(|v| v == "whole") {
        Library { modules: Vec::new(), files: Vec::new(), keys: Vec::new(), program: lower::lower(files.into_iter().flat_map(|(_, statements): (PathBuf, Vec<syntax::Statement>)| statements).collect())? }
    } else {
        let (modules, members, keys) = library_modules_with_files(paths, files, store)?;
        // The linked library is an image too (decision 0246 §2, §6), under its modules' keys: a store that holds it
        // reads it, and the whole-program half does not run again.
        let key = linked_key(&keys);
        let stored = images::read(store, key).and_then(|(bytes, id)| {
            let program = super::program_image::Version::open(&bytes).and_then(|v| v.program(&Default::default())).ok()?;
            if let (Some(store), Some(id)) = (store, id) {
                renew_image(store, id);
            }
            crate::code_rule_dsl::prof::count("rr.linked_library_read", 1);
            Some(program)
        });
        let program = match stored {
            Some(program) => program,
            None => {
                let program = lower::link(modules.iter().map(lower::Module::clone).collect())?;
                if let Ok(bytes) = encode_program(&program)
                    && keep_image(store, "a linked library's image", key, bytes)
                {
                    crate::code_rule_dsl::prof::count("rr.linked_library_written", 1);
                }
                program
            }
        };
        Library { modules, files: members, keys, program }
    };
    let library = Arc::new(library);
    let mut kept = cache.lock().expect("the lowered libraries");
    if kept.len() >= 8 {
        kept.clear();
    }
    kept.insert(key, Arc::clone(&library));
    Ok(library)
}

/// Library files (canonical paths, includes first, with their statements less their includes) lowered one module
/// at a time, each against the modules it reaches (decision 0246 §1), and linked. A module is a file, or files that
/// include each other (`python/names.rr` and `python/imports.rr`: one unit, read together). `core.rr` is in every
/// other file's scope, as `parse` reads a file with it first. `RERAIL_LIBRARY_LOWERING=whole` lowers them as one, as
/// before, to compare.
pub fn lower_modules(paths: &[PathBuf], files: Vec<(PathBuf, Vec<syntax::Statement>)>) -> Result<Program, Error> {
    lower::link(library_modules(paths, files)?)
}

/// The library files' modules (`lower_modules`), includes first, each lowered against the modules it reaches.
pub fn library_modules(paths: &[PathBuf], files: Vec<(PathBuf, Vec<syntax::Statement>)>) -> Result<Vec<lower::Module>, Error> {
    let store = image_store();
    Ok(library_modules_with_files(paths, files, store.as_deref())?.0)
}

/// The relations `statements` declare, and whether each is declared `extensible` (decision 0263).
fn declared_in(statements: &[syntax::Statement]) -> impl Iterator<Item = (&str, bool)> {
    statements.iter().filter_map(|(item, _)| match item {
        syntax::Item::Decl { name, tail, .. } => Some((name.as_str(), tail.iter().any(|w| w == "extensible"))),
        _ => None,
    })
}

/// A library's relations are closed (decision 0263): a file adds no rules or facts to a relation another file
/// declares, unless that declaration says `extensible`. `open`: the relations of the files `file` includes, with the
/// file declaring each and whether it is extensible.
fn closed_heads(file: &str, statements: &[syntax::Statement], open: &std::collections::HashMap<&str, (&Path, bool)>) -> Result<(), Error> {
    let own: BTreeSet<&str> = declared_in(statements).map(|(n, _)| n).collect();
    for (item, line) in statements {
        let syntax::Item::Clause { heads, .. } = item else { continue };
        for h in heads.iter().filter(|h| !own.contains(h.rel.as_str())) {
            if let Some((home, false)) = open.get(h.rel.as_str()) {
                return Err(Error::Syntax(format!(
                    "{file}:{line}: `{}` is declared in {}, and not `extensible`: a file that includes it adds no rules or facts to it (decision 0263)",
                    h.rel,
                    home.display()
                )));
            }
        }
    }
    Ok(())
}

/// `closed_heads` for a program's own statements, over every library file it includes.
fn closed_program(files: &[(PathBuf, [u8; 32], std::sync::Arc<Vec<syntax::Statement>>)], own: &[syntax::Statement]) -> Result<(), Error> {
    let mut open = std::collections::HashMap::new();
    for (path, _, statements) in files {
        for (rel, ext) in declared_in(statements) {
            open.entry(rel).or_insert((path.as_path(), ext));
        }
    }
    closed_heads("the program", own, &open)
}

/// `library_modules`, with each module's files and its key; their images read from and kept in `store` (decision
/// 0264), and read from the shipped bundle first.
#[allow(clippy::type_complexity)]
fn library_modules_with_files(paths: &[PathBuf], files: Vec<(PathBuf, Vec<syntax::Statement>)>, store: Option<&crate::content_store::ContentStore>) -> Result<(Vec<lower::Module>, Vec<Vec<PathBuf>>, Vec<crate::content_store::HashKey>), Error> {
    let _span = crate::code_rule_dsl::logic::profile::span(|| format!("lower the library one module at a time ({} files)", paths.len()));
    let core = library().join("core.rr").canonicalize().ok();
    let at = |p: &Path| paths.iter().position(|q| q == p);
    let n = paths.len();
    // Each file's direct includes, by position: the library's file, else the one beside it (`#include`).
    let mut direct: Vec<BTreeSet<usize>> = Vec::with_capacity(n);
    let mut digests: Vec<[u8; 32]> = Vec::with_capacity(n);
    for (i, path) in paths.iter().enumerate() {
        let (statements, file) = read_shared(path)?;
        digests.push(file.sha256.unwrap_or_default());
        let mut mine: BTreeSet<usize> = BTreeSet::new();
        if let Some(c) = core.as_deref().and_then(at).filter(|&c| c != i) {
            mine.insert(c);
        }
        for (item, line) in statements.iter() {
            let syntax::Item::Include(name) = item else { continue };
            let found = [library().join(name), path.parent().map_or_else(|| PathBuf::from(name), |d| d.join(name))].into_iter().find_map(|p| p.canonicalize().ok());
            let Some(j) = found.as_deref().and_then(at) else {
                return Err(Error::Syntax(format!("{}:{line}: #include {name:?} is not among the run's library files", path.display())));
            };
            mine.insert(j);
        }
        mine.remove(&i);
        direct.push(mine);
    }
    // What each file reaches, itself not counted unless through a cycle.
    let mut reach: Vec<BTreeSet<usize>> = direct.clone();
    loop {
        let mut grew = false;
        for i in 0..n {
            let more: BTreeSet<usize> = reach[i].iter().flat_map(|&j| reach[j].iter().copied()).collect();
            let before = reach[i].len();
            reach[i].extend(more);
            grew |= reach[i].len() != before;
        }
        if !grew {
            break;
        }
    }
    // The modules: files that reach each other are one, in the order of their first file; their files in the list's
    // order, so the module's statements are as one lowering of the list reads them.
    let mut unit_of: Vec<usize> = vec![usize::MAX; n];
    let mut units: Vec<Vec<usize>> = Vec::new();
    for i in 0..n {
        if unit_of[i] != usize::MAX {
            continue;
        }
        let members: Vec<usize> = (i..n).filter(|&j| j == i || (reach[i].contains(&j) && reach[j].contains(&i))).collect();
        for &j in &members {
            unit_of[j] = units.len();
        }
        units.push(members);
    }
    let scopes: Vec<BTreeSet<usize>> = units.iter().enumerate().map(|(u, members)| members.iter().flat_map(|&i| reach[i].iter().map(|&j| unit_of[j])).filter(|&v| v != u).collect()).collect();
    for (u, scope) in scopes.iter().enumerate() {
        if let Some(&v) = scope.iter().find(|&&v| v > u) {
            return Err(Error::Syntax(format!("{} includes {}, listed after it", paths[units[u][0]].display(), paths[units[v][0]].display())));
        }
    }
    // Each module is kept in the store as its image (decision 0246 §6), by its key: the image's format, the engine's
    // build, its files' bytes and the keys of the modules in its scope. A module whose image the store holds is read
    // from it, not lowered.
    // Each file adds to no relation of the files it includes but their `extensible` ones (decision 0263); files that
    // include each other are one module, and write each other's relations.
    for i in 0..n {
        let mut open = std::collections::HashMap::new();
        for &j in reach[i].iter().filter(|&&j| unit_of[j] != unit_of[i]) {
            for (rel, ext) in declared_in(&files[j].1) {
                open.entry(rel).or_insert((paths[j].as_path(), ext));
            }
        }
        closed_heads(&paths[i].display().to_string(), &files[i].1, &open)?;
    }
    let mut statements: Vec<Option<Vec<syntax::Statement>>> = files.into_iter().map(|(_, s)| Some(s)).collect();
    let mut modules: Vec<lower::Module> = Vec::with_capacity(units.len());
    let mut keys: Vec<crate::content_store::HashKey> = Vec::with_capacity(units.len());
    for (u, members) in units.iter().enumerate() {
        let key = module_key(members.iter().map(|&i| &digests[i]), scopes[u].iter().map(|&v| &keys[v]));
        keys.push(key);
        if let Some((bytes, id)) = images::read(store, key)
            && let Ok(m) = module_image::decode(&bytes)
        {
            if let (Some(store), Some(id)) = (store, id) {
                renew_image(store, id);
            }
            crate::code_rule_dsl::prof::count("rr.module_image_read", 1);
            modules.push(m);
            continue;
        }
        let own: Vec<syntax::Statement> = members.iter().flat_map(|&i| statements[i].take().unwrap_or_default()).collect();
        let scope: Vec<&lower::Module> = scopes[u].iter().map(|&v| &modules[v]).collect();
        let m = lower::lower_module_in(own, &scope).map_err(|e| match e {
            Error::Syntax(m) => Error::Syntax(format!("{}: {m}", paths[members[0]].display())),
            e => e,
        })?;
        // Written by the store's writer, behind the run (decisions 0104, 0240 §1); dropped where its queue is full, as
        // a tape is: the next run lowers the module again.
        if let Ok(bytes) = module_image::encode(&m)
            && keep_image(store, "a module image", key, bytes)
        {
            crate::code_rule_dsl::prof::count("rr.module_image_written", 1);
        }
        modules.push(m);
    }
    Ok((modules, units.iter().map(|members| members.iter().map(|&i| paths[i].clone()).collect()).collect(), keys))
}

/// The store rule images are read from and kept in (`images::store`), unless `RERAIL_MODULE_IMAGES=off`.
pub(in crate::code_rule_dsl) fn image_store() -> Option<std::sync::Arc<crate::content_store::ContentStore>> {
    if cfg!(feature = "tape-cache") && std::env::var("RERAIL_MODULE_IMAGES").map_or(true, |v| v != "off") { images::store() } else { None }
}

/// How long a module's or program's image stays in the store after the last run that read or wrote it
/// (`keep_image`): collection keeps a source while a pin's lease runs, as it keeps the combined program's entry before
/// a run names it (`combined::remember`). Naming the images in the run's record is decision 0246 §6's.
const IMAGE_LEASE_SECS: i64 = 7 * 24 * 3600;

/// The owner of the images' pins.
const IMAGE_OWNER: &str = "rr images (decision 0246)";

fn image_lease() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64) + IMAGE_LEASE_SECS
}

/// Keeps `bytes`, an image, in `store` under alias `key`, pinned for its lease: written by the store's writer, behind
/// the run (decisions 0104, 0240 §1), and dropped where its queue is full, as a tape is (the next run lowers it
/// again). Whether it was queued.
fn keep_image(store: Option<&crate::content_store::ContentStore>, what: &'static str, key: crate::content_store::HashKey, bytes: Vec<u8>) -> bool {
    use crate::content_store::write_back::{Key, Syms};
    images::export(key, &bytes);
    let Some(store) = store else { return false };
    let n = bytes.len();
    store.write_back_unless_full(Syms::None, what, n, vec![(Key::of(&[what.as_bytes(), &key.sha256_bytes().unwrap_or_default()]), None)], Box::new(move |store| {
        let id = store.put_source(&bytes, &[key])?;
        store.pin_source(id, IMAGE_OWNER, image_lease())
    }))
}

/// The lease of image `id`, read by this run, renewed (`keep_image`).
fn renew_image(store: &crate::content_store::ContentStore, id: crate::content_store::SourceId) {
    use crate::content_store::write_back::Syms;
    store.write_back_unless_full(Syms::None, "an image's lease", 0, Vec::new(), Box::new(move |store| store.pin_source(id, IMAGE_OWNER, image_lease())));
}

/// Adds to `program`, which derives library relations `asked`, the linked library's part a request copies (decision
/// 0089 §3): the library relations reading `asked`, through any rule or native, with `asked` themselves; and the
/// relations their rules read that no library file declares (lowering's helpers), with their own. Their rules,
/// declarations, natives, facts and lattices, as the library has them.
fn with_cone(program: &mut Program, library: &Program, asked: &BTreeSet<String>, declared: &BTreeSet<String>) {
    use super::front::Literal;
    fn reads(r: &super::front::Rule) -> Vec<&str> {
        r.body.iter().filter_map(|l| match l {
            Literal::Atom(a) | Literal::Agg { atom: a, .. } => Some(a.rel.as_str()),
            _ => None,
        }).collect()
    }
    let mut cone: BTreeSet<String> = asked.clone();
    loop {
        let before = cone.len();
        for r in &library.rules {
            if !cone.contains(r.head.rel.as_str()) && reads(r).iter().any(|b| cone.contains(*b)) {
                cone.insert(r.head.rel.to_string());
            }
        }
        for (rel, n) in &library.natives {
            if !cone.contains(rel) && n.reads().iter().any(|b| cone.contains(*b)) {
                cone.insert(rel.clone());
            }
        }
        if cone.len() == before {
            break;
        }
    }
    let core = core_names();
    let mut take = cone;
    let mut work: Vec<String> = take.iter().cloned().collect();
    while let Some(rel) = work.pop() {
        let read: Vec<String> = library.rules.iter().filter(|r| r.head.rel.as_str() == rel).flat_map(|r| reads(r).into_iter().map(str::to_owned)).chain(library.natives.get(&rel).into_iter().flat_map(|n| n.reads().into_iter().map(str::to_owned))).collect();
        for b in read {
            if !declared.contains(&b) && !core.contains(&b) && !take.contains(&b) && (library.rules.iter().any(|r| r.head.rel.as_str() == b) || library.natives.contains_key(&b)) {
                take.insert(b.clone());
                work.push(b);
            }
        }
    }
    for r in library.rules.iter().filter(|r| take.contains(r.head.rel.as_str())) {
        if !program.rules.contains(r) {
            program.rules.push(r.clone());
        }
    }
    for (rel, cols) in library.decls.iter().filter(|(r, _)| take.contains(r.as_str())) {
        program.decls.entry(rel.clone()).or_insert_with(|| cols.clone());
    }
    for (rel, n) in library.natives.iter().filter(|(r, _)| take.contains(*r)) {
        program.natives.entry(rel.clone()).or_insert_with(|| n.clone());
    }
    for (rel, rows) in library.facts.iter().filter(|(r, _)| take.contains(*r)) {
        program.facts.entry(rel.clone()).or_insert_with(|| rows.clone());
    }
    for (rel, d) in library.lattice.iter().filter(|(r, _)| take.contains(*r)) {
        program.lattice.entry(rel.clone()).or_insert_with(|| d.clone());
    }
}

/// A lowered program's image (decision 0173): its rules, declarations, facts and `.about` rows.
fn encode_program(program: &Program) -> Result<Vec<u8>, String> {
    let mut b = super::program_image::Builder::of_program_less_maintenance(program)?;
    b.set_about(&super::program_image::about_of(program));
    b.finish_bytes()
}

/// The alias the library linked from modules `keys` is kept under.
fn linked_key(keys: &[crate::content_store::HashKey]) -> crate::content_store::HashKey {
    let mut bytes = format!("rerail linked library {}\0{}\0", super::program_image::FORMAT, crate::code_rule_dsl::build_ids::engine()).into_bytes();
    for k in keys {
        bytes.extend_from_slice(&k.sha256_bytes().unwrap_or_default());
    }
    crate::content_store::HashKey::sha256(&bytes)
}

/// The alias a program's image is kept under (`Text::lower_in`): its text and the keys of the modules in its scope.
fn program_key(src: &str, scope: &[crate::content_store::HashKey]) -> crate::content_store::HashKey {
    let digest = crate::content_store::HashKey::sha256(src.as_bytes()).sha256_bytes().unwrap_or_default();
    let mut bytes = format!("rerail program image {}\0{}\0", super::program_image::FORMAT, crate::code_rule_dsl::build_ids::engine()).into_bytes();
    bytes.extend_from_slice(&digest);
    bytes.push(0);
    for k in scope {
        bytes.extend_from_slice(&k.sha256_bytes().unwrap_or_default());
    }
    crate::content_store::HashKey::sha256(&bytes)
}

/// The alias a module's image is kept under (`library_modules`).
fn module_key<'k>(files: impl Iterator<Item = &'k [u8; 32]>, scope: impl Iterator<Item = &'k crate::content_store::HashKey>) -> crate::content_store::HashKey {
    let mut bytes = format!("rerail module image {}\0{}\0", super::program_image::FORMAT, crate::code_rule_dsl::build_ids::engine()).into_bytes();
    for d in files {
        bytes.extend_from_slice(d);
    }
    bytes.push(0);
    for k in scope {
        bytes.extend_from_slice(&k.sha256_bytes().unwrap_or_default());
    }
    crate::content_store::HashKey::sha256(&bytes)
}

/// A program text read once for what a run needs of it besides its rules (`plan::corpus::run_with_options`): its
/// statements, and the seed phase 0 derives its include graph from. Once every text's graph is derived
/// (`phase0::graphs`, one run for all of them), [`TextRead::resolved`] gives its library files, the relations it
/// declares and the phase it names. The whole-run cutoff, its record and the texts' digest take these; none reads a
/// text again (each parsed a pack's 278 texts again, about eight times a process).
pub struct TextRead {
    /// `None` for a text that includes nothing, or while phase 0 itself runs (`text_seed`).
    pub seed: Option<super::plan::corpus::phase0::Seed>,
    pub(in crate::code_rule_dsl) statements: Vec<syntax::Statement>,
}

/// What [`TextRead::resolved`] reads off a text: its library files (includes resolved, `core.rr` first unless the
/// text is in T's shape, and the engine's own library programs), the relations it and its includes declare, and the
/// phase it names itself (`.phase`).
pub struct TextNames {
    pub included: Vec<PathBuf>,
    pub names: Vec<String>,
    pub phase: Option<String>,
}

/// `src` parsed, and its seed (`TextRead`).
pub fn read_seed(src: &str) -> Result<TextRead, Error> {
    use super::plan::corpus::phase0;
    let statements = from_tape::statements(src, &nodes(src))?;
    let includes: Vec<(i64, String)> = statements
        .iter()
        .filter_map(|(item, _)| match item {
            syntax::Item::Include(name) => Some(name.clone()),
            _ => None,
        })
        .enumerate()
        .map(|(k, n)| (k as i64, n))
        .collect();
    let seed = (!includes.is_empty() && !phase0::active()).then(|| {
        let t_shape = statements.iter().any(|(item, _)| matches!(item, syntax::Item::Decl { name, .. } if name == "doc"));
        phase0::Seed::Text { t_shape, includes }
    });
    Ok(TextRead { seed, statements })
}

impl TextRead {
    /// The text's library files, names and phase, its includes resolved in the graph phase 0 derived for its seed.
    pub fn resolved(self) -> Result<TextNames, Error> {
        let (files, own) = if self.statements.iter().any(|(item, _)| matches!(item, syntax::Item::Include(_))) { resolve(self.statements)? } else { (Vec::new(), self.statements) };
        let (mut names, mut phase) = (Vec::new(), None);
        for (item, _) in files.iter().flat_map(|(_, st)| st.iter()).chain(own.iter()) {
            match item {
                syntax::Item::Decl { name, .. } => names.push(name.clone()),
                syntax::Item::Setting { key, text } if key == "phase" && phase.is_none() => phase = Some(text.trim().to_owned()),
                _ => {}
            }
        }
        names.sort();
        names.dedup();
        let mut included: Vec<PathBuf> = files.into_iter().map(|(p, _)| p).collect();
        // The engine's own library programs, which the runner adds to a run (decision 0076's components).
        included.extend(ENGINE_FILES.iter().filter_map(|f| library().join(f).canonicalize().ok()));
        Ok(TextNames { included, names, phase })
    }
}

impl TextNames {
    /// A program already read: the relations it declares and the phase it names, and the engine's library programs
    /// (it includes nothing of its own: its libraries are in it), as a print of it reads (decision 0249).
    pub fn of_program(prog: &Program) -> TextNames {
        let mut names: Vec<String> = prog.decls.keys().map(|n| n.to_string()).collect();
        names.sort();
        names.dedup();
        let phase = prog.settings.iter().find(|(k, _)| k == "phase").map(|(_, v)| v.trim().to_owned());
        let included = ENGINE_FILES.iter().filter_map(|f| library().join(f).canonicalize().ok()).collect();
        TextNames { included, names, phase }
    }
}

/// The library files a program's text reads (`TextNames::included`): what a record of the run must check besides
/// the text (phase 0, t23). For a text read alone; a run reads its programs once (`read_seed`).
pub fn included_files(src: &str) -> Result<Vec<PathBuf>, Error> {
    Ok(read_seed(src)?.resolved()?.included)
}

/// The relations a program's text declares, its includes' with its own, and the phase it names itself
/// (`TextNames`): for a text read alone (a phase program found by name, `corpus::phases::phase_files`).
pub fn declared_names(src: &str) -> Result<(Vec<String>, Option<String>), Error> {
    let t = read_seed(src)?.resolved()?;
    Ok((t.names, t.phase))
}

/// `statements`' `#include`s resolved in the library: its files (`core.rr` first unless the statements are in T's
/// shape, each once, includes first) and the statements without the includes.
///
/// Phase 0 resolves them (`plan::corpus::phase0`): `bootstrap.rr`'s run loads each file as a document, and the
/// files are read here off their stored tapes, in the graph's order.
fn resolve(statements: Vec<syntax::Statement>) -> Result<(Files, Vec<syntax::Statement>), Error> {
    let (files, own) = resolve_shared(statements)?;
    Ok((files.into_iter().map(|(path, _, statements)| (path, statements.iter().filter(|(item, _)| !matches!(item, syntax::Item::Include(_))).cloned().collect())).collect(), own))
}

/// `resolve`, each file's statements as `read` keeps them (its includes still in them) with its bytes' digest.
type SharedFiles = Vec<(PathBuf, [u8; 32], std::sync::Arc<Vec<syntax::Statement>>)>;

fn resolve_shared(statements: Vec<syntax::Statement>) -> Result<(SharedFiles, Vec<syntax::Statement>), Error> {
    use super::plan::corpus::phase0;
    if phase0::active() {
        let (files, own) = builtin_resolve(statements)?;
        let mut out = Vec::new();
        for (path, statements) in files {
            let (_, file) = read_shared(&path)?;
            out.push((path, file.sha256.unwrap_or_default(), std::sync::Arc::new(statements)));
        }
        return Ok((out, own));
    }
    let (includes, own): (Vec<_>, Vec<_>) = statements.into_iter().partition(|(item, _)| matches!(item, syntax::Item::Include(_)));
    let t_shape = own.iter().any(|(item, _)| matches!(item, syntax::Item::Decl { name, .. } if name == "doc"));
    let names: Vec<(i64, String, usize)> = includes
        .iter()
        .enumerate()
        .filter_map(|(k, (item, line))| match item {
            syntax::Item::Include(name) => Some((k as i64, name.clone(), *line)),
            _ => None,
        })
        .collect();
    let g = phase0::graph(&phase0::Seed::Text { t_shape, includes: names.iter().map(|(k, n, _)| (*k, n.clone())).collect() })?;
    if let Some((_, at, name)) = g.missing.iter().find(|(f, _, _)| f == phase0::TEXT) {
        let line = names.iter().find(|(k, _, _)| k == at).map_or(0, |(_, _, l)| *l);
        return Err(Error::Syntax(format!("line {line}: #include {name:?}: no such file in the library")));
    }
    let mut order = Vec::new();
    g.order(phase0::TEXT, &mut order)?;
    let files = order
        .into_iter()
        .map(|p| {
            let (statements, file) = read_shared(&p)?;
            Ok((p, file.sha256.unwrap_or_default(), statements))
        })
        .collect::<Result<_, Error>>()?;
    Ok((files, own))
}

/// Each module's statements, without its includes (the graph placed them), read off its stored tape.

/// The seed phase 0 starts from for a program text: whether it is in T's shape, and the names it includes. `None`
/// for a text that includes nothing, or while phase 0 itself runs.
pub fn text_seed(src: &str) -> Result<Option<super::plan::corpus::phase0::Seed>, Error> {
    use super::plan::corpus::phase0;
    // An include is the lexer's `#include` (`syntax::Tok::Hash`): a text without those letters includes nothing.
    // A printed program, its includes resolved, is not read for it; its parse errors are its parse's to report.
    if phase0::active() || !src.contains("#include") {
        return Ok(None);
    }
    // Read once a run: the run hands its texts' graph digests to its combines (`corpus::run_handed`).
    read_text_seed(src)
}

fn read_text_seed(src: &str) -> Result<Option<super::plan::corpus::phase0::Seed>, Error> {
    use super::plan::corpus::phase0;
    // The letters may stand in a string, a pattern or a comment, which are not directives: the lexer's tokens say,
    // before another syntax tree is made for a program without includes.
    let lexed = syntax::lex(src);
    if lexed.error.is_none() && !lexed.toks.iter().any(|(t,_,_)| matches!(t, syntax::Tok::Hash(_))) {
        return Ok(None);
    }
    let statements = from_tape::statements(src, &nodes(src))?;
    let names: Vec<(i64, String)> = statements
        .iter()
        .filter_map(|(item, _)| match item {
            syntax::Item::Include(name) => Some(name.clone()),
            _ => None,
        })
        .enumerate()
        .map(|(k, n)| (k as i64, n))
        .collect();
    if names.is_empty() || phase0::active() {
        return Ok(None);
    }
    let t_shape = statements.iter().any(|(item, _)| matches!(item, syntax::Item::Decl { name, .. } if name == "doc"));
    Ok(Some(phase0::Seed::Text { t_shape, includes: names }))
}

/// The program's module graph's digest (`phase0::Graph::digest`): what its text is a function of besides itself.
/// `None` for a text that includes nothing.
pub(in crate::code_rule_dsl) fn graph_digest(src: &str) -> Result<Option<[u8; 32]>, Error> {
    match text_seed(src)? {
        Some(seed) => Ok(Some(super::plan::corpus::phase0::graph(&seed)?.digest())),
        None => Ok(None),
    }
}

/// The resolution phase 0's own program (`bootstrap.rr`) is read with: the library, then beside the includer.
fn builtin_resolve(statements: Vec<syntax::Statement>) -> Result<(Files, Vec<syntax::Statement>), Error> {
    let (includes, own): (Vec<_>, Vec<_>) = statements.into_iter().partition(|(item, _)| matches!(item, syntax::Item::Include(_)));
    let core = library().join("core.rr").canonicalize().map_err(|e| Error::Syntax(format!("core.rr: {e}")))?;
    let t_shape = own.iter().any(|(item, _)| matches!(item, syntax::Item::Decl { name, .. } if name == "doc"));
    let (mut out, mut read_files, mut seen) = (Vec::new(), Vec::new(), Vec::new());
    if t_shape {
        seen.push(core);
    } else {
        visit(&core, &mut out, &mut read_files, &mut seen, false)?;
    }
    for (item, line) in includes {
        let syntax::Item::Include(name) = item else { unreachable!("an include") };
        let path = library().join(&name);
        if !path.exists() {
            return Err(Error::Syntax(format!("line {line}: #include {name:?}: no such file in the library")));
        }
        visit(&path, &mut out, &mut read_files, &mut seen, true)?;
    }
    Ok((out, own))
}


/// The first error a `.rr` file's parse gives, as running it would report it: its line and message
/// (`syntax_error/3`). `None` when it parses.
pub fn syntax_error(src: &str) -> Option<(usize, String)> {
    let error = match syntax::Parser::new(src).and_then(|mut parser| parser.statements()) {
        Ok(_) => return None,
        Err(error) => error,
    };
    let mut parser = syntax::Parser::taped(src);
    let _ = parser.statements();
    let at = parser.into_tape().1.map_or(0, |(at, _)| at);
    let rerail_rr_syntax::Error::Syntax(message) = error;
    Some((syntax::line_of(src, at), message))
}

/// A `.rr` string literal's value, as the rule engine reads it (`"a\nb"` is `a`, newline, `b`); `None` for text
/// that is no whole string literal.
pub(in crate::code_rule_dsl) fn string_value(text: &str) -> Option<String> {
    match syntax::tokens(text).ok()?.as_slice() {
        [(syntax::Tok::Str(value), 0, end)] if *end == text.len() => Some(value.to_string()),
        _ => None,
    }
}

/// A backtick literal's body, without its fences (and, for a fence of two or more, the one space inside each).
pub(in crate::code_rule_dsl) fn code_body(text: &str) -> &str {
    let n = text.bytes().take_while(|&b| b == b'`').count();
    let body = text.get(n..text.len().saturating_sub(n)).unwrap_or("");
    if n > 1 { body.strip_prefix(' ').unwrap_or(body).strip_suffix(' ').unwrap_or(body) } else { body }
}

/// The compiler's named policy headers and settings, without reparsing rule source in a host adapter.
pub fn metadata(program: &Program) -> Result<serde_json::Value, Error> {
    use syntax::Tok;
    let mut policies = program.policies.clone();
    for (_, text) in program.settings.iter().filter(|(key, _)| key == "policy") {
        let tokens = syntax::tokens(text)?;
        let Some((Tok::Str(id), _, _)) = tokens.first() else {
            return Err(Error::Syntax("policy metadata requires a quoted identifier".to_owned()));
        };
        let revision = tokens.windows(2).find_map(|pair| match (&pair[0].0, &pair[1].0) {
            (Tok::Id(key), Tok::Num(n)) if *key == "revision" => Some(*n),
            _ => None,
        });
        policies.push(serde_json::json!({"id": id, "revision": revision}));
    }
    Ok(serde_json::json!({"policies": policies, "settings": program.settings}))
}

/// A lowered program run over files a caller gives (a test's), not found by walking a repository: its
/// `.scope` filter on findings is taken out, as `.scope given` leaves it out when lowering.
pub fn files_given(program: &mut Program) {
    let filter = ["rr_scope_drop", "rr_scope_in", "rr_report_in"];
    program.rules.retain(|r| !filter.contains(&r.head.rel.as_str()));
    for r in &mut program.rules {
        r.body.retain(|l| !l.atom().is_some_and(|a| filter.contains(&a.rel.as_str())));
    }
    filter.iter().for_each(|f| {
        program.decls.remove(*f);
    });
    program.settings.push(("scope".to_owned(), "given".to_owned()));
    program.doc_scopes = None;
}

/// A program's `.rr` file, with what it includes.
pub fn parse_file(path: &Path) -> Result<Program, Error> {
    read_file(path).map(|(program, _)| program)
}

/// A library file a program read: decision 0044's seeded fact `library_file(path, hash)`. `sha256`
/// is of the file's bytes, or `None` where an `#include` looked and found no file (a file
/// appearing there changes what the program reads).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryFile {
    pub path: PathBuf,
    pub sha256: Option<[u8; 32]>,
}

impl LibraryFile {
    /// The file as it is now, by phase 0's observer: one `stat`, and a read only when it changed.
    fn now(path: &Path) -> Self {
        Self { path: path.to_owned(), sha256: super::plan::corpus::observe(|o| o.digest(path)) }
    }

    /// The seeded fact, for a file that exists.
    // Read by the runner's record when it seeds the store (0044 step 2).
    #[allow(dead_code)]
    pub fn fact(&self) -> Option<String> {
        let hash = self.sha256?;
        let hex = hash.iter().map(|b| format!("{b:02x}")).collect::<String>();
        Some(format!("library_file({}, \"sha256:{hex}\").", quote(&self.path.to_string_lossy())))
    }
}

/// The program of a `.rr` file and the library files it read or probed, in reading order (core.rr, then
/// each file before what it includes).
///
/// The lowered program is a derivation of those files (0044 §4): it is kept for the process and
/// reused while every file it read hashes the same, so only an edit to a file it read (or a
/// file appearing where an include looked) reads and lowers it again.
/// The relations `core.rr` declares for libraries to derive: the engine reads them (decision 0055's embeds).
/// And the documents they load (t21 E2).
pub const LIBRARY_DERIVED: [&str; 3] = ["embed", "embed_value", "load"];
/// The library's programs the engine adds to a run itself (decision 0076's import components).
const ENGINE_FILES: [&str; 1] = ["components.rr"];

pub fn read_file(path: &Path) -> Result<(Program, Vec<LibraryFile>), Error> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static PROGRAMS: OnceLock<Mutex<HashMap<PathBuf, (Vec<LibraryFile>, Program)>>> = OnceLock::new();
    let key = path.canonicalize().map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    let cache = PROGRAMS.get_or_init(Default::default);
    if let Some((files, program)) = cache.lock().expect("the program cache").get(&key)
        && files.iter().all(|file| LibraryFile::now(&file.path) == *file)
    {
        crate::code_rule_dsl::prof::count("rr.program_reused", 1);
        return Ok((program.clone(), files.clone()));
    }
    // Decision 0275: rederived until a key hits. The file's sha (by its `stat` while that holds), then the entry kept
    // under its path and sha, taken while every file it read still reads as recorded (by `stat`, then sha).
    let kept = kept_program(&key);
    if let Some((_, _, Some((program, files)))) = &kept {
        crate::code_rule_dsl::prof::count("rr.program_stored", 1);
        files.iter().for_each(noted);
        cache.lock().expect("the program cache").insert(key.clone(), (files.clone(), program.clone()));
        return Ok((program.clone(), files.clone()));
    }
    // The kept statements alone copied from the process's (`read_shared`): each policy copied its libraries whole,
    // to drop most of them (`unreached_dropped`).
    let (shared, files) = program_shared(path)?;
    let own = path.canonicalize().unwrap_or_else(|_| path.to_owned());
    let statements = reached_statements(&own, &shared.iter().collect::<Vec<_>>());
    let program = lower_files_dropping(path, statements, false)?;
    // Kept behind the run (the store's writer), with what it read: the next run's check above.
    if let Some((entry, Some(store), None)) = kept {
        let (read, program) = (files.clone(), program.clone());
        store.write_back_syms(crate::content_store::write_back::Syms::None, "a rule file's program", 0, vec![(super::plan::corpus::combined::queued(&entry), None)], Box::new(move |store| super::plan::corpus::combined::remember(store, &entry, &read, &program, &[]).map(drop)));
        super::plan::corpus::combined::read_before_the_run(entry);
    }
    cache.lock().expect("the program cache").insert(key, (files.clone(), program.clone()));
    Ok((program, files))
}

/// `read_text(src)?.header()`, rederived until a key hits (decision 0275): the header kept under the text's bytes,
/// taken while every library file its includes read still reads as recorded (by `stat`, then sha). A batch reads
/// each policy's header before its run (`tools::policy_groups`): 44 policies parsed, checked and lowered again each
/// run, over the same libraries.
pub fn text_header(src: &str) -> Result<Program, Error> {
    use super::plan::corpus::combined;
    let (key, store, found) = kept_header(src);
    if let Some((program, files)) = found {
        crate::code_rule_dsl::prof::count("rr.header_stored", 1);
        files.iter().for_each(noted);
        combined::read_before_the_run(key);
        return Ok(program);
    }
    let text = read_text(src)?;
    let read: Vec<LibraryFile> = match &text.0 {
        Read::Plain(_) => Vec::new(),
        Read::Modular { files, .. } => files.iter().map(|(path, digest, _)| LibraryFile { path: path.clone(), sha256: Some(*digest) }).collect(),
    };
    let program = text.header()?;
    if let Some(store) = store {
        let kept = program.clone();
        store.write_back_syms(crate::content_store::write_back::Syms::None, "a text's header", 0, vec![(combined::queued(&key), None)], Box::new(move |store| combined::remember(store, &key, &read, &kept, &[]).map(drop)));
        combined::read_before_the_run(key);
    }
    Ok(program)
}

/// The entry of text `src`'s header: its key, the store it is kept in, and the header with its read record when the
/// store keeps one that still holds.
#[allow(clippy::type_complexity)]
fn kept_header(src: &str) -> ([u8; 32], Option<std::sync::Arc<crate::content_store::ContentStore>>, Option<(Program, Vec<LibraryFile>)>) {
    use super::plan::corpus::{combined, phases};
    let key = combined::text_header_key(src);
    let store = (cfg!(feature = "tape-cache") && !super::plan::corpus::phase0::active()).then(phases::reading_store).flatten();
    let found = store.as_ref().and_then(|store| {
        let _held = store.hold_collection().ok()?;
        store.settle(&combined::queued(&key));
        combined::lookup_read(store, &key, &Default::default()).ok().flatten()
    });
    (key, store, found)
}

/// The header the store keeps for text `src`, when it still holds (`kept_header`): for the tests of decision 0275.
#[cfg(any(test, feature = "test-hooks"))]
pub fn kept_header_found(src: &str) -> Option<Program> {
    kept_header(src).2.map(|(program, _)| program)
}

/// The program and read record the store keeps for the rule file at `path` (canonical), when they still hold
/// (`kept_program`): for the tests of decision 0275.
#[cfg(any(test, feature = "test-hooks"))]
pub fn kept_program_found(path: &Path) -> Option<(Program, Vec<LibraryFile>)> {
    kept_program(path).and_then(|(_, _, found)| found)
}

/// The entry of the rule file at `path` (canonical): its key, the store it is kept in, and its program and read
/// record when the store keeps one that still holds (`combined::lookup_read`). `None` where there is no file to key
/// by, or the program is phase 0's own (its builtin reader).
#[allow(clippy::type_complexity)]
fn kept_program(path: &Path) -> Option<([u8; 32], Option<std::sync::Arc<crate::content_store::ContentStore>>, Option<(Program, Vec<LibraryFile>)>)> {
    use super::plan::corpus::{combined, phases};
    if !cfg!(feature = "tape-cache") || super::plan::corpus::phase0::active() {
        return None;
    }
    let sha = super::plan::corpus::observe(|o| o.digest(path))?;
    let key = combined::rule_file_key(path, &sha);
    let Some(store) = phases::reading_store() else { return Some((key, None, None)) };
    let found = {
        // Read while collection is held off, as a combined program's is (`combine_stored`).
        let _held = store.hold_collection().ok()?;
        store.settle(&combined::queued(&key));
        combined::lookup_read(&store, &key, &Default::default()).ok().flatten()
    };
    if found.is_some() {
        combined::read_before_the_run(key);
    }
    Some((key, Some(store), found))
}

/// A file's program from its modules' statements (`program`): its names checked, its own rules grounded, and lowered,
/// as `read_file` reads it. Testing mode (`rr::testing`, decision 0130) gives the file's statements with a test's
/// spliced in.
pub fn lower_files(path: &Path, statements: Files) -> Result<Program, Error> {
    lower_files_dropping(path, statements, true)
}

/// `lower_files`, with the unreached clauses of included files dropped first (`drop`) or lowered too.
pub fn lower_files_dropping(path: &Path, statements: Files, drop: bool) -> Result<Program, Error> {
    let key = path.canonicalize().map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    check_names(&statements, false)?;
    // The file's own rules grounded (`refuse_ungrounded`), its includes' `fix` relations given.
    let own_path = path.canonicalize().unwrap_or_else(|_| path.to_owned());
    let given: BTreeSet<String> = statements.iter().filter(|(p, _)| *p != own_path).flat_map(|(_, s)| check::fix_relations(s)).collect();
    for (_, own) in statements.iter().filter(|(p, _)| *p == own_path) {
        refuse_ungrounded(own, &given)?;
    }
    // The included files' clauses nothing in the program reaches: dropped before they are checked and lowered.
    let statements = if drop { unreached_dropped(&own_path, statements) } else { statements };
    token_reads::refuse(&statements)?;
    // The relations an included file declares are the library's: canonical, one copy however many policies of a
    // combined run include it (decision 0091). Printed as `.global library …`, so a program's text keeps that.
    let core = library().join("core.rr").canonicalize().map_err(|e| Error::Syntax(format!("core.rr: {e}")))?;
    let library: Vec<String> = statements
        .iter()
        .filter(|(file, _)| *file != core && *file != key)
        .flat_map(|(_, statements)| statements)
        .filter_map(|(item, _)| if let syntax::Item::Decl { name, .. } = item { Some(name.clone()) } else { None })
        .collect();
    let mut program = lower::lower(statements.into_iter().flat_map(|(_, statements)| statements).collect())?;
    if !library.is_empty() {
        program.settings.push((super::front::GLOBAL.to_owned(), format!("{} {}", super::front::LIBRARY, library.join(", "))));
    }
    Ok(program)
}

/// `statements` less the included files' clauses that nothing in the program reaches. Of the 44 data-layer
/// policies' rules, 76% were lowered (with the libraries' whole) and then pruned (`front::prune`): reading a policy
/// was the largest part of a batch's setup (`lower_files`, 16% of the main thread). Kept whole: the file itself,
/// `core.rr` and the engine's files (lowering reads their relations by name), and every item but a clause. A clause
/// of another file is kept when one of its heads is reached: named by the file itself (each identifier of each of
/// its items, so none is missed), by another file's output, violation or setting, by the engine (`LIBRARY_DERIVED`),
/// by its own body when that makes a request (`asks`), or read by a kept clause. Every clause for a relation is kept with any, so a library relation is the same in each
/// program that reaches it (decision 0091). `front::prune` of the program lowered from these is the same as of the whole
/// (`dropping_unreached_clauses_prunes_to_the_same_program`).
pub fn unreached_dropped(own: &Path, statements: Files) -> Files {
    reached_statements(own, &statements.iter().map(|(p, s)| (p, s.iter().collect())).collect::<Vec<_>>())
}

/// Whether a read of `name` makes a request when lowered (`lower`'s demands): the file system's relations (`WORLD`), a
/// guest's (`parsed`, `sub_tape`) and a tape operation's (`parse_of`, `splice_of`, and `@parse`, `@splice`, which
/// lower to them). What is asked for is made for every reader, not the asking clause alone.
fn asks(name: &str) -> bool {
    WORLD.iter().any(|(r, _)| *r == name) || matches!(name, "parsed" | "sub_tape" | "parse_of" | "splice_of" | "parse" | "splice")
}

/// `unreached_dropped` of statements held elsewhere (`read_shared`'s): only the kept are copied.
fn reached_statements(own: &Path, statements: &[(&PathBuf, Vec<&syntax::Statement>)]) -> Files {
    fn idents(text: &str, out: &mut BTreeSet<String>) {
        out.extend(text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).filter(|w| w.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')).map(str::to_owned));
    }
    fn term(t: &syntax::Term, out: &mut BTreeSet<String>) {
        match t {
            syntax::Term::Call { name, args, .. } => {
                out.insert(name.clone());
                args.iter().for_each(|a| term(a, out));
            }
            syntax::Term::Bin { l, r, .. } => {
                term(l, out);
                term(r, out);
            }
            syntax::Term::Agg { over, by, body, .. } => {
                over.iter().for_each(|o| term(o, out));
                by.iter().for_each(|b| term(b, out));
                lits(body, out);
            }
            syntax::Term::Named(_, t) => term(t, out),
            // A relation may be an argument (`nearest(n, p, block_node)`, a set of nodes): every name in a term may
            // be one.
            syntax::Term::Json(text) | syntax::Term::Str(text) | syntax::Term::Tick(text) => idents(text, out),
            syntax::Term::Var(v) => {
                out.insert(v.clone());
            }
            _ => {}
        }
    }
    fn lits(body: &[syntax::Lit], out: &mut BTreeSet<String>) {
        for l in body {
            match l {
                syntax::Lit::Atom { rel, args, .. } => {
                    out.insert(rel.clone());
                    args.iter().for_each(|a| term(a, out));
                }
                syntax::Lit::Cmp { l, r, .. } => {
                    term(l, out);
                    term(r, out);
                }
                syntax::Lit::Pattern { subject, .. } => term(subject, out),
                syntax::Lit::Group { alts, .. } => alts.iter().for_each(|a| lits(a, out)),
            }
        }
    }
    let whole: Vec<PathBuf> = ["core.rr"].iter().chain(ENGINE_FILES.iter()).filter_map(|f| library().join(f).canonicalize().ok()).chain(std::iter::once(own.to_owned())).collect();
    // The relations a library derives for the engine to read (`LIBRARY_DERIVED`: embeds, loads) are reached: no rule
    // reads them, and the corpus run makes them outputs (`corpus::run_observed`'s requests). Dropped, a library's
    // `load(…)` asked for no document, and no import was resolved.
    let mut reached: BTreeSet<String> = LIBRARY_DERIVED.iter().map(|r| (*r).to_owned()).collect();
    // Each other file's clauses by head: (file, statement) indices.
    let mut by_head: std::collections::HashMap<&str, Vec<(usize, usize)>> = std::collections::HashMap::new();
    for (f, (file, items)) in statements.iter().enumerate() {
        let kept_whole = whole.contains(file);
        for (s, (item, _)) in items.iter().copied().enumerate() {
            match item {
                syntax::Item::Clause { heads, body, edits } if !kept_whole => {
                    heads.iter().for_each(|h| by_head.entry(h.rel.as_str()).or_default().push((f, s)));
                    // A clause whose reads make requests (`asks`) is reached though nothing reads its heads: the engine
                    // answers its request for every reader (`python/modules.rr`'s `py_listed` lists the directories the
                    // other rules read entries of).
                    let mut read = BTreeSet::new();
                    lits(body, &mut read);
                    edits.iter().for_each(|e| term(e, &mut read));
                    if read.iter().any(|r| asks(r)) {
                        reached.extend(heads.iter().map(|h| h.rel.clone()));
                    }
                }
                syntax::Item::Clause { heads, body, edits } => {
                    heads.iter().for_each(|h| {
                        reached.insert(h.rel.clone());
                        h.args.iter().for_each(|a| term(a, &mut reached));
                    });
                    lits(body, &mut reached);
                    edits.iter().for_each(|e| term(e, &mut reached));
                }
                // What another file declares reads nothing (`.decl`, `.input`, `.stored`, `.functor`, `.type`, `.edit`):
                // its relations are reached only when read. Its outputs, violations and settings are read.
                syntax::Item::Decl { .. } | syntax::Item::Functor { .. } | syntax::Item::Type { .. } | syntax::Item::Edit { .. } | syntax::Item::Lattice { .. } | syntax::Item::Include(_) | syntax::Item::Io { output: false, .. } if !kept_whole => {}
                other => idents(&format!("{other:?}"), &mut reached),
            }
        }
    }
    let mut keep: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut todo: Vec<String> = reached.iter().cloned().collect();
    while let Some(rel) = todo.pop() {
        for &(f, s) in by_head.get(rel.as_str()).into_iter().flatten() {
            if !keep.insert((f, s)) {
                continue;
            }
            let syntax::Item::Clause { heads, body, edits } = &statements[f].1[s].0 else { continue };
            let mut more = BTreeSet::new();
            heads.iter().for_each(|h| {
                more.insert(h.rel.clone());
                h.args.iter().for_each(|a| term(a, &mut more));
            });
            lits(body, &mut more);
            edits.iter().for_each(|e| term(e, &mut more));
            todo.extend(more.into_iter().filter(|r| reached.insert(r.clone())));
        }
    }
    statements
        .iter()
        .enumerate()
        .map(|(f, (file, items))| {
            let kept_whole = whole.contains(*file);
            let items = items.iter().copied().enumerate().filter(|(s, (item, _))| kept_whole || !matches!(item, syntax::Item::Clause { .. }) || keep.contains(&(f, *s))).map(|(_, i)| i.clone()).collect();
            ((*file).clone(), items)
        })
        .collect()
}

/// The names a program's files may not declare or derive: `core.rr`'s, but in `core.rr` (decision 0141), and names
/// starting `rr_`, but in the engine's own library programs.
fn check_names(statements: &Files, core_unlisted: bool) -> Result<(), Error> {
    // `core.rr`'s relations are the engine's (decision 0141): no other file declares or derives one.
    let core = library().join("core.rr").canonicalize().map_err(|e| Error::Syntax(format!("core.rr: {e}")))?;
    // From `core.rr` among the files, else (`core_unlisted`: the program reads it) as `core_names` reads it.
    let listed = statements.iter().any(|(file, _)| *file == core);
    let engine: std::collections::BTreeSet<&String> = statements
        .iter()
        .filter(|(file, _)| *file == core)
        .flat_map(|(_, statements)| statements)
        .filter_map(|(item, _)| if let syntax::Item::Decl { name, .. } = item { Some(name) } else { None })
        .chain(core_names().iter().filter(|_| core_unlisted && !listed))
        .filter(|name| !LIBRARY_DERIVED.contains(&name.as_str()))
        .collect();
    for (file, statements) in statements.iter().filter(|(file, _)| *file != core) {
        for (item, line) in statements {
            let names: Vec<&String> = match item {
                syntax::Item::Decl { name, .. } => vec![name],
                syntax::Item::Clause { heads, .. } => heads.iter().map(|h| &h.rel).collect(),
                _ => Vec::new(),
            };
            if let Some(name) = names.into_iter().find(|n| engine.contains(n)) {
                return Err(Error::Syntax(format!("{}:{line}: {name} is core.rr's: a program reads it and cannot declare or derive it", file.display())));
            }
        }
    }
    // Names starting `rr_` are the engine's (`front::RESERVED`): its helpers, axes and places. The engine's own
    // library programs (`ENGINE_FILES`, which the runner adds; no rule includes them) may use them.
    let own: Vec<PathBuf> = ENGINE_FILES.iter().filter_map(|f| library().join(f).canonicalize().ok()).collect();
    for (file, statements) in statements.iter().filter(|(file, _)| !own.contains(file)) {
        for (item, line) in statements {
            let names: Vec<&String> = match item {
                syntax::Item::Decl { name, .. } | syntax::Item::Violation { name, .. } => vec![name],
                syntax::Item::Clause { heads, .. } => heads.iter().map(|h| &h.rel).collect(),
                _ => Vec::new(),
            };
            if let Some(name) = names.into_iter().find(|n| n.starts_with(super::front::RESERVED)) {
                return Err(Error::Syntax(format!("{}:{line}: {name}: names starting `{}` are the engine's", file.display(), super::front::RESERVED)));
            }
        }
    }
    Ok(())
}

thread_local! {
    /// The library files read or probed while `recording` runs on this thread.
    static READS: std::cell::RefCell<Option<Vec<LibraryFile>>> = const { std::cell::RefCell::new(None) };
}

/// `f`'s result and every library file it read or probed, misses included, each once in reading order: what a
/// program read from the library, so a later run can tell whether reading it again would read the same (decision
/// 0094 §6: the loader records what it read).
pub(in crate::code_rule_dsl) fn recording<T>(f: impl FnOnce() -> T) -> (T, Vec<LibraryFile>) {
    let outer = READS.with(|r| r.borrow_mut().replace(Vec::new()));
    let out = f();
    let read = READS.with(|r| std::mem::replace(&mut *r.borrow_mut(), outer)).unwrap_or_default();
    // An enclosing recording sees what this one read too.
    READS.with(|r| {
        if let Some(outer) = r.borrow_mut().as_mut() {
            outer.extend(read.iter().cloned());
        }
    });
    let mut seen = std::collections::HashSet::new();
    (out, read.into_iter().filter(|f| seen.insert(f.path.clone())).collect())
}

/// `f` run on another thread for a caller inside `recording`: its result, and the library files it read, which
/// `replay` notes on the caller's thread, in the order the caller's own reading would have noted them.
pub(in crate::code_rule_dsl) fn recorded_on_worker<T>(f: impl FnOnce() -> T) -> (T, Vec<LibraryFile>) {
    let outer = READS.with(|r| r.borrow_mut().replace(Vec::new()));
    let out = f();
    let read = READS.with(|r| std::mem::replace(&mut *r.borrow_mut(), outer)).unwrap_or_default();
    (out, read)
}

/// The files a worker read (`recorded_on_worker`), noted on this thread as if it had read them.
pub(in crate::code_rule_dsl) fn replay(files: Vec<LibraryFile>) {
    READS.with(|r| {
        if let Some(read) = r.borrow_mut().as_mut() {
            read.extend(files);
        }
    });
}

fn noted(file: &LibraryFile) {
    READS.with(|r| {
        if let Some(read) = r.borrow_mut().as_mut() {
            read.push(file.clone());
        }
    });
}

impl LibraryFile {
    /// Whether the file still reads as recorded: the same bytes, or still no file.
    pub(in crate::code_rule_dsl) fn unchanged(&self) -> bool {
        Self::now(&self.path) == *self
    }
}

pub fn read(path: &Path) -> Result<(Vec<syntax::Statement>, LibraryFile), Error> {
    read_shared(path).map(|(statements, file)| ((*statements).clone(), file))
}

/// A library file's statements as kept for the process, and the file as read.
fn read_shared(path: &Path) -> Result<(std::sync::Arc<Vec<syntax::Statement>>, LibraryFile), Error> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    // Each file's statements, kept for the process by its path and bytes' digest: programs that include one library
    // (the Python pack's files) parse it once, not once per program. Phase 0's observer gives the digest by a `stat`
    // while the file is unchanged, so a kept file is not read again. Held shared: a reader copies what it keeps.
    static PARSED: OnceLock<Mutex<HashMap<PathBuf, ([u8; 32], Arc<Vec<syntax::Statement>>)>>> = OnceLock::new();
    let cache = PARSED.get_or_init(Default::default);
    if let Some(digest) = super::plan::corpus::observe(|o| o.digest(path))
        && let Some((kept, statements)) = cache.lock().expect("the parsed files").get(path)
        && *kept == digest
    {
        crate::code_rule_dsl::prof::count("rr.file_reused", 1);
        let file = LibraryFile { path: path.to_owned(), sha256: Some(digest) };
        noted(&file);
        return Ok((Arc::clone(statements), file));
    }
    let (bytes, digest) = super::plan::corpus::observe(|o| o.read(path)).ok_or_else(|| Error::Syntax(format!("{}: cannot read the file", path.display())))?;
    let file = LibraryFile { path: path.to_owned(), sha256: Some(digest) };
    noted(&file);
    let src = String::from_utf8(bytes).map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    let statements = Arc::new(stored_statements(path, src).map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?);
    cache.lock().expect("the parsed files").insert(path.to_owned(), (digest, Arc::clone(&statements)));
    Ok((statements, file))
}

/// A rule file's statements, read off the lean parser's tape (decision 0275). Its tape was the content store's, as a
/// corpus file's is (decision 0062): a lookup and a decode of facts cost more than the lean parse.
/// `src` (a document named `name`) as a program, read off its tape as a rule file's is (`stored_statements`): a
/// prepared program's document (decision 0074).
pub(in crate::code_rule_dsl) fn parse_stored(name: &str, src: String) -> Result<Program, Error> {
    lower::lower(stored_statements(Path::new(name), src)?)
}

fn stored_statements(_path: &Path, src: String) -> Result<Vec<syntax::Statement>, Error> {
    from_tape::statements(&src, &nodes(&src))
}

pub type Files = Vec<(PathBuf, Vec<syntax::Statement>)>;

/// `path` and what it includes, each once (`seen`), includes first, added to `out`; every library file read or
/// probed, to `read_files`. An `included` file's `.output`s are left out, as are those of what it includes.
fn visit(path: &Path, out: &mut Files, read_files: &mut Vec<LibraryFile>, seen: &mut Vec<PathBuf>, included: bool) -> Result<(), Error> {
    let path = path.canonicalize().map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    if seen.contains(&path) {
        return Ok(());
    }
    seen.push(path.clone());
    let (statements, file) = read(&path)?;
    read_files.push(file);
    let (includes, mut rest): (Vec<_>, Vec<_>) = statements.into_iter().partition(|(item, _)| matches!(item, syntax::Item::Include(_)));
    if included {
        rest.retain(|(item, _)| !matches!(item, syntax::Item::Io { output: true, .. }));
    }
    for (item, line) in includes {
        let syntax::Item::Include(name) = item else { unreachable!("an include") };
        let in_library = library().join(&name);
        let beside = path.parent().map(|d| d.join(&name)).unwrap_or_default();
        let found = if in_library.exists() {
            in_library
        } else {
            let miss = LibraryFile { path: in_library, sha256: None };
            noted(&miss);
            read_files.push(miss);
            beside
        };
        if !found.exists() {
            return Err(Error::Syntax(format!("{}:{line}: #include {name:?}: no such file", path.display())));
        }
        visit(&found, out, read_files, seen, true)?;
    }
    out.push((path, rest));
    Ok(())
}

/// The files of `path`'s program, each once, includes first (`path` last); and every library file read or probed,
/// in reading order.
fn program_order(path: &Path) -> Result<(Vec<PathBuf>, Vec<LibraryFile>), Error> {
    use super::plan::corpus::phase0;
    let path = path.canonicalize().map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    let g = phase0::graph(&phase0::Seed::File(path.clone()))?;
    let mut order = Vec::new();
    g.order(&path.to_string_lossy(), &mut order)?;
    order.retain(|p| *p != path);
    order.push(path.clone());
    // The files in reading order, as `LibraryFile`s check them: `core.rr` (an include at offset -1), then the
    // file, then each include, its library probe first where it was found beside the file.
    let library = library();
    let mut files = Vec::new();
    let mut seen = Vec::new();
    let key = path.to_string_lossy().into_owned();
    let (first, rest): (Vec<_>, Vec<_>) = g.includes.get(&key).cloned().unwrap_or_default().into_iter().partition(|(at, _, _)| *at < 0);
    for (_, _, target) in first {
        trace(&g, &library, &target, &mut files, &mut seen);
    }
    seen.push(path.clone());
    files.push(LibraryFile { path: path.clone(), sha256: g.digests.get(&path).copied() });
    for (_, name, target) in rest {
        probe(&library, &name, &target, &mut files);
        trace(&g, &library, &target, &mut files, &mut seen);
    }
    Ok((order, files))
}

/// A file and everything it includes, each once, includes first: (path, statements without the
/// includes, the included files' `.output`s left out); and every library file read or probed, in reading order.
pub fn program(path: &Path) -> Result<(Files, Vec<LibraryFile>), Error> {
    let (shared, files) = program_shared(path)?;
    Ok((shared.iter().map(|(p, s)| (p.clone(), s.iter().map(|s| (*s).clone()).collect())).collect(), files))
}

/// The modules of a program (`program_order`), each its statements as the process holds them (`read_shared`) and
/// those a program reads of them: without its includes, and an included file without its `.output`s (its own, not
/// the program's; `core.rr` is the prelude, read as the file is).
struct Shared(Vec<(PathBuf, std::sync::Arc<Vec<syntax::Statement>>)>);

impl Shared {
    fn iter(&self) -> impl Iterator<Item = (&PathBuf, Vec<&syntax::Statement>)> {
        let core = library().join("core.rr").canonicalize().ok();
        let own = self.0.last().map(|(p, _)| p.clone());
        self.0.iter().map(move |(p, statements)| {
            let included = Some(p) != own.as_ref() && Some(p) != core.as_ref();
            (p, statements.iter().filter(|(item, _)| !matches!(item, syntax::Item::Include(_)) && !(included && matches!(item, syntax::Item::Io { output: true, .. }))).collect())
        })
    }
}

fn program_shared(path: &Path) -> Result<(Shared, Vec<LibraryFile>), Error> {
    if super::plan::corpus::phase0::active() {
        let (modules, files) = builtin_program(path)?;
        return Ok((Shared(modules.into_iter().map(|(p, s)| (p, std::sync::Arc::new(s))).collect()), files));
    }
    if super::json_rules::is_policy_path(path) {
        return json_program_shared(path);
    }
    let (order, files) = program_order(path)?;
    let modules = order.into_iter().map(|p| Ok((p.clone(), read_shared(&p)?.0))).collect::<Result<Vec<_>, Error>>()?;
    Ok((Shared(modules), files))
}

/// A JSON policy (`rerail.policy/v1`, decision 0175), read as the `.rr` file it translates to: its text translated
/// as it is read (`json_rules`), its `#include`s (library files only) resolved as a text's are (`resolve_shared`).
/// The modules and the files read are in `program_shared`'s order: includes first and the file last; `core.rr`, the
/// file (its JSON bytes' digest, so an edit reads it again), then the rest.
fn json_program_shared(path: &Path) -> Result<(Shared, Vec<LibraryFile>), Error> {
    let path = path.canonicalize().map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    let (bytes, digest) = super::plan::corpus::observe(|o| o.read(&path)).ok_or_else(|| Error::Syntax(format!("{}: cannot read the file", path.display())))?;
    let file = LibraryFile { path: path.clone(), sha256: Some(digest) };
    noted(&file);
    let json = String::from_utf8(bytes).map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    let id = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let src = super::json_rules::Translator::new().translate_text(&json, &id).map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    let statements = stored_statements(&path, src).map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    let (modules, own) = resolve_shared(statements)?;
    let core = library().join("core.rr").canonicalize().ok();
    let (first, rest): (Vec<_>, Vec<_>) = modules.iter().partition(|(p, _, _)| Some(p) == core.as_ref());
    let mut files: Vec<LibraryFile> = first.iter().map(|(p, d, _)| LibraryFile { path: p.clone(), sha256: Some(*d) }).collect();
    files.push(file);
    files.extend(rest.iter().map(|(p, d, _)| LibraryFile { path: p.clone(), sha256: Some(*d) }));
    let mut shared: Vec<(PathBuf, std::sync::Arc<Vec<syntax::Statement>>)> = modules.into_iter().map(|(p, _, s)| (p, s)).collect();
    shared.push((path, std::sync::Arc::new(own)));
    Ok((Shared(shared), files))
}

/// `path` and what it includes, each once, as phase 0 found them: the reading order of the builtin reader.
fn trace(g: &super::plan::corpus::phase0::Graph, library: &Path, path: &Path, files: &mut Vec<LibraryFile>, seen: &mut Vec<PathBuf>) {
    if seen.iter().any(|p| p == path) {
        return;
    }
    seen.push(path.to_path_buf());
    files.push(LibraryFile { path: path.to_path_buf(), sha256: g.digests.get(path).copied() });
    for (_, name, target) in g.includes.get(&*path.to_string_lossy()).cloned().unwrap_or_default() {
        probe(library, &name, &target, files);
        trace(g, library, &target, files, seen);
    }
}

/// The library candidate an include probed and did not find, when its file is the one beside the includer.
fn probe(library: &Path, name: &str, target: &Path, files: &mut Vec<LibraryFile>) {
    let in_library = library.join(name);
    if in_library.canonicalize().ok().as_deref() != Some(target) {
        files.push(LibraryFile { path: in_library, sha256: None });
    }
}

/// `program` for phase 0's own reads.
fn builtin_program(path: &Path) -> Result<(Files, Vec<LibraryFile>), Error> {
    let (mut out, mut read_files) = (Vec::new(), Vec::new());
    // `core.rr` is every program's prelude (the base relations and built-ins), included or not; except a program in
    // T's shape (decision 0046), which declares its base relations itself (`doc` among them), and whose libraries
    // read only the relations both shapes have.
    let t_shape = read(&path.canonicalize().map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?)?
        .0
        .iter()
        .any(|(item, _)| matches!(item, syntax::Item::Decl { name, .. } if name == "doc"));
    if !t_shape {
        visit(&library().join("core.rr"), &mut out, &mut read_files, &mut Vec::new(), false)?;
    }
    let mut seen = vec![library().join("core.rr").canonicalize().map_err(|e| Error::Syntax(format!("core.rr: {e}")))?];
    visit(path, &mut out, &mut read_files, &mut seen, false)?;
    Ok((out, read_files))
}

pub use crate::code_rule_dsl::facts::vtab::syntax::core_rr::single_metavars;
