//! The spike's two modes on a small Python corpus, through Rerail's grammars: `measure` checks
//! each format's round trip, and `sql` checks the SQL decodings against the Rust ones.

use std::time::{SystemTime, UNIX_EPOCH};

fn corpus() -> std::path::PathBuf {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("rerail-v9np-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for n in 0..4 {
        let source = format!(
            "def f{n}(x):\n    if x:\n        return g(\"a{n}\", x)\n    return [h(y) for y in x]\n\n\
             class C{n}:\n    def m(self):\n        return f{n}(self)\n"
        );
        std::fs::write(dir.join(format!("m{n}.py")), source).unwrap();
    }
    dir
}

#[test]
#[ignore = "a measurement run (P's v8 spike `measure`), no assertions; run with --include-ignored"]
fn measure_runs_on_rerails_python_grammar() {
    let dir = corpus();
    let args = ["v8spike", "measure", "small", "python", dir.to_str().unwrap()].map(String::from);
    super::main(&args);
}

#[test]
fn sql_decodings_agree_with_the_rust_ones() {
    let dir = corpus();
    let db = dir.join("spike.sqlite");
    let args = ["v8spike", "sql", "python", db.to_str().unwrap(), "100", dir.to_str().unwrap()].map(String::from);
    super::main(&args);
}
