//! The source guard for decision 0239's readers: a run's outputs, the memo's entries and the kept records are read as
//! typed rows (`output::Outputs`, `Rows`, `reuse::decode_lists`), and no rule or fact is ever read back from text. So
//! product code splits no line on a tab, the separator the old `.csv` outputs and the text records used.
//!
//! The sites left read text that is not a run's rules or facts: each is in `ALLOWED`, with its reason and how many
//! times it may appear. A new site fails here by file and line; an allowance no longer needed fails too. Test code
//! (a `#[cfg(test)]` module, a `*tests.rs` file) is not read.

use crate::executors_guard_tests::{product_lines, repo, sources};

/// A split on a tab, in any of its spellings.
const SPLITS: [&str; 7] = ["split('\\t')", "split_once('\\t')", "rsplit_once('\\t')", "rsplit('\\t')", "split(\"\\t\")", "split_once(\"\\t\")", ", '\\t')"];

/// The files that may split on a tab, how many times, and why.
const ALLOWED: [(&str, usize, &str); 5] = [
    ("native/src/code_rule_dsl/logic/strings/git.rs", 1, "a consulted read's key (repository root, revision), made in memory and stored length-prefixed"),
    ("native/src/code_rule_dsl/logic/plan/corpus/inputs.rs", 2, "a `.input` file's text (a repository file the rule declares as input), and a consulted read's key (revision, path)"),
    // Two: the ledger compared, and its device cells kept where there is no Metal device (e946ff7205).
    ("native/src/code_rule_dsl/logic/plan/coverage.rs", 2, "the coverage ledger, a hand-kept document"),
    ("native/src/code_rule_dsl/typing/rounds.rs", 8, "the typing spike's fact dumps (T's research inputs), not a run's outputs"),
    ("native/src/code_rule_dsl/facts/ygraph/mod.rs", 2, "the y-graph's member lists and its recorded edges (fixtures of the replay)"),
];

/// Every lean parser's edit oracle (`native/crates/<language>_lean/src/oracle.rs`) reads a recorded edit script,
/// whose head line is a path, a cut and an edit apart by tabs: one split each.
const LEAN_ORACLE: (&str, &str, usize) = ("native/crates/", "_lean/src/oracle.rs", 1);

#[test]
fn no_rule_or_fact_is_read_from_text() {
    let root = repo();
    let mut bad = Vec::new();
    let mut seen = Vec::new();
    for f in sources() {
        let rel = f.strip_prefix(&root).unwrap_or(&f).display().to_string();
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        let at: Vec<usize> = product_lines(&text)
            .into_iter()
            .filter(|(_, l)| !l.trim_start().starts_with("//") && SPLITS.iter().any(|p| l.contains(p) && (!p.starts_with(',') || l.contains("splitn("))))
            .map(|(n, _)| n)
            .collect();
        if at.is_empty() {
            continue;
        }
        let oracle = rel.starts_with(LEAN_ORACLE.0) && rel.ends_with(LEAN_ORACLE.1);
        let allowed = ALLOWED.iter().map(|(f, n, _)| (*f, *n)).find(|(file, _)| *file == rel).or(oracle.then_some((LEAN_ORACLE.1, LEAN_ORACLE.2)));
        match allowed {
            Some((_, n)) if n == at.len() => seen.push(rel),
            Some((_, n)) => bad.push(format!("{rel}: {} tab splits, where its allowance is {n} (lines {at:?})", at.len())),
            None => bad.push(format!("{rel}:{}: a split on a tab", at.iter().map(usize::to_string).collect::<Vec<_>>().join(","))),
        }
    }
    for (file, _) in ALLOWED.iter().map(|(f, n, _)| (*f, *n)) {
        if !seen.iter().any(|s| s == file) && !bad.iter().any(|b| b.starts_with(file)) {
            bad.push(format!("{file}: its allowance is no longer needed: remove it"));
        }
    }
    assert!(
        bad.is_empty(),
        "text read where typed rows belong (decision 0239): read a run's outputs through `output::Outputs`, a record through `reuse::decode_lists`:\n{}",
        bad.join("\n")
    );
}
