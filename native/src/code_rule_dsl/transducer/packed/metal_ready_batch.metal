struct ReadyParams { uint capacity,output_capacity,tasks,spans,programs,nodes,cursors,repeats,captures,max_kind,semantic_tasks; };
bool ready_lengths(device const uint* h,device const uint* values,constant ReadyParams& p) {
#if INPUT_COLUMNS == 3
 if(h[0]!=p.capacity)return false;
#endif
 return h[0]<=p.capacity && h[1]<=p.captures && h[2]==(h[0]+255u)/256u && h[3]==1u && h[4]==1u && values[0]<=p.cursors && values[1]<=p.repeats;
}
void ready_initialize_item(device const uint* h,device const uint* values,device atomic_uint* tasks,device atomic_uint* status,device uint* output,constant ReadyParams& p,uint tid) {
 if(tid<p.tasks)for(uint k=0u;k<3u;k++)atomic_store_explicit(tasks+tid*3u+k,NONE,memory_order_relaxed);
 if(tid==0u) {
  atomic_store_explicit(status,ready_lengths(h,values,p)?0u:2u,memory_order_relaxed);
  output[16]=0u;for(uint k=0;k<4;k++) { output[k]=0u;output[4u+3u*k]=0u;output[5u+3u*k]=1u;output[6u+3u*k]=1u; }
 }
}
kernel void ready_initialize(device const uint* h [[buffer(0)]],device const uint* values [[buffer(1)]],device atomic_uint* tasks [[buffer(2)]],device atomic_uint* status [[buffer(3)]],device uint* output [[buffer(4)]],constant ReadyParams& p [[buffer(5)]],uint tid [[thread_position_in_grid]]) {
 ready_initialize_item(h,values,tasks,status,output,p,tid);
}
bool ready_state(View v,device const uint* s,device const uint* h,device const uint* values,constant ReadyParams& p,uint i) {
 uint n=h[0],task=s[i],pc=input_scalar(v,s,n,1u,i),node=input_scalar(v,s,n,2u,i),pg=input_scalar(v,s,n,4u,i),kind=input_scalar(v,s,n,5u,i);
 if(task>=p.semantic_tasks || pg>=p.programs || node>=p.nodes || (kind>p.max_kind || (kind>9u && !frame_predicate(kind) && !frame_work_failure(kind))) || input_scalar(v,s,n,3u,i)>3u) return false;
 Program program=v.programs[pg];
 if(pc>=program.structural_count || (frame_query(kind)!=((program.flags&2u)!=0u))) return false;
 if(frame_operand_failure(kind) && !operand_failure_opcode(v.words[program.structural_start+pc]&255u))return false;
 if(frame_predicate(kind)){uint op=v.words[program.structural_start+pc]&255u;if(op!=4u && (op!=6u || (program.flags&1u)))return false;}

 uint low=0u,high=p.spans;
 while(low<high) { uint mid=low+(high-low)/2u;if(v.spans[v.semantic_order[mid]].semantic_start<=task) low=mid+1u;else high=mid; }
 if(low==0u) return false;
 Span span=v.spans[v.semantic_order[low-1u]];Evaluation evaluation=v.evaluations[span.evaluation];File file=v.files[evaluation.file];
 if(task-span.semantic_start>=span.root_count || evaluation.program!=pg || node<file.node_start || node-file.node_start>=file.node_count) return false;
#if INPUT_COLUMNS == 3
 if(!initial_root_matches(v,span,evaluation,task,node))return false;
#endif
 uint3 starts=uint3(input_scalar(v,s,n,6u,i),input_scalar(v,s,n,8u,i),input_scalar(v,s,n,10u,i));uint3 lengths=uint3(input_scalar(v,s,n,7u,i),input_scalar(v,s,n,9u,i),input_scalar(v,s,n,11u,i));uint3 valid=uint3(values[0],values[1],h[1]);
 return all(starts<=valid) && all(lengths<=valid-starts);
}
void ready_classify_item(device const uchar* data,device const uint* s,device const uint* h,device const uint* values,device const uint2* proof,device atomic_uint* tasks,device atomic_uint* status,device atomic_uint* output,constant ReadyParams& p,uint i) {
 if(!ready_lengths(h,values,p) || i>=h[0]) return;
 View v=view(data);
 if(!ready_state(v,s,h,values,p,i)) { atomic_store_explicit(status,2u,memory_order_relaxed);return; }
 uint n=h[0],kind=input_scalar(v,s,n,5u,i);if(frame_predicate(kind)){atomic_fetch_add_explicit(output+16u,1u,memory_order_relaxed);return;}if(frame_finished(kind) || frame_pending(kind)) return;
 uint task=active_task_slot(tasks,3u,p.tasks,s[i]);
 if(task==NONE){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
#if INPUT_COLUMNS == 3
 // A task has one original seed, including when distinct tasks share a root.
 if(atomic_exchange_explicit(tasks+task*3u+2u,input_scalar(v,s,n,2u,i),memory_order_relaxed)!=NONE){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
#endif
 uint rank=proof[v.programs[input_scalar(v,s,n,4u,i)].structural_start+input_scalar(v,s,n,1u,i)].x;
 uint selected=kind>=4u || rank==NONE?0u:rank+1u;
 if(selected<atomic_load_explicit(tasks+task*3u+1u,memory_order_relaxed))atomic_fetch_min_explicit(tasks+task*3u+1u,selected,memory_order_relaxed);
}
kernel void ready_classify(device const uchar* data [[buffer(0)]],device const uint* s [[buffer(1)]],device const uint* h [[buffer(2)]],device const uint* values [[buffer(3)]],device const uint2* proof [[buffer(4)]],device atomic_uint* tasks [[buffer(5)]],device atomic_uint* status [[buffer(6)]],device atomic_uint* output [[buffer(7)]],constant ReadyParams& p [[buffer(8)]],uint i [[thread_position_in_grid]]) {
 ready_classify_item(data,s,h,values,proof,tasks,status,output,p,i);
}
kernel void ready_nodes(device const uchar* data [[buffer(0)]],device const uint* s [[buffer(1)]],device const uint* h [[buffer(2)]],device const uint2* proof [[buffer(3)]],device atomic_uint* tasks [[buffer(4)]],device atomic_uint* status [[buffer(5)]],constant ReadyParams& p [[buffer(6)]],uint i [[thread_position_in_grid]]) {
 if(atomic_load_explicit(status,memory_order_relaxed)!=0u || i>=h[0]) return;
 View v=view(data);uint n=h[0],kind=input_scalar(v,s,n,5u,i);if(kind!=0u) return;uint task=find_active_task_slot((device const uint*)tasks,3u,p.tasks,s[i]),rank=proof[v.programs[input_scalar(v,s,n,4u,i)].structural_start+input_scalar(v,s,n,1u,i)].x;
 if(task==NONE){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 if(rank!=NONE && rank+1u==atomic_load_explicit(tasks+task*3u+1u,memory_order_relaxed) && input_scalar(v,s,n,2u,i)<atomic_load_explicit(tasks+task*3u+2u,memory_order_relaxed))atomic_fetch_min_explicit(tasks+task*3u+2u,input_scalar(v,s,n,2u,i),memory_order_relaxed);
}
uint ready_choice(device const uchar* data,device const uint* s,device const uint* h,device const uint2* proof,device const uint* tasks,device atomic_uint* status,constant ReadyParams& p,uint i) {
 View v=view(data);uint n=h[0],kind=input_scalar(v,s,n,5u,i),choice=0u;uint2 region=proof[v.programs[input_scalar(v,s,n,4u,i)].structural_start+input_scalar(v,s,n,1u,i)];
 if(frame_finished(kind)) choice=3u;
 else if(frame_pending(kind) || frame_predicate(kind)) choice=2u;
 else if(kind==0u && region.x!=NONE) {
  uint task=find_active_task_slot(tasks,3u,p.tasks,s[i]);
  if(task==NONE){atomic_store_explicit(status,2u,memory_order_relaxed);return 2u;}
  bool ready=tasks[task*3u+1u]==region.x+1u;
  choice=ready?(region.y==1u?1u:(region.y==2u || input_scalar(v,s,n,2u,i)==tasks[task*3u+2u]?0u:2u)):2u;
 }
 return choice;
}
// Each lane selects at most 256 consecutive rows. The cooperative scan preserves
// all four stable lists. Larger frontiers retain the same hierarchical operator.
kernel void ready_select_small(device const uchar* data [[buffer(0)]],device const uint* s [[buffer(1)]],device const uint* h [[buffer(2)]],device const uint2* proof [[buffer(3)]],device const uint* tasks [[buffer(4)]],device atomic_uint* status [[buffer(5)]],device uint* output [[buffer(6)]],device uint* counts [[buffer(7)]],constant ReadyParams& p [[buffer(8)]],uint lane [[thread_index_in_threadgroup]]) {
 threadgroup uint4 prefix[256];threadgroup uint invalid;
 if(!lane)invalid=uint(atomic_load_explicit(status,memory_order_relaxed)!=0u || p.capacity>65536u);
 threadgroup_barrier(mem_flags::mem_threadgroup);if(invalid)return;
 uint width=(p.capacity+255u)/256u,first=lane*width,last=min(first+width,h[0]);uint4 local=uint4(0u);
 for(uint i=first;i<last;i++)local[ready_choice(data,s,h,proof,tasks,status,p,i)]++;
 prefix[lane]=local;threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint step=1u;step<256u;step*=2u){uint4 add=lane>=step?prefix[lane-step]:uint4(0u);
  threadgroup_barrier(mem_flags::mem_threadgroup);prefix[lane]=uint4(min(ulong4(prefix[lane])+ulong4(add),ulong4(NONE)));
  threadgroup_barrier(mem_flags::mem_threadgroup);}
 uint4 total=prefix[255u];
 if(any(total>uint4(p.output_capacity))
#if INPUT_COLUMNS == 3
 || ulong(total.x)+total.y+total.z+total.w>p.output_capacity
#endif
 ){if(!lane)atomic_store_explicit(status,1u,memory_order_relaxed);return;}
 if(!lane)for(uint k=0u;k<4u;k++){uint n=total[k];counts[k]=n;counts[4u+3u*k]=(n+255u)/256u;}
 uint4 at=lane?prefix[lane-1u]:uint4(0u);
 for(uint i=first;i<last;i++){uint choice=ready_choice(data,s,h,proof,tasks,status,p,i);output[input_list_base(total,p.output_capacity,choice)+at[choice]++]=i;}
}
kernel void ready_select_cooperative(device const uchar* data [[buffer(0)]],device const uint* s [[buffer(1)]],device const uint* h [[buffer(2)]],device const uint2* proof [[buffer(3)]],device const uint* tasks [[buffer(4)]],device atomic_uint* status [[buffer(5)]],device uint* output [[buffer(6)]],device uint* counts [[buffer(7)]],constant ReadyParams& p [[buffer(8)]],uint lane [[thread_index_in_threadgroup]]) {
 threadgroup uint4 prefix[256];threadgroup uint invalid;
 if(!lane)invalid=uint(atomic_load_explicit(status,memory_order_relaxed)!=0u || p.capacity>8388608u);
 threadgroup_barrier(mem_flags::mem_threadgroup);if(invalid)return;
 uint width=(p.capacity+255u)/256u,first=lane*width,last=min(first+width,h[0]);uint4 local=uint4(0u);
 for(uint i=first;i<last;i++)local[ready_choice(data,s,h,proof,tasks,status,p,i)]++;
 prefix[lane]=local;threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint step=1u;step<256u;step*=2u){uint4 add=lane>=step?prefix[lane-step]:uint4(0u);
  threadgroup_barrier(mem_flags::mem_threadgroup);prefix[lane]=uint4(min(ulong4(prefix[lane])+ulong4(add),ulong4(NONE)));
  threadgroup_barrier(mem_flags::mem_threadgroup);}
 uint4 total=prefix[255u];
 if(any(total>uint4(p.output_capacity))
#if INPUT_COLUMNS == 3
 || ulong(total.x)+total.y+total.z+total.w>p.output_capacity
#endif
 ){if(!lane)atomic_store_explicit(status,1u,memory_order_relaxed);return;}
 if(!lane)for(uint k=0u;k<4u;k++){uint n=total[k];counts[k]=n;counts[4u+3u*k]=(n+255u)/256u;}
 uint4 at=lane?prefix[lane-1u]:uint4(0u);
 for(uint i=first;i<last;i++){uint choice=ready_choice(data,s,h,proof,tasks,status,p,i);output[input_list_base(total,p.output_capacity,choice)+at[choice]++]=i;}
}
// A block contains at most 65,536 rows. Only block totals enter the global scan.
// Recomputing four choices retires the former per-row uint4 and kind arenas.
kernel void ready_count_blocks(device const uchar* data [[buffer(0)]],device const uint* s [[buffer(1)]],device const uint* h [[buffer(2)]],device const uint2* proof [[buffer(3)]],device const uint* tasks [[buffer(4)]],device atomic_uint* status [[buffer(5)]],device uint4* counts [[buffer(6)]],constant ReadyParams& p [[buffer(7)]],uint group [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]) {
 threadgroup uint4 partial[256];uint first=group*65536u+lane*256u,last=min(first+256u,h[0]);uint4 local=uint4(0u);
 if(!atomic_load_explicit(status,memory_order_relaxed))for(uint i=first;i<last;i++)local[ready_choice(data,s,h,proof,tasks,status,p,i)]++;
 partial[lane]=local;threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint step=128u;step;step/=2u){if(lane<step)partial[lane]+=partial[lane+step];threadgroup_barrier(mem_flags::mem_threadgroup);}
 if(!lane)counts[group]=partial[0];
}
kernel void ready_scatter_blocks(device const uchar* data [[buffer(0)]],device const uint* s [[buffer(1)]],device const uint* h [[buffer(2)]],device const uint2* proof [[buffer(3)]],device const uint* tasks [[buffer(4)]],device const uint4* offsets [[buffer(5)]],device atomic_uint* status [[buffer(6)]],device uint* output [[buffer(7)]],device const uint4* total [[buffer(8)]],device uint* counts [[buffer(9)]],constant ReadyParams& p [[buffer(10)]],uint group [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]) {
 if(atomic_load_explicit(status,memory_order_relaxed))return;
 if(any(total[0]>uint4(p.output_capacity))
#if INPUT_COLUMNS == 3
 || ulong(total[0].x)+total[0].y+total[0].z+total[0].w>p.output_capacity
#endif
 ){if(!group && !lane)atomic_store_explicit(status,1u,memory_order_relaxed);return;}
 if(!group && !lane)for(uint k=0u;k<4u;k++){uint n=total[0][k];counts[k]=n;counts[4u+3u*k]=(n+255u)/256u;}
 threadgroup uint4 prefix[256];uint first=group*65536u+lane*256u,last=min(first+256u,h[0]);uint4 local=uint4(0u);
 for(uint i=first;i<last;i++)local[ready_choice(data,s,h,proof,tasks,status,p,i)]++;
 prefix[lane]=local;threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint step=1u;step<256u;step*=2u){uint4 add=lane>=step?prefix[lane-step]:uint4(0u);
  threadgroup_barrier(mem_flags::mem_threadgroup);prefix[lane]+=add;threadgroup_barrier(mem_flags::mem_threadgroup);}
 uint4 at=offsets[group]+(lane?prefix[lane-1u]:uint4(0u));
 for(uint i=first;i<last;i++){uint choice=ready_choice(data,s,h,proof,tasks,status,p,i);output[input_list_base(total[0],p.output_capacity,choice)+at[choice]++]=i;}
}
