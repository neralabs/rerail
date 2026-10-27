// The shared resident view and selector operations precede this source.
#ifndef SEED_COLUMNS
#define SEED_COLUMNS 12u
#endif
// The host compiles immutable root eligibility, not semantic task instances.
// Each proof stops before any instruction that can fail or observe admission.
bool seed_possible(View v,Lane item,device const uint4* probes) {
 uint pg=v.evaluations[v.spans[item.span].evaluation].program;
 uint4 probe=probes[pg*2u];uint op=probe.x&255u,mode=(probe.x>>8u)&255u;
 Node node=v.nodes[item.node];
 if((node.flags&probe.y)!=probe.y || (node.flags&probe.z)!=0u || (probe.w!=NONE && node.kind!=probe.w))return false;
 if(op==0x10u) {
  if(mode==1u?v.relations[item.node].x==NONE:node.child_count==0u)return false;
  uint4 guard=probes[pg*2u+1u];if(guard.x==NONE)return true;
  uint child=unique_child(v,item.node,guard.x,guard.y);if(child==NONE)return false;
  Node selected=v.nodes[child];
  return (mode!=1u || (selected.flags&1u)!=0u) && (guard.z==NONE || selected.field==guard.z) && (guard.w==NONE || selected.kind==guard.w);
 }
 if(op==0x13u)return node.descendant_end>item.node+1u;
 if(op==0x14u)return (mode==1u?node.next_named:node.next_any)!=NONE;
 return true;
}
kernel void seed_span_counts(device const uchar* data [[buffer(0)]],device const uint* roots [[buffer(1)]],device const uint* ids [[buffer(2)]],device uint4* counts [[buffer(3)]],device uint* masks [[buffer(4)]],device atomic_uint* status [[buffer(5)]],device const uint4* probes [[buffer(6)]],constant Params& p [[buffer(7)]],uint group [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]) {
 View v=view(data); Block block=v.blocks[ids[group]];Lane item=lane_input(v,block,lane,roots,p);
 threadgroup uint keeps[256],prefix[256],spans[256];uint keep=0u;
 if(item.span!=NONE) {
  Span span=v.spans[item.span];Evaluation evaluation=v.evaluations[span.evaluation];
  File file=v.files[evaluation.file];
  uint index=select_root(v,v.selectors[evaluation.selector],span.selector_offset+item.ordinal-span.semantic_start);
  bool valid=index<v.header->root_count;
  if(valid){Root root=v.roots[index];valid=root.file==evaluation.file && root.node<file.node_count && item.node==file.node_start+root.node;}
  if(!valid || item.node<file.node_start || item.node-file.node_start>=file.node_count)atomic_store_explicit(status,2u,memory_order_relaxed);
  else {
   uint program=v.evaluations[v.spans[item.span].evaluation].program;
   uint possible=initial_root_status(v,program,item.node,file.node_start,file.node_count);
   if(possible>1u)atomic_store_explicit(status,2u,memory_order_relaxed);
   else keep=uint(possible!=0u && seed_possible(v,item,probes));
  }
 }
 keeps[lane]=prefix[lane]=keep;spans[lane]=item.span;threadgroup_barrier(mem_flags::mem_threadgroup);
 if(lane%32u==0u) {uint bits=0u;for(uint k=0u;k<32u;k++)bits|=keeps[lane+k]<<k;masks[group*8u+lane/32u]=bits;}
 for(uint step=1u;step<256u;step<<=1u) {
  uint add=lane>=step && spans[lane-step]==item.span?prefix[lane-step]:0u;
  threadgroup_barrier(mem_flags::mem_threadgroup);prefix[lane]+=add;threadgroup_barrier(mem_flags::mem_threadgroup);
 }
 if(item.span!=NONE && (lane==255u || spans[lane+1u]!=item.span))counts[item.position]=uint4(prefix[lane],0u,0u,0u);
}
kernel void seed_boundary(device const uint4* total [[buffer(0)]],device uint* counts [[buffer(1)]],device atomic_uint* status [[buffer(2)]],constant Params& p [[buffer(3)]],uint tid [[thread_position_in_grid]]) {
 if(tid) return;
 uint4 size=total[0];
 uint failure=atomic_load_explicit(status,memory_order_relaxed);
 if(!failure)failure=any(size.yzw!=uint3(0u))?2u:((size.x>p.result_capacity || size.x==NONE)?1u:0u);
 atomic_store_explicit(status,failure,memory_order_relaxed);
 uint n=failure?0u:size.x;
 counts[0]=n; counts[1]=0u; counts[2]=n/256u+uint(n%256u!=0u); counts[3]=counts[4]=1u;
}
kernel void seed_scatter(device const uchar* data [[buffer(0)]],device const uint* roots [[buffer(1)]],device const uint* ids [[buffer(2)]],device const uint4* offsets [[buffer(3)]],device uint* states [[buffer(4)]],device const uint* counts [[buffer(5)]],device atomic_uint* status [[buffer(6)]],device const uint* masks [[buffer(7)]],constant Params& p [[buffer(8)]],uint group [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]) {
 if(atomic_load_explicit(status,memory_order_relaxed)!=0u) return;
 View v=view(data); Block block=v.blocks[ids[group]];
 Lane item=lane_input(v,block,lane,roots,p);
 if(item.span==NONE) return;
 if((masks[group*8u+lane/32u]&(1u<<(lane%32u)))==0u)return;
 Span span=v.spans[item.span];
 uint rank=0u;
 for(uint word=0u;word<8u;word++) {
  uint first=word*32u,last=first+32u,lo=max(first,span.lane_start),hi=min(last,lane);
  if(lo<hi) {uint bits=masks[group*8u+word]>>(lo-first),width=hi-lo;rank+=popcount(width==32u?bits:(bits&((1u<<width)-1u)));}
 }
 uint n=counts[0],target=offsets[item.position].x+rank;
 if(target>=n) { atomic_fetch_or_explicit(status,2u,memory_order_relaxed); return; }
 uint opcode=v.words[item.pg.structural_start]&255u;
 uint requirement=opcode==0x11u?1u:(opcode==0x07u?2u:(opcode==0x08u?3u:0u));
 uint program=v.evaluations[span.evaluation].program;
 states[target]=item.ordinal;
#if SEED_COLUMNS == 1
 // The original task owns its root and Program through immutable selectors.
#elif SEED_COLUMNS == 3
 states[n+target]=item.node;states[2u*n+target]=program;
#else
 states[n+target]=0u;
 states[2u*n+target]=item.node;
 states[3u*n+target]=requirement; states[4u*n+target]=program; states[5u*n+target]=(item.pg.flags&2u)!=0u?0u:2u;
 // Empty value spans. Only the ranked schema also contains an initial ordinal.
 for(uint field=6u;field<SEED_COLUMNS;field++) states[field*n+target]=0u;
#endif
}
