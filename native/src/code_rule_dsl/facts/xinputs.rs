//! The cross-file families' per-content inputs from this crate's own tapes (open items O39, O40;
//! notes/t16-xref-from-tapes.md): what Y1 (`ygraph::summary`) and the symbol table
//! (`y3::input`) read, produced per content instead of read from facts-engine's X6 exports.
//!
//! - Y1's rows (`cgn_*`): native's call summary of the content's tape under the three symbolic
//!   module variants, after `as_references` with the pack's links (`cgnative`).
//! - The symbol table's rows: the X6 pack's thirteen inputs, evaluated by the matcher over the
//!   same tape (`pack`), shaped into the exports' rows by the pack's per-file stages (`exports`).
//!
//! Every row is a function of the content alone; the path enters only where a reader puts it:
//! the pack's scope (`in_scope`), an input's path predicate, and the module the stages name.
pub(crate) mod cgnative;
pub(crate) mod exports;
pub(crate) mod pack;
pub(crate) mod store;
#[cfg(test)]
mod tests;

use crate::code_rule_dsl::eval::{self, TapeCacheSlot};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::model::FileInput;
use crate::code_rule_dsl::tape::{FileImage, Grammar, TapeImage};
use cgnative::{Rendered, Variant};
use std::sync::Arc;

/// One content's inputs: the three variants' summaries (rendered, compacted), the pack's inputs
/// in `pack::INPUTS` order (`dc__raw_module_names` left empty once read), and the names
/// `dc__module_names` keeps (`exports::module_names`).
#[derive(Debug, Default)]
pub(crate) struct Produced {
    pub cgn: Vec<Rendered>,
    pub inputs: Vec<pack::Table>,
    pub module_names: Vec<String>,
}

/// The producer: the compiled pack and its links.
pub(crate) struct Producer {
    pub pack: pack::Pack,
    links: crate::code_rule_dsl::calls::contracts::Key,
    grammar: Arc<Grammar>,
}

impl Producer {
    pub fn new() -> Result<Producer, String> {
        let pack = pack::Pack::compile()?;
        let links = cgnative::links_key(&pack.links)?;
        let grammar = Grammar::new(GrammarDialect::Python)?;
        Ok(Producer { pack, links, grammar })
    }

    /// Parses `source` into its tape image (what the tape family stores).
    pub fn image(&self, source: &str) -> Result<FileImage, String> {
        TapeImage::parse(&self.grammar, source, &|| false).ok_or("the parser gave up")?
    }

    /// The inputs of `source` over its tape `image`.
    pub fn produce(&self, source: Arc<String>, image: &FileImage) -> Result<Produced, String> {
        let file = FileInput::new(pack::READING.into(), "python".into(), Arc::clone(&source), None, None, "xinputs").map_err(|e| e.reason.to_string())?;
        let _tapes = TapeCacheSlot::install();
        TapeCacheSlot::seed(GrammarDialect::Python, &file, image).map_err(|e| e.reason.to_string())?;
        let (doc, _) = eval::file_tape(GrammarDialect::Python, &file).map_err(|e| e.reason.to_string())?.ok_or("no tape")?;
        let errors = !doc.syntax_error_ranges().is_empty();
        let cgn = Variant::ALL
            .iter()
            .map(|v| cgnative::compacted(&cgnative::render(&cgnative::summary(&doc, pack::READING, &v.module(), errors, source.len()).as_references(self.links))))
            .collect();
        let mut inputs = self.pack.evaluate(&file)?;
        let module_names = exports::module_names(self, &inputs);
        let raw = pack::INPUTS.iter().position(|n| *n == "dc__raw_module_names").expect("an input");
        inputs[raw] = pack::Table::default();
        Ok(Produced { cgn, inputs, module_names })
    }
}
