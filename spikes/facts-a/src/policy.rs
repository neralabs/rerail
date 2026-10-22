//! Policy documents: `$option` resolution (native `compile/config.rs`) per call configuration, and
//! the clause structure the spike evaluates.
use serde_json::{Map, Value as Json};
use std::collections::BTreeMap;

fn json_text(v: &Json) -> String {
    match v {
        Json::Null => "null".into(),
        Json::Bool(b) => b.to_string(),
        Json::Number(n) => {
            // JavaScript writes integral floats without a fraction.
            if let Some(f) = n.as_f64().filter(|f| f.fract() == 0.0 && !n.is_i64() && !n.is_u64()) {
                format!("{}", f as i64)
            } else {
                n.to_string()
            }
        }
        other => other.to_string(),
    }
}

fn case_key(v: &Json) -> String {
    match v {
        Json::String(s) => s.clone(),
        other => json_text(other),
    }
}

fn text(v: &Json) -> Result<String, String> {
    match v {
        Json::String(s) => Ok(s.clone()),
        Json::Number(_) | Json::Bool(_) => Ok(json_text(v)),
        _ => Err("option value is not text".into()),
    }
}

enum Resolved {
    Value(Json),
    Spread(Vec<Json>),
}

struct Options {
    values: BTreeMap<String, Json>,
}

impl Options {
    fn substitute(&self, v: &Json) -> Result<Json, String> {
        match self.reference(v)? {
            Some(Resolved::Value(v)) => Ok(v),
            Some(Resolved::Spread(_)) => Err("only an array's item may spread".into()),
            None => self.inner(v),
        }
    }
    fn inner(&self, v: &Json) -> Result<Json, String> {
        match v {
            Json::Object(fields) => {
                let mut out = Map::new();
                for (k, c) in fields {
                    out.insert(k.clone(), self.substitute(c)?);
                }
                Ok(Json::Object(out))
            }
            Json::Array(items) => {
                let mut out = Vec::new();
                for it in items {
                    match self.reference(it)? {
                        Some(Resolved::Value(v)) => out.push(v),
                        Some(Resolved::Spread(vs)) => out.extend(vs),
                        None => out.push(self.inner(it)?),
                    }
                }
                Ok(Json::Array(out))
            }
            other => Ok(other.clone()),
        }
    }
    fn reference(&self, v: &Json) -> Result<Option<Resolved>, String> {
        let Some(fields) = v.as_object().filter(|f| f.contains_key("$option")) else { return Ok(None) };
        let name = fields["$option"].as_str().ok_or("$option must name an option")?;
        let option = self.values.get(name).ok_or_else(|| format!("unknown option {name}"))?;
        let spread = fields.get("spread").and_then(Json::as_bool).unwrap_or(false);
        let resolved = if let Some(cases) = fields.get("cases") {
            let key = case_key(option);
            let chosen = match cases.get(&key) {
                Some(c) => c,
                None => fields.get("default").ok_or_else(|| format!("no case for {key}"))?,
            };
            self.substitute(chosen)?
        } else {
            let mut value = option.clone();
            if let Some(esc) = fields.get("escape").and_then(Json::as_str) {
                if esc != "regex" {
                    return Err(format!("escape {esc}"));
                }
                let e = |v: &Json| -> Result<Json, String> { Ok(Json::String(regex::escape(&text(v)?))) };
                value = match &value {
                    Json::Array(items) => Json::Array(items.iter().map(e).collect::<Result<_, _>>()?),
                    o => e(o)?,
                };
            }
            if let Some(t) = fields.get("map").and_then(Json::as_str) {
                let items = value.as_array().ok_or("map over non-array")?;
                value = Json::Array(items.iter().map(|i| Ok(Json::String(t.replace("{}", &text(i)?)))).collect::<Result<_, String>>()?);
            }
            if let Some(sep) = fields.get("join").and_then(Json::as_str) {
                let items = value.as_array().ok_or("join over non-array")?;
                value = Json::String(items.iter().map(text).collect::<Result<Vec<_>, _>>()?.join(sep));
            }
            if let Some(t) = fields.get("template").and_then(Json::as_str) {
                value = Json::String(t.replace("{}", &text(&value)?));
            }
            value
        };
        if !spread {
            return Ok(Some(Resolved::Value(resolved)));
        }
        match resolved {
            Json::Array(items) => Ok(Some(Resolved::Spread(items))),
            _ => Err("spread of a non-array".into()),
        }
    }
}

/// The document with every `$option` reference replaced, under `config` (option values that
/// override the defaults, as `replay.ts` writes them into `define.config`).
pub fn resolve(document: &Json, config: &Map<String, Json>) -> Result<Json, String> {
    let clauses = document["clauses"].as_array().ok_or("clauses")?;
    let mut values = BTreeMap::new();
    for clause in clauses {
        if clause.get("$option").is_some() {
            continue;
        }
        if let Some(opts) = clause.get("define").and_then(|d| d.get("options")).and_then(Json::as_object) {
            for (name, decl) in opts {
                values.insert(name.clone(), decl.get("default").cloned().unwrap_or(Json::Null));
            }
        }
    }
    for clause in clauses {
        if let Some(cfg) = clause.get("define").and_then(|d| d.get("config")).and_then(Json::as_object) {
            for (k, v) in cfg {
                values.insert(k.clone(), v.clone());
            }
        }
    }
    for (k, v) in config {
        if !values.contains_key(k) {
            return Err(format!("config {k} is not a declared option"));
        }
        values.insert(k.clone(), v.clone());
    }
    let options = Options { values };
    let root = document.as_object().ok_or("document")?;
    let mut out = Map::new();
    for (key, value) in root {
        let v = match key.as_str() {
            "format" | "id" | "revision" => value.clone(),
            "clauses" => {
                let mut items = Vec::new();
                for clause in clauses {
                    match options.reference(clause)? {
                        None => {
                            if let Some(define) = clause.get("define") {
                                let mut d = Map::new();
                                for (k, v) in define.as_object().ok_or("define")? {
                                    d.insert(k.clone(), if k == "options" || k == "config" { v.clone() } else { options.substitute(v)? });
                                }
                                let mut c = clause.as_object().unwrap().clone();
                                c.insert("define".into(), Json::Object(d));
                                items.push(Json::Object(c));
                            } else {
                                items.push(options.substitute(clause)?);
                            }
                        }
                        Some(Resolved::Value(v)) => items.push(v),
                        Some(Resolved::Spread(vs)) => items.extend(vs),
                    }
                }
                Json::Array(items)
            }
            _ => options.substitute(value)?,
        };
        out.insert(key.clone(), v);
    }
    Ok(Json::Object(out))
}

/// Globset scope (`literal_separator(true)`), matched against the repo-relative POSIX path.
pub struct Scope {
    include: globset::GlobSet,
    exclude: globset::GlobSet,
}
impl Scope {
    pub fn new(v: Option<&Json>) -> Result<Self, String> {
        let build = |items: &[Json]| -> Result<globset::GlobSet, String> {
            let mut b = globset::GlobSetBuilder::new();
            for p in items {
                let p = p.as_str().ok_or("glob")?;
                b.add(globset::GlobBuilder::new(p).literal_separator(true).build().map_err(|e| e.to_string())?);
            }
            b.build().map_err(|e| e.to_string())
        };
        match v {
            None => Ok(Self { include: build(&[Json::String("**".into())])?, exclude: build(&[])? }),
            Some(s) => Ok(Self {
                include: build(s["include"].as_array().ok_or("scope.include")?)?,
                exclude: build(s["exclude"].as_array().ok_or("scope.exclude")?)?,
            }),
        }
    }
    pub fn matches(&self, path: &str) -> bool {
        self.include.is_match(path) && !self.exclude.is_match(path)
    }
}
