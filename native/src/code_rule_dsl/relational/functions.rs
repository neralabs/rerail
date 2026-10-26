//! Production SQL regex functions, shared by live sessions and offline failure replay.
use rusqlite::{Connection, Result, functions::FunctionFlags};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

pub(super) fn install(db: &Connection, max_pattern: usize, work: Arc<AtomicUsize>) -> Result<()> {
    let flags = FunctionFlags::SQLITE_UTF8
        | FunctionFlags::SQLITE_DETERMINISTIC
        | FunctionFlags::SQLITE_INNOCUOUS;
    for (name, arity) in [("rx_matches", 2), ("rx_extract", 3), ("rx_replace", 3)] {
        let cache = parking_lot::Mutex::new(HashMap::<String, regex::Regex>::new());
        let work = Arc::clone(&work);
        db.create_scalar_function(name, arity, flags, move |ctx| {
            let text = ctx.get::<Option<String>>(0)?.unwrap_or_default();
            let pattern = ctx.get::<String>(1)?;
            let fail =
                |message: &str| rusqlite::Error::UserFunctionError(message.to_owned().into());
            let cost = text.len().div_ceil(256).max(1);
            if pattern.len() > max_pattern
                || work
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                        left.checked_sub(cost)
                    })
                    .is_err()
            {
                return Err(fail("regular expression work or pattern budget exceeded"));
            }
            let mut cache = cache.lock();
            if !cache.contains_key(&pattern) {
                let compiled =
                    regex::Regex::new(&pattern).map_err(|error| fail(&error.to_string()))?;
                if cache.len() >= 64 {
                    cache.clear();
                }
                cache.insert(pattern.clone(), compiled);
            }
            let regex = &cache[&pattern];
            match name {
                "rx_matches" => Ok(rusqlite::types::Value::Integer(i64::from(
                    regex.is_match(&text),
                ))),
                "rx_extract" => {
                    let group = usize::try_from(ctx.get::<i64>(2)?)
                        .map_err(|_| fail("regex group must be nonnegative"))?;
                    if group >= regex.captures_len() {
                        return Err(fail("unknown regex group"));
                    }
                    Ok(regex
                        .captures(&text)
                        .and_then(|matched| {
                            matched
                                .get(group)
                                .map(|part| rusqlite::types::Value::Text(part.as_str().to_owned()))
                        })
                        .unwrap_or(rusqlite::types::Value::Null))
                }
                _ => {
                    let by: String = ctx.get(2)?;
                    if by.len() > max_pattern {
                        return Err(fail("replacement exceeds pattern budget"));
                    }
                    Ok(rusqlite::types::Value::Text(
                        regex.replace_all(&text, by.as_str()).into_owned(),
                    ))
                }
            }
        })?;
    }
    Ok(())
}
