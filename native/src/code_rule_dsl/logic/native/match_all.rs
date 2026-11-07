//! `match_all("re", s)` over symbols (`core.rr`): one row per non-overlapping match of `re` anywhere in `s`, with
//! that match's named groups bound to their text. A match in which a group takes no part gives no row, as in `match`.
//!
//! The evaluator reads the distinct texts the rule asks about (its context relation, `rr::lower`) and inserts the
//! rows; the regex runs once per distinct text, as `match` runs once per distinct string.
//!
//! Over a `source` or `src_line` span the captures are spans, and the batch makes them (`rx_span`,
//! `facts::vtab::syntax::core_rr`); this is the symbol form.

use regex::Regex;

/// Every occurrence, its captures, and zero-based Unicode scalar offsets. Advance through each text once:
/// counting every prefix independently would make many matches quadratic in the text length.
pub(in crate::code_rule_dsl) fn occurrences<'s>(re: &Regex, groups: &[String], s: &'s str) -> Vec<(Vec<&'s str>, [i64; 3])> {
    let mut out = Vec::new();
    let (mut byte, mut chars) = (0, 0);
    for (ordinal, caps) in re.captures_iter(s).enumerate() {
        let m = caps.get(0).expect("a complete match");
        let start = chars + s[byte..m.start()].chars().count();
        let end = start + m.as_str().chars().count();
        byte = m.end();
        chars = end;
        let row: Option<Vec<&str>> = groups.iter().map(|g| caps.name(g).map(|m| m.as_str())).collect();
        if let Some(row) = row {
            out.push((row, [ordinal as i64, start as i64, end as i64]));
        }
    }
    out
}

/// Each match's groups' texts, in `groups`' order; matches missing a group are left out. Without groups, one empty
/// row when `re` matches anywhere.
pub(in crate::code_rule_dsl) fn rows<'s>(re: &Regex, groups: &[String], s: &'s str) -> Vec<Vec<&'s str>> {
    let mut out = Vec::new();
    for caps in re.captures_iter(s) {
        let row: Option<Vec<&str>> = groups.iter().map(|g| caps.name(g).map(|m| m.as_str())).collect();
        if let Some(row) = row {
            out.push(row);
        }
        if groups.is_empty() {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_match_binds_its_groups_and_a_missing_group_gives_no_row() {
        let re = Regex::new(r#""(?<k>[a-z_]+)": (?<v>-?[0-9]+)"#).unwrap();
        let g = ["k".to_owned(), "v".to_owned()];
        assert_eq!(rows(&re, &g, r#"{"a": 1, "b_c": -20, "d": "x"}"#), [vec!["a", "1"], vec!["b_c", "-20"]]);
        let re = Regex::new("(?<a>x)(?<b>y)?").unwrap();
        assert_eq!(rows(&re, &["a".to_owned(), "b".to_owned()], "xy x"), [vec!["x", "y"]]);
        assert_eq!(rows(&re, &["a".to_owned()], "xy x"), [vec!["x"], vec!["x"]]);
        assert_eq!(rows(&Regex::new("q").unwrap(), &[], "aqq").len(), 1);
        assert!(rows(&Regex::new("q").unwrap(), &[], "abc").is_empty());
    }
}
