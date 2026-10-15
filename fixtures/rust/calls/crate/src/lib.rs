// Rerail-authored fixture (Apache-2.0).
#![allow(dead_code, unused_variables, unused_imports)]

pub mod util;
mod net;

pub fn root() { helper(); }
fn helper() {}

use util::f as renamed_f;
use util::{g, h as renamed_h};
pub use util::f as reexported_f;
use net::*;

pub fn imports() {
    renamed_f();
    g();
    renamed_h();
    reexported_f();
    net_call();
    crate::util::f();
    self::helper();
    util::f();
    nested();
    fn nested() { helper(); }
}

pub struct Point(pub i32, pub i32);
pub enum Shape { Circle(i32), Square }
impl Point {
    pub fn new(x: i32, y: i32) -> Self { Point(x, y) }
    pub fn m(&self) { helper(); }
}

pub trait Trait { fn f(&self); fn defaulted(&self) { helper(); } }
impl Trait for Point {
    fn f(&self) { self.m(); }
    fn defaulted(&self) { Self::m(self); }
}
impl Point {
    pub fn associated() -> Self { Self::new(1, 2) }
}

pub fn methods<T: Trait>(x: T) {
    x.f();
    x.defaulted();
    <T as Trait>::f(&x);
    Trait::f(&x);
    let p: fn() = helper;
    p();
    let c = || helper();
    fn map_fn(_: &i32) -> i32 { 0 }
    let values = [1, 2, 3];
    values.iter().map(map_fn).count();
}

pub fn constructors() {
    let _ = Point(1, 2);
    let _ = Point::new(1, 2);
    let _ = Shape::Circle(4);
    let _ = Some(1);
}

macro_rules! local_calls { () => { helper(); } }
pub fn macros() {
    println!("{:?}", helper());
    vec![g()];
    local_calls!();
}

pub fn std_calls(v: &mut Vec<i32>, s: &str) {
    let _ = v.len();
    let _ = s.to_string();
    let mut w = Vec::new();
    std::mem::swap(v, &mut w);
    drop(v);
}

pub fn recursive(n: usize) { if n > 0 { mutually_a(n - 1); recursive(n - 1); } }
fn mutually_a(n: usize) { if n > 0 { mutually_b(n - 1); } }
fn mutually_b(n: usize) { if n > 0 { mutually_a(n - 1); } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_only() { helper(); }
}

pub async fn async_fn() { helper(); }
pub async fn async_caller() { async_fn().await; }
