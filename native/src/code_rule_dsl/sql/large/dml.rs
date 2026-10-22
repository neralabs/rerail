use super::*;

#[derive(Clone)]
pub(super) struct Schema {
    pub declaration: String,
    pub columns: Vec<String>,
    pub defaults: Vec<String>,
    pub keys: BTreeSet<String>,
    rowid: Option<String>,
    non_null: BTreeSet<String>,
}
impl Schema {
    pub fn of(declaration: String) -> Result<Self, String> {
        let open = declaration.find('(').ok_or("program schema opening")?;
        let close = declaration.rfind(')').ok_or("program schema closing")?;
        let mut columns = Vec::new();
        let mut defaults = Vec::new();
        let mut defs = Vec::new();
        let mut keys = BTreeSet::new();
        let mut non_null = BTreeSet::new();
        let mut rowid = None;
        for def in lex::arguments(&declaration[open + 1..close]) {
            if let Some(key) = def.strip_prefix("PRIMARY KEY") {
                keys.extend(
                    lex::arguments(key.trim().trim_start_matches('(').trim_end_matches(')'))
                        .iter()
                        .map(|s| s.to_string()),
                );
                defs.push(def.to_owned());
                continue;
            }
            if def.starts_with("INDEX ") {
                defs.push(def.to_owned());
                continue;
            }
            let name = def
                .split_whitespace()
                .next()
                .ok_or("program column")?
                .to_owned();
            if def.contains("PRIMARY KEY") {
                keys.insert(name.clone());
            }
            if def.contains("INTEGER PRIMARY KEY") {
                rowid = Some(name.clone());
            }
            // A composite PRIMARY KEY alone permits NULL in SQLite rowid tables.
            if def.contains("NOT NULL") || def.contains("INTEGER PRIMARY KEY") {
                non_null.insert(name.clone());
            }
            columns.push(name);
            defaults.push(
                def.split_once("DEFAULT")
                    .map_or("NULL", |(_, d)| d.trim())
                    .to_owned(),
            );
            // Omitted defaults are expanded in the SELECT. The native buffer never guesses
            // whether NULL was explicit or omitted.
            defs.push(
                def.split_once("DEFAULT")
                    .map_or(def, |(d, _)| d.trim())
                    .to_owned(),
            );
        }
        Ok(Self {
            declaration: format!("{}({})", &declaration[..open], defs.join(",")),
            columns,
            defaults,
            keys,
            rowid,
            non_null,
        })
    }
}

/// Every mutable input has an explicit dependency on the recursive SQL iteration. Merely
/// correlating the publication aggregate leaves independent MATERIALIZED CTEs cached forever.
pub(super) fn correlate(sql: &str, schemas: &BTreeMap<String, Schema>) -> String {
    let mut text = sql.to_owned();
    let words = lex::identifiers(sql);
    let mut rounds = Vec::new();
    for words in words.windows(4) {
        if words[..3] != ["SELECT", "round", "FROM"]
            || schemas.get(words[3]).is_none_or(|s| s.columns != ["round"])
        {
            continue;
        }
        let name = words[3];
        let begin = words[0].as_ptr() as usize - sql.as_ptr() as usize;
        if begin == 0 {
            continue;
        }
        let original = format!("(SELECT round FROM {name})");
        if sql[begin - 1..].starts_with(&original) {
            rounds.push((
                begin - 1,
                begin - 1 + original.len(),
                format!("json_extract(driver.loops,'$.{name}')"),
            ));
        }
    }
    for (begin, end, value) in rounds.into_iter().rev() {
        text.replace_range(begin..end, &value);
    }
    let sql = text.as_str();
    let words = lex::identifiers(sql);
    let mut changes = Vec::new();
    for (at, &word) in words.iter().enumerate() {
        if word == "term" {
            let start = word.as_ptr() as usize - sql.as_ptr() as usize + word.len();
            let tail = sql[start..].trim_start();
            if let Some(value) = tail.strip_prefix('=').map(str::trim_start) {
                let len = value.bytes().take_while(u8::is_ascii_digit).count();
                if len > 0 {
                    let from = value.as_ptr() as usize - sql.as_ptr() as usize;
                    changes.push((
                        from,
                        from + len,
                        format!("({}+(driver.tick-driver.tick))", &value[..len]),
                    ));
                }
            }
        }
        if at == 0 || !matches!(words[at - 1], "FROM" | "JOIN") {
            continue;
        }
        // Shared stages are immutable after their one guarded publication in a batch.
        // Their membership lists can safely stay cached across SQL control iterations.
        if !schemas.contains_key(word) || word.starts_with("stage_") {
            continue;
        }
        let start = word.as_ptr() as usize - sql.as_ptr() as usize;
        let end = start + word.len();
        let tail = sql[end..].trim_start();
        let next = tail
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .next()
            .unwrap_or("");
        let alias = !next.is_empty()
            && !matches!(
                next,
                "WHERE"
                    | "ON"
                    | "USING"
                    | "LEFT"
                    | "RIGHT"
                    | "INNER"
                    | "CROSS"
                    | "JOIN"
                    | "UNION"
                    | "EXCEPT"
                    | "INTERSECT"
                    | "GROUP"
                    | "ORDER"
                    | "LIMIT"
                    | "HAVING"
                    | "WINDOW"
            );
        changes.push((
            start,
            end,
            format!(
                "(SELECT * FROM {word} WHERE driver.tick >= 0){}",
                if alias {
                    String::new()
                } else {
                    format!(" AS {word}")
                }
            ),
        ));
    }
    changes.sort_by_key(|(start, _, _)| *start);
    let mut result = sql.to_owned();
    for (start, end, text) in changes.into_iter().rev() {
        result.replace_range(start..end, &text);
    }
    result
}

pub(super) fn publish(
    schemas: &BTreeMap<String, Schema>,
    target: &str,
    body: &str,
    mode: &str,
) -> Result<String, String> {
    let schema = schemas
        .get(target)
        .ok_or_else(|| format!("program missing table {target}"))?;
    let fields = schema
        .columns
        .iter()
        .map(|c| format!("p.{c}"))
        .collect::<Vec<_>>()
        .join(",");
    let body = key_probes(body, schemas);
    let body = correlate(&body, schemas);
    // The aggregate owns every input value before finalize publishes the result. A second
    // materialized SQLite copy retains all these intermediate rows until the program ends.
    Ok(format!(
        "(WITH payload({}) AS NOT MATERIALIZED ({body}) SELECT p_{mode}_{target}(driver.tick,{fields}) FROM payload p)",
        schema.columns.join(",")
    ))
}

fn aliases<'a>(
    sql: &'a str,
    schemas: &BTreeMap<String, Schema>,
) -> Vec<(usize, usize, &'a str, &'a str)> {
    let words = lex::identifiers(sql);
    let mut out = Vec::new();
    let mut cursor = 0;
    let mut depth = 0usize;
    let mut quote = false;
    for (i, &word) in words.iter().enumerate() {
        let pos = word.as_ptr() as usize - sql.as_ptr() as usize;
        while cursor < pos {
            match sql.as_bytes()[cursor] {
                b'\'' if quote && sql.as_bytes().get(cursor + 1) == Some(&b'\'') => {
                    cursor += 2;
                    continue;
                }
                b'\'' => quote = !quote,
                b'(' if !quote => depth += 1,
                b')' if !quote => depth = depth.saturating_sub(1),
                _ => {}
            }
            cursor += 1;
        }
        if !matches!(word, "FROM" | "JOIN") {
            continue;
        }
        // A declared key on the nullable side of an outer join is not a non-null value.
        if word == "JOIN"
            && (i > 0 && words[i - 1] == "LEFT"
                || i > 1 && words[i - 1] == "OUTER" && words[i - 2] == "LEFT")
        {
            continue;
        }
        let Some(&table) = words.get(i + 1) else {
            continue;
        };
        if !schemas.contains_key(table) {
            continue;
        }
        let next = words.get(i + 2).copied().unwrap_or("");
        let alias = if next == "AS" {
            words.get(i + 3).copied().unwrap_or(table)
        } else if !next.is_empty()
            && !matches!(
                next,
                "WHERE"
                    | "ON"
                    | "JOIN"
                    | "LEFT"
                    | "CROSS"
                    | "RIGHT"
                    | "INNER"
                    | "GROUP"
                    | "ORDER"
                    | "UNION"
                    | "EXCEPT"
                    | "INTERSECT"
                    | "LIMIT"
                    | "HAVING"
            )
        {
            next
        } else {
            table
        };
        out.push((pos, depth, table, alias));
    }
    out
}
pub(super) fn key_probes(sql: &str, schemas: &BTreeMap<String, Schema>) -> String {
    let words = lex::identifiers(sql);
    if words.iter().any(|w| matches!(*w, "RIGHT" | "FULL")) {
        return sql.to_owned();
    }
    let bindings = aliases(sql, schemas);
    let mut edits = Vec::new();
    let mut used = 0;
    for (i, &word) in words.iter().enumerate() {
        if word != "IN" || i == 0 {
            continue;
        }
        let negated = words[i - 1] == "NOT";
        if negated && i < 2 {
            continue;
        }
        let column = i - 1 - usize::from(negated);
        let col = words[column];
        let mut start = col.as_ptr() as usize - sql.as_ptr() as usize;
        if start < used {
            continue;
        }
        let alias = if start > 0 && sql.as_bytes()[start - 1] == b'.' && column >= 1 {
            let alias = words[column - 1];
            start = alias.as_ptr() as usize - sql.as_ptr() as usize;
            Some(alias)
        } else {
            None
        };
        let before = &sql[..start];
        let depth = before.bytes().fold(0isize, |n, b| {
            n + isize::from(b == b'(') - isize::from(b == b')')
        });
        let table = bindings
            .iter()
            .rev()
            .find(|(pos, level, _, a)| {
                *pos < start && *level as isize <= depth && alias.is_none_or(|wanted| wanted == *a)
            })
            .or_else(|| {
                bindings.iter().find(|(pos, level, _, a)| {
                    *pos > start
                        && *level as isize == depth
                        && alias.is_none_or(|wanted| wanted == *a)
                })
            });
        let Some((_, _, table, _)) = table else {
            continue;
        };
        if !schemas[*table].keys.contains(col) || !schemas[*table].non_null.contains(col) {
            continue;
        }
        let at = word.as_ptr() as usize - sql.as_ptr() as usize + 2;
        let tail = sql[at..].trim_start();
        let Some(inner) = tail.strip_prefix("(SELECT ") else {
            continue;
        };
        let begin = inner.as_ptr() as usize - sql.as_ptr() as usize;
        let mut end = begin;
        let mut nesting = 1;
        let mut quote = false;
        while end < sql.len() {
            match sql.as_bytes()[end] {
                b'\'' if quote && sql.as_bytes().get(end + 1) == Some(&b'\'') => {
                    end += 2;
                    continue;
                }
                b'\'' => quote = !quote,
                b'(' if !quote => nesting += 1,
                b')' if !quote => {
                    nesting -= 1;
                    if nesting == 0 {
                        break;
                    }
                }
                _ => {}
            }
            end += 1;
        }
        if end == sql.len() {
            continue;
        }
        let inner = &sql[begin..end];
        let Some(from) = lex::top(inner)
            .into_iter()
            .find(|&j| inner[j..].starts_with(" FROM "))
        else {
            continue;
        };
        let projection = inner[..from]
            .trim()
            .strip_prefix("DISTINCT ")
            .unwrap_or(inner[..from].trim());
        if lex::arguments(projection).len() != 1 {
            continue;
        }
        let field = projection.rsplit('.').next().unwrap();
        let qualified = projection.rsplit_once('.').map(|(a, _)| a);
        if !aliases(inner, schemas).iter().any(|(_, _, t, a)| {
            qualified.is_none_or(|q| q == *a)
                && schemas[*t].keys.contains(field)
                && schemas[*t].non_null.contains(field)
        }) {
            continue;
        }
        let lhs = &sql[start..col.as_ptr() as usize - sql.as_ptr() as usize + col.len()];
        edits.push((start,end+1,format!("{}EXISTS(SELECT 1 FROM (SELECT {projection} AS __key{}) membership WHERE membership.__key={lhs})",if negated{"NOT "}else{""},&inner[from..])));
        used = end + 1;
    }
    let mut out = sql.to_owned();
    for (start, end, text) in edits.into_iter().rev() {
        out.replace_range(start..end, &text);
    }
    out
}

pub(super) enum Action {
    Clear(String),
    Patch {
        target: String,
        body: String,
        columns: Vec<usize>,
    },
    Publish {
        target: String,
        body: String,
        mode: &'static str,
    },
}
pub(super) fn lower(schemas: &BTreeMap<String, Schema>, sql: &str) -> Result<Action, String> {
    let sql = sql.trim();
    let at = lex::top(sql)
        .into_iter()
        .find(|&at| sql[at..].starts_with("INSERT "));
    if let Some(at) = at {
        let rest = &sql[at..];
        let (mode, rest) = if let Some(s) = rest.strip_prefix("INSERT OR IGNORE INTO ") {
            ("ignore", s)
        } else {
            (
                "append",
                rest.strip_prefix("INSERT INTO ")
                    .ok_or_else(|| format!("program INSERT {sql}"))?,
            )
        };
        let end = rest
            .find(|c: char| c == '(' || c.is_whitespace())
            .ok_or("program target")?;
        let target = &rest[..end];
        let mut tail = rest[end..].trim_start();
        let schema = schemas
            .get(target)
            .ok_or_else(|| format!("program table {target}"))?;
        let supplied = if tail.starts_with('(') {
            let close = tail.find(')').ok_or("program insert columns")?;
            let names = lex::arguments(&tail[1..close])
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>();
            tail = tail[close + 1..].trim_start();
            names
        } else {
            schema.columns.clone()
        };
        let body = format!("{}{}", &sql[..at], tail);
        if supplied == schema.columns {
            return Ok(Action::Publish {
                target: target.into(),
                body,
                mode,
            });
        }
        let fields = schema
            .columns
            .iter()
            .zip(&schema.defaults)
            .map(|(name, default)| {
                if supplied.contains(name) {
                    format!("v.{name}")
                } else {
                    default.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        return Ok(Action::Publish {
            target: target.into(),
            body: format!(
                "WITH supplied({}) AS ({body}) SELECT {fields} FROM supplied v",
                supplied.join(",")
            ),
            mode,
        });
    }
    if let Some(rest) = sql.strip_prefix("UPDATE ") {
        let (target, rest) = rest.split_once(" SET ").ok_or("program update SET")?;
        let (set, predicate) = lex::top(rest)
            .into_iter()
            .find(|&at| rest[at..].starts_with(" WHERE "))
            .map_or((rest, "1"), |at| (&rest[..at], &rest[at + 7..]));
        let schema = schemas.get(target).ok_or("program update target")?;
        let sets = lex::arguments(set)
            .into_iter()
            .map(|s| s.split_once(" = ").ok_or("program assignment"))
            .collect::<Result<HashMap<_, _>, _>>()?;
        if let Some(key) = &schema.rowid
            && sets.keys().all(|c| !schema.keys.contains(*c))
        {
            let columns = schema
                .columns
                .iter()
                .enumerate()
                .filter_map(|(i, c)| sets.contains_key(c.as_str()).then_some(i))
                .collect::<Vec<_>>();
            if columns.len() != sets.len() {
                return Err("program update column".into());
            }
            let fields = columns
                .iter()
                .map(|&i| sets[schema.columns[i].as_str()])
                .collect::<Vec<_>>()
                .join(",");
            return Ok(Action::Patch {
                target: target.into(),
                body: format!("SELECT {key},{fields} FROM {target} WHERE {predicate}"),
                columns,
            });
        }
        let fields = schema
            .columns
            .iter()
            .map(|c| {
                sets.get(c.as_str()).map_or(c.clone(), |v| {
                    format!("CASE WHEN {predicate} THEN {v} ELSE {c} END")
                })
            })
            .collect::<Vec<_>>()
            .join(",");
        return Ok(Action::Publish {
            target: target.into(),
            body: format!("SELECT {fields} FROM {target}"),
            mode: "replace",
        });
    }
    if let Some(rest) = sql.strip_prefix("DELETE FROM ") {
        let (target, predicate) = rest
            .split_once(" WHERE ")
            .map_or((rest, None), |(t, p)| (t, Some(p)));
        return match predicate {
            None => Ok(Action::Clear(target.into())),
            Some(condition) => Ok(Action::Publish {
                target: target.into(),
                body: format!("SELECT * FROM {target} WHERE NOT COALESCE(({condition}),0)"),
                mode: "replace",
            }),
        };
    }
    Err(format!("program statement not lowered: {sql}"))
}

#[cfg(test)]
pub(super) fn statement(schemas: &BTreeMap<String, Schema>, sql: &str) -> Result<String, String> {
    match lower(schemas, sql)? {
        Action::Clear(name) => Ok(format!("p_clear(driver.tick,'{name}')")),
        Action::Publish { target, body, mode } => publish(schemas, &target, &body, mode),
        Action::Patch {
            target,
            body,
            columns,
        } => Ok(patch(schemas, &target, &body, &columns)),
    }
}

pub(super) fn patch(
    schemas: &BTreeMap<String, Schema>,
    target: &str,
    body: &str,
    columns: &[usize],
) -> String {
    let names = (0..=columns.len())
        .map(|i| format!("v{i}"))
        .collect::<Vec<_>>();
    let fields = columns
        .iter()
        .enumerate()
        .map(|(i, c)| format!(",{c},p.v{}", i + 1))
        .collect::<String>();
    let body = correlate(&key_probes(body, schemas), schemas);
    format!(
        "(WITH payload({}) AS NOT MATERIALIZED ({body}) SELECT p_patch_{target}(driver.tick,p.v0{fields}) FROM payload p)",
        names.join(",")
    )
}
