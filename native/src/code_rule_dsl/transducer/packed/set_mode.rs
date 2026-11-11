//! Set mode's executor freedoms (t1 spec-transducer §1 :95-100, §3 :368-377; list step 14).
//!
//! A program in set mode (`Program::in_set_mode`) is read as a set of (task, label) keys. An
//! executor may then stop a label after its first emit in a task, and drop a path whose reachable
//! labels have all fired. `Reach` is that analysis: each pc's labels reachable along the
//! program's control flow (fall-through, `FORK`'s two targets, `GOTO`), as bits of the program's
//! own labels; `Fired` is a task's emitted labels. Every backend keeps the first freedom (a task
//! ends once every label fired); the host's executors (the interpreter, the micro walk) also prune
//! by reach.

// The analysis is the compiler crate's (`rerail_compiler::set_mode`).
pub(in crate::code_rule_dsl) use rerail_compiler::set_mode::*;

// The tests read the transducer's names as they did when this file held the code.
#[cfg(test)]
#[allow(unused_imports)]
use super::*;

#[cfg(test)]
mod tests {
    use super::super::{CAPTURE, CHILD, DESCEND, ExecutionBackend, KIND, Micro, NAMED, Program, execute_with_backend, word};
    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use ast_grep_core::tree_sitter::{LanguageExt, StrDoc};
    use std::collections::BTreeSet;
    use std::sync::Arc;

    fn tape(source: &str) -> TapeDoc {
        let grammar = Grammar::new(GrammarDialect::Python).unwrap();
        let native = StrDoc::new(source, GrammarDialect::Python);
        let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
        TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap()
    }

    /// Set mode's output is exact mode's as a set of (start, label) keys, each once, on the
    /// interpreter, the micro walk and the CPU JIT; a root with many emitting paths emits once a
    /// label.
    #[test]
    fn set_mode_emits_each_key_once_on_every_cpu_executor() {
        let identifier = GrammarDialect::Python.get_ts_language().id_for_node_kind("identifier", true);
        let doc = tape("def f(a, b):\n    return a + b * g(a, b)\nclass K:\n    x = y + z\n");
        let roots = (0..doc.transducer_len() as u32).collect::<Vec<_>>();
        let program = |words: Vec<u32>, labels: usize| {
            Program::new(words, vec![b"one".to_vec(), b"two".to_vec()], vec![], (0..labels as u16).collect(), vec![]).unwrap()
        };
        // One label: every identifier below a start. Two labels: those, or a named child.
        let one = vec![word(DESCEND, NAMED, 0), word(KIND, 0, identifier), word(CAPTURE, 0, 0), word(EMIT, 0, 0)];
        let two = vec![
            word(FORK, 0, 5),
            word(DESCEND, NAMED, 0),
            word(KIND, 0, identifier),
            word(EMIT, 0, 0),
            word(STOP, 0, 0),
            word(CHILD, NAMED, 0),
            word(EMIT, 0, 1),
        ];
        for (words, labels) in [(one, 1), (two, 2)] {
            let exact = program(words.clone(), labels);
            let set = program(words, labels).in_set_mode();
            assert!(set.set_mode() && !exact.set_mode());
            assert_ne!(set.plain_digest(), exact.plain_digest(), "the JIT's cache tells them apart");
            let keys = |outputs: &[super::super::Output]| outputs.iter().map(|o| (o.start, o.label)).collect::<BTreeSet<_>>();
            let exact_out = exact.execute_roots(&doc, roots.clone()).unwrap();
            let expected = keys(&exact_out);
            assert!(exact_out.len() > expected.len(), "some start emits a label more than once");
            let once = |outputs: &[super::super::Output], by: &str| {
                assert_eq!(keys(outputs), expected, "{by}: the keys");
                assert_eq!(outputs.len(), expected.len(), "{by}: each key once");
            };
            once(&set.execute_roots(&doc, roots.clone()).unwrap(), "interpreter");
            let jit = execute_with_backend(ExecutionBackend::CpuJit, std::slice::from_ref(&set), std::slice::from_ref(&doc), &[vec![roots.clone()]]).unwrap();
            let jit = jit.into_iter().flatten().flatten().collect::<Vec<_>>();
            if labels == 1 {
                once(&jit, "jit");
            } else {
                // The JIT stops only a single-label program; with more it is exact, the same keys.
                assert_eq!(keys(&jit), expected, "jit: the keys");
            }
            let walk = Micro::of(&set).expect("a micro walk");
            let texts = walk.texts(&set, &doc).unwrap();
            let mut frames = super::super::Frames::default();
            let mut records = Vec::new();
            for &start in &roots {
                walk.records(&doc, &texts, start, &mut frames, &mut records).unwrap();
            }
            let walked = records.iter().map(|r| (r.start, r.label)).collect::<BTreeSet<_>>();
            assert_eq!(walked, expected, "micro: the keys");
            assert_eq!(records.len(), expected.len(), "micro: each key once");
        }
    }

    #[test]
    fn reach_follows_forks_and_gotos_to_each_emit() {
        // 0 FORK 4; 1 KIND; 2 EMIT 7; 3 STOP; 4 CHILD; 5 GOTO 7; 6 STOP; 7 EMIT 9
        let words = [word(FORK, 0, 4), word(KIND, 0, 1), word(EMIT, 0, 7), word(STOP, 0, 0), word(CHILD, 0, 0), word(GOTO, 0, 7), word(STOP, 0, 0), word(EMIT, 0, 9)];
        let reach = Reach::of(&words).unwrap();
        assert_eq!(reach.labels(), &[7, 9]);
        let (seven, nine) = (reach.bit(7), reach.bit(9));
        assert!(reach.live(0, 0) && reach.live(0, seven) && !reach.live(0, seven | nine));
        assert!(reach.live(1, 0) && !reach.live(1, seven));
        assert!(reach.live(4, seven) && !reach.live(4, nine));
        assert!(!reach.live(3, 0) && !reach.live(6, 0));
        assert_eq!(reach.all(), seven | nine);
    }
}
