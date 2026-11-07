#ifdef SUCCESSOR_REGISTER_FRAMES
// Virtual finite jobs use private frames until completed counts authorize storage.
uint4 successor_register_job(device const uchar* data,device const uint* s,device const uint* h,device const uint4* cursors,device const uint4* repeats,device const uint4* slots,device const uint4* offsets,device const uint* spans,device const uint4* sites,device const uint* path_work,constant SuccessorParams& p,uint job,device atomic_uint* status,thread Step& x) {
 if(job>=p.input_jobs || atomic_load_explicit(status,memory_order_relaxed)!=0u)return uint4(0u);
 uint i=owner(offsets,h[0],job,1u);
 if(i>=h[0] || job<offsets[i].y || job-offsets[i].y>=slots[i].z || slots[i].x>p.signatures){atomic_store_explicit(status,2u,memory_order_relaxed);return uint4(0u);}
 uint signature=slots[i].x;View v=view(data);Program pg=v.programs[input_scalar(v,s,h[0],4u,i)];
 x=Step{};x.logical_step=job;
 return successor_frame(v,pg,s,h[0],cursors,repeats,spans,sites,path_work,p,i,job-offsets[i].y,signature,status,x);
}
uint successor_register_words(thread const Step& x) {
 return successor_step_words(uint4(x.cursor_add,x.repeat_add,x.capture_add,x.branch_count));
}
uint2 successor_register_prefix(uint2 value,threadgroup uint2* sums,uint lane) {
 sums[lane]=value;threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint d=1u;d<256u;d*=2u){uint2 before=lane>=d?sums[lane-d]:uint2(0u);threadgroup_barrier(mem_flags::mem_threadgroup);sums[lane]+=before;threadgroup_barrier(mem_flags::mem_threadgroup);}
 return lane?sums[lane-1u]:uint2(0u);
}
kernel void successor_register_counts(device const uchar* data [[buffer(0)]],device const uint* s [[buffer(1)]],device const uint* h [[buffer(2)]],device const uint4* cursors [[buffer(3)]],device const uint4* repeats [[buffer(4)]],device const uint4* slots [[buffer(5)]],device const uint4* offsets [[buffer(6)]],device const uint* spans [[buffer(7)]],device const uint4* sites [[buffer(8)]],device const uint* path_work [[buffer(9)]],device atomic_uint* status [[buffer(10)]],device uint4* counts [[buffer(11)]],constant SuccessorParams& p [[buffer(12)]],uint group [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]) {
 alignas(16) uint words[SUCCESSOR_PRIVATE_WORDS];thread Step& x=step_at(words,0u);
 uint4 output=successor_register_job(data,s,h,cursors,repeats,slots,offsets,spans,sites,path_work,p,group*256u+lane,status,x);
 threadgroup uint2 sums[256];(void)successor_register_prefix(output.x?uint2(1u,successor_register_words(x)):uint2(0u),sums,lane);
 if(!lane)counts[group]=uint4(sums[255],0u,0u);
}
kernel void successor_register_scatter(device const uchar* data [[buffer(0)]],device const uint* s [[buffer(1)]],device const uint* h [[buffer(2)]],device const uint4* cursors [[buffer(3)]],device const uint4* repeats [[buffer(4)]],device const uint4* slots [[buffer(5)]],device const uint4* offsets [[buffer(6)]],device const uint* spans [[buffer(7)]],device const uint4* sites [[buffer(8)]],device const uint* path_work [[buffer(9)]],device atomic_uint* status [[buffer(10)]],device const uint4* blocks [[buffer(11)]],device uint* steps [[buffer(12)]],device uint4* expansion [[buffer(13)]],constant SuccessorParams& p [[buffer(14)]],uint group [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]) {
 alignas(16) uint words[SUCCESSOR_PRIVATE_WORDS];thread Step& x=step_at(words,0u);
 uint4 output=successor_register_job(data,s,h,cursors,repeats,slots,offsets,spans,sites,path_work,p,group*256u+lane,status,x);
 threadgroup uint2 sums[256];uint2 local=successor_register_prefix(output.x?uint2(1u,successor_register_words(x)):uint2(0u),sums,lane);
 if(!output.x || atomic_load_explicit(status,memory_order_relaxed)!=0u)return;
 uint step=blocks[group].x+local.x,word=blocks[group].y+local.y,length=successor_register_words(x);
 if(step>=p.steps || word>p.step_words || length>p.step_words-word){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 device Step& target=step_at(steps,word);target=x;target.logical_step=step;
 target.repeat_offset=SUCCESSOR_HEADER_WORDS+2u*x.cursor_add;
 target.capture_offset=(target.repeat_offset+x.repeat_add+3u)&~3u;target.branch_offset=target.capture_offset+4u*x.capture_add;
 for(uint k=0u;k<x.cursor_add;k++)step_cursors(target)[k]=step_cursors(x)[k];
 for(uint k=0u;k<x.repeat_add;k++)step_repeats(target)[k]=step_repeats(x)[k];
 for(uint k=0u;k<x.capture_add;k++)step_captures(target)[k]=step_captures(x)[k];
 for(uint k=0u;k<x.branch_count;k++)step_branches(target)[k]=step_branches(x)[k];
 output.z=length;expansion[step]=output;
}

#endif
