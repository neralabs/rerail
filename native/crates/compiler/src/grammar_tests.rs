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
