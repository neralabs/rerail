use super::*;
use crate::code_rule_dsl::{
    CodeRuleDslRuntime,
    model::ClauseKind,
    violations::{
        Captures, Span,
        projection::{self, Input},
    },
};
use serde_json::json;
use std::sync::atomic::Ordering;

fn rule() -> crate::code_rule_dsl::CompiledCodeRuleDsl {
    CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"stream","revision":1,"mode":"log","clauses":[{"id":"rows","relational":{"inputs":{"facts":{"evidence":{"as":"f","query":{"ast":{"language":"python","rule":{"kind":"identifier","pattern":"$NAME"}}},"project":{"name":"capture.NAME","pos":"capture.NAME.startByte"}}}},"query":"SELECT __path path,__start start_byte,__end end_byte,name,pos FROM facts"},"violation":{"code":"STREAM","message":"Stream","anchor":"witness.span"}}]}).to_string(),"json".into(),include_str!("../../capabilities.json").into()).unwrap()
}
fn fill(sink: &mut Sink<'_>, count: usize, source: &Arc<String>) {
    let _scope = projection::Scope::enter();
    let names: Arc<[String]> = sink.input.project.iter().map(|p| p.field.clone()).collect();
    for row in 0..count {
        let values = names.iter().map(|name| match name.as_str() {
            "name" if row % 2 == 0 => Input::Source(source, 0..source.len()),
            "name" => Input::Null,
            "pos" => Input::Integer(42),
            _ => unreachable!(),
        });
        let projection = projection::append((1, 1, 1), || names.to_vec(), values);
        let mut captures = Captures::default();
        captures.set_typed_projection(projection);
        let witness = sink.emit(Witness {
            path: "a.py".into(),
            span: Span {
                start_byte: 0,
                end_byte: 2,
                start_line: 1,
                end_line: 1,
                start_column: 0,
                end_column: 1,
            },
            captures,
            message: None,
            fix: None,
            ignored: false,
        });
        assert!(witness.captures.projection().is_none());
    }
}
#[test]
fn columns_stream_in_bounded_batches_preserving_unicode_nulls_and_duplicates() {
    let rule = rule();
    let ClauseKind::Relational(plan) = &rule.inner.clauses[0].kind else {
        panic!()
    };
    let input = &plan.inputs[0].1;
    let db = Connection::open_in_memory().unwrap();
    super::super::create_input(
        &db,
        "facts",
        &input
            .project
            .iter()
            .map(|p| p.field.clone())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let work = Arc::new(AtomicUsize::new(1000));
    let mut sink = Sink {
        db: &db,
        table: "facts",
        input,
        work: &work,
        pending: Vec::new(),
        path: Arc::from(""),
        error: None,
        rows: 0,
        peak: 0,
        batches: 0,
    };
    let source = Arc::new("é猫".to_string());
    let weak = Arc::downgrade(&source);
    fill(&mut sink, 600, &source);
    sink.flush().unwrap();
    assert!(sink.pending.is_empty());
    assert!(sink.error.is_none());
    assert_eq!((sink.peak, sink.batches), (256, 3));
    assert_eq!(work.load(Ordering::Relaxed), 400);
    drop(source);
    assert!(
        weak.upgrade().is_none(),
        "SQLite import must release shared source columns"
    );
    let counts: (i64, i64, i64, i64) = db
        .query_row(
            "SELECT count(*),sum(name='é猫'),sum(name IS NULL),sum(pos=42) FROM facts",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(counts, (600, 300, 300, 600));
}
#[test]
fn failed_stream_rolls_back_previously_imported_batches() {
    let rule = rule();
    let ClauseKind::Relational(plan) = &rule.inner.clauses[0].kind else {
        panic!()
    };
    let input = &plan.inputs[0].1;
    let db = Connection::open_in_memory().unwrap();
    super::super::create_input(
        &db,
        "facts",
        &input
            .project
            .iter()
            .map(|p| p.field.clone())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    db.execute_batch("BEGIN").unwrap();
    let work = Arc::new(AtomicUsize::new(300));
    let mut sink = Sink {
        db: &db,
        table: "facts",
        input,
        work: &work,
        pending: Vec::new(),
        path: Arc::from(""),
        error: None,
        rows: 0,
        peak: 0,
        batches: 0,
    };
    fill(&mut sink, 600, &Arc::new("é猫".into()));
    assert!(sink.error.take().is_some());
    let count: i64 = db
        .query_row("SELECT count(*) FROM facts", [], |r| r.get(0))
        .unwrap();
    assert!(
        count >= 256,
        "the failure follows at least one complete import batch"
    );
    drop(sink);
    db.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM facts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}
