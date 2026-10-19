//! Transducer pushdown (`sql::pushdown`): the programs it compiles, run by the node-set executor
//! (`Program::holds`, what the `program_match` terminal runs), agree with the transducer's path
//! executor (`Program::execute_roots`, the language's reference semantics) over the fixtures, for
//! randomly generated capture-free rules of every operator pushdown compiles. That the programs
//! agree with their SQL lowering is `differential::pushed_down_programs_agree_with_their_sql_lowering`.

use super::differential::tape;
use crate::code_rule_dsl::language::dialect_by_label;
use crate::code_rule_dsl::sql::plan::{Stop, Walk};
use crate::code_rule_dsl::sql::pushdown::{Rule, compile};
use crate::code_rule_dsl::tape::TapeDoc;
use crate::code_rule_dsl::transducer::NodeSet;
use std::collections::BTreeSet;

/// SplitMix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound.max(1) as u64) as usize
    }
}

struct Vocabulary {
    kinds: Vec<u16>,
    fields: Vec<u16>,
    regexes: Vec<regex::Regex>,
}

fn stop(rng: &mut Rng) -> Stop {
    if rng.below(2) == 0 {
        Stop::Neighbor
    } else {
        Stop::End
    }
}

fn random_rule(rng: &mut Rng, vocabulary: &Vocabulary, depth: usize) -> Rule {
    let leaf = depth == 0 || rng.below(3) == 0;
    if leaf {
        return match rng.below(4) {
            0 => Rule::Regex(vocabulary.regexes[rng.below(vocabulary.regexes.len())].clone()),
            1 => Rule::Kinds(
                (0..1 + rng.below(3))
                    .map(|_| vocabulary.kinds[rng.below(vocabulary.kinds.len())])
                    .collect(),
            ),
            _ => Rule::Kinds(vec![vocabulary.kinds[rng.below(vocabulary.kinds.len())]]),
        };
    }
    let sub = |rng: &mut Rng| Box::new(random_rule(rng, vocabulary, depth - 1));
    let items = |rng: &mut Rng| {
        (0..rng.below(4))
            .map(|_| random_rule(rng, vocabulary, depth - 1))
            .collect()
    };
    let field = |rng: &mut Rng| vocabulary.fields[rng.below(vocabulary.fields.len())];
    match rng.below(9) {
        0 => Rule::All(items(rng)),
        1 => Rule::Any(items(rng)),
        2 => Rule::Not(sub(rng)),
        3 => Rule::Walk(Walk::Has(stop(rng)), sub(rng)),
        4 => Rule::Walk(Walk::Inside(stop(rng)), sub(rng)),
        5 => {
            let stop = stop(rng);
            Rule::Walk(Walk::FieldHas(field(rng), stop), sub(rng))
        }
        6 => {
            let stop = stop(rng);
            Rule::Walk(Walk::FieldInside(field(rng), stop), sub(rng))
        }
        _ => {
            let walk = match rng.below(4) {
                0 => Walk::Has(Stop::Neighbor),
                1 => Walk::Has(Stop::End),
                2 => Walk::Inside(Stop::Neighbor),
                _ => Walk::Inside(Stop::End),
            };
            let min = rng.below(3);
            let max = (rng.below(2) == 0).then(|| min + rng.below(3));
            Rule::Count {
                walk,
                sub: sub(rng),
                min,
                max,
            }
        }
    }
}

fn vocabulary(docs: &[TapeDoc]) -> Vocabulary {
    let mut kinds = BTreeSet::new();
    let mut fields = BTreeSet::new();
    for doc in docs {
        for node in 0..doc.transducer_len() as u32 {
            kinds.insert(doc.transducer_kind(node));
            let field = doc.transducer_field(node);
            if field != 0 {
                fields.insert(field);
            }
        }
    }
    Vocabulary {
        kinds: kinds.into_iter().collect(),
        fields: fields.into_iter().collect(),
        regexes: ["^[a-z_]+$", "self", "\\(", "^[A-Z]", "=", "^def ", "return", "\\d"]
            .iter()
            .map(|source| regex::Regex::new(source).expect("a regex"))
            .collect(),
    }
}

#[test]
fn the_node_set_executor_agrees_with_the_path_executor() {
    let fixtures = [
        (
            "python",
            &[
                "python/scope/usage/F841.py",
                "python/value/ruff/SIM116.py",
                "python/calls/project/pkg/models.py",
            ][..],
        ),
        (
            "typescript",
            &[
                "ecma/scope/typescript/bindings.ts",
                "ecma/value/eslint/no-dupe-keys.ts",
            ][..],
        ),
    ];
    let mut rng = Rng(0x5e7_c0de);
    let (mut programs, mut held, mut subsets) = (0usize, 0usize, 0usize);
    for (label, paths) in fixtures {
        let dialect = dialect_by_label(label).expect("a dialect");
        let docs: Vec<TapeDoc> = paths
            .iter()
            .map(|path| {
                let source = std::fs::read_to_string(format!(
                    "{}/{path}",
                    crate::code_rule_dsl::fixtures_dir!()
                ))
                .unwrap_or_else(|error| panic!("{path}: {error}"));
                tape(dialect, &source)
            })
            .collect();
        let vocabulary = vocabulary(&docs);
        for _ in 0..300 {
            let rule = random_rule(&mut rng, &vocabulary, 3);
            let program = compile(&rule).expect("a program");
            assert!(program.forward(), "{rule:?}");
            programs += 1;
            for doc in &docs {
                let len = doc.transducer_len() as u32;
                let paths: BTreeSet<u32> = program
                    .execute(doc)
                    .expect("the path executor")
                    .into_iter()
                    .map(|output| output.start)
                    .collect();
                let sets: BTreeSet<u32> = program
                    .holds(doc, &NodeSet::full(len))
                    .expect("the node-set executor")
                    .iter()
                    .collect();
                assert_eq!(paths, sets, "{rule:?}");
                held += sets.len();
                // From some starting nodes only: the same verdicts there.
                let mut starts = NodeSet::new(len);
                let mut chosen = BTreeSet::new();
                for _ in 0..rng.below(8) {
                    let node = rng.below(len as usize) as u32;
                    starts.insert(node);
                    chosen.insert(node);
                }
                let some: BTreeSet<u32> = program
                    .holds(doc, &starts)
                    .expect("the node-set executor")
                    .iter()
                    .collect();
                assert_eq!(
                    some,
                    paths.intersection(&chosen).copied().collect(),
                    "{rule:?} from {chosen:?}"
                );
                subsets += 1;
            }
        }
    }
    eprintln!(
        "{programs} programs agree on every fixture node ({held} holding), and on {subsets} start subsets"
    );
}
