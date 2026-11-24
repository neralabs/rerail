//! Moved from `syntax.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;
use crate::code_rule_dsl::language::adapter::php::Php;
use crate::code_rule_dsl::language::adapter::blade::Blade;

/// A template's own tree holds no literal `parse` reads: its PHP's are the embedded program's
/// (`blade::view`), a PHP document (`ast.embedded`, `embedded`).
impl SyntaxRules for Blade {
    const DOLLAR_NAMES: bool = false;

    fn feature(feature: SyntaxFeature) -> Result<(), Refusal> {
        match feature {
            SyntaxFeature::LanguageOptions => {
                refused("`languageOptions` is not available for blade policies")
            }
            SyntaxFeature::Parse(grammar) => unhosted(
                grammar,
                "; a template's PHP is its embedded program's: read it with `ast.embedded: compiled-view` or `embedded`",
            ),
        }
    }

    fn compile_language_options(value: &Value, path: &str) -> napi::Result<QueryOptions> {
        <Php as SyntaxRules>::compile_language_options(value, path)
    }

    fn compile_when(
        value: &Value,
        path: &str,
        options: Option<&LanguageOptions>,
        python: Option<&PythonOptions>,
        document: &Options,
    ) -> napi::Result<QueryWhen> {
        <Php as SyntaxRules>::compile_when(value, path, options, python, document)
    }
}
