use super::super::{
    CAPTURE, EMIT, ExecutionBackend, Program, SUPERTYPE, execute_with_backend,
    execute_with_backend_packed_result_limits, word,
};
use super::{resolve, settle};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::tape::{DERIVED_CLASS_BASE, Grammar, TapeDoc, TapeImage};
use std::sync::Arc;

fn python_tape(source: &str) -> TapeDoc {
    let source = Arc::new(source.to_owned());
    let grammar = Grammar::new(GrammarDialect::Python).expect("Python grammar");
    let image = TapeImage::parse(&grammar, &source, &|| false)
        .expect("parser completed")
        .expect("tape image");
    TapeDoc::restore(source, grammar, &image, &|| false).expect("restored tape")
}

/// Two programs no other test compiles (`tag` is in their strings, and Metal's kernel, which is
/// of their words, has `captures` captures): every node, and the nodes with the first derived
/// class.
fn programs(tag: &str, captures: usize) -> Vec<Program> {
    let program = |words: Vec<u32>| {
        Program::new(words, vec![b"match".to_vec(), tag.as_bytes().to_vec()], Vec::new(), vec![0, 1], Vec::new())
            .expect("valid program")
    };
    let capture = std::iter::repeat_n(word(CAPTURE, 0, 0), captures);
    vec![
        program(capture.clone().chain([word(EMIT, 0, 1)]).collect()),
        program([word(SUPERTYPE, 0, DERIVED_CLASS_BASE)].into_iter().chain(capture).chain([word(EMIT, 0, 1)]).collect()),
    ]
}

/// Two tapes, each with one derived class on some of its nodes.
fn tapes() -> (Vec<TapeDoc>, Vec<Vec<Vec<u32>>>) {
    let tapes = [("x = 1\n", [1_u32, 3]), ("def f(): pass\n", [0, 2])]
        .into_iter()
        .map(|(source, marked)| {
            let tape = python_tape(source);
            let mut bits = vec![0_u64; tape.transducer_len().div_ceil(64)];
            marked.iter().for_each(|&node| bits[node as usize / 64] |= 1 << (node % 64));
            tape.with_labels(Arc::new(vec![bits]))
        })
        .collect::<Vec<_>>();
    let anchors = tapes
        .iter()
        .map(|tape| vec![(0..tape.transducer_len() as u32).collect(); 2])
        .collect();
    (tapes, anchors)
}

/// Runs `backend` in the background mode: its interpreter first, the same results, and once the
/// compilation it requested is done, its compiled programs.
fn background(backend: ExecutionBackend, interpreter: ExecutionBackend, compiled: ExecutionBackend, tag: &str) {
    let programs = programs(tag, 1);
    let (tapes, anchors) = tapes();
    assert_eq!(resolve(backend, &programs).unwrap(), interpreter, "nothing compiled yet");
    let expected = execute_with_backend(compiled, &programs, &tapes, &anchors).unwrap();
    let marked = expected.iter().map(|file| file[1].iter().map(|output| output.start).collect::<Vec<_>>()).collect::<Vec<_>>();
    assert_eq!(marked, vec![vec![1, 3], vec![0, 2]], "the derived class is tested as a bit");
    let packed = |backend| {
        execute_with_backend_packed_result_limits(backend, &programs, &tapes, &anchors, None)
            .unwrap()
            .into_outcome()
            .unwrap()
            .outputs
    };
    assert_eq!(packed(interpreter), expected, "the interpreter's batch publishes as the compiled one");
    // `compiled` above compiled these programs for itself; fresh ones take the background path.
    let programs = self::programs(&format!("{tag} fresh"), 2);
    assert_eq!(resolve(backend, &programs).unwrap(), interpreter);
    let first = execute_with_backend(backend, &programs, &tapes, &anchors).unwrap();
    settle();
    assert_eq!(resolve(backend, &programs).unwrap(), compiled, "compiled once the request finished");
    assert_eq!(execute_with_backend(backend, &programs, &tapes, &anchors).unwrap(), first);
    assert_eq!(first.len(), 2);
}

#[test]
fn the_cpu_runs_the_interpreter_until_its_programs_compile() {
    background(ExecutionBackend::CpuBackground, ExecutionBackend::CpuInterpreter, ExecutionBackend::CpuJit, "compilation cpu");
}

#[cfg(all(target_os = "macos", feature = "metal-tests"))]
#[test]
fn metal_runs_its_interpreter_until_its_kernel_compiles() {
    background(ExecutionBackend::MetalBackground, ExecutionBackend::MetalInterpreter, ExecutionBackend::MetalJit, "compilation metal");
}
