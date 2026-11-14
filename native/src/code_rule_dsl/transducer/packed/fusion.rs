//! Boolean fusion of programs (t1 spec-fusion §4.3, §4.4; multi-target packing §2.6; list step
//! 15's open items): programs walked from the same starts become one program, their common leading
//! words once (a prefix trie of depth one, §4.3), then a `FORK` chain with one branch per member
//! (§4.4), each member's emits relabelled by the fused program's emit table (`Fused::emits`). One
//! walk from a start then answers every member, and the consumer routes an emit by its label to
//! the member, its original label and its capture names (`Fused::captures`).
//!
//! Only micro walks fuse (`compile::micro::Micro::of`): straight node tests, moves, cursors,
//! captures, text slots, forks and gotos, with no directives, outputs or capture predicates. The
//! members' tables (strings, text predicates, capture names) are concatenated and their operands
//! offset.

use super::{
    split, word, Program, TextPredicate, CAPTURE, CHILD, DESCEND, EMIT, FIELD, FLAGS, FOLLOWING, FORK, GOTO, KIND,
    NEXT, PARENT, RESTORE_CURSOR, SAVE_CURSOR, SUPERTYPE,
};

/// The most emits (so members) a fused program has: a set-mode walk tracks each label's first emit
/// in one word (`set_mode::Reach`; the device walk's fired mask is a `uint`).
pub const MEMBERS: usize = 32;

/// A fused program.
pub struct Fused {
    pub program: Program,
    /// By the fused program's emit label: the member and the member's own label.
    pub emits: Vec<(usize, u16)>,
    /// Per member, where its capture labels start among the fused program's.
    pub captures: Vec<u16>,
    /// How many leading words the members share (the trie's shared path).
    pub(in crate::code_rule_dsl) prefix: usize,
    /// Per member, where its branch starts in the fused program (the dispatch past the prefix).
    pub(in crate::code_rule_dsl) starts: Vec<usize>,
}

impl Fused {
    /// `members` as one program, or None where one is not a micro walk, they are fewer than two or
    /// more than `MEMBERS`, or the result would not fit a program. In set mode where every member
    /// is (a member's first emit per start is all its consumer reads).
    pub fn of(members: &[&Program]) -> Option<Self> {
        if members.len() < 2 || members.len() > MEMBERS {
            return None;
        }
        if members.iter().any(|member| {
            super::Micro::of(member).is_none()
                || !member.capture_predicates.is_empty()
                || member.ast_pattern.is_some()
                || !member.longs.is_empty()
        }) {
            return None;
        }
        let prefix = shared_prefix(members);
        let (mut words, mut strings, mut predicates, mut labels) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        // The emit table first: an emit's operand names a label (a string), so the fused emits'
        // labels are the table's first `emits.len()`, each its member's label's name.
        let mut emits = Vec::<(usize, u16)>::new();
        for (k, member) in members.iter().enumerate() {
            for &original in &member.words[prefix..] {
                if let (EMIT, _, arg) = split(original)
                    && !emits.contains(&(k, arg))
                {
                    emits.push((k, arg));
                }
            }
        }
        if emits.len() > MEMBERS {
            return None;
        }
        let mut string_bases = Vec::with_capacity(members.len());
        let mut total = 0_usize;
        for member in members {
            string_bases.push(total);
            total += member.strings.len();
        }
        for &(k, arg) in &emits {
            let name = *members[k].labels.get(usize::from(arg))?;
            labels.push(u16::try_from(usize::from(name) + string_bases[k]).ok()?);
        }
        let (mut captures, mut starts) = (Vec::new(), Vec::new());
        words.extend_from_slice(&members[0].words[..prefix]);
        let last = members.len() - 1;
        for (k, member) in members.iter().enumerate() {
            let (string_base, value_base, label_base) = (strings.len(), predicates.len(), labels.len());
            captures.push(u16::try_from(label_base).ok()?);
            let offset = |index: u16, base: usize| u16::try_from(usize::from(index) + base).ok();
            strings.extend(member.strings.iter().cloned());
            for predicate in &member.predicates {
                predicates.push(match predicate {
                    TextPredicate::Equal(string) => TextPredicate::Equal(offset(*string, string_base)?),
                    TextPredicate::Prefix(string) => TextPredicate::Prefix(offset(*string, string_base)?),
                    TextPredicate::Regex { source, compiled } => TextPredicate::Regex {
                        source: offset(*source, string_base)?,
                        compiled: compiled.clone(),
                    },
                    TextPredicate::LuaPattern { source, compiled } => TextPredicate::LuaPattern {
                        source: offset(*source, string_base)?,
                        compiled: compiled.clone(),
                    },
                });
            }
            for &label in &member.labels {
                labels.push(offset(label, string_base)?);
            }
            // The branch: a fork to the next member's, then this member past the prefix.
            let fork_at = words.len();
            if k < last {
                words.push(0);
            }
            let base = words.len();
            starts.push(base);
            let at = |pc: u16| -> Option<u16> {
                let pc = usize::from(pc);
                (pc >= prefix).then(|| u16::try_from(base + pc - prefix).ok()).flatten()
            };
            for &original in &member.words[prefix..] {
                let (op, mode, arg) = split(original);
                words.push(match op {
                    FORK | GOTO => word(op, mode, at(arg)?),
                    EMIT => word(op, mode, emits.iter().position(|&emit| emit == (k, arg))? as u16),
                    super::TEXT => word(op, mode, offset(arg, value_base)?),
                    CAPTURE => word(op, mode, offset(arg, label_base)?),
                    _ => original,
                });
            }
            if k < last {
                words[fork_at] = word(FORK, 0, u16::try_from(words.len()).ok()?);
            }
        }
        if words.len() > usize::from(u16::MAX) {
            return None;
        }
        let mut program = Program::new(words, strings, predicates, labels, Vec::new()).ok()?;
        if members.iter().all(|member| member.set_mode()) {
            program = program.in_set_mode();
        }
        super::Micro::of(&program)?;
        Some(Self { program, emits, captures, prefix, starts })
    }
}

/// A program's entry test, its first word where that tests its start's kind or supertype: the
/// programs a consumer fuses share one (they start at the same nodes, and share that word).
pub fn entry_key(program: &Program) -> Option<u32> {
    let first = *program.words.first()?;
    matches!(split(first).0, KIND | SUPERTYPE).then_some(first)
}

/// The members' common leading words a walk can share: equal node tests and moves that no member
/// jumps back into (a jump into the prefix would re-enter every member's branch).
fn shared_prefix(members: &[&Program]) -> usize {
    let shared = |op: u8| {
        matches!(op, KIND | FIELD | FLAGS | SUPERTYPE | CHILD | DESCEND | NEXT | FOLLOWING | PARENT | SAVE_CURSOR | RESTORE_CURSOR)
    };
    let mut prefix = 0;
    let shortest = members.iter().map(|member| member.words.len()).min().unwrap_or(0);
    while prefix < shortest {
        let first = members[0].words[prefix];
        if !shared(split(first).0) || members.iter().any(|member| member.words[prefix] != first) {
            break;
        }
        prefix += 1;
    }
    // No jump of any member may land inside the shared words.
    let lowest = members
        .iter()
        .flat_map(|member| member.words.iter())
        .filter_map(|&word| match split(word) {
            (FORK | GOTO, _, arg) => Some(usize::from(arg)),
            _ => None,
        })
        .min()
        .unwrap_or(usize::MAX);
    prefix.min(lowest)
}

#[cfg(test)]
pub mod tests {
    use super::super::Micro;
    use super::*;

    fn program(words: Vec<u32>) -> Program {
        Program::new(words, vec![b"match".to_vec(), b"matcher".to_vec()], Vec::new(), vec![0, 1], Vec::new()).expect("a program")
    }

    #[test]
    fn fused_programs_share_their_prefix_and_fork_per_member() {
        let a = program(vec![word(KIND, 0, 3), word(CHILD, 0, 0), word(KIND, 0, 4), word(EMIT, 0, 1)]);
        let b = program(vec![word(KIND, 0, 3), word(CHILD, 0, 0), word(KIND, 0, 5), word(EMIT, 0, 1)]);
        let fused = Fused::of(&[&a, &b]).expect("fused");
        assert_eq!(fused.prefix, 2);
        let ops = fused.program.words.iter().map(|&w| split(w).0).collect::<Vec<_>>();
        assert_eq!(ops, vec![KIND, CHILD, FORK, KIND, EMIT, KIND, EMIT]);
        assert_eq!(split(fused.program.words[2]).2, 5);
        assert_eq!(split(fused.program.words[4]).2, 0);
        assert_eq!(split(fused.program.words[6]).2, 1);
        assert_eq!(fused.emits, vec![(0, 1), (1, 1)]);
        assert!(Micro::of(&fused.program).is_some());
    }

    #[test]
    fn a_jump_into_the_prefix_stops_it_there() {
        let a = program(vec![word(KIND, 0, 3), word(FORK, 0, 3), word(GOTO, 0, 4), word(KIND, 0, 1), word(EMIT, 0, 1)]);
        let b = program(vec![word(KIND, 0, 3), word(EMIT, 0, 1)]);
        let fused = Fused::of(&[&a, &b]).expect("fused");
        assert_eq!(fused.prefix, 1);
        // The chain's fork at 1; a's branch from 2, its fork to its pc 3 at 2 + 3 - 1.
        assert_eq!(split(fused.program.words[1]), (FORK, 0, 6));
        assert_eq!(split(fused.program.words[2]), (FORK, 0, 4));
        assert_eq!(split(fused.program.words[3]), (GOTO, 0, 5));
    }
}
