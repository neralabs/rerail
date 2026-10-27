struct InlineParams {uint capacity,arguments,captures,spans,output,states;};
uint inline_column(device const uint* states,uint n,uint column,uint i){return states[column*n+i];}
uint inline_owner(device const uint4* offsets,uint n,uint index,uint field){uint lo=0u,hi=n;while(lo<hi){uint mid=lo+(hi-lo)/2u;if(offsets[mid][field]<=index)lo=mid+1u;else hi=mid;}return lo?lo-1u:NONE;}
kernel void inline_argument_counts(device const uchar* data [[buffer(0)]],device const uint* states [[buffer(1)]],device const uint* header [[buffer(2)]],device uint4* sizes [[buffer(3)]],device const uint* status [[buffer(4)]],constant InlineParams& p [[buffer(5)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.capacity)return;sizes[i]=uint4(0u);uint n=header[0];if(status[0] || i>=n || !frame_predicate(states[5u*n+i]))return;
 View v=view(data);Program pg=v.programs[states[4u*n+i]];uint op=v.words[pg.structural_start+states[n+i]]&255u;
 sizes[i]=uint4(1u,op==4u?1u:states[11u*n+i],0u,0u);
}
kernel void inline_argument_frames(device const uchar* data [[buffer(0)]],device const uint* states [[buffer(1)]],device const uint* header [[buffer(2)]],device const uint* work [[buffer(3)]],device const uint4* offsets [[buffer(4)]],device const uint4* total [[buffer(5)]],device CaptureSpan* spans [[buffer(6)]],device PredicateSpec* specs [[buffer(7)]],device uint4* meta [[buffer(8)]],device uint* argument_work [[buffer(9)]],device atomic_uint* status [[buffer(10)]],constant InlineParams& p [[buffer(11)]],uint i [[thread_position_in_grid]]) {
 uint n=header[0];if(n!=p.states){if(!i)atomic_store_explicit(status,2u,memory_order_relaxed);return;}if(i>=n || atomic_load_explicit(status,memory_order_relaxed))return;
 uint4 first=offsets[i],last=i+1u<p.capacity?offsets[i+1u]:total[0];bool pending=frame_predicate(states[5u*n+i]);
 if(last.x-first.x!=uint(pending)){atomic_store_explicit(status,2u,memory_order_relaxed);return;}if(!pending)return;
 uint owner=first.x,pg_index=states[4u*n+i],pc=states[n+i];View v=view(data);Program pg=v.programs[pg_index];uint word=v.words[pg.structural_start+pc],op=word&255u,arg=word>>16;
 uint count=op==4u?1u:states[11u*n+i];
 if(owner>=p.arguments || last.y-first.y!=count || first.y>p.captures || count>p.captures-first.y || (op!=4u && op!=6u) || (op==6u && (pg.flags&1u)) || work[i]>PATH_WORK_LIMIT){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint task=states[i],lo=0u,hi=p.spans;while(lo<hi){uint mid=lo+(hi-lo)/2u;if(v.spans[v.semantic_order[mid]].semantic_start<=task)lo=mid+1u;else hi=mid;}
 if(!lo){atomic_store_explicit(status,2u,memory_order_relaxed);return;}Span span=v.spans[v.semantic_order[lo-1u]];Evaluation e=v.evaluations[span.evaluation];
 if(task-span.semantic_start>=span.root_count || e.program!=pg_index){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 spans[owner]={first.y,count};meta[owner]=uint4(span.evaluation,pg_index,e.file,1u);argument_work[owner]=work[i];
 if(op==4u)specs[owner]={1u,0u,arg,0u,0u,0u,NONE,0u};
 else {uint index=pg.capture_start+arg;Assertion a=v.assertions[index];specs[owner]={a.kind,a.first,a.second,a.data_start,a.data_count,a.flags,index,0u};}
}
kernel void inline_argument_captures(device const uchar* data [[buffer(0)]],device const uint* states [[buffer(1)]],device const uint* header [[buffer(2)]],device const uint4* captures [[buffer(3)]],device const uint4* offsets [[buffer(4)]],device ArgumentCapture* output [[buffer(5)]],device uint* owners [[buffer(6)]],device atomic_uint* status [[buffer(7)]],constant InlineParams& p [[buffer(8)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.captures || atomic_load_explicit(status,memory_order_relaxed))return;uint n=header[0],row=inline_owner(offsets,p.capacity,i,1u);
 if(row>=n || !frame_predicate(states[5u*n+row])){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint local=i-offsets[row].y;View v=view(data);Program pg=v.programs[states[4u*n+row]];uint op=v.words[pg.structural_start+states[n+row]]&255u;
 ArgumentCapture cap;if(op==4u){Node node=v.nodes[states[2u*n+row]];cap={states[2u*n+row],0u,node.source_start,node.source_end};}
 else {uint4 cell=captures[states[10u*n+row]+local];cap={cell.x,cell.y,cell.z,cell.w};}
 output[i]=cap;owners[i]=offsets[row].x;
}
kernel void inline_continuation_counts(device const uint4* argument_offsets [[buffer(0)]],device const uint4* total [[buffer(1)]],device const uint4* meta [[buffer(2)]],device const uint4* outcomes [[buffer(3)]],device uint4* sizes [[buffer(4)]],device atomic_uint* status [[buffer(5)]],constant InlineParams& p [[buffer(6)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.capacity)return;sizes[i]=uint4(0u);if(atomic_load_explicit(status,memory_order_relaxed))return;
 if(p.states>p.capacity || total[0].x!=p.arguments || total[0].y!=p.captures || any(total[0].zw!=uint2(0u))){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 if(i>=p.states)return;
 uint owner=argument_offsets[i].x,end=i+1u<p.capacity?argument_offsets[i+1u].x:total[0].x;
 if(owner>p.arguments || end<owner || end>p.arguments || end-owner>1u){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 bool keep=true;if(end!=owner){uint accepted=meta[owner].w,failed=outcomes[owner].w;if(accepted>1u || failed>1u || (accepted && failed)){atomic_store_explicit(status,2u,memory_order_relaxed);return;}keep=accepted || failed;}
 sizes[i].x=uint(keep);
}
kernel void inline_continuation_publish(device const uchar* data [[buffer(0)]],device const uint* states [[buffer(1)]],device const uint* header [[buffer(2)]],device const uint* spans [[buffer(3)]],device const uint* work [[buffer(4)]],device const uint4* argument_offsets [[buffer(5)]],device const uint4* meta [[buffer(6)]],device const uint4* outcomes [[buffer(7)]],device const uint* argument_work [[buffer(8)]],device const uint4* offsets [[buffer(9)]],device const uint4* total [[buffer(10)]],device uint* output [[buffer(11)]],device uint* output_header [[buffer(12)]],device uint* output_spans [[buffer(13)]],device uint* output_work [[buffer(14)]],device atomic_uint* status [[buffer(15)]],constant InlineParams& p [[buffer(16)]],uint i [[thread_position_in_grid]]) {
 if(atomic_load_explicit(status,memory_order_relaxed))return;uint n=header[0],m=total[0].x;
 if(n!=p.states || n>p.capacity){if(!i)atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 if(m>p.output){if(!i)atomic_store_explicit(status,1u,memory_order_relaxed);return;}
 if(!i){output_header[0]=m;output_header[1]=header[1];output_header[2]=(m+255u)/256u;output_header[3]=output_header[4]=1u;}
 if(i>=n)return;uint first=offsets[i].x,last=i+1u<p.capacity?offsets[i+1u].x:m;if(last==first)return;
 if(last!=first+1u || first>=m){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 for(uint column=0u;column<12u;column++)output[column*m+first]=states[column*n+i];
 for(uint column=0u;column<6u;column++)output_spans[column*m+first]=spans[column*n+i];output_work[first]=work[i];
 if(frame_predicate(states[5u*n+i])){
  uint owner=argument_offsets[i].x,end=i+1u<p.capacity?argument_offsets[i+1u].x:p.arguments;
  if(owner>=p.arguments || end!=owner+1u){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
  if(outcomes[owner].w){output[5u*m+first]=frame_query(states[5u*n+i])?FRAME_QUERY_WORK_FAILURE:FRAME_ORDINARY_WORK_FAILURE;output_work[first]=PATH_WORK_LIMIT+1u;}
  else {View v=view(data);Program pg=v.programs[states[4u*n+i]];uint pc=states[n+i]+1u;
   if(!meta[owner].w || pc>=pg.structural_count || argument_work[owner]>PATH_WORK_LIMIT){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
   output[m+first]=pc;uint op=v.words[pg.structural_start+pc]&255u;output[3u*m+first]=op==17u?1u:(op==7u?2u:(op==8u?3u:0u));output[5u*m+first]=(pg.flags&2u)?0u:2u;output_work[first]=argument_work[owner];
  }
 }
}
