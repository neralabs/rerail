fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 3 {
        return Err("usage: plan_reuse <programs.json> <repeats> <output.json>".into());
    }
    rerail_sql_preparation_lab::post_c::plan_reuse(
        std::path::Path::new(&args[0]),
        args[1].parse()?,
        std::path::Path::new(&args[2]),
    )
}
