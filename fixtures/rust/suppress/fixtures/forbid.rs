// Rerail-authored fixture (MIT). `forbid` cannot be lowered: an inner `allow`, `expect` or `warn` of the same
// lint is error E0453 and the site is still reported at the forbidden level; an inner `deny` or `forbid` is fine.
#![forbid(clippy::double_parens)]

pub fn forbidden() -> i32 {
    ((1))
}

#[allow(clippy::double_parens)]
pub fn allow_under_forbid() -> i32 {
    ((2))
}

#[expect(clippy::double_parens)]
pub fn expect_under_forbid() -> i32 {
    ((3))
}

#[warn(clippy::double_parens)]
pub fn warn_under_forbid() -> i32 {
    ((4))
}

#[deny(clippy::double_parens)]
pub fn deny_under_forbid() -> i32 {
    ((5))
}

#[forbid(unused_variables)]
pub mod rustc_lint {
    #[deny(unused_variables)]
    pub fn deny_under_forbid() {
        let unused = 0;
    }
}
