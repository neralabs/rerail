//! D (RER-131, t31): every pack rule's token tests, sorted by what named-only tapes do to them
//! (`compile::token_tests`, which refuses a rule with a test nothing answers).

use crate::code_rule_dsl::compile::token_tests::{ORDER, Test, UNANSWERED, tests};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// Every token test of every pack rule (`packs/*/rules`, `packs/*/candidates/rules`): `each` gets the
/// pack's label, the rule's name, its language and the test, a test reached in several contexts
/// once, in its worst category.
fn census(mut each: impl FnMut(&str, &str, &str, Test)) {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join("../packs");
    let mut packs: Vec<_> = std::fs::read_dir(&root).unwrap().map(|e| e.unwrap().path()).filter(|p| p.is_dir()).collect();
    packs.sort();
    for pack in packs {
        let pack_name = pack.file_name().unwrap().to_string_lossy().into_owned();
        for sub in ["rules", "candidates/rules"] {
            let Ok(entries) = std::fs::read_dir(pack.join(sub)) else { continue };
            let mut files: Vec<_> = entries.map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect();
            files.sort();
            let label = if sub == "rules" { pack_name.clone() } else { format!("{pack_name}/candidates") };
            for file in files {
                let document: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
                let name = file.file_stem().unwrap().to_string_lossy().into_owned();
                for (language, test) in tests(&document) {
                    each(&label, &name, &language, test);
                }
            }
        }
    }
}

/// RER-131 D (ignored): every token test of every pack rule, its category, and where its token is
/// implied, to `RERAIL_TOKEN_READERS` (a TSV); a summary per pack and category on stderr.
#[test]
#[ignore = "census: RERAIL_TOKEN_READERS=<tsv>"]
fn token_readers() {
    let out = std::env::var("RERAIL_TOKEN_READERS").expect("RERAIL_TOKEN_READERS");
    let mut tsv = String::from("pack\trule\tlanguage\tcategory\tnegated\trelation\tregex\timplied (parent token)\tpath\n");
    // (pack, category) -> tests; (pack, rule) -> its worst category's rank.
    let mut tests_by: BTreeMap<(String, &str), usize> = BTreeMap::new();
    let mut worst: BTreeMap<(String, String), usize> = BTreeMap::new();
    census(|label, name, language, test| {
        let implied: Vec<String> = test.implied.iter().take(8).map(|(parent, token)| format!("{parent} {token}")).collect();
        let more = if test.implied.len() > 8 { format!(" +{}", test.implied.len() - 8) } else { String::new() };
        writeln!(tsv, "{label}\t{name}\t{language}\t{}\t{}\t{}\t{}\t{}{more}\t{}", test.category, test.negated, test.relation, test.regex, implied.join(", "), test.path).unwrap();
        *tests_by.entry((label.to_owned(), test.category)).or_default() += 1;
        let rank = ORDER.iter().position(|c| *c == test.category).unwrap();
        let entry = worst.entry((label.to_owned(), name.to_owned())).or_insert(rank);
        *entry = (*entry).min(rank);
    });
    let mut rules_by: BTreeMap<(String, &str), usize> = BTreeMap::new();
    for ((label, _), rank) in worst {
        *rules_by.entry((label, ORDER[rank])).or_default() += 1;
    }
    std::fs::write(&out, tsv).unwrap();
    let labels: BTreeSet<&String> = rules_by.keys().map(|(pack, _)| pack).collect();
    eprintln!("rules by worst category (tests in brackets):");
    for label in labels {
        let row: Vec<String> = ORDER
            .iter()
            .map(|c| format!("{c} {} ({})", rules_by.get(&(label.clone(), *c)).unwrap_or(&0), tests_by.get(&(label.clone(), *c)).unwrap_or(&0)))
            .collect();
        eprintln!("  {label}: {}", row.join(", "));
    }
    let total = |c: &str, map: &BTreeMap<(String, &str), usize>| map.iter().filter(|((_, k), _)| *k == c).map(|(_, n)| n).sum::<usize>();
    let row: Vec<String> = ORDER.iter().map(|c| format!("{c} {} ({})", total(c, &rules_by), total(c, &tests_by))).collect();
    eprintln!("  all: {}", row.join(", "));
}

/// The guard (t31): no pack rule tests, as a node or in a gap nothing answers, a token the tape
/// may leave out. The compiler refuses such a rule (`compile::token_tests`); this names every one
/// across the packs at once.
#[test]
fn no_rule_reads_a_token_the_tape_may_leave_out() {
    let mut unanswered = BTreeSet::new();
    census(|label, name, _, test| {
        if UNANSWERED.contains(&test.category) {
            unanswered.insert(format!("{label}/{name} {} {} at {}", test.category, test.regex, test.path));
        }
    });
    assert!(unanswered.is_empty(), "token tests nothing answers: {unanswered:#?}");
}

/// RER-131 (ignored): what the tape writer leaves out, over real files. For each file of
/// `RERAIL_FILES` (one path per line) it parses with tree-sitter and asks the writer's own filter
/// (`TokenGrammar::stored_children`) which children are kept. To `RERAIL_DROP_CENSUS` (a TSV): per
/// language, parent kind and anonymous token, how often it occurs and how often it is left out; on
/// stderr, per language, the files, named children, and anonymous children kept and left out.
#[test]
#[ignore = "census: RERAIL_FILES=<list> RERAIL_DROP_CENSUS=<tsv>"]
fn drop_census() {
    use ast_grep_language::LanguageExt;
    let list = std::fs::read_to_string(std::env::var("RERAIL_FILES").expect("RERAIL_FILES")).unwrap();
    let out = std::env::var("RERAIL_DROP_CENSUS").expect("RERAIL_DROP_CENSUS");
    let mut tally: BTreeMap<(String, String, String), (u64, u64, u64)> = BTreeMap::new();
    let mut totals: BTreeMap<String, [u64; 5]> = BTreeMap::new();
    for path in list.lines().filter(|l| !l.is_empty()) {
        let Some(label) = crate::code_rule_dsl::language::label_of_path(path) else { continue };
        let Some(lang) = crate::code_rule_dsl::language::dialect_by_label(label) else { continue };
        let Ok(source) = std::fs::read_to_string(path) else { continue };
        let Ok(grammar) = super::Grammar::new(lang) else { continue };
        let Some(filter) = grammar.kinds.grammar().and_then(|language| super::tokens::TokenGrammar::of(lang, language)) else { continue };
        let mut parser = tree_sitter::Parser::new();
        if parser.set_language(&lang.get_ts_language()).is_err() {
            continue;
        }
        let parsed = crate::code_rule_dsl::language::adapter::parse_input(lang, &source);
        let Some(tree) = parser.parse(parsed.as_bytes(), None) else { continue };
        let t = totals.entry(label.to_owned()).or_default();
        t[0] += 1;
        t[4] += source.len() as u64;
        let mut stack = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            let stored = filter.stored_children(node);
            let mut cursor = node.walk();
            for (i, child) in node.children(&mut cursor).enumerate() {
                let kept = stored.as_ref().is_none_or(|s| s[i]);
                if child.is_named() {
                    t[1] += 1;
                } else {
                    let e = tally.entry((label.to_owned(), node.kind().to_owned(), child.kind().to_owned())).or_default();
                    e.0 += 1;
                    if stored.is_none() {
                        e.2 += 1;
                    }
                    if kept {
                        t[2] += 1;
                    } else {
                        e.1 += 1;
                        t[3] += 1;
                    }
                }
                if kept {
                    stack.push(child);
                }
            }
        }
    }
    // whole: kept because the writer keeps all of the node's children (an error inside, or children
    // its automaton cannot walk).
    let mut tsv = String::from("language\tparent\ttoken\tseen\tdropped\twhole\n");
    for ((label, parent, token), (seen, dropped, whole)) in &tally {
        let token = token.replace('\t', "\\t").replace('\n', "\\n");
        writeln!(tsv, "{label}\t{parent}\t{token}\t{seen}\t{dropped}\t{whole}").unwrap();
    }
    std::fs::write(&out, tsv).unwrap();
    for (label, [files, named, kept, dropped, bytes]) in &totals {
        eprintln!("{label}\tfiles {files}\tbytes {bytes}\tnamed {named}\tanonymous kept {kept}\tanonymous dropped {dropped}");
    }
}
