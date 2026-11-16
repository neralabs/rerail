//! What a run adds to its programs (decision 0249): the run's settings (`env`, from `<root>/rerail.env` and
//! `--env`), the typing declarations (`declared_ret`), and the dialect a program of a language of several is run in
//! (the `dialect` option, `tools::run_program`). They are rows and a setting, never text: each program that declares
//! a relation of them is given its rows once it is read, a program that names its language the dialect
//! (`add_to`), and every key the run makes of its programs is made of these too (`hash`), as it once was of the
//! text they were printed into.

use crate::code_rule_dsl::logic::front::{self, Value};
use std::collections::BTreeMap;

/// Rows by relation, each row its values; and the dialect.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Added(BTreeMap<String, Vec<Vec<Value>>>, Option<String>);

impl Added {
    /// What a run of `options` over the root of `settings` adds: the settings' rows, the `dialect` option.
    pub fn of(settings: &[(String, String)], options: &serde_json::Value) -> Added {
        let mut added = Added::settings(settings);
        added.1 = options.get("dialect").and_then(|v| v.as_str()).map(str::to_owned);
        added
    }

    /// The run's settings, as `env(key, value)` rows.
    pub fn settings(settings: &[(String, String)]) -> Added {
        Added::default().with("env", settings.iter().map(|(k, v)| vec![Value::Str(k.as_str().into()), Value::Str(v.as_str().into())]))
    }

    /// These, and `rows` of `rel`.
    pub fn with(mut self, rel: &str, rows: impl IntoIterator<Item = Vec<Value>>) -> Added {
        let mut rows = rows.into_iter().peekable();
        if rows.peek().is_some() {
            self.0.entry(rel.to_owned()).or_default().extend(rows);
        }
        self
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty() && self.1.is_none()
    }

    /// `prog` given the rows of each relation it declares, and the dialect as its language when it names one. A
    /// program that declares none of them and names no language is as it was.
    pub fn add_to(&self, prog: &mut front::Program) {
        if let Some(dialect) = &self.1 {
            for (key, value) in &mut prog.settings {
                if key == "language" {
                    *value = dialect.clone();
                }
            }
        }
        for (rel, rows) in &self.0 {
            if prog.decls.contains_key(rel.as_str()) {
                prog.facts.entry(rel.clone()).or_default().extend(rows.iter().map(|r| front::List::from(r.clone())));
            }
        }
    }

    /// The rows into a key: each relation, its row count, and each value by its type.
    pub fn hash(&self, h: &mut impl sha2::Digest) {
        h.update(b"added\0");
        for (rel, rows) in &self.0 {
            h.update((rel.len() as u64).to_le_bytes());
            h.update(rel.as_bytes());
            h.update((rows.len() as u64).to_le_bytes());
            for row in rows {
                h.update((row.len() as u64).to_le_bytes());
                for v in row {
                    match v {
                        Value::Int(n) => {
                            h.update([0]);
                            h.update(n.to_le_bytes());
                        }
                        Value::Str(s) => {
                            h.update([1]);
                            h.update((s.len() as u64).to_le_bytes());
                            h.update(s.as_bytes());
                        }
                        // A branded atom (decision 0270) has its own tag, its class with it.
                        Value::Branded(b) => {
                            h.update([2, b.class as u8]);
                            h.update(b.atom.to_le_bytes());
                        }
                    }
                }
            }
        }
        if let Some(dialect) = &self.1 {
            h.update(b"dialect\0");
            h.update(dialect.as_bytes());
        }
    }

    /// `hash`'s digest, alone.
    pub fn digest(&self) -> [u8; 32] {
        use sha2::Digest as _;
        let mut h = sha2::Sha256::new();
        self.hash(&mut h);
        h.finalize().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn added_rows_go_to_the_programs_that_declare_them_and_key_by_value() {
        let mut declares = front::Program::default();
        declares.decls.insert("env".into(), vec![("k".into(), "symbol".into()), ("v".into(), "symbol".into())].into());
        let mut not = front::Program::default();
        let added = Added::settings(&[("a".into(), "1".into()), ("b".into(), "x\ty".into())]).with("declared_ret", Vec::new());
        added.add_to(&mut declares);
        added.add_to(&mut not);
        let env: Vec<Vec<Value>> = declares.facts["env"].iter().map(|r| r.to_vec()).collect();
        assert_eq!(env, [vec![Value::Str("a".into()), Value::Str("1".into())], vec![Value::Str("b".into()), Value::Str("x\ty".into())]]);
        assert!(not.facts.is_empty());
        // No rows: no relation.
        assert!(Added::default().with("env", Vec::new()).is_empty());
        // Keyed by every value, and by its type.
        assert_ne!(added.digest(), Added::settings(&[("a".into(), "1".into())]).digest());
        assert_ne!(Added::default().with("r", [vec![Value::Int(1)]]).digest(), Added::default().with("r", [vec![Value::Str("1".into())]]).digest());
        assert_eq!(added.digest(), Added::settings(&[("a".into(), "1".into()), ("b".into(), "x\ty".into())]).digest());
        // The dialect: a program's language, and part of the key.
        let tsx = Added::of(&[], &serde_json::json!({"dialect": "tsx"}));
        assert!(!tsx.is_empty());
        let mut named = front::Program { settings: vec![("language".into(), "typescript".into()), ("policy".into(), "\"p\"".into())], ..Default::default() };
        tsx.add_to(&mut named);
        assert_eq!(named.settings, [("language".to_owned(), "tsx".to_owned()), ("policy".to_owned(), "\"p\"".to_owned())]);
        let mut unnamed = front::Program::default();
        tsx.add_to(&mut unnamed);
        assert!(unnamed.settings.is_empty());
        assert_ne!(tsx.digest(), Added::default().digest());
        assert_eq!(Added::of(&[], &serde_json::json!({})), Added::default());
    }
}
