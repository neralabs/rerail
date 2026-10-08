// Rerail-authored fixture (MIT). `cfg_attr` applies its attributes only when the predicate holds; clippy-driver
// sets `cfg(clippy)`. Nested `cfg_attr` and several attributes in one `cfg_attr` expand in order.

#[cfg_attr(all(), allow(clippy::double_parens))]
pub fn always() -> i32 {
    ((1))
}

#[cfg_attr(any(), allow(clippy::double_parens))]
pub fn never() -> i32 {
    ((2))
}

#[cfg_attr(clippy, allow(clippy::double_parens))]
pub fn under_clippy() -> i32 {
    ((3))
}

#[cfg_attr(not(clippy), allow(clippy::double_parens))]
pub fn not_under_clippy() -> i32 {
    ((4))
}

#[cfg_attr(all(), cfg_attr(all(), allow(clippy::double_parens)))]
pub fn nested() -> i32 {
    ((5))
}

#[cfg_attr(all(), allow(clippy::double_parens), warn(clippy::double_parens))]
pub fn two_attributes_last_wins() -> i32 {
    ((6))
}

pub fn on_statement() -> i32 {
    #[cfg_attr(all(), allow(unused_variables))]
    let unused_allowed = 0;
    #[cfg_attr(any(), allow(unused_variables))]
    let unused_reported = 0;
    ((7))
}

pub mod inner {
    #![cfg_attr(all(), allow(clippy::double_parens))]
    pub fn f() -> i32 {
        ((8))
    }
}
