//! Finite normal-fork work. Each path stops at its next admission boundary.
use super::*;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct Branch {
    operations: Vec<usize>,
    /// Operation offset and branch digit, in execution order.
    choices: Vec<(usize, u32)>,
    end: usize,
}

pub(super) fn edit_shape(paths: &[Branch], words: &[u32]) -> [usize; 4] {
    let mut maximum = [0; 4];
    for path in paths {
        let shape = super::metal_successor_codegen::edit_shape(
            path.operations.iter().copied(),
            words,
            path.choices.len(),
        );
        for (bound, value) in maximum.iter_mut().zip(shape) {
            *bound = (*bound).max(value);
        }
    }
    maximum
}

pub(super) fn prove(words: &[u32], readiness: &[[u32; 2]], start: usize) -> Option<Vec<Branch>> {
    words.get(start)?;
    let mut branched = false;
    // Only normal control flow permits deferred full-state admission.
    if words.iter().any(|&w| matches!(split(w), (FORK, GREEDY, _))) {
        return None;
    }
    let mut pending = vec![(start, Vec::new(), Vec::new(), BTreeSet::new(), 0usize)];
    let mut paths = Vec::new();
    while let Some((mut pc, mut operations, mut choices, mut seen, mut edits)) = pending.pop() {
        loop {
            if seen.len() >= 64
                || choices.len() >= 63
                || !seen.insert(pc)
                || (pc != start && readiness.get(pc)?[0] != u32::MAX)
                || (pc != start && super::bounded_repeat_proof::prove(words, pc).is_some())
            {
                break;
            }
            let (op, mode, arg) = split(*words.get(pc)?);
            if op == FORK && mode == GREEDY {
                break;
            }
            match op {
                FORK => {
                    branched = true;
                    operations.push(pc);
                    let instruction = operations.len() - 1;
                    let mut other = choices.clone();
                    other.push((instruction, 1));
                    pending.push((
                        usize::from(arg),
                        operations.clone(),
                        other,
                        seen.clone(),
                        edits,
                    ));
                    choices.push((instruction, 0));
                    pc += 1;
                    if pending.len() + paths.len() >= 32 {
                        return None;
                    }
                    continue;
                }
                GOTO => {
                    operations.push(pc);
                    pc = usize::from(arg);
                    continue;
                }
                SAVE_CURSOR | CAPTURE | REPEAT_BEGIN => {
                    if edits == 32 {
                        break;
                    }
                    edits += 1;
                }
                _ => {}
            }
            operations.push(pc);
            if matches!(op, STOP | EMIT) {
                break;
            }
            pc += 1;
            if op == TEXT {
                // Successful finite work resumes after the operation. A
                // suspended operation returns before the generated increment.
                break;
            }
            if matches!(op, CHILD | DESCEND | FOLLOWING) {
                break;
            }
        }
        paths.push(Branch {
            operations,
            choices,
            end: pc,
        });
    }
    branched.then_some(paths)
}

pub(super) fn definition(paths: &[Branch], words: &[u32], signature: u32) -> Result<String, Error> {
    let mut source = format!(
        "__attribute__((noinline)) void branch_prefix{signature}(View v,Program pg,device const uint4* cursors,device const uint4* repeats,uint cs,uint rs,uint variant,device Step& x) {{switch(variant){{\n"
    );
    for (variant, path) in paths.iter().enumerate() {
        writeln!(source, "case {variant}u:{{").unwrap();
        for (index, &pc) in path.operations.iter().enumerate() {
            writeln!(source,"{{x.pc={pc}u;if(!successor_charge(x))return;uint arg=v.words[pg.structural_start+{pc}u]>>16;(void)arg;").unwrap();
            for &(_, choice) in path.choices.iter().filter(|&&(at, _)| at == index) {
                writeln!(source, "step_branches(x)[x.branch_count++]={choice}u;").unwrap();
            }
            let (op, mode, _) = split(words[pc]);
            let line = match op {
                FORK => String::new(),
                GOTO => "x.pc=arg;".to_owned(),
                _ => super::metal_successor_codegen::instruction_line(op, mode, false, true)?,
            };
            writeln!(
                source,
                "{line}if(x.error!=NONE || !x.live)return;{}}}",
                if op == GOTO { "" } else { "x.pc++;" }
            )
            .unwrap();
        }
        for &(_, choice) in path
            .choices
            .iter()
            .filter(|&&(at, _)| at == path.operations.len())
        {
            writeln!(source, "step_branches(x)[x.branch_count++]={choice}u;").unwrap();
        }
        writeln!(source, "x.pc={}u;return;}}", path.end).unwrap();
    }
    source.push_str("}x.live=0u;}\n");
    Ok(source)
}
