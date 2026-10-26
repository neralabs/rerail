//! enhanced-resolve's `util/entrypoints.js`: a package's `exports` and `imports` fields, matched
//! against a request under a set of condition names.

use super::json::Json;
use std::borrow::Cow;
use std::cmp::Ordering;

/// `parseIdentifier`'s path part of a target: up to its query (`?`) or fragment (`#`, past the
/// first character).
pub(super) fn identifier_path(identifier: &str) -> &str {
    let query = identifier.find('?');
    let fragment = identifier
        .char_indices()
        .skip(1)
        .find(|&(_, c)| c == '#')
        .map(|(at, _)| at);
    let end = match (query, fragment) {
        (Some(query), Some(fragment)) => query.min(fragment),
        (Some(at), None) | (None, Some(at)) => at,
        (None, None) => identifier.len(),
    };
    &identifier[..end]
}

/// `patternKeyCompare`: the longer prefix (up to and with its `*`) first, then a pattern, then
/// the longer key.
fn pattern_key_compare(a: &str, b: &str) -> Ordering {
    let a_star = a.find('*');
    let b_star = b.find('*');
    let base_a = a_star.map_or(a.len(), |at| at + 1);
    let base_b = b_star.map_or(b.len(), |at| at + 1);
    if base_a > base_b {
        return Ordering::Less;
    }
    if base_b > base_a {
        return Ordering::Greater;
    }
    if a_star.is_none() {
        return Ordering::Greater;
    }
    if b_star.is_none() {
        return Ordering::Less;
    }
    b.len().cmp(&a.len())
}

/// A key's match: its mapping, the request's remainder, whether the key ends with `/`, whether it
/// is a pattern, and the key.
struct Match<'f> {
    mapping: &'f Json,
    remaining: String,
    subpath: bool,
    pattern: bool,
    key: &'f str,
}

/// `findMatch`: the key the request names exactly, else the best pattern or legacy subpath key.
fn find_match<'f>(request: &str, field: &'f [(String, Json)]) -> Option<Match<'f>> {
    if !request.contains('*')
        && !request.ends_with('/')
        && let Some((key, mapping)) = field.iter().find(|(key, _)| key == request)
    {
        return Some(Match {
            mapping,
            remaining: String::new(),
            subpath: false,
            pattern: false,
            key,
        });
    }
    let mut best: Option<Match<'f>> = None;
    for (key, mapping) in field {
        let best_key = best.as_ref().map_or("", |best| best.key);
        match key.find('*') {
            Some(star) => {
                let valid = !key[star + 1..].contains('*');
                let (prefix, suffix) = (&key[..star], &key[star + 1..]);
                if !valid
                    || !request.starts_with(prefix)
                    || request.len() < key.len()
                    || !request.ends_with(suffix)
                    || pattern_key_compare(best_key, key) != Ordering::Greater
                {
                    continue;
                }
                best = Some(Match {
                    mapping,
                    remaining: request[star..request.len() - suffix.len()].to_owned(),
                    subpath: key.ends_with('/'),
                    pattern: true,
                    key,
                });
            }
            None => {
                if key.ends_with('/')
                    && request.starts_with(key.as_str())
                    && pattern_key_compare(best_key, key) == Ordering::Greater
                {
                    best = Some(Match {
                        mapping,
                        remaining: request[key.len()..].to_owned(),
                        subpath: true,
                        pattern: false,
                        key,
                    });
                }
            }
        }
    }
    best
}

/// `conditionalMapping`: the first value, in key order, whose condition is `default` or one of
/// `conditions`, descending into nested condition objects and back out of ones that hold none.
fn conditional_mapping<'f>(mapping: &'f [(String, Json)], conditions: &[&str]) -> Option<&'f Json> {
    for (condition, inner) in mapping {
        if condition != "default" && !conditions.contains(&condition.as_str()) {
            continue;
        }
        match inner {
            Json::Object(nested) => {
                if let Some(found) = conditional_mapping(nested, conditions) {
                    return Some(found);
                }
            }
            inner => return Some(inner),
        }
    }
    None
}

/// `assertExportTarget`/`assertImportTarget`: a folder mapping's target ends with `/`, a file
/// mapping's does not.
fn assert_target(target: &str, folder: bool) -> Result<(), String> {
    let path = identifier_path(target);
    if path.ends_with('/') == folder {
        return Ok(());
    }
    let quoted = serde_json::to_string(target).unwrap_or_default();
    Err(if folder {
        format!("Expecting folder to folder mapping. {quoted} should end with \"/\"")
    } else {
        format!("Expecting file to file mapping. {quoted} should not end with \"/\"")
    })
}

/// `targetMapping`.
fn target_mapping(found: &Match<'_>, target: &str) -> Result<String, String> {
    if found.subpath {
        assert_target(target, true)?;
        return Ok(format!("{target}{}", found.remaining));
    }
    assert_target(target, false)?;
    Ok(if found.pattern {
        target.replace('*', &found.remaining)
    } else {
        target.to_owned()
    })
}

/// `directMapping`.
fn direct_mapping(
    found: &Match<'_>,
    target: &Json,
    conditions: &[&str],
    targets: &mut Vec<String>,
) -> Result<(), String> {
    match target {
        Json::String(target) => targets.push(target_mapping(found, target)?),
        Json::Array(items) => {
            for item in items {
                match item {
                    Json::String(target) => targets.push(target_mapping(found, target)?),
                    Json::Object(mapping) => {
                        if let Some(mapping) = conditional_mapping(mapping, conditions)
                            && mapping.truthy()
                        {
                            direct_mapping(found, mapping, conditions, targets)?;
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// `createFieldProcessor`'s processor: the targets the request maps to, and the key it used.
fn process(
    field: &[(String, Json)],
    request: &str,
    conditions: &[&str],
) -> Result<(Vec<String>, Option<String>), String> {
    let Some(found) = find_match(request, field) else {
        return Ok((Vec::new(), None));
    };
    let direct = match found.mapping {
        Json::Object(mapping) => match conditional_mapping(mapping, conditions) {
            Some(direct) if !direct.is_null() => direct,
            _ => return Ok((Vec::new(), None)),
        },
        direct => direct,
    };
    let mut targets = Vec::new();
    direct_mapping(&found, direct, conditions, &mut targets)?;
    Ok((targets, Some(found.key.to_owned())))
}

/// `buildExportsField`: a string or array is the `.` export; an object whose keys are conditions
/// is too; one whose keys are subpaths is the field itself.
fn build_exports_field(field: &Json) -> Result<Cow<'_, [(String, Json)]>, String> {
    let root = |field: &Json| Cow::Owned(vec![(".".to_owned(), field.clone())]);
    let Json::Object(entries) = field else {
        return Ok(root(field));
    };
    let invalid = |key: &str| {
        format!(
            "Exports field key should be relative path and start with \".\" (key: {})",
            serde_json::to_string(key).unwrap_or_default()
        )
    };
    for (index, (key, _)) in entries.iter().enumerate() {
        if !key.starts_with('.') {
            if index == 0 {
                if let Some((key, _)) = entries
                    .iter()
                    .find(|(key, _)| key.starts_with('.') || key.starts_with('/'))
                {
                    return Err(invalid(key));
                }
                return Ok(root(field));
            }
            return Err(invalid(key));
        }
        if key.len() > 1 && !key[1..].starts_with('/') {
            return Err(format!(
                "Exports field key should be relative path and start with \"./\" (key: {})",
                serde_json::to_string(key).unwrap_or_default()
            ));
        }
    }
    Ok(Cow::Borrowed(entries))
}

/// `processExportsField(field)(request, conditions)`: `request` is `.` or `./…`.
pub(super) fn exports(
    field: &Json,
    request: &str,
    conditions: &[&str],
) -> Result<(Vec<String>, Option<String>), String> {
    let field = build_exports_field(field)?;
    if !request.starts_with('.') {
        return Err("Request should be relative path and start with \".\"".to_owned());
    }
    let normalized = if request.len() == 1 {
        ".".to_owned()
    } else {
        if !request[1..].starts_with('/') {
            return Err("Request should be relative path and start with \"./\"".to_owned());
        }
        if request.ends_with('/') {
            return Err("Only requesting file allowed".to_owned());
        }
        let rest = &request[2..];
        if rest.is_empty() {
            ".".to_owned()
        } else {
            format!("./{rest}")
        }
    };
    process(&field, &normalized, conditions)
}

/// `processImportsField(field)(request, conditions)`: `request` is `#…`.
pub(super) fn imports(
    field: &Json,
    request: &str,
    conditions: &[&str],
) -> Result<(Vec<String>, Option<String>), String> {
    let Json::Object(field) = field else {
        return Ok((Vec::new(), None));
    };
    if !request.starts_with('#') {
        return Err("Request should start with \"#\"".to_owned());
    }
    if request.len() == 1 {
        return Err("Request should have at least 2 characters".to_owned());
    }
    if request.ends_with('/') {
        return Err("Only requesting file allowed".to_owned());
    }
    process(field, request, conditions)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(text: &str) -> Json {
        Json::parse(text).unwrap()
    }

    #[test]
    fn exports_map_requests_under_conditions() {
        let field = json(
            r#"{
            ".": { "import": "./main.mjs", "require": "./main.cjs" },
            "./sub": "./sub.js",
            "./features/*.js": { "types": null, "default": "./features/*.js" },
            "./dir/": "./lib/"
        }"#,
        );
        let run = |request: &str, conditions: &[&str]| {
            exports(&field, request, conditions).map(|(paths, _)| paths)
        };
        assert_eq!(
            run(".", &["node", "import"]),
            Ok(vec!["./main.mjs".to_owned()])
        );
        assert_eq!(
            run(".", &["node", "require"]),
            Ok(vec!["./main.cjs".to_owned()])
        );
        assert_eq!(run("./sub", &["node"]), Ok(vec!["./sub.js".to_owned()]));
        assert_eq!(run("./sub.mjs", &["node"]), Ok(Vec::new()));
        assert_eq!(
            run("./features/a.js", &["node"]),
            Ok(vec!["./features/a.js".to_owned()])
        );
        assert_eq!(run("./features/a.js", &["types"]), Ok(Vec::new()));
        assert_eq!(
            run("./dir/x.js", &["node"]),
            Ok(vec!["./lib/x.js".to_owned()])
        );
        assert!(exports(&json(r#"{ "a": 1, "./b": 2 }"#), ".", &[]).is_err());
        assert_eq!(
            exports(&json(r#""./index.js""#), ".", &[]).map(|(paths, _)| paths),
            Ok(vec!["./index.js".to_owned()])
        );
        assert_eq!(
            imports(
                &json(r##"{ "#a": { "node": "./a.js" } }"##),
                "#a",
                &["node"]
            )
            .map(|(paths, _)| paths),
            Ok(vec!["./a.js".to_owned()])
        );
    }
}
