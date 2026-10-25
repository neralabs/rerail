fn main() -> Result<(), Box<dyn std::error::Error>> {
    rerail_sql_preparation_lab::post_c::inference_guard(std::path::Path::new(
        &std::env::args()
            .nth(1)
            .expect("inference-program.json: inputs and captured SQL"),
    ))
}
