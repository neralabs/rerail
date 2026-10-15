#![warn(clippy::allow_attributes_without_reason)]

#[cfg_attr(test, allow(dead_code))]
fn off_in_a_build_without_tests() {}

#[cfg_attr(all(), allow(dead_code), allow(unused_variables, reason = "documented"))]
//~^ allow_attributes_without_reason
fn first_allow_has_no_reason() {}

#[cfg_attr(all(), allow(dead_code, reason = "documented"), allow(unused_variables))]
//~^ allow_attributes_without_reason
fn second_allow_has_no_reason() {}

#[cfg_attr(all(), allow(dead_code, reason = "a"), allow(unused_variables, reason = "b"))]
fn both_have_reasons() {}
