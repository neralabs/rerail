//! Finite initial units over virtual child, descendant, and sibling ranges.
use super::metal_child_index::InitialChild;
use super::metal_grid_plan::ResidentInput;
use super::*;

/// Disjoint whole-program classes share the same initial range operator.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WorkClass {
    All,
    Continuing,
    Completed,
}

fn complete_programs(resident: &ResidentInput) -> Result<Vec<bool>, Error> {
    resident.input.children.validate(&resident.input.packed)?;
    Ok(resident
        .input
        .packed
        .programs
        .iter()
        .map(|pg| {
            let start = pg.structural_start as usize;
            let words = &resident.input.packed.words[start..start + pg.structural_count as usize];
            let unit = prove(words, pg.flags, &resident.input.children);
            complete(&unit)
        })
        .collect())
}

fn complete(unit: &Unit) -> bool {
    unit.prefix
        .iter()
        .chain(&unit.suffix)
        .next_back()
        .is_some_and(|operation| matches!(operation.op, EMIT | STOP))
}

pub(super) fn mixed_classes(
    resident: &ResidentInput,
    plan: &super::metal_grid_plan::ExecutionPlan,
) -> Result<bool, Error> {
    plan.validate(resident)?;
    let complete = complete_programs(resident)?;
    let grid = &resident.input.grid;
    let mut classes = 0u8;
    for id in super::metal_frontier_seed::blocks(plan)? {
        let block = grid.blocks.get(id as usize).ok_or(Error::BackendFailure)?;
        let end = block
            .block_span_start
            .checked_add(block.block_span_count)
            .ok_or(Error::BackendFailure)?;
        for span in grid
            .block_spans
            .get(block.block_span_start as usize..end as usize)
            .ok_or(Error::BackendFailure)?
        {
            let evaluation = grid
                .evaluation_spans
                .get(span.evaluation_span as usize)
                .ok_or(Error::BackendFailure)?;
            let completed = *complete
                .get(evaluation.program as usize)
                .ok_or(Error::BackendFailure)?;
            classes |= if completed { 1 } else { 2 };
        }
    }
    Ok(classes == 3)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Operation {
    pc: usize,
    op: u8,
    mode: u8,
    child: Option<InitialChild>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Unit {
    prefix: Vec<Operation>,
    suffix: Vec<Operation>,
    variable: bool,
    query: bool,
}

fn prove(words: &[u32], flags: u32, children: &super::metal_child_index::ChildIndex) -> Unit {
    let query = flags & 2 != 0;
    let incoming = super::metal_initial_prefix::incoming_counts(words);
    let mut unit = Unit {
        prefix: vec![],
        suffix: vec![],
        variable: false,
        query: flags & 1 != 0,
    };
    if incoming.first() != Some(&1) {
        return unit;
    }
    let mut path = Vec::new();
    let mut cursors = BTreeSet::new();
    let mut repeats = Vec::new();
    let mut edits = 0usize;
    let mut guards = 0usize;
    for (pc, &word) in words.iter().enumerate().take(64) {
        let (op, mode, arg) = split(word);
        if (pc != 0 && incoming[pc] != 1)
            || matches!(op, FORK | TEXT)
            || (op == ASSERT && flags & 1 == 0)
            || (op == GOTO && usize::from(arg) != pc + 1)
        {
            break;
        }
        if unit.variable && query && matches!(op, RESTORE_CURSOR | PARENT | NEXT) {
            break;
        }
        if !unit.variable {
            match op {
                RESTORE_CURSOR if !cursors.contains(&arg) => break,
                REPEAT_BUMP | REPEAT_END if repeats.last() != Some(&arg) => break,
                _ => {}
            }
        }
        let next_edits = edits + usize::from(matches!(op, SAVE_CURSOR | REPEAT_BEGIN | CAPTURE));
        if next_edits > super::metal_successor_codegen::PREFIX_EDITS {
            break;
        }
        let child = (op == CHILD)
            .then(|| children.initial_child(words, &path, pc))
            .flatten();
        if let Some(child) = child {
            guards = guards.saturating_add(
                child
                    .maximum_children
                    .saturating_mul(child.guard_offset + 1),
            );
            if guards.saturating_add(128) > MAX_WORK_PER_TASK {
                break;
            }
        }
        let variable = matches!(op, CHILD | DESCEND | FOLLOWING) && child.is_none();
        if unit.variable && variable {
            break;
        }
        let operation = Operation {
            pc,
            op,
            mode,
            child,
        };
        if unit.variable {
            unit.suffix.push(operation);
        } else {
            unit.prefix.push(operation);
        }
        edits = next_edits;
        path.push(pc);
        match op {
            SAVE_CURSOR => {
                cursors.retain(|&slot| slot <= arg);
                cursors.insert(arg);
            }
            REPEAT_BEGIN => repeats.push(arg),
            REPEAT_END => {
                repeats.pop();
            }
            _ => {}
        }
        if variable {
            unit.variable = true;
        }
        if matches!(op, STOP | EMIT) {
            break;
        }
    }
    unit
}

fn definition(name: &str, operations: &[Operation], query: bool) -> Result<String, Error> {
    let mut source = format!(
        "__attribute__((noinline)) void {name}(View v,Program pg,uint node_count,thread Step& x){{device const uint4* cursors=(device const uint4*)v.nodes;device const uint4* repeats=cursors;uint cs=0u,rs=0u;\n"
    );
    for operation in operations {
        let Operation {
            pc,
            op,
            mode,
            child,
        } = *operation;
        writeln!(source,"if(!x.live || x.error!=NONE || x.boundary)return;x.pc={pc}u;if(!successor_charge(x))return;{{uint arg=v.words[pg.structural_start+{pc}u]>>16;(void)arg;").unwrap();
        let line = if let Some(child) = child {
            format!(
                "initial_child(v,pg,node_count,{}u,{}u,{}u,{}u,x);",
                child.guard_offset,
                child.selector as u8,
                u32::from(mode == NAMED),
                child.maximum_children
            )
        } else {
            super::metal_successor_codegen::instruction_line(op, mode, op == GOTO, query)?
        };
        writeln!(
            source,
            "{line}}}if(x.error!=NONE || x.boundary)return;x.pc={next}u;",
            next = pc + 1
        )
        .unwrap();
    }
    source.push_str("}\n");
    Ok(source)
}

pub(super) struct Code {
    pub source: String,
    /// Immutable Program index -> complete finite unit signature.
    pub programs: Vec<u32>,
}

pub(super) fn compile(
    resident: &ResidentInput,
    plan: &super::metal_grid_plan::ExecutionPlan,
) -> Result<Code, Error> {
    let base = super::metal_successor_codegen::compile::<
        { super::metal_state_values::INITIAL_COLUMNS },
    >(resident, &plan.readiness)?;
    let packed = &resident.input.packed;
    resident.input.children.validate(packed)?;
    let mut signatures = BTreeMap::new();
    let mut programs = Vec::with_capacity(packed.programs.len());
    let mut definitions = String::new();
    let mut layouts = String::new();
    let mut prefixes = String::new();
    let mut suffixes = String::new();
    let mut variables = String::new();
    let mut completions = String::new();
    let mut maximum = super::metal_successor_codegen::STEP_HEADER_WORDS;
    for pg in &packed.programs {
        let first = pg.structural_start as usize;
        let words = &packed.words[first..first + pg.structural_count as usize];
        let unit = prove(words, pg.flags, &resident.input.children);
        let operations = unit.prefix.iter().chain(&unit.suffix);
        let branches = operations
            .clone()
            .filter(|operation| operation.child.is_some())
            .count()
            + usize::from(unit.variable);
        let shape = super::metal_successor_codegen::edit_shape(
            operations.map(|operation| operation.pc),
            words,
            branches,
        );
        let next = signatures.len();
        let signature = *signatures.entry((unit.clone(), shape)).or_insert(next);
        programs.push(u32::try_from(signature).map_err(|_| Error::BackendCapacity)?);
        if signature != next {
            continue;
        }
        writeln!(
            completions,
            "case {signature}u:return {}u;",
            u32::from(complete(&unit))
        )
        .unwrap();
        let capture = (super::metal_successor_codegen::STEP_HEADER_WORDS + 2 * shape[0] + shape[1])
            .next_multiple_of(4);
        maximum = maximum.max((capture + 4 * shape[2] + shape[3]).next_multiple_of(4));
        definitions.push_str(&definition(
            &format!("range_prefix{signature}"),
            &unit.prefix,
            unit.query,
        )?);
        definitions.push_str(&definition(
            &format!("range_suffix{signature}"),
            &unit.suffix,
            unit.query,
        )?);
        writeln!(
            layouts,
            "case {signature}u:return uint4({}u,{}u,{}u,{}u);",
            shape[0], shape[1], shape[2], shape[3]
        )
        .unwrap();
        writeln!(
            prefixes,
            "case {signature}u:range_prefix{signature}(v,pg,node_count,x);return;"
        )
        .unwrap();
        writeln!(
            suffixes,
            "case {signature}u:range_suffix{signature}(v,pg,node_count,x);return;"
        )
        .unwrap();
        writeln!(
            variables,
            "case {signature}u:return {}u;",
            u32::from(unit.variable)
        )
        .unwrap();
    }
    let thread_helpers = include_str!("metal_step_helpers.metal")
        .replace("STEP_ADDRESS", "thread")
        .replace(
            "SUCCESSOR_HEADER_WORDS",
            &super::metal_successor_codegen::STEP_HEADER_WORDS.to_string(),
        )
        .replace(
            "PREFIX_EDIT_CAPACITY",
            &super::metal_successor_codegen::PREFIX_EDITS.to_string(),
        )
        .replace(
            "SUCCESSOR_UNIT_INSTRUCTIONS",
            &super::bounded_repeat_proof::MAX_JOB_INSTRUCTIONS.to_string(),
        );
    let source = format!("{}\n{thread_helpers}\n{definitions}\nuint4 range_layout(uint signature){{switch(signature){{{layouts}default:return uint4(NONE);}}}}\nvoid range_prefix(View v,Program pg,uint signature,uint node_count,thread Step& x){{switch(signature){{{prefixes}default:x.boundary=2u;return;}}}}\nvoid range_suffix(View v,Program pg,uint signature,uint node_count,thread Step& x){{switch(signature){{{suffixes}default:x.boundary=2u;return;}}}}\nuint range_variable(uint signature){{switch(signature){{{variables}default:return NONE;}}}}\nuint range_complete(uint signature){{switch(signature){{{completions}default:return NONE;}}}}\n#define RANGE_FRAME_WORDS {maximum}u\n{}",base.source,include_str!("metal_initial_range.metal"))
        .replace("SUCCESSOR_HEADER_WORDS",&super::metal_successor_codegen::STEP_HEADER_WORDS.to_string());
    Ok(Code { source, programs })
}
