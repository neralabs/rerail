//! One input-bound linear proof serves seed eligibility and initial execution.
use super::metal_child_index::{ChildIndex, InitialChild, InitialSelector};
use super::metal_grid_plan::ResidentInput;
use super::*;

pub(super) const PREFIX_EDITS: usize = 32;

pub(super) fn incoming_counts(words: &[u32]) -> Vec<usize> {
    let mut incoming = vec![0usize; words.len()];
    if let Some(first) = incoming.first_mut() {
        *first = 1;
    }
    for (pc, &word) in words.iter().enumerate() {
        let (op, _, target) = split(word);
        let mut edge = |next: usize| {
            if let Some(count) = incoming.get_mut(next) {
                *count += 1;
            }
        };
        match op {
            STOP | EMIT => {}
            GOTO => edge(usize::from(target)),
            FORK => {
                edge(pc + 1);
                edge(usize::from(target));
            }
            _ => edge(pc + 1),
        }
    }
    incoming
}

/// One original seed supplies one path before the first branch or control join.
/// Generic fan-out and later generations retain their existing barriers.
pub(super) struct InitialRegion {
    pub end: usize,
    pub children: BTreeMap<usize, InitialChild>,
}

pub(super) fn prove(words: &[u32], query: bool, children: &ChildIndex) -> Option<InitialRegion> {
    let incoming = incoming_counts(words);
    if incoming.first() != Some(&1) {
        return None;
    }
    let mut selected = BTreeMap::new();
    let mut path = Vec::new();
    let mut end = 0;
    let mut edits = 0;
    let mut guard_work = 0usize;
    for (pc, &word) in words.iter().enumerate().take(64) {
        let (op, _, target) = split(word);
        if (pc != 0 && incoming[pc] != 1) || (query && op == FORK) {
            break;
        }
        edits += usize::from(matches!(op, SAVE_CURSOR | REPEAT_BEGIN | CAPTURE));
        if edits > PREFIX_EDITS {
            break;
        }
        let child = (op == CHILD)
            .then(|| children.initial_child(words, &path, pc))
            .flatten();
        if let Some(child) = child {
            guard_work = guard_work.saturating_add(
                child
                    .maximum_children
                    .saturating_mul(child.guard_offset + 1),
            );
            if guard_work.saturating_add(64) > MAX_WORK_PER_TASK {
                break;
            }
            selected.insert(pc, child);
        }
        path.push(pc);
        end = pc + 1;
        if (op == CHILD && child.is_none())
            || matches!(op, STOP | EMIT | FORK | DESCEND | FOLLOWING | TEXT)
            || (op == ASSERT && !query)
            || (op == GOTO && usize::from(target) != pc + 1)
        {
            break;
        }
    }
    (!selected.is_empty()).then_some(InitialRegion {
        end,
        children: selected,
    })
}

/// Generated guards only reject roots that cannot reach an observable result
/// or failure. A semantic failure or shared predicate stops this proof.
fn guard_body(words: &[u32], flags: u32, region: &InitialRegion) -> Result<String, Error> {
    let mut body = "uint n0=root;\n".to_owned();
    let mut current = 0usize;
    let mut next = 1usize;
    let mut cursors = BTreeMap::new();
    let mut repeats = Vec::new();
    for (pc, &word) in words[..region.end].iter().enumerate() {
        let (op, mode, operand) = split(word);
        let node = format!("n{current}");
        let arg = format!("(v.words[pg.structural_start+{pc}u]>>16)");
        let condition = match op {
            KIND => Some(format!("v.nodes[{node}].kind=={arg}")),
            FIELD => Some(format!("v.nodes[{node}].field=={arg}")),
            FLAGS => Some(format!(
                "(v.nodes[{node}].flags&{mode}u)=={mode}u && !(v.nodes[{node}].flags&{arg})"
            )),
            SIZE => {
                let comparison = match mode & !SIZE_LONG {
                    0 => "==",
                    1 => ">=",
                    2 => "<=",
                    _ => return Err(Error::BackendFailure),
                };
                let size = if mode & SIZE_LONG == 0 {
                    format!("ulong({arg})")
                } else {
                    format!("v.longs[pg.long_start+{arg}]")
                };
                Some(format!(
                    "ulong(v.nodes[{node}].source_end-v.nodes[{node}].source_start){comparison}{size}"
                ))
            }
            FIRST | LAST => Some(format!(
                "at_edge(v,{node},{}u,{}u)",
                u32::from(op == LAST),
                u32::from(mode == NAMED)
            )),
            ABSENT_FIELD => Some(format!(
                "!has_pair(v.fields,v.header->field_count,{node},{arg})"
            )),
            SUPERTYPE => Some(format!(
                "has_pair(v.supertypes,v.header->supertype_count,{node},{arg})"
            )),
            SAVE_CURSOR => {
                cursors.retain(|&slot, _| slot <= operand);
                cursors.insert(operand, current);
                None
            }
            RESTORE_CURSOR => {
                let Some(&saved) = cursors.get(&operand) else {
                    break;
                };
                current = saved;
                None
            }
            REPEAT_BEGIN => {
                repeats.push(operand);
                None
            }
            REPEAT_BUMP | REPEAT_END => {
                if repeats.last() != Some(&operand) {
                    break;
                }
                if op == REPEAT_END {
                    repeats.pop();
                }
                None
            }
            CHILD => {
                let Some(child) = region.children.get(&pc) else {
                    break;
                };
                writeln!(
                    body,
                    "if(v.nodes[{node}].child_count>{}u)return INITIAL_ROOT_INVALID;",
                    child.maximum_children
                )
                .unwrap();
                let expression = match child.selector {
                    InitialSelector::Field | InitialSelector::Kind => format!(
                        "unique_child(v,{node},v.words[pg.structural_start+{}u]>>16,{}u)",
                        pc + child.guard_offset,
                        child.selector as u8
                    ),
                    InitialSelector::FirstAny => format!(
                        "v.nodes[{node}].child_count?v.children[v.nodes[{node}].child_start]:NONE"
                    ),
                    InitialSelector::FirstNamed => format!("v.relations[{node}].x"),
                };
                writeln!(
                    body,
                    "uint n{next}={expression};if(n{next}==NONE)return INITIAL_ROOT_DEAD;"
                )
                .unwrap();
                writeln!(body,"if(n{next}<file_start || n{next}-file_start>=file_count || v.nodes[n{next}].parent!={node})return INITIAL_ROOT_INVALID;").unwrap();
                if mode == NAMED {
                    writeln!(
                        body,
                        "if(!(v.nodes[n{next}].flags&1u))return INITIAL_ROOT_DEAD;"
                    )
                    .unwrap();
                }
                current = next;
                next += 1;
                None
            }
            PARENT | NEXT => {
                let field = if op == PARENT {
                    "parent"
                } else if mode == NAMED {
                    "next_named"
                } else {
                    "next_any"
                };
                writeln!(body,"uint n{next}=v.nodes[{node}].{field};if(n{next}==NONE)return INITIAL_ROOT_DEAD;").unwrap();
                writeln!(body,"if(n{next}<file_start || n{next}-file_start>=file_count)return INITIAL_ROOT_INVALID;").unwrap();
                current = next;
                next += 1;
                None
            }
            CAPTURE => None,
            ASSERT if flags & 1 != 0 => None,
            GOTO if usize::from(operand) == pc + 1 => None,
            STOP => {
                body.push_str("return INITIAL_ROOT_DEAD;\n");
                return Ok(body);
            }
            _ => break,
        };
        if let Some(condition) = condition {
            writeln!(body, "if(!({condition}))return INITIAL_ROOT_DEAD;").unwrap();
        }
    }
    body.push_str("return INITIAL_ROOT_KEEP;\n");
    Ok(body)
}

pub(super) fn guards(resident: &ResidentInput) -> Result<String, Error> {
    let packed = &resident.input.packed;
    resident.input.children.validate(packed)?;
    let mut source =
        "constant uint INITIAL_ROOT_DEAD=0u,INITIAL_ROOT_KEEP=1u,INITIAL_ROOT_INVALID=2u;\n"
            .to_owned();
    let mut functions = BTreeMap::new();
    let mut cases = String::new();
    for (index, pg) in packed.programs.iter().enumerate() {
        if resident.input.children.eligible(index) {
            continue;
        }
        let start = pg.structural_start as usize;
        let words = &packed.words[start..start + pg.structural_count as usize];
        let Some(region) = prove(words, pg.flags & 2 != 0, &resident.input.children) else {
            continue;
        };
        let body = guard_body(words, pg.flags, &region)?;
        let next = functions.len();
        let signature = *functions.entry(body.clone()).or_insert(next);
        if signature == next {
            writeln!(source,"uint initial_guard{signature}(View v,Program pg,uint root,uint file_start,uint file_count){{{body}}}").unwrap();
        }
        writeln!(cases,"case {index}u:return initial_guard{signature}(v,v.programs[program],node,file_start,file_count);").unwrap();
    }
    writeln!(source,"uint initial_root_status(View v,uint program,uint node,uint file_start,uint file_count){{if(program>=v.header->program_count)return INITIAL_ROOT_INVALID;switch(program){{{cases}default:return INITIAL_ROOT_KEEP;}}}}").unwrap();
    Ok(source)
}
