fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 6 {
        return Err("usage: composition <capture.sqlite> <file-ids: 2,3> <range|demand|utility|batch> <variant> <depth> <output.json>".into());
    }
    let files = args[1]
        .split(',')
        .map(str::parse)
        .collect::<Result<Vec<i64>, _>>()?;
    let result = rerail_sql_preparation_lab::composition::run(
        std::path::Path::new(&args[0]),
        &files,
        &args[2],
        &args[3],
        args[4].parse()?,
    )?;
    std::fs::write(&args[5], serde_json::to_vec_pretty(&result)?)?;
    println!(
        "{} {}: {} rows, {:.3} ms kernel CPU",
        args[2],
        args[3],
        result["rows"].as_array().unwrap().len(),
        result["kernelCpuMs"].as_f64().unwrap()
    );
    Ok(())
}
