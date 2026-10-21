use super::*;
use crate::code_rule_dsl::sql::tests::differential::tape;
use crate::code_rule_dsl::test_support::{compiled, grammar_matchers};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use serde_json::json;

#[test]
fn selected_empty_and_all_roots_preserve_captures_and_session_reuse_after_failure() {
    let rule = compiled(json!({"rule":{"pattern":"$F($A)"}}));
    let grammar = &grammar_matchers(&rule)[0];
    let program = compile(&[grammar]).unwrap();
    let mut session = program.session().unwrap();
    let docs = [
        tape(GrammarDialect::TypeScript, "f(a); g(b); h(c);"),
        tape(GrammarDialect::TypeScript, "i(d); j(e);"),
    ];
    let input = || {
        docs.iter()
            .map(|doc| (doc.clone(), FileContext::default()))
            .collect()
    };
    let all = session
        .run(
            input(),
            &[ClauseRun {
                statement: 0,
                files: vec![(0, None), (1, None)],
            }],
        )
        .unwrap();
    assert_eq!(all[0][0].len(), 3);
    assert_eq!(all[0][1].len(), 2);
    let selected: Arc<[u32]> = Arc::from([all[0][0][2].root, all[0][0][0].root, all[0][0][2].root]);
    let runs = [ClauseRun {
        statement: 0,
        files: vec![(0, Some(selected)), (1, Some(Arc::from([])))],
    }];
    let found = session.run(input(), &runs).unwrap();
    assert_eq!(found[0][0], [all[0][0][0].clone(), all[0][0][2].clone()]);
    assert!(found[0][1].is_empty());
    assert_eq!(
        session
            .connection
            .query_row("SELECT count(*) FROM roots", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    // Force a rule read to fail after input publication. Cleanup must release the inputs, and
    // the same cached session must then handle a new batch without old roots or stale counts.
    session
        .connection
        .authorizer(Some(|context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Read {
                    table_name: "node",
                    ..
                }
            ) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }))
        .unwrap();
    assert!(session.run(input(), &runs).is_err());
    session
        .connection
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .unwrap();
    assert_eq!(
        session
            .connection
            .query_row("SELECT count(*) FROM roots", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    let rerun = session
        .run(
            input(),
            &[ClauseRun {
                statement: 0,
                files: vec![(0, None), (1, None)],
            }],
        )
        .unwrap();
    assert_eq!(rerun, all);
}
