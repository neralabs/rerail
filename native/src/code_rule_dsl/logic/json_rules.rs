//! Main's JSON rules (`rerail.policy/v1`), read by the engine (decision 0175).
//!
//! A rule file whose top-level `format` is `rerail.policy/v1` is translated on read into `.rr` text, which then
//! goes through the ordinary reader and lowering. The translator writes the display dialect, and the engine pass
//! (engine_dialect.rs) rewrites it into the dialect the reader takes. It began as a port of a Python translator in
//! the docs, retired in decision 0182; this is now the only one. Its parts:
//!
//! - the clause loop and printing (translate.rs), the construct-by-construct lowering (lower.rs), the shared
//!   metavariable passes (share.rs), the demand passes and recursion unrolling (demand.rs), and the text helpers
//!   they share (text.rs, program.rs);
//! - main's terms (main_terms.rs), the merge of copies (simplify.rs) and of families of them (fold.rs);
//! - `check_rule`, `whole` and `segments` (souffle.rs), and a regex's glob reading (textglob.rs);
//! - the engine dialect (engine_dialect.rs).
//!
//! The Python semantics the port kept are kept where the output depends on them: ordered dicts (`value::Obj`),
//! `json.dumps`, `re.sub`/`re.split`/`findall` (pyre.rs, over `fancy-regex` for look-around and back references).
//! The module-level regexes are one `Patterns` value (patterns.rs), compiled once for a batch of translations
//! (`Translator`) and passed to each pass as `px`, not process-wide statics.
//!
//! `docs/rule-syntax/packs/python-main` is this translator's output on main's 537 Python rules, with no hand-written
//! overrides; the test `json_rules::tests::json_rules_pack_is_current` holds it current.

mod demand;
mod engine_dialect;
mod fold;
mod lower;
mod main_terms;
#[doc(hidden)]
pub mod patterns;
mod program;
#[doc(hidden)]
pub mod pyre;
mod share;
mod simplify;
#[doc(hidden)]
pub mod souffle;
mod text;
#[doc(hidden)]
pub mod textglob;
mod translate;
#[doc(hidden)]
pub mod value;

use std::collections::BTreeSet;
use std::path::Path;

/// The JSON format the engine translates on read.
pub const FORMAT: &str = "rerail.policy/v1";

/// A policy's display translation and its two counts.
pub struct Display {
    pub text: String,
    pub untranslated: usize,
    pub ungrounded: usize,
}

/// What every translation reads, made once for a batch of them and passed to each: the names a folded family
/// must not take (the library's relations and functors, and the built-ins), and the translator's patterns.
pub struct Translator {
    reserved: BTreeSet<String>,
    px: patterns::Patterns,
}

impl Translator {
    pub fn new() -> Self {
        Translator { reserved: fold::reserved(&super::rr::library()), px: patterns::Patterns::new() }
    }

    /// One JSON policy's text as engine `.rr` text. `id` names the rule (the file's stem), as the Python takes it.
    pub fn translate_text(&self, json: &str, id: &str) -> Result<String, String> {
        let doc = value::parse(json).map_err(|e| format!("{id}: not JSON: {e}"))?;
        let (rr, _) = translate::translate(&self.px, &doc, &self.reserved).map_err(|e| format!("{id}: {e}"))?;
        engine_dialect::engine_text(&self.px, rr, id).map_err(|e| format!("{id}: {e}"))
    }

    /// One JSON policy's display translation (the rules docs' generated view: before the engine's dialect), with
    /// the JSON terms it left as `ast(..)` atoms and its rules no positive atom grounds.
    pub fn display_text(&self, json: &str, id: &str) -> Result<Display, String> {
        let doc = value::parse(json).map_err(|e| format!("{id}: not JSON: {e}"))?;
        let (text, untranslated) = translate::translate(&self.px, &doc, &self.reserved).map_err(|e| format!("{id}: {e}"))?;
        let ungrounded = text.lines().filter(|l| l.starts_with("// Not grounded:")).count();
        Ok(Display { text, untranslated, ungrounded })
    }

    /// The JSON policy at `path` as engine `.rr` text, named by the file's stem.
    pub fn translate_file(&self, path: &Path) -> Result<String, String> {
        let json = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let id = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        self.translate_text(&json, &id)
    }
}

impl Default for Translator {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether `text` is a JSON policy: a JSON object whose top-level `format` is `rerail.policy/v1`.
pub fn is_policy_json(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with('{') && matches!(value::parse(t), Ok(value::Value::Obj(o)) if o.get("format").and_then(value::Value::as_str) == Some(FORMAT))
}

/// Whether the file at `path` is read as a JSON policy: a `.json` file in that format.
pub fn is_policy_path(path: &Path) -> bool {
    path.extension().is_some_and(|x| x == "json") && std::fs::read_to_string(path).is_ok_and(|t| is_policy_json(&t))
}
