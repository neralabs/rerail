//! Moved from `syntax.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;

/// String literals: formats and annotations.
impl SyntaxRules for Python {
    fn parse_literal<D: Doc>(
        grammar: Grammar,
        node: &Node<'_, D>,
        interpolation: &str,
        spend: &mut dyn FnMut(usize) -> bool,
    ) -> Option<crate::code_rule_dsl::literal::Parsed> {
        if grammar.interpolates() {
            crate::code_rule_dsl::literal::python_fstring::parse(node, interpolation, spend)
        } else {
            crate::code_rule_dsl::literal::python_string::parse_string(node, grammar, spend)
        }
    }

    const DOLLAR_NAMES: bool = true;

    fn feature(feature: SyntaxFeature) -> Result<(), Refusal> {
        match feature {
            SyntaxFeature::LanguageOptions => Ok(()),
            SyntaxFeature::Parse(grammar) if crate::code_rule_dsl::language::Family::of(grammar.host_language()) == crate::code_rule_dsl::language::Family::Python => Ok(()),
            SyntaxFeature::Parse(grammar) => match grammar.name() {
                "scanfFormat" => unhosted(grammar, "; Python has no scanf formats"),
                "printfFormat" => unhosted(grammar, "; Python's printf formats are `percentFormat`"),
                _ => unhosted(grammar, ""),
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
