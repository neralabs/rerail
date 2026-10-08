// Rerail-authored fixture (MIT). Attributes and macros: on an item-position and a statement-position macro
// invocation, inside a `macro_rules!` body (the attribute travels with the expansion), at the call site of a
// macro whose body carries the site (the diagnostic points into the macro definition), and around a macro
// argument that carries the site (the diagnostic points at the argument).

macro_rules! make_fn {
    ($name:ident) => {
        pub fn $name() -> i32 {
            ((1))
        }
    };
}

make_fn!(unallowed_generated);

#[allow(clippy::double_parens)]
make_fn!(allowed_on_item_invocation);

macro_rules! make_allowed_fn {
    ($name:ident) => {
        #[allow(clippy::double_parens)]
        pub fn $name() -> i32 {
            ((2))
        }
    };
}

make_allowed_fn!(allowed_in_body);

macro_rules! body_expr {
    () => {
        ((3))
    };
}

pub fn body_site_unallowed() -> i32 {
    body_expr!()
}

#[allow(clippy::double_parens)]
pub fn body_site_allowed_at_call() -> i32 {
    body_expr!()
}

macro_rules! pass {
    ($e:expr) => {
        $e
    };
}

pub fn argument_site_unallowed() -> i32 {
    pass!(((4)))
}

#[allow(clippy::double_parens)]
pub fn argument_site_allowed() -> i32 {
    pass!(((5)))
}

macro_rules! bind_unused {
    () => {
        let unused = 0;
    };
}

pub fn statement_invocation() {
    #[allow(unused_variables)]
    bind_unused!();
    bind_unused!();
}

macro_rules! allow_inside {
    ($e:expr) => {{
        #[allow(clippy::double_parens)]
        let v = $e;
        v
    }};
}

pub fn allow_from_macro_reaches_argument() -> i32 {
    allow_inside!(((6)))
}
