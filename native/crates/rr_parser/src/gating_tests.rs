//! `.rr`'s `\\u` checks: tier 2's, at the string (tier 1's `gating_tests` has the gated regions).

use rerail_compiler::native_cpu_block::State;
use rerail_compiler::block_native::Pass;

/// `.rr`'s `\\u` check is tier 2's (`CHECK`, a local check at the string token): tier 1 makes
/// the escapes' mask alone, none of the digits' values; the check is the escape's ops.
#[test]
fn rr_checks_its_unicode_escapes_at_the_string() {
    let c = super::compiled().expect("compiles");
    for name in ["h4", "b_short", "b_four", "b_five", "b_six", "u_ok"] {
        assert!(!c.values.iter().any(|v| v.name == name), "`{name}` is still tier 1's");
    }
    // The escape's, and `json_bad`'s `\\u` part: checked in a JSON document's levels alone. The
    // third, a number's 19 digits (`long_numbers_go_to_the_engine`).
    assert_eq!(c.program.local_checks.len(), 3, "three local checks");
    let json = |v: &str| format!(".policy {{\"a\": [1, \"{v}\"]}}");
    for (src, ok) in [(json("\\u0041"), true), (json("\\u{41}"), false), (json("\\u12"), false), ("r(x) :- x = \"\\u{41}\".".to_owned(), true)] {
        assert_eq!(super::compiled_tape(&src).is_some(), ok, "{src}");
        assert_eq!(super::tape(&src), super::engine_tape(&src), "{src}");
    }
    assert!(c.words.contains(" CHECK"), "tier 2 checks at the string");
    // The words refuse a bad escape and take a good one, as the engine's tape says.
    for (src, ok) in [("r(x) :- x = \"\\u0041\".", true), ("r(x) :- x = \"\\u{1F600}\".", true), ("r(x) :- x = \"\\uD800\".", false), ("r(x) :- x = \"a\\u12\".", false), ("r(x) :- x = \"\\u{110000}\".", false)] {
        assert_eq!(super::compiled_tape(src).is_some(), ok, "{src}");
        assert_eq!(super::tape(src), super::engine_tape(src), "{src}");
    }
    // A string past a word of 64 bytes, its escape in the second word.
    let long = format!("r(x) :- x = \"{}\\uD800\".", "a".repeat(70));
    assert!(super::compiled_tape(&long).is_none());
    let long = format!("r(x) :- x = \"{}\\u0041\".", "a".repeat(70));
    assert!(super::compiled_tape(&long).is_some());
    // The check word-parallel: an escape at every place across two words' edges, its digits
    // in the next word, good and bad, as the engine's tape says.
    for pad in 40..140 {
        for esc in ["\\u0041", "\\uD800", "\\u12", "\\u{41}", "\\u{110000}", "\\\\u12"] {
            let src = format!("r(x) :- x = \"{}{esc}\".", "a".repeat(pad));
            assert_eq!(super::tape(&src), super::engine_tape(&src), "{src}");
        }
    }
}

#[test]
#[ignore = "a dump: run by hand"]
fn dump_rr_unicode() {
    let c = super::compiled().expect("compiles");
    let passes = c.native.passes_with(&c.native.pure());
    let names = ["uesc", "h4", "u_four", "u_brace", "b_short", "b_four", "b_six", "u_ok"];
    for (k, pass) in passes.passes.iter().enumerate() {
        let Pass::Forward { body, .. } = pass else { continue };
        let origin = &passes.origins[k];
        let plan = State::of(body).plan;
        let local = |name: &str| c.values.iter().find(|v| v.name == name).and_then(|v| origin.iter().position(|&o| o as usize == usize::from(v.op)));
        for name in names {
            let Some(x) = local(name) else { continue };
            let region: Vec<usize> = plan.regions.iter().enumerate().filter(|(_, r)| r.members.contains(&x) || r.vectors.contains(&x)).map(|(i, _)| i).collect();
            let users: Vec<String> = body.insts.iter().enumerate().filter(|(_, i)| i.operands().contains(&x)).map(|(u, i)| format!("%{u} {i:?} lane {} mask {} region {:?}", plan.lanes[u], plan.mask[u], plan.region_of[u])).collect();
            eprintln!("pass {k} `{name}` %{x} {:?} lane {} mask {} regions {region:?} users {users:?}", body.insts[x], plan.lanes[x], plan.mask[x]);
        }
        for (i, r) in plan.regions.iter().enumerate() {
            eprintln!("pass {k} region {i}: gate %{} {:?}, {} members, {} vectors", r.gate, body.insts[r.gate], r.members.len(), r.vectors.len());
        }
    }
}
