//! Moved from `syntax.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;

/// String literals: formats and annotations.
impl SyntaxRules for Python {
    const DOLLAR_NAMES: bool = true;

    fn feature(feature: SyntaxFeature) -> Result<(), Refusal> {
        match feature {
            SyntaxFeature::LanguageOptions => Ok(()),
            SyntaxFeature::Parse(grammar) => match grammar.literal_host() {
                LiteralHost::PythonString => Ok(()),
                LiteralHost::EcmaComment | LiteralHost::PhpComment | LiteralHost::RustString => {
                    unhosted(grammar, "")
                }
                LiteralHost::PhpString => match grammar {
                    Grammar::Scanf => unhosted(grammar, "; Python has no scanf formats"),
                    _ => unhosted(grammar, "; Python's printf formats are `percentFormat`"),
                },
            },
        }
    }

    fn compile_language_options(value: &Value, path: &str) -> napi::Result<QueryOptions> {
        let python = options::compile_python_options(value, path)?;
        Ok((None, Some(Arc::new(python))))
    }

    fn compile_when(
        value: &Value,
        path: &str,
        _: Option<&LanguageOptions>,
        python: Option<&PythonOptions>,
        document: &Options,
    ) -> napi::Result<QueryWhen> {
        Ok((
            options::compile_python_when(value, path, python, document)?,
            None,
        ))
    }
}
