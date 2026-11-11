//! Moved from `format_args.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;
use crate::code_rule_dsl::language::adapter::php::Php;

/// The printf family and `sscanf`/`fscanf`.
impl FormatArgRules for Php {
    const SUPPORTED: bool = true;
    const FOLLOWS_STRINGS: bool = true;

    fn feature(feature: FormatArgFeature) -> Result<(), Refusal> {
        match feature {
            FormatArgFeature::Style(Style::Printf | Style::Scanf) => Ok(()),
            FormatArgFeature::Style(Style::Percent) => refused(
                "`percent` reads Python's `%` formats; PHP's printf-family calls are style `printf`",
            ),
            FormatArgFeature::Style(Style::Brace) => refused(
                "`brace` reads Python's `str.format` calls; PHP has no such templates (its printf-family calls are style `printf`)",
            ),
            FormatArgFeature::Style(Style::Rust) => refused(
                "`rust` reads Rust's format macros; PHP's printf-family calls are style `printf`",
            ),
        }
    }
}
