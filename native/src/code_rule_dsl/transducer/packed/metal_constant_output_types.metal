// Finite literal queries and constant directives write final literal columns.
// The same finite run preserves raw ordinals before capture-predicate filtering.
constant uint CONSTANT_OUTPUT_DIRECTIVE_LIMIT=/* MAX_CONSTANT_DIRECTIVES */;
bool constant_output_program(View v,Program pg) {
 if(pg.output_count>CONSTANT_OUTPUT_DIRECTIVE_LIMIT)return false;
 if(pg.capture_count){
  if(!(pg.flags&1u) || pg.capture_count>CONSTANT_OUTPUT_DIRECTIVE_LIMIT)return false;
  uint captures=0u;for(uint pc=0u;pc<pg.structural_count;pc++)captures+=uint((v.words[pg.structural_start+pc]&255u)==0x20u);
  ulong comparisons=0ul;
  for(uint i=0u;i<pg.capture_count;i++){
   Assertion a=v.assertions[pg.capture_start+i];
   if(a.kind==1u){Value value=v.values[pg.value_start+a.second];if(value.kind>1u || v.strings[pg.string_start+value.operand].y>64u)return false;comparisons++;}
   else if(a.kind==2u && a.data_count<=CONSTANT_OUTPUT_DIRECTIVE_LIMIT){
    for(uint j=0u;j<a.data_count;j++)if(v.strings[pg.string_start+v.output_words[a.data_start+j]].y>64u)return false;
    comparisons+=max(1u,a.data_count);
   }else return false;
  }
  if(ulong(captures)*comparisons*66ul>PATH_WORK_LIMIT)return false;
 }
 for(uint i=0u;i<pg.output_count;i++)if(v.output_words[pg.output_start+i*6u]>1u)return false;
 return true;
}
__attribute__((noinline)) bool constant_set_member(View v,Program pg,Assertion a,uint node){
 Node n=v.nodes[node];
 for(uint j=0u;j<a.data_count;j++){
  uint2 literal=v.strings[pg.string_start+v.output_words[a.data_start+j]];
  if(n.source_end-n.source_start!=literal.y)continue;
  bool same=true;for(uint k=0u;k<64u;k++)if(k<literal.y)same=same && v.source[n.source_start+k]==v.string_bytes[literal.x+k];
  if(same)return true;
 }
 return false;
}
// Keep bounded literal loops shared across generated finite paths.
__attribute__((noinline)) bool constant_capture_predicates(View v,Program pg,thread const uint2* captures,uint count){
 for(uint i=0u;i<pg.capture_count;i++){
  Assertion a=v.assertions[pg.capture_start+i];uint members=0u,passed=0u;
  for(uint c=0u;c<count;c++)if(captures[c].y==a.first){members++;bool answer=a.kind==1u?finite_text(v,pg,a.second,captures[c].x)==1u:constant_set_member(v,pg,a,captures[c].x);passed+=uint(answer)^(a.flags&1u);}
  if(!members || ((a.flags&2u)?!passed:passed!=members))return false;
 }
 return true;
}
uint2 constant_output_bound(uint signature);
bool constant_root_valid(View v,Lane item){
 if(item.span==NONE)return false;Span span=v.spans[item.span];
 if(item.ordinal<span.semantic_start || item.ordinal-span.semantic_start>=span.root_count)return false;
 Evaluation e=v.evaluations[span.evaluation];uint index=select_root(v,v.selectors[e.selector],span.selector_offset+item.ordinal-span.semantic_start);
 if(index>=v.header->root_count)return false;Root root=v.roots[index];File file=v.files[e.file];
 return root.file==e.file && root.node<file.node_count && ulong(file.node_start)+root.node==item.node;
}
struct ConstantWriter {
 device const uint4* segments;device const uint2* headers;
 device atomic_uint* words;device atomic_uint* status;
 uint span,header_count,word_count;
};
uint constant_fields(uint arena){return arena==0u?10u:(arena==1u?9u:(arena==2u?8u:4u));}
uint constant_width(uint2 range){return range.x==range.y?0u:32u-clz(range.y-range.x);}
uint2 constant_range(View v,Span span,File file,Program pg,uint2 bound,uint4 counts,uint arena,uint field) {
 if(arena==0u){
  if(field==0u)return uint2(span.semantic_start,span.semantic_start+span.root_count-1u);
  if(field==1u)return uint2(0u,bound.x-1u);
  if(field==2u)return uint2(file.node_start,file.node_start+file.node_count-1u);
  if(field==3u)return uint2(0u,pg.label_count-1u);
  if(field==4u)return uint2(0u,counts.y);
  if(field==5u)return uint2(0u,bound.y);
  if(field==6u)return uint2(0u,counts.z);
  if(field==7u)return uint2(0u,pg.output_count);
  return uint2(0u);
 }
 if(arena==1u){
  if(field==0u)return uint2(0u,pg.label_count-1u);
  if(field==1u)return uint2(file.node_start,file.node_start+file.node_count-1u);
  if(field==2u || field==3u)return uint2(file.source_start,file.source_start+file.source_length);
  if(field==4u)return uint2(NONE);
  if(field==6u)return uint2(0u,counts.z);
  if(field==7u)return uint2(0u,pg.output_count);
  return uint2(field==8u?1u:0u);
 }
 if(arena==2u){
  if(field==0u)return uint2(0u,1u);
  if(field==1u)return uint2(0u,max(counts.x,counts.y)-1u);
  if(field==2u || field==3u){uint lo=NONE,hi=0u;
   for(uint i=0u;i<pg.output_count;i++){device const uint* op=v.output_words+pg.output_start+i*6u;
    uint value=op[(op[0]==0u?1u:2u)+(field==3u?1u:0u)];lo=min(lo,value);hi=max(hi,value);}
   return uint2(lo,hi);
  }
  return uint2(0u);
 }
 if(field==0u || field==2u)return uint2(0u,counts.z);
 return uint2(0u);
}
void constant_write(ConstantWriter writer,uint arena,uint row,thread const uint* values) {
 uint4 m=writer.segments[writer.span*3u],extra=writer.segments[writer.span*3u+1u];
 uint4 lengths=uint4(m.z,m.w,extra.x,m.z);uint first=m.y,fields=constant_fields(arena);
 if(m.x!=1u || row>=lengths[arena]){atomic_store_explicit(writer.status,2u,memory_order_relaxed);return;}
 for(uint k=0u;k<arena;k++)first+=((lengths[k]+255u)/256u)*constant_fields(k);
 for(uint field=0u;field<fields;field++){
  uint group=first+(row/256u)*fields+field;
  if(group>=writer.header_count){atomic_store_explicit(writer.status,2u,memory_order_relaxed);return;}
  uint2 h=writer.headers[group];uint bits=(h.y&31u)+(h.y>>31u)*32u,at=(h.y&0x7fffffe0u)/4u;
  uint value=values[field];
  if(bits>32u || value<h.x || (bits<32u && ulong(value)-h.x>=(1ul<<bits))){atomic_store_explicit(writer.status,2u,memory_order_relaxed);return;}
  uint count=min(256u,lengths[arena]-(row/256u)*256u),need=((count*bits+255u)/256u)*8u;
  if(at>writer.word_count || need>writer.word_count-at){atomic_store_explicit(writer.status,2u,memory_order_relaxed);return;}
  uint delta=value-h.x;if(!bits || !delta)continue;
  uint bit=(row%256u)*bits,word=bit/32u,shift=bit%32u;
  bool crossed=shift && bits>32u-shift;
  if(ulong(at)+word+uint(crossed)>=writer.word_count){atomic_store_explicit(writer.status,2u,memory_order_relaxed);return;}
  atomic_fetch_or_explicit(writer.words+at+word,delta<<shift,memory_order_relaxed);
  if(crossed)atomic_fetch_or_explicit(writer.words+at+word+1u,delta>>(32u-shift),memory_order_relaxed);
 }
}
uint constant_emit(View v,Program pg,uint ordinal,uint root,uint local,uint label,uint count,
 thread const uint2* captures,uint4 base,ConstantWriter writer) {
 uint match_properties=0u;
 for(uint i=0u;i<pg.output_count;i++)match_properties+=uint(v.output_words[pg.output_start+i*6u]==0u);
 uint property=base.z;
 for(uint i=0u;i<pg.output_count;i++){device const uint* op=v.output_words+pg.output_start+i*6u;
  if(op[0]==0u){uint values[8]={0u,base.x,op[1],op[2],0u,0u,0u,0u};constant_write(writer,2u,property++,values);}
 }
 uint row[10]={ordinal,local,root,label,base.y,count,match_properties?base.z:0u,match_properties,0u,0u};
 constant_write(writer,0u,base.x,row);
 for(uint c=0u;c<count;c++){
  uint props=0u;for(uint i=0u;i<pg.output_count;i++){device const uint* op=v.output_words+pg.output_start+i*6u;props+=uint(op[0]==1u && op[1]==captures[c].y);}
  Node node=v.nodes[captures[c].x];uint cap[9]={captures[c].y,captures[c].x,node.source_start,node.source_end,NONE,0u,props?property:0u,props,1u};
  constant_write(writer,1u,base.y+c,cap);
  for(uint i=0u;i<pg.output_count;i++){device const uint* op=v.output_words+pg.output_start+i*6u;
   if(op[0]==1u && op[1]==captures[c].y){uint values[8]={1u,base.y+c,op[2],op[3],0u,0u,0u,0u};constant_write(writer,2u,property++,values);}
  }
 }
 uint layout[4]={property-base.z,0u,base.z,0u};constant_write(writer,3u,base.x,layout);
 return property-base.z;
}
