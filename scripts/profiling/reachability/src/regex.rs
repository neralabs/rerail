//! Use the engine's regex crate and SQL semantics during captured-input replay.
use crate::graph::failure;
use rusqlite::{Connection, Result, functions::FunctionFlags, types::Value};
use std::collections::HashMap;
use std::sync::Mutex;

pub fn register(db: &Connection) -> Result<()> {
    for (name, arity) in [("rx_matches", 2), ("rx_extract", 3), ("rx_replace", 3)] {
        let cache = Mutex::new(HashMap::<String, ::regex::Regex>::new());
        let flags = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
        db.create_scalar_function(name, arity, flags, move |ctx| {
            let text = ctx.get::<Option<String>>(0)?.unwrap_or_default();
            let pattern = ctx.get::<String>(1)?;
            let mut cache = cache.lock().map_err(|_| failure("regex cache poisoned"))?;
            if !cache.contains_key(&pattern) {
                let compiled =
                    ::regex::Regex::new(&pattern).map_err(|e| failure(&e.to_string()))?;
                if cache.len() >= 64 {
                    cache.clear();
                }
                cache.insert(pattern.clone(), compiled);
            }
            let regex = &cache[&pattern];
            match name {
                "rx_matches" => Ok(Value::Integer(i64::from(regex.is_match(&text)))),
                "rx_extract" => {
                    let group = usize::try_from(ctx.get::<i64>(2)?)
                        .map_err(|_| failure("regex group must be nonnegative"))?;
                    if group >= regex.captures_len() {
                        return Err(failure("unknown regex group"));
                    }
                    Ok(regex
                        .captures(&text)
                        .and_then(|matched| {
                            matched
                                .get(group)
                                .map(|part| Value::Text(part.as_str().to_owned()))
                        })
                        .unwrap_or(Value::Null))
                }
                _ => Ok(Value::Text(
                    regex
                        .replace_all(&text, ctx.get::<String>(2)?.as_str())
                        .into_owned(),
                )),
            }
        })?;
    }
    Ok(())
}
