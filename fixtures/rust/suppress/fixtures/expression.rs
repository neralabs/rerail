// Rerail-authored fixture (Apache-2.0). A lint attribute on a closure expression that is not a statement is unstable
// (`stmt_expr_attributes`, error E0658 on stable 1.99.0); the build fails, the attribute still applies.

pub fn closure_expression() -> i32 {
    let f = #[allow(clippy::double_parens)]
    || ((1));
    f()
}
