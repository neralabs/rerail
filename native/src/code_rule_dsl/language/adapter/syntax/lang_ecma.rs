//! Moved from `syntax.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;

/// Comments: ESLint's directives.
impl SyntaxRules for Ecma {
    const DOLLAR_NAMES: bool = false;

    fn feature(feature: SyntaxFeature) -> Result<(), Refusal> {
        match feature {
            SyntaxFeature::LanguageOptions => Ok(()),
            SyntaxFeature::Parse(grammar) => match grammar.literal_host() {
                LiteralHost::EcmaComment => Ok(()),
                LiteralHost::PythonString
                | LiteralHost::PhpString
                | LiteralHost::PhpComment
                | LiteralHost::RustString => unhosted(grammar, ""),
            },
        }
    }

    fn compile_language_options(value: &Value, path: &str) -> napi::Result<QueryOptions> {
        let options = options::compile_language_options(value, path)?;
        Ok((Some(Arc::new(options)), None))
    }

    fn compile_when(
        value: &Value,
        path: &str,
        options: Option<&LanguageOptions>,
        _: Option<&PythonOptions>,
        document: &Options,
    ) -> napi::Result<QueryWhen> {
        options::compile_ecma_when(value, path, options, document)
    }
}
