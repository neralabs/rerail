use super::*;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::matcher::anchors::{Condition, Required, Requirement};

fn doc(source: &str) -> TapeDoc {
    super::super::tests::document(source, GrammarDialect::TypeScript)
}

fn plan() -> Arc<AnchorPlan> {
    let kinds = GrammarDialect::TypeScript.kinds();
    let call = kinds.kind_id("call_expression");
    Arc::new(AnchorPlan {
        root_kinds: Some([call].into()),
        condition: Condition::Pattern(vec![
            Requirement {
                value: Required::Kind(call),
                parents: vec![],
            },
            Requirement {
                value: Required::Kind(kinds.kind_id("identifier")),
                parents: vec![Some(call)],
            },
        ]),
        exact: false,
    })
}

fn template(corpus: &Corpus, plan: &Arc<AnchorPlan>) -> Arc<super::super::lower::Statement> {
    let mut guard = corpus.state.lock();
    let state = guard.as_mut().unwrap();
    let lang =
        u32::from(crate::code_rule_dsl::language::tape_tag(GrammarDialect::TypeScript).unwrap());
    let id = Arc::as_ptr(plan) as usize;
    state.plans.insert(id, plan.clone());
    let statistics =
        super::super::statistics::Statistics::read(&state.writer.db, [(lang, plan.as_ref())])
            .unwrap();
    Corpus::template(state, lang, &[id], &[plan.as_ref()], &statistics)
}

#[test]
fn anchor_planning_never_prices_posting_rows_during_execution() {
    let corpus = Corpus::default();
    corpus
        .put("a.ts", &doc("const x = y; target();"), &|| false)
        .unwrap();
    let statement = template(&corpus, &plan());
    let sql = statement.graph.join("\n");
    assert!(
        !sql.contains("SELECT count(*) FROM"),
        "anchor pricing enumerates postings: {sql}"
    );
    assert!(
        !sql.contains("ORDER BY cost"),
        "execution still chooses an anchor: {sql}"
    );
}

#[test]
fn anchor_planning_replaces_template_only_when_the_choice_flips() {
    let corpus = Corpus::default();
    let plan = plan();
    corpus
        .put("sentinel.ts", &doc("target();"), &|| false)
        .unwrap();
    corpus
        .put(
            "identifiers.ts",
            &doc(&"const x = y;\n".repeat(30)),
            &|| false,
        )
        .unwrap();
    let first = template(&corpus, &plan);
    assert_eq!(first.anchors[0].requirement, 0);
    assert!(Arc::ptr_eq(&first, &template(&corpus, &plan)));
    corpus
        .put("more-identifiers.ts", &doc("const z = w;"), &|| false)
        .unwrap();
    assert!(
        Arc::ptr_eq(&first, &template(&corpus, &plan)),
        "new statistics with the same winner must reuse the template"
    );
    corpus
        .put("calls.ts", &doc(&"(0)();\n".repeat(200)), &|| false)
        .unwrap();
    let second = template(&corpus, &plan);
    assert_eq!(second.anchors[0].requirement, 1);
    assert!(
        !Arc::ptr_eq(&first, &second),
        "newly rare identifier must replace the cached call anchor"
    );
    assert!(Arc::ptr_eq(&second, &template(&corpus, &plan)));
}

#[test]
fn anchor_planning_does_not_enter_an_unchosen_posting_arm() {
    let corpus = Corpus::default();
    let doc = doc(&("const x = y;\n".repeat(30) + "target();"));
    let fid = corpus.put("a.ts", &doc, &|| false).unwrap();
    let identifier = GrammarDialect::TypeScript.kinds().kind_id("identifier");
    corpus
        .state
        .lock()
        .as_ref()
        .unwrap()
        .writer
        .db
        .execute(
            "UPDATE file_kind_counts SET postings=X'80' WHERE fid=?1 AND code=?2",
            rusqlite::params![fid, identifier],
        )
        .unwrap();
    let plan = plan();
    let language =
        u32::from(crate::code_rule_dsl::language::tape_tag(GrammarDialect::TypeScript).unwrap());
    corpus
        .prepare(&BTreeMap::from([(language, vec![plan.clone()])]))
        .unwrap();
    let file = corpus
        .prepared_file("a.ts", language, doc.index_source_digest())
        .unwrap();
    assert_eq!(
        file.roots[&(Arc::as_ptr(&plan) as usize)]
            .ordinals()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn anchor_planning_literal_seed_never_enumerates_the_common_root_or_identifier_kind() {
    let corpus = Corpus::default();
    let source = doc("target(); other(); other();");
    let fid = corpus.put("a.ts", &source, &|| false).unwrap();
    let kinds = GrammarDialect::TypeScript.kinds();
    let call = kinds.kind_id("call_expression");
    let identifier = kinds.kind_id("identifier");
    let plan = Arc::new(AnchorPlan {
        root_kinds: Some([call].into()),
        exact: false,
        condition: Condition::Pattern(vec![
            Requirement {
                value: Required::Kind(call),
                parents: vec![],
            },
            Requirement {
                value: Required::Terminal(identifier, "target".into()),
                parents: vec![Some(call)],
            },
        ]),
    });
    corpus
        .state
        .lock()
        .as_ref()
        .unwrap()
        .writer
        .db
        .execute(
            "UPDATE file_kind_counts SET postings=X'80' WHERE fid=?1 AND code IN (?2,?3)",
            rusqlite::params![fid, call, identifier],
        )
        .unwrap();
    let statement = template(&corpus, &plan);
    assert_eq!(statement.anchors[0].requirement, 1);
    assert_eq!(statement.anchors[0].estimated_rows, Some(1));
    let language =
        u32::from(crate::code_rule_dsl::language::tape_tag(GrammarDialect::TypeScript).unwrap());
    corpus
        .prepare(&BTreeMap::from([(language, vec![plan.clone()])]))
        .unwrap();
    let file = corpus
        .prepared_file("a.ts", language, source.index_source_digest())
        .unwrap();
    assert_eq!(
        file.roots[&(Arc::as_ptr(&plan) as usize)]
            .ordinals()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn anchor_planning_retains_a_cheap_conjunct_when_a_larger_conjunct_exceeds_admission() {
    let corpus = Corpus::default();
    let doc = doc("target();");
    corpus.put("a.ts", &doc, &|| false).unwrap();
    let kinds = GrammarDialect::TypeScript.kinds();
    let call = kinds.kind_id("call_expression");
    let identifier = kinds.kind_id("identifier");
    let leaf = |term: String| AnchorPlan {
        root_kinds: Some([call].into()),
        exact: false,
        condition: Condition::Pattern(vec![Requirement {
            value: Required::Terminal(identifier, term),
            parents: vec![Some(call)],
        }]),
    };
    let expensive = AnchorPlan {
        root_kinds: Some([call].into()),
        exact: false,
        condition: Condition::All((0..400).map(|i| leaf(format!("missing_{i}"))).collect()),
    };
    let plan = Arc::new(AnchorPlan {
        root_kinds: Some([call].into()),
        exact: false,
        condition: Condition::All(vec![expensive, leaf("absent".into())]),
    });
    let statement = template(&corpus, &plan);
    assert!(
        statement.admissions[0].partial,
        "the original compiler allowance still bounds this plan"
    );
    let lang =
        u32::from(crate::code_rule_dsl::language::tape_tag(GrammarDialect::TypeScript).unwrap());
    corpus
        .prepare(&BTreeMap::from([(lang, vec![plan.clone()])]))
        .unwrap();
    let file = corpus
        .prepared_file("a.ts", lang, doc.index_source_digest())
        .unwrap();
    assert!(
        file.roots[&(Arc::as_ptr(&plan) as usize)]
            .ordinals()
            .unwrap()
            .is_empty(),
        "bounded admission must retain the rejecting conjunct"
    );
}
