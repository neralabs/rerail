use super::*;
use crate::code_rule_dsl::{
    language::{self, GrammarDialect},
    prefilter::tests::{document, matcher, open, put, temp},
    tape::TAPE_FORMAT_VERSION,
};
use crate::content_store::ContentStore;

fn request(kind: u16, term: Option<&str>) -> AnchorPlan {
    AnchorPlan {
        root_kinds: None,
        exact: false,
        condition: Condition::Pattern(vec![Requirement {
            value: term.map_or(Required::Kind(kind), |s| Required::Terminal(kind, s.into())),
            parents: vec![],
        }]),
    }
}
fn read(store: &ContentStore, lang: u32, plan: &AnchorPlan) -> Statistics {
    store
        .candidate_transaction(false, |db| Statistics::read(db, [(lang, plan)]))
        .unwrap()
}
fn estimate(store: &ContentStore, language: GrammarDialect, kind: u16, term: Option<&str>) -> Estimate {
    let lang = u32::from(language::tape_tag(language).unwrap());
    let plan = request(kind, term);
    let stats = read(store, lang, &plan);
    let Condition::Pattern(required) = &plan.condition else {
        unreachable!()
    };
    stats.choose(lang, required).1.unwrap()
}
fn count(store: &ContentStore, sql: &str) -> i64 {
    store.query_rows(sql, [], |r| r.get(0)).unwrap()[0]
}

#[test]
fn statistics_count_nodes_not_segments_and_follow_publication_and_removal() {
    let store = open(&temp());
    let lang = GrammarDialect::TypeScript;
    let kind = lang.kinds().kind_id("identifier");
    let (a, _) = put(&store, "target(); target();", lang);
    let (b, _) = put(&store, "target();", lang);
    assert_eq!(estimate(&store, lang, kind, Some("target")).rows, 3);
    assert_eq!(count(&store, "SELECT n FROM term_freq WHERE term='target'"), 2);
    store.remove_prefilter(a).unwrap();
    store.delete_tape_for_test(b).unwrap();
    assert_eq!(estimate(&store, lang, kind, Some("target")).rows, 0);
    assert_eq!(estimate(&store, lang, kind, None).rows, 0);
    assert_eq!(count(&store, "SELECT (SELECT count(*) FROM term_node_freq)+(SELECT count(*) FROM term_kind_freq)"), 0);
    put(&store, "replacement();", lang);
    put(&store, "target(); target(); target();", lang);
    assert_eq!(estimate(&store, lang, kind, Some("target")).rows, 3);
    assert_eq!(estimate(&store, lang, kind, Some("absent")).rows, 0);
}

#[test]
fn statistics_keep_uncovered_values_conservative_and_languages_separate() {
    let store = open(&temp());
    let ts = GrammarDialect::TypeScript;
    put(&store, "function target() { return 1; }", ts);
    let function = ts.kinds().kind_id("function_declaration");
    let unknown = estimate(&store, ts, function, Some("absent text"));
    assert_eq!(unknown.rows, 1, "unindexed interior text is not proven absent");
    assert_eq!(unknown.uncovered, 1);
    assert_eq!(estimate(&store, ts, ts.kinds().kind_id("identifier"), Some("absent")).rows, 0);
    let py = GrammarDialect::Python;
    put(&store, "target = target\n", py);
    assert_eq!(estimate(&store, py, py.kinds().kind_id("identifier"), Some("target")).rows, 2);
    assert_eq!(estimate(&store, ts, ts.kinds().kind_id("identifier"), Some("target")).rows, 1);
}

#[test]
fn statistics_rollback_a_failed_publication() {
    let store = open(&temp());
    let lang = GrammarDialect::TypeScript;
    let identifier = lang.kinds().kind_id("identifier");
    put(&store, "target();", lang);
    store
        .candidate_transaction(true, |db| {
            db.execute_batch("CREATE TRIGGER reject_value BEFORE INSERT ON term_postings WHEN NEW.term='explode' BEGIN SELECT RAISE(ABORT,'publication failed'); END")
        })
        .unwrap();
    let source = "target(); explode();";
    let (doc, bytes) = document(source, lang);
    let grammar = i64::from(language::tape_tag(lang).unwrap());
    let source_id = store.put_source(source.as_bytes(), &[]).unwrap();
    let tape = store.put_tape(source_id, grammar, 1, i64::from(TAPE_FORMAT_VERSION), &bytes).unwrap();
    assert!(crate::code_rule_dsl::prefilter::publish(&store, &[tape], &[doc]).is_err());
    assert_eq!(estimate(&store, lang, identifier, Some("target")).rows, 1);
    assert_eq!(estimate(&store, lang, identifier, Some("explode")).rows, 0);
}

#[test]
fn statistics_ties_keep_pattern_order_and_an_empty_store_is_safe() {
    let lang = GrammarDialect::TypeScript;
    let matcher = matcher(&serde_json::json!({"pattern":"obj.target($ARG)"}), lang);
    let plan = AnchorPlan::compile(&matcher);
    let Condition::Pattern(required) = &plan.condition else {
        panic!("pattern plan")
    };
    assert!(required[0].parents.is_empty(), "the root precedes descendants in pattern order");
    let store = open(&temp());
    let tag = u32::from(language::tape_tag(lang).unwrap());
    let stats = read(&store, tag, &plan);
    assert_eq!(stats.choose(tag, required).0, 0);
    assert_eq!(stats.choose(tag, required).1.unwrap().rows, 0);
    assert_eq!(
        Statistics::default().choose(tag, required).0,
        0,
        "unavailable statistics use a deterministic conservative arm"
    );
}
