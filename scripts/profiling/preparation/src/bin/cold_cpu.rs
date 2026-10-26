fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 5 {
        return Err("usage: cold_cpu <capture.sqlite> <anchor|fusion|native|planning|composed|publication> <variant> <scale> <output.json>".into());
    }
    let report = rerail_sql_preparation_lab::cold_cpu::run(
        std::path::Path::new(&args[0]),
        &args[1],
        &args[2],
        args[3].parse()?,
    )?;
    std::fs::write(&args[4], serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{} {}: {} rows; {:.3} ms measured CPU",
        args[1],
        args[2],
        report["rows"].as_array().map_or(0, Vec::len),
        report["cpuMs"].as_f64().unwrap()
    );
    Ok(())
}
