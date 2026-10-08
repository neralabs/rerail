// Rerail-authored fixture (MIT). Lint names that are not current: a renamed clippy lint still suppresses its
// new name (`renamed_and_removed_lints` at the attribute), a clippy lint renamed into rustc likewise, a removed
// (deprecated) clippy lint and unknown names only warn at the attribute and suppress nothing.

#[allow(clippy::const_static_lifetime)]
pub const RENAMED: &'static str = "a";
pub const NOT_ALLOWED: &'static str = "b";

#[allow(clippy::unused_label)]
pub fn renamed_into_rustc() {
    'unused: for _ in 0..1 {}
}

#[allow(clippy::assign_ops)]
pub fn removed_lint() -> i32 {
    ((1))
}

#[allow(clippy::no_such_lint)]
pub fn unknown_clippy_lint() -> i32 {
    ((2))
}

#[allow(no_such_lint)]
pub fn unknown_rustc_lint() -> i32 {
    ((3))
}

#[allow(clippy::Double_Parens)]
pub fn wrong_case() -> i32 {
    ((4))
}

#[allow(unknown_lints, clippy::no_such_lint_either)]
pub fn unknown_lints_allowed_first() -> i32 {
    ((5))
}

#[allow(clippy::no_such_lint_third, unknown_lints)]
pub fn unknown_lints_allowed_after() -> i32 {
    ((6))
}

#[allow(clippy::double_parens, clippy::double_parens)]
pub fn duplicated() -> i32 {
    ((7))
}
