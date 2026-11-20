//! A bound input is read in place by the name its statement writes (O-pse-alone-strict).
use super::*;

/// A relation named like a SQLite keyword is quoted in every statement (`sql::ident`, O106): `plan` is read as
/// `main."plan"`. `view` found no bound input of that name and read it through SQL, which strict mode refuses inside
/// the waves (0104): the data-layer's plan-scope rules alone stopped at `SELECT c0 FROM main."plan"`.
#[test]
fn a_bound_input_named_like_a_sql_keyword_is_read_in_place() {
    let db = Connection::open_in_memory().unwrap();
    let modules = crate::code_rule_dsl::facts::vtab::Modules::default();
    let source = crate::code_rule_dsl::logic::native::component::columns_source(vec!["c0".into()], vec![Some(vec![4, 7])]);
    modules.bind(&db, "plan", source).unwrap();
    let table = format!("main.{}", crate::code_rule_dsl::logic::sql::ident("plan"));
    assert_eq!(table, "main.\"plan\"");
    let all = view(&db, &table, "c0").expect("read in place");
    assert_eq!(all.src.len(), 2);
    let some = view(&db, &format!("{table} WHERE c0 = 7"), "c0").expect("read in place, filtered");
    assert_eq!(some.ids.as_deref(), Some(&[1u32][..]));
}

/// `unquote` reads back what `ident` wrote, for a keyword, a name that is no plain identifier, and a plain one.
#[test]
fn unquote_reads_back_an_identifier() {
    use crate::code_rule_dsl::logic::sql::{ident, unquote};
    for name in ["plan", "values", "a\"b", "has space", "p47_plan"] {
        assert_eq!(unquote(&ident(name)), name, "{}", ident(name));
    }
}

/// A relation the file-local part derives and hands on under a base input's name is bound as an export, its columns
/// positional (`c0`, `c1`), while statements name its columns as the input's (`plan::sql_col`): `value` (decision
/// 0292 §3: `@rr_node_value` read in the chunk) was read as `SELECT n, v FROM main.value`, and `view` found no column
/// `n` in the bound export and read it through SQL, which strict mode refuses inside the waves (0104).
#[test]
fn an_export_named_like_an_input_is_read_in_place_by_the_inputs_column_names() {
    let db = Connection::open_in_memory().unwrap();
    let modules = crate::code_rule_dsl::facts::vtab::Modules::default();
    let source = crate::code_rule_dsl::logic::native::component::columns_source(vec!["c0".into(), "c1".into()], vec![Some(vec![4, 7]), Some(vec![40, 70])]);
    modules.bind(&db, "value", source).unwrap();
    let (n, v) = (crate::code_rule_dsl::logic::plan::sql_col("value", 0), crate::code_rule_dsl::logic::plan::sql_col("value", 1));
    assert_eq!((n.as_str(), v.as_str()), ("n", "v"), "the statement names `value`'s columns as the input's");
    let all = view(&db, "main.value", &format!("{n}, {v}")).expect("read in place");
    assert_eq!(all.cols, [0, 1]);
    let some = view(&db, &format!("main.value WHERE {n} = 7"), &v).expect("read in place, filtered");
    assert_eq!((some.cols.as_slice(), some.ids.as_deref()), (&[1usize][..], Some(&[1u32][..])));
}

/// Its table is named by the base names when first bound (`vtab::named`), so a later binding of the same shape is
/// compared by them too: the kept inputs' `value` (`c0`, `c1`) bound again over the chunks' failed "native binding
/// schema changed" on an edit. A source of another shape is still refused.
#[test]
fn an_export_named_like_an_input_is_bound_again_with_the_same_columns() {
    let db = Connection::open_in_memory().unwrap();
    let modules = crate::code_rule_dsl::facts::vtab::Modules::default();
    let source = |rows: Vec<i64>| crate::code_rule_dsl::logic::native::component::columns_source(vec!["c0".into(), "c1".into()], vec![Some(rows.clone()), Some(rows)]);
    modules.bind(&db, "value", source(vec![4])).unwrap();
    modules.bind(&db, "value", source(vec![4, 7])).expect("bound again, the same columns");
    assert_eq!(crate::code_rule_dsl::facts::vtab::bound_source(&db, "value").unwrap().len(), 2);
    let other = crate::code_rule_dsl::logic::native::component::columns_source(vec!["c0".into()], vec![Some(vec![1])]);
    assert!(modules.bind(&db, "value", other).is_err(), "another shape is refused");
}
