// Rerail-authored fixture (MIT). Lint groups: `clippy::all` (default-on groups, not pedantic),
// `clippy::complexity` (double_parens), `clippy::style` (needless_else), `clippy::pedantic`
// (unreadable_literal), rustc's `unused` (unused_variables) and `warnings` (every lint at warn level).
#![warn(clippy::pedantic)]

fn v() {}

#[allow(clippy::all)]
pub fn all_covers_complexity_and_style(x: bool) -> i32 {
    if x {
        v();
    } else {
    }
    ((1))
}

#[allow(clippy::all)]
pub fn all_does_not_cover_pedantic() -> u64 {
    1234567890
}

#[allow(clippy::pedantic)]
pub fn pedantic_covers_unreadable_literal() -> u64 {
    1234567891
}

#[allow(clippy::style)]
pub fn style_covers_needless_else_only(x: bool) -> i32 {
    if x {
        v();
    } else {
    }
    ((2))
}

#[allow(clippy::complexity)]
pub fn complexity_covers_double_parens_only(x: bool) -> i32 {
    if x {
        v();
    } else {
    }
    ((3))
}

#[allow(unused)]
pub fn unused_group() {
    let unused = 0;
}

#[allow(warnings)]
pub fn warnings_group(x: bool) -> u64 {
    let unused = 0;
    if x {
        v();
    } else {
    }
    1234567892
}

#[allow(clippy::pedantic)]
pub mod pedantic_then_lint {
    #[warn(clippy::unreadable_literal)]
    pub fn re_enabled() -> u64 {
        1234567893
    }
}
