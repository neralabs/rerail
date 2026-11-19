//! Public SDK smoke test: a background caller, bounded SQL work and one shared pool.
//! RERAIL_THREADS=1 nice -n 19 cargo run --example sql-work-plan --features headless
use rerail_engine::tools::CodeRuleSqlExecutor;
use serde_json::{Value, json};

fn main() {
    let directory = tempfile::tempdir().unwrap();
    let store = directory.path().join("facts.sqlite");
    let result=std::thread::spawn(move || {
        let mut session=CodeRuleSqlExecutor::new(store.to_string_lossy().into_owned()).unwrap();
        session.bind_relation(json!({"name":"edges","columns":["src","dst"],
            "rows":[[1,2],[2,3],[3,4]]}).to_string()).unwrap();
        session.query_plan(json!({"nodes":[
            {"name":"reach","width":2,"reads":["edges"],"sql":
                "WITH RECURSIVE pairs(a,b) AS (SELECT src,dst FROM edges UNION SELECT p.a,e.dst FROM pairs p JOIN edges e ON e.src=p.b) SELECT a,b FROM pairs"},
            {"name":"counts","width":2,"reads":["reach"],"sql":
                "SELECT c0,count(*) FROM reach GROUP BY c0 ORDER BY c0"}
        ],"outputs":["counts"],"max_submissions":1}).to_string()).unwrap()
    }).join().unwrap();
    let result: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["submissions"], 1);
    assert_eq!(result["rows"]["counts"], json!([[1, 3], [2, 2], [3, 1]]));
    println!(
        "{}",
        json!({"submissions":result["submissions"],"rows":result["rows"],"vmSteps":result["vmSteps"]})
    );
}
