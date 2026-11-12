//! Tier 1's token records (`records`) on every Bitlex ladder step with a tree section: each
//! token's first byte, last byte and code, by the interpreter and by the compiled record pass,
//! against a plain oracle (the first byte's kind; the word, by comparing the token's bytes with
//! each word).

use crate::bitlex_masks::{self, FALLBACK, Symbols};
use crate::{native_cpu_block, native_ir, rir_asm};
use std::cell::RefCell;
use std::collections::HashMap;

#[derive(Default)]
struct Ids(RefCell<HashMap<String, u16>>);

impl Ids {
    fn of(&self, name: &str) -> u16 {
        let mut m = self.0.borrow_mut();
        let n = m.len() as u16 + 1;
        *m.entry(name.to_owned()).or_insert(n)
    }
}

const DOCUMENTS: &[&str] = &[
    "",
    "vrai fausse nul",
    "(vrai;fausse;nul)",
    "<#a# = vrai; #b# = (1;2)>",
    "{\"a\": [1, 2.5e3, true, null, \"x\\n\"]}",
    "[1, 2,]",
    "// c\n[1]",
    "{- open",
    "r(x) :- s(x), !t(x). // c\n.decl r(a: number)\n.policy {\"a\": 1}",
    "r(n) :- n = sum min x : { s(x) } : { t(n) }, m = count : { u(_) }. violation violations",
    "#include \"x\" .functor f(a: number): number stateful",
];

#[test]
fn records_are_each_tokens_bytes_and_code() {
    let mut steps: Vec<_> = std::fs::read_dir(crate::ladder::DIR).expect("the ladder").map(|e| e.expect("an entry").path()).filter(|p| p.extension().is_some_and(|x| x == "bitlex")).collect();
    steps.sort();
    let (mut with, mut tokens, mut worded) = (0, 0, 0);
    for path in steps {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
        let stmts = ::bitlex::syntax::parse(&std::fs::read_to_string(&path).expect("a step")).expect("parses");
        let ids = Ids::default();
        let kind = |n: &str| Some(ids.of(n));
        let field = |n: &str| Some(ids.of(&format!("field {n}")));
        let symbols = Symbols { kind: &kind, field: &field };
        let compiled = bitlex_masks::compile(&stmts, &[FALLBACK], Some(&symbols)).expect("compiles");
        let words = rir_asm::assemble(&compiled.text()).unwrap_or_else(|e| panic!("{name}: assembles: {e}\n{}", compiled.text().lines().filter(|l| !l.contains(" %")).take(40).collect::<Vec<_>>().join("\n")));
        let block = &words.block_programs[0];
        let Some(r) = &block.records else { continue };
        with += 1;
        // The words are tier 2's strings; a switch on the first byte is alike within a kind.
        assert_eq!(r.words, words.strings, "{name}");
        for s in words.switches.iter().filter(|s| s.offset == 0) {
            for a in 0..256 {
                for b in 0..256 {
                    if r.kinds[a] == r.kinds[b] {
                        assert_eq!(s.targets[a], s.targets[b], "{name}: bytes {a:#x} and {b:#x} share a kind and not a target");
                    }
                }
            }
        }
        let native = native_ir::lower(&words, 0, 0).expect("lowers");
        let tier1 = native_cpu_block::compile(&native.block_programs[0]).expect("Cranelift compiles tier 1");
        for doc in DOCUMENTS {
            let bytes = doc.as_bytes();
            let run = block.run(bytes);
            let (starts, lasts) = (&run.positions[usize::from(r.starts)], &run.positions[usize::from(r.lasts)]);
            if starts.len() == lasts.len() && run.totals == [0] {
                let oracle: Vec<u32> = starts
                    .iter()
                    .zip(lasts)
                    .map(|(&s, &l)| {
                        let text = &bytes[s as usize..=l as usize];
                        let w = r.words.iter().position(|w| w[..] == *text).map_or(0, |i| i as u32 + 1);
                        u32::from(r.kinds[usize::from(bytes[s as usize])]) | w << crate::records::KIND_BITS
                    })
                    .collect();
                assert_eq!(run.codes, oracle, "{name}: {doc:?}");
                tokens += oracle.len();
                worded += oracle.iter().filter(|&&c| c >> crate::records::KIND_BITS != 0).count();
            }
            let got = tier1.extract(bytes);
            assert_eq!((&got.positions, &got.codes, &got.totals), (&run.positions, &run.codes, &run.totals), "{name}: {doc:?}: the compiled record pass");
        }
    }
    assert!(with >= 10, "{with} steps with records");
    assert!(tokens > 200 && worded > 20, "{tokens} tokens, {worded} words");
}

/// The words' table: every word found, longer tokens and other words not; a word past eight
/// bytes compared whole; two words alike in their first eight bytes and length refused.
#[test]
fn the_word_table_finds_each_word_only() {
    let words: Vec<Vec<u8>> = ["a", "if", "violation", "violations", "abcdefghijklmnopqrstuvwxyz", "(", "!="].iter().map(|w| w.as_bytes().to_vec()).collect();
    let t = crate::records::WordTable::new(&words).expect("a table");
    let pad = |s: &str| {
        let mut v = s.as_bytes().to_vec();
        v.extend([b'#'; 64]);
        v
    };
    for (i, w) in words.iter().enumerate() {
        let s = String::from_utf8(w.clone()).expect("text");
        assert_eq!(t.word(&pad(&s), 0, w.len()), i as u32 + 1, "{s}");
    }
    for other in ["b", "i", "ifs", "violatioN", "violationz", "abcdefghijklmnopqrstuvwxyZ", "(("] {
        assert_eq!(t.word(&pad(other), 0, other.len()), 0, "{other}");
    }
    let alike: Vec<Vec<u8>> = vec![b"abcdefghX".to_vec(), b"abcdefghY".to_vec()];
    assert!(crate::records::WordTable::new(&alike).is_err());
}
