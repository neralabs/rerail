fn main() -> Result<(), Box<dyn std::error::Error>> {
    // SAFETY: this diagnostic process enables accounting before any SQLite initialization.
    let code =
        unsafe { rusqlite::ffi::sqlite3_config(rusqlite::ffi::SQLITE_CONFIG_MEMSTATUS, 1i32) };
    if code != rusqlite::ffi::SQLITE_OK {
        return Err(format!("cannot enable SQLite diagnostic memory accounting: {code}").into());
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err("usage: lifetimes <candidate.json> <eager|scheduled> <output.json>".into());
    }
    rerail_sql_preparation_lab::residency::run(
        std::path::Path::new(&args[0]),
        &args[1],
        std::path::Path::new(&args[2]),
    )?;
    Ok(())
}
