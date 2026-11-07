//! Necessary conditions a regex places on the whole text it matches (a leaf's text or a term),
//! derived from its HIR. Moved from `matcher/anchors/text.rs`: the in-memory anchor index and the
//! store's vocabulary (`content_store::vocabulary`) both use them.
use regex_syntax::hir::{
    Hir, Look,
    literal::{ExtractKind, Extractor},
};

const MAX_PATTERN_BYTES: usize = 16 * 1024;
const MAX_PREFIXES: usize = 64;
const MAX_PREFIX_CHARS: usize = 8;

/// Necessary conditions only: passing this probe never replaces regex matching.
/// All lengths are UTF-8 byte lengths, as in the regex engine's HIR properties.
#[derive(Clone, Debug, Default)]
pub(crate) struct TextProbe {
    min_len: usize,
    max_len: Option<usize>,
    prefixes: Option<Vec<String>>,
    /// A complete union of nonempty suffixes at the end of the text, when the end is anchored.
    suffixes: Option<Vec<String>>,
    /// Groups of literals the text contains at least one of: every match starts with one of a
    /// prefix group and ends with one of a suffix group, wherever it lies in the text.
    required: Vec<Vec<String>>,
}

impl TextProbe {
    pub(crate) fn compile(pattern: &str) -> Self {
        // Parsing and extraction are optional optimizations. Bound their work,
        // and leave authoritative syntax/size validation to the actual matcher.
        if pattern.len() > MAX_PATTERN_BYTES {
            return Self::default();
        }
        let Ok(hir) = regex_syntax::ParserBuilder::new()
            .nest_limit(64)
            .build()
            .parse(pattern)
        else {
            return Self::default();
        };
        let properties = hir.properties();
        let anchored_start = properties.look_set_prefix().contains(Look::Start);
        let anchored_end = properties.look_set_suffix().contains(Look::End);
        Self {
            min_len: properties.minimum_len().unwrap_or(0),
            // A match-length upper bound is a node-length upper bound only
            // when every branch consumes from the absolute start to the end.
            max_len: if anchored_start && anchored_end {
                properties.maximum_len()
            } else {
                None
            },
            prefixes: if anchored_start {
                affixes(&hir, ExtractKind::Prefix)
            } else {
                None
            },
            suffixes: if anchored_end {
                affixes(&hir, ExtractKind::Suffix)
            } else {
                None
            },
            // An anchored side is already the stronger condition.
            required: [
                (!anchored_start).then_some(ExtractKind::Prefix),
                (!anchored_end).then_some(ExtractKind::Suffix),
            ]
            .into_iter()
            .flatten()
            .filter_map(|kind| affixes(&hir, kind))
            .collect(),
        }
    }

    /// A complete union of nonempty prefixes at the start of the node text.
    /// None is unconstrained, including unanchored and multiline-only starts.
    pub(crate) fn prefixes(&self) -> Option<&[String]> {
        self.prefixes.as_deref()
    }

    pub(crate) fn allows(&self, text: &str) -> bool {
        text.len() >= self.min_len
            && self.max_len.is_none_or(|max| text.len() <= max)
            && self
                .prefixes
                .as_ref()
                .is_none_or(|prefixes| prefixes.iter().any(|prefix| text.starts_with(prefix)))
            && self
                .suffixes
                .as_ref()
                .is_none_or(|suffixes| suffixes.iter().any(|suffix| text.ends_with(suffix)))
            && self
                .required
                .iter()
                .all(|group| group.iter().any(|literal| text.contains(literal.as_str())))
    }

    /// The prefixes as half-open ranges of UTF-8 (memcmp) order: every allowed text lies in one.
    /// The upper bound is the prefix with its last character advanced; None is unbounded.
    pub(crate) fn prefix_ranges(&self) -> Option<Vec<(String, Option<String>)>> {
        let prefixes = self.prefixes.as_ref()?;
        Some(prefixes.iter().map(|prefix| (prefix.clone(), successor(prefix))).collect())
    }
}

/// The least string greater than every string that starts with `prefix`, in code point order
/// (UTF-8 byte order is code point order).
fn successor(prefix: &str) -> Option<String> {
    let mut chars: Vec<char> = prefix.chars().collect();
    while let Some(last) = chars.pop() {
        let next = (u32::from(last) + 1..=u32::from(char::MAX)).find_map(char::from_u32);
        if let Some(next) = next {
            chars.push(next);
            return Some(chars.into_iter().collect());
        }
    }
    None
}

fn affixes(hir: &Hir, kind: ExtractKind) -> Option<Vec<String>> {
    // The HIR extractor composes unions, concatenations and repetitions. Its
    // limits shorten literals or broaden to an infinite sequence, never drop
    // alternatives. Flags have already been resolved into HIR classes/looks.
    // Look assertions are relaxed to empty matches; only the separately proven
    // absolute-start (or -end) property permits using these literals as node
    // prefixes (suffixes). Unanchored, they bound where a match starts (ends).
    let suffix = matches!(kind, ExtractKind::Suffix);
    let sequence = Extractor::new()
        .kind(kind)
        .limit_class(MAX_PREFIXES)
        .limit_total(MAX_PREFIXES)
        .limit_repeat(MAX_PREFIX_CHARS)
        .limit_literal_len(MAX_PREFIX_CHARS * 4)
        .extract(hir);
    let literals = sequence.literals()?;
    if literals.is_empty() || literals.len() > MAX_PREFIXES {
        return None;
    }
    let mut prefixes = Vec::with_capacity(literals.len());
    for literal in literals {
        let bytes = literal.as_bytes();
        // The extractor truncates bytes, which can split a Unicode character: a prefix loses
        // its tail, a suffix its head. Keeping only complete characters weakens the constraint.
        let text = if suffix {
            let start = bytes.iter().position(|&byte| byte & 0xC0 != 0x80).unwrap_or(bytes.len());
            std::str::from_utf8(&bytes[start..]).ok()?
        } else {
            match std::str::from_utf8(bytes) {
                Ok(text) => text,
                Err(error) => std::str::from_utf8(&bytes[..error.valid_up_to()]).ok()?,
            }
        };
        let kept = if suffix {
            let skip = text.chars().count().saturating_sub(MAX_PREFIX_CHARS);
            &text[text.char_indices().nth(skip).map_or(text.len(), |(offset, _)| offset)..]
        } else {
            &text[..text
                .char_indices()
                .nth(MAX_PREFIX_CHARS)
                .map_or(text.len(), |(offset, _)| offset)]
        };
        if kept.is_empty() {
            // Even one empty alternative makes the entire union unconstrained.
            return None;
        }
        prefixes.push(kept.to_owned());
    }
    if suffix {
        prefixes.sort_unstable();
        prefixes.dedup();
        return Some(prefixes);
    }
    prefixes.sort_unstable();
    // Lexicographic ordering makes a prefix precede all of its extensions.
    // Compare with the last retained prefix so duplicate/covered vocabulary
    // ranges disappear without changing the union accepted by the probe.
    prefixes.dedup_by(|next, previous| next.starts_with(previous.as_str()));
    Some(prefixes)
}

#[cfg(test)]
mod tests {
    use super::TextProbe;
    use regex::Regex;

    fn assert_sound(pattern: &str, texts: &[String]) {
        let regex = Regex::new(pattern).unwrap();
        let probe = TextProbe::compile(pattern);
        for text in texts {
            if regex.is_match(text) {
                assert!(probe.allows(text), "{pattern:?} rejected {text:?}");
                if let Some(prefixes) = probe.prefixes() {
                    assert!(
                        prefixes.iter().any(|prefix| text.starts_with(prefix)),
                        "{pattern:?} excluded {text:?} from the prefix index"
                    );
                }
                if let Some(ranges) = probe.prefix_ranges() {
                    assert!(
                        ranges.iter().any(|(lo, hi)| text.as_str() >= lo.as_str()
                            && hi.as_ref().is_none_or(|hi| text.as_str() < hi.as_str())),
                        "{pattern:?} excluded {text:?} from the prefix ranges"
                    );
                }
            }
        }
    }

    #[test]
    fn absolute_anchors_filter_without_treating_search_or_line_starts_as_prefixes() {
        let probe = TextProbe::compile(r"\A(?:ab|é)\z");
        assert!(probe.allows("ab"));
        assert!(probe.allows("é"));
        assert!(!probe.allows("ba"));
        assert!(!probe.allows("a"));
        assert!(!probe.allows("aba"));
        let texts = ["xxab", "x\nab", "x\r\nab", "ab\n", "ab longer"].map(str::to_owned);
        for pattern in ["ab", "(?m)^ab$", "(?mR)^ab$", "^ab", "ab$"] {
            assert_sound(pattern, &texts);
        }
    }

    #[test]
    fn optional_branches_flags_and_boundaries_keep_every_matching_alternative() {
        let texts = ["", "a", "ab", "b", "K", "k", "K", "é", "a\n", "!a!"].map(str::to_owned);
        for pattern in [
            r"\A(?:a|)",
            r"\A(?:a?b?)\z",
            r"\A(?:a*|b+)\z",
            r"\A(?i:k)\z",
            r"\A(?i:a)(?-i:b)?\z",
            r"\A\b(?:a|é)\b",
            r"\A\B(?:!|)",
            r"(?:\Aa|b)",
            r"\A(?:a|b\z)",
            r"\A(?m:a$)",
            r"\A(?-u:[ab])\z",
        ] {
            assert_sound(pattern, &texts);
        }
    }

    #[test]
    fn overlapping_alternatives_visit_each_matching_text_only_once() {
        let pattern = r"\A(?:foobar|foo|food|éclair|é)";
        let probe = TextProbe::compile(pattern);
        let prefixes = probe.prefixes().unwrap();
        for text in ["foo", "foobar", "food", "foodstuff", "é", "éclair"] {
            assert!(probe.allows(text));
            assert_eq!(
                prefixes
                    .iter()
                    .filter(|prefix| text.starts_with(prefix.as_str()))
                    .count(),
                1,
                "{text:?} would be visited through overlapping prefix ranges"
            );
        }
    }

    #[test]
    fn bounded_analysis_broadens_instead_of_losing_unicode_or_late_alternatives() {
        let alternatives = (0..80)
            .map(|index| format!("item{index:03}"))
            .collect::<Vec<_>>();
        assert_sound(
            &format!(r"\A(?:{})\z", alternatives.join("|")),
            &alternatives,
        );
        let unicode = "é".repeat(20);
        assert_sound(&format!(r"\A{unicode}\z"), std::slice::from_ref(&unicode));
        let split_character = format!("a{}", "é".repeat(20));
        assert_sound(
            &format!(r"\A{split_character}\z"),
            std::slice::from_ref(&split_character),
        );
        let long = "a".repeat(super::MAX_PATTERN_BYTES + 1);
        assert_sound(&format!(r"\A{long}\z"), std::slice::from_ref(&long));
        assert_sound(r"\Aa{100}\z", &["a".repeat(100)]);
    }

    #[test]
    fn generated_patterns_never_reject_a_regex_match() {
        let alphabet = ["a", "b", "k", "K", "é", "\n", "\r", "!"];
        let mut texts = vec![String::new()];
        let mut frontier = vec![String::new()];
        for _ in 0..3 {
            frontier = frontier
                .iter()
                .flat_map(|prefix| {
                    alphabet
                        .iter()
                        .map(move |suffix| format!("{prefix}{suffix}"))
                })
                .collect();
            texts.extend(frontier.iter().cloned());
        }
        for start in ["", r"\A", "^", "(?m)^", "(?mR)^"] {
            for atom in ["a", "(?:a|)", "[ab]", "(?i:k)", "é", r"\ba\b", "(?s:.)"] {
                for repetition in ["", "?", "{0,2}", "+"] {
                    for end in ["", r"\z", "$"] {
                        assert_sound(&format!("{start}(?:{atom}){repetition}{end}"), &texts);
                    }
                }
            }
        }
    }

    #[test]
    fn suffixes_and_required_literals_reject_without_losing_matches() {
        let probe = TextProbe::compile(r"(?:Error|Exception)\z");
        assert!(probe.allows("ValueError"));
        assert!(!probe.allows("ValueErrors"));
        let probe = TextProbe::compile(r"foo.*bar");
        assert!(probe.allows("xfooybarz"));
        assert!(!probe.allows("xbar"), "a match starts with foo");
        assert!(!probe.allows("nothing here"));
        let texts = ["Error", "xError", "Errorx", "éError", "foo", "afoob", "bar", "é", "ab"].map(str::to_owned);
        for pattern in [r"(?:Error|é)\z", r"(?i)error\z", r"foo|bar", r"(?m)foo$", r"\bfoo", r"[a-c]+é", r"(?:a|)b"] {
            assert_sound(pattern, &texts);
        }
    }

    #[test]
    fn prefix_ranges_hold_every_extension_and_nothing_below() {
        assert_eq!(super::successor("ab"), Some("ac".to_owned()));
        assert_eq!(super::successor("a\u{D7FF}"), Some("a\u{E000}".to_owned()));
        assert_eq!(super::successor("a\u{10FFFF}"), Some("b".to_owned()));
        assert_eq!(super::successor("\u{10FFFF}"), None);
        let probe = TextProbe::compile(r"\A(?:tar|é)");
        let ranges = probe.prefix_ranges().unwrap();
        for text in ["tar", "target", "tar\u{10FFFF}x", "é", "éclair"] {
            assert!(ranges.iter().any(|(lo, hi)| text >= lo.as_str() && hi.as_ref().is_none_or(|hi| text < hi.as_str())), "{text}");
        }
        for text in ["ta", "tas", "e", "f"] {
            assert!(!ranges.iter().any(|(lo, hi)| text >= lo.as_str() && hi.as_ref().is_none_or(|hi| text < hi.as_str())), "{text}");
        }
    }
}
