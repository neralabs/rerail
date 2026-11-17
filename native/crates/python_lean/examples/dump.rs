//! `dump <file>` (or `-` and the text as the next argument): the lean parser's tape as plain text, or its refusal.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let src = if args[1] == "-" { args[2].clone().into_bytes() } else { std::fs::read(&args[1]).unwrap() };
    match rerail_python_lean::tape(&src) {
        Ok(t) => print!("{}", rerail_python_lean::tape_text(&t)),
        Err(r) => println!("refused: {r:?}"),
    }
}
