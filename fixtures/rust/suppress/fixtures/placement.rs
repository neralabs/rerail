// Rerail-authored fixture (Apache-2.0). Where an outer or inner `allow` reaches: fn, impl, impl item, trait item,
// inline mod, statement, expression statement, block, match arm, closure statement, struct field, enum
// variant, const item. Each block has one suppressed site and one unsuppressed sibling.
#![warn(clippy::unreadable_literal)]

pub fn plain() -> i32 {
    ((1))
}

#[allow(clippy::double_parens)]
pub fn on_fn() -> i32 {
    ((2))
}

pub struct S;

#[allow(clippy::double_parens)]
impl S {
    pub fn in_impl(&self) -> i32 {
        ((3))
    }
}

impl S {
    #[allow(clippy::double_parens)]
    pub fn on_impl_item(&self) -> i32 {
        ((4))
    }

    pub fn next_impl_item(&self) -> i32 {
        ((5))
    }
}

pub trait T {
    #[allow(clippy::double_parens)]
    fn on_trait_item(&self) -> i32 {
        ((6))
    }

    fn next_trait_item(&self) -> i32 {
        ((7))
    }
}

#[allow(clippy::double_parens)]
pub mod outer_on_mod {
    pub fn f() -> i32 {
        ((8))
    }
}

pub mod inner_in_mod {
    #![allow(clippy::double_parens)]
    pub fn f() -> i32 {
        ((9))
    }
}

pub fn inner_in_fn() -> i32 {
    #![allow(clippy::double_parens)]
    ((10))
}

pub fn statements() -> i32 {
    #[allow(clippy::double_parens)]
    let a = ((11));
    let b = ((12));
    #[allow(unused_variables)]
    let unused_allowed = 0;
    let unused_reported = 0;
    #[allow(clippy::unreadable_literal)]
    let c = 1234567890;
    let d = 1234567890;
    a + b + c + d
}

pub fn expression_statement(v: &mut Vec<i32>) {
    #[allow(clippy::double_parens)]
    v.push(((13)));
    v.push(((14)));
}

pub fn block_statement(x: bool) -> i32 {
    #[allow(clippy::needless_else)]
    {
        if x {
            v();
        } else {
        }
    }
    if x {
        v();
    } else {
    }
    0
}

fn v() {}

pub fn match_arms(x: u8) -> i32 {
    match x {
        #[allow(clippy::double_parens)]
        0 => ((15)),
        _ => ((16)),
    }
}

pub fn closure_statement() -> i32 {
    #[allow(clippy::double_parens)]
    let f = || ((17));
    let g = || ((18));
    f() + g()
}

pub struct Fields {
    #[allow(clippy::unreadable_literal)]
    pub allowed: [u8; 1048576],
    pub reported: [u8; 2097152],
}

pub enum E {
    #[allow(clippy::unreadable_literal)]
    A = 1234567890,
    B = 1234567891,
}

#[allow(clippy::unreadable_literal)]
pub const ALLOWED: u64 = 9876543210;
pub const REPORTED: u64 = 9876543211;
