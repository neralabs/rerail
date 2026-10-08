//! `convert`'s case conversions, ported from ast-grep-config 0.45.3 `transform/string_case.rs`
//! (MIT, Herrington Darkholme; see THIRD_PARTY_NOTICES) without its serde derives.

use std::ops::Range;

fn capitalize(string: &str) -> String {
    let mut chars = string.chars();
    if let Some(c) = chars.next() {
        c.to_uppercase().chain(chars).collect()
    } else {
        string.to_string()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum StringCase {
    LowerCase,
    UpperCase,
    Capitalize,
    CamelCase,
    SnakeCase,
    KebabCase,
    PascalCase,
}

impl StringCase {
    pub(crate) const NAMES: [(&'static str, Self); 7] = [
        ("lowerCase", Self::LowerCase),
        ("upperCase", Self::UpperCase),
        ("capitalize", Self::Capitalize),
        ("camelCase", Self::CamelCase),
        ("snakeCase", Self::SnakeCase),
        ("kebabCase", Self::KebabCase),
        ("pascalCase", Self::PascalCase),
    ];

    pub(crate) fn apply(&self, s: &str, seps: Option<&[Separator]>) -> String {
        match &self {
            Self::LowerCase => s.to_lowercase(),
            Self::UpperCase => s.to_uppercase(),
            Self::Capitalize => capitalize(s),
            Self::CamelCase => join_camel_case(split(s, seps)),
            Self::SnakeCase => join(split(s, seps), '_'),
            Self::KebabCase => join(split(s, seps), '-'),
            Self::PascalCase => split(s, seps).map(capitalize).collect(),
        }
    }
}

/// Separator to split a string: `user_accountName` -> `user`, `accountName`. It is rejoined
/// according to `StringCase`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Separator {
    CaseChange,
    Dash,
    Dot,
    Slash,
    Space,
    Underscore,
}

impl Separator {
    pub(crate) const NAMES: [(&'static str, Self); 6] = [
        ("caseChange", Self::CaseChange),
        ("dash", Self::Dash),
        ("dot", Self::Dot),
        ("slash", Self::Slash),
        ("space", Self::Space),
        ("underscore", Self::Underscore),
    ];
}

impl From<&[Separator]> for Delimiter {
    fn from(value: &[Separator]) -> Self {
        let mut delimiter = vec![];
        let mut state = CaseState::IgnoreCase;
        value.iter().for_each(|v| match v {
            Separator::CaseChange => state = CaseState::Lower,
            Separator::Dash => delimiter.push('-'),
            Separator::Dot => delimiter.push('.'),
            Separator::Slash => delimiter.push('/'),
            Separator::Space => delimiter.push(' '),
            Separator::Underscore => delimiter.push('_'),
        });
        Self {
            left: 0,
            right: 0,
            state,
            delimiter,
        }
    }
}

/// The case change between two characters, used where the separator is `CaseChange`.
#[derive(PartialEq, Eq)]
enum CaseState {
    Lower,
    OneUpper,
    /// Consecutive upper-case characters; the char is the last one, for the split range.
    MultiUpper(char),
    IgnoreCase,
}

struct Delimiter {
    left: usize,
    right: usize,
    state: CaseState,
    delimiter: Vec<char>,
}

impl Delimiter {
    fn all() -> Delimiter {
        Delimiter {
            left: 0,
            right: 0,
            state: CaseState::Lower,
            delimiter: vec!['-', '.', '/', ' ', '_'],
        }
    }

    fn delimit(&mut self, c: char) -> Option<Range<usize>> {
        let Self {
            left,
            right,
            state,
            delimiter,
        } = self;
        use CaseState::*;
        // A plain delimiter.
        if delimiter.contains(&c) {
            let range = *left..*right;
            *left = *right + 1;
            *right = *left;
            if *state != IgnoreCase {
                self.state = Lower;
            }
            return Some(range);
        }
        // A case delimiter, from lower case to upper case.
        if *state == Lower && c.is_uppercase() {
            let range = *left..*right;
            *left = *right;
            *right = *left + c.len_utf8();
            self.state = OneUpper;
            return Some(range);
        }
        // Consecutive upper case followed by lower case: XMLHttp -> XML Http.
        if let MultiUpper(last_char) = state
            && c.is_lowercase()
        {
            let new_left = *right - last_char.len_utf8();
            let range = *left..new_left;
            *left = new_left;
            *right += c.len_utf8();
            self.state = Lower;
            return Some(range);
        }
        *right += c.len_utf8();
        if *state == CaseState::IgnoreCase {
            return None;
        } else if c.is_lowercase() {
            self.state = Lower;
        } else if *state == Lower {
            self.state = OneUpper;
        } else {
            self.state = MultiUpper(c);
        }
        None
    }

    fn conclude(&mut self, len: usize) -> Option<Range<usize>> {
        let Self { left, right, .. } = self;
        if left < right && *right <= len {
            let range = *left..*right;
            *left = *right;
            Some(range)
        } else {
            None
        }
    }
}

/// Splits a string by the separators.
fn split<'a>(s: &'a str, seps: Option<&[Separator]>) -> impl Iterator<Item = &'a str> {
    let mut chars = s.chars();
    let mut delimiter = if let Some(seps) = seps {
        Delimiter::from(seps)
    } else {
        Delimiter::all()
    };
    std::iter::from_fn(move || {
        for c in chars.by_ref() {
            if let Some(range) = delimiter.delimit(c)
                && range.start != range.end
            {
                return Some(&s[range]);
            }
        }
        let range = delimiter.conclude(s.len())?;
        if range.start != range.end {
            Some(&s[range])
        } else {
            None
        }
    })
}

fn join<'a, I>(mut words: I, sep: char) -> String
where
    I: Iterator<Item = &'a str>,
{
    let mut result = String::new();
    if let Some(w) = words.next() {
        result.push_str(&w.to_lowercase());
    }
    for w in words {
        result.push(sep);
        result.push_str(&w.to_lowercase());
    }
    result
}

fn join_camel_case<'a, I>(words: I) -> String
where
    I: Iterator<Item = &'a str>,
{
    let mut result = String::new();
    for (i, word) in words.enumerate() {
        if i == 0 {
            result.push_str(&word.to_lowercase());
        } else {
            result.push_str(&capitalize(word));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cases_convert_as_upstream() {
        assert_eq!(StringCase::LowerCase.apply("aBc", None), "abc");
        assert_eq!(StringCase::UpperCase.apply("aBc", None), "ABC");
        assert_eq!(StringCase::Capitalize.apply("aBc", None), "ABc");
        fn split_all(s: &str) -> Vec<&str> {
            split(s, None).collect()
        }
        assert_eq!(
            split_all("camelsLiveInTheDesert"),
            ["camels", "Live", "In", "The", "Desert"]
        );
        assert_eq!(split_all("x.com/hd_nvim"), ["x", "com", "hd", "nvim"]);
        assert_eq!(split_all("XMLHttpRequest"), ["XML", "Http", "Request"]);
        assert_eq!(split_all("whatHTML"), ["what", "HTML"]);
        fn by<'a>(s: &'a str, seps: &[Separator]) -> Vec<&'a str> {
            split(s, Some(seps)).collect()
        }
        assert_eq!(
            by("user_accountName", &[Separator::Underscore]),
            ["user", "accountName"]
        );
        assert_eq!(
            by("user_accountName", &[Separator::Space]),
            ["user_accountName"]
        );
        assert_eq!(
            by("user_accountName", &[Separator::CaseChange]),
            ["user_account", "Name"]
        );
        assert_eq!(
            StringCase::SnakeCase.apply("camelsLiveInTheDesert", None),
            "camels_live_in_the_desert"
        );
        assert_eq!(
            StringCase::PascalCase.apply("kebab-is-a-delicious-food", None),
            "KebabIsADeliciousFood"
        );
        assert_eq!(
            StringCase::CamelCase.apply("snakes_live_in_forests", None),
            "snakesLiveInForests"
        );
    }
}
