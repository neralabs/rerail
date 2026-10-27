//! Terminal repetition proof: disjoint static node alternatives, nonempty
//! captures on every iteration, and one unconditional terminal continuation.
use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TerminalRepeat {
    pub begin: usize,
    pub children: bool,
    pub named: bool,
    pub guards: Vec<usize>,
    pub prefix: Vec<usize>,
    pub suffix: Vec<usize>,
    pub paths: Vec<Vec<usize>>,
    pub initial_work: Vec<Vec<usize>>,
    pub repeating_work: Vec<Vec<usize>>,
    /// Closing END and complete terminal continuation, including EMIT.
    pub suffix_work: usize,
    pub emit: usize,
}

fn guard(op: u8) -> bool {
    matches!(
        op,
        KIND | FIELD | FLAGS | SIZE | FIRST | LAST | ABSENT_FIELD | SUPERTYPE
    )
}

type BodyPaths = Vec<(Vec<usize>, Vec<usize>)>;

fn paths(program: &Program, start: usize, header: usize, repeat: u16) -> Option<BodyPaths> {
    let mut pending = vec![(start, Vec::new(), BTreeSet::new(), Vec::new())];
    let mut output = Vec::new();
    while let Some((mut pc, mut path, mut seen, mut instructions)) = pending.pop() {
        loop {
            if seen.len() >= 64 || output.len() + pending.len() > 32 || !seen.insert(pc) {
                return None;
            }
            let (op, mode, arg) = split(*program.words.get(pc)?);
            instructions.push(pc);
            match op {
                REPEAT_BUMP if arg == repeat => {
                    let next = pc + 1;
                    if next != header
                        && program.words.get(next).copied() != Some(word(GOTO, 0, header as u16))
                    {
                        return None;
                    }
                    if next != header {
                        instructions.push(next);
                    }
                    output.push((path, instructions));
                    break;
                }
                FORK if mode == 0 => pending.push((
                    usize::from(arg),
                    path.clone(),
                    seen.clone(),
                    instructions.clone(),
                )),
                GOTO => {
                    pc = usize::from(arg);
                    continue;
                }
                ASSERT => {}
                CAPTURE => path.push(pc),
                _ if guard(op) => path.push(pc),
                _ => return None,
            }
            pc += 1;
        }
    }
    Some(output)
}

pub(super) fn prove(program: &Program) -> Option<TerminalRepeat> {
    if !program.tree_sitter_query {
        return None;
    }
    let headers = program
        .words
        .iter()
        .enumerate()
        .filter_map(|(pc, &w)| {
            let (op, mode, exit) = split(w);
            (op == FORK
                && mode == 0
                && pc > 0
                && split(program.words[pc - 1]).0 == REPEAT_BUMP
                && program
                    .words
                    .get(pc + 1)
                    .is_some_and(|&next| split(next).0 == NEXT)
                && program
                    .words
                    .get(usize::from(exit))
                    .is_some_and(|&end| split(end).0 == REPEAT_END))
            .then_some((pc, usize::from(exit)))
        })
        .collect::<Vec<_>>();
    let &(header, exit) = headers.first()?;
    if headers.len() != 1 || header >= exit || header > usize::from(u16::MAX) {
        return None;
    }
    let repeat = split(program.words[header - 1]).2;
    let begin = program
        .words
        .iter()
        .position(|&w| w == word(REPEAT_BEGIN, 0, repeat))?;
    if begin > 64 {
        return None;
    }
    let mut guards = Vec::new();
    let mut prefix = Vec::new();
    let mut saved = false;
    for pc in 0..begin {
        let (op, _, arg) = split(program.words[pc]);
        match op {
            SAVE_CURSOR if arg == 0 => saved = true,
            CAPTURE => prefix.push(pc),
            ASSERT => {}
            _ if guard(op) => guards.push(pc),
            _ => return None,
        }
    }
    if !saved {
        return None;
    }
    let (_, next_mode, _) = split(program.words[header + 1]);
    let mut first = begin + 1;
    let children = split(*program.words.get(first)?).0 == CHILD;
    if children {
        if split(program.words[first]).1 != next_mode {
            return None;
        }
        first += 1;
    }
    let initial = paths(program, first, header, repeat)?;
    let repeating = paths(program, header + 2, header, repeat)?;
    let normalize = |paths: &BodyPaths| {
        paths
            .iter()
            .map(|(path, _)| path.iter().map(|&pc| program.words[pc]).collect::<Vec<_>>())
            .collect::<Vec<_>>()
    };
    if initial.is_empty() || normalize(&initial) != normalize(&repeating) {
        return None;
    }
    let mut kinds = BTreeSet::new();
    for (path, _) in &initial {
        if !path.iter().any(|&pc| split(program.words[pc]).0 == CAPTURE) {
            return None;
        }
        let found = path
            .iter()
            .filter_map(|&pc| {
                let (op, _, arg) = split(program.words[pc]);
                (op == KIND).then_some(arg)
            })
            .collect::<BTreeSet<_>>();
        if initial.len() > 1 && (found.len() != 1 || !kinds.insert(*found.first()?)) {
            return None;
        }
    }
    if program.words[exit] != word(REPEAT_END, 0, repeat) {
        return None;
    }
    let mut pc = exit + 1;
    let mut suffix = Vec::new();
    let mut restored = false;
    let mut visited = BTreeSet::new();
    let mut suffix_work = 1;
    loop {
        if visited.len() > 64 || !visited.insert(pc) {
            return None;
        }
        let (op, _, arg) = split(*program.words.get(pc)?);
        suffix_work += 1;
        match op {
            GOTO => {
                pc = usize::from(arg);
                continue;
            }
            RESTORE_CURSOR if arg == 0 => restored = true,
            CAPTURE if restored => suffix.push(pc),
            ASSERT => {}
            EMIT if restored => break,
            _ => return None,
        }
        pc += 1;
    }
    if prefix.len() + suffix.len() > 32 {
        return None;
    }
    Some(TerminalRepeat {
        begin,
        children,
        named: next_mode == NAMED,
        guards,
        prefix,
        suffix,
        paths: initial.iter().map(|(path, _)| path.clone()).collect(),
        initial_work: initial.into_iter().map(|(_, work)| work).collect(),
        repeating_work: repeating.into_iter().map(|(_, work)| work).collect(),
        suffix_work,
        emit: pc,
    })
}
