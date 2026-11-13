use crate::code_rule_dsl::json::invalid;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::language::{self, CodeRuleLanguage};
use crate::code_rule_dsl::matcher::AstMatcher;
use crate::code_rule_dsl::model::Capabilities;
use napi::Result;
use serde_json::Value;

/// Grammar dialects a rule language spans, keyed by the file language label that selects each one.
type GrammarDialects = Vec<(&'static str, GrammarDialect)>;

pub fn language_capability(
    name: &str,
    capabilities: &Capabilities,
) -> Result<(GrammarDialects, String)> {
    let languages = capabilities
        .raw
        .get("ast")
        .and_then(|value| value.get("languages"))
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("missing ast language capabilities"))?;
    let grammar = languages
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("missing {name} language capability")))?
        .to_owned();
    let code_rule_language = CodeRuleLanguage::parse(name)
        .ok_or_else(|| invalid(format!("missing {name} language capability")))?;
    let dialects = code_rule_language
        .dialects()
        .iter()
        .map(|&dialect| {
            (
                language::label(dialect).expect("policy dialect has a file label"),
                dialect,
            )
        })
        .collect();
    Ok((dialects, grammar))
}

/// The dialects of the programs a file of `dialects` embeds through `via` (`ast.embedded`), each
/// keyed by its label: what an embedded clause compiles for. `None` where the language embeds
/// nothing through `via`.
pub fn embedded_dialects(
    dialects: &[(&'static str, GrammarDialect)],
    via: language::Via,
) -> Option<GrammarDialects> {
    let mut embedded: GrammarDialects = Vec::new();
    for &(_, host) in dialects {
        for &(through, dialect) in host.embeds() {
            if through == via && !embedded.iter().any(|&(_, known)| known == dialect) {
                embedded.push((
                    language::label(dialect).expect("embedded dialect has a file label"),
                    dialect,
                ));
            }
        }
    }
    (!embedded.is_empty()).then_some(embedded)
}

pub fn grammar_gap(gaps: &mut Vec<String>, reason: String) -> AstMatcher {
    gaps.push(reason);
    AstMatcher::Never
}

// tests: a test the engine hosts (it reads above this crate; tools/cratify/test-hosts.tsv)
