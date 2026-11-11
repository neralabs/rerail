//! Moved from `syntax.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;

/// Strings (printf and scanf formats) and doc comments.
impl SyntaxRules for Php {
    const DOLLAR_NAMES: bool = true;

    fn feature(feature: SyntaxFeature) -> Result<(), Refusal> {
        match feature {
            SyntaxFeature::LanguageOptions => {
                refused("`languageOptions` is not available for php policies")
            }
            SyntaxFeature::Parse(grammar) => match grammar.literal_host() {
                LiteralHost::PhpString | LiteralHost::PhpComment => Ok(()),
                LiteralHost::EcmaComment | LiteralHost::RustString => unhosted(grammar, ""),
                LiteralHost::PythonString => match grammar {
                    Grammar::Percent => {
                        unhosted(grammar, "; PHP's printf formats are `printfFormat`")
                    }
                    Grammar::Brace => unhosted(
                        grammar,
                        "; PHP has no str.format templates (its printf formats are `printfFormat`)",
                    ),
                    Grammar::Annotation => unhosted(
                        grammar,
                        "; PHP holds no type in a string: its declared types are syntax, and its PHPDoc types are `phpDoc`",
                    ),
                    _ => unhosted(grammar, ""),
                },
            },
        }
    }

    fn compile_language_options(value: &Value, path: &str) -> napi::Result<QueryOptions> {
        <Ecma as SyntaxRules>::compile_language_options(value, path)
    }

    fn compile_when(
        value: &Value,
        path: &str,
        options: Option<&LanguageOptions>,
        python: Option<&PythonOptions>,
        document: &Options,
    ) -> napi::Result<QueryWhen> {
        // A PHP file has no module system: a `when.sourceType` reads ESLint's default, a module.
        let (applies, types) =
            <Ecma as SyntaxRules>::compile_when(value, path, options, python, document)?;
        Ok((
            applies && types.is_none_or(|types| types.contains(&SourceType::Module)),
            None,
        ))
    }
}
