use super::*;
use crate::code_rule_dsl::logic::rr::parse;

fn packs() -> Vec<(String, Value)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../packs");
    let mut out = Vec::new();
    for pack in std::fs::read_dir(root).unwrap() {
        let rules = pack.unwrap().path().join("rules");
        for f in std::fs::read_dir(&rules).into_iter().flatten() {
            let path = f.unwrap().path();
            if path.extension().is_some_and(|e| e == "json") {
                let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
                // Packs also ship conversation (command) rules, which are no policy documents.
                if doc["format"] == "rerail.policy/v1" {
                    out.push((path.display().to_string(), doc));
                }
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Every pack's policy round-trips: document → AST → `.rr` text → AST → the same document, its matchers as `ast`
/// atoms of `violation` rules.
#[test]
fn pack_policies_round_trip_through_datalog_text() {
    let docs = packs();
    assert!(docs.len() > 300, "{} documents", docs.len());
    let mut rules = 0;
    for (path, doc) in &docs {
        let mut prog = Program::default();
        // A forbid clause of another form (main's packs hold `regex` ones) is no matcher: the front refuses
        // the whole policy, as the wave path does (`waves::policies`).
        let ast_only = doc["clauses"].as_array().into_iter().flatten().all(|clause| {
            clause.get("forbid").is_none_or(|forbid| forbid.as_object().is_some_and(|f| f.len() == 1 && f.contains_key(MARK)))
        });
        if !ast_only {
            assert!(add(&mut prog, doc).is_err(), "{path}: a non-ast forbid clause is refused");
            continue;
        }
        add(&mut prog, doc).unwrap_or_else(|e| panic!("{path}: {e}"));
        rules += prog.rules.len();
        let text = text(&prog);
        let back = parse(&text).unwrap_or_else(|e| panic!("{path}: {e}\n{text}"));
        assert_eq!(back.rules, prog.rules, "{path}");
        assert_eq!(documents(&back).unwrap(), [doc.clone()], "{path}");
    }
    assert!(rules >= 500, "{rules} matcher rules");
}

/// Several documents share one program, and YAML reads to the same document as JSON.
#[test]
fn documents_share_a_program_and_yaml_reads_like_json() {
    let docs: Vec<Value> = packs().into_iter().take(3).map(|(_, d)| d).collect();
    let mut prog = Program::default();
    for d in &docs {
        add(&mut prog, d).unwrap();
    }
    assert_eq!(documents(&parse(&text(&prog)).unwrap()).unwrap(), docs);
    let yaml = serde_yaml_ng::to_string(&docs[0]).unwrap();
    assert_eq!(read(&yaml).unwrap(), docs[0]);
    let first = &prog.rules[0];
    assert!(matches!(first.body.as_slice(), [Literal::Ast { document, node, .. }] if document == "D" && node == "N"));
    assert_eq!(first.head.rel, "violation");
}
