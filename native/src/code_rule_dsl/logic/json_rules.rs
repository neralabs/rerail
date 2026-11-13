//! Main's JSON rules (`rerail.policy/v1`), read by the engine (decision 0175).
//!
//! A rule file whose top-level `format` is `rerail.policy/v1` is translated on read into `.rr` text, which then
//! goes through the ordinary reader and lowering. The translator is a port of the Python one that made
//! `packs/python-main` (`docs/rule-syntax/site/translate_rules.py`): `translate.translate(doc)` writes the display
//! dialect, and `engine_dialect.engine_text(text, id)` rewrites it into the dialect the reader takes. Only what that
//! path uses is ported, file by file:
//!
//! - `translate.py`: the clause loop and printing (translate.rs), the construct-by-construct lowering (lower.rs),
//!   the shared metavariable passes (share.rs), the demand passes and recursion unrolling (demand.rs), and the
//!   text helpers they share (text.rs, program.rs);
//! - `main_terms.py` (main_terms.rs), `simplify.py` (simplify.rs), `fold.py` (fold.rs);
//! - `souffle.py`'s `check_rule`, `whole` and `segments` (souffle.rs), `textglob.py`'s `text_test` (textglob.rs);
//! - `engine_dialect.py` (engine_dialect.rs).
//!
//! Python's semantics are kept where the output depends on them: ordered dicts (`value::Obj`), `json.dumps`,
//! `re.sub`/`re.split`/`findall` (pyre.rs, over `fancy-regex` for look-around and back references).
//! Python's module-level `re.compile`s are one `Patterns` value (patterns.rs), compiled once for a batch of
//! translations (`Translator`) and passed to each pass as `px`, not process-wide statics.
//! The rules' manual overrides (`packs/python-main/overrides`) are not part of reading.
//!
//! The parity test (`json_rules::tests::json_rules_parity_with_python`) translates all 537 JSON rules of
//! `docs/rule-syntax/packs/python-main/source/rules` and compares each with the Python output, byte for byte: the
//! pack's `.rr` file, or for the 9 overridden rules the generated text kept in
//! `packs/python-main/generated-before-override`.

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
        let rr = translate::translate(&self.px, &doc, &self.reserved).map_err(|e| format!("{id}: {e}"))?;
        engine_dialect::engine_text(&self.px, rr, id).map_err(|e| format!("{id}: {e}"))
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
