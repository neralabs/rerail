//! Set mode's executor freedoms (t1 spec-transducer §1 :95-100, §3 :368-377; list step 14).
//!
//! A program in set mode (`Program::in_set_mode`) is read as a set of (task, label) keys. An
//! executor may then stop a label after its first emit in a task, and drop a path whose reachable
//! labels have all fired. `Reach` is that analysis: each pc's labels reachable along the
//! program's control flow (fall-through, `FORK`'s two targets, `GOTO`), as bits of the program's
//! own labels; `Fired` is a task's emitted labels. Every backend keeps the first freedom (a task
//! ends once every label fired); the host's executors (the interpreter, the micro walk) also prune
//! by reach.
use crate::program::{EMIT, FORK, GOTO, STOP, split};

/// Each pc's reachable labels, for a program of at most 64 distinct emitted labels.
#[derive(Clone, Debug)]
pub struct Reach {
    /// The program's emitted labels, ascending: a label's bit is its index.
    labels: Vec<u16>,
    /// Per pc: the bits of the labels some path from it can emit.
    masks: Vec<u64>,
}

impl Reach {
    /// `words`' reach, or None where the program emits more than 64 labels (no pruning).
    pub fn of(words: &[u32]) -> Option<Self> {
        let mut labels = words.iter().map(|&word| split(word)).filter(|(op, ..)| *op == EMIT).map(|(.., label)| label).collect::<Vec<_>>();
        labels.sort_unstable();
        labels.dedup();
        if labels.len() > 64 {
            return None;
        }
        let bit = |label: u16| 1_u64 << labels.binary_search(&label).expect("an emitted label");
        let mut masks = vec![0_u64; words.len()];
        // Backward to a fixpoint: a pc reaches what its successors reach (loops by `GOTO`).
        loop {
            let mut changed = false;
            for pc in (0..words.len()).rev() {
                let (op, _, operand) = split(words[pc]);
                let at = |target: usize| masks.get(target).copied().unwrap_or(0);
                let mask = match op {
                    STOP => 0,
                    EMIT => bit(operand),
                    GOTO => at(usize::from(operand)),
                    FORK => at(pc + 1) | at(usize::from(operand)),
                    _ => at(pc + 1),
                };
                if mask != masks[pc] {
                    masks[pc] = mask;
                    changed = true;
                }
            }
            if !changed {
                return Some(Self { labels, masks });
            }
        }
    }

    /// Every label's bits.
    pub fn all(&self) -> u64 {
        if self.labels.len() == 64 { u64::MAX } else { (1 << self.labels.len()) - 1 }
    }

    /// `label`'s bit.
    pub fn bit(&self, label: u16) -> u64 {
        self.labels.binary_search(&label).map_or(0, |index| 1 << index)
    }

    /// Whether a path at `pc` can still emit a label not in `fired`.
    pub fn live(&self, pc: u16, fired: u64) -> bool {
        self.masks.get(usize::from(pc)).is_some_and(|mask| mask & !fired != 0)
    }

    /// The program's emitted labels.
    pub fn labels(&self) -> &[u16] {
        &self.labels
    }
}
