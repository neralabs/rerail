//! Bounded normal-loop jobs retain rejected branches, exact work, and scalar exits.
use super::bounded_repeat_proof::Job;
use super::*;

fn instruction(
    source: &mut String,
    words: &[u32],
    pc: usize,
    choice: Option<u32>,
) -> Result<(), Error> {
    let (op, mode, _) = split(words[pc]);
    writeln!(source, "{{x.pc={pc}u;").unwrap();
    if matches!(op, CAPTURE | SAVE_CURSOR | REPEAT_BEGIN) {
        source
            .push_str("if(x.cursor_add+x.repeat_add+x.capture_add>=PREFIX_EDIT_CAPACITY)return;\n");
    }
    source.push_str("if(!successor_charge(x))return;\n");
    writeln!(
        source,
        "uint arg=v.words[pg.structural_start+{pc}u]>>16;(void)arg;"
    )
    .unwrap();
    match op {
        FORK => {
            let choice = choice.ok_or(Error::BackendFailure)?;
            writeln!(
                source,
                "step_branches(x)[x.branch_count++]={choice}u;x.pc={pc}u+1u;"
            )
            .unwrap();
        }
        GOTO => source.push_str("x.pc=arg;\n"),
        _ => {
            if choice.is_some() {
                return Err(Error::BackendFailure);
            }
            source.push_str(&super::metal_successor_codegen::instruction_line(
                op, mode, false, true,
            )?);
            source.push_str("if(x.error!=NONE || !x.live)return;x.pc++;\n");
        }
    }
    source.push_str("}\n");
    Ok(())
}

fn body(source: &mut String, job: &Job, words: &[u32]) -> Result<(), Error> {
    source.push_str("switch(path){\n");
    for (index, body) in job.bodies.iter().enumerate() {
        writeln!(source, "case {index}u:{{").unwrap();
        for (at, &(pc, choice)) in body.instructions.iter().enumerate() {
            if let Some(cut) = body.admission.iter().find(|cut| cut.at == at) {
                writeln!(
                    source,
                    "{{uint node=x.node;(void)node;if({}){{x.live=0u;return;}}}}",
                    admission_condition(words, cut)
                )
                .unwrap();
            }
            instruction(source, words, pc, choice)?;
        }
        source.push_str("break;}\n");
    }
    source.push_str("default:x.boundary=2u;return;}\n");
    Ok(())
}

fn admission_condition(words: &[u32], cut: &super::bounded_repeat_proof::AdmissionCut) -> String {
    cut.alternatives
        .iter()
        .map(|guards| {
            if guards.is_empty() {
                return "true".to_owned();
            }
            format!(
                "({})",
                guards
                    .iter()
                    .map(|&pc| format!(
                        "({})",
                        super::metal_terminal_repeat::guard_source(words, pc)
                    ))
                    .collect::<Vec<_>>()
                    .join(" && ")
            )
        })
        .collect::<Vec<_>>()
        .join(" || ")
}

fn tail(
    source: &mut String,
    job: &Job,
    words: &[u32],
    readiness: &[[u32; 2]],
) -> Result<(), Error> {
    let mut pc = job.exit + 1;
    let mut seen = BTreeSet::new();
    while seen.len() < 64 && seen.insert(pc) {
        if readiness.get(pc).is_some_and(|entry| entry[0] != u32::MAX) {
            break;
        }
        let (op, _, target) = split(*words.get(pc).ok_or(Error::BackendFailure)?);
        if matches!(op, FORK | CHILD | DESCEND | FOLLOWING) {
            break;
        }
        instruction(source, words, pc, None)?;
        if matches!(op, STOP | EMIT | TEXT) {
            break;
        }
        pc = if op == GOTO {
            usize::from(target)
        } else {
            pc + 1
        };
    }
    Ok(())
}

pub(super) fn definition(
    job: &Job,
    words: &[u32],
    readiness: &[[u32; 2]],
    signature: u32,
) -> Result<String, Error> {
    // A rejection cost includes every control instruction before the first false
    // guard. Matching paths return NONE; their continuations execute separately.
    let mut source = format!(
        "__attribute__((noinline)) uint repeat_rejection{signature}(View v,Program pg,uint node,uint path){{switch(path){{\n"
    );
    for (index, body) in job.bodies.iter().enumerate() {
        writeln!(source, "case {index}u:").unwrap();
        for (at, &(pc, _)) in body.instructions.iter().enumerate() {
            if let Some(cut) = body.admission.iter().find(|cut| cut.at == at) {
                writeln!(
                    source,
                    "if({})return {at}u;",
                    admission_condition(words, cut)
                )
                .unwrap();
            }
            if matches!(
                split(words[pc]).0,
                KIND | FIELD | FLAGS | SIZE | FIRST | LAST | ABSENT_FIELD | SUPERTYPE
            ) {
                writeln!(
                    source,
                    "if(!({}))return {}u;",
                    super::metal_terminal_repeat::guard_source(words, pc),
                    at + 1
                )
                .unwrap();
            }
        }
        source.push_str("return NONE;\n");
    }
    source.push_str("default:return 0u;}}\n");
    writeln!(source,"__attribute__((noinline)) uint2 repeat_probe{signature}(View v,Program pg,device Step& x){{uint remaining=PATH_WORK_LIMIT-x.path_work;if(remaining<2u)return uint2(NONE,0u);uint node=v.nodes[x.node].{};if(node==NONE)return uint2(NONE);uint match=NONE,error=NONE;for(uint path=0u;path<{}u;path++){{uint rejection=repeat_rejection{signature}(v,pg,node,path);if(!rejection){{x.boundary=2u;return uint2(NONE);}}if(rejection==NONE && match==NONE)match=path;else if(rejection!=NONE && remaining<2u+rejection && error==NONE)error=path;}}return uint2(match,error);}}",if job.named {"next_named"} else {"next_any"},job.bodies.len()).unwrap();
    writeln!(source,"__attribute__((noinline)) void repeat_prefix{signature}(View v,Program pg,device const uint4* cursors,device const uint4* repeats,uint cs,uint rs,uint variant,device Step& x){{uint depth=variant,code=0u;").unwrap();
    if !job.disjoint {
        source.push_str("uint first=0u,width=1u;depth=0u;\n");
        writeln!(source,"for(uint d=0u;d<={}u;d++){{if(variant-first<width){{depth=d;code=variant-first;break;}}first+=width;width*={}u;}}",job.iterations,job.bodies.len()).unwrap();
    }
    writeln!(source,"for(uint iteration=0u;iteration<{}u;iteration++){{if(iteration>=depth)break;x.pc={}u;uint2 probe=repeat_probe{signature}(v,pg,x);if(x.boundary)return;uint path=probe.x;",job.iterations,job.header).unwrap();
    if !job.disjoint {
        writeln!(source,"uint divisor=1u;for(uint k=iteration+1u;k<depth;k++)divisor*={}u;path=(code/divisor)%{}u;",job.bodies.len(),job.bodies.len()).unwrap();
        writeln!(source,"uint node=v.nodes[x.node].{};if(probe.y!=NONE && (probe.y<path || node==NONE || repeat_rejection{signature}(v,pg,node,path)!=NONE))path=probe.y;",if job.named {"next_named"} else {"next_any"}).unwrap();
    } else {
        source.push_str("if(probe.y!=NONE && (path==NONE || probe.y<path))path=probe.y;\n");
    }
    source.push_str("if(path==NONE){x.live=0u;return;}\n");
    instruction(&mut source, words, job.header, Some(0))?;
    instruction(&mut source, words, job.header + 1, None)?;
    body(&mut source, job, words)?;
    source.push_str("}\n");
    writeln!(source,"x.pc={}u;if(depth<{}u){{uint2 probe=repeat_probe{signature}(v,pg,x);if(x.boundary)return;if(probe.y!=NONE){{uint path=probe.y;",job.header,job.iterations).unwrap();
    instruction(&mut source, words, job.header, Some(0))?;
    instruction(&mut source, words, job.header + 1, None)?;
    body(&mut source, job, words)?;
    // The selected rejection cannot fit the remaining budget. Reaching this
    // point is a producer defect, not permission to publish a successful exit.
    source.push_str("x.boundary=2u;return;}\n");
    instruction(&mut source, words, job.header, Some(1))?;
    if readiness[job.exit][0] != u32::MAX {
        // The selected exit can meet an earlier active path at its closing
        // instruction. Admit that complete key before charging or popping.
        writeln!(source, "x.pc={}u;return;", job.exit).unwrap();
    } else {
        instruction(&mut source, words, job.exit, None)?;
        tail(&mut source, job, words, readiness)?;
    }
    source.push_str("}\n}\n");
    Ok(source)
}
