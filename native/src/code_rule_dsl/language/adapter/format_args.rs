//! Format strings' arguments (`formatArgs`): `Semantic::FormatArgs`.

use super::{Refusal, Refused};
use crate::code_rule_dsl::matcher::Style;
use std::borrow::Cow;
// cratify: `lang_python` is rerail-lang-python's (it mounts it)
// cratify: `lang_php` is rerail-lang-php's (it mounts it)

/// An option of `formatArgs` a family may not read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FormatArgFeature {
    /// `style`: the format calls read, each style one family's.
    Style(Style),
}

/// A family's format strings.
pub trait FormatArgRules {
    /// Whether the family reads format strings' arguments: its languages' registry capability,
    /// which the adapter kit checks.
    #[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
    const SUPPORTED: bool;
    /// Whether a format's string is read as PHPStan reads a constant string: through
    /// concatenation, casts and the one binding that reaches a variable. Otherwise it is the
    /// literal written in the call.
    const FOLLOWS_STRINGS: bool;
    /// Whether the family reads `feature`, or why not.
    fn feature(feature: FormatArgFeature) -> Result<(), Refusal>;
}

pub fn refused(message: &'static str) -> Result<(), Refusal> {
    Err(Refusal::Refused(Cow::Borrowed(message)))
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
#[cfg(any(test, feature = "test-hooks"))]
pub mod kit {
    use super::super::syntax::kit::refusal;
    use super::{FormatArgFeature, FormatArgRules};
    use crate::code_rule_dsl::language::adapter::Adapter;
    use crate::code_rule_dsl::matcher::Style;

    /// What a family's `FormatArgRules` answer.
    pub struct Expect {
        pub follows_strings: bool,
        /// The refusals of the styles `brace`, `percent`, `printf`, `scanf` and `rust`; `None`
        /// where read.
        pub styles: [Option<&'static str>; 5],
    }

    pub fn check<A: Adapter>(expect: &Expect) {
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
