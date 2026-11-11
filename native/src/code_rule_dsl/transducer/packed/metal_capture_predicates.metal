// One finite comparison per capture; reductions preserve empty-group semantics.
kernel void capture_predicate_clear(device const uchar* data [[buffer(0)]],device const uint4* meta [[buffer(1)]],device PredicateSpec* specs [[buffer(2)]],device uint4* counts [[buffer(3)]],device atomic_uint* status [[buffer(4)]],constant CaptureParams& p [[buffer(5)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.n)return;counts[i]=uint4(0u);
 View v=view(data);if(p.layout>1u || !capture_metadata_valid(v,meta[i])){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 if(p.layout==0u)specs[i]=query_spec(v,meta[i],p.step);

}
kernel void capture_inline_predicate_clear(device uint4* counts [[buffer(0)]],constant CaptureParams& p [[buffer(1)]],uint i [[thread_position_in_grid]]) {if(i<p.n)counts[i]=uint4(0u);}
kernel void capture_predicate_text(device const uchar* data [[buffer(0)]],device const ArgumentCapture* captures [[buffer(1)]],device const uint4* meta [[buffer(2)]],device const uint* owners [[buffer(3)]],device atomic_uint* counts [[buffer(4)]],device atomic_uint* status [[buffer(5)]],device const PredicateSpec* specs [[buffer(6)]],constant CaptureParams& p [[buffer(7)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.c || atomic_load_explicit(status,memory_order_relaxed))return;
 uint owner=owners[i];if(owner>=p.n){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 if(!meta[owner].w)return;View v=view(data);Program pg=v.programs[meta[owner].y];
 PredicateSpec assertion=argument_spec(v,meta,specs,owner,p);
 if(assertion.kind!=1u)return;
 ArgumentCapture capture=argument_capture(captures,i,p);if(capture.label!=assertion.first)return;
 Value value=v.values[pg.value_start+assertion.second];if(value.kind>=2u || v.strings[pg.string_start+value.operand].y>64u)return;
 uint answer=finite_text(v,pg,assertion.second,capture.node);
 if(answer==NONE){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 atomic_fetch_add_explicit(counts+owner*4u,1u,memory_order_relaxed);
 if(answer^(assertion.flags&1u))atomic_fetch_add_explicit(counts+owner*4u+1u,1u,memory_order_relaxed);
}
kernel void capture_predicate_accept(device const uchar* data [[buffer(0)]],device uint4* meta [[buffer(1)]],device const uint4* counts [[buffer(2)]],device const uint* failures [[buffer(3)]],device const uint* status [[buffer(4)]],device const PredicateSpec* specs [[buffer(5)]],constant CaptureParams& p [[buffer(6)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.n || status[0] || !meta[i].w)return;if(p.abort_evaluation && failures[meta[i].x]){meta[i].w=0u;return;}View v=view(data);Program pg=v.programs[meta[i].y];
 PredicateSpec assertion=argument_spec(v,meta,specs,i,p);if(assertion.kind==NONE)return;uint4 count=counts[i];
 bool keep=count.x!=0u && ((assertion.flags&2u)?count.y!=0u:count.y==count.x);
 if(assertion.kind==3u && !count.x && (assertion.flags&8u))keep=true;
 if(assertion.kind==4u && !count.z)keep=!(assertion.flags&1u);
 if(assertion.kind==5u)keep=!count.x?!(assertion.flags&1u):((assertion.flags&1u)?count.y==0u:count.y!=0u);
 if(count.w)keep=false;
 if(!keep)meta[i].w=0u;
}

kernel void capture_predicate_regex_counts(device const uchar* data [[buffer(0)]],device const uint* code [[buffer(1)]],device const ArgumentCapture* captures [[buffer(2)]],device const uint4* meta [[buffer(3)]],device const uint* owners [[buffer(4)]],device uint4* sizes [[buffer(5)]],device atomic_uint* status [[buffer(6)]],device const PredicateSpec* specs [[buffer(7)]],constant CaptureParams& p [[buffer(8)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.c)return;sizes[i]=uint4(0u);if(atomic_load_explicit(status,memory_order_relaxed))return;
 uint owner=owners[i];if(owner>=p.n){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 if(!meta[owner].w)return;View v=view(data);Program pg=v.programs[meta[owner].y];
 PredicateSpec assertion=argument_spec(v,meta,specs,owner,p);if(assertion.kind!=1u)return;uint value=pg.value_start+assertion.second;
 if(argument_capture(captures,i,p).label!=assertion.first || (v.values[value].kind!=2u && v.values[value].kind!=3u))return;
 uint width=code[code[1]+value*4u+1u];if(!width){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 Node node=v.nodes[argument_capture(captures,i,p).node];
 uint physical=regex_workgroup_predicate(code,value,width,node.source_end-node.source_start,0u)?0u:width;
 sizes[i]=uint4(1u,physical,0u,0u);
}
kernel void capture_predicate_regex_scatter(device const uchar* data [[buffer(0)]],device const uint* code [[buffer(1)]],device const ArgumentCapture* captures [[buffer(2)]],device const uint4* meta [[buffer(3)]],device const uint* owners [[buffer(4)]],device const uint4* offsets [[buffer(5)]],device const uint4* total [[buffer(6)]],device RegexRequest* requests [[buffer(7)]],device uint* ids [[buffer(8)]],device atomic_uint* status [[buffer(9)]],device const PredicateSpec* specs [[buffer(10)]],constant CaptureParams& p [[buffer(11)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.c || atomic_load_explicit(status,memory_order_relaxed))return;
 uint4 first=offsets[i],last=i+1u<p.c?offsets[i+1u]:total[0];if(last.x==first.x)return;
 if(last.x!=first.x+1u || first.x>=total[0].x){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint owner=owners[i];View v=view(data);Program pg=v.programs[meta[owner].y];PredicateSpec assertion=argument_spec(v,meta,specs,owner,p);if(assertion.kind!=1u){atomic_store_explicit(status,2u,memory_order_relaxed);return;}uint value=pg.value_start+assertion.second;
 uint width=code[code[1]+value*4u+1u];Node node=v.nodes[argument_capture(captures,i,p).node];
 uint physical=regex_workgroup_predicate(code,value,width,node.source_end-node.source_start,0u)?0u:width;
 if(last.y-first.y!=physical || first.y>total[0].y || physical>total[0].y-first.y){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 requests[first.x]={value,node.source_start,node.source_end-node.source_start,0u,REGEX_WORK_LIMIT,first.y,width,0u,0u,0u,0u,NONE,NONE,0u,owner,meta[owner].x,0u,0u,0u,0u};ids[first.x]=first.x;
}
kernel void capture_predicate_regex_results(device const uchar* data [[buffer(0)]],device const RegexRequest* requests [[buffer(1)]],device const uint4* total [[buffer(2)]],device const uint4* meta [[buffer(3)]],device atomic_uint* counts [[buffer(4)]],device const uint* failures [[buffer(5)]],device atomic_uint* status [[buffer(6)]],device const PredicateSpec* specs [[buffer(7)]],constant CaptureParams& p [[buffer(8)]],uint i [[thread_position_in_grid]]) {
 if(i>=total[0].x || atomic_load_explicit(status,memory_order_relaxed))return;
 RegexRequest r=requests[i];View v=view(data);
 if(r.owner>=p.n || r.evaluation!=meta[r.owner].x || r.evaluation>=v.header->evaluation_count){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 if(r.status==3u || r.status==5u){
  if(p.abort_evaluation){if(failures[r.evaluation]!=2u)atomic_store_explicit(status,2u,memory_order_relaxed);}
  else if(r.status==3u)atomic_store_explicit(counts+r.owner*4u+3u,1u,memory_order_relaxed);
  else atomic_store_explicit(status,2u,memory_order_relaxed);
  return;
 }
 if((r.status!=1u && r.status!=2u) || !r.used || r.used>r.budget){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 Program pg=v.programs[meta[r.owner].y];
 PredicateSpec assertion=argument_spec(v,meta,specs,r.owner,p);
 if(assertion.kind!=1u || r.value!=pg.value_start+assertion.second){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 if(p.abort_evaluation && failures[r.evaluation])return;
 atomic_fetch_add_explicit(counts+r.owner*4u,1u,memory_order_relaxed);
 if(uint(r.status==2u)^(assertion.flags&1u))atomic_fetch_add_explicit(counts+r.owner*4u+1u,1u,memory_order_relaxed);
}
