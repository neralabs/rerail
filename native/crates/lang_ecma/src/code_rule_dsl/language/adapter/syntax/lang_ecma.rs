//! Moved from `syntax.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;
use crate::code_rule_dsl::language::adapter::ecma::Ecma;

/// Comments: ESLint's directives.
impl SyntaxRules for Ecma {
    fn parse_literal<D: Doc>(
        _: Grammar,
        node: &Node<'_, D>,
        _: &str,
        spend: &mut dyn FnMut(usize) -> bool,
    ) -> Option<crate::code_rule_dsl::literal::Parsed> {
        crate::code_rule_dsl::literal::eslint_directive::parse_comment(node, spend)
    }

    const DOLLAR_NAMES: bool = false;

    fn feature(feature: SyntaxFeature) -> Result<(), Refusal> {
        match feature {
            SyntaxFeature::LanguageOptions => Ok(()),
            SyntaxFeature::Parse(grammar) if crate::code_rule_dsl::language::Family::of(grammar.host_language()) == crate::code_rule_dsl::language::Family::Ecma => Ok(()),
            SyntaxFeature::Parse(grammar) => unhosted(grammar, ""),
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
