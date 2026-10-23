//! Admission ledger for the subsequent main-compiler replacement. No rule is skipped.
use crate::code_rule_dsl::{
    CodeRuleDslRuntime,
    model::{ClauseKind, Query},
    test_support::CAPABILITIES,
};
use serde_json::json;
use sha2::{Digest, Sha256};

#[test]
#[ignore = "set RERAIL_RELATIONAL_CUTOVER_LEDGER"]
fn relational_cutover_catalog_admission() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let mut paths = std::fs::read_dir(root.join("packs/python/rules"))
        .unwrap()
        .map(|p| p.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect::<Vec<_>>();
    paths.sort();
    let mut rows = Vec::new();
    for path in paths {
        let source = std::fs::read_to_string(&path).unwrap();
        let policy = match CodeRuleDslRuntime::compile(
            source.clone(),
            "json".into(),
            CAPABILITIES.into(),
        ) {
            Ok(policy) => policy,
            Err(error) => {
                rows.push(json!({"policy":path.strip_prefix(root).unwrap().to_string_lossy(),"policySha256":format!("{:x}",Sha256::digest(source.as_bytes())),"stage":"policy compilation","reason":error.to_string()}));
                continue;
            }
        };
        for clause in &policy.inner.clauses {
            let mut grammars = Vec::new();
            if let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &clause.kind {
                for (index, grammar) in matchers.iter().enumerate() {
                    let result = match super::compile(grammar) {
                        Ok(compiled) => {
                            json!({"admitted":true,"sqlBytes":compiled.sql.len(),"sqlSha256":format!("{:x}",Sha256::digest(compiled.sql.as_bytes()))})
                        }
                        Err(error) => json!({"admitted":false,"reason":error}),
                    };
                    grammars.push(json!({"grammar":index,"result":result}));
                }
            }
            rows.push(json!({"policy":path.strip_prefix(root).unwrap().to_string_lossy(),"policySha256":format!("{:x}",Sha256::digest(source.as_bytes())),"clause":clause.id,"ast":!grammars.is_empty(),"grammars":grammars}));
        }
    }
    let report = json!({"scope":"Compilation admission only. This ledger does not certify execution parity, resource limits, or complete-job batching.","clauses":rows});
    std::fs::write(
        std::env::var("RERAIL_RELATIONAL_CUTOVER_LEDGER").unwrap(),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}
