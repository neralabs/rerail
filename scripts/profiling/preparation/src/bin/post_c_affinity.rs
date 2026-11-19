fn main() -> Result<(), Box<dyn std::error::Error>> {
    // SAFETY: configure accounting before this process opens any SQLite connection.
    let code =
        unsafe { rusqlite::ffi::sqlite3_config(rusqlite::ffi::SQLITE_CONFIG_MEMSTATUS, 1i32) };
    if code != rusqlite::ffi::SQLITE_OK {
        return Err(format!("cannot enable SQLite memory accounting: {code}").into());
    }
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.len() != 2 {
        return Err("usage: post_c_affinity <programs.json> <report.json>".into());
    }
    rerail_sql_preparation_lab::post_c::affinity(
        std::path::Path::new(&args[0]),
        std::path::Path::new(&args[1]),
    )
}
