//! `.rerail`'s `\\u` escapes: a string's as written, JSON's checked by tier 2 at the string (tier 1's `gating_tests` has the gated regions).

use rerail_cpu_block::native_cpu_block::State;
use rerail_compiler::block_native::Pass;

/// A `\\u` in a `.rerail` string is the string's, whatever its digits (decision 0655 step 13): a
/// checker reads a string to its quote, as tree-sitter does, and no executor refuses one. JSON's
/// `\\u`, four hex digits, is `json_bad`'s: tier 2's local check at the string, in a JSON
/// document's levels alone. Tier 1 makes no mask of the digits.
#[test]
fn rr_takes_a_strings_unicode_escapes_as_written() {
    let c = super::compiled().expect("compiles");
    assert!(!c.values.iter().any(|v| v.name == "h4"), "`h4` is tier 1's");
    assert_eq!(c.program.local_checks.len(), 1, "one local check: JSON's `\\u`");
    let json = |v: &str| format!(".policy {{\"a\": [1, \"{v}\"]}}");
    for (src, ok) in [(json("\\u0041"), true), (json("\\u{41}"), false), (json("\\u12"), false), ("r(x) :- x = \"\\u{41}\".".to_owned(), true)] {
        assert_eq!(super::compiled_tape(&src).is_some(), ok, "{src}");
        assert_eq!(super::tape(&src), super::engine_tape(&src), "{src}");
    }
    assert!(c.words.contains(" CHECK"), "tier 2 checks JSON's at the string");
    // Each escape as written, with the escape the engine reads in its place, of its length:
    // the same tape (a string leaf over the same bytes).
    let read = [("\\u0041", "\\u0041"), ("\\u{1F600}", "\\u{1F600}"), ("\\uD800", "\\u0041"), ("\\u12", "\\\\u1"), ("\\u{110000}", "\\u{100000}"), ("\\u{41}", "\\u{41}"), ("\\\\u12", "\\\\u12")];
    let check = |src: &str, engine: &str| {
        assert!(super::compiled_tape(src).is_some(), "{src}: taken");
        assert_eq!(super::tape(src), super::engine_tape(engine), "{src}");
    };
    for (esc, engine) in read {
        check(&format!("r(x) :- x = \"a{esc}\"."), &format!("r(x) :- x = \"a{engine}\"."));
        // A string past a word of 64 bytes, its escape in the second word.
        check(&format!("r(x) :- x = \"{}{esc}\".", "a".repeat(70)), &format!("r(x) :- x = \"{}{engine}\".", "a".repeat(70)));
    }
    // An escape at every place across two words' edges, its digits in the next word.
    for pad in 40..140 {
        for (esc, engine) in read {
            check(&format!("r(x) :- x = \"{}{esc}\".", "a".repeat(pad)), &format!("r(x) :- x = \"{}{engine}\".", "a".repeat(pad)));
        }
    }
}

#[test]
#[ignore = "a dump: run by hand"]
fn dump_rr_unicode() {
    let c = super::compiled().expect("compiles");
    let passes = c.native.passes_with(&c.native.pure());
    let names = ["uesc", "h4"];
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
