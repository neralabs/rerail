//! Finite register prefixes. Runtime operands belong to the resident Program.
use super::metal_grid_plan::ResidentInput;
use super::metal_initial_prefix::incoming_counts;
use super::*;

pub(super) use super::metal_initial_prefix::PREFIX_EDITS;
// Scalar state, logical input identity, and offsets into the signature's edits.
pub(super) const STEP_HEADER_WORDS: usize = 16;

pub(super) fn edit_shape(
    operations: impl IntoIterator<Item = usize>,
    words: &[u32],
    branches: usize,
) -> [usize; 4] {
    let mut shape = [0, 0, 0, branches];
    for pc in operations {
        match split(words[pc]).0 {
            SAVE_CURSOR => shape[0] += 1,
            REPEAT_BEGIN => shape[1] += 1,
            CAPTURE => shape[2] += 1,
            _ => {}
        }
    }
    shape
}

fn step_words(shape: [usize; 4]) -> usize {
    let capture = (STEP_HEADER_WORDS + 2 * shape[0] + shape[1]).next_multiple_of(4);
    (capture + 4 * shape[2] + shape[3]).next_multiple_of(4)
}

/// Query prefixes retain proved admission boundaries before further observable
/// work. Every unit has finite edits, instructions, and fan-out; variable
/// navigation remains a shared expansion.
fn region_ends(words: &[u32], query: bool, readiness: &[[u32; 2]]) -> Vec<usize> {
    // GREEDY control also observes joins inside its cyclic component. Acyclic
    // admission joins use the shared readiness proof for every repeat query.
    let observe_admission = query && words.iter().any(|&w| matches!(split(w), (FORK, GREEDY, _)));
    let incoming = incoming_counts(words);
    (0..words.len())
        .map(|entry| {
            let mut end = entry;
            let mut edits = 0;
            for (pc, &word) in words.iter().enumerate().skip(entry).take(64) {
                if query && pc != entry && readiness[pc][0] != u32::MAX {
                    break;
                }
                if observe_admission && pc != entry && incoming[pc] != 1 {
                    break;
                }
                let (op, mode, target) = split(word);
                let adjacent = op == GOTO && usize::from(target) == pc + 1;
                // Closure readiness must observe the header itself. Executing
                // its FORK here would bypass the bulk-closure operator.
                if query
                    && pc != entry
                    && op == FORK
                    && (!observe_admission
                        || mode == GREEDY
                        || super::bounded_repeat_proof::prove(words, pc).is_some())
                {
                    break;
                }
                edits += usize::from(matches!(op, SAVE_CURSOR | REPEAT_BEGIN | CAPTURE));
                if edits > PREFIX_EDITS {
                    break;
                }
                end = pc + 1;
                if matches!(op, STOP | EMIT | FORK | CHILD | DESCEND | FOLLOWING | TEXT)
                    || (op == ASSERT && !query)
                    || (op == GOTO && !adjacent)
                    || (observe_admission
                        && !matches!(
                            op,
                            KIND | FIELD
                                | FLAGS
                                | SIZE
                                | FIRST
                                | LAST
                                | ABSENT_FIELD
                                | SUPERTYPE
                                | CAPTURE
                                | ASSERT
                                | REPEAT_BEGIN
                                | REPEAT_BUMP
                                | REPEAT_END
                                | GOTO
                        ))
                {
                    break;
                }
            }
            end
        })
        .collect()
}

pub(super) struct Code {
    pub source: String,
    /// Signature, entry offset, bounded work count, and checked query greedy-exit flag.
    pub sites: Vec<[u32; 4]>,
    pub signatures: usize,
    pub variants: usize,
    pub maximum_step_words: usize,
}

pub(super) fn compile<const FIELDS: usize>(
    resident: &ResidentInput,
    readiness: &super::metal_ready_proof::ReadyProof,
) -> Result<Code, Error> {
    if ![
        super::metal_state_values::INITIAL_COLUMNS,
        super::metal_state_values::POSITIONED_COLUMNS,
    ]
    .contains(&FIELDS)
    {
        return Err(Error::BackendFailure);
    }
    let ready_entries = readiness.entries(resident)?;
    let packed = &resident.input.packed;
    resident.input.children.validate(packed)?;
    let mut sites = vec![[u32::MAX, 0, 1, 0]; packed.words.len()];
    let mut signatures = BTreeMap::new();
    let mut repeat_signatures = BTreeMap::new();
    let mut branch_signatures = BTreeMap::new();
    let mut signature_count = 0u32;
    let mut variants = 1;
    let mut definitions = String::new();
    let mut cases = String::new();
    let mut layouts = Vec::new();
    for program in &packed.programs {
        let first = program.structural_start as usize;
        let words = &packed.words[first..first + program.structural_count as usize];
        let mut ends = region_ends(
            words,
            program.flags & 2 != 0,
            &ready_entries[first..first + words.len()],
        );
        let initial_children = if FIELDS == super::metal_state_values::INITIAL_COLUMNS {
            super::metal_initial_prefix::prove(
                words,
                program.flags & 2 != 0,
                &resident.input.children,
            )
            .map(|region| {
                ends[0] = region.end;
                region.children
            })
            .unwrap_or_default()
        } else {
            BTreeMap::new()
        };
        let mut pc = 0;
        while pc < words.len() {
            if let Some(job) = (program.flags & 2 != 0)
                .then(|| super::bounded_repeat_proof::prove(words, pc))
                .flatten()
            {
                let next = signature_count;
                // Operands and absolute sites are part of this finite job's
                // proof. Different Programs retain their own resident operands.
                let signature = *repeat_signatures
                    .entry((job.clone(), words.to_vec()))
                    .or_insert(next);
                sites[first + pc] = [signature, 0, job.variants as u32, 0];
                pc += 1;
                variants = variants.max(job.variants);
                if signature == next {
                    signature_count += 1;
                    // The bounded repeat proof caps combined edits and branch
                    // positions. Each typed tail keeps that complete bound.
                    layouts.push([PREFIX_EDITS, PREFIX_EDITS, PREFIX_EDITS, 64]);
                    definitions.push_str(&super::metal_successor_repeat_codegen::definition(
                        &job,
                        words,
                        &ready_entries[first..first + words.len()],
                        signature,
                    )?);
                    writeln!(cases,"case {signature}u:repeat_prefix{signature}(v,pg,cursors,repeats,cs,rs,variant,x);break;").unwrap();
                }
                continue;
            }
            if let Some(paths) = (program.flags & 2 != 0)
                .then(|| {
                    super::metal_successor_branch_codegen::prove(
                        words,
                        &ready_entries[first..first + words.len()],
                        pc,
                    )
                })
                .flatten()
            {
                let next = signature_count;
                let signature = *branch_signatures
                    .entry((paths.clone(), words.to_vec()))
                    .or_insert(next);
                sites[first + pc] = [signature, 0, paths.len() as u32, 0];
                pc += 1;
                variants = variants.max(paths.len());
                if signature == next {
                    signature_count += 1;
                    layouts.push(super::metal_successor_branch_codegen::edit_shape(
                        &paths, words,
                    ));
                    definitions.push_str(&super::metal_successor_branch_codegen::definition(
                        &paths, words, signature,
                    )?);
                    writeln!(cases,"case {signature}u:branch_prefix{signature}(v,pg,cursors,repeats,cs,rs,variant,x);break;").unwrap();
                }
                continue;
            }
            let end = ends[pc];
            let mut shape = Vec::new();
            for (offset, &w) in words[pc..end].iter().enumerate() {
                let (op, mode, arg) = split(w);
                shape.push((
                    op,
                    mode,
                    op == GOTO && usize::from(arg) == pc + offset + 1,
                    initial_children.get(&(pc + offset)).copied(),
                ));
            }
            let key = (program.flags & 1, shape.clone());
            let next = signature_count;
            let signature = *signatures.entry(key).or_insert(next);
            for (offset, site) in sites[first + pc..first + end].iter_mut().enumerate() {
                *site = [signature, offset as u32, 1, 0];
            }
            pc = end;
            if signature != next {
                continue;
            }
            signature_count += 1;
            layouts.push(edit_shape(
                0..shape.len(),
                &words[pc - shape.len()..pc],
                shape.iter().filter(|entry| entry.3.is_some()).count(),
            ));
            writeln!(definitions, "__attribute__((noinline)) void prefix{signature}(View v, Program pg, device const uint4* cursors, device const uint4* repeats, uint cs, uint rs, uint skip, uint node_count, device Step& x) {{ (void)node_count;").unwrap();
            for (offset, (op, mode, adjacent, child)) in shape.into_iter().enumerate() {
                writeln!(definitions, "if({offset}u>=skip && x.live && x.expansion==0u) {{ uint arg=v.words[pg.structural_start+x.pc]>>16;").unwrap();
                writeln!(definitions, "if(!successor_charge(x))return;").unwrap();
                let line = if let Some(child) = child {
                    format!(
                        "initial_child(v,pg,node_count,{}u,{}u,{}u,{}u,x);if(x.boundary>1u)return;",
                        child.guard_offset,
                        child.selector as u8,
                        u32::from(mode == NAMED),
                        child.maximum_children
                    )
                } else {
                    instruction_line(op, mode, adjacent, program.flags & 1 != 0)?
                };
                writeln!(
                    definitions,
                    "(void)arg;{line}if(x.error!=NONE) return;x.pc++; }}"
                )
                .unwrap();
            }
            definitions.push_str("}\n");
            writeln!(
                cases,
                "case {signature}u: prefix{signature}(v,pg,cursors,repeats,cs,rs,sites[pg.structural_start+x.pc].y,p.nodes,x);break;"
            )
            .unwrap();
        }
        if program.flags & 2 != 0 {
            for &instruction in words {
                let (op, mode, target) = split(instruction);
                if op == FORK && mode == GREEDY {
                    sites
                        .get_mut(first + usize::from(target))
                        .ok_or(Error::BackendFailure)?[3] = 1;
                }
            }
        }
    }
    if layouts.len() != signature_count as usize
        || layouts
            .iter()
            .any(|shape| shape[..3].iter().any(|&count| count > PREFIX_EDITS) || shape[3] > 64)
    {
        return Err(Error::BackendFailure);
    }
    // Finished, held, and predicate rows have no new edits in this unit.
    layouts.push([0; 4]);
    let maximum_step_words = layouts.iter().copied().map(step_words).max().unwrap();
    let mut layout_source =
        "uint4 successor_layout(uint signature) { switch(signature) {\n".to_owned();
    for (signature, shape) in layouts.iter().enumerate() {
        writeln!(
            layout_source,
            "case {signature}u:return uint4({}u,{}u,{}u,{}u);",
            shape[0], shape[1], shape[2], shape[3]
        )
        .unwrap();
    }
    layout_source.push_str("default:return uint4(NONE);}}\n");
    // Both address spaces execute the same finite operations and normalization.
    // Rename private definitions because rejection helpers have no Step argument.
    let private_names = |source: &str| {
        source
            .replace("prefix", "register_prefix")
            .replace("repeat_probe", "register_repeat_probe")
            .replace("repeat_rejection", "register_repeat_rejection")
    };
    let private_definitions = private_names(&definitions).replace("device Step&", "thread Step&");
    let private_cases = private_names(&cases);
    let frame = include_str!("metal_successor_frame.metal");
    let functions = format!(
        "#ifndef SUCCESSOR_REGISTER_FRAMES\n{definitions}\n{}\n#else\n{}\n{private_definitions}\n{}\n#endif",
        frame
            .replace("STEP_ADDRESS", "device")
            .replace("// SUCCESSOR_CASES", &cases),
        include_str!("metal_step_helpers.metal").replace("STEP_ADDRESS", "thread"),
        frame
            .replace("STEP_ADDRESS", "thread")
            .replace("// SUCCESSOR_CASES", &private_cases)
    );
    let source = concat!(
        include_str!("metal_resident.metal"),
        include_str!("metal_input_layout.metal"),
        include_str!("metal_task_groups.metal"),
        include_str!("metal_successor_common.metal"),
        include_str!("metal_successor.metal"),
        include_str!("metal_successor_active.metal"),
        include_str!("metal_successor_register.metal")
    )
    .replace(
        "// STEP_HELPERS",
        &include_str!("metal_step_helpers.metal").replace("STEP_ADDRESS", "device"),
    )
    .replace("// SUCCESSOR_PREFIXES", &functions)
    .replace("// SUCCESSOR_LAYOUTS", &layout_source)
    .replace("// SUCCESSOR_CASES", &cases)
    .replace(
        "SUCCESSOR_UNIT_INSTRUCTIONS",
        &super::bounded_repeat_proof::MAX_JOB_INSTRUCTIONS.to_string(),
    )
    .replace("PREFIX_EDIT_CAPACITY", &PREFIX_EDITS.to_string())
    .replace("SUCCESSOR_HEADER_WORDS", &STEP_HEADER_WORDS.to_string())
    .replace("SUCCESSOR_PRIVATE_WORDS", &maximum_step_words.to_string());
    let source = format!("#define INPUT_COLUMNS {FIELDS}u\n{source}");

    Ok(Code {
        source,
        sites,
        signatures: signature_count as usize,
        variants,
        maximum_step_words,
    })
}

pub(super) fn instruction_line(
    op: u8,
    mode: u8,
    adjacent: bool,
    query: bool,
) -> Result<String, Error> {
    Ok(match op {
                    STOP => "x.live=0u;".to_owned(),
                    KIND => "x.live=uint(v.nodes[x.node].kind==arg);".to_owned(),
                    FIELD => "x.live=uint(v.nodes[x.node].field==arg);".to_owned(),
                    FLAGS => format!("x.live=uint((v.nodes[x.node].flags&{mode}u)=={mode}u && (v.nodes[x.node].flags&arg)==0u);"),
                    SIZE => {
                        let comparison = match mode & !SIZE_LONG { 0 => "==", 1 => ">=", 2 => "<=", _ => return Err(Error::BackendFailure) };
                        let expected = if mode & SIZE_LONG == 0 { "ulong(arg)" } else { "v.longs[pg.long_start+arg]" };
                        format!("x.live=uint(ulong(v.nodes[x.node].source_end-v.nodes[x.node].source_start){comparison}{expected});")
                    }
                    PARENT | NEXT => format!("x.node=v.nodes[x.node].{};x.live=uint(x.node!=NONE);", if op==PARENT { "parent" } else if mode==NAMED { "next_named" } else { "next_any" }),
                    FIRST | LAST => format!("x.live=uint(at_edge(v,x.node,{}u,{}u));",u32::from(op==LAST),u32::from(mode==NAMED)),
                    ABSENT_FIELD => "x.live=uint(!has_pair(v.fields,v.header->field_count,x.node,arg));".to_owned(),
                    SUPERTYPE => "x.live=uint(has_pair(v.supertypes,v.header->supertype_count,x.node,arg));".to_owned(),
                    SAVE_CURSOR => "save_cursor(cursors,cs,arg,x);".to_owned(),
                    RESTORE_CURSOR => "restore_cursor(cursors,cs,arg,x);".to_owned(),
                    REPEAT_BEGIN => "step_repeats(x)[x.repeat_add++]=arg;".to_owned(),
                    REPEAT_BUMP | REPEAT_END => format!("repeat_top(repeats,rs,arg,{}u,x);",u32::from(op==REPEAT_END)),
                    CAPTURE => "Node n=v.nodes[x.node];step_captures(x)[x.capture_add++]=uint4(x.node,arg,n.source_start,n.source_end);".to_owned(),
                    CHILD | DESCEND | FOLLOWING => format!("expand_nodes(v,{}u,{}u,x);",if op==CHILD {3} else if op==DESCEND {4} else {5},u32::from(mode==NAMED)),
                    FORK => format!("x.expansion={}u;x.begin=arg;x.end=2u;",if mode==GREEDY { 2 } else { 1 }),
                    GOTO if adjacent => String::new(),
                    GOTO => "x.pc=arg;return;".to_owned(),
                    EMIT => "x.kind=x.kind==0u?1u:3u;return;".to_owned(),
                    ASSERT if query => String::new(),
                    TEXT => "uint answer=finite_text(v,pg,arg,x.node);if(answer==NONE){x.boundary=1u;return;}x.live=answer;".to_owned(),
                    ASSERT => "x.boundary=1u;return;".to_owned(),
                    _ => return Err(Error::BackendFailure),
                })
}
