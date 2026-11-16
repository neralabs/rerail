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
