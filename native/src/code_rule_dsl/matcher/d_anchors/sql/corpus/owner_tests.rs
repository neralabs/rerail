use super::*;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::matcher::anchors::tests::compile_rule;
use serde_json::json;

fn doc(source: &str) -> TapeDoc {
    super::super::tests::document(source, GrammarDialect::TypeScript)
}

#[test]
fn one_sql_publication_serves_independent_memberships_and_coexisting_versions() {
    let owner = Corpus::default();
    let a = owner.fork();
    let b = owner.fork();
    let (old, new) = (doc("target();"), doc("other();"));
    let first = a.put("a.ts", &old, &|| false).unwrap();
    let changes = owner.census().0;
    assert_eq!(b.put("a.ts", &old, &|| false).unwrap(), first);
    assert_eq!(
        owner.census().0,
        changes,
        "reuse performs no SQL publication"
    );
    let second = b.put("a.ts", &new, &|| false).unwrap();
    assert_ne!(first, second);
    assert_eq!(
        owner.census().1,
        2,
        "versions coexist in one physical index"
    );
    assert!(
        a.prepared_file("a.ts", 1, new.index_source_digest())
            .is_none()
    );
    b.remove("a.ts").unwrap();
    assert_eq!(a.members.lock()["a.ts"].1.fid, first);
    assert!(b.members.lock().is_empty());
    let changes = owner.census().0;
    assert_eq!(b.put("a.ts", &old, &|| false).unwrap(), first);
    assert_eq!(owner.census().0, changes);
}

#[test]
fn prepared_candidates_share_only_equal_admitted_domains_and_complete_empty_outputs() {
    let owner = Corpus::default();
    let a = owner.fork();
    let b = owner.fork();
    let source = doc("other();");
    for domain in [&a, &b] {
        domain.put("a.ts", &source, &|| false).unwrap();
    }
    let matcher = compile_rule(
        &json!({"pattern":"target($VALUE)"}),
        GrammarDialect::TypeScript,
        "owner",
        &mut Vec::new(),
    )
    .unwrap();
    let plans = BTreeMap::from([(
        u32::from(crate::code_rule_dsl::language::tape_tag(GrammarDialect::TypeScript).unwrap()),
        vec![Arc::new(AnchorPlan::compile(&matcher))],
    )]);
    a.prepare(&plans).unwrap();
    let queries = owner.batch_queries();
    b.prepare(&plans).unwrap();
    assert_eq!(
        owner.batch_queries(),
        queries,
        "empty completion is reusable"
    );
    b.put("b.ts", &source, &|| false).unwrap();
    b.prepare(&plans).unwrap();
    assert!(
        owner.batch_queries() > queries,
        "a different domain has its own preparation binding"
    );
    let file = a.members.lock()["a.ts"].1.clone();
    assert_eq!(
        file.scope.len(),
        1,
        "another consumer cannot overwrite a pinned prepared view"
    );
}

#[test]
fn failed_publication_does_not_publish_membership_and_last_owner_releases_the_database() {
    let owner = Corpus::default();
    let a = owner.fork();
    assert!(a.put("a.ts", &doc("target();"), &|| true).is_err());
    assert!(a.members.lock().is_empty());
    assert_eq!(owner.census().1, 0);
    a.put("a.ts", &doc("target();"), &|| false).unwrap();
    let path = owner
        .state
        .lock()
        .as_ref()
        .unwrap()
        .writer
        .db
        .path()
        .unwrap()
        .to_owned();
    drop(owner);
    assert!(std::path::Path::new(&path).exists());
    drop(a);
    assert!(!std::path::Path::new(&path).exists());
}

#[test]
fn complete_structural_index_preserves_first_child_and_absence_for_every_field() {
    use crate::code_rule_dsl::matcher::anchors::sql::{execute, store::Store};
    for (lang, source) in [
        (
            GrammarDialect::TypeScript,
            "class Ω { method(a: number, b = target(x)) { return f(a, b); } }",
        ),
        (
            GrammarDialect::Python,
            "@decorate\nclass Ω:\n def method(self, a=None):\n  return f(a, target(x))\n",
        ),
        (
            GrammarDialect::Rust,
            "fn main(a: i32) { if a > 0 { f(a, target()); } }",
        ),
    ] {
        let doc = super::super::tests::document(source, lang);
        let store = Store::temporary().unwrap();
        let fid = store.put("source", &doc).unwrap();
        execute::prepare_structural(&store, &[fid]).unwrap();
        let language = u32::from(crate::code_rule_dsl::language::tape_tag(lang).unwrap());
        let mismatches = store.db.prepare("WITH fields(field) AS (SELECT 0 UNION ALL SELECT fcode FROM field WHERE lang=?2), answers AS MATERIALIZED (SELECT p.pre,f.field,nera_child(p.fid,p.pre,f.field) AS original,(SELECT c.pre FROM decoded_properties c INDEXED BY decoded_child_lookup WHERE c.fid=p.fid AND c.parent=p.pre AND c.field=f.field AND c.pre>p.pre ORDER BY c.pre LIMIT 1) AS replacement FROM decoded_properties p CROSS JOIN fields f WHERE p.fid=?1) SELECT pre,field,original,replacement FROM answers WHERE original IS NOT replacement").unwrap()
            .query_map(rusqlite::params![fid, language], |row| Ok((row.get::<_,i64>(0)?, row.get::<_,i64>(1)?, row.get::<_,Option<i64>>(2)?, row.get::<_,Option<i64>>(3)?)))
            .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        assert!(mismatches.is_empty(), "{lang:?}: {mismatches:?}");
    }
}
