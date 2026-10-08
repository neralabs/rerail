// Rerail-authored fixture (MIT). `expect` suppresses like `allow` and reports `unfulfilled_lint_expectations`
// at the attribute when nothing it covers fires, including when an inner `allow` swallows the diagnostic.

#[expect(clippy::double_parens)]
pub fn fulfilled() -> i32 {
    ((1))
}

#[expect(clippy::double_parens)]
pub fn unfulfilled() -> i32 {
    1
}

#[expect(clippy::double_parens, reason = "fixture")]
pub fn fulfilled_with_reason() -> i32 {
    ((2))
}

#[expect(clippy::double_parens, unused_variables)]
pub fn half_fulfilled() -> i32 {
    ((3))
}

#[expect(clippy::double_parens)]
pub fn swallowed_by_inner_allow() -> i32 {
    #[allow(clippy::double_parens)]
    let a = ((4));
    a
}

#[expect(clippy::double_parens)]
pub fn two_sites_one_expectation() -> i32 {
    ((5)) + ((6))
}

#[expect(clippy::all)]
pub fn group_expectation() -> i32 {
    ((7))
}

#[allow(clippy::double_parens)]
pub fn expect_under_allow() -> i32 {
    #[expect(clippy::double_parens)]
    let a = ((8));
    a
}

pub fn expect_on_statement() -> i32 {
    #[expect(unused_variables)]
    let unused = 0;
    #[expect(unused_variables)]
    let used = 0;
    used
}

#[expect(unfulfilled_lint_expectations)]
pub fn expect_the_expectation_lint() -> i32 {
    1
}
