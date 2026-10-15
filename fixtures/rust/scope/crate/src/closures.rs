// Rerail-authored fixture (Apache-2.0).
pub fn closure_params(values: &[i32]) -> i32 {
    let offset = 2;
    let add = |mut value: i32| { value += offset; value };
    let pair = |(left, right): (i32, i32)| left + right;
    values.iter().map(|&item| add(item)).fold(0, |sum, item| sum + item) + pair((1, 2))
}

pub fn nested_function() -> i32 {
    let local = 4;
    fn inner(value: i32) -> i32 { value + 1 }
    inner(local)
}

pub fn move_closure() -> String {
    let text = String::from("scope");
    let consume = move || text;
    consume()
}

pub fn callback<F: Fn(i32) -> i32>(f: F) -> i32 {
    let result = f(3);
    result
}
