//! Finite normal-loop jobs. NEXT strictly advances, static guards cannot change
//! a frame, and all exits retain their exact logical branch positions.
use super::*;

pub(super) const MAX_JOB_INSTRUCTIONS: usize = super::MAX_FINITE_JOB_INSTRUCTIONS;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct AdmissionCut {
    /// Index before the first instruction already owned by an earlier path.
    pub at: usize,
    /// Read-only guards that prove an earlier equal frame reaches this key.
    pub alternatives: Vec<Vec<usize>>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct Body {
    pub operations: Vec<usize>,
    pub choices: Vec<u32>,
    /// Complete executed path, including control instructions and their choices.
    pub instructions: Vec<(usize, Option<u32>)>,
    pub admission: Vec<AdmissionCut>,
}

struct BodyState {
    pc: usize,
    captures: Vec<u16>,
    choices: Vec<u32>,
    guards: Vec<usize>,
}

fn is_guard(op: u8) -> bool {
    matches!(
        op,
        KIND | FIELD | FLAGS | SIZE | FIRST | LAST | ABSENT_FIELD | SUPERTYPE
    )
}

/// Every body uses the same advanced node and immutable input frame. Only
/// ordered capture labels can distinguish its intermediate query keys.
fn admission_cuts(words: &[u32], bodies: &mut [Body]) {
    let states = bodies
        .iter()
        .map(|body| {
            let mut captures = Vec::new();
            let mut choices = Vec::new();
            let mut guards = Vec::new();
            body.instructions
                .iter()
                .map(|&(pc, choice)| {
                    let state = BodyState {
                        pc,
                        captures: captures.clone(),
                        choices: choices.clone(),
                        guards: guards.clone(),
                    };
                    let (op, _, arg) = split(words[pc]);
                    if op == CAPTURE {
                        captures.push(arg);
                    }
                    if is_guard(op) {
                        guards.push(pc);
                    }
                    if let Some(choice) = choice {
                        choices.push(choice);
                    }
                    state
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    for (index, body) in bodies.iter_mut().enumerate() {
        for (at, state) in states[index].iter().enumerate() {
            let mut alternatives = BTreeSet::new();
            for earlier in states[..index].iter().flatten() {
                if earlier.pc != state.pc || earlier.captures != state.captures {
                    continue;
                }
                // Shared prefixes describe one CPU event. Admission starts only
                // after an actual earlier/later FORK decision diverges.
                let earlier_choice = earlier
                    .choices
                    .iter()
                    .zip(&state.choices)
                    .find_map(|(a, b)| (a != b).then_some(a < b));
                if earlier_choice == Some(true) {
                    alternatives.insert(earlier.guards.clone());
                }
            }
            if !alternatives.is_empty() {
                body.admission.push(AdmissionCut {
                    at,
                    alternatives: alternatives.into_iter().collect(),
                });
            }
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct Job {
    pub header: usize,
    pub exit: usize,
    pub named: bool,
    pub repeat: u16,
    pub bodies: Vec<Body>,
    pub disjoint: bool,
    pub iterations: usize,
    pub variants: usize,
}

pub(super) fn prove(words: &[u32], header: usize) -> Option<Job> {
    if header > usize::from(u16::MAX) {
        return None;
    }
    let (op, mode, exit) = split(*words.get(header)?);
    if op != FORK || mode != 0 {
        return None;
    }
    let (op, mode, repeat) = split(*words.get(usize::from(exit))?);
    if op != REPEAT_END || mode != 0 {
        return None;
    }
    let (op, mode, _) = split(*words.get(header + 1)?);
    if op != NEXT {
        return None;
    }
    let named = mode == NAMED;
    let mut pending = vec![(
        header + 2,
        Vec::new(),
        Vec::new(),
        BTreeSet::new(),
        Vec::new(),
    )];
    let mut bodies = Vec::new();
    while let Some((mut pc, mut operations, mut choices, mut seen, mut instructions)) =
        pending.pop()
    {
        loop {
            if seen.len() >= 32 || choices.len() >= 16 || !seen.insert(pc) {
                return None;
            }
            let (op, mode, arg) = split(*words.get(pc)?);
            match op {
                REPEAT_BUMP if arg == repeat => {
                    if pc + 1 != header
                        && words.get(pc + 1).copied() != Some(word(GOTO, 0, header as u16))
                    {
                        return None;
                    }
                    instructions.push((pc, None));
                    if pc + 1 != header {
                        instructions.push((pc + 1, None));
                    }
                    bodies.push(Body {
                        operations,
                        choices,
                        instructions,
                        admission: Vec::new(),
                    });
                    break;
                }
                FORK if mode == 0 => {
                    let mut other = choices.clone();
                    other.push(1);
                    let mut other_instructions = instructions.clone();
                    other_instructions.push((pc, Some(1)));
                    pending.push((
                        usize::from(arg),
                        operations.clone(),
                        other,
                        seen.clone(),
                        other_instructions,
                    ));
                    choices.push(0);
                    instructions.push((pc, Some(0)));
                }
                GOTO => {
                    instructions.push((pc, None));
                    pc = usize::from(arg);
                    continue;
                }
                KIND | FIELD | FLAGS | SIZE | FIRST | LAST | ABSENT_FIELD | SUPERTYPE | CAPTURE => {
                    operations.push(pc);
                    instructions.push((pc, None));
                }
                _ => return None,
            }
            if bodies.len() + pending.len() > 16 {
                return None;
            }
            pc += 1;
        }
    }
    if bodies.is_empty() {
        return None;
    }
    admission_cuts(words, &mut bodies);
    let mut kinds = BTreeSet::new();
    let disjoint = bodies.len() == 1
        || bodies.iter().all(|body| {
            let found = body
                .operations
                .iter()
                .filter_map(|&pc| {
                    let (op, _, arg) = split(words[pc]);
                    (op == KIND).then_some(arg)
                })
                .collect::<BTreeSet<_>>();
            found.len() == 1 && kinds.insert(*found.first().unwrap())
        });
    let captures = bodies
        .iter()
        .map(|body| {
            body.operations
                .iter()
                .filter(|&&pc| split(words[pc]).0 == CAPTURE)
                .count()
        })
        .max()?;
    let digits = bodies.iter().map(|body| body.choices.len() + 1).max()?;
    let instructions = bodies
        .iter()
        .map(|body| body.instructions.len() + 2)
        .max()?;
    // An exit appends one additional digit after all body choices.
    let limit = 32
        .min(32 / captures.max(1))
        .min(63 / digits)
        .min(MAX_JOB_INSTRUCTIONS / instructions);
    let mut iterations = 0;
    let mut variants = 1;
    let mut leaves = 1;
    for depth in 1..=limit {
        leaves *= if disjoint { 1 } else { bodies.len() };
        if variants + leaves > 64 {
            break;
        }
        variants += leaves;
        iterations = depth;
    }
    (iterations > 0).then_some(Job {
        header,
        exit: usize::from(exit),
        named,
        repeat,
        bodies,
        disjoint,
        iterations,
        variants,
    })
}
