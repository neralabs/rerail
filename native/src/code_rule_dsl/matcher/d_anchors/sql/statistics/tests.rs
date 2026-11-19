use super::*;
use crate::code_rule_dsl::{
    language::{self, GrammarDialect},
    matcher::d_anchors::sql::{store::Store, tests::document},
};

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
fn estimate(store: &Store, language: GrammarDialect, kind: u16, term: Option<&str>) -> Estimate {
    let lang = u32::from(language::tape_tag(language).unwrap());
    let plan = request(kind, term);
    let stats = Statistics::read(&store.db, [(lang, &plan)]).unwrap();
    let Condition::Pattern(required) = &plan.condition else {
        unreachable!()
    };
    stats.choose(lang, required).1.unwrap()
}

#[test]
fn statistics_count_nodes_not_segments_and_follow_publication_and_removal() {
    let store = Store::temporary().unwrap();
    let lang = GrammarDialect::TypeScript;
    let kind = lang.kinds().kind_id("identifier");
    let a = store
        .put("a.ts", &document("target(); target();", lang))
        .unwrap();
    let b = store.put("b.ts", &document("target();", lang)).unwrap();
    assert_eq!(estimate(&store, lang, kind, Some("target")).rows, 3);
    assert_eq!(
        store
            .db
            .query_row("SELECT n FROM vlex WHERE term='target'", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    store.remove_many(&[a, b]).unwrap();
    assert_eq!(estimate(&store, lang, kind, Some("target")).rows, 0);
    assert_eq!(estimate(&store, lang, kind, None).rows, 0);
    assert_eq!(store.db.query_row("SELECT (SELECT count(*) FROM value_frequency)+(SELECT count(*) FROM value_kind_frequency)",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    store
        .put("a.ts", &document("replacement();", lang))
        .unwrap();
    store
        .put("a.ts", &document("target(); target(); target();", lang))
        .unwrap();
    assert_eq!(estimate(&store, lang, kind, Some("target")).rows, 3);
    assert_eq!(estimate(&store, lang, kind, Some("replacement")).rows, 0);
}

#[test]
fn statistics_keep_uncovered_values_conservative_and_languages_separate() {
    let store = Store::temporary().unwrap();
    let ts = GrammarDialect::TypeScript;
    store
        .put("a.ts", &document("function target() { return 1; }", ts))
        .unwrap();
    let function = ts.kinds().kind_id("function_declaration");
    let unknown = estimate(&store, ts, function, Some("absent text"));
    assert_eq!(
        unknown.rows, 1,
        "unindexed interior text is not proven absent"
    );
    assert_eq!(unknown.uncovered, 1);
    assert_eq!(
        estimate(&store, ts, ts.kinds().kind_id("identifier"), Some("absent")).rows,
        0
    );
    let py = GrammarDialect::Python;
    store
        .put("b.py", &document("target = target\n", py))
        .unwrap();
    assert_eq!(
        estimate(&store, py, py.kinds().kind_id("identifier"), Some("target")).rows,
        2
    );
    assert_eq!(
        estimate(&store, ts, ts.kinds().kind_id("identifier"), Some("target")).rows,
        1
    );
}

#[test]
fn statistics_rollback_a_failed_publication() {
    let store = Store::temporary().unwrap();
    let lang = GrammarDialect::TypeScript;
    store.put("a.ts", &document("target();", lang)).unwrap();
    store.db.execute_batch("CREATE TRIGGER reject_value BEFORE INSERT ON vidx WHEN NEW.term='explode' BEGIN SELECT RAISE(ABORT,'publication failed'); END").unwrap();
    assert!(
        store
            .put("a.ts", &document("target(); explode();", lang))
            .is_err()
    );
    assert_eq!(
        estimate(
            &store,
            lang,
            lang.kinds().kind_id("identifier"),
            Some("target")
        )
        .rows,
        1
    );
    assert_eq!(
        estimate(
            &store,
            lang,
            lang.kinds().kind_id("identifier"),
            Some("explode")
        )
        .rows,
        0
    );
}

#[test]
fn statistics_upgrade_backfills_old_segments_once_and_removes_the_migration_callback() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let lang = GrammarDialect::TypeScript;
    let store = Store::open(file.path()).unwrap();
    store
        .put("a.ts", &document("target(); target();", lang))
        .unwrap();
    store
        .db
        .execute_batch(
            "DROP TRIGGER value_insert;DROP TRIGGER value_delete;
        DROP TABLE value_frequency;DROP TABLE value_kind_frequency;
        ALTER TABLE vidx DROP COLUMN n;PRAGMA user_version=2",
        )
        .unwrap();
    drop(store);
    let store = Store::open(file.path()).unwrap();
    assert_eq!(
        estimate(
            &store,
            lang,
            lang.kinds().kind_id("identifier"),
            Some("target")
        )
        .rows,
        2
    );
    assert_eq!(
        store
            .db
            .query_row("SELECT n FROM vlex WHERE term='target'", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .db
            .query_row(
                "SELECT count(*) FROM pragma_function_list WHERE name='__rerail_posting_count'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .db
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    store.remove("a.ts").unwrap();
    assert_eq!(
        estimate(
            &store,
            lang,
            lang.kinds().kind_id("identifier"),
            Some("target")
        )
        .rows,
        0
    );
}

#[test]
fn statistics_ties_keep_pattern_order_and_an_empty_store_is_safe() {
    use crate::code_rule_dsl::matcher::d_anchors::tests::compile_rule;
    let lang = GrammarDialect::TypeScript;
    let matcher = compile_rule(
        &serde_json::json!({"pattern":"obj.target($ARG)"}),
        lang,
        "ordering",
        &mut Vec::new(),
    )
    .unwrap();
    let plan = AnchorPlan::compile(&matcher);
    let Condition::Pattern(required) = &plan.condition else {
        panic!("pattern plan")
    };
    assert!(
        required[0].parents.is_empty(),
        "the root precedes descendants in pattern order"
    );
    let store = Store::temporary().unwrap();
    let tag = u32::from(language::tape_tag(lang).unwrap());
    let stats = Statistics::read(&store.db, [(tag, &plan)]).unwrap();
    assert_eq!(stats.choose(tag, required).0, 0);
    assert_eq!(stats.choose(tag, required).1.unwrap().rows, 0);
    assert_eq!(
        Statistics::default().choose(tag, required).0,
        0,
        "unavailable statistics use a deterministic conservative arm"
    );
}
