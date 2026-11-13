//! Prove strict preorder progress on every returning control-flow edge.
//!
//! Facts describe positions relative to the node at the loop entry. Branch joins
//! retain only facts true on every path. Cursor saves preserve those facts, and
//! lower saves truncate higher slots exactly as ExecutionArena::save_cursor does.
use super::{
    CHILD, DESCEND, EMIT, Error, FOLLOWING, FORK, GOTO, NEXT, PARENT, RESTORE_CURSOR, SAVE_CURSOR,
    STOP, split,
};
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Order {
    Unknown,
    AtLeastEntry,
    AfterEntry,
}

impl Order {
    fn advance(self) -> Self {
        match self {
            Self::Unknown => Self::Unknown,
            Self::AtLeastEntry | Self::AfterEntry => Self::AfterEntry,
        }
    }
}

/// Node order followed by the orders of known ancestors. An absent position is
/// unknown. Keeping ancestors permits CHILD/PARENT and sibling round trips.
#[derive(Clone, Default)]
struct Position(Vec<Order>);

impl Position {
    fn order(&self) -> Order {
        self.0.first().copied().unwrap_or(Order::Unknown)
    }

    fn canonicalize(&mut self) {
        while self.0.last() == Some(&Order::Unknown) {
            self.0.pop();
        }
    }

    fn meet(&mut self, other: &Self) -> bool {
        let mut changed = self.0.len() > other.0.len();
        self.0.truncate(other.0.len());
        for (left, right) in self.0.iter_mut().zip(&other.0) {
            let merged = (*left).min(*right);
            changed |= *left != merged;
            *left = merged;
        }
        self.canonicalize();
        changed
    }

    fn navigate(&mut self, opcode: u8) {
        let order = self.order();
        match opcode {
            NEXT | FOLLOWING => {
                if let Some(node) = self.0.first_mut() {
                    *node = order.advance();
                }
            }
            CHILD => self.0.insert(0, order.advance()),
            DESCEND => {
                // A descendant is later. Its parent is the old node or a later
                // descendant; no fact about higher ancestors follows.
                self.0 = vec![order.advance(), order];
            }
            PARENT => {
                if !self.0.is_empty() {
                    self.0.remove(0);
                }
            }
            _ => unreachable!("navigation opcode"),
        }
        self.canonicalize();
    }
}

#[derive(Clone, Default)]
struct State {
    node: Position,
    cursors: BTreeMap<u16, Position>,
}

impl State {
    fn meet(&mut self, other: &Self) -> bool {
        let mut changed = self.node.meet(&other.node);
        self.cursors.retain(|slot, position| {
            let Some(other) = other.cursors.get(slot) else {
                changed = true;
                return false;
            };
            changed |= position.meet(other);
            !position.0.is_empty()
        });
        changed
    }

    fn apply(&mut self, opcode: u8, operand: u16) {
        match opcode {
            NEXT | FOLLOWING | CHILD | DESCEND | PARENT => self.node.navigate(opcode),
            SAVE_CURSOR => {
                self.cursors.retain(|&slot, _| slot < operand);
                if !self.node.0.is_empty() {
                    self.cursors.insert(operand, self.node.clone());
                }
            }
            RESTORE_CURSOR => {
                self.node = self.cursors.get(&operand).cloned().unwrap_or_default();
            }
            _ => {}
        }
    }
}

fn successors(words: &[u32], pc: usize) -> [Option<usize>; 2] {
    let (opcode, _, operand) = split(words[pc]);
    let next = (pc + 1 < words.len()).then_some(pc + 1);
    match opcode {
        STOP | EMIT => [None, None],
        GOTO => [Some(usize::from(operand)), None],
        FORK => [next, Some(usize::from(operand))],
        _ => [next, None],
    }
}

fn advances(words: &[u32], edge: usize, entry: usize) -> bool {
    let mut states = vec![None::<State>; words.len()];
    states[entry] = Some(State {
        node: Position(vec![Order::AtLeastEntry]),
        cursors: BTreeMap::new(),
    });
    let mut pending = VecDeque::from([entry]);
    let mut queued = vec![false; words.len()];
    queued[entry] = true;
    while let Some(pc) = pending.pop_front() {
        queued[pc] = false;
        let mut state = states[pc].as_ref().unwrap().clone();
        if pc == edge && state.node.order() != Order::AfterEntry {
            return false;
        }
        let (opcode, _, operand) = split(words[pc]);
        state.apply(opcode, operand);
        for next in successors(words, pc).into_iter().flatten() {
            // Re-entry starts a new proof relative to that entry's new node.
            // In particular, an outer iteration must not change an inner loop's
            // origin. Nested cycles with other entries still reach a fixed point.
            if next == entry {
                continue;
            }
            let changed = if let Some(previous) = &mut states[next] {
                previous.meet(&state)
            } else {
                states[next] = Some(state.clone());
                true
            };
            if changed && !queued[next] {
                pending.push_back(next);
                queued[next] = true;
            }
        }
    }
    // An edge unreachable from its own target cannot close a cycle.
    true
}

/// Operands and targets must be structurally validated before this proof runs.
pub(super) fn validate(words: &[u32]) -> Result<(), Error> {
    for (pc, &instruction) in words.iter().enumerate() {
        let (opcode, _, operand) = split(instruction);
        if matches!(opcode, FORK | GOTO)
            && usize::from(operand) <= pc
            && !advances(words, pc, usize::from(operand))
        {
            return Err(Error::NonProgressingCycle(pc));
        }
    }
    Ok(())
}
