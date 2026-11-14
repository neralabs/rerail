//! A span pattern's prefilter as the facts read it: where a match can end over a source, marked by a program the
//! transducer compiles (its block programs, on the cores or the GPU). The facts define what they ask of it; the
//! transducer registers what compiles and launches one at load (`install_span_prefilter`). Without one, no pattern
//! is prefiltered and every span is the regex's.

use std::any::Any;
use std::sync::{Arc, OnceLock};

/// Where a batch of span programs ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RanOn {
    Metal,
    Cpu,
}

/// A compiled pattern's prefilter.
pub trait SpanEnds: Send + Sync + 'static {
    /// Bit `i` of the words: a match can end after byte `i` of `bytes`.
    fn ends(&self, bytes: &[u8]) -> Vec<u64>;
    /// The prefilter as its own type (the launcher reads it).
    fn into_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;
}

/// Compiles a pattern over sources (`lines` false) or over each line (`lines` true); None: the host's.
pub type Compile = fn(&str, bool) -> Option<Arc<dyn SpanEnds>>;
/// Ends over each of `sources` (`files`: (source, the pattern's index in the prefilters)), placed by price on the
/// cores or, when `device` and the executor has one, the GPU.
pub type Launch = fn(&rerail_accel::Executor, &[Arc<dyn SpanEnds>], &[Arc<String>], &[(usize, usize)], bool) -> Result<(Vec<Vec<u64>>, RanOn), String>;

static SPAN_PREFILTER: OnceLock<(Compile, Launch)> = OnceLock::new();

/// Registers what compiles and launches span prefilters (once, at load).
pub fn install_span_prefilter(compile: Compile, launch: Launch) {
    let _ = SPAN_PREFILTER.set((compile, launch));
}

/// `re` with each class of every character (`[\s\S]`, `[\w\W]`, `[\d\D]`, either way round) written `(?s-i:.)`, which
/// matches the same characters. Under `(?i)` the regex compiler case-folds a class range by range, and the class of
/// every character is all of Unicode: one `[\s\S]` in a `(?i)` pattern took 0.2 s to compile, per batch. A pattern
/// that turns Unicode off (`-u`), where `.` would be another set, is left as it is.
pub fn unfolded(re: &str) -> std::borrow::Cow<'_, str> {
    const WHOLE: [&str; 6] = ["[\\s\\S]", "[\\S\\s]", "[\\w\\W]", "[\\W\\w]", "[\\d\\D]", "[\\D\\d]"];
    if re.contains("-u") || !WHOLE.iter().any(|w| re.contains(w)) {
        return std::borrow::Cow::Borrowed(re);
    }
    let (mut out, bytes, mut i, mut depth) = (String::with_capacity(re.len()), re.as_bytes(), 0, 0usize);
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                // An escape and what it escapes, as they are (one character, which may be several bytes).
                let next = re[i + 1..].chars().next().map_or(0, char::len_utf8);
                out.push_str(&re[i..i + 1 + next]);
                i += 1 + next;
                continue;
            }
            b'[' if depth == 0 && let Some(w) = WHOLE.iter().find(|w| re[i..].starts_with(**w)) => {
                out.push_str("(?s-i:.)");
                i += w.len();
                continue;
            }
            b'[' => {
                depth += 1;
                out.push('[');
                i += 1;
                // A `]` first in a class (after `^`) is a literal.
                if bytes.get(i) == Some(&b'^') {
                    out.push('^');
                    i += 1;
                }
                if depth == 1 && bytes.get(i) == Some(&b']') {
                    out.push(']');
                    i += 1;
                }
                continue;
            }
            b']' if depth > 0 => depth -= 1,
            _ => {}
        }
        let c = re[i..].chars().next().expect("a character");
        out.push(c);
        i += c.len_utf8();
    }
    std::borrow::Cow::Owned(out)
}

/// `re`'s prefilter, when one is registered and the pattern compiles.
pub fn compile(re: &str, lines: bool) -> Option<Arc<dyn SpanEnds>> {
    SPAN_PREFILTER.get().and_then(|(c, _)| c(re, lines))
}

/// The prefilters' ends over the sources (see `Launch`); none registered, none compiled, so none to launch.
pub fn launch(ex: &rerail_accel::Executor, filters: &[Arc<dyn SpanEnds>], sources: &[Arc<String>], files: &[(usize, usize)], device: bool) -> Result<(Vec<Vec<u64>>, RanOn), String> {
    match SPAN_PREFILTER.get() {
        Some((_, l)) => l(ex, filters, sources, files, device),
        None if files.is_empty() => Ok((Vec::new(), RanOn::Cpu)),
        None => Err("span prefilter: none registered".into()),
    }
}

/// Whether a match can end in `(a, b]`, a span's exclusive end, given `ends`.
pub fn any_in(ends: &[u64], a: usize, b: usize) -> bool {
    // Bits a..b (a match ending at e is bit e - 1).
    let (mut i, end) = (a, b);
    while i < end {
        let (w, bit) = (i / 64, i % 64);
        let take = (64 - bit).min(end - i);
        let mask = if take == 64 { u64::MAX } else { ((1u64 << take) - 1) << bit };
        if ends.get(w).is_some_and(|x| x & mask != 0) {
            return true;
        }
        i += take;
    }
    false
}

#[cfg(test)]
mod unfolded_tests {
    use super::unfolded;

    #[test]
    fn a_class_of_every_character_is_any_character_and_nothing_else_changes() {
        assert_eq!(unfolded(r"(?i)CAST\([\s\S]{0,160}?x"), r"(?i)CAST\((?s-i:.){0,160}?x");
        assert_eq!(unfolded(r"a[\w\W]b[\D\d]"), r"a(?s-i:.)b(?s-i:.)");
        // Escaped, inside a class, or with Unicode off: as written.
        assert_eq!(unfolded(r"\[\s\S]"), r"\[\s\S]");
        assert_eq!(unfolded(r"[a[\s\S]]"), r"[a[\s\S]]");
        assert_eq!(unfolded(r"[]\s\S]"), r"[]\s\S]");
        assert_eq!(unfolded(r"(?-u)[\s\S]"), r"(?-u)[\s\S]");
        for (re, text) in [(r"(?i)a[\s\S]{0,3}B", "A\n\u{e9}b"), (r"(?i)x[\w\W]y", "X\u{1F600}Y")] {
            let (was, now) = (regex::Regex::new(re).unwrap(), regex::Regex::new(&unfolded(re)).unwrap());
            assert_eq!(was.find(text).map(|m| m.range()), now.find(text).map(|m| m.range()), "{re}");
        }
    }
}
