//! Generate finite register work inside broad class kernels. No opcode interpreter.
use super::metal_grid_plan::ResidentInput;
use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RegisterClass {
    Direct,
    Bounded,
}

/// The whole-pattern catalog class can require a predicate frontier while its
/// complete structural paths still satisfy the finite register proof.
pub(super) fn register_class(
    input: &super::metal_grid::MetalGridInput,
    program: &super::metal_batch_abi::MetalProgram,
) -> Result<Option<RegisterClass>, Error> {
    input.children.validate(&input.packed)?;
    if input.children.paths(program)?.is_none() {
        return Ok(None);
    }
    let batch = &input.packed;
    let words = &batch.words[program.structural_start as usize
        ..(program.structural_start + program.structural_count) as usize];
    Ok(Some(
        if words.iter().any(|&word| {
            matches!(
                split(word).0,
                CHILD | PARENT | NEXT | SAVE_CURSOR | RESTORE_CURSOR
            )
        }) {
            RegisterClass::Bounded
        } else {
            RegisterClass::Direct
        },
    ))
}

#[derive(Clone, Copy)]
pub(super) struct RegisterBound {
    pub class: RegisterClass,
    pub matches: usize,
    pub captures: usize,
}

pub(super) struct RegisterCode {
    pub source: String,
    pub bounds: BTreeMap<u32, RegisterBound>,
}

pub(super) fn compile(resident: &ResidentInput) -> Result<RegisterCode, Error> {
    let batch = &resident.input.packed;
    let grid = &resident.input.grid;
    let mut bounds = BTreeMap::new();
    let mut definitions = String::new();
    let mut counts = String::new();
    let mut scatters = String::new();
    let mut constant_scatters = String::new();
    let mut constant_bounds = String::new();
    for block in &grid.blocks {
        let signature = block.dispatch_signature;
        if bounds.contains_key(&signature) {
            continue;
        }
        let span = grid.block_spans[block.block_span_start as usize];
        let program =
            batch.programs[grid.evaluation_spans[span.evaluation_span as usize].program as usize];
        let start = program.structural_start as usize;
        let words = &batch.words[start..start + program.structural_count as usize];
        resident.input.children.validate(batch)?;
        let Some(paths) = resident.input.children.paths(&program)? else {
            continue;
        };
        let captures = paths
            .iter()
            .map(|path| {
                path.iter()
                    .filter(|&&pc| split(words[pc]).0 == CAPTURE)
                    .count()
            })
            .max()
            .unwrap_or(0);
        let bound = RegisterBound {
            class: register_class(&resident.input, &program)?.ok_or(Error::BackendFailure)?,
            matches: paths.len(),
            captures,
        };
        bounds.insert(signature, bound);
        writeln!(
            constant_bounds,
            "case {signature}u: return uint2({}u,{}u);",
            bound.matches, bound.captures
        )
        .unwrap();
        let suffix = signature.to_string();
        writeln!(
            definitions,
            "struct Match{suffix} {{ uint local,label,count; uint2 caps[{}]; }};",
            captures.max(1)
        )
        .unwrap();
        writeln!(definitions, "template<bool Store,bool Properties=false> __attribute__((noinline)) uint4 run{suffix}(View v, Program pg, uint root, uint maximum, thread Match{suffix}* matches) {{ uint4 total=uint4(0); uint ordinal=0;").unwrap();
        for path in paths {
            definitions.push_str("{ uint node=root; bool live=true;\n");
            let mut saved = BTreeSet::new();
            let mut capture_count = 0;
            for &pc in path {
                let (op, mode, operand) = split(words[pc]);
                let arg = format!("(v.words[pg.structural_start+{pc}u]>>16)");
                let line = match op {
                    KIND => format!("if(live) live=v.nodes[node].kind=={arg};"),
                    FIELD => format!("if(live) live=v.nodes[node].field=={arg};"),
                    FLAGS => format!(
                        "if(live) live=(v.nodes[node].flags&{mode}u)=={mode}u && (v.nodes[node].flags&{arg})==0u;"
                    ),
                    SIZE => {
                        let comparison = match mode & !SIZE_LONG {
                            0 => "==",
                            1 => ">=",
                            2 => "<=",
                            _ => return Err(Error::BackendFailure),
                        };
                        let expected = if mode & SIZE_LONG == 0 {
                            format!("ulong({arg})")
                        } else {
                            format!("v.longs[pg.long_start+{arg}]")
                        };
                        format!(
                            "if(live) live=ulong(v.nodes[node].source_end-v.nodes[node].source_start){comparison}{expected};"
                        )
                    }
                    ASSERT if program.flags & 1 != 0 => String::new(),
                    TEXT => format!("if(live) live=finite_text(v,pg,{arg},node)==1u;"),
                    CHILD => super::metal_child_index::lookup_source(words, pc, mode)?,
                    PARENT => "if(live) { node=v.nodes[node].parent; live=node!=NONE; }".to_owned(),
                    NEXT => format!(
                        "if(live) {{ node=v.nodes[node].{}; live=node!=NONE; }}",
                        if mode == NAMED {
                            "next_named"
                        } else {
                            "next_any"
                        }
                    ),
                    FIRST | LAST => format!(
                        "if(live) live=at_edge(v,node,{}u,{}u);",
                        u32::from(op == LAST),
                        u32::from(mode == NAMED)
                    ),
                    ABSENT_FIELD => format!(
                        "if(live) live=!has_pair(v.fields,v.header->field_count,node,{arg});"
                    ),
                    SUPERTYPE => format!(
                        "if(live) live=has_pair(v.supertypes,v.header->supertype_count,node,{arg});"
                    ),
                    SAVE_CURSOR => {
                        let declaration = if saved.insert(operand) { "uint " } else { "" };
                        format!("{declaration}cursor{operand}=node;")
                    }
                    RESTORE_CURSOR => format!("if(live) node=cursor{operand};"),
                    CAPTURE => {
                        let line = format!("uint2 capture{capture_count}=uint2(node,{arg});");
                        capture_count += 1;
                        line
                    }
                    EMIT => {
                        let mut line = format!(
                            "if(live) {{ uint local=ordinal++; bool accepted=true;if(Properties){{uint2 all_captures[{}];\n",
                            captures.max(1)
                        );
                        for index in 0..capture_count {
                            writeln!(line, "all_captures[{index}]=capture{index};").unwrap();
                        }
                        writeln!(line,"accepted=constant_capture_predicates(v,pg,all_captures,{capture_count}u);}}else{{uint prefix=register_prefix_label(v,pg,{capture_count}u);if(prefix!=NONE){{accepted=false;").unwrap();
                        for index in 0..capture_count {
                            writeln!(line,"accepted=accepted || (capture{index}.y==prefix && register_prefix_possible(v,pg,capture{index}.x));").unwrap();
                        }
                        writeln!(line,"}}}}if(accepted && total.x<maximum){{if(Store){{matches[total.x].local=local;matches[total.x].label={arg};matches[total.x].count={capture_count}u;for(uint c=0u;c<{}u;c++)matches[total.x].caps[c]=uint2(0u);",captures.max(1)).unwrap();
                        for index in 0..capture_count {
                            writeln!(line, "matches[total.x].caps[{index}]=capture{index};")
                                .unwrap();
                        }
                        line.push_str("} uint props=0u; if(Properties) for(uint step=0u;step<pg.output_count;step++){device const uint* op=v.output_words+pg.output_start+step*6u;if(op[0]==0u)props++;else if(op[0]==1u){");
                        for index in 0..capture_count {
                            write!(line, "props+=uint(op[1]==capture{index}.y);").unwrap();
                        }
                        write!(
                            line,
                            "}}}} total+=uint4(1u,{capture_count}u,props,0u); }} }}"
                        )
                        .unwrap();
                        line
                    }
                    _ => return Err(Error::BackendFailure),
                };
                writeln!(definitions, "{line}").unwrap();
            }
            definitions.push_str("}\n");
        }
        definitions.push_str("return total; }\n");
        writeln!(counts,"case {signature}u: {{ Match{suffix} unused[1]; return properties?run{suffix}<false,true>(v,pg,root,maximum,unused):run{suffix}<false>(v,pg,root,maximum,unused); }}").unwrap();
        writeln!(scatters,"case {signature}u: {{ Match{suffix} matches[{}]; uint count=run{suffix}<true>(v,pg,root,maximum,matches).x;", paths.len().max(1)).unwrap();
        scatters.push_str("register_order(v,pg,matches,count);\n");
        scatters.push_str("for(uint i=0;i<count;i++) { auto match=matches[i]; results[base.x]={ordinal,match.local,root,match.label,base.y,match.count,0u,0u,0u,0u}; for(uint c=0;c<match.count;c++) { uint2 cap=match.caps[c]; Node node=v.nodes[cap.x]; captures[base.y+c]={cap.y,cap.x,node.source_start,node.source_end,NONE,0u,0u,0u,1u}; } base+=uint4(1u,match.count,0u,0u); } return; }\n");
        writeln!(constant_scatters,"case {signature}u: {{ Match{suffix} matches[{}]; uint count=run{suffix}<true,true>(v,pg,root,maximum,matches).x;register_order(v,pg,matches,count);for(uint i=0u;i<count;i++){{auto match=matches[i];uint props=constant_emit(v,pg,ordinal,root,match.local,match.label,match.count,match.caps,base,writer);base+=uint4(1u,match.count,props,0u);}}return base;}}",paths.len().max(1)).unwrap();
    }
    let source = concat!(
        include_str!("metal_resident.metal"),
        include_str!("metal_constant_output_types.metal"),
        include_str!("metal_value_prefix.metal"),
        include_str!("metal_register_stages.metal"),
        include_str!("metal_constant_output.metal")
    )
    .replace("// GENERATED_REGISTER_FUNCTIONS", &definitions)
    .replace("// GENERATED_REGISTER_COUNTS", &counts)
    .replace("// GENERATED_REGISTER_SCATTERS", &scatters)
    .replace("// GENERATED_CONSTANT_SCATTERS", &constant_scatters)
    .replace("// GENERATED_CONSTANT_BOUNDS", &constant_bounds)
    .replace(
        "/* MAX_CONSTANT_DIRECTIVES */",
        &super::metal_constant_output::MAX_CONSTANT_DIRECTIVES.to_string(),
    );
    Ok(RegisterCode { source, bounds })
}
