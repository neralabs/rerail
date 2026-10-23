//! A `forbid.regex` clause's options and its matches in one file's text: the content gate,
//! match-text filters and the `lines` scan (line filters, `first`, `terminators`, the line state
//! machine and the indentation filter). Not part of the AST matcher; it lives under `ast/` for
//! ownership. Ported from the native engine (`native/src/code_rule_dsl/`):
//!
//! - `compile/mod.rs:589-705` (`compile_regex_options`) → [`options`], less `paths` (the caller's
//!   `policy::Scope`). Its document errors (shape, 64-pattern lists, non-empty patterns within
//!   `capabilities.json`'s `maxRegexPatternBytes` 16384) are this port's `Err`s.
//! - `compile/line_state.rs:8-149` (`pattern`, `state`, `indentation`) → [`pattern`], [`state`],
//!   [`indentation`].
//! - `query/mod.rs:21-120` (`regex_query_matches`) → [`matches`]: the whole-haystack content gate
//!   first (`include` all match, no `exclude` matches; a failed gate is no match at all); with
//!   `lines`, the haystack splits at `\n` (`terminators: lf`) or Python's `str.splitlines`
//!   terminators (`python`, the default; `\r\n` is one), a final empty line after the last
//!   terminator is not scanned, and each line in order: (1) advances the state machine and the
//!   indentation filter and stops there unless they emit; (2) stops unless every `lines.include`
//!   and no `lines.exclude` matches the line; (3) keeps each match of the pattern whose text every
//!   clause-level `include` and no `exclude` matches, and only the first such under `first`.
//!   Without `lines`, the haystack is one context and its matches pass the same text filters.
//! - `query/line_state.rs:1-60` (`State::allows`) → [`State::allows`].
//! - `eval/query.rs:750-763` and `workspace/evaluate.rs:722-735`: a match under `lines` reports
//!   the scan's own 0-based line counter ([`Match::line`]); without it the caller places the
//!   match through the file's line table.
//!
//! Refusals: none; every native regex option is ported. Not modelled, as nowhere in this
//! executor's regex leaf: native's match budget (`matcher_budget_exhausted`, a work deadline)
//! and its witness cap (`maxWitnesses` + 1 matches stop the scan as an incomplete result).
use regex::Regex;
use serde_json::{Map, Value as Json};
use std::ops::Range;

/// `capabilities.json` `limits.maxRegexPatternBytes`.
const MAX_REGEX_PATTERN_BYTES: usize = 16384;

/// A regex clause's options (native `model.rs` `RegexOptions`, without `paths`).
pub struct Options {
    /// Match-text filters: every `include` and no `exclude` matches the match's text.
    pub include: Vec<Regex>,
    pub exclude: Vec<Regex>,
    /// The `content` gate over the whole haystack.
    pub content_include: Vec<Regex>,
    pub content_exclude: Vec<Regex>,
    pub lines: Option<Lines>,
}

/// `lines` (native `RegexLines`).
pub struct Lines {
    /// Line filters: every `include` and no `exclude` matches the line's text.
    pub include: Vec<Regex>,
    pub exclude: Vec<Regex>,
    pub first: bool,
    pub lf_only: bool,
    pub state: Option<LineState>,
    pub indentation: Option<Indentation>,
}

/// `lines.state` (native `RegexLineState`): states are interned in order of appearance, the
/// initial state first (index 0).
pub struct LineState {
    pub initial: usize,
    pub emit: Vec<bool>,
    pub transitions: Vec<Transition>,
}

/// One `lines.state.transitions` entry (native `RegexLineTransition`).
pub struct Transition {
    pub from: usize,
    pub to: usize,
    pub pattern: Regex,
    pub min: u64,
    pub max: Option<u64>,
    pub odd: Option<bool>,
    pub skip: bool,
}

/// `lines.indentation` (native `RegexIndentation`).
pub struct Indentation {
    pub exclude_under: Regex,
    pub reset: Regex,
    pub ignore: Vec<Regex>,
}

/// One kept match: `range` within `context` (the whole haystack, or one line without its
/// terminator); `line` is the scan's 0-based line under `lines`, else `None`.
pub struct Match {
    pub range: Range<usize>,
    pub context: Range<usize>,
    pub line: Option<usize>,
}

fn object<'a>(value: &'a Json, at: &str) -> Result<&'a Map<String, Json>, String> {
    value.as_object().ok_or_else(|| format!("{at} must be an object"))
}

fn reject_unknown(object: &Map<String, Json>, allowed: &[&str], at: &str) -> Result<(), String> {
    match object.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(format!("{at}: unknown key `{k}`")),
        None => Ok(()),
    }
}

/// A non-empty (after trimming) pattern within `maxRegexPatternBytes` (native `json.rs`
/// `bounded_text`, then `Regex::new`).
fn pattern(value: Option<&Json>, at: &str) -> Result<Regex, String> {
    let text = value
        .and_then(Json::as_str)
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| format!("{at} must be a non-empty string"))?;
    if text.len() > MAX_REGEX_PATTERN_BYTES {
        return Err(format!("{at} is {} bytes; capabilities.limits.maxRegexPatternBytes allows at most {MAX_REGEX_PATTERN_BYTES}", text.len()));
    }
    crate::rx::get(text).map_err(|e| format!("{at}: {e}"))
}

/// An optional array of at most 64 patterns (`compile/mod.rs:594-618`).
fn patterns(value: Option<&Json>, at: &str) -> Result<Vec<Regex>, String> {
    let Some(value) = value else { return Ok(Vec::new()) };
    let values = value.as_array().ok_or_else(|| format!("{at} must be an array"))?;
    if values.len() > 64 {
        return Err(format!("{at} allows at most 64 patterns"));
    }
    values.iter().enumerate().map(|(i, v)| pattern(Some(v), &format!("{at}/{i}"))).collect()
}

fn required_str<'a>(value: &'a Map<String, Json>, key: &str, at: &str) -> Result<&'a str, String> {
    value.get(key).and_then(Json::as_str).ok_or_else(|| format!("{at}/{key} must be a string"))
}

/// A `forbid.regex` object's options (`compile/mod.rs:589-705`); `paths` is the caller's.
pub fn options(spec: &Json) -> Result<Options, String> {
    let spec = object(spec, "regex")?;
    let content_source = spec.get("source").and_then(Json::as_str) == Some("file.content");
    let lines = spec
        .get("lines")
        .map(|value| -> Result<Lines, String> {
            if !content_source {
                return Err("regex/lines requires file.content".into());
            }
            let at = "regex/lines";
            let value = object(value, at)?;
            reject_unknown(value, &["include", "exclude", "first", "terminators", "state", "indentation"], at)?;
            let first = match value.get("first") {
                None => false,
                Some(v) => v.as_bool().ok_or_else(|| format!("{at}/first must be boolean"))?,
            };
            Ok(Lines {
                include: patterns(value.get("include"), &format!("{at}/include"))?,
                exclude: patterns(value.get("exclude"), &format!("{at}/exclude"))?,
                first,
                lf_only: match value.get("terminators") {
                    None => false,
                    Some(t) => match t.as_str() {
                        Some("python") => false,
                        Some("lf") => true,
                        _ => return Err(format!("{at}/terminators must be python or lf")),
                    },
                },
                state: value.get("state").map(|v| state(v, &format!("{at}/state"))).transpose()?,
                indentation: value.get("indentation").map(|v| indentation(v, &format!("{at}/indentation"))).transpose()?,
            })
        })
        .transpose()?;
    let (content_include, content_exclude) = spec
        .get("content")
        .map(|v| -> Result<(Vec<Regex>, Vec<Regex>), String> {
            let at = "regex/content";
            if !content_source {
                return Err(format!("{at} requires file.content"));
            }
            let value = object(v, at)?;
            reject_unknown(value, &["include", "exclude"], at)?;
            Ok((patterns(value.get("include"), &format!("{at}/include"))?, patterns(value.get("exclude"), &format!("{at}/exclude"))?))
        })
        .transpose()?
        .unwrap_or_default();
    Ok(Options {
        include: patterns(spec.get("include"), "regex/include")?,
        exclude: patterns(spec.get("exclude"), "regex/exclude")?,
        content_include,
        content_exclude,
        lines,
    })
}

/// `lines.state` (`compile/line_state.rs:13-121`).
fn state(value: &Json, path: &str) -> Result<LineState, String> {
    let spec = object(value, path)?;
    reject_unknown(spec, &["initial", "emit", "transitions"], path)?;
    let initial = required_str(spec, "initial", path)?;
    let transitions = spec.get("transitions").and_then(Json::as_array).ok_or_else(|| format!("{path}/transitions must be an array"))?;
    if transitions.is_empty() || transitions.len() > 64 {
        return Err(format!("{path}/transitions requires 1 to 64 transitions"));
    }
    let mut names = vec![initial.to_owned()];
    let mut intern = |name: &str| -> Result<usize, String> {
        if name.is_empty() || name.len() > 64 {
            return Err(format!("{path}: state names require 1 to 64 bytes"));
        }
        if let Some(at) = names.iter().position(|known| known == name) {
            return Ok(at);
        }
        if names.len() >= 16 {
            return Err(format!("{path} allows at most 16 states"));
        }
        names.push(name.to_owned());
        Ok(names.len() - 1)
    };
    intern(initial)?;
    let transitions = transitions
        .iter()
        .enumerate()
        .map(|(i, value)| -> Result<Transition, String> {
            let at = format!("{path}/transitions/{i}");
            let transition = object(value, &at)?;
            reject_unknown(transition, &["from", "to", "pattern", "count", "skip"], &at)?;
            let from = intern(required_str(transition, "from", &at)?)?;
            let to = intern(required_str(transition, "to", &at)?)?;
            let pattern = pattern(Some(transition.get("pattern").unwrap_or(&Json::Null)), &format!("{at}/pattern"))?;
            let (mut min, mut max, mut odd) = (1, None, None);
            if let Some(value) = transition.get("count") {
                let cat = format!("{at}/count");
                let count = object(value, &cat)?;
                reject_unknown(count, &["min", "max", "parity"], &cat)?;
                let number = |key: &str| -> Result<Option<u64>, String> {
                    count.get(key).map(|v| v.as_u64().ok_or_else(|| format!("{cat}/{key} must be non-negative"))).transpose()
                };
                min = number("min")?.unwrap_or(1);
                max = number("max")?;
                if max.is_some_and(|max| min > max) {
                    return Err(format!("{cat}: min exceeds max"));
                }
                odd = match count.get("parity") {
                    None => None,
                    Some(p) => match p.as_str() {
                        Some("odd") => Some(true),
                        Some("even") => Some(false),
                        _ => return Err(format!("{cat}/parity must be odd or even")),
                    },
                };
            }
            let skip = match transition.get("skip") {
                None => false,
                Some(Json::Bool(v)) => *v,
                _ => return Err(format!("{at}/skip must be boolean")),
            };
            Ok(Transition { from, to, pattern, min, max, odd, skip })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let emit_names = spec.get("emit").and_then(Json::as_array).ok_or_else(|| format!("{path}/emit must be an array"))?;
    if emit_names.is_empty() || emit_names.len() > 16 {
        return Err(format!("{path}/emit requires 1 to 16 states"));
    }
    let mut emit = vec![false; names.len()];
    for name in emit_names {
        let at = names.iter().position(|known| Some(known.as_str()) == name.as_str()).ok_or_else(|| format!("{path}/emit names an unknown state"))?;
        emit[at] = true;
    }
    Ok(LineState { initial: 0, emit, transitions })
}

/// `lines.indentation` (`compile/line_state.rs:123-149`).
fn indentation(value: &Json, path: &str) -> Result<Indentation, String> {
    let spec = object(value, path)?;
    reject_unknown(spec, &["excludeUnder", "reset", "ignore"], path)?;
    let read = |key: &str| pattern(Some(spec.get(key).unwrap_or(&Json::Null)), &format!("{path}/{key}"));
    let ignore = spec.get("ignore").and_then(Json::as_array).ok_or_else(|| format!("{path}/ignore must be an array"))?;
    if ignore.len() > 64 {
        return Err(format!("{path}/ignore allows at most 64 patterns"));
    }
    Ok(Indentation {
        exclude_under: read("excludeUnder")?,
        reset: read("reset")?,
        ignore: ignore.iter().enumerate().map(|(i, v)| pattern(Some(v), &format!("{path}/ignore/{i}"))).collect::<Result<_, _>>()?,
    })
}

/// One file's line-filter state (`query/line_state.rs`), fresh per scan.
struct State {
    state: usize,
    indentation: Option<usize>,
}

impl State {
    fn new(lines: &Lines) -> Self {
        Self { state: lines.state.as_ref().map_or(0, |s| s.initial), indentation: None }
    }

    /// Whether the current line emits; advances the indentation filter and the state machine.
    fn allows(&mut self, lines: &Lines, text: &str) -> bool {
        let mut emit = true;
        if let Some(indentation) = &lines.indentation {
            let width = text.chars().take_while(|ch| ch.is_whitespace()).count();
            emit = self.indentation.is_none_or(|prior| width <= prior);
            if !indentation.ignore.iter().any(|r| r.is_match(text)) {
                if indentation.reset.is_match(text) {
                    self.indentation = None;
                }
                if indentation.exclude_under.is_match(text) {
                    self.indentation = Some(self.indentation.map_or(width, |prior| prior.min(width)));
                }
            }
        }
        if let Some(state) = &lines.state {
            emit &= state.emit[self.state];
            for transition in &state.transitions {
                if transition.from != self.state {
                    continue;
                }
                let count = transition.pattern.find_iter(text).count() as u64;
                if count >= transition.min && transition.max.is_none_or(|max| count <= max) && transition.odd.is_none_or(|odd| (count % 2 == 1) == odd) {
                    self.state = transition.to;
                    emit &= !transition.skip;
                    break;
                }
            }
        }
        emit
    }
}

/// Python `str.splitlines` terminators besides `\n` (`query/mod.rs:87-101`).
fn python_terminator(ch: char) -> bool {
    matches!(ch, '\r' | '\u{b}' | '\u{c}' | '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

/// The clause's kept matches of `regex` in `haystack`, in order (`query/mod.rs:21-120`).
pub fn matches(regex: &Regex, haystack: &str, options: &Options, found: &mut dyn FnMut(Match)) {
    if !options.content_include.iter().all(|r| r.is_match(haystack)) || options.content_exclude.iter().any(|r| r.is_match(haystack)) {
        return;
    }
    let keep = |text: &str| options.include.iter().all(|r| r.is_match(text)) && !options.exclude.iter().any(|r| r.is_match(text));
    let Some(lines) = &options.lines else {
        for m in regex.find_iter(haystack) {
            if keep(m.as_str()) {
                found(Match { range: m.range(), context: 0..haystack.len(), line: None });
            }
        }
        return;
    };
    let mut state = State::new(lines);
    let mut scan = |context: Range<usize>, line: usize| {
        let text = &haystack[context.clone()];
        if !state.allows(lines, text) {
            return;
        }
        if !lines.include.iter().all(|r| r.is_match(text)) || lines.exclude.iter().any(|r| r.is_match(text)) {
            return;
        }
        for m in regex.find_iter(text) {
            if !keep(m.as_str()) {
                continue;
            }
            found(Match { range: m.range(), context: context.clone(), line: Some(line) });
            if lines.first {
                break;
            }
        }
    };
    let (mut start, mut line) = (0, 0);
    let mut chars = haystack.char_indices().peekable();
    while let Some((at, ch)) = chars.next() {
        if ch == '\n' || (!lines.lf_only && python_terminator(ch)) {
            scan(start..at, line);
            start = at + ch.len_utf8();
            if ch == '\r' && chars.peek().is_some_and(|(_, next)| *next == '\n') {
                start += 1;
                chars.next();
            }
            line += 1;
        }
    }
    if start != haystack.len() {
        scan(start..haystack.len(), line);
    }
}
