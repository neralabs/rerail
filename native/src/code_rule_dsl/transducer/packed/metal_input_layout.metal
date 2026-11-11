// The Rust producer selects one closed input schema before shader compilation.
// Initial records contain task, original root, and Program. Their PC, values,
// positions, and work are intrinsically zero; they cannot carry active state.
#if INPUT_COLUMNS == 3
bool initial_root_matches(View v,Span span,Evaluation evaluation,uint task,uint node) {
 uint index=select_root(v,v.selectors[evaluation.selector],span.selector_offset+task-span.semantic_start);
 if(index>=v.header->root_count)return false;
 Root root=v.roots[index];File file=v.files[evaluation.file];
 return root.file==evaluation.file && root.node<file.node_count && node==file.node_start+root.node;
}
#endif
uint input_scalar(View v,device const uint* s,uint n,uint field,uint i) {
#if INPUT_COLUMNS == 3
 if(field==0u)return s[i];
 if(field==2u)return s[n+i];
 if(field==4u)return s[2u*n+i];
 if(field==1u || (field>=6u && field<12u))return 0u;
 if(field==3u || field==5u) {
  uint program=s[2u*n+i];if(program>=v.header->program_count)return NONE;
  Program pg=v.programs[program];if(!pg.structural_count)return NONE;
  if(field==5u)return (pg.flags&2u)?0u:2u;
  uint op=v.words[pg.structural_start]&255u;
  return op==17u?1u:(op==7u?2u:(op==8u?3u:0u));
 }
 return NONE;
#else
 return field<12u?s[field*n+i]:NONE;
#endif
}
uint input_control(device const uint* spans,uint n,uint field,uint i) {
 if(field>=6u)return NONE;
#if INPUT_COLUMNS == 3
 return 0u;
#else
 return spans[field*n+i];
#endif
}
uint input_value_span(device const uint* values,uint n,uint field,uint i) {
 if(field<6u || field>=12u)return NONE;
#if INPUT_COLUMNS == 3
 return 0u;
#else
 return values[field*n+i];
#endif
}
uint input_work(device const uint* work,uint i) {
#if INPUT_COLUMNS == 3
 return 0u;
#else
 return work[i];
#endif
}
uint input_list_base(uint4 lengths,uint capacity,uint kind) {
#if INPUT_COLUMNS == 3
 uint base=0u;for(uint k=0u;k<kind;k++)base+=lengths[k];return base;
#else
 return kind*capacity;
#endif
}
