#![warn(clippy::allow_attributes)]

#[cfg_attr(test, allow(dead_code))]
fn off_in_a_build_without_tests() {}

#[cfg_attr(all(), allow(dead_code), allow(unused_variables))]
//~^ allow_attributes
//~| allow_attributes
fn two_expanded_allows() {}

#[cfg_attr(clippy, allow(dead_code))]
//~^ allow_attributes
fn clippy_is_on() {}
