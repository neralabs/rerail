#![warn(clippy::precedence)]
#![allow(clippy::eq_op, clippy::identity_op, clippy::no_effect, clippy::erasing_op, clippy::if_same_then_else)]

fn takes(_: i32) {}

fn shift_with_multiplicative() {
    1 << 2 * 3;
    //~^ precedence
    8 >> 4 / 2;
    //~^ precedence
    1 << 5 % 3;
    //~^ precedence
    1 * 2 << 3;
    //~^ precedence
    16 / 2 >> 3;
    //~^ precedence
    4 % 3 << 1;
    //~^ precedence
}

fn bitwise_with_arithmetic() {
    1 & 2 * 3;
    //~^ precedence
    1 | 2 / 3;
    //~^ precedence
    1 ^ 2 % 3;
    //~^ precedence
    1 * 2 & 3;
    //~^ precedence
    1 - 2 | 3;
    //~^ precedence
    1 / 2 ^ 3;
    //~^ precedence
}

fn arithmetic_on_both_sides() {
    1 + 2 & 3 * 4;
    //~^ precedence
    1 << 2 & 3 + 4;
    //~^ precedence
}

fn nested() {
    let _ = (1 << 2 + 3) == 0;
    //~^ precedence
    let _ = 1 + (2 & 3 - 1);
    //~^ precedence
    let _ = 1 | 2 & 3 * 4;
    //~^ precedence
    takes(1 + (2 << 3 * 4));
    //~^ precedence
    let _ = [1 << 2 % 3, 4];
    //~^ precedence
    let _ = if true { 1 } else { 2 >> 1 * 1 };
    //~^ precedence
    let _ = 1 << 2 - 3 << 4;
    //~^ precedence
}

fn parenthesised() {
    (1 << 2) + 3;
    1 << (2 + 3);
    (1 & 2) * 3;
    1 | (2 / 3);
    1 << 2 << 3;
    1 + 2 * 3;
}
