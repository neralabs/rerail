//! Exact finite trace costs for the proved bulk terminal operator.
use super::*;
use crate::code_rule_dsl::transducer::packed::terminal_repeat_proof::TerminalRepeat;

fn guard(op: u8) -> bool {
    matches!(
        op,
        KIND | FIELD | FLAGS | SIZE | FIRST | LAST | ABSENT_FIELD | SUPERTYPE
    )
}

fn path_costs(
    source: &mut String,
    words: &[u32],
    paths: &[Vec<usize>],
    matched: &str,
    failed: &str,
    extra: usize,
) {
    for path in paths {
        source.push_str("{uint failed=0u;\n");
        for (at, &pc) in path.iter().enumerate() {
            if guard(split(words[pc]).0) {
                writeln!(
                    source,
                    "if(!failed && !({}))failed={}u;",
                    guard_source(words, pc),
                    at + 1 + extra
                )
                .unwrap();
            }
        }
        writeln!(
            source,
            "if(failed)cost.{failed}=max(cost.{failed},failed);else cost.{matched}={}u;}}",
            path.len() + extra
        )
        .unwrap();
    }
}

pub(super) fn root(source: &mut String, index: u32, proof: &TerminalRepeat, words: &[u32]) {
    writeln!(source, "case {index}u:{{").unwrap();
    for &pc in &proof.guards {
        writeln!(
            source,
            "if(!({}))return uint2(0u,{}u);",
            guard_source(words, pc),
            pc + 1
        )
        .unwrap();
    }
    writeln!(
        source,
        "return uint2(1u,{}u);}}",
        proof.begin + 1 + usize::from(proof.children)
    )
    .unwrap();
}

pub(super) fn costs(source: &mut String, index: u32, proof: &TerminalRepeat, words: &[u32]) {
    writeln!(source, "case {index}u:{{uint4 cost=uint4(0u);").unwrap();
    path_costs(source, words, &proof.initial_work, "x", "y", 0);
    path_costs(source, words, &proof.repeating_work, "z", "w", 2);
    source.push_str("return cost;}\n");
}

pub(super) fn bounds(source: &mut String, index: u32, proof: &TerminalRepeat) {
    writeln!(
        source,
        "case {index}u:return uint2({}u,{}u);",
        proof.begin + 1 + usize::from(proof.children),
        1 + proof.suffix_work
    )
    .unwrap();
}
