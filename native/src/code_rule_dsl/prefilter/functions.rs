//! Bounded scalar decoders. SQL owns the index, joins, filtering, and traversal.
use rusqlite::{
    Connection, Result,
    functions::{Context, FunctionFlags},
};

fn invalid(message: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::UserFunctionError(message.into().into())
}
fn number(ctx: &Context<'_>, at: usize) -> Result<usize> {
    usize::try_from(ctx.get::<i64>(at)?).map_err(|_| invalid("negative tape ordinal"))
}
pub(super) fn install(db: &Connection) -> Result<super::reader::Readers> {
    use super::reader::{Column, Readers};
    let readers = Readers::new(db);
    super::nodes::install(db, readers.0.clone())?;
    super::tree::install(db, readers.0.clone())?;
    super::postings::install(db)?;
    rusqlite::vtab::series::load_module(db)?;
    let flags = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_INNOCUOUS;
    #[cfg(feature = "code-rule-profile")]
    let regex_readers = readers.0.clone();
    db.create_scalar_function(
        c"nera_term_matches",
        2,
        flags | FunctionFlags::SQLITE_DETERMINISTIC,
        move |ctx| {
            let regex = ctx.get_or_create_aux(0, |value| {
                #[cfg(feature = "code-rule-profile")]
                {
                    regex_readers.lock().counters.regex_compiles += 1;
                }
                regex::Regex::new(value.as_str()?).map_err(
                    |error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) },
                )
            })?;
            Ok(regex.is_match(ctx.get_raw(1).as_str()?))
        },
    )?;
    // Vocabulary maintenance supplies the pattern from a column. SQLite discards auxdata
    // for that varying argument on every row. A constant third argument owns one bounded
    // pattern cache instead; the SQL visits each predicate's complete term batch together.
    #[cfg(feature = "code-rule-profile")]
    let regex_readers = readers.0.clone();
    db.create_scalar_function(
        c"nera_term_matches",
        3,
        flags | FunctionFlags::SQLITE_DETERMINISTIC,
        move |ctx| {
            type Cache = parking_lot::Mutex<Option<(String, regex::Regex)>>;
            let cache = ctx.get_or_create_aux(2, |_| {
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>(Cache::new(None))
            })?;
            let mut cache = cache.lock();
            let expression = ctx.get_raw(0).as_str()?;
            if cache
                .as_ref()
                .is_none_or(|(pattern, _)| pattern != expression)
            {
                #[cfg(feature = "code-rule-profile")]
                {
                    regex_readers.lock().counters.regex_compiles += 1;
                }
                let regex = regex::Regex::new(expression).map_err(|e| invalid(e.to_string()))?;
                *cache = Some((expression.to_owned(), regex));
            }
            Ok(cache
                .as_ref()
                .expect("compiled pattern")
                .1
                .is_match(ctx.get_raw(1).as_str()?))
        },
    )?;
    // One row per term instead of one per pattern and term: the patterns of a set are compiled
    // once and tried in turn (a combined RegexSet was slower, regex.md §2.2).
    #[cfg(feature = "code-rule-profile")]
    let regex_readers = readers.0.clone();
    let sets = parking_lot::Mutex::new(Vec::<(String, Vec<(String, regex::Regex)>)>::new());
    db.create_scalar_function(
        c"nera_terms_matching",
        2,
        flags | FunctionFlags::SQLITE_DETERMINISTIC,
        move |ctx| {
            let set = ctx.get_raw(0).as_str()?;
            let term = ctx.get_raw(1).as_str()?;
            let mut sets = sets.lock();
            let at = match sets.iter().position(|(key, _)| key == set) {
                Some(at) => at,
                None => {
                    let expressions: Vec<String> =
                        serde_json::from_str(set).map_err(|e| invalid(e.to_string()))?;
                    let compiled = expressions
                        .into_iter()
                        .map(|expression| {
                            #[cfg(feature = "code-rule-profile")]
                            {
                                regex_readers.lock().counters.regex_compiles += 1;
                            }
                            let regex = regex::Regex::new(&expression)
                                .map_err(|e| invalid(e.to_string()))?;
                            Ok((expression, regex))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    if sets.len() == 4 {
                        sets.remove(0);
                    }
                    sets.push((set.to_owned(), compiled));
                    sets.len() - 1
                }
            };
            let matched: Vec<&str> = sets[at]
                .1
                .iter()
                .filter(|(_, regex)| regex.is_match(term))
                .map(|(expression, _)| expression.as_str())
                .collect();
            Ok((!matched.is_empty())
                .then(|| serde_json::to_string(&matched).expect("expression JSON")))
        },
    )?;
    db.create_scalar_function(
        c"nera_literal_set",
        1,
        flags | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            Ok(literal_set(ctx.get_raw(0).as_str()?)
                .map(|literals| serde_json::to_string(&literals).expect("literal JSON")))
        },
    )?;
    db.create_scalar_function(
        c"nera_word",
        2,
        flags | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let at = number(ctx, 1)?
                .checked_mul(4)
                .ok_or_else(|| invalid("word offset overflow"))?;
            let word = ctx
                .get_raw(0)
                .as_blob()?
                .get(at..)
                .and_then(|bytes| bytes.get(..4))
                .ok_or_else(|| invalid("missing packed word"))?;
            Ok(u32::from_le_bytes(word.try_into().expect("word width")))
        },
    )?;
    let shared = readers.0.clone();
    db.create_scalar_function(c"nera_decode", 2, flags, move |ctx| {
        let (fid, pre) = (ctx.get::<i64>(0)?, number(ctx, 1)?);
        let mut reader = shared.lock();
        #[cfg(feature = "code-rule-profile")]
        {
            reader.counters.decoded_nodes += 1;
        }
        let rec = reader.rec(fid, pre)?;
        let flags = reader.flags(fid, pre)?;
        let parent = pre as u32 + 1 - reader.value(fid, Column::Parent, pre, rec[1])?;
        let end = pre as u32 + reader.value(fid, Column::Size, pre, rec[2])?;
        let kind = if flags & 8 != 0 {
            u16::MAX
        } else {
            rec[0] & 1023
        };
        Ok([
            parent,
            end,
            u32::from(kind),
            u32::from(rec[0] >> 10),
            u32::from(flags),
        ]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect::<Vec<_>>())
    })?;
    let shared = readers.0.clone();
    db.create_scalar_function(c"nera_span", 2, flags, move |ctx| {
        let range = shared.lock().span(ctx.get(0)?, number(ctx, 1)?)?;
        Ok([range.start as u32, range.len() as u32]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>())
    })?;
    let shared = readers.0.clone();
    db.create_scalar_function(c"nera_slice", 3, flags, move |ctx| {
        shared
            .lock()
            .slice(ctx.get(0)?, number(ctx, 1)?, number(ctx, 2)?)
    })?;
    let shared = readers.0.clone();
    db.create_scalar_function(c"nera_parent", 2, flags, move |ctx| {
        let (fid, pre) = (ctx.get::<i64>(0)?, number(ctx, 1)?);
        let mut reader = shared.lock();
        let rec = reader.rec(fid, pre)?;
        let delta = reader.value(fid, Column::Parent, pre, rec[1])?;
        Ok(pre as i64 - i64::from(delta))
    })?;
    let shared = readers.0.clone();
    db.create_scalar_function(c"nera_end", 2, flags, move |ctx| {
        let (fid, pre) = (ctx.get::<i64>(0)?, number(ctx, 1)?);
        let mut reader = shared.lock();
        let rec = reader.rec(fid, pre)?;
        Ok(pre as i64 + i64::from(reader.value(fid, Column::Size, pre, rec[2])?))
    })?;
    let shared = readers.0.clone();
    db.create_scalar_function(c"nera_field", 2, flags, move |ctx| {
        Ok(shared.lock().rec(ctx.get(0)?, number(ctx, 1)?)?[0] >> 10)
    })?;
    let shared = readers.0.clone();
    db.create_scalar_function(c"nera_child", 3, flags, move |ctx| {
        shared
            .lock()
            .child(ctx.get(0)?, number(ctx, 1)?, ctx.get(2)?)
    })?;
    let shared = readers.0.clone();
    db.create_scalar_function(c"nera_flags", 2, flags, move |ctx| {
        shared.lock().flags(ctx.get(0)?, number(ctx, 1)?)
    })?;
    let shared = readers.0.clone();
    db.create_scalar_function(c"nera_kind", 2, flags, move |ctx| {
        let (fid, pre) = (ctx.get::<i64>(0)?, number(ctx, 1)?);
        let mut reader = shared.lock();
        Ok(if reader.flags(fid, pre)? & 8 != 0 {
            u16::MAX
        } else {
            reader.rec(fid, pre)?[0] & 1023
        })
    })?;
    let shared = readers.0.clone();
    db.create_scalar_function(c"nera_equal", 3, flags, move |ctx| {
        Ok(ctx.get_raw(0).as_str()? == shared.lock().text(ctx.get(1)?, number(ctx, 2)?)?)
    })?;
    let shared = readers.0.clone();
    db.create_scalar_function(c"nera_matches", 3, flags, move |ctx| {
        let regex = ctx.get_or_create_aux(0, |value| {
            regex::Regex::new(value.as_str()?)
                .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })
        })?;
        Ok(regex.is_match(&shared.lock().text(ctx.get(1)?, number(ctx, 2)?)?))
    })?;
    db.create_scalar_function(c"nera_has_kind", 2, flags, |ctx| {
        let code = number(ctx, 1)?;
        Ok(ctx
            .get_raw(0)
            .as_blob()?
            .get(code / 8)
            .is_some_and(|v| v & (1 << (code % 8)) != 0))
    })?;
    Ok(readers)
}

/// The terms a pattern matches when it is `\A` and `\z` around a finite set of exact literals.
pub(super) fn literal_set(pattern: &str) -> Option<Vec<String>> {
    use regex_syntax::hir::{Hir, HirKind, Look, literal::Extractor};
    let hir = regex_syntax::parse(pattern).ok()?;
    let HirKind::Concat(parts) = hir.kind() else {
        return None;
    };
    let (first, rest) = parts.split_first()?;
    let (last, middle) = rest.split_last()?;
    if !matches!(first.kind(), HirKind::Look(Look::Start))
        || !matches!(last.kind(), HirKind::Look(Look::End))
    {
        return None;
    }
    let core = Hir::concat(middle.to_vec());
    // The extractor reads an assertion as the empty string.
    if !core.properties().look_set().is_empty() {
        return None;
    }
    let sequence = Extractor::new().extract(&core);
    let literals = sequence.literals()?;
    if literals.is_empty() || !literals.iter().all(|literal| literal.is_exact()) {
        return None;
    }
    literals
        .iter()
        .map(|literal| String::from_utf8(literal.as_bytes().to_vec()).ok())
        .collect()
}
