// Where lizard's Rust reader and Rerail differ on purpose (README, "Deviations"); each is a listed deviation.

// A where clause bounds types, it decides nothing: lizard counts each where (2), Rerail none (1).
pub fn bounded<T>(value: T) -> T
where
    T: Clone,
{
    value.clone()
}

// ?Sized relaxes a bound, it is no ? operator: lizard counts it (2), Rerail nothing (1).
pub fn unsized_len<T: ?Sized + AsRef<str>>(value: &T) -> usize {
    value.as_ref().len()
}

// || with nothing between the bars opens a closure: lizard counts it as an or (4), Rerail only the two real || (3).
pub fn empty_closure(values: &[i32]) -> bool {
    let fallback = || false;
    values.iter().any(|v| *v < 0 || *v > 100) || fallback()
}

// Rerail reads Rust's syntax before macro expansion, so not inside a macro invocation's tokens: lizard counts the && and
// the if there (3), Rerail none (1).
pub fn in_macros(ready: bool, forced: bool) {
    assert!(ready && forced);
    println!("{}", if ready { "ready" } else { "waiting" });
}

// A char or byte literal is no lifetime: lizard reads b'0' as one and loses the && after it (2), Rerail counts it (3).
pub fn starts_with_digit(text: &str) -> bool {
    match text.as_bytes().first() {
        None => false,
        Some(&byte) => byte >= b'0' && byte <= b'9',
    }
}
