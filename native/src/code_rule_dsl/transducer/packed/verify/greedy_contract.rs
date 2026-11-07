//! Query-mode greedy exits belong to capture-free, single-entry repetition regions.
//!
//! Ordinary non-query GREEDY branches retain their multiplicity contract. Operand
//! and cycle validation run first. This proof never changes words or query results.
use super::{CAPTURE, EMIT, Error, FORK, GOTO, GREEDY, NEXT, REPEAT_BUMP, REPEAT_END, STOP, split};

fn successors(words: &[u32], pc: usize) -> [Option<usize>; 2] {
    let (opcode, _, operand) = split(words[pc]);
    let next = (pc + 1 < words.len()).then_some(pc + 1);
    match opcode {
        EMIT | STOP => [None, None],
        GOTO => [Some(usize::from(operand)), None],
        FORK => [next, Some(usize::from(operand))],
        _ => [next, None],
    }
}

pub(super) fn validate(words: &[u32]) -> Result<(), Error> {
    ordered_headers(words).map(|_| ())
}

/// Validated GREEDY headers and their control-flow component order. Each exit
/// precedes any region that it can enter. Callers first validate word operands.
pub(super) fn ordered_headers(words: &[u32]) -> Result<Vec<(usize, usize)>, Error> {
    if !words.iter().any(|&w| matches!(split(w), (FORK, GREEDY, _))) {
        return Ok(Vec::new());
    }
    let mut predecessors = vec![Vec::new(); words.len()];
    let mut reaches_capture = vec![false; words.len()];
    let mut pending = Vec::new();
    for (pc, &instruction) in words.iter().enumerate() {
        for successor in successors(words, pc).into_iter().flatten() {
            predecessors[successor].push(pc);
        }
        if split(instruction).0 == CAPTURE {
            reaches_capture[pc] = true;
            pending.push(pc);
        }
    }
    // Reverse reachability checks both branches and all later continuations. Each
    // instruction and edge is visited once, regardless of the number of exits.
    while let Some(pc) = pending.pop() {
        for &predecessor in &predecessors[pc] {
            if !reaches_capture[predecessor] {
                reaches_capture[predecessor] = true;
                pending.push(predecessor);
            }
        }
    }
    let components = components(words, &predecessors);
    let mut headers = Vec::new();
    let mut owned = vec![false; words.len()];
    for (pc, &instruction) in words.iter().enumerate() {
        let (FORK, GREEDY, exit) = split(instruction) else {
            continue;
        };
        let exit = usize::from(exit);
        let fail = || Error::InvalidGreedyRegion(pc);
        // The direct predecessor check below cannot detect an exit that returns
        // through the preceding BUMP. Such an exit is not a terminal repetition.
        if pc == 0 || exit < pc + 4 || reaches_capture[pc] || components[pc] == components[exit] {
            return Err(fail());
        }
        let (REPEAT_BUMP, 0, repeat) = split(words[pc - 1]) else {
            return Err(fail());
        };
        if split(words[pc + 1]) != (NEXT, 0, 0)
            || split(words[exit - 2]) != (REPEAT_BUMP, 0, repeat)
            || split(words[exit - 1]) != (GOTO, 0, pc as u16)
            || split(words[exit]) != (REPEAT_END, 0, repeat)
            || predecessors[pc]
                .iter()
                .any(|&from| from != pc - 1 && from != exit - 1)
        {
            return Err(fail());
        }
        headers.push((pc, components[pc]));
        // Every body entry comes from this region. Only its final returning edge
        // can leave the body, and it returns to this header. Regions do not nest.
        for at in pc..exit {
            if std::mem::replace(&mut owned[at], true) {
                return Err(fail());
            }
            if at == pc {
                continue;
            }
            if predecessors[at]
                .iter()
                .any(|&from| from < pc || from >= exit)
            {
                return Err(fail());
            }
            for next in successors(words, at).into_iter().flatten() {
                if (at != exit - 1 && !(pc + 1..exit).contains(&next))
                    || (at == exit - 1 && next != pc)
                {
                    return Err(fail());
                }
            }
            if matches!(split(words[at]), (EMIT, _, _) | (FORK, GREEDY, _)) {
                return Err(fail());
            }
        }
    }
    Ok(headers)
}

/// Budget-sensitive acyclic joins in a validated query. A complete acyclic
/// path with no early value work executes at most words.len() instructions.
/// Otherwise a join stays active before a different budget can exhaust.
#[cfg(target_os = "macos")]
pub(super) fn acyclic_joins(program: &super::Program) -> Vec<(usize, usize)> {
    let words = &program.words;
    let mut predecessors = vec![Vec::new(); words.len()];
    for pc in 0..words.len() {
        for next in successors(words, pc).into_iter().flatten() {
            predecessors[next].push(pc);
        }
    }
    let order = components(words, &predecessors);
    let mut sizes = vec![0usize; words.len()];
    for &component in &order {
        sizes[component] += 1;
    }
    let cyclic = predecessors
        .iter()
        .enumerate()
        .any(|(pc, incoming)| sizes[order[pc]] > 1 || incoming.contains(&pc));
    let early_value_work = words.iter().any(|&word| {
        let (op, _, arg) = split(word);
        op == super::TEXT
            && matches!(
                program.value_programs[usize::from(arg)],
                super::value::ValueProgram::Regex(_) | super::value::ValueProgram::Lua(_)
            )
    });
    // Resident execution starts each root at zero work. Query ASSERT values
    // run later with their own budget; literal TEXT adds no value work.
    if !cyclic && !early_value_work && words.len() <= super::MAX_STEPS_PER_PATH {
        return Vec::new();
    }
    predecessors
        .iter()
        .enumerate()
        .filter_map(|(pc, incoming)| {
            (incoming.len() > 1 && sizes[order[pc]] == 1 && !incoming.contains(&pc))
                .then_some((pc, order[pc]))
        })
        .collect()
}

// Iterative Kosaraju traversal avoids a call stack proportional to Program size.
// Both passes visit every instruction and edge a bounded number of times.
fn components(words: &[u32], predecessors: &[Vec<usize>]) -> Vec<usize> {
    let mut visited = vec![false; words.len()];
    let mut finished = Vec::with_capacity(words.len());
    for root in 0..words.len() {
        if visited[root] {
            continue;
        }
        visited[root] = true;
        let mut pending = vec![(root, 0)];
        while let Some((at, edge)) = pending.last_mut() {
            if *edge == 2 {
                finished.push(*at);
                pending.pop();
                continue;
            }
            let next = successors(words, *at)[*edge];
            *edge += 1;
            if let Some(next) = next
                && !std::mem::replace(&mut visited[next], true)
            {
                pending.push((next, 0));
            }
        }
    }
    let mut components = vec![usize::MAX; words.len()];
    let mut component = 0;
    for root in finished.into_iter().rev() {
        if components[root] != usize::MAX {
            continue;
        }
        components[root] = component;
        let mut pending = vec![root];
        while let Some(at) = pending.pop() {
            for &previous in &predecessors[at] {
                if components[previous] == usize::MAX {
                    components[previous] = component;
                    pending.push(previous);
                }
            }
        }
        component += 1;
    }
    components
}
