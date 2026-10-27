// SUCCESSOR_PREFIXES
bool successor_header(device const uint* h,device const uint* lengths,device const uint* control,constant SuccessorParams& p) {
#if INPUT_COLUMNS == 3
 if(h[0]!=p.capacity)return false;
#endif
 return h[0]<=p.capacity && h[1]<=p.captures && h[2]==uint((ulong(h[0])+255ul)/256ul) && h[3]==1u && h[4]==1u
 && lengths[0]<=p.cursors && lengths[1]<=p.repeats && control[0]<=p.digits;
}
bool successor_lists_header(device const uint* h,device const uint* lengths,device const uint* control,device const uint* ready,constant SuccessorParams& p) {
 bool valid=successor_header(h,lengths,control,p);ulong total=0ul;
 for(uint k=0u;k<4u;k++) {uint n=ready[k];total+=n;valid &= n<=p.capacity && ready[4u+3u*k]==uint((ulong(n)+255ul)/256ul) && ready[5u+3u*k]==1u && ready[6u+3u*k]==1u;}
 return valid && total==h[0] && ready[16]<=ready[2];
}
kernel void successor_input_arguments(device const uint4* groups [[buffer(0)]],device uint* arguments [[buffer(1)]],device atomic_uint* status [[buffer(2)]],device const uint4* jobs [[buffer(3)]],uint tid [[thread_position_in_grid]]) {
 if(tid)return;bool valid=atomic_load_explicit(status,memory_order_relaxed)==0u && groups[0].x!=NONE && jobs[0].x==jobs[0].y;
 if(jobs[0].x!=jobs[0].y)atomic_store_explicit(status,2u,memory_order_relaxed);
 if(groups[0].x==NONE)atomic_store_explicit(status,1u,memory_order_relaxed);
 arguments[0]=valid?groups[0].x:0u;arguments[1]=arguments[2]=1u;
}
kernel void successor_lists(device const uint* h [[buffer(0)]],device const uint* ids [[buffer(1)]],device const uint* ready [[buffer(2)]],device atomic_uint* categories [[buffer(3)]],device atomic_uint* status [[buffer(4)]],device const uint* lengths [[buffer(5)]],device const uint* control [[buffer(6)]],constant SuccessorParams& p [[buffer(7)]],uint tid [[thread_position_in_grid]],uint lane [[thread_index_in_threadgroup]]) {
 threadgroup bool header_valid[1];
 if(!lane)header_valid[0]=successor_lists_header(h,lengths,control,ready,p);
 threadgroup_barrier(mem_flags::mem_threadgroup);
 if(!header_valid[0]){if(!lane)atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 if(atomic_load_explicit(status,memory_order_relaxed)!=0u)return;
 if(!p.capacity)return;
 uint k,j,index;
#if INPUT_COLUMNS == 3
 if(tid>=h[0])return;k=0u;j=tid;while(k<4u && j>=ready[k])j-=ready[k++];
 if(k>=4u){atomic_store_explicit(status,2u,memory_order_relaxed);return;}index=tid;
#else
 k=tid/p.capacity;j=tid%p.capacity;if(k>=4u || j>=ready[k])return;index=k*p.capacity+j;
#endif
 uint i=ids[index];if(i>=h[0]) {atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 if(atomic_fetch_add_explicit(categories+i,1u+4u*k,memory_order_relaxed)!=0u)atomic_store_explicit(status,2u,memory_order_relaxed);
}
uint4 successor_input_row(device const uchar* data,device const uint* s,device const uint* h,device const uint* lengths,device const uint* spans,device const uint* count,device const uint* categories,device const uint4* sites,device const uint* path_work,constant SuccessorParams& p,uint i) {
 View v=view(data);uint n=h[0],task=s[i],pc=input_scalar(v,s,n,1u,i),node=input_scalar(v,s,n,2u,i),pg=input_scalar(v,s,n,4u,i),kind=input_scalar(v,s,n,5u,i),cat=categories[i];
 bool valid=task<p.tasks && pg<p.programs && node<p.nodes && (kind<=9u || frame_predicate(kind) || frame_work_failure(kind)) && cat>=1u && cat<=13u && cat%4u==1u;
 if(!valid) {return uint4(0u);}
 Program program=v.programs[pg];valid=pc<program.structural_count && (frame_query(kind)==((program.flags&2u)!=0u));
 if(valid)valid &= input_scalar(v,s,n,3u,i)==successor_requirement(v,program,pc);
 if(kind==1u || kind==3u)valid &= cat==13u && (v.words[program.structural_start+min(pc,program.structural_count-1u)]&255u)==64u;
 if(frame_operand_failure(kind))valid &= cat==13u && operand_failure_opcode(v.words[program.structural_start+min(pc,program.structural_count-1u)]&255u);
 if(frame_pending(kind))valid &= cat==9u;
 if(kind==6u && valid)valid &= sites[program.structural_start+pc].w==1u;
 valid &= input_work(path_work,i) <= PATH_WORK_LIMIT+uint(frame_work_failure(kind));
 if(frame_predicate(kind)){uint op=v.words[program.structural_start+min(pc,program.structural_count-1u)]&255u;valid &= cat==9u && (op==4u || (op==6u && !(program.flags&1u)));}
 if(frame_work_failure(kind))valid &= cat==13u && input_work(path_work,i)==PATH_WORK_LIMIT+1u;

 uint lo=0u,hi=p.spans;while(lo<hi) {uint mid=lo+(hi-lo)/2u;if(v.spans[v.semantic_order[mid]].semantic_start<=task)lo=mid+1u;else hi=mid;}
 if(!lo)valid=false;
 else {Span span=v.spans[v.semantic_order[lo-1u]];Evaluation e=v.evaluations[span.evaluation];File file=v.files[e.file];valid &= task-span.semantic_start<span.root_count && e.program==pg && node>=file.node_start && node-file.node_start<file.node_count;
#if INPUT_COLUMNS == 3
  if(valid)valid &= initial_root_matches(v,span,e,task,node);
#endif
 }
 uint width=1u;
 for(uint k=0u;k<3u;k++) {
  uint start=input_scalar(v,s,n,6u+2u*k,i),length=input_scalar(v,s,n,7u+2u*k,i),available=k==2u?h[1]:lengths[k];
  valid &= start<=available && length<=available-min(start,available);width=max(width,length);
  start=input_control(spans,n,2u*k,i);length=input_control(spans,n,2u*k+1u,i);available=count[0];
  valid &= start<=available && length<=available-min(start,available);width=max(width,length);
 }
 if(!frame_pending(kind))valid &= input_control(spans,n,3u,i)==0u && input_control(spans,n,5u,i)==0u;
 else valid &= input_control(spans,n,1u,i)>0u && input_control(spans,n,1u,i)==input_control(spans,n,3u,i);
 if(!valid) {return uint4(0u);}
 uint signature=cat==1u?sites[program.structural_start+pc].x:p.signatures;
 if(signature>p.signatures || (cat==1u && (frame_finished(kind) || frame_pending(kind)))) {return uint4(0u);}
 uint variants=cat==1u?sites[program.structural_start+pc].z:1u;
 if(!variants || variants>64u){return uint4(0u);}
 return uint4(signature,variants,width,1u);
}
bool successor_input_cell(device const uchar* data,device const uint* s,device const uint* h,device const uint4* cursors,device const uint4* repeats,device const uint4* captures,device const uint* spans,device const uint* digits,constant SuccessorParams& p,uint i,uint entry) {
 uint n=h[0];View v=view(data);bool valid=true;
 // Checked input metadata validates task membership. Typed node references
 // must belong to that same file, even when another file has equal geometry.
 uint low=0u,high=p.spans;
 while(low<high) {uint mid=low+(high-low)/2u;if(v.spans[v.semantic_order[mid]].semantic_start<=s[i])low=mid+1u;else high=mid;}
 File file=v.files[v.evaluations[v.spans[v.semantic_order[low-1u]].evaluation].file];
 if(entry==0u && frame_operand_failure(input_scalar(v,s,n,5u,i))) {
  Program pg=v.programs[input_scalar(v,s,n,4u,i)];uint w=v.words[pg.structural_start+input_scalar(v,s,n,1u,i)],op=w&255u,arg=w>>16;
  if(op==0x16u) {
   uint start=input_scalar(v,s,n,6u,i),count=input_scalar(v,s,n,7u,i),at=cursor_lower(cursors,start,count,arg);
   valid &= at==count || cursors[start+at].x!=arg;
  } else {
   uint count=input_scalar(v,s,n,9u,i);valid &= count==0u || repeats[input_scalar(v,s,n,8u,i)+count-1u].y!=arg;
  }
 }
 for(uint k=0u;k<3u;k++) {
  uint start=input_scalar(v,s,n,6u+2u*k,i),length=input_scalar(v,s,n,7u+2u*k,i);if(entry>=length)continue;
  device const uint4* arena=k==0u?cursors:(k==1u?repeats:captures);uint4 value=arena[start+entry];
  if(k==0u)valid &= value.x<=65535u && value.y>=file.node_start && value.y-file.node_start<file.node_count && value.z==0u && value.w==0u && (entry==0u || arena[start+entry-1u].x<value.x);
  else if(k==1u)valid &= value.x==entry && value.y<=65535u && value.z==0u && value.w==0u;
  else {
   Program pg=v.programs[input_scalar(v,s,n,4u,i)];
   valid &= value.x>=file.node_start && value.x-file.node_start<file.node_count && value.y<pg.label_count && value.z<=value.w;
   if(entry && ((pg.flags&1u) || !pg.capture_count)) {
    uint4 previous=arena[start+entry-1u];valid &= !geo_less(value,previous);
    // Ordinary producers sort equal geometry by label. Query ties retain order.
    if(!(pg.flags&1u) && value.z==previous.z && value.w==previous.w)valid &= previous.y<=value.y;
   }
   if(value.x<p.nodes)valid &= value.z==v.nodes[value.x].source_start && value.w==v.nodes[value.x].source_end;
  }
 }
 uint kind=input_scalar(v,s,n,5u,i);
 if(frame_pending(kind)) {
  uint start=input_control(spans,n,0u,i),length=input_control(spans,n,1u,i),body=input_control(spans,n,2u,i),offer=input_control(spans,n,4u,i),offern=input_control(spans,n,5u,i);
  if(entry<length)valid &= entry+1u==length?(digits[start+entry]==1u && digits[body+entry]==0u):digits[start+entry]==digits[body+entry];
  bool initial=offern+1u==length;valid &= initial || offern>=length;
  if(entry<min(offern,length))valid &= digits[offer+entry]==digits[body+entry];
 }
 return valid;
}
// One group checks metadata, counts its signature jobs, and validates every
// typed cell. No consumer reads the histogram before this dispatch completes.
kernel void successor_inputs_checked(device const uchar* data [[buffer(0)]],device const uint* s [[buffer(1)]],device const uint* h [[buffer(2)]],device const uint* lengths [[buffer(3)]],device const uint* spans [[buffer(4)]],device const uint* count [[buffer(5)]],device const uint* categories [[buffer(6)]],device const uint4* sites [[buffer(7)]],device atomic_uint* histogram [[buffer(8)]],device uint4* slots [[buffer(9)]],device atomic_uint* status [[buffer(10)]],device const uint* path_work [[buffer(11)]],device const uint4* cursors [[buffer(12)]],device const uint4* repeats [[buffer(13)]],device const uint4* captures [[buffer(14)]],device const uint* digits [[buffer(15)]],constant SuccessorParams& p [[buffer(16)]],uint i [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]) {
 threadgroup uint width;
 if(!lane){
  width=0u;
  if(i<p.capacity){
   slots[i]=uint4(0u);
   if(i<h[0] && atomic_load_explicit(status,memory_order_relaxed)==0u){
    uint4 row=successor_input_row(data,s,h,lengths,spans,count,categories,sites,path_work,p,i);
    if(!row.w)atomic_store_explicit(status,2u,memory_order_relaxed);
    else{
     atomic_store_explicit(histogram+i*4u+1u,row.y,memory_order_relaxed);
     uint words=successor_step_words(successor_layout(row.x));
     atomic_store_explicit(histogram+i*4u+2u,uint(min(ulong(words)*row.y,0xfffffffful)),memory_order_relaxed);
     slots[i]=uint4(row.x,atomic_fetch_add_explicit(histogram+row.x*4u,row.y,memory_order_relaxed),row.y,row.z);
     width=row.z;
    }
   }
  }
 }
 // The shared width makes every lane take the same checked cell domain even
 // when another group reports a failure during this group's execution.
 threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint entry=lane;entry<width;entry+=256u)
  if(!successor_input_cell(data,s,h,cursors,repeats,captures,spans,digits,p,i,entry))
   atomic_store_explicit(status,2u,memory_order_relaxed);
}
kernel void successor_validate_frontier(device const uchar* data [[buffer(0)]],device const uint* s [[buffer(1)]],device const uint* h [[buffer(2)]],device const uint* lengths [[buffer(3)]],device const uint* spans [[buffer(4)]],device const uint* count [[buffer(5)]],device const uint* categories [[buffer(6)]],device const uint4* sites [[buffer(7)]],device const uint4* cursors [[buffer(8)]],device const uint4* repeats [[buffer(9)]],device const uint4* captures [[buffer(10)]],device const uint* digits [[buffer(11)]],device atomic_uint* status [[buffer(12)]],device const uint* path_work [[buffer(13)]],constant SuccessorParams& p [[buffer(14)]],uint i [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]) {
 threadgroup uint width[1];
 if(lane==0u){
  width[0]=0u;
  if(i<h[0] && atomic_load_explicit(status,memory_order_relaxed)==0u){
   uint4 row=successor_input_row(data,s,h,lengths,spans,count,categories,sites,path_work,p,i);
   if(!row.w)atomic_store_explicit(status,2u,memory_order_relaxed);else width[0]=row.z;
  }
 }
 threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint entry=lane;entry<width[0];entry+=256u)
  if(!successor_input_cell(data,s,h,cursors,repeats,captures,spans,digits,p,i,entry))atomic_store_explicit(status,2u,memory_order_relaxed);
}
// The immutable signature domain has at most 4096 rows here. Each lane
// counts <=16 consecutive buckets; one cooperative prefix also builds arguments.
kernel void successor_groups(device const uint4* offsets [[buffer(0)]],device const uint4* jobs [[buffer(1)]],
 device uint4* groups [[buffer(2)]],device uint4* total [[buffer(3)]],device uint* arguments [[buffer(4)]],
 device atomic_uint* status [[buffer(5)]],constant SuccessorParams& p [[buffer(6)]],uint lane [[thread_index_in_threadgroup]]) {
 threadgroup uint sums[256];uint n=p.signatures+1u,width=(n+255u)/256u,first=lane*width,last=min(first+width,n),local=0u;
 bool valid=n<=4096u && atomic_load_explicit(status,memory_order_relaxed)==0u && jobs[0].x!=NONE && jobs[0].x==jobs[0].y;
 for(uint i=first;i<last;i++){uint end=i+1u<n?offsets[i+1u].x:jobs[0].x;local+=valid?uint((ulong(end-offsets[i].x)+255ul)/256ul):0u;}
 sums[lane]=local;threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint distance=1u;distance<256u;distance*=2u){uint value=lane>=distance?sums[lane-distance]:0u;
  threadgroup_barrier(mem_flags::mem_threadgroup);sums[lane]+=value;threadgroup_barrier(mem_flags::mem_threadgroup);}
 uint at=lane?sums[lane-1u]:0u;
 for(uint i=first;i<last;i++){groups[i]=uint4(at,0u,0u,0u);uint end=i+1u<n?offsets[i+1u].x:jobs[0].x;
  at+=valid?uint((ulong(end-offsets[i].x)+255ul)/256ul):0u;}
 if(!lane){total[0]=uint4(sums[255],0u,0u,0u);arguments[0]=sums[255];arguments[1]=arguments[2]=1u;
  if(jobs[0].x==NONE)atomic_store_explicit(status,1u,memory_order_relaxed);
  else if(jobs[0].x!=jobs[0].y || n>4096u)atomic_store_explicit(status,2u,memory_order_relaxed);}
}
kernel void successor_group_counts(device const uint4* offsets [[buffer(0)]],device const uint4* total [[buffer(1)]],device uint4* groups [[buffer(2)]],constant SuccessorParams& p [[buffer(3)]],uint i [[thread_position_in_grid]]) {
 if(i>p.signatures)return;uint count=(i<p.signatures?offsets[i+1u].x:total[0].x)-offsets[i].x;groups[i]=uint4(uint((ulong(count)+255ul)/256ul),0u,0u,0u);
}
kernel void successor_group_items(device const uint* h [[buffer(0)]],device uint4* slots [[buffer(1)]],device const uint4* offsets [[buffer(2)]],device uint2* items [[buffer(3)]],device atomic_uint* status [[buffer(4)]],device uint4* groups [[buffer(5)]],constant SuccessorParams& p [[buffer(6)]],uint i [[thread_position_in_grid]]) {
 if(atomic_load_explicit(status,memory_order_relaxed)!=0u)return;
 if(i<=p.signatures)groups[i].y=offsets[i].x;
 if(i>=h[0])return;uint4 slot=slots[i];
 if(slot.x>p.signatures || offsets[slot.x].x>p.steps || slot.y>p.steps-offsets[slot.x].x || slot.z>p.steps-offsets[slot.x].x-slot.y){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 for(uint variant=0u;variant<slot.z;variant++)items[offsets[slot.x].x+slot.y+variant]=uint2(i,variant);
 // Validation consumed width. The completed group now owns logical and word bases.
 slots[i].y=offsets[i].y;slots[i].w=offsets[i].z;
}
#ifndef SUCCESSOR_REGISTER_FRAMES
kernel void successor_prefix(device const uchar* data [[buffer(0)]],device const uint* s [[buffer(1)]],device const uint* h [[buffer(2)]],device const uint4* cursors [[buffer(3)]],device const uint4* repeats [[buffer(4)]],device const uint4* slots [[buffer(5)]],device const uint4* groups [[buffer(6)]],device const uint4* group_total [[buffer(7)]],device const uint2* items [[buffer(8)]],device uint* steps [[buffer(9)]],device uint4* counts [[buffer(10)]],device atomic_uint* status [[buffer(11)]],device const uint* spans [[buffer(12)]],device const uint4* sites [[buffer(13)]],device const uint* path_work [[buffer(14)]],constant SuccessorParams& p [[buffer(15)]],uint group [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]) {
 if(atomic_load_explicit(status,memory_order_relaxed)!=0u || group>=group_total[0].x)return;
 uint signature=owner(groups,p.signatures+1u,group,0u),at=groups[signature].y+(group-groups[signature].x)*256u+lane;
 uint end=signature<p.signatures?groups[signature+1u].y:p.steps;if(at>=end)return;
 uint i=items[at].x,variant=items[at].y,n=h[0];View v=view(data);Program pg=v.programs[input_scalar(v,s,n,4u,i)];
 // Signature grouping changes execution order, not the stable input/variant slots.
 uint step=slots[i].y+variant;
 if(step>=p.steps){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint4 layout=successor_layout(signature);uint frame_words=successor_step_words(layout);
 uint word=slots[i].w+variant*frame_words;
 if(word>p.step_words || frame_words>p.step_words-word){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 device Step& x=step_at(steps,word);x=Step{};x.logical_step=step;
 counts[step]=successor_frame(v,pg,s,n,cursors,repeats,spans,sites,path_work,p,i,variant,signature,status,x);
}
#endif
// Frame addresses are word offsets. Logical step indices remain in the header.
struct Candidate {uint frame_word,choice,node,pc,kind,append,offer,live,task_slot;};
static_assert(sizeof(Candidate)==9u*4u,"candidate task-group layout");
kernel void successor_candidates(device const uchar* data [[buffer(0)]],device const uint* s [[buffer(1)]],device const uint* h [[buffer(2)]],device const uint* spans [[buffer(3)]],device const uint* steps [[buffer(4)]],device const uint4* offsets [[buffer(5)]],device const uint4* total [[buffer(6)]],device Candidate* candidates [[buffer(7)]],device uint4* sizes [[buffer(8)]],device uint4* selection [[buffer(9)]],device atomic_uint* tasks [[buffer(10)]],device atomic_uint* status [[buffer(11)]],device atomic_uint* task_keys [[buffer(12)]],constant SuccessorParams& p [[buffer(13)]],uint i [[thread_position_in_grid]]) {
 if(total[0].x>p.output) {if(i==0u)atomic_store_explicit(status,1u,memory_order_relaxed);return;}
 if(i>=p.output)return;sizes[i]=uint4(0u);selection[i].y=0u;candidates[i]={};
 if(atomic_load_explicit(status,memory_order_relaxed)!=0u || atomic_load_explicit(status+1u,memory_order_relaxed)!=NONE || atomic_load_explicit(status+2u,memory_order_relaxed)!=NONE || i>=total[0].x)return;
 uint n=h[0],step=owner(offsets,p.steps,i,0u),choice=i-offsets[step].x;device const Step& x=step_at(steps,offsets[step].z);uint parent=x.input_parent;View v=view(data);
 Candidate c={offsets[step].z,choice,x.node,x.pc,x.kind,uint(x.expansion!=0u),uint(x.expansion==2u && choice==1u),1u,NONE};
 if(x.expansion<=2u && x.expansion!=0u && choice==1u) {c.pc=x.begin;if(c.offer)c.kind=x.kind==0u?6u:7u;}
 if(x.expansion>=3u) {c.node=x.expansion==4u?x.begin+choice:v.children[x.begin+choice];c.live=uint(!x.named || (v.nodes[c.node].flags&1u)!=0u);}
 candidates[i]=c;if(!c.live)return;
 if(!p.task_slots && frame_pending(c.kind)){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint digits=input_control(spans,n,1u,parent)+x.branch_count+c.append;
 if(c.offer)digits+=input_control(spans,n,1u,parent)*2u+1u;
 else if(frame_pending(c.kind))digits+=input_control(spans,n,3u,parent)+input_control(spans,n,5u,parent);
 sizes[i]=uint4(x.cursor_keep+x.cursor_add,x.repeat_keep+x.repeat_add,input_scalar(v,s,n,11u,parent)+x.capture_add,digits);
 // Task histogram atomics own x; this candidate exclusively owns y.
 selection[i].y=1u;
 if(p.task_slots) {
  uint task=active_task_slot(task_keys,1u,p.task_slots,s[parent]);
  if(task==NONE){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
  candidates[i].task_slot=task;
  (void)atomic_fetch_add_explicit(tasks+task*4u,1u,memory_order_relaxed);
 }
}

void successor_records(device const uchar* data,device const uint* input,device const uint* ih,device const uint* isp,device const uint* steps,device const Candidate* candidates,device const uint4* total,device const uint4* sizes,device const uint4* sums,device uint* output,device uint* osp,device atomic_uint* status,device uint* meta,device uint2* heads,constant SuccessorParams& p,uint i) {
 if(i<p.active_slots)heads[i]=uint2(NONE);
 if(any(sums[0]>uint4(p.out_cursors,p.out_repeats,p.out_captures,p.out_digits))){if(!i)atomic_store_explicit(status,1u,memory_order_relaxed);return;}
 if(atomic_load_explicit(status,memory_order_relaxed)!=0u || atomic_load_explicit(status+1u,memory_order_relaxed)!=NONE || atomic_load_explicit(status+2u,memory_order_relaxed)!=NONE || i>=total[0].x)return;
 Candidate c=candidates[i];if(!c.live)return;uint n=total[0].x,m=ih[0];device const Step& x=step_at(steps,c.frame_word);uint parent=x.input_parent;View v=view(data);
 for(uint k=0u;k<6u;k++)output[k*n+i]=input_scalar(v,input,m,k,parent);
 output[n+i]=c.pc;output[2u*n+i]=c.node;Program pg=v.programs[input_scalar(v,input,m,4u,parent)];output[3u*n+i]=successor_requirement(v,pg,c.pc);output[5u*n+i]=c.kind;
 uint4 start=sizes[i],end=i+1u<p.output?sizes[i+1u]:sums[0];
 for(uint k=0u;k<3u;k++) {output[(6u+2u*k)*n+i]=start[k];output[(7u+2u*k)*n+i]=end[k]-start[k];}
 uint length=input_control(isp,m,1u,parent)+x.branch_count+c.append,body=c.offer?length:(frame_pending(c.kind)?input_control(isp,m,3u,parent):0u),offer=c.offer?length-1u:(frame_pending(c.kind)?input_control(isp,m,5u,parent):0u);
 meta[i*7u]=body;meta[i*7u+2u]=c.offer?NONE:i;
 osp[i]=start.w;osp[n+i]=length;osp[2u*n+i]=start.w+length;osp[3u*n+i]=body;osp[4u*n+i]=start.w+length+body;osp[5u*n+i]=offer;
}
void successor_copy_values(device const uint* input,device const uint* ih,device const uint4* cursors,device const uint4* repeats,device const uint4* captures,device const uint* isp,device const uint* digits,device const uint* steps,device const Candidate* candidates,device const uint4* total,device const uint4* sizes,device const uint4* sums,device const uint4* selected,device const uint4* step_offsets,device uint4* oc,device uint4* orepeat,device uint4* ocapture,device uint* od,device const uint* status,constant SuccessorParams& p,uint tid) {
 if(status[0]!=0u || status[1]!=NONE || status[2]!=NONE)return;
 uint m=ih[0];
 for(uint k=0u;k<4u;k++) {
  if(tid>=sums[0][k])continue;uint i=owner(sizes,p.output,tid,k),entry=tid-sizes[i][k];Candidate c=candidates[i];device const Step& x=step_at(steps,c.frame_word);uint parent=x.input_parent;
  if(k==0u)oc[tid]=entry<x.cursor_keep?cursors[input_value_span(input,m,6u,parent)+entry]:uint4(step_cursors(x)[entry-x.cursor_keep],0u,0u);
  else if(k==1u)orepeat[tid]=uint4(entry,entry<x.repeat_keep?repeats[input_value_span(input,m,8u,parent)+entry].y:step_repeats(x)[entry-x.repeat_keep],0u,0u);
  else if(k==2u) {
   // Merge old ordered values with the finite region's new values. Each cell owns
   // its destination, including equal-geometry multiplicity and insertion order.
   uint start=input_value_span(input,m,10u,parent),old=input_value_span(input,m,11u,parent);
   if(x.query_order==2u){ocapture[tid]=entry<old?captures[start+entry]:step_captures(x)[entry-old];continue;}

   if(entry<old) {uint4 value=captures[start+entry];uint before=0u;for(uint a=0u;a<x.capture_add;a++)before+=uint(capture_edit_less(step_captures(x)[a],value,x.query_order));ocapture[sizes[i].z+entry+before]=value;}
   else {uint a=entry-old;uint4 value=step_captures(x)[a];uint lo=0u,hi=old;while(lo<hi) {uint mid=lo+(hi-lo)/2u;if(!capture_edit_less(value,captures[start+mid],x.query_order))lo=mid+1u;else hi=mid;}ocapture[sizes[i].z+lo+a]=value;}
  } else {
   uint length=input_control(isp,m,1u,parent),position=length+x.branch_count+c.append,body=c.offer?position:(frame_pending(c.kind)?input_control(isp,m,3u,parent):0u);
   if(entry<position)od[tid]=entry<length?digits[input_control(isp,m,0u,parent)+entry]:(entry<length+x.branch_count?step_branches(x)[entry-length]:selected[i].y-selected[step_offsets[x.logical_step].x].y);
   else if(entry<position+body) {entry-=position;od[tid]=c.offer?(entry<length?digits[input_control(isp,m,0u,parent)+entry]:0u):digits[input_control(isp,m,2u,parent)+entry];}
   else {entry-=position+body;od[tid]=digits[input_control(isp,m,c.offer?0u:4u,parent)+entry];}
  }
 }
}
kernel void successor_materialize(
 device const uchar* data [[buffer(0)]],device const uint* input [[buffer(1)]],device const uint* h [[buffer(2)]],device const uint* isp [[buffer(3)]],
 device const uint* steps [[buffer(4)]],device const Candidate* candidates [[buffer(5)]],device const uint4* total [[buffer(6)]],
 device const uint4* sizes [[buffer(7)]],device const uint4* sums [[buffer(8)]],device uint* columns [[buffer(9)]],device uint* spans [[buffer(10)]],
 device atomic_uint* status [[buffer(11)]],device uint* meta [[buffer(12)]],device uint2* heads [[buffer(13)]],
 device const uint4* cursors [[buffer(14)]],device const uint4* repeats [[buffer(15)]],device const uint4* captures [[buffer(16)]],device const uint* digits [[buffer(17)]],
 device const uint4* selected [[buffer(18)]],device const uint4* step_offsets [[buffer(19)]],
 device uint4* oc [[buffer(20)]],device uint4* orepeat [[buffer(21)]],device uint4* ocapture [[buffer(22)]],device uint* od [[buffer(23)]],
 constant SuccessorParams& p [[buffer(24)]],uint i [[thread_position_in_grid]]) {
 successor_records(data,input,h,isp,steps,candidates,total,sizes,sums,columns,spans,status,meta,heads,p,i);
 // Every lane rejects an overflowing arena before any independent cell copy.
 if(any(sums[0]>uint4(p.out_cursors,p.out_repeats,p.out_captures,p.out_digits)))return;
 successor_copy_values(input,h,cursors,repeats,captures,isp,digits,steps,candidates,total,sizes,sums,selected,step_offsets,
  oc,orepeat,ocapture,od,(device const uint*)status,p,i);
}
uint comparison_width(device const uint* s,device const uint* spans,uint n,uint i) {
 uint width=1u;for(uint k=0u;k<3u;k++)width=max(width,max(sc(s,n,7u+2u*k,i),sc(spans,n,1u+2u*k,i)));return width;
}

// Peers contain only surviving active keys and unmerged pending offers.
// Full comparisons below retain the original task-local control relations.
kernel void successor_peer_counts(device const uint* s [[buffer(0)]],device const Candidate* candidates [[buffer(1)]],device const uint4* total [[buffer(2)]],device const uint* meta [[buffer(3)]],device atomic_uint* counts [[buffer(4)]],device uint2* slots [[buffer(5)]],device atomic_uint* status [[buffer(6)]],constant SuccessorParams& p [[buffer(7)]],uint i [[thread_position_in_grid]]){
 if(atomic_load_explicit(status,memory_order_relaxed) || atomic_load_explicit(status+1u,memory_order_relaxed)!=NONE || atomic_load_explicit(status+2u,memory_order_relaxed)!=NONE || i>=total[0].x || !candidates[i].live || meta[i*7u+4u])return;
 uint task=candidates[i].task_slot;if(task>=p.task_slots || s[i]>=p.tasks){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 slots[i]=uint2(task,atomic_fetch_add_explicit(counts+task*4u,1u,memory_order_relaxed));
}
kernel void successor_peer_items(device const Candidate* candidates [[buffer(0)]],device const uint4* total [[buffer(1)]],device const uint* meta [[buffer(2)]],device const uint2* slots [[buffer(3)]],device const uint4* tasks [[buffer(4)]],device uint* items [[buffer(5)]],device const uint* status [[buffer(6)]],constant SuccessorParams& p [[buffer(7)]],uint i [[thread_position_in_grid]]){
 if(status[0] || status[1]!=NONE || status[2]!=NONE || i>=total[0].x || !candidates[i].live || meta[i*7u+4u])return;
 uint2 slot=slots[i];items[tasks[slot.x].x+slot.y]=i;
}
kernel void successor_pair_counts(device const uint* s [[buffer(0)]],device const uint* spans [[buffer(1)]],device const Candidate* candidates [[buffer(2)]],device const uint4* total [[buffer(3)]],device const uint4* tasks [[buffer(4)]],device const uint4* task_total [[buffer(5)]],device uint4* counts [[buffer(6)]],device uint2* domains [[buffer(7)]],device const uint* meta [[buffer(8)]],device const uint* status [[buffer(9)]],constant SuccessorParams& p [[buffer(10)]],uint i [[thread_position_in_grid]]){
 if(i>=p.output)return;counts[i]=uint4(0u);
 if(status[0] || status[1]!=NONE || status[2]!=NONE || i>=total[0].x || !candidates[i].live || meta[i*7u+4u] || !frame_pending(sc(s,total[0].x,5u,i)))return;
 uint task=candidates[i].task_slot,start=tasks[task].x,count=(task+1u<p.task_slots?tasks[task+1u].x:task_total[0].x)-start;
 domains[i]=uint2(start,count);ulong work=ulong(count)*comparison_width(s,spans,total[0].x,i);counts[i]=uint4(count,uint(min(work,0xfffffffful)),0u,0u);
}
kernel void successor_pair_initialize(device const uint4* total [[buffer(0)]],device uint4* comparisons [[buffer(1)]],device atomic_uint* status [[buffer(2)]],constant SuccessorParams& p [[buffer(3)]],uint tid [[thread_position_in_grid]]) {
 if(total[0].x>p.pairs || total[0].y>p.work) {if(!tid)atomic_store_explicit(status,1u,memory_order_relaxed);return;}
 if(tid<total[0].x)comparisons[tid]=uint4(0u,NONE,NONE,0u);
}
// Pair flags: unequal active key, unequal pending-exit key, failed descendant
// prefix, failed ancestor prefix. Other words locate the first differing digit.
kernel void successor_compare(device const uint* s [[buffer(0)]],device const uint* spans [[buffer(1)]],device const uint* digits [[buffer(2)]],device const uint4* cursors [[buffer(3)]],device const uint4* repeats [[buffer(4)]],device const uint4* captures [[buffer(5)]],device const uint4* total [[buffer(6)]],device const uint2* domains [[buffer(7)]],device const uint* items [[buffer(8)]],device const uint4* offsets [[buffer(9)]],device const uint4* sums [[buffer(10)]],device atomic_uint* comparisons [[buffer(11)]],device const uint* status [[buffer(12)]],constant SuccessorParams& p [[buffer(13)]],uint tid [[thread_position_in_grid]]) {
 if(status[0]!=0u || status[1]!=NONE || status[2]!=NONE || tid>=sums[0].y)return;
 uint n=total[0].x,i=owner(offsets,p.output,tid,1u),width=comparison_width(s,spans,n,i),local=tid-offsets[i].y,j=items[domains[i].x+local/width],entry=local%width,pair=offsets[i].x+local/width;
 bool equal=true,exit=true;for(uint k=0u;k<6u;k++) {bool same=sc(s,n,k,i)==sc(s,n,k,j);equal &= same;if(k!=2u)exit &= same;}
 for(uint k=0u;k<3u;k++) {
  uint start=sc(s,n,6u+2u*k,i),count=sc(s,n,7u+2u*k,i),other=sc(s,n,6u+2u*k,j),othern=sc(s,n,7u+2u*k,j);bool same=count==othern;
  device const uint4* arena=k==0u?cursors:(k==1u?repeats:captures);if(entry<min(count,othern))same &= all(arena[start+entry]==arena[other+entry]);equal &= same;exit &= same;
 }
 uint ip=sc(spans,n,0u,i),il=sc(spans,n,1u,i),jp=sc(spans,n,0u,j),jl=sc(spans,n,1u,j),ib=sc(spans,n,2u,i),ibl=sc(spans,n,3u,i),jb=sc(spans,n,2u,j),jbl=sc(spans,n,3u,j);
 bool descendant=jl>=ibl,ancestor=il>=jbl;
 if(entry<ibl && entry<jl)descendant &= digits[jp+entry]==digits[ib+entry];
 if(entry<jbl && entry<il)ancestor &= digits[ip+entry]==digits[jb+entry];
 uint bits=uint(!equal)|uint(!exit)*2u|uint(!descendant)*4u|uint(!ancestor)*8u;
 if(bits)atomic_fetch_or_explicit(comparisons+pair*4u,bits,memory_order_relaxed);
 if(entry<min(il,jl) && digits[ip+entry]!=digits[jp+entry])atomic_fetch_min_explicit(comparisons+pair*4u+2u,entry,memory_order_relaxed);
 uint io=sc(spans,n,4u,i),iol=sc(spans,n,5u,i),jo=sc(spans,n,4u,j),jol=sc(spans,n,5u,j);
 if(entry<min(iol,jol) && digits[io+entry]!=digits[jo+entry])atomic_fetch_min_explicit(comparisons+pair*4u+1u,entry,memory_order_relaxed);
}
bool path_less(device const uint* spans,device const uint* digits,uint n,uint i,uint j,uint field,uint difference) {
 uint start=sc(spans,n,field,i),other=sc(spans,n,field,j);
 return difference==NONE?sc(spans,n,field+1u,i)<sc(spans,n,field+1u,j):digits[start+difference]<digits[other+difference];
}
bool path_equal(device const uint* spans,uint n,uint i,uint j,uint field,uint difference) {return difference==NONE && sc(spans,n,field+1u,i)==sc(spans,n,field+1u,j);}
// Metadata per candidate: minimum ancestor depth, offer rank, owning marker index,
// latest winning offer rank plus one, duplicate flag, live descendant flag, output rank.
kernel void successor_ranks(device const uint* s [[buffer(0)]],device const uint* spans [[buffer(1)]],device const uint* digits [[buffer(2)]],device const uint4* total [[buffer(3)]],device const uint2* domains [[buffer(4)]],device const uint* items [[buffer(5)]],device const uint4* offsets [[buffer(6)]],device const uint4* sums [[buffer(7)]],device const uint4* comparisons [[buffer(8)]],device atomic_uint* meta [[buffer(9)]],device const Candidate* candidates [[buffer(10)]],device const uint* status [[buffer(11)]],constant SuccessorParams& p [[buffer(12)]],uint pair [[thread_position_in_grid]]) {
 if(status[0]!=0u || status[1]!=NONE || status[2]!=NONE || pair>=sums[0].x)return;
 uint n=total[0].x,i=owner(offsets,p.output,pair,0u),j=items[domains[i].x+pair-offsets[i].x];uint4 comparison=comparisons[pair];
 uint kind=sc(s,n,5u,i);
 if(frame_pending(kind) && frame_pending(sc(s,n,5u,j)) && (path_less(spans,digits,n,j,i,4u,comparison.y) || (path_equal(spans,n,i,j,4u,comparison.y) && j<i)))atomic_fetch_add_explicit(meta+i*7u+1u,1u,memory_order_relaxed);
}
kernel void successor_rank_ancestors(device const uint* s [[buffer(0)]],device const Candidate* candidates [[buffer(1)]],device const uint4* total [[buffer(2)]],device const uint2* domains [[buffer(3)]],device const uint* items [[buffer(4)]],device const uint4* offsets [[buffer(5)]],device const uint4* sums [[buffer(6)]],device const uint4* comparisons [[buffer(7)]],device const uint* spans [[buffer(8)]],device atomic_uint* meta [[buffer(9)]],device const uint* status [[buffer(10)]],device const uint4* tasks [[buffer(11)]],device uint* offers [[buffer(12)]],constant SuccessorParams& p [[buffer(13)]],uint pair [[thread_position_in_grid]]) {
 if(status[0]!=0u || status[1]!=NONE || status[2]!=NONE)return;
 if(pair<total[0].x && candidates[pair].live && frame_pending(sc(s,total[0].x,5u,pair)) && atomic_load_explicit(meta+pair*7u+4u,memory_order_relaxed)==0u)offers[tasks[candidates[pair].task_slot].x+atomic_load_explicit(meta+pair*7u+1u,memory_order_relaxed)]=pair;
 if(pair>=sums[0].x)return;
 uint n=total[0].x,i=owner(offsets,p.output,pair,0u),j=items[domains[i].x+pair-offsets[i].x];
 if(candidates[i].offer && atomic_load_explicit(meta+i*7u+4u,memory_order_relaxed)==0u && atomic_load_explicit(meta+j*7u+4u,memory_order_relaxed)==0u && frame_pending(sc(s,n,5u,j)) && (comparisons[pair].x&10u)==0u)atomic_fetch_min_explicit(meta+i*7u,sc(spans,n,3u,j),memory_order_relaxed);
}
kernel void successor_owners(device const uint* s [[buffer(0)]],device const uint* spans [[buffer(1)]],device const Candidate* candidates [[buffer(2)]],device const uint4* total [[buffer(3)]],device const uint2* domains [[buffer(4)]],device const uint* items [[buffer(5)]],device const uint4* offsets [[buffer(6)]],device const uint4* sums [[buffer(7)]],device const uint4* comparisons [[buffer(8)]],device atomic_uint* meta [[buffer(9)]],device const uint* status [[buffer(10)]],constant SuccessorParams& p [[buffer(11)]],uint pair [[thread_position_in_grid]]) {
 if(status[0]!=0u || status[1]!=NONE || status[2]!=NONE || pair>=sums[0].x)return;
 uint n=total[0].x,i=owner(offsets,p.output,pair,0u),j=items[domains[i].x+pair-offsets[i].x];
 if(candidates[i].offer && atomic_load_explicit(meta+i*7u+4u,memory_order_relaxed)==0u && atomic_load_explicit(meta+j*7u+4u,memory_order_relaxed)==0u && frame_pending(sc(s,n,5u,j)) && (i==j || (comparisons[pair].x&10u)==0u) && sc(spans,n,3u,j)==atomic_load_explicit(meta+i*7u,memory_order_relaxed))atomic_fetch_min_explicit(meta+i*7u+2u,j,memory_order_relaxed);
}
bool retained(device const uint* meta,uint i) {return meta[i*7u+4u]==0u && meta[i*7u+2u]==i;}
bool retained_atomic(device atomic_uint* meta,uint i){return atomic_load_explicit(meta+i*7u+4u,memory_order_relaxed)==0u && atomic_load_explicit(meta+i*7u+2u,memory_order_relaxed)==i;}
kernel void successor_offer_liveness(device const uint* s [[buffer(0)]],device const uint4* total [[buffer(1)]],device const uint2* domains [[buffer(2)]],device const uint* items [[buffer(3)]],device const uint4* offsets [[buffer(4)]],device const uint4* sums [[buffer(5)]],device const uint4* comparisons [[buffer(6)]],device atomic_uint* meta [[buffer(7)]],device atomic_uint* alive [[buffer(8)]],device const uint* status [[buffer(9)]],device const Candidate* candidates [[buffer(10)]],constant SuccessorParams& p [[buffer(11)]],uint pair [[thread_position_in_grid]]) {
 if(status[0]!=0u || status[1]!=NONE || status[2]!=NONE)return;
 if(pair<total[0].x && candidates[pair].live && frame_pending(sc(s,total[0].x,5u,pair)) && atomic_load_explicit(meta+pair*7u+4u,memory_order_relaxed)==0u){
 uint i=pair;uint marker=atomic_load_explicit(meta+i*7u+2u,memory_order_relaxed);
 atomic_fetch_max_explicit(meta+marker*7u+3u,atomic_load_explicit(meta+i*7u+1u,memory_order_relaxed)+1u,memory_order_relaxed);
 }
 if(pair>=sums[0].x)return;
 uint n=total[0].x,i=owner(offsets,p.output,pair,0u),j=items[domains[i].x+pair-offsets[i].x],kind=sc(s,n,5u,j);
 if(frame_pending(sc(s,n,5u,i)) && retained_atomic(meta,i) && retained_atomic(meta,j) && !frame_finished(kind) && (comparisons[pair].x&4u)==0u)atomic_store_explicit(alive+i,1u,memory_order_relaxed);
}
kernel void successor_final_counts(device const Candidate* candidates [[buffer(0)]],device const uint4* total [[buffer(1)]],device const uint* meta [[buffer(2)]],device uint4* counts [[buffer(3)]],device const uint* status [[buffer(4)]],constant SuccessorParams& p [[buffer(5)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.output)return;counts[i]=uint4(0u);if(status[0]!=0u || status[1]!=NONE || status[2]!=NONE || i>=total[0].x)return;counts[i].x=uint(candidates[i].live && retained(meta,i));
}
// At most 64 consecutive final rows per lane; no inactive slots survive.
kernel void successor_survivors(device const Candidate* candidates [[buffer(0)]],device const uint4* total [[buffer(1)]],device const uint* meta [[buffer(2)]],
 device uint4* offsets [[buffer(3)]],device const uint* status [[buffer(4)]],device uint4* sums [[buffer(5)]],constant SuccessorParams& p [[buffer(6)]],uint lane [[thread_index_in_threadgroup]]) {
 threadgroup uint counts[256];uint width=(p.output+255u)/256u,first=lane*width,last=min(first+width,p.output),local=0u;
 bool valid=status[0]==0u && status[1]==NONE && status[2]==NONE;
 for(uint i=first;i<last;i++)local+=uint(valid && i<total[0].x && candidates[i].live && retained(meta,i));
 counts[lane]=local;threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint step=1u;step<256u;step*=2u){uint add=lane>=step?counts[lane-step]:0u;
  threadgroup_barrier(mem_flags::mem_threadgroup);counts[lane]+=add;threadgroup_barrier(mem_flags::mem_threadgroup);}
 uint at=lane?counts[lane-1u]:0u;
 for(uint i=first;i<last;i++){offsets[i]=uint4(at,0u,0u,0u);at+=uint(valid && i<total[0].x && candidates[i].live && retained(meta,i));}
 if(!lane)sums[0]=uint4(counts[255],0u,0u,0u);
}
void successor_publish(device const uint* s,device const uint* spans,device const Candidate* candidates,device const uint4* total,device const uint4* sizes,device const uint4* offsets,device const uint4* sums,device const uint4* tasks,device const uint* offers,device const uint* meta,device const uint* alive,device uint* output,device uint* osp,device uint* h,device uint* lengths,device uint* controls,device const uint* status,device const uint* steps,device uint* path_work,constant SuccessorParams& p,uint i) {
 if(!i){h[0]=h[1]=h[2]=0u;h[3]=h[4]=1u;lengths[0]=lengths[1]=controls[0]=0u;}
 if(status[0]!=0u || status[1]!=NONE || status[2]!=NONE)return;uint n=total[0].x,m=sums[0].x;
 if(i==0u) {h[0]=m;h[1]=sizes[0].z;h[2]=(m+255u)/256u;h[3]=h[4]=1u;lengths[0]=sizes[0].x;lengths[1]=sizes[0].y;controls[0]=sizes[0].w;}
 if(i>=n || !candidates[i].live || !retained(meta,i))return;
 uint at=offsets[i].x,kind=sc(s,n,5u,i),winner=frame_pending(kind)?offers[tasks[candidates[i].task_slot].x+meta[i*7u+3u]-1u]:i;
 path_work[at]=step_at(steps,candidates[winner].frame_word).path_work;
 for(uint k=0u;k<12u;k++)output[k*m+at]=sc(s,n,k,winner);
 for(uint k=0u;k<6u;k++)osp[k*m+at]=sc(spans,n,k,k>=4u?winner:i);
 if(frame_pending(kind) && !alive[i]) {output[5u*m+at]=kind==6u?0u:2u;osp[3u*m+at]=osp[5u*m+at]=0u;}

}
void successor_publish_values(device const uint4* cursors,device const uint4* repeats,device const uint4* captures,device const uint* digits,device const uint4* sizes,device uint4* oc,device uint4* orepeat,device uint4* ocapture,device uint* od,device const uint* status,constant SuccessorParams& p,uint i) {
 if(status[0]!=0u || status[1]!=NONE || status[2]!=NONE)return;
 if(i<sizes[0].x)oc[i]=cursors[i];if(i<sizes[0].y)orepeat[i]=repeats[i];if(i<sizes[0].z)ocapture[i]=captures[i];if(i<sizes[0].w)od[i]=digits[i];
}

// One broad scatter publishes scalar frames and all four typed value arenas.
kernel void successor_scatter(
 device const uint* s [[buffer(0)]],device const uint* spans [[buffer(1)]],device const Candidate* candidates [[buffer(2)]],device const uint4* total [[buffer(3)]],
 device const uint4* sizes [[buffer(4)]],device const uint4* offsets [[buffer(5)]],device const uint4* sums [[buffer(6)]],device const uint4* tasks [[buffer(7)]],
 device const uint* offers [[buffer(8)]],device const uint* meta [[buffer(9)]],device const uint* alive [[buffer(10)]],device uint* output [[buffer(11)]],
 device uint* osp [[buffer(12)]],device uint* h [[buffer(13)]],device uint* lengths [[buffer(14)]],device uint* controls [[buffer(15)]],
 device const uint* status [[buffer(16)]],device const uint* steps [[buffer(17)]],device uint* path_work [[buffer(18)]],
 device const uint4* cursors [[buffer(19)]],device const uint4* repeats [[buffer(20)]],device const uint4* captures [[buffer(21)]],device const uint* digits [[buffer(22)]],
 device uint4* oc [[buffer(23)]],device uint4* orepeat [[buffer(24)]],device uint4* ocapture [[buffer(25)]],device uint* od [[buffer(26)]],
 constant SuccessorParams& p [[buffer(27)]],uint i [[thread_position_in_grid]]) {
 successor_publish(s,spans,candidates,total,sizes,offsets,sums,tasks,offers,meta,alive,output,osp,h,lengths,controls,status,steps,path_work,p,i);
 successor_publish_values(cursors,repeats,captures,digits,sizes,oc,orepeat,ocapture,od,status,p,i);
}
