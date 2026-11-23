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
    pub fn of(settings: &[(String, String)], options: &serde_json::Value) -> Added {
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
    pub fn add_to(&self, prog: &mut (impl crate::code_rule_dsl::logic::program_image::fields::FieldsMut + ?Sized)) {
        if let Some(dialect) = &self.1 {
            let mut settings = prog.settings();
            if settings.iter().any(|(key, value)| key == "language" && value != dialect) {
                for (key, value) in &mut settings {
                    if key == "language" {
                        *value = dialect.clone();
                    }
                }
                prog.set_field(crate::code_rule_dsl::logic::program_facts::Field::Settings(&settings));
            }
        }
        for (rel, rows) in &self.0 {
            if prog.declares(rel) {
                let rows: Vec<front::List<front::Value>> = rows.iter().map(|r| front::List::from(r.clone())).collect();
                prog.add_facts(rel, &rows);
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

// cratify: `tests` is rerail-logic's (it mounts it: it reads the language, `rr::parse_version`, or the corpus's root)
