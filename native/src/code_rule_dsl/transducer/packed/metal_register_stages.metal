// GENERATED_REGISTER_FUNCTIONS
template<typename Match> void register_order(View v,Program pg,thread Match* matches,uint count) {
 // Query captures retain insertion order at equal geometry. Non-query labels break ties.
 for(uint i=0;i<count;i++)for(uint j=1;j<matches[i].count;j++){
  uint2 cap=matches[i].caps[j];uint k=j;
  while(k>0u && capture_less(v,cap,matches[i].caps[k-1u],(pg.flags&1u)!=0u)){matches[i].caps[k]=matches[i].caps[k-1u];k--;}
  matches[i].caps[k]=cap;
 }
 if((pg.flags&1u)!=0u)for(uint i=1;i<count;i++){
  auto match=matches[i];uint j=i;
  while(j>0u && ((match.count==0u && matches[j-1u].count!=0u) || (match.count!=0u && matches[j-1u].count!=0u && match.caps[0].x<matches[j-1u].caps[0].x))){matches[j]=matches[j-1u];j--;}
  matches[j]=match;
 }
}
uint4 count_registers(View v,uint signature,Program pg,uint root,uint maximum,bool properties) {
 switch(signature) { // GENERATED_REGISTER_COUNTS
 }
 return uint4(0);
}
uint4 count_registers(View v,uint signature,Program pg,uint root,uint maximum){return count_registers(v,signature,pg,root,maximum,false);}
uint2 constant_output_bound(uint signature){switch(signature){// GENERATED_CONSTANT_BOUNDS
 }return uint2(NONE);}
bool register_owns(View v,Lane item,Params p){return p.output_class<=REGISTER_CONSTANT && item.span!=NONE &&
 (p.output_class==REGISTER_ALL || (p.output_class==REGISTER_CONSTANT)==constant_output_program(v,item.pg));}
void scatter_registers(View v,uint signature,Program pg,uint root,uint maximum,uint ordinal,uint4 base,device Result* results,device Capture* captures) {
 switch(signature) { // GENERATED_REGISTER_SCATTERS
 }
}
uint maximum_matches(View v,Lane item,Params p,uint local,device const uint4* unlimited_offsets,device const uint* progress) {
 if(!p.limited) return NONE;
 uint evaluation=v.spans[item.span].evaluation;
 uint first=max(v.eval_first[evaluation],p.first)-p.first;
 ulong used=ulong(progress[evaluation])+unlimited_offsets[item.position].x-unlimited_offsets[first].x+local;
 uint limit=p.output_class==REGISTER_CONSTANT?v.accepted_limits[v.evaluations[evaluation].program]:v.limits[v.evaluations[evaluation].program];
 return used>=limit?0u:limit-uint(used);
}
// One group is one homogeneous block. The scratch record belongs to a span, never a tuple.
void count_block(device const uchar* data,device const uint* resolved,device const uint* ids,device uint4* counts,device const uint4* unlimited_offsets,device const uint* progress,Params p,uint group,uint lane,threadgroup uint4* values,threadgroup uint* spans) {
 View v=view(data); Block block=v.blocks[ids[group]]; Lane item=lane_input(v,block,lane,resolved,p);
 bool owns=register_owns(v,item,p),valid=p.output_class!=REGISTER_CONSTANT || !owns || constant_root_valid(v,item);
 uint4 count=owns?(valid?count_registers(v,block.signature,item.pg,item.node,NONE,p.output_class==REGISTER_CONSTANT):uint4(0u,0u,0u,1u)):uint4(0u);
 values[lane]=count; spans[lane]=item.span; threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint step=1u;step<256u;step<<=1u) { uint4 add=lane>=step && spans[lane-step]==item.span?values[lane-step]:uint4(0); threadgroup_barrier(mem_flags::mem_threadgroup); values[lane]+=add; threadgroup_barrier(mem_flags::mem_threadgroup); }
 if(p.limited && owns && valid) count=count_registers(v,block.signature,item.pg,item.node,maximum_matches(v,item,p,values[lane].x-count.x,unlimited_offsets,progress),p.output_class==REGISTER_CONSTANT);
 values[lane]=count; threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint step=1u;step<256u;step<<=1u) { uint4 add=lane>=step && spans[lane-step]==item.span?values[lane-step]:uint4(0); threadgroup_barrier(mem_flags::mem_threadgroup); values[lane]+=add; threadgroup_barrier(mem_flags::mem_threadgroup); }
 if(item.span!=NONE && (lane==255u || spans[lane+1u]!=item.span)) counts[item.position]=values[lane];
}
#define COUNT_KERNEL(name) kernel void name(device const uchar* data [[buffer(0)]],device const uint* roots [[buffer(1)]],device const uint* ids [[buffer(2)]],device uint4* counts [[buffer(3)]],device const uint4* unlimited [[buffer(4)]],device const uint* progress [[buffer(5)]],constant Params& p [[buffer(6)]],uint group [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]) { threadgroup uint4 values[256]; threadgroup uint spans[256]; count_block(data,roots,ids,counts,unlimited,progress,p,group,lane,values,spans); }
COUNT_KERNEL(register_direct_count)
COUNT_KERNEL(register_bounded_count)
kernel void register_boundary(device const uint4* totals [[buffer(0)]],device uint4* header [[buffer(1)]],constant Params& p [[buffer(2)]],uint tid [[thread_position_in_grid]]) {
 if(tid) return; uint4 total=totals[0];
 header[0]=total; header[1]=uint4(p.first,p.first+p.count,0u,uint(p.output_class<=REGISTER_SHARED && total.x<=p.result_capacity && total.y<=p.capture_capacity && total.z==0u && total.w==0u));
}
kernel void register_scatter(device const uchar* data [[buffer(0)]],device const uint* resolved [[buffer(1)]],device const uint* ids [[buffer(2)]],device const uint4* offsets [[buffer(3)]],device const uint4* unlimited [[buffer(4)]],device const uint* progress [[buffer(5)]],device Result* results [[buffer(6)]],device Capture* captures [[buffer(7)]],constant Params& p [[buffer(8)]],uint group [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]) {
 View v=view(data); Block block=v.blocks[ids[group]]; Lane item=lane_input(v,block,lane,resolved,p);
 threadgroup uint4 values[256]; threadgroup uint spans[256];
 bool owns=register_owns(v,item,p);uint4 raw=owns?count_registers(v,block.signature,item.pg,item.node,NONE):uint4(0u);
 values[lane]=raw; spans[lane]=item.span; threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint step=1u;step<256u;step<<=1u) { uint4 add=lane>=step && spans[lane-step]==item.span?values[lane-step]:uint4(0); threadgroup_barrier(mem_flags::mem_threadgroup); values[lane]+=add; threadgroup_barrier(mem_flags::mem_threadgroup); }
 uint maximum=item.span==NONE?0u:maximum_matches(v,item,p,values[lane].x-raw.x,unlimited,progress);
 uint4 exact=owns?count_registers(v,block.signature,item.pg,item.node,maximum):uint4(0u);
 values[lane]=exact; threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint step=1u;step<256u;step<<=1u) { uint4 add=lane>=step && spans[lane-step]==item.span?values[lane-step]:uint4(0); threadgroup_barrier(mem_flags::mem_threadgroup); values[lane]+=add; threadgroup_barrier(mem_flags::mem_threadgroup); }
 if(owns) scatter_registers(v,block.signature,item.pg,item.node,maximum,item.ordinal,offsets[item.position]+values[lane]-exact,results,captures);
}
kernel void register_progress(device const uchar* data [[buffer(0)]],device const uint4* offsets [[buffer(1)]],device const uint4* totals [[buffer(2)]],device uint* progress [[buffer(3)]],constant Params& p [[buffer(4)]],uint tid [[thread_position_in_grid]]) {
 if(tid>=p.eval_count) return; View v=view(data); uint evaluation=p.eval_first+tid;
 uint first=max(v.eval_first[evaluation],p.first)-p.first;
 uint next=evaluation+1u<p.eval_first+p.eval_count?v.eval_first[evaluation+1u]:p.first+p.count;
 uint last=min(next,p.first+p.count)-p.first-1u;
 progress[evaluation]+=(last+1u<p.count?offsets[last+1u].x:totals[0].x)-offsets[first].x;
}
