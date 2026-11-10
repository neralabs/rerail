//! JSON as `JSON.parse` reads it for the files a resolution reads: an object keeps its keys in
//! the order the text gives them (a repeated key keeps its first place and its last value), which
//! decides the order `exports` and `imports` conditions and tsconfig `paths` are tried in. The
//! engine's `serde_json` values keep keys sorted.

use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    /// `JSON.parse(text)`, a leading byte-order mark aside.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|error| error.to_string())
    }

    /// A JSON-with-comments text, as TypeScript reads a configuration file.
    pub(crate) fn parse_jsonc(text: &str) -> Option<Self> {
        serde_json::from_str(&crate::code_rule_dsl::module_graph::strip_jsonc(text)).ok()
    }

    /// An object's value at `key`.
    pub(crate) fn get(&self, key: &str) -> Option<&Json> {
        self.entries()?
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    /// An object's entries, in order.
    pub(crate) fn entries(&self) -> Option<&[(String, Json)]> {
        match self {
            Json::Object(entries) => Some(entries),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(text) => Some(text),
            _ => None,
        }
    }

    pub(crate) fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }

    pub(crate) fn is_null(&self) -> bool {
        matches!(self, Json::Null)
    }

    /// JavaScript's truthiness.
    pub(crate) fn truthy(&self) -> bool {
        match self {
            Json::Null => false,
            Json::Bool(flag) => *flag,
            Json::Number(number) => *number != 0.0 && !number.is_nan(),
            Json::String(text) => !text.is_empty(),
            Json::Array(_) | Json::Object(_) => true,
        }
    }

    /// JavaScript's `String(value)` of a pattern list's item.
    pub(crate) fn to_js_string(&self) -> String {
        match self {
            Json::Null => "null".to_owned(),
            Json::Bool(flag) => flag.to_string(),
            Json::Number(number) if number.fract() == 0.0 && number.abs() < 1e21 => {
                format!("{number:.0}")
            }
            Json::Number(number) => number.to_string(),
            Json::String(text) => text.clone(),
            Json::Array(_) => String::new(),
            Json::Object(_) => "[object Object]".to_owned(),
        }
    }
}

struct JsonVisitor;

impl<'de> Visitor<'de> for JsonVisitor {
    type Value = Json;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_unit<E>(self) -> Result<Json, E> {
        Ok(Json::Null)
    }

    fn visit_none<E>(self) -> Result<Json, E> {
        Ok(Json::Null)
    }

    fn visit_bool<E>(self, value: bool) -> Result<Json, E> {
        Ok(Json::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Json, E> {
        Ok(Json::Number(value as f64))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Json, E> {
        Ok(Json::Number(value as f64))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Json, E> {
        Ok(Json::Number(value))
    }

    fn visit_str<E>(self, value: &str) -> Result<Json, E> {
        Ok(Json::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Json, E> {
        Ok(Json::String(value))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Json, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(Json::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Json, A::Error> {
        let mut entries: Vec<(String, Json)> = Vec::new();
        while let Some((key, value)) = map.next_entry::<String, Json>()? {
            match entries.iter_mut().find(|(known, _)| *known == key) {
                Some(entry) => entry.1 = value,
                None => entries.push((key, value)),
            }
        }
        Ok(Json::Object(entries))
    }
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(JsonVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::Json;

    #[test]
    fn objects_keep_their_keys_in_text_order() {
        let parsed = Json::parse(r#"{ "types": null, "default": "./a.js", "types": 1 }"#).unwrap();
        let keys = parsed
            .entries()
            .unwrap()
            .iter()
            .map(|(key, _)| key.as_str());
        assert_eq!(keys.collect::<Vec<_>>(), ["types", "default"]);
        assert_eq!(parsed.get("types"), Some(&Json::Number(1.0)));
        assert!(Json::parse("{").is_err());
        assert_eq!(
            Json::parse_jsonc("{ // c\n \"a\": [1,], }").and_then(|json| json.get("a").cloned()),
            Some(Json::Array(vec![Json::Number(1.0)]))
        );
    }
}
