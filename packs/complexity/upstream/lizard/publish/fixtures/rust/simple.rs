// Only simple functions: nothing here is over the limit.

pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn describe(value: Option<i32>) -> String {
    match value {
        Some(number) => number.to_string(),
        None => String::from("nothing"),
    }
}

pub fn first(items: &[i32]) -> Option<i32> {
    for item in items {
        return Some(*item);
    }
    None
}
