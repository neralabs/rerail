//! Rules in `.rerail`, the engine's rule language: Soufflé Datalog with the additions of
//! `docs/rule-syntax/lib/core.rerail` (decision 0043). Every rule the engine runs is read here.
//!
//! - `syntax`: tokens and statements as written;
//! - `lower`: statements to a program version (decision 0276), the form the planner, magic sets and the backends take;
//! - `print`: a version as `.rerail` text, read back as the same program;
//! - `check`: the checks of `docs/rule-syntax/site/check_rr.py` (grounded, declared once, stratified);
//! - `translate`: the earlier `.rdl` dialect's text in `.rerail`, comments kept, for the move and for the converter.
//!
//! `#include "x.rerail"` reads `docs/rule-syntax/lib/x.rerail`, or else `x.rerail` beside the including file; each file once.
//! A file read with `parse_file_version` has `core.rerail` before it, included or not. An included file's `.output`s are its own,
//! not the program's: they are left out (`python/calls.rerail`'s).

#[allow(unused_imports)]
use rerail_store::Canonical as _;
pub use rerail_rr::code_rule_dsl::logic::rr::check;
// Its files are its test modules alone.
#[cfg_attr(any(test, feature = "test-hooks"), path = "rr/aggregate_demand")]
pub mod aggregate_demand {
    pub use rerail_rr::code_rule_dsl::logic::rr::aggregate_demand::*;
    #[cfg(any(test, feature = "test-hooks"))]
    pub mod tests;
    #[allow(unused_imports)]
    use std::collections::{BTreeMap, BTreeSet};
}
pub use rerail_rr::code_rule_dsl::logic::rr::from_tape;
pub use rerail_rr::code_rule_dsl::logic::rr::clause;
pub use from_tape::{code_body, string_value, syntax_error};
mod git_policy;
pub use git_policy::GitPolicyInputs;
/// The planner's (`program_text` keeps its images); the store they are kept in is `images_store`, here.
pub use rerail_planner::code_rule_dsl::logic::rr::images;
pub mod module_image;
pub use rerail_rr::code_rule_dsl::logic::rr::lower;
/// The text tests `.rerail` names, and the engine's test each lowers to (the coverage gate probes each).
pub use rerail_rr::code_rule_dsl::logic::rr::lower::TESTS as RR_TESTS;
pub use rerail_rr::code_rule_dsl::logic::rr::print::{self};
// The parser and the tape's tree, the `.rerail` parser crate's (`rerail_rr_syntax`).
use rerail_store::Canonical as _;
use rerail_rr_syntax::syntax;
pub use rerail_rr_syntax::tape;
use from_tape::{RrFile, S, Stmt};

/// `src` with each `.violation`'s message reduced to its placeholders (`lower::findings::message_skeleton`): the
/// text a program's keys are made of (`corpus::texts_digest`). Lowering reads a message's placeholders alone, and the
/// writer puts its texts in (decision 0225 §3), so an edit of a message's words is no edit of the program.
pub fn message_masked(src: &str) -> std::borrow::Cow<'_, str> {
    if !src.contains(".violation") {
        return std::borrow::Cow::Borrowed(src);
    }
    let message = tape::Field::Message as u16;
    // The messages' spans, read off the lean parser's image in place (`tape_format::View`).
    let image = tape(src);
    let Ok(v) = rerail_rr_syntax::tape_format::View::new(&image) else { return std::borrow::Cow::Borrowed(src) };
    let mut spans: Vec<(usize, usize)> = (0..v.len())
        .filter(|&i| matches!(tape::kind_field(v.key(i)), (kind, field) if field == message && kind != tape::ERROR))
        .map(|i| (v.start(i) as usize, v.end(i) as usize))
        .collect();
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
    let Ok(file) = RrFile::read(src) else { return Vec::new() };
    let statements = file.statements();
    let policy = statements
        .iter()
        .find_map(|s| match s.what()? {
            S::Setting { key: "policy", text } => text.split('"').nth(1).map(str::to_owned),
            _ => None,
        })
        .unwrap_or_default();
    statements
        .iter()
        .filter_map(|s| match s.what()? {
            S::Violation(_) => {
                let (t, n) = s.tape();
                t.violation(n).ok().map(|(name, _, message)| (name, message))
            }
            S::Setting { key, text } if key == lower::findings::MESSAGE => setting_template(text),
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

// Its files are its test modules alone.
#[cfg_attr(any(test, feature = "test-hooks"), path = "rr")]
pub mod tape_type {
    pub use rerail_rr::code_rule_dsl::logic::rr::tape_type::*;
    #[cfg(any(test, feature = "test-hooks"))]
    #[path = "tape_type_tests.rs"]
    pub mod tests;
    #[allow(unused_imports)]
    use std::collections::{BTreeMap, BTreeSet};
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
#[cfg(any(test, feature = "test-hooks"))]
pub use lower::drop_unread_core;
pub use lower::is_core;
pub use lower::findings::{is_diagnostic, HEAD as DIAGNOSTIC_HEAD, UNPLACED};
pub use lower::{FILE_SYSTEM, WORLD};
pub use print::print_version;
pub use syntax::quote;
pub use translate::translate;

/// The relations `core.rerail` declares: the engine's names (decision 0141). Empty when `core.rerail` does not read.
pub fn core_names() -> &'static std::collections::BTreeSet<String> {
    static CORE: std::sync::OnceLock<std::collections::BTreeSet<String>> = std::sync::OnceLock::new();
    CORE.get_or_init(|| {
        std::fs::read_to_string(library().join("core.rerail")).ok().and_then(|src| parse_version(&src).ok()).map(|v| v.declared_names().into_iter().collect()).unwrap_or_default()
    })
}

/// `core_names` less the relations decision 0174's rewrite adds (`lower::made_on_request`): what a program in
/// `core.rerail`'s shape declares whichever attributes it answers on request (a round may be undone, `lower::undo_round`).
pub fn core_shape() -> &'static std::collections::BTreeSet<String> {
    static SHAPE: std::sync::OnceLock<std::collections::BTreeSet<String>> = std::sync::OnceLock::new();
    SHAPE.get_or_init(|| core_names().iter().filter(|r| !lower::made_on_request(r)).cloned().collect())
}

/// The library `#include` reads from (`rerail-rr`'s: the JSON rules' translator reads it too, under the language).
pub use rerail_rr::code_rule_dsl::logic::rr::library::library;

/// The directory whose include resolution is part of a program read. Preserve a missing root as a distinct input.
pub fn library_root() -> PathBuf {
    let path = library();
    path.canonical().or_else(|_| std::path::absolute(&path)).unwrap_or(path)
}

/// A program's `.rerail` text. Its `#include`s name library files (decision 0089), read as `parse_file_version` reads
/// them: `core.rerail` first unless the text is in T's shape, each file once, includes first. A text that includes nothing
/// is read as it is, without `core.rerail`.
pub fn parse_version(src: &str) -> Result<std::sync::Arc<super::program_image::Version>, Error> {
    read_text(src)?.lower_version(true)
}

/// A program's `.rerail` text as `parse_version` reads it, but with only its libraries' declarations, not their clauses:
/// what a run that holds the libraries once (decision 0089 §5) needs of a program that adds to no library relation.
/// Its own rules are lowered against the library's names; the library's rules are the run's, lowered once.
pub fn parse_own_version(src: &str) -> Result<std::sync::Arc<super::program_image::Version>, Error> {
    read_text(src)?.lower_version(false)
}

/// A program text read once (its tape, its statements, its includes resolved), for what a run needs of it: its
/// library modules (`modules`) and its program (`lower`). A run of a pack asks both of every program; each read of
/// a text makes its tape and resolves its includes again.
pub struct Text(Read);

enum Read {
    /// A text that includes nothing: read as it is.
    Plain(Vec<Stmt>),
    /// A text's library files (includes first: canonical path, bytes' digest, the file as `read` keeps it) and its
    /// own statements.
    Modular { files: SharedFiles, own: Vec<Stmt> },
}

/// `src` read once, its includes resolved (`Text`).
pub fn read_text(src: &str) -> Result<Text, Error> {
    read_parsed(RrFile::read(src)?.statements())
}

/// `read_text` of statements already read.
pub(in crate::code_rule_dsl) fn read_parsed(statements: Vec<Stmt>) -> Result<Text, Error> {
    if !statements.iter().any(is_include) {
        refuse_ungrounded(&statements, &BTreeSet::new())?;
        return Ok(Text(Read::Plain(statements)));
    }
    let (files, own) = resolve_shared(statements)?;
    let given: BTreeSet<String> = files.iter().flat_map(|(_, _, f)| check::fix_relations(f)).collect();
    refuse_ungrounded(&own, &given)?;
    Ok(Text(Read::Modular { files, own }))
}

/// A program's own rules must be grounded (`check::grounded`, which the library's files pass by test): every head
/// variable, and every variable a negation, comparison or aggregate reads from outside, bound by a positive literal
/// of the body. An ungrounded rule otherwise fails late and far from its cause — "unplaced literals", "$x is unbound
/// in SQL", a statement over SQLite's 65535 references — or, when no rule reads the unbound column, is silently
/// projected away.
fn refuse_ungrounded(statements: &[Stmt], given: &BTreeSet<String>) -> Result<(), Error> {
    // A program in T's shape (it declares `doc` itself: a lowered program printed and read again, `combine`) holds
    // the lowering's own rules, which are not source rules: only a source text is checked.
    if t_shape(statements) {
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
        let declares = |statements: &[Stmt]| -> std::collections::BTreeSet<String> {
            statements
                .iter()
                .filter_map(|s| match s.what()? {
                    S::Decl(name) | S::Violation(name) => Some(name.to_owned()),
                    _ => None,
                })
                .collect()
        };
        let heads = own.iter().filter(|s| !s.has_edits()).flat_map(|s| s.heads().into_iter().map(str::to_owned)).collect();
        Some(Modules { files: files.iter().map(|(path, digest, file)| (path.clone(), *digest, declares(file))).collect(), heads, declared: declares(own) })
    }

    /// The text's header: its program without a clause of its own or of its libraries (their declarations, its
    /// directives, its findings' declarations), for what a batch reads of a policy before its run (decision 0168):
    /// its settings, `.policy`, `.language` and outputs, as its whole program has them, less `.global library` (which
    /// `read_file` writes) and the settings a clause makes. Lowering only that is a fraction of lowering its rules.
    pub fn header(self) -> Result<std::sync::Arc<super::program_image::Version>, Error> {
        let header = |s: &Stmt| !matches!(s.what(), Some(S::Clause | S::Include(_)));
        match self.0 {
            Read::Plain(statements) => lower_statements_version(statements.into_iter().filter(header).collect()),
            Read::Modular { files, own } => {
                let mut statements: Vec<Stmt> = files.iter().flat_map(|(_, _, f)| f.iter().cloned()).filter(header).collect();
                statements.extend(own.into_iter().filter(header));
                lower_statements_version(statements)
            }
        }
    }

    /// The text's own program as a version, with the library's cone it asks for. A text that asks nothing of the
    /// library is its own version; one that asks has its cone copied on the version (`plan::corpus::cone_on_version`),
    /// the library's tree never decoded.
    fn lowered_in_version(self, library: &Library, scoped: &Scoped) -> Result<Option<std::sync::Arc<super::program_image::Version>>, Error> {
        let Some(Own { version: own, asked, declared }) = self.lowered_own(scoped, Some(library))? else { return Ok(None) };
        if asked.is_empty() {
            return Ok(Some(own));
        }
        super::plan::corpus::cone_on_version(&own, &library.version, &asked, &declared).map(Some).map_err(Error::Syntax)
    }

    /// The text's own program, lowered and linked in its libraries' scope, and what it asks of the library: the
    /// library relations it adds to (decision 0089 §3), and every relation the library's files declare.
    /// `library`, the scope's library linked, gives the guarantees of its relations to the value cycles' check.
    fn lowered_own(self, scoped: &Scoped, library: Option<&Library>) -> Result<Option<Own>, Error> {
        let Read::Modular { files, own } = self.0 else { return Ok(None) };
        let checked: Vec<Stmt> = own.iter().filter(|s| !(matches!(s.what(), Some(S::Clause)) && !s.has_body())).cloned().collect();
        let reads_core = files.first().is_some_and(|(path, _, _)| path.ends_with("core.rerail"));
        check_names(&vec![(PathBuf::from("<program>"), checked)], reads_core)?;
        closed_program(&files, &own)?;
        let own: Vec<Stmt> = own.into_iter().filter(|s| !is_include(s)).collect();
        // Its rules written as a version in the walk (decision 0377 step 3).
        let linked = lower::link_seeded_parts(lower::lower_module_seeded_written(own, &scoped.seeds)?, &scoped.seeds)?;
        let written = std::sync::Arc::clone(&linked.own);
        let version = link_seeded_version(linked, written, scoped, core_names(), library)?;
        // A text that adds to library relations asks of the library (decision 0089 §3): its request's cone, the library
        // relations that read what it adds, and the helpers they read, are copied from the linked library, made once
        // (decision 0246 §2: a cone is made at link time). The run's `own_part` then holds them as the request.
        let declared: BTreeSet<String> = files.iter().flat_map(|(_, _, f)| f.iter()).filter_map(|s| match s.what()? {
            S::Decl(name) | S::Violation(name) => Some(name.to_owned()),
            _ => None,
        }).collect();
        // A fact adds as a rule does (`lit_repr_threshold(16384).`): left out, the library's readers read the library's
        // relation without it, and found nothing in a pack (PR #508's 363b91c0b, t47).
        let asked: BTreeSet<String> = version
            .rules()
            .map(|r| version.rel_name(r.head().rel()).to_owned())
            .chain(super::program_image::fields::ProgramFields::fact_names(&*version))
            .filter(|h| declared.contains(h) && !core_names().contains(h))
            .collect();
        Ok(Some(Own { version, asked, declared }))
    }

    /// The text's own program as its walk writes it (`lower::lower_module_seeded_written`) and its link leaves it: a
    /// test's view of decision 0377 step 3.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn own_written(self, scoped: &Scoped) -> Result<std::sync::Arc<super::program_image::Version>, Error> {
        let Read::Modular { own, .. } = self.0 else { return Err(Error::Syntax("not a modular text".to_owned())) };
        let own: Vec<Stmt> = own.into_iter().filter(|s| !is_include(s)).collect();
        Ok(lower::link_seeded_parts(lower::lower_module_seeded_written(own, &scoped.seeds)?, &scoped.seeds)?.own)
    }

    /// The text's own program, lowered as a module against the library's modules its includes reach and linked in
    /// their scope (`lower::link_in`, decision 0246 §2), without lowering the library's declarations again. A text
    /// that asks of the library has its request's part copied from the linked library (`with_cone_version`). `None`
    /// for a text that includes nothing. `store` holds it as a program image (decision 0246 §6) under its text `src`
    /// and its scope's keys: a store that holds it hands it on unread (decision 0281's path, step 7a-2); else the
    /// text is lowered, and its image kept.
    /// `found`: the image as the caller read it already (`images::read_images`, its lease renewed there), or `None` to
    /// read it here.
    pub fn lower_in_version(self, src: &str, library: &Library, scoped: &Scoped, store: Option<&crate::content_store::ContentStore>, found: Option<Option<images::ImageFound>>) -> Result<Option<std::sync::Arc<super::program_image::Version>>, Error> {
        if !matches!(self.0, Read::Modular { .. }) {
            return Ok(None);
        }
        let key = {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "a program's image key".into());
            program_key(src, &scoped.keys)
        };
        let read_here = found.is_none();
        let found = match found {
            Some(found) => found,
            None => {
                let _s = crate::code_rule_dsl::logic::profile::span(|| "a program's image, from the store".into());
                images::read_image(store, key)
            }
        };
        // Read where it is held (decision 0464): the bundle's mapping, a segment's, or its row loaded at 16 bytes.
        if let Some((bytes, id)) = found
            && let Ok(version) = super::program_image::Version::open_shared(bytes)
        {
            if read_here && let (Some(store), Some(id)) = (store, id) {
                renew_image(store, id);
            }
            crate::code_rule_dsl::prof::count("rr.program_image_read", 1);
            return Ok(Some(version));
        }
        let Some(version) = self.lowered_in_version(library, scoped)? else { return Ok(None) };
        let version = if version.base_bytes().is_some() { version } else { version.fold().map_err(Error::Syntax)? };
        if keep_image(store, "a program image", key, version.own_image().filter(|_| version.base_bytes().is_some()).expect("a folded version is a base").clone()) {
            crate::code_rule_dsl::prof::count("rr.program_image_written", 1);
        }
        Ok(Some(version))
    }

    /// The text's program as a version, as `parse_version` (`clauses`) or `parse_own_version` reads it.
    pub fn lower_version(self, clauses: bool) -> Result<std::sync::Arc<super::program_image::Version>, Error> {
        let (files, own) = match self.0 {
            Read::Plain(statements) => return lower_statements_version(statements),
            Read::Modular { files, own } => (files, own),
        };
        // The text's own rules and declarations; its facts may be the runner's (`env`, decision 0089 §1).
        let checked: Vec<Stmt> = own.iter().filter(|s| !(matches!(s.what(), Some(S::Clause)) && !s.has_body())).cloned().collect();
        // `core.rerail`'s names, unless the text is in T's shape and reads no `core.rerail` (`resolve`).
        let reads_core = files.first().is_some_and(|(path, _, _)| path.ends_with("core.rerail"));
        check_names(&vec![(PathBuf::from("<program>"), checked)], reads_core)?;
        closed_program(&files, &own)?;
        // Lowering reads one kind of clause statically: a kind set's (`fscope("module")`, `lfscope(k) :- fscope(k)`),
        // whose relation has one column. Those are kept; the rest are the run's. Only what is kept is copied.
        let all: Vec<Stmt> = files.iter().flat_map(|(_, _, f)| f.iter().cloned()).collect();
        let unary: std::collections::BTreeSet<String> = if clauses {
            Default::default()
        } else {
            all.iter()
                .filter(|s| matches!(s.what(), Some(S::Decl(_))))
                .filter_map(|s| {
                    let (t, n) = s.tape();
                    t.decl(n).ok().filter(|(_, cols, _)| cols.len() == 1).map(|(name, _, _)| name)
                })
                .collect()
        };
        let kept = |s: &Stmt| match s.what() {
            Some(S::Include(_)) => false,
            Some(S::Clause) => clauses || s.heads().iter().all(|h| unary.contains(*h)),
            _ => true,
        };
        let mut statements: Vec<Stmt> = all.into_iter().filter(kept).collect();
        statements.extend(own);
        lower_statements_version(statements)
    }
}

/// A program text's library modules (decision 0089): the files its `#include`s name, as `parse_version` reads them, and
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

/// `modules` of many texts, in their order: every text's include graph derived by one run of phase 0 first, as a
/// corpus run reads its programs (`phase0::graphs`), not one nested run a text; then the texts read on the pool, each
/// finding its graph kept. A text that does not read gives its error in its place.
pub fn modules_of(srcs: &[&str]) -> Vec<Result<Option<Modules>, Error>> {
    use rayon::prelude::*;
    let seeds: Vec<_> = srcs.iter().filter_map(|src| read_seed(src).ok()?.seed).collect();
    if !seeds.is_empty() {
        // An error is left to the read that meets it, which reports it in its text's place.
        let _ = super::plan::corpus::phase0::graphs(&seeds);
    }
    // The caller's context on each job's thread (its observer and store, a test's own), as the corpus's reads carry it.
    let carry = super::run_state::Carry::here();
    super::profile::on_cores(|| {
        srcs.par_iter()
            .map(|src| {
                let _in = carry.enter();
                modules(src)
            })
            .collect()
    })
}

/// A run's library (decision 0089): its modules, each with its files (canonical paths), and the program linked from
/// them. A program that asks nothing of the library is lowered against the modules its includes reach
/// (`Text::lower_in`), and the run holds the linked program once.
pub struct Library {
    pub modules: Vec<lower::Module>,
    pub files: Vec<Vec<PathBuf>>,
    /// Each module's key in the store (`module_key`).
    pub keys: Vec<crate::content_store::HashKey>,
    /// The linked library, as its image read from the store or made once (decision 0281's path, step 7a-2).
    pub version: std::sync::Arc<super::program_image::Version>,
    /// Its columns' spans (`lower::span_columns`), made when a program lowered in its scope first reads them (`spans`).
    spans: std::sync::OnceLock<std::sync::Arc<lower::SpanColumns>>,
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
    /// The scope's program as a version (its seeds' header), made once: each program lowered in the scope starts from
    /// it (`link_seeded_version`).
    version: std::sync::OnceLock<std::sync::Arc<super::program_image::Version>>,
}

impl Scoped {
    /// The scope's program as a version, made on first use.
    fn version(&self) -> Result<std::sync::Arc<super::program_image::Version>, Error> {
        if let Some(v) = self.version.get() {
            return Ok(std::sync::Arc::clone(v));
        }
        let v = self.seeds.version()?;
        Ok(std::sync::Arc::clone(self.version.get_or_init(|| v)))
    }

    /// The alias a program of text `src` lowered in this scope is kept under as its image.
    pub fn program_image(&self, src: &str) -> crate::content_store::HashKey {
        program_key(src, &self.keys)
    }
}

impl Library {
    fn of(modules: Vec<lower::Module>, files: Vec<Vec<PathBuf>>, keys: Vec<crate::content_store::HashKey>, version: std::sync::Arc<super::program_image::Version>) -> Self {
        Library { modules, files, keys, version, spans: Default::default() }
    }

    /// Its columns' spans, which a program lowered in its scope reads of the library relations it does not derive
    /// (`lowering_tail::SpanPatterns`, PR #508's 4146acbd9). Made once; a library whose rules the analysis refuses
    /// gives none.
    pub fn spans(&self) -> std::sync::Arc<lower::SpanColumns> {
        std::sync::Arc::clone(self.spans.get_or_init(|| {
            let columns = super::plan::corpus::preparation::lowering_tail::with_span_program(&self.version, None, lower::span_columns);
            std::sync::Arc::new(columns.ok().and_then(Result::ok).unwrap_or_default())
        }))
    }

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
        Scoped { keys, seeds: lower::seeds(&scope), version: Default::default() }
    }

    /// The modules a program whose library files are `files` reads: those all of whose files it includes, in order,
    /// with their keys.
    pub fn scope(&self, files: &[PathBuf]) -> (Vec<&lower::Module>, Vec<crate::content_store::HashKey>) {
        self.modules.iter().zip(&self.files).zip(&self.keys).filter(|((_, fs), _)| fs.iter().all(|f| files.contains(f))).map(|((m, _), k)| (m, *k)).unzip()
    }
}

/// The library files `paths` (canonical, includes first, as `Modules::files` lists them) as one program, lowered
/// once for every program of a run that includes them (decision 0089). Kept for the process by the files' digests.
pub fn lower_library(paths: &[PathBuf], store: Option<&crate::content_store::ContentStore>) -> Result<std::sync::Arc<Library>, Error> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    #[derive(Default)]
    struct Libraries(Mutex<HashMap<Vec<(PathBuf, [u8; 32])>, Arc<Library>>>);
    let mut files = Vec::new();
    let mut key = Vec::new();
    for path in paths {
        let (statements, file) = read(path)?;
        key.push((path.clone(), file.sha256.unwrap_or_default()));
        files.push((path.clone(), statements.into_iter().filter(|s| !is_include(s)).collect()));
    }
    let cache_held = crate::code_rule_dsl::logic::profile::shared::<Libraries>();
    let cache = &cache_held.0;
    if let Some(library) = cache.lock().expect("the lowered libraries").get(&key) {
        crate::code_rule_dsl::prof::count("rr.library_reused", 1);
        return Ok(Arc::clone(library));
    }
    check_names(&files, false)?;
    let library = if std::env::var("RERAIL_LIBRARY_LOWERING").is_ok_and(|v| v == "whole") {
        let version = lower_statements_version(files.into_iter().flat_map(|(_, statements): (PathBuf, Vec<Stmt>)| statements).collect())?;
        Library::of(Vec::new(), Vec::new(), Vec::new(), version)
    } else {
        let (modules, members, keys) = library_modules_with_files(paths, files, store)?;
        // The linked library is an image too (decision 0246 §2, §6), under its modules' keys: a store that holds it
        // reads it, and the whole-program half does not run again.
        let key = linked_key(&keys);
        // Read as its version, with no tree decoded (decision 0281's path, step 7a-2).
        let stored = images::read_image(store, key).and_then(|(bytes, id)| {
            let version = super::program_image::Version::open_shared(bytes).ok()?;
            if let (Some(store), Some(id)) = (store, id) {
                renew_image(store, id);
            }
            crate::code_rule_dsl::prof::count("rr.linked_library_read", 1);
            Some(version)
        });
        // Its declared kind sets are relations its includers may read (`lower::keep_declared_kind_sets`), whether
        // its image was kept before this or not.
        match stored {
            Some(version) => {
                let version = lower::keep_declared_kind_sets(&version, &modules)?;
                Library::of(modules, members, keys, version)
            }
            None => {
                // Linked on versions (decision 0377): the modules' rules are never a tree.
                let version = link_library_versions(modules.iter().map(lower::Module::clone).collect())?;
                if keep_image(store, "a linked library's image", key, version.own_image().filter(|_| version.base_bytes().is_some()).expect("a folded version is a base").clone()) {
                    crate::code_rule_dsl::prof::count("rr.linked_library_written", 1);
                }
                let version = lower::keep_declared_kind_sets(&version, &modules)?;
                Library::of(modules, members, keys, version)
            }
        }
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
/// at a time, each against the modules it reaches (decision 0246 §1), to be linked (`link_library_versions`). A
/// module is a file, or files that include each other (`python/names.rerail` and `python/imports.rerail`: one unit, read
/// together). `core.rerail` is in every other file's scope, as `parse_version` reads a file with it first.
/// `RERAIL_LIBRARY_LOWERING=whole` lowers them as one (`lower_library`), as before, to compare.
pub fn library_modules(paths: &[PathBuf], files: Files) -> Result<Vec<lower::Module>, Error> {
    let store = image_store();
    Ok(library_modules_with_files(paths, files, store.as_deref())?.0)
}

/// The relations `statements` declare, and whether each is declared `extensible` (decision 0263).
fn declared_in(statements: &[Stmt]) -> impl Iterator<Item = (String, bool)> + '_ {
    statements.iter().filter(|s| matches!(s.what(), Some(S::Decl(_)))).filter_map(|s| {
        let (t, n) = s.tape();
        let (name, _, tail) = t.decl(n).ok()?;
        Some((name, tail.iter().any(|w| w == "extensible")))
    })
}

/// A library's relations are closed (decision 0263): a file adds no rules or facts to a relation another file
/// declares, unless that declaration says `extensible`. `open`: the relations of the files `file` includes, with the
/// file declaring each and whether it is extensible.
fn closed_heads(file: &str, statements: &[Stmt], open: &std::collections::HashMap<String, (&Path, bool)>) -> Result<(), Error> {
    let own: BTreeSet<String> = declared_in(statements).map(|(n, _)| n).collect();
    for s in statements {
        for h in s.heads().into_iter().filter(|h| !own.contains(*h)) {
            if let Some((home, false)) = open.get(h) {
                return Err(Error::Syntax(format!(
                    "{file}:{}: `{h}` is declared in {}, and not `extensible`: a file that includes it adds no rules or facts to it (decision 0263)",
                    s.line(),
                    home.display()
                )));
            }
        }
    }
    Ok(())
}

/// `closed_heads` for a program's own statements, over every library file it includes.
fn closed_program(files: &SharedFiles, own: &[Stmt]) -> Result<(), Error> {
    let mut open = std::collections::HashMap::new();
    for (path, _, file) in files {
        for (rel, ext) in declared_in(file) {
            open.entry(rel).or_insert((path.as_path(), ext));
        }
    }
    closed_heads("the program", own, &open)
}

/// `library_modules`, with each module's files and its key; their images read from and kept in `store` (decision
/// 0264), and read from the shipped bundle first.
#[allow(clippy::type_complexity)]
fn library_modules_with_files(paths: &[PathBuf], files: Files, store: Option<&crate::content_store::ContentStore>) -> Result<(Vec<lower::Module>, Vec<Vec<PathBuf>>, Vec<crate::content_store::HashKey>), Error> {
    let _span = crate::code_rule_dsl::logic::profile::span(|| format!("lower the library one module at a time ({} files)", paths.len()));
    let core = library().join("core.rerail").canonical().ok();
    // An include is found by its canonical path, so the run's files are compared canonical too: on Windows a path
    // through a link, or with `/` past a `\`-joined root, names the same file another way.
    let canonical: Vec<PathBuf> = paths.iter().map(|p| p.canonical().unwrap_or_else(|_| p.clone())).collect();
    let at = |p: &Path| canonical.iter().position(|q| q == p);
    let n = paths.len();
    // Each file's direct includes, by position: the library's file, else the one beside it (`#include`).
    let mut direct: Vec<BTreeSet<usize>> = Vec::with_capacity(n);
    let mut digests: Vec<[u8; 32]> = Vec::with_capacity(n);
    for (i, path) in paths.iter().enumerate() {
        let (rr_file, file) = read_shared(path)?;
        digests.push(file.sha256.unwrap_or_default());
        let mut mine: BTreeSet<usize> = BTreeSet::new();
        if let Some(c) = core.as_deref().and_then(at).filter(|&c| c != i) {
            mine.insert(c);
        }
        for s in rr_file.statements() {
            let Some(S::Include(name)) = s.what() else { continue };
            let name = name.as_str();
            let line = s.line();
            let found = [library().join(name), path.parent().map_or_else(|| PathBuf::from(name), |d| d.join(name))].into_iter().find_map(|p| p.canonical().ok());
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
    let mut statements: Vec<Option<Vec<Stmt>>> = files.into_iter().map(|(_, s)| Some(s)).collect();
    let mut modules: Vec<lower::Module> = Vec::with_capacity(units.len());
    let mut keys: Vec<crate::content_store::HashKey> = Vec::with_capacity(units.len());
    for (u, members) in units.iter().enumerate() {
        let key = module_key(members.iter().map(|&i| &digests[i]), scopes[u].iter().map(|&v| &keys[v]));
        keys.push(key);
        if let Some((bytes, id)) = images::read_image(store, key)
            && let Ok(m) = module_image::decode(bytes)
        {
            if let (Some(store), Some(id)) = (store, id) {
                renew_image(store, id);
            }
            crate::code_rule_dsl::prof::count("rr.module_image_read", 1);
            modules.push(m);
            continue;
        }
        let own: Vec<Stmt> = members.iter().flat_map(|&i| statements[i].take().unwrap_or_default()).collect();
        let scope: Vec<&lower::Module> = scopes[u].iter().map(|&v| &modules[v]).collect();
        let m = lower::lower_module_in_written(own, &scope).map_err(|e| match e {
            Error::Syntax(m) => Error::Syntax(format!("{}: {m}", paths[members[0]].display())),
            e => e,
        })?;
        // Written by the store's writer, behind the run (decisions 0104, 0240 §1); dropped where its queue is full, as
        // a tape is: the next run lowers the module again.
        if let Ok(image) = module_image::encode(&m)
            && keep_image(store, "a module image", key, std::sync::Arc::new(image))
        {
            crate::code_rule_dsl::prof::count("rr.module_image_written", 1);
        }
        modules.push(m);
    }
    Ok((modules, units.iter().map(|members| members.iter().map(|&i| paths[i].clone()).collect()).collect(), keys))
}

/// The store rule images are read from and kept in (`images_store`), unless `RERAIL_MODULE_IMAGES=off`.
/// The store the run's rule images are kept in (decision 0264, `images`): the engine build's, shared by every project
/// (`RERAIL_IMAGE_STORE`: a path, or `project` for the run's own store); a test's own store under test.
fn images_store() -> Option<std::sync::Arc<crate::content_store::ContentStore>> {
    let project = || crate::code_rule_dsl::logic::plan::corpus::phases::reading_store();
    if cfg!(any(test, feature = "test-hooks")) {
        return project();
    }
    let path: std::path::PathBuf = match std::env::var("RERAIL_IMAGE_STORE") {
        Ok(v) if v == "project" => return project(),
        Ok(v) => std::path::PathBuf::from(v),
        Err(_) => crate::code_rule_dsl::logic::plan::corpus::build_store("rerail-images", None),
    };
    crate::content_store::ContentStore::open_images(&path).ok().map(std::sync::Arc::new)
}

pub(in crate::code_rule_dsl) fn image_store() -> Option<std::sync::Arc<crate::content_store::ContentStore>> {
    if cfg!(feature = "tape-cache") && std::env::var("RERAIL_MODULE_IMAGES").map_or(true, |v| v != "off") { images_store() } else { None }
}

/// How long a module's or program's image stays in the store after the last run that read or wrote it
/// (`keep_image`): collection keeps a source while a pin's lease runs, as it keeps the combined program's entry before
/// a run names it (`combined::record_version`). Naming the images in the run's record is decision 0246 §6's.
const IMAGE_LEASE_SECS: i64 = 7 * 24 * 3600;

/// The owner of the images' pins.
const IMAGE_OWNER: &str = "rr images (decision 0246)";

fn image_lease() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64) + IMAGE_LEASE_SECS
}

/// Keeps `bytes`, an image, in `store` under alias `key`, pinned for its lease: written by the store's writer, behind
/// the run (decisions 0104, 0240 §1), and dropped where its queue is full, as a tape is (the next run lowers it
/// again). Whether it was queued.
fn keep_image(store: Option<&crate::content_store::ContentStore>, what: &'static str, key: crate::content_store::HashKey, bytes: crate::content_store::record::Bytes) -> bool {
    use crate::content_store::write_back::{Key, Syms};
    images::export(key, (*bytes).as_ref());
    let Some(store) = store else { return false };
    // A record of its own (decision 0306), dropped where the queue is full: the image its piece, as its maker holds
    // it, placed in a segment by the writer and read there (decision 0464).
    fn apply(cx: &crate::content_store::record::Apply<'_>, body: crate::content_store::record::codec::Body) -> crate::content_store::StoreResult<()> {
        let key: crate::content_store::HashKey = crate::content_store::record::codec::from_body("an image", body)?;
        let hash = crate::content_store::HashKey::sha256(cx.bytes);
        let id = cx.store.put_sources_placed(&[(cx.bytes, hash, &[key])])?[0];
        cx.store.pin_source(id, IMAGE_OWNER, image_lease())
    }
    let queued = Key::of(&[what.as_bytes(), &key.sha256_bytes().unwrap_or_default()]);
    store.queue_record_held(Syms::None, what, vec![queued], &key, vec![bytes], apply, true)
}

/// The text tests' compiled groups of automata as images (decisions 0075, 0264): each under its key, read from the
/// shipped bundle, else the engine build's image store (`image_store`), so a project's first run reads the groups the
/// bundle or an earlier project determinized. A group's DFA is the build's (its bytes are regex-automata's, in the
/// build's byte order), as the other images are. An entry is a tag (1: a DFA follows; 0: the group does not
/// determinize within the limit) and the DFA's bytes.
pub struct ImageDfas(pub Option<std::sync::Arc<crate::content_store::ContentStore>>);

impl ImageDfas {
    fn key(group: &str) -> crate::content_store::HashKey {
        crate::content_store::HashKey::sha256(format!("rerail dfa group image 1\0{}\0{group}", crate::code_rule_dsl::build_ids::engine()).as_bytes())
    }
}

impl crate::code_rule_dsl::text::vocabulary::DfaCache for ImageDfas {
    fn get(&self, group: &str) -> Option<Option<Vec<u8>>> {
        let (bytes, id) = images::read(self.0.as_deref(), Self::key(group))?;
        if let (Some(store), Some(id)) = (self.0.as_deref(), id) {
            renew_image(store, id);
        }
        crate::code_rule_dsl::prof::count("rr.dfa_image_read", 1);
        match bytes.split_first() {
            Some((1, dfa)) => Some(Some(dfa.to_vec())),
            Some((0, _)) => Some(None),
            _ => None,
        }
    }

    fn put(&self, group: &str, dfa: Option<&[u8]>) {
        let mut entry = vec![u8::from(dfa.is_some())];
        entry.extend_from_slice(dfa.unwrap_or_default());
        crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, entry.len() as u64);
        keep_image(self.0.as_deref(), "a group of automata", Self::key(group), std::sync::Arc::new(entry));
    }
}

/// Phase 0's graph of a seed as an image (decision 0246): read from the shipped bundle, else `store` (the engine
/// build's, `image_store`, opened once by the caller for all its seeds), its lease renewed; `phase0::graphs` checks it
/// holds before it takes it.
pub(in crate::code_rule_dsl) fn graph_image(store: Option<&crate::content_store::ContentStore>, key: crate::content_store::HashKey) -> Option<Vec<u8>> {
    let (bytes, id) = images::read(store, key)?;
    if let (Some(store), Some(id)) = (store, id) {
        renew_image(store, id);
    }
    Some(bytes.into_owned())
}

/// A process can first read this graph for a different store. Keep it where this reader can find it on restart.
pub(in crate::code_rule_dsl) fn retain_graph_image(store: Option<&crate::content_store::ContentStore>, key: crate::content_store::HashKey, encode: impl FnOnce() -> Vec<u8>) {
    let Some(store) = store else { return };
    if images::shipped().is_some_and(|bundle| bundle.get(key).is_some()) { return; }
    if let Ok(Some(id)) = store.source_by_alias(key) {
        renew_image(store, id);
    } else {
        keep_graph_image(Some(store), key, encode());
    }
}

/// Keeps a graph's image (`graph_image`) where none is kept under its key yet. A seed's key names the seed, not the
/// files its graph reached, so an edited library gives the same key another graph: the store's alias holds one
/// image, and the first stays. A run then finds it does not hold (`Graph::holds`) and derives the graph, as without
/// images. The shipped bundle's graphs are its library's; exported, the last derived is the one written.
pub(in crate::code_rule_dsl) fn keep_graph_image(store: Option<&crate::content_store::ContentStore>, key: crate::content_store::HashKey, bytes: Vec<u8>) {
    use crate::content_store::write_back::{Key, Syms};
    const WHAT: &str = "phase 0's graph of a seed";
    images::export(key, &bytes);
    let Some(store) = store else { return };
    // A record of its own (decision 0306), dropped where the queue is full, as `keep_image`'s: the graph in its byte
    // area, the first kept under its key staying.
    fn apply(cx: &crate::content_store::record::Apply<'_>, body: crate::content_store::record::codec::Body) -> crate::content_store::StoreResult<()> {
        let key: crate::content_store::HashKey = crate::content_store::record::codec::from_body(WHAT, body)?;
        if cx.store.source_by_alias(key)?.is_some() {
            return Ok(());
        }
        let id = cx.store.put_source(cx.bytes, &[key])?;
        cx.store.pin_source(id, IMAGE_OWNER, image_lease())
    }
    let queued = Key::of(&[WHAT.as_bytes(), &key.sha256_bytes().unwrap_or_default()]);
    store.queue_record(Syms::None, WHAT, vec![queued], &key, Some(bytes), apply, true);
}

/// The lease of image `id`, read by this run, renewed (`keep_image`) once more than half of it has run: a run that reads
/// images an earlier run kept or renewed within that writes nothing (decision 0250), and an image read at least
/// every half lease is never collected.
/// `renew_image` of many images, their leases read in one statement's hold.
pub(in crate::code_rule_dsl) fn renew_images(store: &crate::content_store::ContentStore, ids: &[crate::content_store::SourceId]) {
    let now = image_lease() - IMAGE_LEASE_SECS;
    let Ok(leases) = store.source_leases(ids, IMAGE_OWNER) else { return };
    for (id, until) in ids.iter().zip(leases) {
        if !until.is_some_and(|until| until - now > IMAGE_LEASE_SECS / 2) {
            renew_image(store, *id);
        }
    }
}

fn renew_image(store: &crate::content_store::ContentStore, id: crate::content_store::SourceId) {
    use crate::content_store::write_back::Syms;
    let now = image_lease() - IMAGE_LEASE_SECS;
    if store.source_lease(id, IMAGE_OWNER).ok().flatten().is_some_and(|until| until - now > IMAGE_LEASE_SECS / 2) {
        return;
    }
    fn apply(cx: &crate::content_store::record::Apply<'_>, body: crate::content_store::record::codec::Body) -> crate::content_store::StoreResult<()> {
        let id: crate::content_store::SourceId = crate::content_store::record::codec::from_body("an image's lease", body)?;
        cx.store.pin_source(id, IMAGE_OWNER, image_lease())
    }
    store.queue_record(Syms::None, "an image's lease", Vec::new(), &id, None, apply, true);
}

/// A text's own program lowered (`Text::lowered_own`), and what it asks of the library.
struct Own {
    version: std::sync::Arc<super::program_image::Version>,
    asked: BTreeSet<String>,
    declared: BTreeSet<String>,
}

/// The relations of `library`'s cone for `asked` (`with_cone_version`'s): those reading what `asked` reaches, and the helpers
/// they read that `declared` and core leave the library's. Over the library's rules' heads and reads and its natives.
fn cone_of<'a>(heads_reads: &[(&'a str, Vec<&'a str>)], natives: &'a std::collections::BTreeMap<String, super::front::Native>, asked: &BTreeSet<String>, declared: &BTreeSet<String>) -> BTreeSet<String> {
    let mut cone: BTreeSet<String> = asked.clone();
    loop {
        let before = cone.len();
        for (head, reads) in heads_reads {
            if !cone.contains(*head) && reads.iter().any(|b| cone.contains(*b)) {
                cone.insert((*head).to_owned());
            }
        }
        for (rel, n) in natives {
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
        let read: Vec<String> = heads_reads.iter().filter(|(h, _)| *h == rel).flat_map(|(_, r)| r.iter().map(|x| (*x).to_owned())).chain(natives.get(&rel).into_iter().flat_map(|n| n.reads().into_iter().map(str::to_owned))).collect();
        for b in read {
            if !declared.contains(&b) && !core.contains(&b) && !take.contains(&b) && (heads_reads.iter().any(|(h, _)| *h == b) || natives.contains_key(&b)) {
                take.insert(b.clone());
                work.push(b);
            }
        }
    }
    take
}

/// Adds to `program`, which derives library relations `asked`, the linked library's part a request copies (decision
/// 0089 §3): the library relations reading `asked`, through any rule or native, with `asked` themselves; and the
/// relations their rules read that no library file declares (lowering's helpers), with their own. Their rules,
/// declarations, natives, facts and lattices, as the library has them.
/// A program with the library's cone for `asked` (decision 0281's path, step 7b): the cone's relations by `cone_of`
/// over the library version's heads and reads, its rules copied (`Builder::copy`), and their declarations, natives,
/// facts and lattices set in the header.
pub(crate) fn with_cone_version(program: &std::sync::Arc<super::program_image::Version>, library: &std::sync::Arc<super::program_image::Version>, asked: &BTreeSet<String>, declared: &BTreeSet<String>) -> Result<std::sync::Arc<super::program_image::Version>, String> {
    use super::program_image::Lit;
    use super::program_image::fields::{FieldsMut, ProgramFields};
    use super::program_facts::Field;
    let lib_natives = ProgramFields::natives(&**library);
    // The library as written: the on-request rewrite's rules are the program's to make again over its cone.
    let written = |r: &super::program_image::RuleRef<'_>| !lower::on_request_rule(library.rel_name(r.head().rel()));
    let heads_reads: Vec<(&str, Vec<&str>)> = library
        .rules()
        .filter(written)
        .map(|r| {
            (
                library.rel_name(r.head().rel()),
                r.body()
                    .filter_map(|l| match l.view() {
                        Lit::Atom(a) | Lit::Agg { atom: a, .. } => Some(library.rel_name(a.rel())),
                        _ => None,
                    })
                    .collect(),
            )
        })
        .collect();
    let take = cone_of(&heads_reads, &lib_natives, asked, declared);
    let mut b = super::program_image::Builder::on(program);
    // A rule the program holds is not copied again: by its contents' hash (`RuleRef::hash`, of its texts), as
    // `manager::rebase` matches rules, where the tree's `contains` compares them.
    let mut held: std::collections::HashSet<u64> = program.rules().map(|r| r.hash()).collect();
    for r in library.rules().filter(written).filter(|r| take.contains(library.rel_name(r.head().rel()))) {
        if held.insert(r.hash()) {
            b.copy(library, r);
        }
    }
    // Their declarations, natives, facts and lattices, where the program has none of its own.
    for rel in library.declared_names().into_iter().filter(|r| take.contains(r)) {
        if !program.declares(&rel) {
            let cols = library.columns_of(&rel).unwrap_or_default();
            let pairs: Vec<(&str, &str)> = cols.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
            FieldsMut::declare(&mut b.fields(), &rel, &pairs);
        }
    }
    let mut natives = ProgramFields::natives(&**program);
    let n = natives.len();
    for (rel, native) in lib_natives.iter().filter(|(r, _)| take.contains(*r)) {
        natives.entry(rel.clone()).or_insert_with(|| native.clone());
    }
    if natives.len() != n {
        b.fields().set_field(Field::Natives(&natives));
    }
    // A relation the program states facts of has the library's too, as the program read whole holds both.
    for rel in library.fact_names().into_iter().filter(|r| take.contains(r)) {
        let mut rows = program.fact_rows(&rel);
        let n = rows.len();
        for row in library.fact_rows(&rel) {
            if !rows.contains(&row) {
                rows.push(row);
            }
        }
        if rows.len() != n || !program.states_facts(&rel) {
            b.fields().set_facts(&rel, &rows);
        }
    }
    let mut lattice = ProgramFields::lattice(&**program);
    let n = lattice.len();
    for (rel, d) in ProgramFields::lattice(&**library).into_iter().filter(|(r, _)| take.contains(r)) {
        lattice.entry(rel).or_insert(d);
    }
    if lattice.len() != n {
        b.fields().set_field(Field::Lattice(&lattice));
    }
    b.finish_overlay()
}

/// The module linked in `scoped` (`lower::link_seeded_parts`), its own program `own_v`, appended onto
/// the scope's version and through the lowering's tail.
fn link_seeded_version(linked: lower::Linked, own_v: std::sync::Arc<super::program_image::Version>, scoped: &Scoped, core: &BTreeSet<String>, library: Option<&Library>) -> Result<std::sync::Arc<super::program_image::Version>, Error> {
    use super::program_image::Builder;
    let seeds = &scoped.seeds;
    let lower::Linked { lowered, stated, own_stated, .. } = linked;
    let v = lowering_tail_on(scoped.version()?, vec![(own_v, stated, own_stated)], true, lowered, library)?;
    let mut b = Builder::on(&v);
    lower::unseed(&mut b.fields(), seeds, core, library.map(|l| &*l.version));
    b.finish().map(|(v, _)| v).map_err(Error::Syntax)
}


/// `statements` lowered as one module, its rules written in the walk, and linked alone on versions
/// (`link_library_versions`).
pub fn lower_statements_version(statements: Vec<Stmt>) -> Result<std::sync::Arc<super::program_image::Version>, Error> {
    link_library_versions(vec![lower::lower_module_in_written(statements, &[])?])
}

/// A library's modules linked on versions (decision 0377): `lower::link_versions`, then the lowering's tail
/// (`lowering_tail_on`), folded to a base. The program `lower::link` makes of them as a tree, as a version.
pub fn link_library_versions(modules: Vec<lower::Module>) -> Result<std::sync::Arc<super::program_image::Version>, Error> {
    let linked = lower::link_versions(modules)?;
    let version = lowering_tail_on(linked.first, linked.appended, false, linked.lowered, None)?;
    if version.base_bytes().is_some() { Ok(version) } else { version.fold().map_err(Error::Syntax) }
}

/// The lowering's whole-program half on versions (`lower::whole` on a tree): the modules `appended` onto `start`
/// (`lowering_tail::AppendModule`), checked declared where `declared` (a text's link: `link_seeded`), then the passes
/// `whole` runs, in its order, from what the lowerer knows of the modules (`lowered`).
fn lowering_tail_on(start: std::sync::Arc<super::program_image::Version>, appended: Vec<(std::sync::Arc<super::program_image::Version>, usize, usize)>, declared: bool, lowered: lower::Lowered, library: Option<&Library>) -> Result<std::sync::Arc<super::program_image::Version>, Error> {
    use super::program_image::manager::Pipeline;
    let lower::Lowered { sets, rewritten, docof_of_locs, findings, three, modal, .. } = lowered;
    let mut p = Pipeline::new("the lowering's tail", start);
    {
        use super::plan::corpus::preparation::{lowering_tail, on_request::SourcesOnRequest};
        // The modules appended, in order, and checked declared where the tree's lowering checks it.
        for (own, stated, own_stated) in appended {
            p.program(&mut lowering_tail::AppendModule { own, stated, own_stated });
        }
        if declared {
            p.program(&mut lowering_tail::Declared);
        }
        p.program(&mut lowering_tail::ThreeValued { three: three.clone(), modal: modal.clone() });
        p.program(&mut lowering_tail::Findings { pending: findings.clone(), library: library.map(|l| std::sync::Arc::clone(&l.version)) });
        p.program(&mut lowering_tail::CoreDecls { docof_of_locs }).program(&mut lowering_tail::SpanPatterns { library: library.map(|l| (l.spans(), l.version.digest())) }).program(&mut lowering_tail::GrammarNames).program(&mut lowering_tail::BuiltinsOut { rewritten: rewritten.clone() });
        p.program(&mut lowering_tail::KindSets { sets: sets.clone(), library: library.map(|l| std::sync::Arc::clone(&l.version)) }).program(&mut lowering_tail::Unit);
        p.program(&mut lowering_tail::BooleanRelations).program(&mut lowering_tail::ScopeGlobs);
        for round in lower::ON_REQUEST {
            p.program(&mut SourcesOnRequest { round });
        }
        p.program(&mut lowering_tail::OrderNodes).program(&mut lowering_tail::DeclareAxes);
        // `finish`'s checks, on the version.
        super::front::finish_checks_given(p.current().map_err(Error::Syntax)?, library.map(|l| &*l.version))?;
    }
    {
        use super::plan::corpus::preparation::{DropRepeated, lowering_tail};
        let _demand = crate::code_rule_dsl::logic::profile::span(|| "aggregate demand and sets".to_owned());
        p.program(&mut lowering_tail::AggregateDemand).program(&mut lowering_tail::Unit).program(&mut lowering_tail::LiteralsOnce).program(&mut lowering_tail::DropSelfRules).program(&mut DropRepeated::default()).program(&mut lowering_tail::FactsOnce);
    }
    for _ in 0..16 {
        let mut widen = super::plan::corpus::preparation::lowering_tail::WidenCyclicContexts::default();
        p.program(&mut widen);
        p.current().map_err(Error::Syntax)?;
        if !widen.changed {
            break;
        }
    }
    let (v, _) = p.finish().map_err(Error::Syntax)?;
    Ok(v)
}

/// The alias the library linked from modules `keys` is kept under.
/// The values of the switches that change lowering (`plan::corpus::COMBINING`: `RERAIL_NODE_SOURCES` in
/// `on_request::SourcesOnRequest`, `RERAIL_ORDER_RAW` and `RERAIL_NODE_IDS` in `lowering_tail::OrderNodes`), for an image's
/// key: an image lowered under one value is not another's. Only those set: an unset switch keeps the key every run
/// has had, and the engine's shipped images (decision 0264) with it.
fn switched(bytes: &mut Vec<u8>) {
    for switch in super::plan::corpus::COMBINING {
        if let Some(v) = std::env::var_os(switch) {
            bytes.extend_from_slice(format!("\0{switch}={}", v.to_string_lossy()).as_bytes());
        }
    }
}

fn linked_key(keys: &[crate::content_store::HashKey]) -> crate::content_store::HashKey {
    let mut bytes = format!("rerail linked library {}\0{}\0", super::program_image::FORMAT, crate::code_rule_dsl::build_ids::engine()).into_bytes();
    switched(&mut bytes);
    for k in keys {
        bytes.extend_from_slice(&k.sha256_bytes().unwrap_or_default());
    }
    crate::content_store::HashKey::sha256(&bytes)
}

/// The alias a program's image is kept under (`Text::lower_in`): its text and the keys of the modules in its scope.
fn program_key(src: &str, scope: &[crate::content_store::HashKey]) -> crate::content_store::HashKey {
    let digest = crate::content_store::HashKey::sha256(src.as_bytes()).sha256_bytes().unwrap_or_default();
    let mut bytes = format!("rerail program image {}\0{}\0", super::program_image::FORMAT, crate::code_rule_dsl::build_ids::engine()).into_bytes();
    switched(&mut bytes);
    bytes.extend_from_slice(&digest);
    bytes.push(0);
    for k in scope {
        bytes.extend_from_slice(&k.sha256_bytes().unwrap_or_default());
    }
    crate::content_store::HashKey::sha256(&bytes)
}

/// The alias a module's image is kept under (`library_modules`).
fn module_key<'k>(files: impl Iterator<Item = &'k [u8; 32]>, scope: impl Iterator<Item = &'k crate::content_store::HashKey>) -> crate::content_store::HashKey {
    let mut bytes = format!("rerail module image v2 (helpers made once) {}\0{}\0", super::program_image::FORMAT, crate::code_rule_dsl::build_ids::engine()).into_bytes();
    switched(&mut bytes);
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
    pub(in crate::code_rule_dsl) statements: Vec<Stmt>,
}

/// What [`TextRead::resolved`] reads off a text: its library files (includes resolved, `core.rerail` first unless the
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
    let statements = RrFile::read(src)?.statements();
    let includes: Vec<(i64, String)> = include_names(&statements).into_iter().enumerate().map(|(k, (n, _))| (k as i64, n)).collect();
    let seed = (!includes.is_empty() && !phase0::active()).then(|| {
        phase0::Seed::Text { t_shape: t_shape(&statements), includes, phases: phases_named(&statements) }
    });
    Ok(TextRead { seed, statements })
}

impl TextRead {
    /// The text's library files, names and phase, its includes resolved in the graph phase 0 derived for its seed.
    pub fn resolved(self) -> Result<TextNames, Error> {
        let (files, own) = if self.statements.iter().any(is_include) { resolve(self.statements)? } else { (Vec::new(), self.statements) };
        let (mut names, mut phase) = (Vec::new(), None);
        for s in files.iter().flat_map(|(_, st)| st.iter()).chain(own.iter()) {
            match s.what() {
                Some(S::Decl(name)) => names.push(name.to_owned()),
                Some(S::Setting { key, text }) if key == "phase" && phase.is_none() => phase = Some(text.trim().to_owned()),
                _ => {}
            }
        }
        names.sort();
        names.dedup();
        let mut included: Vec<PathBuf> = files.into_iter().map(|(p, _)| p).collect();
        // The engine's own library programs, which the runner adds to a run (decision 0076's components).
        included.extend(ENGINE_FILES.iter().filter_map(|f| library().join(f).canonical().ok()));
        Ok(TextNames { included, names, phase })
    }
}

impl TextNames {
    /// A program already read: the relations it declares and the phase it names, and the engine's library programs
    /// (it includes nothing of its own: its libraries are in it), as a print of it reads (decision 0249).
    pub fn of_program(prog: &(impl super::program_image::fields::ProgramFields + ?Sized)) -> TextNames {
        let names = prog.declared_names();
        let phase = prog.setting_values("phase").into_iter().next();
        let included = ENGINE_FILES.iter().filter_map(|f| library().join(f).canonical().ok()).collect();
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

/// `statements`' `#include`s resolved in the library: its files (`core.rerail` first unless the statements are in T's
/// shape, each once, includes first) and the statements without the includes.
///
/// Phase 0 resolves them (`plan::corpus::phase0`): `bootstrap.rerail`'s run loads each file as a document, and the
/// files are read here off their stored tapes, in the graph's order.
fn resolve(statements: Vec<Stmt>) -> Result<(Files, Vec<Stmt>), Error> {
    let (files, own) = resolve_shared(statements)?;
    Ok((files.into_iter().map(|(path, _, statements)| (path, statements.into_iter().filter(|s| !is_include(s)).collect())).collect(), own))
}

/// `resolve`, each file's statements (its includes still in them) with its bytes' digest.
type SharedFiles = Vec<(PathBuf, [u8; 32], Vec<Stmt>)>;

fn resolve_shared(statements: Vec<Stmt>) -> Result<(SharedFiles, Vec<Stmt>), Error> {
    use super::plan::corpus::phase0;
    if phase0::active() {
        let (files, own) = builtin_resolve(statements)?;
        let mut out = Vec::new();
        for (path, statements) in files {
            let (_, file) = read_shared(&path)?;
            out.push((path, file.sha256.unwrap_or_default(), statements));
        }
        return Ok((out, own));
    }
    let (includes, own): (Vec<_>, Vec<_>) = statements.into_iter().partition(is_include);
    let names: Vec<(i64, String, usize)> = include_names(&includes).into_iter().enumerate().map(|(k, (name, line))| (k as i64, name, line)).collect();
    let g = phase0::graph(&phase0::Seed::Text { t_shape: t_shape(&own), includes: names.iter().map(|(k, n, _)| (*k, n.clone())).collect(), phases: phases_named(&own) })?;
    if let Some((_, at, name)) = g.missing.iter().find(|(f, _, _)| f == phase0::TEXT) {
        let line = names.iter().find(|(k, _, _)| k == at).map_or(0, |(_, _, l)| *l);
        return Err(Error::Syntax(format!("line {line}: #include {name:?}: no such file in the library")));
    }
    let mut order = Vec::new();
    g.order(phase0::TEXT, &mut order)?;
    let files = order
        .into_iter()
        .map(|p| {
            let (rr_file, file) = read_shared(&p)?;
            Ok((p, file.sha256.unwrap_or_default(), rr_file.statements()))
        })
        .collect::<Result<_, Error>>()?;
    Ok((files, own))
}

/// The phase programs a text's own declarations name (`<phase>__<export>`, as `phases::phase_files` reads them): not
/// its own phase (`.phase`), a typing program's or the engine's (`typing_`, `rr_`). Phase 0 resolves them with its
/// includes (`p0_text_phase`), so the run's rule files are listed without reading a module again (decision 0246).
pub fn phases_named(statements: &[Stmt]) -> Vec<String> {
    let own = statements.iter().find_map(|s| match s.what()? {
        S::Setting { key: "phase", text } => Some(text.trim().to_owned()),
        _ => None,
    });
    let mut out: Vec<String> = statements
        .iter()
        .filter_map(|s| match s.what()? {
            S::Decl(name) => name.split_once("__"),
            _ => None,
        })
        .filter(|(phase, export)| !phase.is_empty() && !export.is_empty() && own.as_deref() != Some(*phase) && !phase.starts_with("typing_") && !phase.starts_with("rr_"))
        .map(|(phase, _)| phase.to_owned())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The engine's own library programs a run is given besides its includes (`TextNames::included`), canonical.
pub fn engine_files() -> Vec<PathBuf> {
    ENGINE_FILES.iter().filter_map(|f| library().join(f).canonical().ok()).collect()
}

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
    let statements = RrFile::read(src)?.statements();
    let names: Vec<(i64, String)> = include_names(&statements).into_iter().enumerate().map(|(k, (n, _))| (k as i64, n)).collect();
    if names.is_empty() || phase0::active() {
        return Ok(None);
    }
    Ok(Some(phase0::Seed::Text { t_shape: t_shape(&statements), includes: names, phases: phases_named(&statements) }))
}

/// The program's module graph's digest (`phase0::Graph::digest`): what its text is a function of besides itself.
/// `None` for a text that includes nothing.
pub(in crate::code_rule_dsl) fn graph_digest(src: &str) -> Result<Option<[u8; 32]>, Error> {
    match text_seed(src)? {
        Some(seed) => Ok(Some(super::plan::corpus::phase0::graph(&seed)?.digest())),
        None => Ok(None),
    }
}

/// The resolution phase 0's own program (`bootstrap.rerail`) is read with: the library, then beside the includer.
fn builtin_resolve(statements: Vec<Stmt>) -> Result<(Files, Vec<Stmt>), Error> {
    let (includes, own): (Vec<_>, Vec<_>) = statements.into_iter().partition(is_include);
    let core = library().join("core.rerail").canonical().map_err(|e| Error::Syntax(format!("core.rerail: {e}")))?;
    let (mut out, mut read_files, mut seen) = (Vec::new(), Vec::new(), Vec::new());
    if t_shape(&own) {
        seen.push(core);
    } else {
        visit(&core, &mut out, &mut read_files, &mut seen, false, &mut |_, _| {})?;
    }
    for (name, line) in include_names(&includes) {
        let path = library().join(&name);
        if !path.exists() {
            return Err(Error::Syntax(format!("line {line}: #include {name:?}: no such file in the library")));
        }
        visit(&path, &mut out, &mut read_files, &mut seen, true, &mut |_, _| {})?;
    }
    Ok((out, own))
}


/// The compiler's named policy headers and settings, without reparsing rule source in a host adapter.
pub fn metadata(program: &(impl super::program_image::fields::ProgramFields + ?Sized)) -> Result<serde_json::Value, Error> {
    use syntax::Tok;
    let mut policies = program.policies();
    let settings = program.settings();
    for (_, text) in settings.iter().filter(|(key, _)| key == "policy") {
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
    Ok(serde_json::json!({"policies": policies, "settings": settings}))
}

/// A lowered program run over files a caller gives (a test's), not found by walking a repository: its
/// `.scope` filter on findings is taken out, as `.scope given` leaves it out when lowering.
pub fn files_given(v: &std::sync::Arc<super::program_image::Version>) -> Result<std::sync::Arc<super::program_image::Version>, Error> {
    use super::program_facts::Field;
    use super::program_image::{Builder, LitRef, Rewrite, fields::ProgramFields};
    const FILTER: [&str; 3] = ["rr_scope_drop", "rr_scope_in", "rr_report_in"];
    struct Unfiltered<'a>(&'a super::program_image::Version);
    impl Rewrite for Unfiltered<'_> {
        fn keep(&mut self, l: LitRef<'_>) -> bool {
            !l.atom().is_some_and(|a| FILTER.contains(&self.0.rel_name(a.rel())))
        }
    }
    let mut b = Builder::derive(v);
    for r in v.rules().filter(|r| !FILTER.contains(&v.rel_name(r.head().rel()))) {
        b.rewrite(v, r, &mut Unfiltered(v)).finish().map_err(Error::Syntax)?;
    }
    for f in FILTER.iter().filter(|f| v.declares(f)) {
        b.fields().undeclare(f);
    }
    let mut settings = ProgramFields::settings(&**v);
    settings.push(("scope".to_owned(), "given".to_owned()));
    b.fields().set_field(Field::Settings(&settings));
    b.fields().set_field(Field::DocScopes(None));
    Ok(b.finish().map_err(Error::Syntax)?.0)
}

/// A program's `.rerail` file, with what it includes.
pub fn parse_file_version(path: &Path) -> Result<std::sync::Arc<super::program_image::Version>, Error> {
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

/// The program of a `.rerail` file and the library files it read or probed, in reading order (core.rerail, then
/// each file before what it includes).
///
/// The lowered program is a derivation of those files (0044 §4): it is kept for the process and
/// reused while every file it read hashes the same, so only an edit to a file it read (or a
/// file appearing where an include looked) reads and lowers it again.
/// The relations `core.rerail` declares for libraries to derive for the engine to read: the documents they load (t21 E2).
pub const LIBRARY_DERIVED: [&str; 1] = ["load"];
/// The relations `core.rerail` declares that one library file derives, and that file (decision 0483: the reference graph).
pub const DERIVED_BY: [(&str, &str); 2] = [("reference_edge", "python/references.rerail"), ("reference_open", "python/references.rerail")];
/// The library's programs the engine adds to a run itself (decision 0076's import components).
const ENGINE_FILES: [&str; 1] = ["components.rerail"];

pub fn read_file(path: &Path) -> Result<(std::sync::Arc<super::program_image::Version>, Vec<LibraryFile>), Error> {
    read_file_after(path, None)
}

pub fn read_file_after(path: &Path, inspected: Option<&[LibraryFile]>) -> Result<(std::sync::Arc<super::program_image::Version>, Vec<LibraryFile>), Error> {
    let read = inspect_file_after(path, inspected)?;
    Ok((read.program, read.files))
}

/// A rule file's program as `inspect_file_after` reads it: its version, the library files it read, and, for a policy
/// read from a Git view, the inputs its include graph is kept under.
pub struct ReadProgram {
    pub program: std::sync::Arc<super::program_image::Version>,
    pub files: Vec<LibraryFile>,
    pub git: Option<GitPolicyInputs>,
}

/// A repeated inspection in the same catalog can use the reader's current in-process program.
/// Execution attaches the inspected graph to its destination store, including after collection.
pub fn inspect_file_after(path: &Path, inspected: Option<&[LibraryFile]>) -> Result<ReadProgram, Error> {
    if let Some(root) = crate::git_workspace::snapshot::view_root(path) {
        return git_policy::read(&root, path);
    }
    let (program, files) = read_worktree_file(path, inspected)?;
    Ok(ReadProgram { program, files, git: None })
}

fn read_worktree_file(path: &Path, inspected: Option<&[LibraryFile]>) -> Result<(std::sync::Arc<super::program_image::Version>, Vec<LibraryFile>), Error> {
    use std::collections::HashMap;
    use std::sync::Mutex;
    // The shared object's lowered programs (decision 0442).
    #[derive(Default)]
    struct Programs(Mutex<HashMap<(PathBuf, PathBuf), (Vec<LibraryFile>, std::sync::Arc<super::program_image::Version>)>>);
    let key = path.canonical().map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    let cache_held = crate::code_rule_dsl::logic::profile::shared::<Programs>();
    let cache = &cache_held.0;
    let library = library_root();
    let entry = (key.clone(), library.clone());
    let cached = cache.lock().expect("the program cache").get(&entry)
        .filter(|(files, _)| crate::code_rule_dsl::prof::time("rr.inspect.inputs", || LibraryFile::inputs_match(files))).cloned();
    if let Some((files, program)) = cached {
        if inspected != Some(files.as_slice()) {
            retain_current_read(&key, &library, &files, &program);
        }
        files.iter().for_each(noted);
        crate::code_rule_dsl::prof::count("rr.program_reused", 1);
        return Ok((program, files));
    }
    // Decision 0275: rederived until a key hits. The file's sha (by its `stat` while that holds), then the entry kept
    // under its path and sha, taken while every file it read still reads as recorded (by `stat`, then sha).
    let kept = kept_program(&key, &library);
    if let Some((_, _, Some((program, files)))) = &kept {
        crate::code_rule_dsl::prof::count("rr.program_stored", 1);
        files.iter().for_each(noted);
        cache.lock().expect("the program cache").insert(entry, (files.clone(), program.clone()));
        return Ok((program.clone(), files.clone()));
    }
    // The kept statements alone copied from the process's (`read_shared`): each policy copied its libraries whole,
    // to drop most of them (`unreached_dropped`).
    let (shared, files) = program_shared(path)?;
    let own = path.canonical().unwrap_or_else(|_| path.to_owned());
    let statements = reached_statements(&own, &shared.iter().collect::<Vec<_>>());
    let program = lower_files_dropping(path, statements, false)?;
    // Kept behind the run (the store's writer), with what it read: the next run's check above.
    if let Some((entry, Some(store), None)) = kept {
        super::plan::corpus::combined::record_version(&store, "a rule file's program", &entry, &files, &program, &[]);
        super::plan::corpus::combined::read_before_the_run(entry);
    }
    cache.lock().expect("the program cache").insert(entry, (files.clone(), program.clone()));
    Ok((program, files))
}

/// `read_text(src)?.header()`, rederived until a key hits (decision 0275): the header kept under the text's bytes,
/// taken while every library file its includes read still reads as recorded (by `stat`, then sha). A batch reads
/// each policy's header before its run (`tools::policy_groups`): 44 policies parsed, checked and lowered again each
/// run, over the same libraries.
pub fn text_header(src: &str) -> Result<std::sync::Arc<super::program_image::Version>, Error> {
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
        combined::record_version(&store, "a text's header", &key, &read, &program, &[]);
        combined::read_before_the_run(key);
    }
    Ok(program)
}

/// The entry of text `src`'s header: its key, the store it is kept in, and the header with its read record when the
/// store keeps one that still holds.
#[allow(clippy::type_complexity)]
fn kept_header(src: &str) -> ([u8; 32], Option<std::sync::Arc<crate::content_store::ContentStore>>, Option<(std::sync::Arc<super::program_image::Version>, Vec<LibraryFile>)>) {
    use super::plan::corpus::{combined, phases};
    let key = combined::text_header_key(src);
    let store = (cfg!(feature = "tape-cache") && !super::plan::corpus::phase0::active()).then(phases::reading_store).flatten();
    let found = store.as_ref().and_then(|store| {
        let _held = store.hold_collection().ok()?;
        store.settle(&combined::queued(&key));
        combined::lookup_read(store, &key).ok().flatten()
    });
    (key, store, found)
}

/// The header the store keeps for text `src`, when it still holds (`kept_header`): for the tests of decision 0275.
#[cfg(any(test, feature = "test-hooks"))]
pub fn kept_header_found(src: &str) -> Option<std::sync::Arc<super::program_image::Version>> {
    kept_header(src).2.map(|(program, _)| program)
}

/// The program and read record the store keeps for the rule file at `path` (canonical), when they still hold
/// (`kept_program`): for the tests of decision 0275.
#[cfg(any(test, feature = "test-hooks"))]
pub fn kept_program_found(path: &Path) -> Option<(std::sync::Arc<super::program_image::Version>, Vec<LibraryFile>)> {
    kept_program(path, &library_root()).and_then(|(_, _, found)| found)
}

/// Attach an inspected program to the store selected by its consumer. The inputs belong to that inspection.
pub fn retain_read_program(path: &Path, library: &Path, files: &[LibraryFile], program: &std::sync::Arc<super::program_image::Version>) {
    if LibraryFile::inputs_match(files) {
        retain_current_read(path, library, files, program);
    }
}

fn retain_current_read(path: &Path, library: &Path, files: &[LibraryFile], program: &std::sync::Arc<super::program_image::Version>) {
    use super::plan::corpus::combined;
    let Some(sha) = files.iter().find(|file| file.path == path).and_then(|file| file.sha256) else { return };
    let Some((key, Some(store))) = rule_file_store(path, library, &sha) else { return };
    let Ok(_held) = crate::code_rule_dsl::prof::time("rr.inspect.collection_lease", || store.hold_collection()) else { return };
    crate::code_rule_dsl::prof::time("rr.inspect.settle", || store.settle(&combined::queued(&key)));
    if !combined::retains_read(&store, &key, files).unwrap_or(false) {
        combined::record_version(&store, "a rule file's program", &key, files, program, &[]);
    }
    combined::read_before_the_run(key);
}

fn rule_file_store(path: &Path, library: &Path, sha: &[u8; 32]) -> Option<([u8; 32], Option<std::sync::Arc<crate::content_store::ContentStore>>)> {
    use super::plan::corpus::{combined, phases};
    if !cfg!(feature = "tape-cache") || super::plan::corpus::phase0::active() { return None; }
    Some((combined::rule_file_key(path, library, sha), crate::code_rule_dsl::prof::time("rr.inspect.store", phases::reading_store)))
}

/// The entry of the rule file at `path` and the program whose recorded inputs still hold.
#[allow(clippy::type_complexity)]
fn kept_program(path: &Path, library: &Path) -> Option<([u8; 32], Option<std::sync::Arc<crate::content_store::ContentStore>>, Option<(std::sync::Arc<super::program_image::Version>, Vec<LibraryFile>)>)> {
    use super::plan::corpus::combined;
    let sha = super::plan::corpus::observe(|o| o.digest(path))?;
    let (key, store) = rule_file_store(path, library, &sha)?;
    let found = if let Some(store) = &store {
        let _held = store.hold_collection().ok()?;
        store.settle(&combined::queued(&key));
        combined::lookup_read(store, &key).ok().flatten()
    } else {
        None
    };
    if found.is_some() {
        combined::read_before_the_run(key);
    }
    Some((key, store, found))
}

/// A file's program from its modules' statements (`program`): its names checked, its own rules grounded, and lowered,
/// as `read_file` reads it. Testing mode (`rr::testing`, decision 0130) gives the file's statements with a test's
/// spliced in.
pub fn lower_files(path: &Path, statements: Files) -> Result<std::sync::Arc<super::program_image::Version>, Error> {
    lower_files_dropping(path, statements, true)
}

/// `lower_files`, with the unreached clauses of included files dropped first (`drop`) or lowered too.
pub fn lower_files_dropping(path: &Path, statements: Files, drop: bool) -> Result<std::sync::Arc<super::program_image::Version>, Error> {
    let key = crate::git_workspace::snapshot::canonicalize(path).map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    check_names(&statements, false)?;
    // The file's own rules grounded (`refuse_ungrounded`), its includes' `fix` relations given.
    let own_path = key.clone();
    let given: BTreeSet<String> = statements.iter().filter(|(p, _)| *p != own_path).flat_map(|(_, s)| check::fix_relations(s)).collect();
    for (_, own) in statements.iter().filter(|(p, _)| *p == own_path) {
        refuse_ungrounded(own, &given)?;
    }
    // The included files' clauses nothing in the program reaches: dropped before they are checked and lowered.
    let statements = if drop { unreached_dropped(&own_path, statements) } else { statements };
    token_reads::refuse(&statements)?;
    // The relations an included file declares are the library's: canonical, one copy however many policies of a
    // combined run include it (decision 0091). Printed as `.global library …`, so a program's text keeps that.
    let core = library().join("core.rerail").canonical().map_err(|e| Error::Syntax(format!("core.rerail: {e}")))?;
    let library: Vec<String> = statements
        .iter()
        .filter(|(file, _)| *file != core && *file != key)
        .flat_map(|(_, statements)| statements)
        .filter_map(|s| if let Some(S::Decl(name)) = s.what() { Some(name.to_owned()) } else { None })
        .collect();
    let program = lower_statements_version(statements.into_iter().flat_map(|(_, statements)| statements).collect())?;
    if library.is_empty() {
        return Ok(program);
    }
    let mut b = super::program_image::Builder::on(&program);
    b.fields().add_setting(super::front::GLOBAL, &format!("{} {}", super::front::LIBRARY, library.join(", ")));
    b.finish().map(|(v, _)| v).map_err(Error::Syntax)
}

/// `statements` less the included files' clauses that nothing in the program reaches. Of the 44 data-layer
/// policies' rules, 76% were lowered (with the libraries' whole) and then pruned (`preparation::Prune`): reading a policy
/// was the largest part of a batch's setup (`lower_files`, 16% of the main thread). Kept whole: the file itself,
/// `core.rerail` and the engine's files (lowering reads their relations by name), and every item but a clause. A clause
/// of another file is kept when one of its heads is reached: named by the file itself (each identifier of each of
/// its items, so none is missed), by another file's output, violation or setting, by the engine (`LIBRARY_DERIVED`),
/// by its own body when that makes a request (`asks`), or read by a kept clause. Every clause for a relation is kept with any, so a library relation is the same in each
/// program that reaches it (decision 0091). `preparation::Prune` of the program lowered from these is the same as of the whole
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
fn reached_statements(own: &Path, statements: &[(&PathBuf, Vec<&Stmt>)]) -> Files {
    fn idents(text: &str, out: &mut BTreeSet<String>) {
        out.extend(text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).filter(|w| w.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')).map(str::to_owned));
    }
    // What a clause reads, off its tape: its body's and edits' names, or (`whole`) its heads' too.
    fn reads(s: &Stmt, whole: bool, out: &mut BTreeSet<String>) {
        let (t, n) = s.tape();
        if whole {
            t.names_in(n, out);
            return;
        }
        for c in t.all(n, tape::Field::Body).chain(t.all(n, tape::Field::Edit)) {
            t.names_in(c, out);
        }
    }
    let whole: Vec<PathBuf> = ["core.rerail"].iter().chain(ENGINE_FILES.iter()).filter_map(|f| library().join(f).canonical().ok()).chain(std::iter::once(own.to_owned())).collect();
    // The relations a library derives for the engine to read (`LIBRARY_DERIVED`: loads) are reached: no rule
    // reads them, and the corpus run makes them outputs (`corpus::run_observed`'s requests). Dropped, a library's
    // `load(…)` asked for no document, and no import was resolved.
    let mut reached: BTreeSet<String> = LIBRARY_DERIVED.iter().map(|r| (*r).to_owned()).collect();
    // Each other file's clauses by head: (file, statement) indices.
    let mut by_head: std::collections::HashMap<&str, Vec<(usize, usize)>> = std::collections::HashMap::new();
    for (f, (file, items)) in statements.iter().enumerate() {
        let kept_whole = whole.contains(file);
        for (s, &st) in items.iter().enumerate() {
            match st.what() {
                Some(S::Clause) if !kept_whole => {
                    let heads = st.heads();
                    heads.iter().for_each(|h| by_head.entry(h).or_default().push((f, s)));
                    // A clause whose reads make requests (`asks`) is reached though nothing reads its heads: the engine
                    // answers its request for every reader (`python/modules.rerail`'s `py_listed` lists the directories the
                    // other rules read entries of).
                    let mut read = BTreeSet::new();
                    reads(st, false, &mut read);
                    if read.iter().any(|r| asks(r)) {
                        reached.extend(heads.iter().map(|h| (*h).to_owned()));
                    }
                }
                Some(S::Clause) => reads(st, true, &mut reached),
                // What another file declares reads nothing (`.decl`, `.input`, `.stored`, `.functor`, `.type`, `.edit`):
                // its relations are reached only when read. Its outputs, violations and settings are read.
                Some(S::Decl(_) | S::Functor(_) | S::Type(_) | S::Edit(_) | S::Lattice(_) | S::Include(_) | S::Io { output: false }) if !kept_whole => {}
                _ => idents(st.text(), &mut reached),
            }
        }
    }
    let mut keep: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut todo: Vec<String> = reached.iter().cloned().collect();
    while let Some(rel) = todo.pop() {
        // An answer read reaches its request: a request of several columns is a rule's (`c_pp_tu_req(tu, spec)`),
        // which nothing reads by name.
        for (_, req) in WORLD.iter().filter(|(a, _)| *a == rel) {
            if reached.insert((*req).to_owned()) {
                todo.push((*req).to_owned());
            }
        }
        for &(f, s) in by_head.get(rel.as_str()).into_iter().flatten() {
            if !keep.insert((f, s)) {
                continue;
            }
            let mut more = BTreeSet::new();
            reads(statements[f].1[s], true, &mut more);
            todo.extend(more.into_iter().filter(|r| reached.insert(r.clone())));
        }
    }
    statements
        .iter()
        .enumerate()
        .map(|(f, (file, items))| {
            let kept_whole = whole.contains(*file);
            let items = items.iter().copied().enumerate().filter(|(s, st)| kept_whole || !matches!(st.what(), Some(S::Clause)) || keep.contains(&(f, *s))).map(|(_, i)| i.clone()).collect();
            ((*file).clone(), items)
        })
        .collect()
}

/// What a program asks guests of when it runs alone (decisions 0055, 0058 §1): a pack's programs make one set of
/// guests, and a rule that reaches a guest reads every one the run made (an annotation one program's library asked
/// for was every program's, `C408` in a `Literal["dict()"]`). Programs that ask the same run as one pack
/// (`tools::policy_groups`); the rest run in packs of their own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestAsks {
    /// Its `.guests` settings: `guest_policy`'s facts, which a pack's programs share.
    pub settings: Vec<String>,
    /// `None` when no rule it reaches reads a guest: alone it makes none (`corpus`'s guest requests are no outputs).
    /// Else the library files whose clauses ask (`asks`): an included file's asking clauses are kept
    /// whatever reads them (`reached_statements`), so these files' clauses are what it asks for.
    pub files: Option<Vec<PathBuf>>,
    /// Its own clauses ask too: what it asks for is its own.
    pub own: bool,
}

/// `src`'s guest requests (`GuestAsks`), read from its statements and its includes'.
pub fn guest_asks(src: &str) -> Result<GuestAsks, Error> {
    use super::plan::corpus::guests::READS;
    let text = read_text(src)?;
    let (files, own): (Vec<(&Path, &[Stmt])>, &[Stmt]) = match &text.0 {
        Read::Plain(statements) => (Vec::new(), statements),
        Read::Modular { files, own } => (files.iter().map(|(path, _, statements)| (path.as_path(), statements.as_slice())).collect(), own),
    };
    let settings: Vec<String> = own.iter().filter_map(|st| match st.what() {
        Some(S::Setting { key: "guests", text }) => Some(text.to_owned()),
        _ => None,
    }).collect();
    // What a clause reads, off its tape, and whether it makes a guest request (a read `lower` makes a demand of): its
    // heads' arguments', its body's and its edits' names.
    let read = |st: &Stmt| -> BTreeSet<String> {
        let (t, n) = st.tape();
        let mut out = BTreeSet::new();
        for h in t.all(n, tape::Field::Head) {
            for a in t.kids(h).filter(|&a| t.field(a) != tape::Field::Relation as u16) {
                t.names_in(a, &mut out);
            }
        }
        for c in t.all(n, tape::Field::Body).chain(t.all(n, tape::Field::Edit)) {
            t.names_in(c, &mut out);
        }
        out
    };
    // A clause asks the run when it reads what the run answers (not the world's own). A head of the engine's to read
    // asked too while it named an embed; the embeds are gone (0450 §3), and `LIBRARY_DERIVED`'s loads ask nothing.
    let asking = |read: &BTreeSet<String>| read.iter().any(|r| asks(r) && !WORLD.iter().any(|(w, _)| w == r));
    let reads_guest = |read: &BTreeSet<String>| read.iter().any(|r| READS.contains(&r.as_str()) || r == "parse" || r == "splice");
    let mut by_head: std::collections::HashMap<&str, Vec<BTreeSet<String>>> = std::collections::HashMap::new();
    let mut asking_files: Vec<PathBuf> = Vec::new();
    for (path, statements) in &files {
        for st in statements.iter().filter(|st| matches!(st.what(), Some(S::Clause))) {
            let (heads, r) = (st.heads(), read(st));
            if asking(&r) && !asking_files.iter().any(|p| p == path) {
                asking_files.push(path.to_path_buf());
            }
            heads.iter().for_each(|h| by_head.entry(h).or_default().push(r.clone()));
        }
    }
    // The program's own clauses read what its findings reach; a guest read among what they reach makes its requests.
    let (mut own_asks, mut reads) = (false, false);
    let mut reached: BTreeSet<String> = BTreeSet::new();
    for st in own.iter().filter(|st| matches!(st.what(), Some(S::Clause))) {
        let (heads, r) = (st.heads(), read(st));
        own_asks |= asking(&r);
        reads |= reads_guest(&r);
        reached.extend(heads.iter().map(|h| (*h).to_owned()));
        reached.extend(r);
    }
    let mut todo: Vec<String> = reached.iter().cloned().collect();
    while let Some(rel) = todo.pop().filter(|_| !reads) {
        for r in by_head.get(rel.as_str()).into_iter().flatten() {
            reads |= reads_guest(r);
            todo.extend(r.iter().filter(|n| reached.insert((*n).clone())).cloned());
        }
    }
    asking_files.sort();
    Ok(GuestAsks { settings, files: reads.then_some(asking_files), own: reads && own_asks })
}

/// The names a program's files may not declare or derive: `core.rerail`'s, but in `core.rerail` (decision 0141), and names
/// starting `rr_`, but in the engine's own library programs.
fn check_names(statements: &Files, core_unlisted: bool) -> Result<(), Error> {
    // `core.rerail`'s relations are the engine's (decision 0141): no other file declares or derives one.
    let core = library().join("core.rerail").canonical().map_err(|e| Error::Syntax(format!("core.rerail: {e}")))?;
    // From `core.rerail` among the files, else (`core_unlisted`: the program reads it) as `core_names` reads it.
    let listed = statements.iter().any(|(file, _)| *file == core);
    let engine: std::collections::BTreeSet<&str> = statements
        .iter()
        .filter(|(file, _)| *file == core)
        .flat_map(|(_, statements)| statements)
        .filter_map(|s| if let Some(S::Decl(name)) = s.what() { Some(name) } else { None })
        .chain(core_names().iter().map(String::as_str).filter(|_| core_unlisted && !listed))
        .filter(|name| !LIBRARY_DERIVED.contains(name))
        .collect();
    for (file, statements) in statements.iter().filter(|(file, _)| *file != core) {
        for s in statements {
            let names: Vec<&str> = match s.what() {
                Some(S::Decl(name)) => vec![name],
                Some(S::Clause) => s.heads(),
                _ => Vec::new(),
            };
            // A relation one library file derives (`DERIVED_BY`) that file's clauses may derive; it declares none.
            let derives = |n: &str| {
                DERIVED_BY.iter().any(|(rel, by)| *rel == n && library().join(by).canonical().is_ok_and(|by| by == **file))
            };
            if let Some(name) = names.into_iter().find(|n| engine.contains(n) && !(matches!(s.what(), Some(S::Clause)) && derives(n))) {
                return Err(Error::Syntax(format!("{}:{}: {name} is core.rerail's: a program reads it and cannot declare or derive it", file.display(), s.line())));
            }
        }
    }
    // Names starting `rr_` are the engine's (`front::RESERVED`): its helpers, axes and places. The engine's own
    // library programs (`ENGINE_FILES`, which the runner adds; no rule includes them) may use them.
    let own: Vec<PathBuf> = ENGINE_FILES.iter().filter_map(|f| library().join(f).canonical().ok()).collect();
    for (file, statements) in statements.iter().filter(|(file, _)| !own.contains(file)) {
        for s in statements {
            let names: Vec<&str> = match s.what() {
                Some(S::Decl(name) | S::Violation(name)) => vec![name],
                Some(S::Clause) => s.heads(),
                _ => Vec::new(),
            };
            if let Some(name) = names.into_iter().find(|n| n.starts_with(super::front::RESERVED)) {
                return Err(Error::Syntax(format!("{}:{}: {name}: names starting `{}` are the engine's", file.display(), s.line(), super::front::RESERVED)));
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
    pub(in crate::code_rule_dsl) fn inputs_match(files: &[Self]) -> bool {
        files.iter().all(Self::unchanged)
    }

    /// Whether the file still reads as recorded: the same bytes, or still no file.
    pub fn unchanged(&self) -> bool {
        Self::now(&self.path) == *self
    }
}

pub fn read(path: &Path) -> Result<(Vec<Stmt>, LibraryFile), Error> {
    read_shared(path).map(|(rr_file, file)| (rr_file.statements(), file))
}

/// A library file as kept for the process (its text and tape), and the file as read.
fn read_shared(path: &Path) -> Result<(std::sync::Arc<RrFile>, LibraryFile), Error> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    // A file of a Git revision is read from its tree (bounded), not the checkout.
    if crate::git_workspace::snapshot::contains(path) {
        use sha2::Digest as _;
        let bytes = crate::git_workspace::snapshot::read_bounded(path, 1024 * 1024)
            .map_err(|error| Error::Syntax(format!("{}: {error}", path.display())))?;
        let file = LibraryFile { path: path.to_owned(), sha256: Some(sha2::Sha256::digest(&bytes).into()) };
        noted(&file);
        let src = String::from_utf8(bytes).map_err(|error| Error::Syntax(format!("{}: {error}", path.display())))?;
        return Ok((RrFile::read(src).map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?, file));
    }
    // Each file's statements, kept for the process by its path and bytes' digest: programs that include one library
    // (the Python pack's files) parse it once, not once per program. Phase 0's observer gives the digest by a `stat`
    // while the file is unchanged, so a kept file is not read again. Held shared: a reader copies what it keeps.
    #[derive(Default)]
    struct ParsedFiles(Mutex<HashMap<PathBuf, ([u8; 32], Arc<RrFile>)>>);
    let cache_held = crate::code_rule_dsl::logic::profile::shared::<ParsedFiles>();
    let cache = &cache_held.0;
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
    let statements = RrFile::read(src).map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    cache.lock().expect("the parsed files").insert(path.to_owned(), (digest, Arc::clone(&statements)));
    Ok((statements, file))
}

/// A program's files, each its path and the statements of it the program reads (positions in its tape).
pub type Files = Vec<(PathBuf, Vec<Stmt>)>;

/// Whether a statement is an `#include`.
fn is_include(s: &Stmt) -> bool {
    matches!(s.what(), Some(S::Include(_)))
}

/// Whether statements are in T's shape: they declare `doc` themselves (a lowered program printed and read again).
fn t_shape(statements: &[Stmt]) -> bool {
    statements.iter().any(|s| s.what() == Some(S::Decl("doc")))
}

/// Each `#include`'s name and line, in order.
fn include_names(statements: &[Stmt]) -> Vec<(String, usize)> {
    statements.iter().filter_map(|s| if let Some(S::Include(name)) = s.what() { Some((name, s.line())) } else { None }).collect()
}

/// `path` and what it includes, each once (`seen`), includes first, added to `out`; every library file read or
/// probed, to `read_files`. An `included` file's `.output`s are left out, as are those of what it includes.
fn visit(requested: &Path, out: &mut Files, read_files: &mut Vec<LibraryFile>, seen: &mut Vec<PathBuf>, included: bool, resolved: &mut impl FnMut(&Path, &Path)) -> Result<(), Error> {
    use crate::git_workspace::snapshot as source;
    let path = source::canonicalize(requested).map_err(|e| Error::Syntax(format!("{}: {e}", requested.display())))?;
    resolved(requested, &path);
    if seen.contains(&path) {
        return Ok(());
    }
    seen.push(path.clone());
    let (statements, file) = read(&path)?;
    read_files.push(file);
    let (includes, mut rest): (Vec<_>, Vec<_>) = statements.into_iter().partition(is_include);
    if included {
        rest.retain(|s| s.what() != Some(S::Io { output: true }));
    }
    for (name, line) in include_names(&includes) {
        let pinned = source::contains(&path);
        if pinned && Path::new(&name).is_absolute() {
            return Err(Error::Syntax(format!("{}:{line}: #include {name:?} must be relative for a Git policy", path.display())));
        }
        let in_library = library().join(&name);
        let beside = path.parent().map(|d| d.join(&name)).unwrap_or_default();
        let bundled = if pinned {
            in_library.canonical().ok().is_some_and(|file| {
                library().canonical().is_ok_and(|root| file.starts_with(root))
            })
        } else {
            in_library.exists()
        };
        let found = if bundled {
            in_library
        } else {
            let miss = LibraryFile { path: in_library, sha256: None };
            noted(&miss);
            read_files.push(miss);
            beside
        };
        if !source::is_file(&found) || pinned && !bundled && !source::contains(&found) {
            return Err(Error::Syntax(format!("{}:{line}: #include {name:?}: no such file", path.display())));
        }
        visit(&found, out, read_files, seen, true, resolved)?;
    }
    out.push((path, rest));
    Ok(())
}

/// The files of `path`'s program, each once, includes first (`path` last); and every library file read or probed,
/// in reading order.
fn program_order(path: &Path) -> Result<(Vec<PathBuf>, Vec<LibraryFile>), Error> {
    use super::plan::corpus::phase0;
    let path = path.canonical().map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    let g = phase0::graph(&phase0::Seed::File(path.clone()))?;
    let mut order = Vec::new();
    g.order(&path.to_string_lossy(), &mut order)?;
    order.retain(|p| *p != path);
    order.push(path.clone());
    // The files in reading order, as `LibraryFile`s check them: `core.rerail` (an include at offset -1), then the
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

/// Index authored clause heads for the current CLI profile. Shared library names survive
/// combination; private heads also have the policy prefix assigned by the combiner.
pub fn profile_join_sources(paths: &[&Path]) -> Option<super::profile::author_joins::Sources> {
    use super::profile::author_joins::{Locations, Sources};
    if !super::profile::on() { return None; }
    let mut locations = Locations::default();
    let mut indexed = BTreeSet::new();
    for (i, root) in paths.iter().enumerate() {
        if root.extension().is_some_and(|e| e == "json") { continue; }
        let own = root.canonical().unwrap_or_else(|_| root.to_path_buf());
        let files = program_order(&own).map(|(order, _)| order).unwrap_or_else(|_| vec![own.clone()]);
        for file in files {
            if file != own && !indexed.insert(file.clone()) { continue; }
            let Ok((rr_file, _)) = read_shared(&file) else { continue };
            for s in rr_file.statements() {
                if s.what() != Some(S::Clause) || !s.has_body() { continue; }
                let line = s.line();
                let mut reads = Vec::new();
                check::clause_reads(&rr_file, &check::clause_body(&rr_file, s.node()), &mut reads);
                for name in s.heads() {
                    let name = &name.to_owned();
                    let key = if file == own && paths.len() > 1 { format!("p{i}_{name}") } else { name.clone() };
                    for read in &reads {
                        locations.reads.entry((key.clone(), read.clone())).or_default().insert((file.clone(), line));
                    }
                    locations.definitions.entry(key).or_insert_with(|| (name.clone(), BTreeSet::new())).1.insert((file.clone(), line));
                }
            }
        }
    }
    Some(Sources::enter(locations))
}

/// A file and everything it includes, each once, includes first: (path, statements without the
/// includes, the included files' `.output`s left out); and every library file read or probed, in reading order.
pub fn program(path: &Path) -> Result<(Files, Vec<LibraryFile>), Error> {
    let (shared, files) = program_shared(path)?;
    Ok((shared.iter().map(|(p, s)| (p.clone(), s.iter().map(|s| (*s).clone()).collect())).collect(), files))
}

/// The modules of a program (`program_order`), each its statements as the process holds them (`read_shared`) and
/// those a program reads of them: without its includes, and an included file without its `.output`s (its own, not
/// the program's; `core.rerail` is the prelude, read as the file is).
struct Shared(Vec<(PathBuf, Vec<Stmt>)>);

impl Shared {
    fn iter(&self) -> impl Iterator<Item = (&PathBuf, Vec<&Stmt>)> {
        let core = library().join("core.rerail").canonical().ok();
        let own = self.0.last().map(|(p, _)| p.clone());
        self.0.iter().map(move |(p, statements)| {
            let included = Some(p) != own.as_ref() && Some(p) != core.as_ref();
            (p, statements.iter().filter(|s| !is_include(s) && !(included && s.what() == Some(S::Io { output: true }))).collect())
        })
    }
}

fn program_shared(path: &Path) -> Result<(Shared, Vec<LibraryFile>), Error> {
    if super::plan::corpus::phase0::active() {
        let (modules, files) = builtin_program(path)?;
        return Ok((Shared(modules), files));
    }
    if super::json_rules::is_policy_path(path) {
        return json_program_shared(path);
    }
    let (order, files) = program_order(path)?;
    let modules = order.into_iter().map(|p| Ok((p.clone(), read_shared(&p)?.0.statements()))).collect::<Result<Vec<_>, Error>>()?;
    Ok((Shared(modules), files))
}

/// A JSON policy (`rerail.policy/v1`, decision 0175), read as the `.rerail` file it translates to: its text translated
/// as it is read (`json_rules`), its `#include`s (library files only) resolved as a text's are (`resolve_shared`).
/// The modules and the files read are in `program_shared`'s order: includes first and the file last; `core.rerail`, the
/// file (its JSON bytes' digest, so an edit reads it again), then the rest.
fn json_program_shared(path: &Path) -> Result<(Shared, Vec<LibraryFile>), Error> {
    let path = path.canonical().map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    let (bytes, digest) = super::plan::corpus::observe(|o| o.read(&path)).ok_or_else(|| Error::Syntax(format!("{}: cannot read the file", path.display())))?;
    let file = LibraryFile { path: path.clone(), sha256: Some(digest) };
    noted(&file);
    let json = String::from_utf8(bytes).map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    let id = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let src = super::json_rules::Translator::new().translate_text(&json, &id).map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?;
    let statements = RrFile::read(src).map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?.statements();
    let (modules, own) = resolve_shared(statements)?;
    let core = library().join("core.rerail").canonical().ok();
    let (first, rest): (Vec<_>, Vec<_>) = modules.iter().partition(|(p, _, _)| Some(p) == core.as_ref());
    let mut files: Vec<LibraryFile> = first.iter().map(|(p, d, _)| LibraryFile { path: p.clone(), sha256: Some(*d) }).collect();
    files.push(file);
    files.extend(rest.iter().map(|(p, d, _)| LibraryFile { path: p.clone(), sha256: Some(*d) }));
    let mut shared: Vec<(PathBuf, Vec<Stmt>)> = modules.into_iter().map(|(p, _, s)| (p, s)).collect();
    shared.push((path, own));
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
    if in_library.canonical().ok().as_deref() != Some(target) {
        files.push(LibraryFile { path: in_library, sha256: None });
    }
}

/// `program` for phase 0's own reads.
fn builtin_program(path: &Path) -> Result<(Files, Vec<LibraryFile>), Error> {
    builtin_program_resolved(path, &mut |_, _| {})
}

fn builtin_program_resolved(path: &Path, resolved: &mut impl FnMut(&Path, &Path)) -> Result<(Files, Vec<LibraryFile>), Error> {
    let (mut out, mut read_files) = (Vec::new(), Vec::new());
    // `core.rerail` is every program's prelude (the base relations and built-ins), included or not; except a program in
    // T's shape (decision 0046), which declares its base relations itself (`doc` among them), and whose libraries
    // read only the relations both shapes have.
    let t_shape = read(&crate::git_workspace::snapshot::canonicalize(path).map_err(|e| Error::Syntax(format!("{}: {e}", path.display())))?)?
        .0
        .iter()
        .any(|s| s.what() == Some(S::Decl("doc")));
    if !t_shape {
        visit(&library().join("core.rerail"), &mut out, &mut read_files, &mut Vec::new(), false, resolved)?;
    }
    let prelude = library().join("core.rerail");
    let core = prelude.canonical().map_err(|e| Error::Syntax(format!("core.rerail: {e}")))?;
    resolved(&prelude, &core);
    let mut seen = vec![core];
    visit(path, &mut out, &mut read_files, &mut seen, false, resolved)?;
    Ok((out, read_files))
}

pub use crate::code_rule_dsl::facts::vtab::syntax::core_rr::single_metavars;

#[cfg(test)]
mod key_tests {
    use super::*;

    /// An image lowered under a switch that changes lowering (`plan::corpus::COMBINING`) is kept under another key than
    /// the default's, and an unset switch keeps the key (the engine's shipped images, decision 0264, are found).
    #[test]
    fn an_image_lowered_under_a_lowering_switch_is_keyed_apart() {
        let scope = [crate::content_store::HashKey::sha256(b"a module")];
        let files = [[7u8; 32]];
        let default = (program_key("x(1).", &scope), module_key(files.iter(), scope.iter()), linked_key(&scope));
        // The keys as they were before switches were keyed: an unset switch adds nothing.
        let mut bytes = format!("rerail linked library {}\0{}\0", crate::code_rule_dsl::logic::program_image::FORMAT, crate::code_rule_dsl::build_ids::engine()).into_bytes();
        bytes.extend_from_slice(&scope[0].sha256_bytes().unwrap_or_default());
        assert_eq!(default.2, crate::content_store::HashKey::sha256(&bytes));
        for switch in ["RERAIL_NODE_SOURCES", "RERAIL_ORDER_RAW", "RERAIL_NODE_IDS"] {
            let _set = crate::code_rule_dsl::logic::plan::corpus::switches::set(&[(switch, "eager")]);
            assert_ne!(program_key("x(1).", &scope), default.0, "{switch}");
            assert_ne!(module_key(files.iter(), scope.iter()), default.1, "{switch}");
            assert_ne!(linked_key(&scope), default.2, "{switch}");
        }
        assert_eq!((program_key("x(1).", &scope), module_key(files.iter(), scope.iter()), linked_key(&scope)), default);
    }
}
