fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: owner_projection <candidate.json> <output.json>".into());
    }
    rerail_sql_preparation_lab::owner_projection::rewrite(
        std::path::Path::new(&args[0]),
        std::path::Path::new(&args[1]),
    )
}
