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

/// An export named like a base relation is held with positional columns (`c0`, `c1`) and read by the base names
/// (`value(n, v)`), as its virtual table names them (`core::input_column_name`). Since 0174 answers `value` on
/// request, it reaches the cross-file part as an export, and `view` looked for `n` among `c0, c1`: the cross part read
/// it with `SELECT n, v FROM main.value`, which strict mode refuses inside the waves (0104), so a cold run aborted
/// and the next run had nothing kept to be warm over.
#[test]
fn an_export_named_like_a_base_relation_is_read_in_place_by_the_base_names() {
    let db = Connection::open_in_memory().unwrap();
    let modules = crate::code_rule_dsl::facts::vtab::Modules::default();
    let source = crate::code_rule_dsl::logic::native::component::columns_source(vec!["c0".into(), "c1".into()], vec![Some(vec![3, 9]), Some(vec![11, 12])]);
    modules.bind(&db, "value", source).unwrap();
    let all = view(&db, "main.value", "n, v").expect("read in place");
    assert_eq!((all.src.len(), all.cols.as_slice()), (2, &[0usize, 1][..]));
    let some = view(&db, "main.value WHERE n = 9", "v").expect("read in place, filtered");
    assert_eq!(some.ids.as_deref(), Some(&[1u32][..]));
}
