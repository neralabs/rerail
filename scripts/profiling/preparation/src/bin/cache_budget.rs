fn main() -> Result<(), Box<dyn std::error::Error>> {
    // SAFETY: enable diagnostic accounting before this process initializes SQLite.
    let code =
        unsafe { rusqlite::ffi::sqlite3_config(rusqlite::ffi::SQLITE_CONFIG_MEMSTATUS, 1i32) };
    if code != rusqlite::ffi::SQLITE_OK {
        return Err(format!("cannot enable SQLite diagnostic memory accounting: {code}").into());
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 4 {
        return Err(
            "usage: cache_budget <candidate.json> <main-kib> <temp-kib> <output.json>".into(),
        );
    }
    let (main_kib, temp_kib): (i64, i64) = (args[1].parse()?, args[2].parse()?);
    if main_kib <= 0 || temp_kib <= 0 {
        return Err("cache budgets must be positive KiB values".into());
    }
    rerail_sql_preparation_lab::residency::run_cache(
        std::path::Path::new(&args[0]),
        main_kib,
        temp_kib,
        std::path::Path::new(&args[3]),
    )?;
    Ok(())
}
