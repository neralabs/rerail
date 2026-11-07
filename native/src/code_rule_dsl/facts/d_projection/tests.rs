use super::*;

#[test]
fn small_projection_batches_retain_space_for_live_rows_only() {
    let scope = Scope::enter();
    let source = Arc::new("évidence".to_owned());
    let rows = (0..6)
        .map(|row| {
            append(
                (71, 72, 73),
                || (0..32).map(|column| format!("field_{column}")).collect(),
                (0..32).map(|column| match column {
                    0 => Input::Source(&source, 0..source.len()),
                    1 => Input::Null,
                    _ => Input::Integer(row * 32 + column),
                }),
            )
        })
        .collect::<Vec<_>>();
    drop(scope);
    assert_eq!(rows[0].identity(), rows[5].identity());
    assert_eq!(rows[5].sql(0), ValueRef::Text(source.as_bytes()));
    assert_eq!(rows[5].sql(1), ValueRef::Null);
    assert_eq!(rows[5].sql(31), ValueRef::Integer(191));
    let (retained, live_rows, columns, _) = rows[0].memory();
    assert_eq!((live_rows, columns), (6, 32));
    assert!(
        retained < 8192,
        "six rows retain {retained} bytes after publication"
    );
}
use serde_json::json;

#[test]
fn chunks_share_schema_and_borrow_unicode_source_without_row_maps() {
    let source = Arc::new("before αβ after".to_owned());
    let mut rows = Vec::new();
    {
        let _scope = Scope::enter();
        for i in 0..600 {
            rows.push(append(
                (1, 2, 3),
                || vec!["missing".into(), "offset".into(), "text".into()],
                [
                    Input::Null,
                    Input::Integer(i),
                    Input::Source(&source, 7..11),
                ]
                .into_iter(),
            ));
        }
    }
    assert_eq!(rows[0].schema_identity(), rows[599].schema_identity());
    assert_eq!(rows[0].identity(), rows[255].identity());
    assert_ne!(rows[255].identity(), rows[256].identity());
    assert_eq!(
        rows[599].json(),
        json!({"missing":null,"offset":599,"text":"αβ"})
    );
    let ValueRef::Text(text) = rows[599].sql(2) else {
        panic!("text");
    };
    assert_eq!(text.as_ptr(), source.as_bytes()[7..].as_ptr());
    assert_eq!(rows[599].memory().1, 88);
}

#[test]
fn nested_scopes_publish_independently_and_restore_the_parent() {
    let outer = Scope::enter();
    let first = append(
        (1, 2, 3),
        || vec!["outer".into()],
        [Input::Integer(1)].into_iter(),
    );
    let inner = {
        let _inner = Scope::enter();
        append(
            (1, 2, 3),
            || vec!["inner".into()],
            [Input::Integer(2)].into_iter(),
        )
    };
    assert_eq!(inner.json(), json!({"inner":2}));
    let second = append(
        (1, 2, 3),
        || vec!["outer".into()],
        [Input::Integer(3)].into_iter(),
    );
    drop(outer);
    assert_eq!(first.identity(), second.identity());
    assert_eq!(first.json(), json!({"outer":1}));
    assert_eq!(second.json(), json!({"outer":3}));
}

#[test]
fn wire_round_trips_keep_json_type_and_missing_distinct_from_null() {
    for value in [
        json!({}),
        json!(null),
        json!(true),
        json!(["a"]),
        json!({"a":true,"b":1.0,"c":[1],"d":"[1]"}),
    ] {
        let row = Row::from_json(value.clone());
        assert_eq!(row.json(), value);
        assert_eq!(serde_json::to_value(&row).unwrap(), value);
    }
    assert_ne!(
        Row::from_json(json!({"v":true})),
        Row::from_json(json!({"v":1}))
    );
    assert_ne!(
        Row::from_json(json!({"v":[1]})),
        Row::from_json(json!({"v":"[1]"}))
    );
    assert_ne!(Row::from_json(json!({"v":null})), Row::from_json(json!({})));
}

#[test]
fn nulls_duplicates_and_subspans_survive_publication_and_sql_binding() {
    let source = Arc::new("prefix NAME suffix".to_owned());
    let row = append(
        (1, 2, 3),
        || vec!["name".into(), "empty".into()],
        [
            Input::Source(&source, 0..source.len()).slice(7..11),
            Input::Null,
        ]
        .into_iter(),
    );
    assert_eq!(row.sql(0), ValueRef::Text(b"NAME"));
    assert_eq!(row.sql(1), ValueRef::Null);
    assert_eq!(row, row.clone());
}
