uint4 constant_scatter_registers(View v,uint signature,Program pg,uint root,uint maximum,
 uint ordinal,uint4 base,ConstantWriter writer){switch(signature){// GENERATED_CONSTANT_SCATTERS
 }return uint4(NONE);}
uint constant_lower(device const Result* rows,uint n,uint task){uint lo=0u,hi=n;
 while(lo<hi){uint mid=lo+(hi-lo)/2u;if(rows[mid].task_ordinal<task)lo=mid+1u;else hi=mid;}return lo;}
uint4 constant_prefix(device const uint4* offsets,device const uint4* total,uint n,uint i){return i<n?offsets[i]:total[0];}
ulong constant_column_words(uint rows,uint bits){return ulong(rows/256u)*bits*8u+ulong(((rows%256u)*bits+255u)/256u)*8u;}
kernel void constant_output_plan(
 device const uchar* data [[buffer(0)]],device const uint4* offsets [[buffer(1)]],device const uint4* total [[buffer(2)]],
 device const Result* rows [[buffer(3)]],device const uint4* layout [[buffer(4)]],device uint4* segments [[buffer(5)]],
 device uint4* sizes [[buffer(6)]],device uint4* semantic_counts [[buffer(7)]],device atomic_uint* status [[buffer(8)]],
 constant uint4& complex [[buffer(9)]],uint i [[thread_position_in_grid]]){
 View v=view(data);uint n=v.header->span_count;if(i>=n)return;
 Span span=v.spans[v.semantic_order[i]];Evaluation evaluation=v.evaluations[span.evaluation];Program pg=v.programs[evaluation.program];File file=v.files[evaluation.file];
 uint2 bound=constant_output_bound(v.blocks[span.block].signature);
 bool simple=bound.x!=NONE && constant_output_program(v,pg);
 uint4 counts=constant_prefix(offsets,total,n,i+1u)-offsets[i];
 uint first=constant_lower(rows,complex.x,span.semantic_start),last=constant_lower(rows,complex.x,span.semantic_start+span.root_count);
 sizes[i]=uint4(0u);
 if(simple){
  bool valid=first==last && !counts.w && ulong(counts.x)<=ulong(span.root_count)*bound.x &&
   ulong(counts.y)<=ulong(counts.x)*bound.y && ulong(counts.z)<=ulong(counts.x)*(ulong(bound.y)+1ul)*pg.output_count &&
   (counts.x || !any(counts!=uint4(0u)));
  if(!valid){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
  segments[i*3u]=uint4(1u,0u,counts.x,counts.y);segments[i*3u+1u]=uint4(counts.z,0u,0u,0u);segments[i*3u+2u]=uint4(0u);
  ulong headers=0ul,words=0ul;
  uint4 lengths=uint4(counts.x,counts.y,counts.z,counts.x);
  for(uint arena=0u;arena<4u;arena++){
   uint rows=lengths[arena],fields=constant_fields(arena);headers+=ulong((rows+255u)/256u)*fields;
   if(rows)for(uint field=0u;field<fields;field++){
    uint2 range=constant_range(v,span,file,pg,bound,counts,arena,field);
    if(range.y<range.x){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
    words+=constant_column_words(rows,constant_width(range));
   }
  }
  if(headers>=NONE || words>500000000ul){atomic_store_explicit(status,1u,memory_order_relaxed);return;}
  sizes[i]=uint4(uint(headers),uint(words),0u,0u);
 }else{
  if(any(counts!=uint4(0u)) || first>last){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
  uint cap_first=first<complex.x?rows[first].capture_start:complex.y,cap_last=last<complex.x?rows[last].capture_start:complex.y;
  uint2 begin=first<complex.x?layout[first].zw:complex.zw,end=last<complex.x?layout[last].zw:complex.zw;
  if(cap_first>cap_last || cap_last>complex.y || any(begin>end) || any(end>complex.zw)){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
  counts=uint4(last-first,cap_last-cap_first,end-begin);
  segments[i*3u]=uint4(2u,NONE,counts.x,counts.y);segments[i*3u+1u]=uint4(counts.zw,0u,0u);
  segments[i*3u+2u]=uint4(first,cap_first,begin);
 }
 semantic_counts[i]=counts;
}
kernel void constant_output_headers(
 device const uchar* data [[buffer(0)]],device const uint4* sizes [[buffer(1)]],device uint4* segments [[buffer(2)]],
 device uint2* headers [[buffer(3)]],device atomic_uint* status [[buffer(4)]],constant uint4& p [[buffer(5)]],uint i [[thread_position_in_grid]]){
 if(i>=p.x)return;uint4 m=segments[i*3u];if(m.x!=1u)return;
 View v=view(data);Span span=v.spans[v.semantic_order[i]];Evaluation e=v.evaluations[span.evaluation];Program pg=v.programs[e.program];File file=v.files[e.file];
 uint2 bound=constant_output_bound(v.blocks[span.block].signature);uint4 extra=segments[i*3u+1u],counts=uint4(m.zw,extra.xy),lengths=uint4(m.z,m.w,extra.x,m.z);
 uint group=sizes[i].x,word=sizes[i].y;segments[i*3u].y=group;
 for(uint arena=0u;arena<4u;arena++)for(uint first=0u;first<lengths[arena];first+=256u){
  uint count=min(256u,lengths[arena]-first);
  for(uint field=0u;field<constant_fields(arena);field++){
   uint2 range=constant_range(v,span,file,pg,bound,counts,arena,field);uint bits=constant_width(range),need=((count*bits+255u)/256u)*8u;
   if(group>=p.y || word>p.z || need>p.z-word || word%8u || bits>32u){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
   headers[group++]=uint2(range.x,word*4u+(bits&31u)+((bits>>5u)<<31u));word+=need;
  }
 }
 uint2 end=i+1u<p.x?sizes[i+1u].xy:p.yz;
 if(group!=end.x || word!=end.y)atomic_store_explicit(status,2u,memory_order_relaxed);
}
struct ConstantScatterParams{Params base;uint headers,words;};
kernel void constant_output_scatter(
 device const uchar* data [[buffer(0)]],device const uint* resolved [[buffer(1)]],device const uint* ids [[buffer(2)]],
 device const uint4* unlimited [[buffer(3)]],device const uint* progress [[buffer(4)]],device const uint4* segments [[buffer(5)]],
 device const uint2* headers [[buffer(6)]],device atomic_uint* words [[buffer(7)]],device atomic_uint* status [[buffer(8)]],
 constant ConstantScatterParams& parameters [[buffer(9)]],uint group [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]){
 Params p=parameters.base;View v=view(data);Block block=v.blocks[ids[group]];Lane item=lane_input(v,block,lane,resolved,p);
 bool owns=register_owns(v,item,p);if(owns && !constant_root_valid(v,item)){atomic_store_explicit(status,2u,memory_order_relaxed);owns=false;}
 uint4 raw=owns?count_registers(v,block.signature,item.pg,item.node,NONE,true):uint4(0u);
 threadgroup uint4 values[256];threadgroup uint spans[256];values[lane]=raw;spans[lane]=item.span;threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint step=1u;step<256u;step*=2u){uint4 add=lane>=step && spans[lane-step]==item.span?values[lane-step]:uint4(0u);threadgroup_barrier(mem_flags::mem_threadgroup);values[lane]+=add;threadgroup_barrier(mem_flags::mem_threadgroup);}
 uint maximum=owns?maximum_matches(v,item,p,values[lane].x-raw.x,unlimited,progress):0u;
 uint4 exact=owns?count_registers(v,block.signature,item.pg,item.node,maximum,true):uint4(0u);
 values[lane]=exact;threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint step=1u;step<256u;step*=2u){uint4 add=lane>=step && spans[lane-step]==item.span?values[lane-step]:uint4(0u);threadgroup_barrier(mem_flags::mem_threadgroup);values[lane]+=add;threadgroup_barrier(mem_flags::mem_threadgroup);}
 if(owns){uint4 base=values[lane]-exact;ConstantWriter writer{segments,headers,words,status,item.position,parameters.headers,parameters.words};
  uint4 end=constant_scatter_registers(v,block.signature,item.pg,item.node,maximum,item.ordinal,base,writer);
  if(any(end-base!=exact))atomic_store_explicit(status,2u,memory_order_relaxed);
 }
}
