//! Lossless integer transport and grammar-domain decoding for SQL extensions.
use super::*;
use rusqlite::functions::FunctionFlags;

const MAX_SAFE: i64 = (1_i64 << 53) - 1;

pub(super) fn integer(n: i64) -> Value {
    if (-MAX_SAFE..=MAX_SAFE).contains(&n) {
        json!(n)
    } else {
        json!({"integer":n.to_string()})
    }
}

pub(super) fn exact(value: &Value) -> Result<i64> {
    if let Some(n) = value.as_i64() {
        return Ok(n);
    }
    if let Some(object) = value.as_object()
        && object.len() == 1
        && let Some(s) = object.get("integer").and_then(Value::as_str)
    {
        return s
            .parse()
            .map_err(|_| error("tagged integer exceeds the signed 64-bit range"));
    }
    Err(error("expected an integer or {integer: decimal-string}"))
}

pub(super) fn fact(symbols: &BTreeMap<&str, u32>, value: &Value, ty: Option<&str>) -> Result<i64> {
    if let Some(s) = value.as_str() {
        if ty.is_some_and(|t| t != "symbol") {
            return Err(error("string fact values require a symbol column"));
        }
        return Ok(i64::from(symbols[s]));
    }
    exact(value)
}

pub(super) fn register(db: &rusqlite::Connection) -> Result<()> {
    let flags = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
    for (function, field) in [(c"kind_name", false), (c"field_name", true)] {
        db.create_scalar_function(function, 2, flags, move |ctx| {
            let label: String = ctx.get(0)?;
            let id: i64 = ctx.get(1)?;
            let dialect = super::super::language::dialect_by_label(&label).ok_or_else(|| {
                rusqlite::Error::UserFunctionError(format!("unknown dialect: {label}").into())
            })?;
            let tag = super::super::language::tape_tag(dialect).ok_or_else(|| {
                rusqlite::Error::UserFunctionError("dialect has no fact encoding".into())
            })?;
            let catalog = dialect.kinds();
            let base = if field {
                i64::from(tag) << 16
            } else {
                i64::from(tag) * i64::from(super::super::facts::core::KIND_STRIDE)
            };
            let code = id
                .checked_sub(base)
                .and_then(|code| u16::try_from(code).ok());
            let result = code.and_then(|code| {
                if field {
                    catalog.field_name(code)
                } else {
                    catalog.kind_name(code)
                }
            });
            Ok(result.map(str::to_owned))
        })
        .map_err(error)?;
    }
    Ok(())
}
