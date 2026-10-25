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
            Requirement { value: Required::Kind(call), parents: vec![] },
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
    let lang = u32::from(crate::code_rule_dsl::language::tape_tag(GrammarDialect::TypeScript).unwrap());
    let id = Arc::as_ptr(plan) as usize;
    state.plans.insert(id, plan.clone());
    Corpus::template(state, lang, &[id], &[plan.as_ref()])
}

#[test]
fn anchor_planning_never_prices_posting_rows_during_execution() {
    let corpus = Corpus::default();
    corpus.put("a.ts", &doc("const x = y; target();"), &|| false).unwrap();
    let statement = template(&corpus, &plan());
    let sql = statement.graph.join("\n");
    assert!(!sql.contains("SELECT count(*) FROM"), "anchor pricing enumerates postings: {sql}");
    assert!(!sql.contains("ORDER BY cost"), "execution still chooses an anchor: {sql}");
}

#[test]
fn anchor_planning_replaces_template_only_when_the_choice_flips() {
    let corpus = Corpus::default();
    let plan = plan();
    corpus.put("sentinel.ts", &doc("target();"), &|| false).unwrap();
    corpus.put("identifiers.ts", &doc(&"const x = y;\n".repeat(30)), &|| false).unwrap();
    let first = template(&corpus, &plan);
    assert!(Arc::ptr_eq(&first, &template(&corpus, &plan)));
    corpus.put("more-identifiers.ts", &doc("const z = w;"), &|| false).unwrap();
    assert!(Arc::ptr_eq(&first, &template(&corpus, &plan)), "new statistics with the same winner must reuse the template");
    corpus.put("calls.ts", &doc(&"(0)();\n".repeat(200)), &|| false).unwrap();
    let second = template(&corpus, &plan);
    assert!(!Arc::ptr_eq(&first, &second), "newly rare identifier must replace the cached call anchor");
    assert!(Arc::ptr_eq(&second, &template(&corpus, &plan)));
}
