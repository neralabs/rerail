//! Which rules of a tree section are shared by calls (`grammar::shared`), and what that does to
//! the state machine. A rule inlined has a copy of its inner states at each place that uses
//! it; called, its level has them once, and each use pays a push and a return when it runs.

use crate::grammar::{self, SHARED_STATES, saved_states};
use bitlex::syntax::Stmt;

/// The tree section of `src`.
fn tree(src: &str) -> Vec<Stmt> {
    let stmts = bitlex::syntax::parse(src).expect("parses");
    stmts.into_iter().find_map(|s| if let Stmt::Tree { body, .. } = s { Some(body) } else { None }).expect("a tree")
}

/// The token kinds the tests' grammars use.
const TOKENS: &[&str] = &["a", "b", "c", "d", "e", "f", "g", "h", "x", "y", "(", ")", "p", "q", "r"];

/// Ten tokens in a row: nine inner positions, one last.
const TEN: &str = "a b c d e f g h x y";

/// The rules `rules` (each `name = body` on a line) share, under a root that uses `root`.
fn shared(root: &str, rules: &[&str]) -> Vec<String> {
    let mut src = format!("tree t {{\n  node root = ({root})*\n");
    for r in rules {
        src.push_str(&format!("  {r}\n"));
    }
    src.push('}');
    let tokens: Vec<String> = TOKENS.iter().map(|t| t.to_string()).collect();
    let mut names = grammar::shared_names(&tree(&src), &tokens);
    names.sort();
    names
}

const NONE: [&str; 0] = [];

#[test]
fn the_states_a_call_saves() {
    // Inner states once instead of at each use, a state per call.
    assert_eq!(SHARED_STATES, 8);
    assert_eq!(saved_states(2, 10), 8);
    assert_eq!(saved_states(2, 9), 7);
    assert_eq!(saved_states(3, 5), 7);
    assert_eq!(saved_states(4, 4), 8);
    // Used once, or with nothing inner (a choice of single tokens): nothing.
    assert_eq!(saved_states(1, 100), 0);
    assert_eq!(saved_states(6, 0), 0);
}

#[test]
fn a_rule_whose_calls_save_enough_is_a_call() {
    // `'(' TEN ')'`: ten inner positions, two uses.
    let block = format!("node block = '(' {TEN} ')'");
    assert_eq!(shared("one | two", &["node one = p block", "node two = q block", &block]), ["block"]);
}

#[test]
fn a_rule_used_once_is_inlined() {
    let block = format!("node block = '(' {TEN} ')'");
    assert_eq!(shared("one | q", &["node one = p block", &block]), NONE);
}

#[test]
fn a_rule_whose_calls_save_too_little_is_inlined() {
    // Nine inner positions, two uses: seven states, not worth a push and a return a use.
    let block = format!("node block = {TEN}");
    assert_eq!(shared("one | two", &["node one = p block", "node two = q block", &block]), NONE);
    // JSON's `pair`, used twice in `pair (',' pair)*`: a few states.
    assert_eq!(shared("'(' [pair (b pair)*] ')'", &["node pair = x c value", "choice value = a | d | e"]), NONE);
}

#[test]
fn a_choice_of_single_tokens_is_inlined() {
    // Every position ends it: each place needs those states either way (JSON's `value`).
    assert_eq!(shared("p value | q value | r value", &["choice value = a | b | c | d | e | x"]), NONE);
}

#[test]
fn a_choice_whose_calls_save_enough_is_a_call() {
    let value = format!("choice value = '(' {TEN} ')' | e");
    assert_eq!(shared("p value | q value", &[&value]), ["value"]);
}

#[test]
fn a_rule_that_may_match_no_token_is_inlined() {
    // A call takes a token: an optional body stays inlined.
    let tail = format!("choice tail = [{TEN} '(' ')']");
    assert_eq!(shared("p tail r | q tail r", &[&tail]), NONE);
}

#[test]
fn a_rule_whose_first_token_merges_is_inlined() {
    let tail = format!("choice tail = merge 'p' {TEN} '(' ')'");
    assert_eq!(shared("p tail | q tail", &[&tail]), NONE);
}

#[test]
fn a_refusing_levels_rules_stay_inlined() {
    // What a level that refuses a mark reaches by inlining refuses it too: no call of its own.
    let block = format!("node block = '(' {TEN} ')'");
    let rules = ["node one = p doc", "node two = q block", "node three = r block", "node doc nested = '(' block block ')'", &block, "refuse x in doc"];
    assert_eq!(shared("one | two | three", &rules), NONE);
}

#[test]
fn rules_are_decided_from_the_inside_out() {
    // `inner` is a call; then `outer` holds its own ten tokens and two calls, eleven inner
    // positions: shared too.
    let inner = format!("node inner = '(' {TEN} ')'");
    let outer = format!("node outer = inner {TEN} inner");
    assert_eq!(shared("p outer | q outer", &[&outer, &inner]), ["inner", "outer"]);
    // With `inner` used once, `outer` holds its positions and is shared; `inner` inlined.
    let outer = format!("node outer = inner {TEN}");
    assert_eq!(shared("p outer | q outer", &[&outer, &inner]), ["outer"]);
}

/// The state machine of a tree section, as `grammar::text` prints it, kinds and fields numbered
/// in order of first use.
fn machine(src: &str) -> String {
    let ids = std::cell::RefCell::new(std::collections::HashMap::<String, u16>::new());
    let id = |n: &str| -> Result<u16, String> {
        let mut m = ids.borrow_mut();
        let k = m.len() as u16 + 1;
        Ok(*m.entry(n.to_owned()).or_insert(k))
    };
    let tokens: Vec<String> = TOKENS.iter().map(|t| t.to_string()).collect();
    let is_token = |n: &str| TOKENS.contains(&n);
    let reading = |n: &str| Err(format!("no reading {n}"));
    let g = grammar::Ids { kind: &id, field: &id, is_token: &is_token, tokens: &tokens, reading: &reading };
    let (states, arms, _) = grammar::build(&tree(src), &g).expect("builds");
    grammar::text(&states, &arms)
}

#[test]
fn levels_that_end_alike_share_their_states() {
    // Two nested levels, each a keyword and then the same ten tokens: the states after the
    // keyword act alike, in either level. One copy of the ten, and one level.
    let src = format!("tree t {{\n  node root = (one | two)*\n  node one nested = p {TEN}\n  node two nested = q {TEN}\n}}");
    let m = machine(&src);
    let states = m.lines().filter(|l| l.starts_with("state ")).count();
    // The root's start and the state after a call (alike: the root goes on the same way after
    // either), and one level's keyword and ten tokens, shared by both.
    assert_eq!(states, 2 + 11, "{m}");
    let levels: std::collections::BTreeSet<&str> = m.lines().filter(|l| l.starts_with("state ")).map(|l| l.split(' ').nth(3).expect("a level")).collect();
    assert_eq!(levels.len(), 2, "the root's and one other: {m}");
}

#[test]
fn levels_with_a_node_and_without_keep_apart() {
    // The same tails, but one level opens a node and the other does not: their ends differ (one
    // closes its node), so their states do too.
    let src = format!("tree t {{\n  node root = (one | two)*\n  node one nested = p {TEN}\n  choice two nested = q {TEN}\n}}");
    let m = machine(&src);
    let states = m.lines().filter(|l| l.starts_with("state ")).count();
    assert_eq!(states, 2 + 2 * 11, "{m}");
}

/// States of `machine(src)`, and its text.
fn states(src: &str) -> (usize, String) {
    let m = machine(src);
    (m.lines().filter(|l| l.starts_with("state ")).count(), m)
}

#[test]
fn remembered_words_share_their_states() {
    // Three words, each a variable that a wrap gives back its own kind: written once per word,
    // each word's state before its wrap acts apart (its tail, the same, is merged); remembered,
    // one state, the wrap naming the level's word's kind (`as word`). In `.rr` the copies
    // differ further on (a reading at an aggregate's `:`), so every state between does.
    let copies = format!("tree t {{\n  node root = (item)*\n  choice item nested = a as v (wrap w f as a {TEN}) | b as v (wrap w f as b {TEN}) | c as v (wrap w f as c {TEN})\n}}");
    let one = format!("tree t {{\n  node root = (item)*\n  choice item nested = a as v (wrap w f as a {TEN})\n}}");
    let remembered = format!("tree t {{\n  node root = (item)*\n  remember word\n  choice word = a | b | c\n  choice item nested = word as v (wrap w f as word {TEN})\n}}");
    let (copies, _) = states(&copies);
    let (one, _) = states(&one);
    let (remembered, m) = states(&remembered);
    assert_eq!(remembered, one, "{m}");
    assert_eq!(copies, one + 2, "{copies} states as copies, {one} as one word");
    // Each arm that takes a word remembers its own kind (three words, from the root's start
    // and from after an item); the wrap's moved node takes the level's word.
    assert_eq!(m.matches(" remember ").count(), 6, "{m}");
    assert!(m.contains(&format!(" as {} ", crate::stream_ir::REMEMBERED)), "{m}");
}

#[test]
fn a_rule_that_takes_a_remembered_word_stays_inlined() {
    // Used twice and long enough to be a call, but it takes a remembered word: a call's level
    // would give its caller's word back when it ends, so it stays inlined.
    let tail = format!("choice tail = word as v {TEN}");
    let mut src = format!("tree t {{\n  node root = (p tail (wrap w f as word q) | r tail (wrap w f as word q))*\n  remember word\n  choice word = a | b\n  {tail}\n}}");
    let tokens: Vec<String> = TOKENS.iter().map(|t| t.to_string()).collect();
    assert_eq!(grammar::shared_names(&tree(&src), &tokens), NONE);
    src = src.replace("remember word\n", "");
    assert_eq!(grammar::shared_names(&tree(&src), &tokens), ["tail"]);
}
