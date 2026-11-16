//! `one <text>`: the tape of a text, or its refusal (for probing the parser by hand).
fn main() {
    let src = std::env::args().nth(1).unwrap();
    match rerail_java_lean::tape(src.as_bytes()) {
        Ok(_) => println!("ok"),
        Err(r) => println!("refused {r:?}"),
    }
}
