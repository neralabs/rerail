//! What a run adds to its programs (decision 0249): the run's settings (`env`, from `<root>/rerail.env` and
//! `--env`), the typing declarations (`declared_ret`), the tool calls it is given (`tool_call`, `tool_input`, from the
//! `toolCalls` option: `run-policies --tool-calls`, a test's `.tool_call`), and the dialect a program of a language of
//! several is run in (the `dialect` option, `tools::run_program`). They are rows and a setting, never text: each
//! program that declares a relation of them is given its rows once it is read, a program that names its language the
//! dialect (`add_to`), and every key the run makes of its programs is made of these too (`hash`), as it once was of
//! the text they were printed into.

use crate::code_rule_dsl::logic::front::{self, Value};
use crate::code_rule_dsl::logic::rr::lower::TOOL_CALLS;
use std::collections::BTreeMap;

/// Rows by relation, each row its values; and the dialect.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Added(BTreeMap<String, Vec<Vec<Value>>>, Option<String>);

/// The `toolCalls` option, a coding agent's tool calls as the product samples them (`[{"id", "tool", "input"}]`: the
/// call's id, the tool's name, its arguments as a JSON object), as the rows of [`TOOL_CALLS`]: `tool_call(id, tool)`
/// for each, and `tool_input(id, key, value)` for each string argument, a nested one by its dotted path. That is what
/// a JSON tool-call rule's `input.<path>` reads (`rules.rs`, `Field::InputPath`): no array is walked, a value that
/// is no string is no row, and a key with a dot in it is none either, for no path names it. An error says what is
/// not of that shape.
pub fn tool_call_rows(calls: &serde_json::Value) -> Result<[Vec<Vec<Value>>; 2], String> {
    let (mut calls_rows, mut inputs) = (Vec::new(), Vec::new());
    let mut ids = std::collections::BTreeSet::new();
    for (k, call) in calls.as_array().ok_or(SHAPE)?.iter().enumerate() {
        let (id, tool, input) = call_fields(k, call)?;
        if !ids.insert(id) {
            return Err(format!("{SHAPE}; two calls have the id {id:?}"));
        }
        calls_rows.push(vec![Value::Str(id.into()), Value::Str(tool.into())]);
        string_rows("", input, id, &mut inputs);
    }
    Ok([calls_rows, inputs])
}

/// The shape of the `toolCalls` option, as its errors name it.
const SHAPE: &str = "toolCalls: a JSON array of {\"id\": string, \"tool\": string, \"input\": object}";

/// The `k`-th call's id, tool and arguments, or what it lacks of them.
fn call_fields(k: usize, call: &serde_json::Value) -> Result<(&str, &str, &serde_json::Map<String, serde_json::Value>), String> {
    let field = |name: &str| call.get(name).ok_or_else(|| format!("{SHAPE}; call {k} has no {name}"));
    match (field("id")?.as_str(), field("tool")?.as_str(), field("input")?.as_object()) {
        (Some(id), Some(tool), Some(input)) => Ok((id, tool, input)),
        _ => Err(format!("{SHAPE}; call {k} is not")),
    }
}

/// A `tool_input(id, path, value)` row for each string of `input`, its path under `prefix`.
fn string_rows(prefix: &str, input: &serde_json::Map<String, serde_json::Value>, id: &str, out: &mut Vec<Vec<Value>>) {
    for (key, value) in input.iter().filter(|(key, _)| !key.contains('.')) {
        let path = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
        match value {
            serde_json::Value::String(text) => out.push(vec![Value::Str(id.into()), Value::Str(path.as_str().into()), Value::Str(text.as_str().into())]),
            serde_json::Value::Object(nested) => string_rows(&path, nested, id, out),
            _ => {}
        }
    }
}

/// The run's `toolCalls` option is of its shape (`tool_call_rows`), or the error that says how not: checked where a run
/// is first given its options (`corpus::run_with_options`, `corpus::run_read_handing`), so that every run reads it whole
/// (`Added::of`).
pub fn check_tool_calls(options: &serde_json::Value) -> Result<(), String> {
    options.get("toolCalls").map_or(Ok(()), |calls| tool_call_rows(calls).map(|_| ()))
}

impl Added {
    /// What a run of `options` over the root of `settings` adds: the settings' rows, the `toolCalls` option's rows
    /// (`tool_call_rows`; the run's entry checked its shape, `check_tool_calls`), the `dialect` option.
    pub(super) fn of(settings: &[(String, String)], options: &serde_json::Value) -> Added {
        let mut added = Added::settings(settings);
        if let Some(calls) = options.get("toolCalls") {
            let [calls, inputs] = tool_call_rows(calls).expect("the run's entry checked its toolCalls option (`check_tool_calls`)");
            added = added.with(TOOL_CALLS[0], calls).with(TOOL_CALLS[1], inputs);
        }
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
    #[allow(clippy::should_implement_trait, reason = "hashes into a SHA-256 digest (a key), not `std::hash::Hash`")]
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
                        // An atom with what it names (decision 0270): apart from an integer of the same value.
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

    /// A run's tool calls are the rows of `tool_call` and `tool_input` (`docs/rule-syntax/tool-calls.md`): each string
    /// argument by its dotted path, as a JSON rule's `input.<path>` reads it; an option of any other shape is an error
    /// where the run is first given it (`check_tool_calls`), and no run.
    #[test]
    fn tool_calls_are_rows_of_their_relations() {
        let s = |t: &str| Value::Str(t.into());
        let calls = serde_json::json!([
            {"id": "e1", "tool": "Bash", "input": {"command": "npm run fixtures:push", "timeout": 10, "env": {"CI": "1"}, "argv": ["a"], "a.b": "x"}},
            {"id": "e2", "tool": "Read", "input": {}}
        ]);
        let options = serde_json::json!({ "toolCalls": calls });
        assert_eq!(check_tool_calls(&options), Ok(()));
        let added = Added::of(&[], &options);
        let mut prog = front::Program::default();
        for rel in TOOL_CALLS {
            prog.decls.insert(rel.into(), Default::default());
        }
        added.add_to(&mut prog);
        let rows = |rel: &str| prog.facts[rel].iter().map(|r| r.to_vec()).collect::<Vec<_>>();
        assert_eq!(rows("tool_call"), [vec![s("e1"), s("Bash")], vec![s("e2"), s("Read")]]);
        assert_eq!(rows("tool_input"), [vec![s("e1"), s("command"), s("npm run fixtures:push")], vec![s("e1"), s("env.CI"), s("1")]]);
        // The calls are part of the run's key.
        assert_ne!(added.digest(), Added::default().digest());
        for bad in [serde_json::json!({}), serde_json::json!([{"id": "e1", "tool": "Bash"}]), serde_json::json!([{"id": 1, "tool": "Bash", "input": {}}]), serde_json::json!([{"id": "e", "tool": "Bash", "input": {}}, {"id": "e", "tool": "Bash", "input": {}}])] {
            assert!(check_tool_calls(&serde_json::json!({ "toolCalls": bad })).is_err(), "{bad}");
        }
        assert_eq!(check_tool_calls(&serde_json::json!({})), Ok(()));
        // The run's entry refuses a malformed option, before any run reads it (nothing is made at `dir`).
        let dir = std::env::temp_dir().join(format!("rerail-tool-calls-refused-{}", std::process::id()));
        let refused = super::super::run_with_options(&[".decl r(x: symbol)\n"], &dir, None, &dir.join("out"), "program", super::super::Binding::Cpu, 1 << 18, &Default::default(), &serde_json::json!({ "toolCalls": {} }));
        assert!(matches!(&refused, Err(super::super::Error::Plan(e)) if e.contains("toolCalls: a JSON array")), "{:?}", refused.err());
        assert!(!dir.exists());
    }
}
