use super::*;
use crate::code_rule_dsl::{
    language::{self, GrammarDialect},
    matcher::AstMatcher,
    prefilter::tests::{open, put, temp},
    tape::TapeDoc,
};
use crate::content_store::{ContentStore, StoreResult};
use ast_grep_core::{AstGrep, matcher::MatcherExt};
use serde_json::{Value as Json, json};

fn store() -> ContentStore {
    open(&temp())
}

fn run(store: &ContentStore, statement: &Statement, emit: impl FnMut(usize, i64, usize)) -> StoreResult<()> {
    super::super::execute::candidates(store, statement, emit)
}

fn temp_tables(store: &ContentStore, filter: &str) -> i64 {
    store.query_rows(&format!("SELECT count(*) FROM temp.sqlite_schema WHERE {filter}"), [], |r| r.get(0)).unwrap()[0]
}

fn compiler() -> Lower {
    Lower::new(1)
}

#[test]
fn scheduled_execution_releases_all_tables_after_success_and_a_later_packet_error() {
    let store = store();
    let mut statement = finish(
        vec![
            "early(fid,pre) AS MATERIALIZED (VALUES(1,2))".into(),
            "late(fid,pre) AS MATERIALIZED (VALUES(1,3))".into(),
        ],
        vec![
            "SELECT 0,fid,pre FROM early".into(),
            "SELECT 1,fid,pre FROM late".into(),
        ],
        vec![],
        BTreeSet::from(["early".into(), "late".into()]),
    );
    let mut rows = Vec::new();
    run(&store, &statement, |q, f, p| rows.push((q, f, p))).unwrap();
    assert_eq!(rows, [(0, 1, 2), (1, 1, 3)]);
    statement.queries[1] = "SELECT 1,fid,missing_column FROM late".into();
    statement.reschedule();
    rows.clear();
    assert!(
        run(&store, &statement, |q, f, p| rows.push((q, f, p))).is_err()
    );
    assert_eq!(rows, [(0, 1, 2)]);
    assert_eq!(temp_tables(&store, "name IN ('early','late','early_keys','late_keys')"), 0);
    statement.queries[1] = "SELECT 1,fid,pre FROM late".into();
    statement.reschedule();
    rows.clear();
    run(&store, &statement, |q, f, p| rows.push((q, f, p))).unwrap();
    assert_eq!(rows, [(0, 1, 2), (1, 1, 3)]);
}

#[test]
fn repeated_kind_restriction_is_idempotent_even_through_intermediate_sets() {
    let mut lower = compiler();
    let a = lower.relation("SELECT 1,1".into(), true).unwrap();
    let b = lower
        .relation("SELECT 1,1 UNION SELECT 2,2".into(), true)
        .unwrap();
    let c = lower
        .relation("SELECT 1,1 UNION SELECT 3,3".into(), true)
        .unwrap();
    let ab = lower.combine(&a, &b, "INTERSECT", true).unwrap();
    let abc = lower.combine(&ab, &c, "INTERSECT", true).unwrap();
    let before = lower.ctes.len();
    let again = lower.combine(&abc, &b, "INTERSECT", false).unwrap();
    assert_eq!(again.name, abc.name);
    assert!(
        !again.exact,
        "set reuse must preserve conservative exactness"
    );
    assert_eq!(
        lower.ctes.len(),
        before,
        "an implied restriction adds no sort or materialization"
    );
}

#[test]
fn difference_from_itself_is_empty_and_remains_sound_under_negation() {
    let mut lower = compiler();
    let a = lower
        .relation("SELECT 1,1 UNION SELECT 1,2".into(), true)
        .unwrap();
    let empty = lower.combine(&a, &a, "EXCEPT", true).unwrap();
    let all = lower.combine(&a, &empty, "EXCEPT", true).unwrap();
    let statement = finish(
        lower.ctes,
        vec![format!(
            "SELECT 0 AS qid,fid,pre FROM {} UNION ALL SELECT 1 AS qid,fid,pre FROM {} ORDER BY qid,fid,pre",
            empty.name, all.name
        )],
        lower.parameters,
        lower.indexed,
    );
    let store = store();
    let mut rows = Vec::new();
    run(&store, &statement, |q, f, p| rows.push((q, f, p))).unwrap();
    assert_eq!(rows, [(1, 1, 1), (1, 1, 2)]);
}

#[test]
fn identical_generated_relations_share_storage_without_changing_exactness() {
    let mut lower = compiler();
    let a = lower.relation("SELECT 1,1".into(), true).unwrap();
    let b = lower.relation("SELECT 1,1".into(), false).unwrap();
    assert_eq!(a.name, b.name);
    assert!(!b.exact);
    assert_eq!(lower.ctes.len(), 1);
}

#[test]
fn empty_union_still_deduplicates_a_projected_input() {
    let mut lower = compiler();
    let a = lower
        .relation("SELECT 1,1 UNION ALL SELECT 1,1".into(), true)
        .unwrap();
    let empty = lower.relation("SELECT 0,0 WHERE 0".into(), true).unwrap();
    let unique = lower.combine(&empty, &a, "UNION", true).unwrap();
    let statement = finish(
        lower.ctes,
        vec![format!("SELECT 0,fid,pre FROM {}", unique.name)],
        lower.parameters,
        lower.indexed,
    );
    let store = store();
    let mut rows = Vec::new();
    run(&store, &statement, |q, f, p| rows.push((q, f, p))).unwrap();
    assert_eq!(rows, [(0, 1, 1)]);
}

#[test]
fn shared_candidate_owner_cleans_up_after_a_query_error_and_retries_with_fresh_completion() {
    let store = store();
    for fail in [true, false, false] {
        let mut statement = finish(
            vec![
                "a(fid,pre) AS MATERIALIZED (VALUES(1,2))".into(),
                "b(fid,pre) AS MATERIALIZED (SELECT 1,3 WHERE 0)".into(),
            ],
            vec![
                "SELECT 0,fid,pre FROM a".into(),
                if fail {
                    "SELECT 1,fid,missing FROM b".into()
                } else {
                    "SELECT 1,fid,pre FROM b".into()
                },
            ],
            vec![],
            BTreeSet::from(["a".into(), "b".into()]),
        );
        statement.own();
        let mut rows = Vec::new();
        let result =
            run(&store, &statement, |q, f, p| rows.push((q, f, p)));
        assert_eq!(result.is_err(), fail);
        assert_eq!(rows, [(0, 1, 2)]);
        assert_eq!(temp_tables(&store, "name LIKE 'candidate_keys%'"), 0);
    }
}

#[test]
fn a_union_of_restrictions_is_one_compound_set_within_their_scope() {
    let mut lower = compiler();
    let scope = lower
        .relation("SELECT 1,1 UNION SELECT 1,2".into(), true)
        .unwrap();
    let a = lower.relation("SELECT 1,1".into(), true).unwrap();
    let b = lower.relation("SELECT 1,2".into(), true).unwrap();
    let a = lower.combine(&scope, &a, "INTERSECT", true).unwrap();
    let b = lower.combine(&scope, &b, "INTERSECT", true).unwrap();
    let empty = lower.relation("SELECT 0,0 WHERE 0".into(), true).unwrap();
    let before = lower.ctes.len();
    let union = lower.union(vec![a.clone(), empty, b.clone(), a]).unwrap();
    assert_eq!(
        lower.ctes.len(),
        before + 1,
        "one compound UNION, without the empty seed"
    );
    let again = lower.combine(&union, &scope, "INTERSECT", true).unwrap();
    assert_eq!(
        again.name, union.name,
        "a union of subsets is within their scope"
    );
    assert_eq!(lower.ctes.len(), before + 1);
    let other = lower.relation("SELECT 2,2".into(), true).unwrap();
    let narrowed = lower.combine(&union, &other, "INTERSECT", true).unwrap();
    assert_ne!(
        narrowed.name, union.name,
        "containment is not membership in every set"
    );
}

#[test]
fn a_union_with_a_superset_of_every_branch_is_that_superset() {
    let mut lower = compiler();
    let scope = lower
        .relation("SELECT 1,1 UNION SELECT 1,2".into(), true)
        .unwrap();
    let filtered = lower
        .relation(
            format!("SELECT fid,pre FROM {} WHERE pre=2", scope.name),
            true,
        )
        .unwrap();
    let filtered = lower.narrowed(filtered, &scope);
    let before = lower.ctes.len();
    let union = lower.union(vec![filtered.clone(), scope.clone()]).unwrap();
    assert_eq!(union.name, scope.name);
    assert_eq!(lower.ctes.len(), before, "no UNION is materialized");
    let unrelated = lower.relation("SELECT 2,2".into(), true).unwrap();
    let union = lower.union(vec![filtered, unrelated]).unwrap();
    assert_ne!(union.name, scope.name);
}

fn matcher(rule: &Json, lang: GrammarDialect) -> AstMatcher {
    let ids = std::collections::BTreeMap::new();
    let utils = crate::code_rule_dsl::compile::UtilScope {
        table: std::sync::Weak::new(),
        ids: &ids,
        options: None,
        python: None,
        transform: None,
        conflicts: None,
        captures: Default::default(),
        calls: None,
        documents: None,
        side: crate::code_rule_dsl::compile::documents::Side::Host,
    };
    crate::code_rule_dsl::compile::compile_ast_rule(rule, lang, "test/rule", &mut Vec::new(), &utils).unwrap()
}

/// The preorders the matcher accepts on `doc`, by a full walk.
fn matches(doc: &TapeDoc, matcher: &AstMatcher) -> Vec<u32> {
    let root = AstGrep::doc(doc.clone());
    root.root()
        .dfs()
        .enumerate()
        .filter(|(_, node)| matcher.match_node(node.clone()).is_some())
        .map(|(pre, _)| pre as u32)
        .collect()
}

/// Per rule, the candidates of each tape and its matches.
fn select_each(
    store: &ContentStore,
    lang: GrammarDialect,
    tapes: &[(crate::content_store::TapeId, TapeDoc)],
    rules: &[Json],
) -> Vec<Vec<(Vec<u32>, Vec<u32>)>> {
    let matchers: Vec<_> = rules.iter().map(|rule| matcher(rule, lang)).collect();
    let plans: Vec<_> = matchers.iter().map(AnchorPlan::compile).collect();
    let refs: Vec<_> = plans.iter().collect();
    let ids: Vec<_> = tapes.iter().map(|(tape, _)| *tape).collect();
    let tag = u32::from(language::tape_tag(lang).unwrap());
    let rows = super::super::select(store, tag, &refs, &ids).unwrap();
    rows.iter()
        .zip(&matchers)
        .map(|(rows, matcher)| {
            tapes
                .iter()
                .map(|(tape, doc)| {
                    let candidates = rows.iter().filter(|(t, _)| *t == tape.row()).map(|(_, pre)| *pre).collect();
                    (candidates, matches(doc, matcher))
                })
                .collect()
        })
        .collect()
}

#[test]
fn candidates_contain_every_match_and_kind_and_term_postings_are_exact() {
    let store = store();
    let lang = GrammarDialect::TypeScript;
    let tapes: Vec<_> = [
        "const a = foo(1); const b = bar(2); const c = foo(3);",
        "function f(x) { if (x) { return foo(x); } return 0; }\nlet s = 'foo';",
        "class K { m() { return this.n; } }",
        "const broken = foo(;",
    ]
    .iter()
    .map(|source| put(&store, source, lang))
    .collect();
    let inner = json!({"kind": "call_expression", "regex": "foo"});
    let rules = [
        json!({"kind": "call_expression"}),
        json!({"kind": "identifier", "regex": "^foo$"}),
        json!({"kind": "identifier", "regex": "o"}),
        json!({"pattern": "foo($A)"}),
        json!({"pattern": "return $X"}),
        json!({"kind": "call_expression", "not": {"regex": "foo"}}),
        json!({"kind": "call_expression", "not": {"pattern": "foo($A)"}}),
        json!({"any": [{"not": {"regex": "foo"}}, {"pattern": "foo($A)"}]}),
        json!({"any": [{"kind": "identifier"}, {"kind": "number"}]}),
        json!({"kind": "lexical_declaration", "has": {"kind": "call_expression"}}),
        json!({"kind": "lexical_declaration", "has": {"kind": "call_expression", "stopBy": "end"}}),
        json!({"kind": "call_expression", "inside": {"kind": "lexical_declaration"}}),
        json!({"kind": "return_statement", "inside": {"kind": "if_statement", "stopBy": "end"}}),
        json!({"kind": "call_expression", "precedes": {"kind": "call_expression"}}),
        json!({"kind": "call_expression", "follows": {"kind": "call_expression", "stopBy": "end"}}),
        json!({"all": [inner, {"has": {"regex": "^\\($"}}]}),
        json!({"kind": "ERROR"}),
        json!({"regex": "foo"}),
    ];
    let selected = select_each(&store, lang, &tapes, &rules);
    for (rule, per_tape) in rules.iter().zip(&selected) {
        for (number, (candidates, matched)) in per_tape.iter().enumerate() {
            for pre in matched {
                assert!(candidates.binary_search(pre).is_ok(), "{rule} misses match {pre} of tape {number}");
            }
        }
    }
    // Kind postings and covered terms are exact: the candidates are the matches.
    for exact in [0, 1, 8, 16] {
        for (candidates, matched) in &selected[exact] {
            assert_eq!(candidates, matched, "{}", rules[exact]);
        }
    }
    // A relation keeps its scope only in tapes where the inner plan can hold.
    let (candidates, _) = &selected[12][2];
    assert!(candidates.is_empty(), "no if_statement in the class tape");
    let (candidates, _) = &selected[3][2];
    assert!(candidates.is_empty(), "no foo call in the class tape");
}

#[test]
fn regex_vocabulary_fills_once_follows_new_terms_and_dies_with_its_terms() {
    let store = store();
    let lang = GrammarDialect::TypeScript;
    let rules = [json!({"kind": "identifier", "regex": "^tar"}), json!({"kind": "identifier", "regex": "^(?:alpha|beta)$"})];
    let count = |sql: &str| -> i64 { store.query_rows(sql, [], |r| r.get(0)).unwrap()[0] };
    // `^tar` is anchored: its terms are sought by prefix range, so the range edges must hold.
    let first = put(&store, "const target = alpha(beta); ta; tar; tarz; tas; taré; sz;", lang);
    let selected = select_each(&store, lang, std::slice::from_ref(&first), &rules);
    assert_eq!(selected[0][0].0, selected[0][0].1);
    assert_eq!(selected[1][0].0, selected[1][0].1);
    assert_eq!(count("SELECT count(*) FROM vocabulary_predicate WHERE ready=1"), 2);
    assert_eq!(count("SELECT count(*) FROM vocabulary_literal"), 2, "an exact literal set joins its literals");
    assert_eq!(count("SELECT count(*) FROM vocabulary_term WHERE expression='^tar'"), 4, "target, tar, tarz, taré");
    assert_eq!(count("SELECT count(*) FROM vocabulary_term"), 6);
    assert_eq!(count("SELECT count(*) FROM vocabulary_pending"), 0);
    // A new tape's new terms are pending until the next statement matches them.
    let second = put(&store, "let tarp = gamma;", lang);
    assert_eq!(count("SELECT count(*) FROM vocabulary_pending WHERE term IN ('tarp','gamma','let')"), 3);
    let tapes = [first, second];
    let selected = select_each(&store, lang, &tapes, &rules);
    for per_tape in &selected {
        for (candidates, matched) in per_tape {
            assert_eq!(candidates, matched);
        }
    }
    assert_eq!(count("SELECT count(*) FROM vocabulary_term WHERE term='tarp'"), 1);
    // A term that leaves the store leaves every predicate's set.
    store.remove_prefilter(tapes[1].0).unwrap();
    assert_eq!(count("SELECT count(*) FROM vocabulary_term WHERE term='tarp'"), 0);
    assert_eq!(count("SELECT count(*) FROM vocabulary_term"), 6);
}
