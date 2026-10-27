//! Shared terminal repetition: classify nodes, scan runs and exact raw counts,
//! then scatter complete typed captures. No per-sibling host continuation.
use super::metal_backend::launch::{commit_checked, wait_checked};
use super::metal_backend::{MappedBuffer, pipeline_named};
use super::metal_batch_abi::{FinalCaptureRecord, ResultRecord};
use super::metal_grid_plan::{ExecutionPlan, ResidentInput, StageKind, WorkDomain};
use super::metal_parallel::{Scan, encode as dispatch};
use super::metal_register_execution::read;
use super::*;
use metal::{BufferRef, CommandBufferRef, CommandQueueRef, Device};

#[path = "metal_terminal_repeat_cost.rs"]
mod cost;

pub(super) fn guard_source(words: &[u32], pc: usize) -> String {
    let (op, mode, _) = split(words[pc]);
    let arg = format!("(v.words[pg.structural_start+{pc}u]>>16)");
    match op {
        KIND => format!("v.nodes[node].kind=={arg}"),
        FIELD => format!("v.nodes[node].field=={arg}"),
        FLAGS => {
            format!("(v.nodes[node].flags&{mode}u)=={mode}u && (v.nodes[node].flags&{arg})==0u")
        }
        SIZE => {
            let comparison = match mode & !SIZE_LONG {
                0 => "==",
                1 => ">=",
                2 => "<=",
                _ => unreachable!("validated size comparison"),
            };
            let expected = if mode & SIZE_LONG == 0 {
                format!("ulong({arg})")
            } else {
                format!("v.longs[pg.long_start+{arg}]")
            };
            format!(
                "ulong(v.nodes[node].source_end-v.nodes[node].source_start){comparison}{expected}"
            )
        }
        FIRST | LAST => format!(
            "at_edge(v,node,{}u,{}u)",
            u32::from(op == LAST),
            u32::from(mode == NAMED)
        ),
        ABSENT_FIELD => format!("!has_pair(v.fields,v.header->field_count,node,{arg})"),
        SUPERTYPE => format!("has_pair(v.supertypes,v.header->supertype_count,node,{arg})"),
        _ => unreachable!("terminal repetition proof contains only static guards"),
    }
}

fn source(resident: &ResidentInput, plan: &ExecutionPlan) -> String {
    let mut modes = String::new();
    let mut roots = String::new();
    let mut classes = String::new();
    let mut sizes = String::new();
    let mut captures = String::new();
    let mut costs = String::new();
    let mut bounds = String::new();
    for (&index, proof) in plan.readiness.repetitions() {
        let pg = resident.input.packed.programs[index as usize];
        let words = &resident.input.packed.words
            [pg.structural_start as usize..(pg.structural_start + pg.structural_count) as usize];
        writeln!(
            modes,
            "case {index}u:return {}u;",
            if proof.children {
                1 + u32::from(proof.named)
            } else {
                3 + u32::from(proof.named)
            }
        )
        .unwrap();
        cost::root(&mut roots, index, proof, words);
        cost::costs(&mut costs, index, proof, words);
        cost::bounds(&mut bounds, index, proof);
        writeln!(classes, "case {index}u:{{").unwrap();
        writeln!(
            sizes,
            "case {index}u:if(cls==0u)return uint3({}u,{}u,{}u);switch(cls){{",
            proof.prefix.len(),
            proof.suffix.len(),
            proof.emit
        )
        .unwrap();
        writeln!(captures, "case {index}u: if(cls==0u) {{switch(at){{").unwrap();
        for (at, &pc) in proof.prefix.iter().chain(&proof.suffix).enumerate() {
            writeln!(captures, "case {at}u:return {pc}u;").unwrap();
        }
        captures.push_str("}return NONE;}switch(cls){\n");
        for (path_index, path) in proof.paths.iter().enumerate() {
            let cls = path_index + 1;
            let guards = path
                .iter()
                .filter(|&&pc| split(words[pc]).0 != CAPTURE)
                .map(|&pc| format!("({})", guard_source(words, pc)))
                .collect::<Vec<_>>();
            writeln!(
                classes,
                "if({})return {cls}u;",
                if guards.is_empty() {
                    "true".to_owned()
                } else {
                    guards.join(" && ")
                }
            )
            .unwrap();
            let labels = path
                .iter()
                .copied()
                .filter(|&pc| split(words[pc]).0 == CAPTURE)
                .collect::<Vec<_>>();
            writeln!(sizes, "case {cls}u:return uint3({}u,0u,0u);", labels.len()).unwrap();
            writeln!(captures, "case {cls}u:switch(at){{").unwrap();
            for (at, pc) in labels.iter().enumerate() {
                writeln!(captures, "case {at}u:return {pc}u;").unwrap();
            }
            captures.push_str("}break;\n");
        }
        classes.push_str("return 0u;}\n");
        sizes.push_str("}return uint3(0u);\n");
        captures.push_str("}return NONE;\n");
    }
    format!(
        "{}\nuint repeat_mode(uint id){{switch(id){{{modes}}}return 0u;}}\nuint2 repeat_root(View v,uint id,Program pg,uint node){{switch(id){{{roots}}}return uint2(0u);}}\nuint4 repeat_cost(View v,uint id,Program pg,uint node){{switch(id){{{costs}}}return uint4(0u);}}\nuint2 repeat_bounds(uint id){{switch(id){{{bounds}}}return uint2(0u);}}\nuint repeat_class(View v,uint id,Program pg,uint node){{switch(id){{{classes}}}return 0u;}}\nuint3 repeat_size(uint id,uint cls){{switch(id){{{sizes}}}return uint3(0u);}}\nuint repeat_pc(uint id,uint cls,uint at){{switch(id){{{captures}}}return NONE;}}\n{}",
        include_str!("metal_resident.metal"),
        include_str!("metal_terminal_repeat.metal")
    )
}

pub(super) fn execute(
    device: &Device,
    queue: &CommandQueueRef,
    resident: &ResidentInput,
    plan: &ExecutionPlan,
    data: &MappedBuffer,
    roots: &MappedBuffer,
) -> Result<super::metal_dynamic_execution::DynamicOutput, Error> {
    execute_with_work_limit(
        device,
        queue,
        resident,
        plan,
        data,
        roots,
        MAX_STEPS_PER_PATH as u32,
    )
}

pub(super) fn execute_with_work_limit(
    device: &Device,
    queue: &CommandQueueRef,
    resident: &ResidentInput,
    plan: &ExecutionPlan,
    data: &MappedBuffer,
    roots: &MappedBuffer,
    work_limit: u32,
) -> Result<super::metal_dynamic_execution::DynamicOutput, Error> {
    if work_limit as usize > MAX_STEPS_PER_PATH {
        return Err(Error::InvalidAnchor);
    }
    plan.validate(resident)?;
    let code = source(resident, plan);
    let grid = &resident.input.grid;
    let mut selected_blocks = vec![false; grid.blocks.len()];
    for stage in &plan.stages {
        if stage.kind != StageKind::CountRepeatedBlocks {
            continue;
        }
        let WorkDomain::Blocks(groups) = &stage.work else {
            return Err(Error::InvalidAnchor);
        };
        for group in groups {
            let blocks = selected_blocks
                .get_mut(group.blocks.start as usize..group.blocks.end as usize)
                .ok_or(Error::InvalidAnchor)?;
            if blocks.iter().any(|selected| *selected) {
                return Err(Error::InvalidAnchor);
            }
            blocks.fill(true);
        }
    }
    // A domain row represents a span, never an expanded semantic task list.
    // Dense repetition ordinals and original semantic IDs have separate columns.
    let mut domain = Vec::new();
    let mut tasks = 0u32;
    for &id in &grid.semantic_span_order {
        let span = grid.block_spans[id as usize];
        if selected_blocks[span.execution_block as usize] {
            domain.push([tasks, span.semantic_start, span.root_count, 0]);
            tasks = tasks
                .checked_add(span.root_count)
                .ok_or(Error::BackendCapacity)?;
        }
    }
    let domains = domain.len();
    let domain = MappedBuffer::from_slice(device, &domain)?;
    let tasks = tasks as usize;
    let task_offsets = Scan::new(device, tasks)?;
    let descriptors = MappedBuffer::zeroed::<[u32; 4]>(device, tasks)?;
    let status = MappedBuffer::zeroed::<u32>(device, 1)?;
    let failures = MappedBuffer::zeroed::<u32>(device, resident.input.grid.evaluation_spans.len())?;
    let mut parameters = [
        tasks,
        resident.input.grid.semantic_span_order.len(),
        resident.input.packed.programs.len(),
        0,
        0,
        0,
        0,
        work_limit as usize,
        domains,
    ]
    .map(|v| u32::try_from(v).map_err(|_| Error::BackendCapacity))
    .into_iter()
    .collect::<Result<Vec<_>, _>>()?;
    let run = |command: &CommandBufferRef,
               name,
               buffers: &[&BufferRef],
               p: &[u32],
               count: usize|
     -> Result<(), Error> {
        dispatch(
            command,
            &pipeline_named(&code, name)?,
            name,
            buffers,
            p,
            count.div_ceil(256).max(1),
            None,
        );
        Ok(())
    };
    metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
        let command = queue.new_command_buffer();
        run(
            command,
            "repeat_root_counts",
            &[
                &data.buffer,
                &roots.buffer,
                &descriptors.buffer,
                &task_offsets.values.buffer,
                &status.buffer,
                &failures.buffer,
                &domain.buffer,
            ],
            &parameters,
            tasks,
        )?;
        task_offsets.encode(command)?;
        commit_checked(command)?;
        wait_checked(command)?;
        Ok(())
    })?;
    let totals = read_total(task_offsets.total())?;
    check(&status)?;
    if totals[0] == u32::MAX || totals[1..] != [0, 0, 0] {
        return Err(Error::BackendCapacity);
    }
    let cells = totals[0] as usize;
    if cells == 0 {
        return Ok(super::metal_dynamic_execution::DynamicOutput {
            candidates: super::metal_candidate_batch::CandidateBatch::empty(device)?,
            errors: evaluation_errors(&failures, resident, plan)?,
        });
    }
    parameters[3] = totals[0];
    let nodes = MappedBuffer::zeroed::<[u32; 4]>(device, cells)?;
    let boundaries = [
        MappedBuffer::zeroed::<[u32; 2]>(device, cells)?,
        MappedBuffer::zeroed::<[u32; 2]>(device, cells)?,
    ];
    let values = Scan::new(device, cells)?;
    let costs = MappedBuffer::zeroed::<[u32; 4]>(device, cells)?;
    let origins = [
        MappedBuffer::zeroed::<i64>(device, cells)?,
        MappedBuffer::zeroed::<i64>(device, cells)?,
    ];
    let raw = Scan::new(device, cells)?;
    let selected = Scan::new(device, cells)?;
    let limited = Scan::new(device, cells)?;
    let mut current = 0;
    metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
        let command = queue.new_command_buffer();
        run(
            command,
            "repeat_cells",
            &[
                &data.buffer,
                &descriptors.buffer,
                &task_offsets.values.buffer,
                &nodes.buffer,
                &boundaries[0].buffer,
                &values.values.buffer,
                &costs.buffer,
            ],
            &parameters,
            cells,
        )?;
        values.encode(command)?;
        run(
            command,
            "repeat_origins",
            &[
                &nodes.buffer,
                &descriptors.buffer,
                &task_offsets.values.buffer,
                &values.values.buffer,
                &costs.buffer,
                &origins[0].buffer,
            ],
            &parameters,
            cells,
        )?;
        let mut step = 1usize;
        while step < cells {
            parameters[6] = u32::try_from(step).map_err(|_| Error::BackendCapacity)?;
            run(
                command,
                "repeat_neighbors",
                &[
                    &nodes.buffer,
                    &descriptors.buffer,
                    &task_offsets.values.buffer,
                    &boundaries[current].buffer,
                    &boundaries[1 - current].buffer,
                    &origins[current].buffer,
                    &origins[1 - current].buffer,
                ],
                &parameters,
                cells,
            )?;
            current = 1 - current;
            step = step.checked_mul(2).ok_or(Error::BackendCapacity)?;
        }
        run(
            command,
            "repeat_work",
            &[
                &data.buffer,
                &nodes.buffer,
                &descriptors.buffer,
                &task_offsets.values.buffer,
                &values.values.buffer,
                &costs.buffer,
                &origins[current].buffer,
                &failures.buffer,
                &status.buffer,
                &domain.buffer,
            ],
            &parameters,
            cells,
        )?;
        run(
            command,
            "repeat_candidate_counts",
            &[
                &nodes.buffer,
                &descriptors.buffer,
                &task_offsets.values.buffer,
                &boundaries[current].buffer,
                &values.values.buffer,
                values.total(),
                &raw.values.buffer,
                &selected.values.buffer,
            ],
            &parameters,
            cells,
        )?;
        raw.encode(command)?;
        selected.encode(command)?;
        run(
            command,
            "repeat_limit",
            &[
                &data.buffer,
                &nodes.buffer,
                &task_offsets.values.buffer,
                &selected.values.buffer,
                selected.total(),
                &limited.values.buffer,
                &failures.buffer,
                &domain.buffer,
                &status.buffer,
            ],
            &parameters,
            cells,
        )?;
        limited.encode(command)?;
        commit_checked(command)?;
        wait_checked(command)?;
        Ok(())
    })?;
    check(&status)?;
    let raw_total = read_total(raw.total())?;
    let value_total = read_total(values.total())?;
    let kept = read_total(limited.total())?;
    if raw_total.contains(&u32::MAX)
        || value_total.contains(&u32::MAX)
        || kept.contains(&u32::MAX)
        || kept[0] > totals[0]
    {
        return Err(Error::BackendCapacity);
    }
    parameters[4] = kept[0];
    parameters[5] = kept[1];
    let results = MappedBuffer::zeroed::<ResultRecord>(device, kept[0] as usize)?;
    let captures = MappedBuffer::zeroed::<FinalCaptureRecord>(device, kept[1] as usize)?;
    metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
        let command = queue.new_command_buffer();
        run(
            command,
            "repeat_results",
            &[
                &data.buffer,
                &nodes.buffer,
                &descriptors.buffer,
                &task_offsets.values.buffer,
                &raw.values.buffer,
                &limited.values.buffer,
                &results.buffer,
                &domain.buffer,
                &status.buffer,
            ],
            &parameters,
            cells,
        )?;
        run(
            command,
            "repeat_captures",
            &[
                &data.buffer,
                &nodes.buffer,
                &descriptors.buffer,
                &limited.values.buffer,
                &values.values.buffer,
                &captures.buffer,
            ],
            &parameters,
            kept[1] as usize,
        )?;
        commit_checked(command)?;
        wait_checked(command)?;
        Ok(())
    })?;
    check(&status)?;
    Ok(super::metal_dynamic_execution::DynamicOutput {
        candidates: super::metal_candidate_batch::CandidateBatch {
            results,
            captures,
            result_count: kept[0] as usize,
            capture_count: kept[1] as usize,
        },
        errors: evaluation_errors(&failures, resident, plan)?,
    })
}

fn evaluation_errors(
    failures: &MappedBuffer,
    resident: &ResidentInput,
    plan: &ExecutionPlan,
) -> Result<Vec<Option<Error>>, Error> {
    read::<u32>(failures, resident.input.grid.evaluation_spans.len())?
        .iter()
        .enumerate()
        .map(|(index, &code)| match code {
            0 => Ok(None),
            1 if plan
                .readiness
                .repetitions()
                .contains_key(&resident.input.grid.evaluation_spans[index].program) =>
            {
                Ok(Some(Error::WorkLimit))
            }
            _ => Err(Error::BackendFailure),
        })
        .collect()
}

fn read_total(buffer: &BufferRef) -> Result<[u32; 4], Error> {
    if buffer.length() < 16 {
        return Err(Error::BackendFailure);
    }
    Ok(unsafe { buffer.contents().cast::<[u32; 4]>().read() })
}
fn check(status: &MappedBuffer) -> Result<(), Error> {
    match read::<u32>(status, 1)?[0] {
        0 => Ok(()),
        1 => Err(Error::BackendCapacity),
        _ => Err(Error::BackendFailure),
    }
}
