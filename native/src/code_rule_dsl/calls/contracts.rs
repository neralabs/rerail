//! Authored delegation from a decorated method to a named sibling. Contracts are
//! immutable and interned; modes carry their small key, not a regex per graph edge.

use crate::code_rule_dsl::json::{invalid, non_empty_str, object, reject_unknown};
use napi::Result;
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

pub(crate) type Key = u32;

struct Link {
    decorator: Regex,
    name: Regex,
    by: String,
}

pub(crate) struct Contracts {
    pub(crate) canonical: Arc<str>,
    links: Vec<Link>,
    constructor: Option<(Regex, String)>,
}

struct Registry {
    values: Vec<Arc<Contracts>>,
    keys: HashMap<String, Key>,
}

static REGISTRY: LazyLock<parking_lot::Mutex<Registry>> = LazyLock::new(|| {
    parking_lot::Mutex::new(Registry {
        values: vec![Arc::new(Contracts {
            canonical: "[]".into(),
            links: Vec::new(),
            constructor: None,
        })],
        keys: HashMap::new(),
    })
});

pub(crate) fn get(key: Key) -> Arc<Contracts> {
    Arc::clone(&REGISTRY.lock().values[key as usize])
}

pub(crate) fn compile(
    value: Option<&Value>,
    constructor: Option<&Value>,
    path: &str,
    max_pattern: usize,
) -> Result<Key> {
    if value.is_none() && constructor.is_none() {
        return Ok(0);
    }
    let sibling_path = format!("{path}/siblingLinks");
    let values = match value {
        None => &[][..],
        Some(value) => value
            .as_array()
            .filter(|values| !values.is_empty() && values.len() <= 32)
            .ok_or_else(|| invalid(format!("{sibling_path} must contain 1 to 32 sibling links")))?
            .as_slice(),
    };
    let canonical =
        serde_json::to_string(&(value, constructor)).map_err(|error| invalid(error.to_string()))?;
    let constructor = constructor
        .map(|value| -> Result<_> {
            let at = format!("{path}/constructorFields");
            let spec = object(value, &at)?;
            reject_unknown(spec, &["fieldToParameter"], &at)?;
            replacement(
                spec.get("fieldToParameter"),
                &format!("{at}/fieldToParameter"),
                max_pattern,
            )
        })
        .transpose()?;
    let mut links = Vec::new();
    for (index, value) in values.iter().enumerate() {
        let at = format!("{sibling_path}/{index}");
        let spec = object(value, &at)?;
        reject_unknown(spec, &["decorator", "name"], &at)?;
        let decorator = bounded_regex(
            spec.get("decorator"),
            &format!("{at}/decorator"),
            max_pattern,
        )?;
        let (pattern, by) = replacement(spec.get("name"), &format!("{at}/name"), max_pattern)?;
        links.push(Link {
            decorator,
            name: pattern,
            by,
        });
    }
    let mut registry = REGISTRY.lock();
    if let Some(key) = registry.keys.get(&canonical) {
        return Ok(*key);
    }
    let key = Key::try_from(registry.values.len())
        .map_err(|_| invalid("too many sibling link configurations"))?;
    registry.values.push(Arc::new(Contracts {
        canonical: canonical.clone().into(),
        links,
        constructor,
    }));
    registry.keys.insert(canonical, key);
    Ok(key)
}

pub(crate) const UNANNOTATED: &str = "<unannotated>";
pub(crate) const UNRESOLVED_TYPE: &str = "<unresolved-type>";

pub(crate) fn parameter_key(class: &str, parameter: &str) -> String {
    format!("{class}.<init-parameter>.{parameter}")
}

fn bounded_regex(value: Option<&Value>, path: &str, max: usize) -> Result<Regex> {
    let pattern = non_empty_str(value, path)?;
    if pattern.len() > max {
        return Err(invalid(format!("{path} exceeds maxRegexPatternBytes")));
    }
    Regex::new(pattern).map_err(|error| invalid(format!("{path}: {error}")))
}

fn replacement(value: Option<&Value>, path: &str, max: usize) -> Result<(Regex, String)> {
    let spec = object(
        value.ok_or_else(|| invalid(format!("{path} is required")))?,
        path,
    )?;
    reject_unknown(spec, &["replace", "by"], path)?;
    let regex = bounded_regex(spec.get("replace"), &format!("{path}/replace"), max)?;
    let by = spec
        .get("by")
        .and_then(Value::as_str)
        .filter(|by| by.len() <= max)
        .ok_or_else(|| {
            invalid(format!(
                "{path}/by must be a string within maxRegexPatternBytes"
            ))
        })?;
    Ok((regex, by.to_owned()))
}

impl Contracts {
    pub(crate) fn constructor_names(&self, field: &str) -> Option<Vec<String>> {
        let (pattern, by) = self.constructor.as_ref()?;
        let mapped = pattern.replace(field, by.as_str());
        let mut names = vec![field.to_owned()];
        if mapped != field {
            names.push(mapped.into_owned());
        }
        Some(names)
    }

    /// The first contract matching both a decorator and the method's name wins.
    pub(crate) fn sibling(&self, qualified: &str, decorators: &[&str]) -> Option<String> {
        let (owner, name) = qualified.rsplit_once('.')?;
        let link = self.links.iter().find(|link| {
            link.name.is_match(name)
                && decorators
                    .iter()
                    .any(|decorator| link.decorator.is_match(decorator))
        })?;
        let name = link.name.replace(name, link.by.as_str());
        (!name.is_empty()).then(|| format!("{owner}.{name}"))
    }
}
