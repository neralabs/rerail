//! core.rr's functors whose meaning is W's or a language's (`@replace_all`, `@nfkc`, `@py_repr`, …), registered on each
//! connection with the rule language's own text functions (`logic::strings::install_functors`).

use rusqlite::Connection;
use std::collections::HashMap;

/// core.rr's functors whose meaning is W's, which `strings::functions` does not give: `@replace_all(s, re, by)`, each
/// match of `re` in `s` replaced by `by`, `$1`
/// and `${name}` its groups, as W's `replace` transform does (`matcher::transform::replace`); `@rx_replace(s, re, by)`
/// the same as W's SQL `rx_replace` does it (`waves::relational::functions`: the `regex` crate's `replace_all`, `$1`,
/// `$name`, `${name}` and `$$` as it reads them); `@ed25519_ok(key, message, sig)`, 1 where `crypto::ed25519_ok`
/// holds, else 0; `@nfkc(s)`, `s` under Unicode NFKC, as CPython normalizes an identifier (PEP 3131: the parser's
/// `unicodedata.normalize("NFKC", …)` of each non-ASCII one; `identifier::nfkc`); `@py_repr(s)`, CPython's `repr(s)` of
/// the str `s` (`cpython::value::str_repr`: W's `repr_unparse` is `@py_repr(@py_unparse(n))`). NULL gives NULL
/// (`rr_ed25519_ok`: 0). `rr_join(sep, x, k, …)` is `join`'s aggregate: the distinct
/// `(k, …, x)` rows' `x` ordered by `k, …` then `x` (numbers by value, then text by code point, as `sorted()` orders a
/// list of one type), joined by `sep`; `""` over no rows.
pub(crate) fn register_functors(db: &Connection) -> rusqlite::Result<()> {
    use rusqlite::functions::FunctionFlags;
    let flags = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
    fn compiled(f: &str, re: &str) -> rusqlite::Result<regex::Regex> {
        static REGEXES: std::sync::LazyLock<std::sync::Mutex<HashMap<String, Option<regex::Regex>>>> = std::sync::LazyLock::new(Default::default);
        let regex = REGEXES.lock().expect("the regex cache").entry(re.to_owned()).or_insert_with(|| regex::Regex::new(re).ok()).clone();
        regex.ok_or_else(|| rusqlite::Error::UserFunctionError(format!("@{f}: bad regex {re:?}").into()))
    }
    let texts = |ctx: &rusqlite::functions::Context<'_>| -> rusqlite::Result<Option<[String; 3]>> {
        Ok(match (ctx.get::<Option<String>>(0)?, ctx.get::<Option<String>>(1)?, ctx.get::<Option<String>>(2)?) {
            (Some(a), Some(b), Some(c)) => Some([a, b, c]),
            _ => None,
        })
    };
    db.create_scalar_function(c"rr_replace_all", 3, flags, move |ctx| {
        let Some([s, re, by]) = texts(ctx)? else { return Ok(None) };
        Ok(Some(crate::code_rule_dsl::matcher::transform::replace(&s, &compiled("replace_all", &re)?, &by)))
    })?;
    db.create_scalar_function(c"rr_rx_replace", 3, flags, move |ctx| {
        let Some([s, re, by]) = texts(ctx)? else { return Ok(None) };
        Ok(Some(compiled("rx_replace", &re)?.replace_all(&s, by.as_str()).into_owned()))
    })?;
    db.create_scalar_function(c"rr_ed25519_ok", 3, flags, move |ctx| {
        Ok(texts(ctx)?.is_some_and(|[key, message, sig]| crate::code_rule_dsl::logic::crypto::ed25519_ok(&key, &message, &sig)))
    })?;
    db.create_scalar_function(c"rr_nfkc", 1, flags, |ctx| Ok(ctx.get::<Option<String>>(0)?.map(|s| crate::code_rule_dsl::identifier::nfkc(&s).into_owned())))?;
    db.create_scalar_function(c"rr_py_repr", 1, flags, |ctx| {
        Ok(ctx.get::<Option<String>>(0)?.map(|s| crate::code_rule_dsl::cpython::value::str_repr(&crate::code_rule_dsl::cpython::value::PyStr::Text(s))))
    })?;
    db.create_aggregate_function(c"rr_join", -1, flags, OrderedJoin)
}

/// `rr_join`'s rows: each one's order key (its `k, …`, then `x`) and `x`'s text.
struct OrderedJoin;
impl rusqlite::functions::Aggregate<(String, Vec<(Vec<rusqlite::types::Value>, String)>), String> for OrderedJoin {
    fn init(&self, ctx: &mut rusqlite::functions::Context<'_>) -> rusqlite::Result<(String, Vec<(Vec<rusqlite::types::Value>, String)>)> {
        Ok((ctx.get::<Option<String>>(0)?.unwrap_or_default(), Vec::new()))
    }
    fn step(&self, ctx: &mut rusqlite::functions::Context<'_>, acc: &mut (String, Vec<(Vec<rusqlite::types::Value>, String)>)) -> rusqlite::Result<()> {
        use rusqlite::types::Value;
        let x: Value = ctx.get(1)?;
        let text = match &x {
            Value::Text(s) => s.clone(),
            Value::Integer(i) => i.to_string(),
            Value::Real(r) => r.to_string(),
            Value::Null => return Ok(()),
            Value::Blob(_) => return Err(rusqlite::Error::UserFunctionError("join: a blob".into())),
        };
        let mut key: Vec<Value> = (2..ctx.len()).map(|i| ctx.get(i)).collect::<rusqlite::Result<_>>()?;
        key.push(x);
        acc.1.push((key, text));
        Ok(())
    }
    fn finalize(&self, _: &mut rusqlite::functions::Context<'_>, acc: Option<(String, Vec<(Vec<rusqlite::types::Value>, String)>)>) -> rusqlite::Result<String> {
        use rusqlite::types::Value;
        use std::cmp::Ordering;
        let Some((sep, mut rows)) = acc else { return Ok(String::new()) };
        // Numbers by value before text by code point (UTF-8 byte order is code point order).
        fn value(a: &Value, b: &Value) -> Ordering {
            let number = |v: &Value| match v {
                Value::Integer(i) => Some(*i as f64),
                Value::Real(r) => Some(*r),
                _ => None,
            };
            match (a, b) {
                (Value::Integer(x), Value::Integer(y)) => x.cmp(y),
                (Value::Text(x), Value::Text(y)) => x.cmp(y),
                _ => match (number(a), number(b)) {
                    (Some(x), Some(y)) => x.total_cmp(&y),
                    (Some(_), None) => Ordering::Less,
                    (None, Some(_)) => Ordering::Greater,
                    (None, None) => Ordering::Equal,
                },
            }
        }
        let key = |a: &[Value], b: &[Value]| a.iter().zip(b).map(|(x, y)| value(x, y)).find(|o| o.is_ne()).unwrap_or(Ordering::Equal);
        rows.sort_by(|a, b| key(&a.0, &b.0));
        rows.dedup_by(|a, b| key(&a.0, &b.0).is_eq());
        Ok(rows.iter().map(|(_, x)| x.as_str()).collect::<Vec<_>>().join(&sep))
    }
}