use super::*;
use crate::code_rule_dsl::matcher::Semantic;

pub(super) fn optional_outward_captures(
    semantic: &Semantic,
    under_any: bool,
    outward: bool,
    memo: &mut SoundMemo<BTreeSet<String>>,
    names: &mut BTreeSet<String>,
) {
    let mut walk = |item: &AstMatcher, under_any: bool, outward: bool| {
        super::optional_outward_captures(item, under_any, outward, memo, names);
    };
    match semantic {
        Semantic::Resolves(_)
        | Semantic::Calls(_)
        | Semantic::Value(_)
        | Semantic::Arguments(_)
        | Semantic::Parse(_)
        | Semantic::Capture(_)
        | Semantic::Layout(_)
        | Semantic::Host(_) => {}
        Semantic::FormatArgs(test) => {
            if let Some(unused) = &test.unused {
                walk(unused, under_any, outward);
            }
        }
        // A binding's nodes lie anywhere in the file, like an outward relation's.
        Semantic::Binding(test) => {
            for rule in test.sub_rules() {
                walk(rule, under_any, true);
            }
        }
        Semantic::Binds(test) => {
            for rule in test.sub_rules() {
                walk(rule, under_any, true);
            }
        }
    }
}

pub(super) fn sound(
    semantic: &Semantic,
    negated: bool,
    outside: bool,
    optional: &BTreeSet<String>,
    memo: &mut SoundMemo<bool>,
) -> bool {
    match semantic {
        Semantic::Arguments(_) | Semantic::Capture(_) => true,
        // A parsed tree is the node's own text, whatever the rest of the file holds; its `host`
        // rules read the file around it, each as sound as it is alone, and none negated.
        Semantic::Parse(test) => {
            test.grammar.reads_only_the_node()
                && test.host_sound.is_none_or(|sound| sound && !negated)
        }
        Semantic::Host(test) => !negated && super::sound_under_errors(&test.rule),
        // Format arguments read two captures, as a comparison does, and the arguments' own nodes.
        // A PHP format followed to a binding is unknown in a file with syntax errors
        // (`binding_value` reads none there), never another binding's.
        Semantic::FormatArgs(test) => {
            (!negated || !(optional.contains(&test.string) || optional.contains(&test.args)))
                && test
                    .unused
                    .as_ref()
                    .is_none_or(|unused| super::sound(unused, negated, outside, optional, memo))
        }
        // Following a name reads its binding, which an error region can hide or rebind.
        Semantic::Value(test) => !test.reading.follow,
        // A binding lies anywhere in the file, and an error region can hide a closer one.
        Semantic::Binding(_) => false,
        // The node's own lines and first byte are the witness's. A gap reads outward, as
        // `precedes` does: its neighbour and the tokens up to it are facts of the recovered tree,
        // but their absence is no fact, so a negated gap is unsound.
        Semantic::Layout(layout) => !negated || !layout.outward(),
        // Whether a binding is read, or what it redefines, depends on the whole file.
        Semantic::Binds(_) => false,
        // Resolution reads bindings anywhere in the file, and an error region can hide one: a
        // shadowed name would read as global. `imported` alone reads only the import statement
        // around the node and names nothing through an error before it; negated, that "nothing"
        // would become a violation.
        Semantic::Resolves(resolves) => !negated && resolves.witness_only(),
        // The callee's name resolves through the file's bindings, which an error region can hide.
        Semantic::Calls(_) => false,
    }
}

pub(super) fn leaves_witness(semantic: &Semantic, memo: &mut UtilitySummary<usize, bool>) -> bool {
    match semantic {
        Semantic::Resolves(_)
        | Semantic::Calls(_)
        | Semantic::Value(_)
        | Semantic::Arguments(_)
        | Semantic::Parse(_)
        | Semantic::Capture(_)
        | Semantic::Layout(_) => false,
        Semantic::Host(_) => true,
        Semantic::FormatArgs(test) => test
            .unused
            .as_ref()
            .is_some_and(|unused| super::leaves_witness(unused, memo)),
        Semantic::Binding(_) | Semantic::Binds(_) => true,
    }
}
