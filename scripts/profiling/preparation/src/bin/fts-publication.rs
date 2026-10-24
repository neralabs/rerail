fn main() {
    let output = std::env::args()
        .nth(1)
        .expect("usage: fts-publication NEW_OUTPUT_JSON");
    let report = rerail_sql_preparation_lab::fts::experiment();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .unwrap();
    serde_json::to_writer_pretty(&mut file, &report).unwrap();
    println!("{report}");
}
