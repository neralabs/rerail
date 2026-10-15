// Rerail-authored fixture (Apache-2.0).
macro_rules! capture {
    ($e:expr) => {{ let local = $e; local }};
}
macro_rules! define_local {
    () => {{ let hygienic = 7; hygienic }};
}

pub fn macro_scope(value: i32) -> i32 {
    let x = value;
    let y = capture!(x + 1);
    let z = define_local!();
    println!("{}", x);
    assert_eq!(y, z + 1);
    let values = vec![x, y, z];
    values.iter().copied().sum()
}

macro_rules! make_fn {
    ($name:ident) => { fn $name(value: i32) -> i32 { value } };
}
make_fn!(generated);

pub fn call_generated() -> i32 { generated(3) }
