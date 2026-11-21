//! `one <text>`: the tape of a text, or its refusal (for probing the parser by hand).
fn main() {
    let src = std::env::args().nth(1).unwrap();
    match rerail_php_lean::tape(src.as_bytes()) {
        Ok(t) => print!("{}", rerail_php_lean::tape_text(&t)),
        Err(r) => println!("refused {r:?}"),
    }
}
