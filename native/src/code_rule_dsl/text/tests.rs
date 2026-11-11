use super::*;

fn regex(pattern: &str) -> Arc<Lowered> {
    lower(&Source::regex(pattern), Domain::ANY).unwrap_or_else(|e| panic!("{pattern}: {e:?}"))
}

fn production(pattern: &str) -> &'static str {
    regex(pattern).production
}

#[test]
fn productions_take_the_cheapest_exact_plan() {
    assert_eq!(production("(?s).*"), "always");
    assert_eq!(production(""), "always");
    assert_eq!(production(r"\b\B"), "never");
    assert_eq!(production("[^\\s\\S]"), "never");
    assert_eq!(production("^(?:on_commit|on_rollback)$"), "literals");
    assert_eq!(production("^foo"), "literals");
    assert_eq!(production("foo$"), "literals");
    assert_eq!(production("(foo|bar)"), "literals");
    assert_eq!(production("(?i)select"), "literals");
    assert_eq!(production(r"^_\w*cache\w*$"), "automaton");
    assert_eq!(production(r"\bfoo"), "automaton");
    assert_eq!(production(r"(.).*\1"), "backtracking");
    // `^` and `$` in every branch move out; in one branch only they stay.
    assert!(matches!(regex("^a$|^b$").plan, Plan::Literals { anchor: Anchor::Whole, .. }));
    assert!(matches!(regex("^a|b").plan, Plan::Automaton { .. }));
    // A prefix set keeps only the literals that decide it.
    assert_eq!(regex("^(?:a|ab|abc)").plan, Plan::Literals { anchor: Anchor::Start, literals: vec![b"a".to_vec()] });
}

#[test]
fn one_test_spelt_many_ways_is_one_key() {
    let key = |source: Source, domain: Domain| lower(&source, domain).unwrap().key.clone();
    let any = Domain::ANY;
    let exact = key(Source::regex("^(?:on_commit|on_rollback)$"), any);
    assert_eq!(exact, key(Source::of(Spelling::OneOf(vec!["on_rollback".into(), "on_commit".into()])), any));
    assert_eq!(exact, key(Source::regex("^(on_rollback|on_commit)$"), any));
    assert_eq!(exact, key(Source::regex("^on_commit$|^on_rollback$"), any));
    assert_eq!(key(Source::regex("^on_"), any), key(Source::of(Spelling::Prefix("on_".into())), any));
    assert_eq!(key(Source::regex("(?s)^.*foo.*$"), any), key(Source::of(Spelling::Contains("foo".into())), any));
    assert_eq!(key(Source::regex("Sql"), any), key(Source::of(Spelling::Contains("Sql".into())), any));
    // `.` stops at a line break, so `^.*x` is "x on the first line" in general text, and
    // "contains x" only where the text is one line.
    assert_ne!(key(Source::regex("^.*foo"), any), key(Source::of(Spelling::Contains("foo".into())), any));
    assert_eq!(key(Source::regex("^.*foo"), Domain::LINE), key(Source::of(Spelling::Contains("foo".into())), Domain::LINE));
    // A glob over a name is its regex: `on_*` is a prefix.
    assert_eq!(key(Source::of(Spelling::Glob("on_*".into())), Domain::LINE), key(Source::of(Spelling::Prefix("on_".into())), Domain::LINE));
    assert_eq!(key(Source::of(Spelling::Glob("*_async".into())), Domain::LINE), key(Source::regex("_async$"), Domain::LINE));
}

/// A small deterministic generator (no randomness crate in the tree).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const ALPHABET: &[u8] = b"ab_c.x \n(SQLsql)";

fn text(rng: &mut Lcg, len: usize) -> Vec<u8> {
    (0..len).map(|_| ALPHABET[rng.below(ALPHABET.len() as u64) as usize]).collect()
}

const PATTERNS: &[&str] = &[
    "ab|b",
    "a.b",
    "^a",
    "b$",
    "^ab$",
    "^(?:a|b|ab)$",
    "(?i)sql",
    r"\bab",
    r"(?m)^x",
    r"a\w*c",
    "^_.*c$",
    "(?s)a.*x",
    "x+",
    r"\(.*\)",
    "c$|^a",
    "^$",
    "(?s).*",
    r"\b\B",
    "[ab]{2,3}",
];

#[test]
fn the_vocabulary_engine_agrees_with_each_regex() {
    let mut rng = Lcg(7);
    let terms = (0..400).map(|_| {
        let len = rng.below(9) as usize;
        text(&mut rng, len)
    }).collect::<Vec<_>>();
    let lowered = PATTERNS.iter().map(|p| regex(p)).collect::<Vec<_>>();
    let plans = lowered.iter().map(|l| &l.plan).collect::<Vec<_>>();
    let refs = terms.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let (found, undecided) = vocabulary::evaluate(&plans, &refs);
    for (p, pattern) in PATTERNS.iter().enumerate() {
        assert!(!undecided[p]);
        let truth = regex::bytes::Regex::new(pattern).unwrap();
        let want = refs.iter().enumerate().filter(|(_, t)| truth.is_match(t)).map(|(i, _)| i as u32).collect::<Vec<_>>();
        assert_eq!(found[p], want, "{pattern} ({})", lowered[p].production);
        for (i, t) in refs.iter().enumerate() {
            assert_eq!(lowered[p].compiled().is_match(t), Some(truth.is_match(t)), "{pattern} on {:?}", String::from_utf8_lossy(t));
            let _ = i;
        }
    }
}

/// A random tree of spans over `len` bytes: each node's children split part of its span.
fn spans(rng: &mut Lcg, len: usize) -> Vec<Range<usize>> {
    let mut out = vec![0..len];
    let mut at = 0;
    while at < out.len() && out.len() < 400 {
        let span = out[at].clone();
        at += 1;
        if span.len() < 2 {
            continue;
        }
        let mut cursor = span.start;
        while cursor < span.end && out.len() < 400 {
            let start = cursor + rng.below(3) as usize;
            let end = (start + 1 + rng.below(span.len() as u64 / 2 + 1) as usize).min(span.end);
            if start >= end {
                break;
            }
            out.push(start..end);
            cursor = end;
        }
    }
    out
}

use std::ops::Range;

/// Assertions read a node's text differently from the file's at the node's edges.
const ASSERTIONS: &[&str] = &[r"\bab\b", r"(?:\A|\.)ab(?:\.|\z)", r"\Bb", r"(?m)a$", r"(?m)^a.", r"\b", r"(?-u:\b)x", r"(?i)\bsql\b", r"\bc\w*"];

#[test]
fn the_scan_engine_agrees_with_each_node_s_own_text() {
    let mut rng = Lcg(11);
    for round in 0..60 {
        let len = 20 + rng.below(200) as usize;
        let mut file = text(&mut rng, len);
        // Some files are not ASCII: a Unicode word boundary's automaton gives up there.
        if round % 4 == 3 {
            file.extend_from_slice("é".as_bytes());
            file.extend(text(&mut rng, 10));
        }
        let len = file.len();
        let tree = spans(&mut rng, len);
        let candidates = (0..tree.len() as u32).collect::<Vec<_>>();
        let span = |n: u32| tree[n as usize].clone();
        let nodes = scan::Nodes { text: &file, len: tree.len(), candidates: &candidates, span: &span };
        for pattern in PATTERNS.iter().chain(ASSERTIONS) {
            let lowered = regex(pattern);
            let bits = scan::bits(&lowered, &nodes).expect("decided");
            let truth = regex::bytes::Regex::new(pattern).unwrap();
            for (n, s) in tree.iter().enumerate() {
                let got = bits[n / 64] >> (n % 64) & 1 != 0;
                assert_eq!(got, truth.is_match(&file[s.clone()]), "round {round}, {pattern} ({}), node {n} {:?}", lowered.production, String::from_utf8_lossy(&file[s.clone()]));
            }
        }
    }
}

#[test]
fn a_batch_agrees_with_each_plan_alone() {
    let mut rng = Lcg(13);
    let lowered = PATTERNS.iter().map(|p| regex(p)).chain(["^(?:ab|b)$", "^a", "x$", "(?:ab|a)b", r"a\w*c$", r"^a\w*", r"(?i)\bsql\w*$", r"^\(.*\)$"].iter().map(|p| regex(p))).collect::<Vec<_>>();
    let batch = scan::Batch::of(lowered.clone());
    for _ in 0..30 {
        let len = 20 + rng.below(200) as usize;
        let file = text(&mut rng, len);
        let tree = spans(&mut rng, len);
        // Every other node: the batch reads candidates only.
        let candidates = (0..tree.len() as u32).filter(|n| n % 2 == 0).collect::<Vec<_>>();
        let span = |n: u32| tree[n as usize].clone();
        let nodes = scan::Nodes { text: &file, len: tree.len(), candidates: &candidates, span: &span };
        for (lowered, bits) in lowered.iter().zip(batch.bits(&nodes)) {
            assert_eq!(bits, scan::bits(lowered, &nodes), "{:?}", lowered.plan);
        }
    }
}

#[test]
fn a_match_hidden_inside_another_still_marks_its_node() {
    // Leftmost-first scanning finds `ab` at 1 and resumes after it, missing `b` at 2.
    let file = b"xab";
    let tree = [0..3, 1..3, 2..3];
    let candidates = [0, 1, 2];
    let span = |n: u32| tree[n as usize].clone();
    let nodes = scan::Nodes { text: file, len: 3, candidates: &candidates, span: &span };
    let bits = scan::bits(&regex("ab|b"), &nodes).unwrap();
    assert_eq!(bits[0], 0b111);
}

#[test]
fn backreferences_run_once_per_string() {
    let lowered = regex(r"(.).*\1");
    let (found, _) = vocabulary::evaluate(&[&lowered.plan], &[b"abca", b"abc", b"xx"]);
    assert_eq!(found[0], vec![0, 2]);
}

/// Every regex of serverplan_quality's port lowers, and the productions they take (decision 0042's
/// census by form): `--ignored --nocapture` prints the distribution.
#[test]
fn serverplan_quality_s_regexes_lower() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../gate/data-layer/final-shape/data-layer-serverplan-quality.json");
    let document: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut patterns = std::collections::BTreeSet::new();
    fn walk(value: &serde_json::Value, out: &mut std::collections::BTreeSet<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, value) in map {
                    match (key.as_str(), value) {
                        ("regex" | "notRegex", serde_json::Value::String(s)) => {
                            out.insert(s.clone());
                        }
                        _ => walk(value, out),
                    }
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(|item| walk(item, out)),
            _ => {}
        }
    }
    walk(&document, &mut patterns);
    let mut by = std::collections::BTreeMap::<&str, usize>::new();
    for pattern in &patterns {
        let lowered = lower(&Source::regex(pattern.clone()), Domain::ANY).unwrap_or_else(|e| panic!("{pattern}: {e:?}"));
        *by.entry(lowered.production).or_default() += 1;
        let _ = lowered.compiled();
    }
    println!("{} distinct regexes: {by:?}", patterns.len());
    assert!(patterns.len() > 200);
    assert_eq!(production("(?s).*"), "always");
    // Most are literal sets in disguise.
    assert!(by.get("literals").copied().unwrap_or(0) * 2 > patterns.len(), "{by:?}");
}

/// A group of automata determinized once and read back from the cache matches what it matched when built, and what
/// each pattern alone matches (decision 0075). A second set of searchers reads every group; none is built again.
#[test]
fn searchers_read_from_the_cache_match_as_when_built() {
    #[derive(Default)]
    struct Memory(std::sync::Mutex<HashMap<String, Option<Vec<u8>>>>);
    impl vocabulary::DfaCache for Memory {
        fn get(&self, key: &str) -> Option<Option<Vec<u8>>> {
            self.0.lock().unwrap().get(key).cloned()
        }
        fn put(&self, key: &str, dfa: Option<&[u8]>) {
            self.0.lock().unwrap().insert(key.to_owned(), dfa.map(<[u8]>::to_vec));
        }
    }
    let mut rng = Lcg(11);
    let mut terms = (0..400).map(|_| {
        let len = rng.below(9) as usize;
        text(&mut rng, len)
    }).collect::<Vec<_>>();
    // Text the DFA may give up on (non-ASCII), asked of the lazy one instead.
    terms.extend(["é", "caché", "_écache", "ß_cache"].map(|t| t.as_bytes().to_vec()));
    let lowered = PATTERNS.iter().map(|p| regex(p)).collect::<Vec<_>>();
    let plans = lowered.iter().map(|l| &l.plan).collect::<Vec<_>>();
    let refs = terms.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let cache = Memory::default();
    let built = vocabulary::Searchers::new(&plans, Some(&cache));
    assert_eq!(built.compiling.read, 0);
    let read = vocabulary::Searchers::new(&plans, Some(&cache));
    assert_eq!(read.compiling.built, 0, "{:?}", read.compiling);
    assert_eq!(read.compiling.read + read.compiling.lazy, built.compiling.built + built.compiling.lazy);
    let (from_built, _) = built.evaluate(&refs);
    let (from_cache, _) = read.evaluate(&refs);
    assert_eq!(from_cache, from_built);
    for (p, pattern) in PATTERNS.iter().enumerate() {
        let truth = regex::bytes::Regex::new(pattern).unwrap();
        let want = refs.iter().enumerate().filter(|(_, t)| truth.is_match(t)).map(|(i, _)| i as u32).collect::<Vec<_>>();
        assert_eq!(from_cache[p], want, "{pattern} ({})", lowered[p].production);
    }
}

/// Decision 0075's measurement: thread CPU to determinize a group of word-shaped patterns, against reading it back.
#[test]
#[ignore = "a measurement (decision 0075), not a check"]
fn determinizing_a_group_costs_more_than_reading_it() {
    #[derive(Default)]
    struct Memory(std::sync::Mutex<HashMap<String, Option<Vec<u8>>>>);
    impl vocabulary::DfaCache for Memory {
        fn get(&self, key: &str) -> Option<Option<Vec<u8>>> {
            self.0.lock().unwrap().get(key).cloned()
        }
        fn put(&self, key: &str, dfa: Option<&[u8]>) {
            self.0.lock().unwrap().insert(key.to_owned(), dfa.map(<[u8]>::to_vec));
        }
    }
    let cpu = || {
        let (u, s) = crate::code_rule_dsl::logic::plan::corpus::thread_usage();
        u + s
    };
    let patterns = [
        r"(?i)select\s+\w+\s+from", r"[A-Z][a-z]{3,12}[0-9]{1,4}", r"\w+_[0-9]{2,6}x", r"(?i)^(?:todo|fixme|xxx)\b",
        r"[a-f0-9]{8}-[a-f0-9]{4}", r"(?i)password\s*=", r"\b[A-Z_]{4,}\b", r"^\s*#\s*type:",
        r"[0-9]+\.[0-9]+\.[0-9]+", r"(?i)https?://\w+", r"__\w+__", r"\w+\.\w+\(\)",
        r"^[a-z]+(?:_[a-z0-9]+)*$", r"(?i)\bsleep\s*\(", r"[<>]=?\s*[0-9]{2,}", r"\\[nrtx0-9]",
    ];
    // ASCII classes: Unicode `\w` and `\b` do not determinize within the limit, and such a group runs lazily.
    let ascii = patterns.iter().map(|p| format!("(?-u){}", p.replace(r"\b", ""))).collect::<Vec<_>>();
    let lowered = ascii.iter().map(|p| regex(p)).collect::<Vec<_>>();
    let plans = lowered.iter().map(|l| &l.plan).collect::<Vec<_>>();
    let cache = Memory::default();
    let at = cpu();
    let built = vocabulary::Searchers::new(&plans, Some(&cache));
    let build = cpu() - at;
    let at = cpu();
    let read = vocabulary::Searchers::new(&plans, Some(&cache));
    let reading = cpu() - at;
    let bytes: usize = cache.0.lock().unwrap().values().flatten().map(Vec::len).sum();
    println!("built {:?} in {build:.4} s CPU; read {:?} in {reading:.4} s CPU; {bytes} bytes stored", built.compiling, read.compiling);
}
