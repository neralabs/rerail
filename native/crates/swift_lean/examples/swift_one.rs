//! `swift_one <file>`: the tape of a file as text, or its refusal (for probing the parser by hand).
fn main() {
    let path = std::env::args().nth(1).expect("a file");
    let src = std::fs::read(&path).unwrap();
    match rerail_swift_lean::tape(&src) {
        Ok(t) => print!("{}", rerail_swift_lean::tape_text(&t)),
        Err(r) => println!("refused {r:?}"),
    }
}
