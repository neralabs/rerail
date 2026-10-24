fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 {
        return Err(
            "usage: candidate_owner <candidate.json> <logical-graph.json> <new-output.json>".into(),
        );
    }
    rerail_sql_preparation_lab::candidate_owner::rewrite(
        std::path::Path::new(&args[0]),
        std::path::Path::new(&args[1]),
        std::path::Path::new(&args[2]),
    )
}
