//! Join shapes (the join lab's design, 2026-11-27): a join as the join program encodes it, with the rule's names
//! and its constants' values taken out.
//!
//! A shape is a join's stages as the device layer encodes them (`setfix::device`'s `PStage`s and their `Stage`
//! records), each with what its input is: a derived relation the program made, the component's own relation, a view
//! of the tape, a tape axis, or words. Two joins with the same stages over inputs of the same kinds have one shape,
//! whatever their relations are called and whatever constants they test, so one class serves every rule (and every
//! language) that has it. A constant is a parameter of the shape, never part of it (the user's decision of
//! 2026-11-27): `kind(n, "call")` and `kind(n, "attribute")` are one shape.
//!
//! Slots are the plan's: a rule's variables numbered in the order its steps bind them, so they carry no names.
//!
//! The key is a 64-bit FNV-1a hash of the shape's canonical words: the same in every build and on every host, so a
//! catalogue made on one machine names the same shapes as one made on another. The YAML (`Shape::yaml`) is the
//! catalogue's entry, which the join lab and the class generator read.

use std::fmt::Write as _;

/// What a stage reads besides its input bindings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Input {
    /// Nothing: a filter, a map, a pass, a text made per row.
    None,
    /// A relation the program derived, of `arity` columns, read where it lies in its block: `dead` when a delta's
    /// dead bits mark rows it skips, `from` when it is read from a first row (a fixpoint's delta), `indexed` when
    /// it is probed by an index the build ahead made.
    Derived { arity: u32, dead: bool, from: bool, indexed: bool },
    /// The component's own relation (recursion): its last round's new rows (`delta`) or all of them.
    Own { delta: bool },
    /// A view of the tape read in place: its number (1 + `device_chunk::VIEW_*`), `parent` when `child` is probed
    /// by its parent off the tape.
    View { view: u32, parent: bool },
    /// A `has` axis read off the tape: 1 by its node (a climb), 2 by its ancestor (the subtree).
    Axis { walk: u32 },
    /// Words a raw mode reads: a text's names, a test's verdict bits.
    Words,
}

impl Input {
    fn words(&self, w: &mut Vec<u32>) {
        match *self {
            Input::None => w.push(0),
            Input::Derived { arity, dead, from, indexed } => w.extend([1, arity, dead as u32 | (from as u32) << 1 | (indexed as u32) << 2]),
            Input::Own { delta } => w.extend([2, delta as u32]),
            Input::View { view, parent } => w.extend([3, view, parent as u32]),
            Input::Axis { walk } => w.extend([4, walk]),
            Input::Words => w.push(5),
        }
    }

    fn yaml(&self) -> String {
        match *self {
            Input::None => "{kind: none}".into(),
            Input::Derived { arity, dead, from, indexed } => format!("{{kind: derived, arity: {arity}, dead: {dead}, from: {from}, indexed: {indexed}}}"),
            Input::Own { delta } => format!("{{kind: own, delta: {delta}}}"),
            Input::View { view, parent } => format!("{{kind: view, view: {view}, parent: {parent}}}"),
            Input::Axis { walk } => format!("{{kind: axis, walk: {walk}}}"),
            Input::Words => "{kind: words}".into(),
        }
    }
}

/// One stage of a shape: its kind and mode, its input, and the structural fields of its `Stage` record. Every list
/// holds the record's words as encoded, with the constants' values left out (`Shape::key`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StageForm {
    /// The stage's kind as the device layer plans it (`atom`, `own`, `filter`, `witness`, `rowtext`, ...).
    pub kind: &'static str,
    /// The record's `mode` (`JOIN`, `SEMI`, `ANTI`, `FILTER`, `AGG`, ...), as its number and its name.
    pub mode: u32,
    pub mode_name: &'static str,
    pub input: Option<Input>,
    /// The steps of the rule it covers.
    pub span: u32,
    /// Its keys: (kind, slot or constant index, the input's column).
    pub keys: Vec<[u32; 3]>,
    /// Its tests: (a, b) as encoded (a comparison's right side a parameter when `b`'s top bit is set).
    pub tests: Vec<[u32; 2]>,
    /// Its row program: (op, a, b, c).
    pub ops: Vec<[u32; 4]>,
    /// Its parameters: (kind, slot or constant index, document).
    pub params: Vec<[u32; 3]>,
    /// Its binds: (slot, kind, index).
    pub binds: Vec<[u32; 3]>,
    /// The input's columns it reads: (kind, index, document).
    pub cols: Vec<[u32; 3]>,
    /// The slots it writes, and the fused steps whose passes it counts.
    pub outputs: Vec<u32>,
    pub passed: Vec<u32>,
    pub agg: [u32; 2],
    pub arith: Vec<[u32; 4]>,
    /// Whether a kept filter screens its key (decision 0488).
    pub screened: bool,
    /// The constants it holds: how many, by where (keys, parameters). Their values are the class's parameters.
    pub key_consts: u32,
    pub par_consts: u32,
}

impl StageForm {
    fn words(&self, w: &mut Vec<u32>) {
        w.extend(self.kind.bytes().map(u32::from));
        w.extend([u32::MAX, self.mode, self.span]);
        match &self.input {
            Some(i) => i.words(w),
            None => w.push(u32::MAX),
        }
        let list = |w: &mut Vec<u32>, n: usize, words: &mut dyn Iterator<Item = u32>| {
            w.push(n as u32);
            w.extend(words);
        };
        list(w, self.keys.len(), &mut self.keys.iter().flatten().copied());
        list(w, self.tests.len(), &mut self.tests.iter().flatten().copied());
        list(w, self.ops.len(), &mut self.ops.iter().flatten().copied());
        list(w, self.params.len(), &mut self.params.iter().flatten().copied());
        list(w, self.binds.len(), &mut self.binds.iter().flatten().copied());
        list(w, self.cols.len(), &mut self.cols.iter().flatten().copied());
        list(w, self.outputs.len(), &mut self.outputs.iter().copied());
        list(w, self.passed.len(), &mut self.passed.iter().copied());
        list(w, self.arith.len(), &mut self.arith.iter().flatten().copied());
        w.extend(self.agg);
        w.extend([self.screened as u32, self.key_consts, self.par_consts]);
    }
}

/// A join's shape: its slots, its stages, and its head (a slot a column, or a constant's).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shape {
    pub slots: u32,
    pub stages: Vec<StageForm>,
    /// Per head column, the slot it takes (`None`: a constant).
    pub head: Vec<Option<u32>>,
    /// Per head column, whether its values are texts the stages made.
    pub made_texts: Vec<bool>,
}

/// FNV-1a over 32-bit words, each as its 4 little-endian bytes.
fn fnv(words: &[u32]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for w in words {
        for b in w.to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

impl Shape {
    /// Its canonical words: everything structural about it, nothing named, no constant's value.
    pub fn words(&self) -> Vec<u32> {
        let mut w = vec![1, self.slots, self.stages.len() as u32];
        for s in &self.stages {
            s.words(&mut w);
        }
        w.push(self.head.len() as u32);
        w.extend(self.head.iter().map(|h| h.unwrap_or(u32::MAX)));
        w.extend(self.made_texts.iter().map(|&m| m as u32));
        w
    }

    /// Its key: the same in every build and on every host.
    pub fn key(&self) -> u64 {
        fnv(&self.words())
    }

    /// Its name in the catalogue and the generated classes: `s-` and its key in hex.
    pub fn name(&self) -> String {
        format!("s-{:016x}", self.key())
    }

    /// Its form in `shapes.yaml`, each line indented by `indent` (the entry's `shape:` line is its caller's).
    pub fn yaml(&self, indent: &str) -> String {
        let mut y = String::new();
        let _ = writeln!(y, "{indent}slots: {}", self.slots);
        let heads: Vec<String> = self.head.iter().zip(self.made_texts.iter().chain(std::iter::repeat(&false))).map(|(h, &m)| match h {
            Some(s) if m => format!("v{s}:text"),
            Some(s) => format!("v{s}"),
            None => "const".into(),
        }).collect();
        let _ = writeln!(y, "{indent}head: [{}]", heads.join(", "));
        let _ = writeln!(y, "{indent}stages:");
        let l3 = |v: &[[u32; 3]]| v.iter().map(|[a, b, c]| format!("[{a}, {b}, {c}]")).collect::<Vec<_>>().join(", ");
        let l4 = |v: &[[u32; 4]]| v.iter().map(|[a, b, c, d]| format!("[{a}, {b}, {c}, {d}]")).collect::<Vec<_>>().join(", ");
        let l1 = |v: &[u32]| v.iter().map(u32::to_string).collect::<Vec<_>>().join(", ");
        for s in &self.stages {
            let _ = writeln!(y, "{indent}  - kind: {}", s.kind);
            let _ = writeln!(y, "{indent}    mode: {}", s.mode_name);
            let _ = writeln!(y, "{indent}    span: {}", s.span);
            if let Some(i) = &s.input {
                let _ = writeln!(y, "{indent}    input: {}", i.yaml());
            }
            let mut field = |name: &str, text: String, empty: bool| {
                if !empty {
                    let _ = writeln!(y, "{indent}    {name}: [{text}]");
                }
            };
            field("keys", l3(&s.keys), s.keys.is_empty());
            field("tests", s.tests.iter().map(|[a, b]| format!("[{a}, {b}]")).collect::<Vec<_>>().join(", "), s.tests.is_empty());
            field("ops", l4(&s.ops), s.ops.is_empty());
            field("params", l3(&s.params), s.params.is_empty());
            field("binds", l3(&s.binds), s.binds.is_empty());
            field("cols", l3(&s.cols), s.cols.is_empty());
            field("outputs", l1(&s.outputs), s.outputs.is_empty());
            field("passed", l1(&s.passed), s.passed.is_empty());
            field("arith", l4(&s.arith), s.arith.is_empty());
            if s.agg != [0, 0] {
                let _ = writeln!(y, "{indent}    agg: [{}, {}]", s.agg[0], s.agg[1]);
            }
            if s.screened {
                let _ = writeln!(y, "{indent}    screened: true");
            }
            if s.key_consts + s.par_consts > 0 {
                let _ = writeln!(y, "{indent}    consts: {{keys: {}, params: {}}}", s.key_consts, s.par_consts);
            }
        }
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atom(view: u32, keys: Vec<[u32; 3]>) -> StageForm {
        StageForm { kind: "atom", mode: 0, mode_name: "join", span: 1, input: Some(Input::View { view, parent: false }), keys, binds: vec![[1, 0, 1]], outputs: vec![0, 1], ..StageForm::default() }
    }

    fn two_step(view: u32, consts: u32) -> Shape {
        let mut probe = atom(view, vec![[1, 0, 0]]);
        probe.key_consts = consts;
        Shape { slots: 2, stages: vec![atom(1, vec![]), probe], head: vec![Some(0), Some(1)], made_texts: vec![false, false] }
    }

    #[test]
    fn a_shape_is_its_structure_alone_and_its_key_is_stable() {
        // Built twice (two rules of the same steps, whatever they are called): one key.
        assert_eq!(two_step(3, 1).key(), two_step(3, 1).key());
        // Its key is the canonical words' FNV-1a, the same in every build: pinned.
        assert_eq!(fnv(&[]), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv(&[0x61]), fnv_bytes(&[0x61, 0, 0, 0]));
        // A different view read, or a different key, is another shape.
        assert_ne!(two_step(3, 1).key(), two_step(4, 1).key());
        let mut other = two_step(3, 1);
        other.stages[1].keys[0][2] = 1;
        assert_ne!(two_step(3, 1).key(), other.key());
        // A different input kind is another shape.
        let mut derived = two_step(3, 1);
        derived.stages[1].input = Some(Input::Derived { arity: 2, dead: false, from: false, indexed: true });
        assert_ne!(two_step(3, 1).key(), derived.key());
        // How many constants it holds is structural; their values are not in the shape at all.
        assert_ne!(two_step(3, 1).key(), two_step(3, 0).key());
        assert!(two_step(3, 1).name().starts_with("s-") && two_step(3, 1).name().len() == 18);
    }

    fn fnv_bytes(bytes: &[u8]) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &b in bytes {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    #[test]
    fn its_yaml_lists_every_stage_with_its_input() {
        let y = two_step(3, 1).yaml("  ");
        assert!(y.starts_with("  slots: 2\n  head: [v0, v1]\n  stages:\n"), "{y}");
        assert_eq!(y.matches("  - kind: atom").count(), 2, "{y}");
        assert!(y.contains("input: {kind: view, view: 3, parent: false}"), "{y}");
        assert!(y.contains("consts: {keys: 1, params: 0}"), "{y}");
        assert!(!y.contains("tests:"), "an empty list is left out: {y}");
    }
}
