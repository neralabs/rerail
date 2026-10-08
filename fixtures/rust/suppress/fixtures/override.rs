// Rerail-authored fixture (Apache-2.0). The innermost level wins: `warn` under an outer `allow`, `allow` under a
// crate-level `deny`, `deny` turning a site into an error, the last of two attributes on one node, and the
// order inside one attribute list.
#![deny(clippy::needless_else)]

#[allow(clippy::double_parens)]
pub mod allowed {
    pub fn still_allowed() -> i32 {
        ((1))
    }

    #[warn(clippy::double_parens)]
    pub fn warned_again() -> i32 {
        ((2))
    }

    pub mod deeper {
        #![warn(clippy::double_parens)]
        pub fn warned_by_inner_attribute() -> i32 {
            ((3))
        }
    }
}

fn v() {}

pub fn denied_at_crate(x: bool) {
    if x {
        v();
    } else {
    }
}

#[allow(clippy::needless_else)]
pub fn allowed_under_crate_deny(x: bool) {
    if x {
        v();
    } else {
    }
}

#[deny(clippy::double_parens)]
pub fn denied_here() -> i32 {
    ((4))
}

#[allow(clippy::double_parens)]
#[warn(clippy::double_parens)]
pub fn later_attribute_wins() -> i32 {
    ((5))
}

#[warn(clippy::double_parens)]
#[allow(clippy::double_parens)]
pub fn later_attribute_wins_allow() -> i32 {
    ((6))
}

#[allow(clippy::double_parens, clippy::all)]
#[warn(clippy::all)]
pub fn group_after_lint() -> i32 {
    ((7))
}

#[warn(clippy::all)]
#[allow(clippy::double_parens)]
pub fn lint_after_group() -> i32 {
    ((8))
}

#[allow(unused)]
pub fn rustc_group_then_lint() {
    #[warn(unused_variables)]
    let unused = 0;
}
