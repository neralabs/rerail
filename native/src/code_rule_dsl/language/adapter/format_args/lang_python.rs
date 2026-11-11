//! Moved from `format_args.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;
use crate::code_rule_dsl::language::adapter::python::Python;

/// `str.format` templates and `%` formats.
impl FormatArgRules for Python {
    const SUPPORTED: bool = true;
    const FOLLOWS_STRINGS: bool = false;

    fn feature(feature: FormatArgFeature) -> Result<(), Refusal> {
        match feature {
            FormatArgFeature::Style(Style::Brace | Style::Percent) => Ok(()),
            FormatArgFeature::Style(Style::Printf) => refused(
                "`printf` reads PHP's printf-family calls; Python's printf formats are style `percent`",
            ),
            FormatArgFeature::Style(Style::Scanf) => {
                refused("`scanf` reads PHP's sscanf and fscanf calls; Python has no scanf formats")
            }
            FormatArgFeature::Style(Style::Rust) => refused(
                "`rust` reads Rust's format macros; Python's format calls are styles `brace` and `percent`",
            ),
        }
    }
}
