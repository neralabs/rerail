mod facts;

use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("parse") => {
            let path = &args[2];
            let source = Arc::new(std::fs::read_to_string(path).unwrap());
            match facts::build(path, source).unwrap() {
                facts::Built::Facts(f) => {
                    println!("nodes {} errors {}", f.len(), f.syntax_errors);
                    for id in 0..f.len().min(40) as u32 {
                        println!(
                            "{id} {} parent={} field={} named={} [{},{}) L{}",
                            facts::name(f.kind[id as usize]),
                            f.parent[id as usize] as i64,
                            f.field[id as usize],
                            f.named(id),
                            f.start[id as usize],
                            f.end[id as usize],
                            f.start_line[id as usize]
                        );
                    }
                }
                facts::Built::Unparsed => println!("unparsed"),
            }
        }
        _ => eprintln!("usage: facts-a parse <file>"),
    }
}
