#![warn(clippy::double_parens)]

fn f(_: i32, _: i32) -> i32 {
    0
}

fn g(_: i32) -> i32 {
    0
}

fn spaced() -> i32 {
    ( (0))
    //~^ double_parens
}

fn newline() -> i32 {
    (
        (0)
    )
    //~^^^ double_parens
}

fn comments() -> i32 {
    // a comment between the parentheses: Clippy's source check rejects it
    (/* c */ (0))
}

fn unit() {
    let _ = ( ());
    //~^ double_parens
    let _ = (());
    //~^ double_parens
}

fn tuple() {
    let _ = ( (1, 2));
    //~^ double_parens
}

fn call_arguments() {
    let _ = f((1), 2);
    let _ = f(1, (2));
    let _ = g( (1));
    //~^ double_parens
    let _ = g((1)) + 2;
    //~^ double_parens
    let _ = 2 + g((1));
    //~^ double_parens
}
