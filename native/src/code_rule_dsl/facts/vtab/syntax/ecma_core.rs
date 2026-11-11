//! ECMAScript's reading of its core.rr facts (`core_rr::DialectFacts`): its strings decoded as ECMAScript decodes them.

use super::core_rr::DialectFacts;

pub(crate) struct EcmaFacts;

impl DialectFacts for EcmaFacts {
    fn string_body(&self, prefix: &str, text: &str) -> Option<String> {
        if !prefix.is_empty() {
            return None;
        }
        crate::code_rule_dsl::value::ecma::string(text).map(|units| String::from_utf16_lossy(&units))
    }
}
