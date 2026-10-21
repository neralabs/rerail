//! Format strings' arguments (`formatArgs`): `Semantic::FormatArgs`.

use super::{Php, Python, Refusal, Refused};
use crate::code_rule_dsl::matcher::Style;
use std::borrow::Cow;

/// An option of `formatArgs` a family may not read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum FormatArgFeature {
    /// `style`: the format calls read, each style one family's.
    Style(Style),
}

/// A family's format strings.
pub(crate) trait FormatArgRules {
    /// Whether the family reads format strings' arguments: its languages' registry capability,
    /// which the adapter kit checks.
    #[cfg_attr(not(test), allow(dead_code))]
    const SUPPORTED: bool;
    /// Whether a format's string is read as PHPStan reads a constant string: through
    /// concatenation, casts and the one binding that reaches a variable. Otherwise it is the
    /// literal written in the call.
    const FOLLOWS_STRINGS: bool;
    /// Whether the family reads `feature`, or why not.
    fn feature(feature: FormatArgFeature) -> Result<(), Refusal>;
}

fn refused(message: &'static str) -> Result<(), Refusal> {
    Err(Refusal::Refused(Cow::Borrowed(message)))
}

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

/// No format calls: every style is another family's.
impl FormatArgRules for Refused {
    const SUPPORTED: bool = false;
    const FOLLOWS_STRINGS: bool = false;

    fn feature(feature: FormatArgFeature) -> Result<(), Refusal> {
        match feature {
            FormatArgFeature::Style(_) => {
                refused("formatArgs reads Python, PHP and Rust format calls")
            }
        }
    }
}

/// The kit's check of a family's `FormatArgRules`.
#[cfg(test)]
pub(super) mod kit {
    use super::super::syntax::kit::refusal;
    use super::{FormatArgFeature, FormatArgRules};
    use crate::code_rule_dsl::language::adapter::Adapter;
    use crate::code_rule_dsl::matcher::Style;

    /// What a family's `FormatArgRules` answer.
    pub(in super::super) struct Expect {
        pub(in super::super) follows_strings: bool,
        /// The refusals of the styles `brace`, `percent`, `printf`, `scanf` and `rust`; `None`
        /// where read.
        pub(in super::super) styles: [Option<&'static str>; 5],
    }

    pub(in super::super) fn check<A: Adapter>(expect: &Expect) {
        assert_eq!(
            <A::FormatArgs as FormatArgRules>::FOLLOWS_STRINGS,
            expect.follows_strings
        );
        let styles = [
            Style::Brace,
            Style::Percent,
            Style::Printf,
            Style::Scanf,
            Style::Rust,
        ]
        .map(|style| {
            refusal(<A::FormatArgs as FormatArgRules>::feature(
                FormatArgFeature::Style(style),
            ))
        });
        assert_eq!(styles, expect.styles.map(|m| m.map(str::to_owned)));
    }
}
