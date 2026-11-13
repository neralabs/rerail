//! A `.rr` file's reading of its core.rr facts (`core_rr::DialectFacts`): its literals as the rule engine reads them,
//! its syntax error the one running it gives.

use super::core_rr::{DialectFacts, Literal};
use crate::code_rule_dsl::facts::core::Facts;

pub struct RrFacts;

impl DialectFacts for RrFacts {
    /// A `.rr` string reads as the rule engine reads it; a backtick literal's `str` is its body.
    fn literal(&self, kind: &str, text: &str) -> Option<Literal> {
        let mut out = Literal::default();
        match kind {
            "string" => {
                if let Some(v) = crate::code_rule_dsl::logic::rr::string_value(text) {
                    out.value = Some((v.clone(), "string"));
                    out.str = Some(v);
                }
            }
            "code_literal" => out.str = Some(crate::code_rule_dsl::logic::rr::code_body(text).to_owned()),
            "number" => out.value = text.parse::<i64>().ok().map(|v| (v.to_string(), "number")),
            "float" => out.value = text.parse::<f64>().ok().map(|v| (v.to_string(), "float")),
            _ => {}
        }
        Some(out)
    }
    /// The error running the file gives (`logic::rr::syntax_error`).
    fn verdict(&self, f: &Facts) -> Option<Result<Option<(u32, String)>, String>> {
        Some(Ok(f.syntax_errors.then(|| crate::code_rule_dsl::logic::rr::syntax_error(&f.source)).flatten().map(|(line, message)| (line as u32, message))))
    }
}
