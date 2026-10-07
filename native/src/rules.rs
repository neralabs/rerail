use model2vec_rs::model::StaticModel;
use napi::{Error, Result, Status};
use napi_derive::napi;
use regex::Regex;
use serde_json::{Map, Value};
use std::cell::OnceCell;
use std::collections::HashSet;
use tokenizers::Tokenizer;

/// Mirrors `maxRegexPatternBytes` in `policies/executable-policy-dsl/capabilities.json`.
const MAX_REGEX_PATTERN_BYTES: usize = 16_384;
const MAX_CONDITION_DEPTH: usize = 16;

#[napi(object)]
pub struct NativeRuleInput {
    pub id: String,
    /// Similarity rules only: the text whose embedding samples are compared against.
    pub reference: Option<String>,
    pub operator: Option<String>,
    pub threshold: Option<f64>,
    /// Structural rules only: the rule's `match` condition as JSON.
    pub conditions: Option<String>,
    pub source: String,
    pub boundary: String,
    pub minimum_tokens: u32,
}

#[napi(object)]
pub struct NativeSample {
    pub source: String,
    pub boundary: String,
    pub text: String,
    /// A tool call's arguments as JSON; absent for every other sample, tool results included.
    pub input: Option<String>,
}

#[napi(object)]
pub struct CompileSummary {
    pub count: u32,
    pub dimensions: u32,
}

#[napi(object)]
pub struct NativeMatch {
    pub rule_id: String,
    /// `similarity` or `structural`; the similarity facts are present only for the former.
    pub kind: String,
    pub similarity: Option<f64>,
    pub operator: Option<String>,
    pub threshold: Option<f64>,
    pub token_count: Option<u32>,
}

/// The model-backed half of matching, behind a seam so structural rules can be shown never to reach it.
pub trait Encoder {
    fn embed(&self, texts: &[String]) -> Vec<Vec<f32>>;
    fn token_count(&self, text: &str) -> Result<usize>;
}

/// The kernel's model, tokenizer, and input-token cap.
pub struct ModelEncoder {
    pub model: StaticModel,
    pub tokenizer: Tokenizer,
    pub max_input_tokens: usize,
}

impl Encoder for ModelEncoder {
    fn embed(&self, texts: &[String]) -> Vec<Vec<f32>> {
        self.model
            .encode_with_args(texts, Some(self.max_input_tokens), texts.len())
    }

    fn token_count(&self, text: &str) -> Result<usize> {
        let encoding = self
            .tokenizer
            .encode(text, false)
            .map_err(|error| invalid(format!("failed to tokenize sample: {error}")))?;
        Ok(encoding.get_ids().len().min(self.max_input_tokens))
    }
}

#[derive(Clone, Copy)]
enum Operator {
    GreaterThanOrEqual,
    LessThanOrEqual,
}

impl Operator {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "gte" => Ok(Self::GreaterThanOrEqual),
            "lte" => Ok(Self::LessThanOrEqual),
            _ => Err(invalid(format!("unsupported cosine operator: {value}"))),
        }
    }

    fn matches(self, score: f64, threshold: f64) -> bool {
        match self {
            Self::GreaterThanOrEqual => score >= threshold,
            Self::LessThanOrEqual => score <= threshold,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::GreaterThanOrEqual => "gte",
            Self::LessThanOrEqual => "lte",
        }
    }
}

#[derive(PartialEq, Eq)]
enum Source {
    UserInput,
    AssistantText,
    Tool(String),
    /// `tool:*`, a rule source only: every tool.
    AnyTool,
}

impl Source {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "user-input" => Ok(Self::UserInput),
            "assistant-text" => Ok(Self::AssistantText),
            "tool:*" => Ok(Self::AnyTool),
            _ => value
                .strip_prefix("tool:")
                .filter(|name| !name.is_empty())
                .map(|name| Self::Tool(name.to_owned()))
                .ok_or_else(|| invalid(format!("unsupported sample source: {value}"))),
        }
    }

    fn parse_sample(value: &str) -> Result<Self> {
        match Self::parse(value)? {
            Self::AnyTool => Err(invalid("sample source must name one tool, not tool:*")),
            source => Ok(source),
        }
    }

    fn is_tool(&self) -> bool {
        matches!(self, Self::Tool(_) | Self::AnyTool)
    }

    fn selects(&self, sample: &Source) -> bool {
        match (self, sample) {
            (Self::AnyTool, Self::Tool(_)) => true,
            (rule, sample) => rule == sample,
        }
    }
}

#[derive(PartialEq, Eq)]
enum Boundary {
    Sentence,
    Message,
    Turn,
    ToolCall,
}

impl Boundary {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "sentence" => Ok(Self::Sentence),
            "message" => Ok(Self::Message),
            "turn" => Ok(Self::Turn),
            "tool-call" => Ok(Self::ToolCall),
            _ => Err(invalid(format!("unsupported sample boundary: {value}"))),
        }
    }
}

enum Field {
    Text,
    ToolName,
    Input,
    InputPath(Vec<String>),
}

impl Field {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "text" => Some(Self::Text),
            "tool.name" => Some(Self::ToolName),
            "input" => Some(Self::Input),
            _ => {
                let path: Vec<String> = value
                    .strip_prefix("input.")?
                    .split('.')
                    .map(str::to_owned)
                    .collect();
                path.iter()
                    .all(|segment| !segment.is_empty())
                    .then_some(Self::InputPath(path))
            }
        }
    }

    fn reads_tool(&self) -> bool {
        !matches!(self, Self::Text)
    }
}

enum Test {
    Eq(String),
    Contains(String),
    Regex(Regex),
}

impl Test {
    fn holds(&self, value: &str) -> bool {
        match self {
            Self::Eq(expected) => value == expected,
            Self::Contains(needle) => value.contains(needle.as_str()),
            Self::Regex(regex) => regex.is_match(value),
        }
    }
}

enum Condition {
    Leaf { field: Field, test: Test },
    All(Vec<Condition>),
    Any(Vec<Condition>),
    Not(Box<Condition>),
}

impl Condition {
    fn compile(value: &Value, path: &str, depth: usize) -> Result<Self> {
        if depth > MAX_CONDITION_DEPTH {
            return Err(invalid(format!(
                "{path}: conditions nest deeper than {MAX_CONDITION_DEPTH}"
            )));
        }
        let object = value
            .as_object()
            .ok_or_else(|| invalid(format!("{path} must be an object")))?;
        for key in ["all", "any"] {
            if let Some(items) = object.get(key) {
                only_key(object, key, path)?;
                let items = items
                    .as_array()
                    .filter(|items| !items.is_empty())
                    .ok_or_else(|| invalid(format!("{path}/{key} must be a non-empty array")))?;
                let conditions = items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| {
                        Self::compile(item, &format!("{path}/{key}/{index}"), depth + 1)
                    })
                    .collect::<Result<Vec<_>>>()?;
                return Ok(if key == "all" {
                    Self::All(conditions)
                } else {
                    Self::Any(conditions)
                });
            }
        }
        if let Some(inner) = object.get("not") {
            only_key(object, "not", path)?;
            return Ok(Self::Not(Box::new(Self::compile(
                inner,
                &format!("{path}/not"),
                depth + 1,
            )?)));
        }
        Self::compile_leaf(object, path)
    }

    fn compile_leaf(object: &Map<String, Value>, path: &str) -> Result<Self> {
        if let Some(key) = object
            .keys()
            .find(|key| !matches!(key.as_str(), "field" | "regex" | "eq" | "contains"))
        {
            return Err(invalid(format!("{path}: unknown field `{key}`")));
        }
        let field = object
            .get("field")
            .ok_or_else(|| invalid(format!("{path} must contain field, all, any, or not")))?;
        let field = field
            .as_str()
            .and_then(Field::parse)
            .ok_or_else(|| {
                invalid(format!(
                    "{path}/field: unsupported field {field}; use text, tool.name, input, or input.<path>"
                ))
            })?;
        let tests: Vec<&str> = ["regex", "eq", "contains"]
            .into_iter()
            .filter(|key| object.contains_key(*key))
            .collect();
        let [key] = tests[..] else {
            return Err(invalid(format!(
                "{path} must contain exactly one of regex, eq, or contains"
            )));
        };
        let text = object
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| invalid(format!("{path}/{key} must be a string")))?;
        let test = match key {
            "regex" => Test::Regex(
                compile_regex(text).map_err(|reason| invalid(format!("{path}/regex {reason}")))?,
            ),
            "eq" => Test::Eq(text.to_owned()),
            _ if text.is_empty() => {
                return Err(invalid(format!("{path}/contains must not be empty")));
            }
            _ => Test::Contains(text.to_owned()),
        };
        Ok(Self::Leaf { field, test })
    }

    fn reads_tool(&self) -> bool {
        match self {
            Self::Leaf { field, .. } => field.reads_tool(),
            Self::All(conditions) | Self::Any(conditions) => {
                conditions.iter().any(Self::reads_tool)
            }
            Self::Not(inner) => inner.reads_tool(),
        }
    }

    fn holds(&self, sample: &SampleView<'_>) -> Result<bool> {
        Ok(match self {
            Self::Leaf { field, test } => {
                sample.field(field)?.is_some_and(|value| test.holds(value))
            }
            Self::All(conditions) => {
                for condition in conditions {
                    if !condition.holds(sample)? {
                        return Ok(false);
                    }
                }
                true
            }
            Self::Any(conditions) => {
                for condition in conditions {
                    if condition.holds(sample)? {
                        return Ok(true);
                    }
                }
                false
            }
            Self::Not(inner) => !inner.holds(sample)?,
        })
    }
}

fn only_key(object: &Map<String, Value>, key: &str, path: &str) -> Result<()> {
    match object.keys().find(|other| other.as_str() != key) {
        Some(other) => Err(invalid(format!(
            "{path} must contain exactly one of field, all, any, or not; found `{key}` and `{other}`"
        ))),
        None => Ok(()),
    }
}

/// Compile a pattern in the policy DSL's regex dialect and size limit. The error is a reason for the
/// caller to prefix with where the pattern sits.
pub fn compile_regex(pattern: &str) -> std::result::Result<Regex, String> {
    if pattern.is_empty() {
        return Err("must not be empty".to_owned());
    }
    if pattern.len() > MAX_REGEX_PATTERN_BYTES {
        return Err(format!(
            "is {} bytes; maxRegexPatternBytes allows at most {MAX_REGEX_PATTERN_BYTES}",
            pattern.len()
        ));
    }
    Regex::new(pattern).map_err(|error| format!("is not a valid regex: {error}"))
}

/// What a structural condition can read from one sample; the input JSON is parsed at most once.
struct SampleView<'a> {
    text: &'a str,
    tool_name: Option<&'a str>,
    input: Option<&'a str>,
    parsed: OnceCell<Value>,
}

impl<'a> SampleView<'a> {
    /// Tool fields are visible only on a tool call: a tool sample that carries its arguments.
    fn new(source: &'a Source, sample: &'a NativeSample) -> Self {
        let tool_name = match (source, &sample.input) {
            (Source::Tool(name), Some(_)) => Some(name.as_str()),
            _ => None,
        };
        Self {
            text: &sample.text,
            tool_name,
            input: tool_name.and(sample.input.as_deref()),
            parsed: OnceCell::new(),
        }
    }

    fn field(&self, field: &Field) -> Result<Option<&str>> {
        Ok(match field {
            Field::Text => Some(self.text),
            Field::ToolName => self.tool_name,
            Field::Input => self.input,
            Field::InputPath(path) => {
                let Some(input) = self.input else {
                    return Ok(None);
                };
                if self.parsed.get().is_none() {
                    let value = serde_json::from_str(input)
                        .map_err(|error| invalid(format!("sample input is not JSON: {error}")))?;
                    let _ = self.parsed.set(value);
                }
                path.iter()
                    .try_fold(self.parsed.get().expect("input parsed"), |value, key| {
                        value.get(key)
                    })
                    .and_then(Value::as_str)
            }
        })
    }
}

enum Matcher {
    Similarity {
        vector: Vec<f32>,
        operator: Operator,
        threshold: f64,
        minimum_tokens: usize,
    },
    Structural {
        condition: Condition,
        /// Whether the condition reads `tool.name` or `input`; such a rule only sees tool calls.
        reads_tool: bool,
    },
}

struct CompiledRule {
    id: String,
    matcher: Matcher,
    source: Source,
    boundary: Boundary,
}

impl CompiledRule {
    fn evaluate(
        &self,
        view: &SampleView<'_>,
        embedded: Option<&Embedded>,
    ) -> Result<Option<NativeMatch>> {
        match &self.matcher {
            Matcher::Structural {
                condition,
                reads_tool,
            } => {
                let applies = !reads_tool || view.input.is_some();
                Ok((applies && condition.holds(view)?).then(|| NativeMatch {
                    rule_id: self.id.clone(),
                    kind: "structural".to_owned(),
                    similarity: None,
                    operator: None,
                    threshold: None,
                    token_count: None,
                }))
            }
            Matcher::Similarity {
                vector,
                operator,
                threshold,
                minimum_tokens,
            } => Ok(embedded
                .filter(|sample| sample.token_count >= *minimum_tokens)
                .map(|sample| (sample.token_count, cosine(&sample.vector, vector)))
                .filter(|(_, similarity)| operator.matches(*similarity, *threshold))
                .map(|(token_count, similarity)| NativeMatch {
                    rule_id: self.id.clone(),
                    kind: "similarity".to_owned(),
                    similarity: Some(similarity),
                    operator: Some(operator.as_str().to_owned()),
                    threshold: Some(*threshold),
                    token_count: Some(token_count as u32),
                })),
        }
    }
}

/// A sample's token count and embedding, computed only for similarity rules.
struct Embedded {
    token_count: usize,
    vector: Vec<f32>,
}

fn cosine(left: &[f32], right: &[f32]) -> f64 {
    f64::from(
        left.iter()
            .zip(right)
            .map(|(left, right)| left * right)
            .sum::<f32>(),
    )
    .clamp(-1.0, 1.0)
}

pub struct RuleSet {
    rules: Vec<CompiledRule>,
    dimensions: usize,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new(Status::InvalidArg, message.into())
}

enum Parsed {
    Similarity {
        reference: String,
        operator: Operator,
        threshold: f64,
        minimum_tokens: usize,
    },
    Structural(Condition),
}

fn parse_rule(input: NativeRuleInput) -> Result<(String, Parsed, Source, Boundary)> {
    let source = Source::parse(&input.source)?;
    let boundary = Boundary::parse(&input.boundary)?;
    let parsed = if input.conditions.is_some() {
        parse_structural(&input, &source, &boundary)?
    } else {
        parse_similarity(&input, &source)?
    };
    Ok((input.id, parsed, source, boundary))
}

fn parse_structural(
    input: &NativeRuleInput,
    source: &Source,
    boundary: &Boundary,
) -> Result<Parsed> {
    let id = &input.id;
    if input.reference.is_some() {
        return Err(invalid(format!(
            "rule {id} is structural and must not have a reference"
        )));
    }
    if input.operator.is_some() || input.threshold.is_some() {
        return Err(invalid(format!(
            "rule {id} is structural and must not have a cosine operator or threshold"
        )));
    }
    let conditions = input.conditions.as_deref().unwrap_or_default();
    let value: Value = serde_json::from_str(conditions)
        .map_err(|error| invalid(format!("rule {id} /match is not JSON: {error}")))?;
    let condition = Condition::compile(&value, "/match", 0)
        .map_err(|error| invalid(format!("rule {id} {}", error.reason)))?;
    if condition.reads_tool() && !source.is_tool() {
        return Err(invalid(format!(
            "rule {id} reads tool.name or input, so its sample source must be tool:<name> or tool:*"
        )));
    }
    if condition.reads_tool() && *boundary != Boundary::ToolCall {
        return Err(invalid(format!(
            "rule {id} reads tool.name or input, so its sample boundary must be tool-call"
        )));
    }
    Ok(Parsed::Structural(condition))
}

fn parse_similarity(input: &NativeRuleInput, source: &Source) -> Result<Parsed> {
    let id = &input.id;
    let reference = input
        .reference
        .clone()
        .filter(|reference| !reference.trim().is_empty())
        .ok_or_else(|| invalid(format!("rule {id} reference must not be empty")))?;
    if *source == Source::AnyTool {
        return Err(invalid(format!(
            "rule {id}: tool:* is supported only by structural rules"
        )));
    }
    let threshold = input
        .threshold
        .filter(|threshold| threshold.is_finite() && (-1.0..=1.0).contains(threshold))
        .ok_or_else(|| invalid(format!("rule {id} threshold must be from -1 through 1")))?;
    Ok(Parsed::Similarity {
        reference,
        operator: Operator::parse(input.operator.as_deref().unwrap_or_default())?,
        threshold,
        minimum_tokens: input.minimum_tokens as usize,
    })
}

impl RuleSet {
    pub fn empty(dimensions: usize) -> Self {
        Self {
            rules: Vec::new(),
            dimensions,
        }
    }

    fn compile(
        &self,
        encoder: &impl Encoder,
        inputs: Vec<NativeRuleInput>,
    ) -> Result<Vec<CompiledRule>> {
        let mut ids = HashSet::with_capacity(inputs.len());
        let parsed = inputs
            .into_iter()
            .map(|input| {
                if input.id.trim().is_empty() {
                    return Err(invalid("rule id must not be empty"));
                }
                if !ids.insert(input.id.clone()) {
                    return Err(invalid(format!("duplicate rule id: {}", input.id)));
                }
                parse_rule(input)
            })
            .collect::<Result<Vec<_>>>()?;

        let references: Vec<String> = parsed
            .iter()
            .filter_map(|(_, rule, _, _)| match rule {
                Parsed::Similarity { reference, .. } => Some(reference.clone()),
                Parsed::Structural(_) => None,
            })
            .collect();
        let embeddings = if references.is_empty() {
            Vec::new()
        } else {
            encoder.embed(&references)
        };
        if embeddings.len() != references.len() {
            return Err(invalid("embedding count did not match rule count"));
        }

        let mut embeddings = embeddings.into_iter();
        parsed
            .into_iter()
            .map(|(id, rule, source, boundary)| {
                let matcher = match rule {
                    Parsed::Structural(condition) => Matcher::Structural {
                        reads_tool: condition.reads_tool(),
                        condition,
                    },
                    Parsed::Similarity {
                        operator,
                        threshold,
                        minimum_tokens,
                        ..
                    } => Matcher::Similarity {
                        vector: self.checked_vector(
                            &id,
                            embeddings.next().expect("one embedding per reference"),
                        )?,
                        operator,
                        threshold,
                        minimum_tokens,
                    },
                };
                Ok(CompiledRule {
                    id,
                    matcher,
                    source,
                    boundary,
                })
            })
            .collect()
    }

    fn checked_vector(&self, id: &str, vector: Vec<f32>) -> Result<Vec<f32>> {
        if vector.is_empty() {
            return Err(invalid(format!("rule {id} produced an empty embedding")));
        }
        if vector.len() != self.dimensions {
            return Err(invalid(format!(
                "rule {id} embedding dimension {} does not match model dimension {}",
                vector.len(),
                self.dimensions
            )));
        }
        Ok(vector)
    }

    pub fn replace(
        &mut self,
        encoder: &impl Encoder,
        inputs: Vec<NativeRuleInput>,
    ) -> Result<CompileSummary> {
        let mut compiled = self.compile(encoder, inputs)?;
        compiled.sort_unstable_by(|left, right| left.id.cmp(&right.id));
        self.rules = compiled;
        Ok(self.summary())
    }

    pub fn upsert(
        &mut self,
        encoder: &impl Encoder,
        inputs: Vec<NativeRuleInput>,
    ) -> Result<CompileSummary> {
        let compiled = self.compile(encoder, inputs)?;
        let incoming: HashSet<&str> = compiled.iter().map(|rule| rule.id.as_str()).collect();
        self.rules
            .retain(|rule| !incoming.contains(rule.id.as_str()));
        self.rules.extend(compiled);
        self.rules
            .sort_unstable_by(|left, right| left.id.cmp(&right.id));
        Ok(self.summary())
    }

    pub fn remove(&mut self, ids: Vec<String>) -> u32 {
        let ids: HashSet<String> = ids.into_iter().collect();
        let previous = self.rules.len();
        self.rules.retain(|rule| !ids.contains(&rule.id));
        (previous - self.rules.len()) as u32
    }

    /// Match one sample. Structural rules run first and never tokenize or embed; the sample is
    /// tokenized and embedded once, and only when a similarity rule could still match it.
    pub fn match_sample(
        &self,
        encoder: &impl Encoder,
        sample: NativeSample,
    ) -> Result<Vec<NativeMatch>> {
        if sample.text.trim().is_empty() {
            return Err(invalid("sample text must not be empty"));
        }
        let source = Source::parse_sample(&sample.source)?;
        let boundary = Boundary::parse(&sample.boundary)?;
        let candidates: Vec<&CompiledRule> = self
            .rules
            .iter()
            .filter(|rule| rule.source.selects(&source) && rule.boundary == boundary)
            .collect();
        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        let view = SampleView::new(&source, &sample);
        let embedded = self.embed_sample(encoder, &sample.text, &candidates)?;
        let mut matches = Vec::new();
        for rule in candidates {
            matches.extend(rule.evaluate(&view, embedded.as_ref())?);
        }
        Ok(matches)
    }

    /// Tokenize and embed the sample only when some similarity candidate's token floor admits it.
    fn embed_sample(
        &self,
        encoder: &impl Encoder,
        text: &String,
        candidates: &[&CompiledRule],
    ) -> Result<Option<Embedded>> {
        let floor = candidates
            .iter()
            .filter_map(|rule| match rule.matcher {
                Matcher::Similarity { minimum_tokens, .. } => Some(minimum_tokens),
                Matcher::Structural { .. } => None,
            })
            .min();
        let Some(floor) = floor else {
            return Ok(None);
        };
        let token_count = encoder.token_count(text)?;
        if token_count < floor {
            return Ok(None);
        }
        let vector = encoder
            .embed(std::slice::from_ref(text))
            .into_iter()
            .next()
            .ok_or_else(|| invalid("sample produced no embedding"))?;
        if vector.len() != self.dimensions {
            return Err(invalid("sample embedding dimension changed"));
        }
        Ok(Some(Embedded {
            token_count,
            vector,
        }))
    }

    fn summary(&self) -> CompileSummary {
        CompileSummary {
            count: self.rules.len() as u32,
            dimensions: self.dimensions as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[derive(Default)]
    struct CountingEncoder {
        embeds: Cell<usize>,
        token_counts: Cell<usize>,
    }

    impl Encoder for CountingEncoder {
        fn embed(&self, texts: &[String]) -> Vec<Vec<f32>> {
            self.embeds.set(self.embeds.get() + 1);
            texts.iter().map(|_| vec![1.0, 0.0]).collect()
        }

        fn token_count(&self, _text: &str) -> Result<usize> {
            self.token_counts.set(self.token_counts.get() + 1);
            Ok(4)
        }
    }

    fn structural(id: &str, conditions: &str) -> NativeRuleInput {
        NativeRuleInput {
            id: id.to_owned(),
            reference: None,
            operator: None,
            threshold: None,
            conditions: Some(conditions.to_owned()),
            source: "tool:*".to_owned(),
            boundary: "tool-call".to_owned(),
            minimum_tokens: 1,
        }
    }

    fn similarity(id: &str) -> NativeRuleInput {
        NativeRuleInput {
            id: id.to_owned(),
            reference: Some("force push".to_owned()),
            operator: Some("gte".to_owned()),
            threshold: Some(-1.0),
            conditions: None,
            source: "tool:Bash".to_owned(),
            boundary: "tool-call".to_owned(),
            minimum_tokens: 1,
        }
    }

    fn call(command: &str) -> NativeSample {
        let input = serde_json::json!({ "command": command }).to_string();
        NativeSample {
            source: "tool:Bash".to_owned(),
            boundary: "tool-call".to_owned(),
            text: input.clone(),
            input: Some(input),
        }
    }

    #[test]
    fn structural_rules_never_tokenize_or_embed() {
        let encoder = CountingEncoder::default();
        let mut rules = RuleSet::empty(2);
        rules
            .replace(
                &encoder,
                vec![structural(
                    "no-force-push",
                    r#"{"field":"input.command","regex":"git push --force"}"#,
                )],
            )
            .unwrap();

        let fired = rules
            .match_sample(&encoder, call("git push --force"))
            .unwrap();
        let quiet = rules.match_sample(&encoder, call("git status")).unwrap();

        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].kind, "structural");
        assert!(fired[0].similarity.is_none() && fired[0].token_count.is_none());
        assert!(quiet.is_empty());
        assert_eq!(encoder.embeds.get(), 0, "structural rules must not embed");
        assert_eq!(
            encoder.token_counts.get(),
            0,
            "structural rules must not tokenize"
        );
    }

    #[test]
    fn similarity_rules_embed_once_per_sample_beside_structural_rules() {
        let encoder = CountingEncoder::default();
        let mut rules = RuleSet::empty(2);
        rules
            .replace(
                &encoder,
                vec![
                    similarity("similar"),
                    structural("exact", r#"{"field":"tool.name","eq":"Bash"}"#),
                ],
            )
            .unwrap();
        assert_eq!(encoder.embeds.get(), 1, "only the reference is embedded");

        let matches = rules.match_sample(&encoder, call("ls")).unwrap();

        let ids: Vec<&str> = matches.iter().map(|m| m.rule_id.as_str()).collect();
        assert_eq!(ids, ["exact", "similar"]);
        assert_eq!(encoder.embeds.get(), 2);
        assert_eq!(encoder.token_counts.get(), 1);
    }

    #[test]
    fn a_sample_no_rule_selects_costs_nothing() {
        let encoder = CountingEncoder::default();
        let mut rules = RuleSet::empty(2);
        rules
            .replace(&encoder, vec![similarity("similar")])
            .unwrap();
        let before = encoder.embeds.get();

        let sample = NativeSample {
            source: "user-input".to_owned(),
            boundary: "message".to_owned(),
            text: "hello".to_owned(),
            input: None,
        };
        assert!(rules.match_sample(&encoder, sample).unwrap().is_empty());
        assert_eq!(encoder.embeds.get(), before);
        assert_eq!(encoder.token_counts.get(), 0);
    }
}
