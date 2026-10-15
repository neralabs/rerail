// Rerail-authored fixture (Apache-2.0).

pub mod lets;
pub mod patterns;
pub mod closures;
pub mod items;
pub mod imports;
pub mod generics;
pub mod macros;
pub mod unused;

pub fn root() -> i32 {
    let x = 1;
    let x = x + 1;
    x
}
