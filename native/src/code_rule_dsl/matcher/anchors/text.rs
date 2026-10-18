use regex_syntax::hir::{Hir, Look, literal::Extractor};

const MAX_PATTERN_BYTES: usize = 16 * 1024;
const MAX_PREFIXES: usize = 64;
const MAX_PREFIX_CHARS: usize = 8;

/// Necessary conditions only: passing this probe never replaces regex matching.
/// All lengths are UTF-8 byte lengths, as in the regex engine's HIR properties.
#[derive(Clone, Debug, Default)]
pub(super) struct TextProbe {
    min_len: usize,
    max_len: Option<usize>,
    prefixes: Option<Vec<String>>,
}

impl TextProbe {
    pub(super) fn compile(pattern: &str) -> Self {
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
                literal_prefixes(&hir)
            } else {
                None
            },
        }
    }

    /// A complete union of nonempty prefixes at the start of the node text.
    /// None is unconstrained, including unanchored and multiline-only starts.
    pub(super) fn prefixes(&self) -> Option<&[String]> {
        self.prefixes.as_deref()
    }

    pub(super) fn allows(&self, text: &str) -> bool {
        text.len() >= self.min_len
            && self.max_len.is_none_or(|max| text.len() <= max)
            && self
                .prefixes
                .as_ref()
                .is_none_or(|prefixes| prefixes.iter().any(|prefix| text.starts_with(prefix)))
    }
}

fn literal_prefixes(hir: &Hir) -> Option<Vec<String>> {
    // The HIR extractor composes unions, concatenations and repetitions. Its
    // limits shorten literals or broaden to an infinite sequence, never drop
    // alternatives. Flags have already been resolved into HIR classes/looks.
    // Look assertions are relaxed to empty matches; only the separately proven
    // absolute-start property permits using these literals as node prefixes.
    let sequence = Extractor::new()
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
        // The extractor truncates bytes, which can split a Unicode character.
        // Keeping only complete initial characters weakens the constraint.
        let text = match std::str::from_utf8(bytes) {
            Ok(text) => text,
            Err(error) => std::str::from_utf8(&bytes[..error.valid_up_to()]).ok()?,
        };
        let end = text
            .char_indices()
            .nth(MAX_PREFIX_CHARS)
            .map_or(text.len(), |(offset, _)| offset);
        if end == 0 {
            // Even one empty alternative makes the entire union unconstrained.
            return None;
        }
        prefixes.push(text[..end].to_owned());
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
}
