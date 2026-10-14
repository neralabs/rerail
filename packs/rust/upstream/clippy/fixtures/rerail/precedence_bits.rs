#![warn(clippy::precedence_bits)]
#![allow(clippy::eq_op, clippy::identity_op, clippy::no_effect, clippy::erasing_op, clippy::precedence)]

fn takes(_: i32) {}

fn outermost() {
    0x0F00 & 0x00F0 << 4;
    //~^ precedence_bits
    0x0F00 | 0xF000 >> 4;
    //~^ precedence_bits
    0x0F00 ^ 1 << 4;
    //~^ precedence_bits
    1 << 4 & 0x0F00;
    //~^ precedence_bits
    1 >> 4 | 0x0F00;
    //~^ precedence_bits
    1 << 4 ^ 0x0F00;
    //~^ precedence_bits
    1 << 2 & 3 >> 4;
    //~^ precedence_bits
}

fn nested() {
    let _ = (0x0F00 & 0x00F0 << 4) == 0;
    //~^ precedence_bits
    let _ = 1 + (2 & 3 << 4);
    //~^ precedence_bits
    takes(1 + (2 | 3 >> 4));
    //~^ precedence_bits
    let _ = [1 ^ 2 << 3, 4];
    //~^ precedence_bits
    let _ = if true { 1 } else { 2 & 3 >> 1 };
    //~^ precedence_bits
    let _ = 1 | 2 & 3 << 4;
    //~^ precedence_bits
}

fn arithmetic_inside_a_shift() {
    // the outer `&` has a shift operand and no arithmetic one; the shift's own `+` is the `precedence` lint's
    1 & 2 << 3 + 4;
    //~^ precedence_bits
}

fn arithmetic_makes_it_precedence() {
    // an arithmetic operand makes it the `precedence` lint's, not `precedence_bits`'
    1 << 2 & 3 + 4;
    1 & 2 + 3;
    1 << 2 + 3;
    1 * 2 & 3 << 4;
}

fn parenthesised() {
    (1 & 2) << 3;
    1 & (2 << 3);
    1 << 2 << 3;
    1 & 2 & 3;
}
