#![warn(clippy::redundant_static_lifetimes)]

fn take(_: &str) {}

const FN_POINTER: fn(&'static str) = take;
const FN_POINTER_RETURN: fn() -> &'static str = || "";
const BOXED: Option<Box<dyn Fn(&'static str)>> = None;
const REFERENCE_TO_DYN: &'static dyn Send = &0;
const REFERENCE_TO_REFERENCE: &'static &'static str = &"";
//~^ redundant_static_lifetimes
const ARRAY_OF_REFERENCES: [&'static str; 1] = [""];
//~^ redundant_static_lifetimes
const TUPLE_OF_REFERENCES: (&'static str, u8) = ("", 0);
//~^ redundant_static_lifetimes
const GENERIC_ARGUMENT: Option<&'static str> = None;
const LOCAL: usize = {
    let _: &'static str = "";
    0
};
