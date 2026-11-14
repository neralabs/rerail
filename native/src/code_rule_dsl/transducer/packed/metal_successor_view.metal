// Read complete active keys from immutable input and bounded edits before copying.
struct ExpandedParams {SuccessorParams p;uint input_count,output_count,copy_kind;};
struct ExpandedInput {
 View v;device const uint* s;device const uint4* cursors;device const uint4* repeats;device const uint4* captures;
 device const uint* spans;device const uint* digits;device const uint* steps;device const uint4* offsets;uint n;device atomic_uint* status;
};
ExpandedInput expanded_input(device const uchar* data,device const uint* s,device const uint4* c,device const uint4* r,device const uint4* a,device const uint* spans,device const uint* digits,device const uint* steps,device const uint4* offsets,uint n,device atomic_uint* status){return {view(data),s,c,r,a,spans,digits,steps,offsets,n,status};}
struct ExpandedRow {uint frame,choice,parent,task,pc,node,requirement,program,kind,live;};
ExpandedRow expanded_row(ExpandedInput v,constant ExpandedParams& p,uint i){
 uint step=owner(v.offsets,p.p.steps,i,0u);device const Step& x=step_at(v.steps,v.offsets[step].z);
 ExpandedRow r={v.offsets[step].z,i-v.offsets[step].x,x.input_parent,0u,x.pc,x.node,0u,0u,x.kind,1u};
 r.task=v.s[r.parent];r.program=v.s[v.n+r.parent];
 if(x.expansion==1u && r.choice==1u)r.pc=x.begin;
 if(x.expansion>=3u){r.node=x.expansion==4u?x.begin+r.choice:v.v.children[x.begin+r.choice];r.live=uint(!x.named || (v.v.nodes[r.node].flags&1u)!=0u);}
 r.requirement=successor_requirement(v.v,v.v.programs[r.program],r.pc);return r;
}
uint expanded_length(ExpandedInput v,ExpandedRow r,uint kind){device const Step& x=step_at(v.steps,r.frame);return kind==0u?x.cursor_keep+x.cursor_add:(kind==1u?x.repeat_keep+x.repeat_add:v.s[5u*v.n+r.parent]+x.capture_add);}
uint4 expanded_capture(device const uint4* old,uint start,uint count,device const Step& x,uint entry,device atomic_uint* status){
 if(entry>=count+x.capture_add){atomic_store_explicit(status,2u,memory_order_relaxed);return uint4(0u);}
 if(x.query_order==2u)return entry<count?old[start+entry]:step_captures(x)[entry-count];
 if(!x.capture_add)return old[start+entry];if(!count)return step_captures(x)[entry];
 // Old values precede equal new values. Complete the divergent partition
 // search before selecting a vector; returning from its loop corrupts values.
 uint low=entry>x.capture_add?entry-x.capture_add:0u,high=min(entry,count);
 while(low<high){uint a=low+(high-low)/2u,b=entry-a;
  if(b && a<count && !capture_edit_less(step_captures(x)[b-1u],old[start+a],x.query_order))low=a+1u;
  else high=a;
 }
 uint a=low,b=entry-a;
 if(a<count){
  if(b==x.capture_add)return old[start+a];
  if(b<x.capture_add && !capture_edit_less(step_captures(x)[b],old[start+a],x.query_order))return old[start+a];
 }
 if(b<x.capture_add)return step_captures(x)[b];
 atomic_store_explicit(status,2u,memory_order_relaxed);return uint4(0u);
}
uint4 expanded_value(ExpandedInput v,ExpandedRow r,uint kind,uint entry){
 device const Step& x=step_at(v.steps,r.frame);uint start=v.s[(2u+kind)*v.n+r.parent];
 if(kind==0u)return entry<x.cursor_keep?v.cursors[start+entry]:uint4(step_cursors(x)[entry-x.cursor_keep],0u,0u);
 if(kind==1u)return uint4(entry,entry<x.repeat_keep?v.repeats[start+entry].y:step_repeats(x)[entry-x.repeat_keep],0u,0u);
 return expanded_capture(v.captures,start,v.s[5u*v.n+r.parent],x,entry,v.status);
}
uint expanded_rank(View v,device const Step& x,uint node,uint choice,device atomic_uint* status){
 if(x.expansion<3u || !x.named)return choice;
 uint before=x.expansion==4u?v.named_order[x.begin].preorder:(x.expansion==5u?v.named_order[x.node].sibling+uint((v.nodes[x.node].flags&1u)!=0u):0u);
 uint ordinal=x.expansion==4u?v.named_order[node].preorder:v.named_order[node].sibling;if(ordinal<before){atomic_store_explicit(status,2u,memory_order_relaxed);return 0u;}return ordinal-before;
}
uint expanded_position_length(ExpandedInput v,ExpandedRow r){device const Step& x=step_at(v.steps,r.frame);return input_control(v.spans,v.n,1u,r.parent)+x.branch_count+uint(x.expansion!=0u);}
uint expanded_position(ExpandedInput v,ExpandedRow r,uint entry){device const Step& x=step_at(v.steps,r.frame);uint length=input_control(v.spans,v.n,1u,r.parent);
 return entry<length?v.digits[input_control(v.spans,v.n,0u,r.parent)+entry]:(entry<length+x.branch_count?step_branches(x)[entry-length]:expanded_rank(v.v,x,r.node,r.choice,v.status));}
uint expanded_hash(ExpandedInput v,ExpandedRow r){
 uint hash=2166136261u;uint fields[6]={r.task,r.pc,r.node,r.requirement,r.program,r.kind};
 for(uint k=0u;k<6u;k++){hash=(hash^fields[k])*16777619u;hash^=hash>>16u;}
 for(uint k=0u;k<3u;k++){uint n=expanded_length(v,r,k);hash=(hash^n)*16777619u;hash^=hash>>16u;
  if(n)for(uint sample=0u;sample<3u;sample++){uint4 value=expanded_value(v,r,k,sample==0u?0u:(sample==1u?n/2u:n-1u));for(uint cell=0u;cell<4u;cell++){hash=(hash^value[cell])*16777619u;hash^=hash>>16u;}}}
 return hash;
}
bool expanded_equal(ExpandedInput v,ExpandedRow a,ExpandedRow b,threadgroup uint* scratch,uint lane){
 bool same=a.task==b.task && a.pc==b.pc && a.node==b.node && a.requirement==b.requirement && a.program==b.program && a.kind==b.kind;
 for(uint k=0u;k<3u;k++){uint an=expanded_length(v,a,k),bn=expanded_length(v,b,k);same &= an==bn;
  for(ulong j=lane;j<min(an,bn);j+=32ul)same &= all(expanded_value(v,a,k,uint(j))==expanded_value(v,b,k,uint(j)));}
 scratch[lane]=uint(!same);threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint width=16u;width;width>>=1u){if(lane<width)scratch[lane]|=scratch[lane+width];threadgroup_barrier(mem_flags::mem_threadgroup);}
 bool result=scratch[0]==0u;threadgroup_barrier(mem_flags::mem_threadgroup);return result;
}
bool expanded_less(ExpandedInput v,ExpandedRow a,ExpandedRow b,uint ai,uint bi,threadgroup uint* scratch,uint lane){
 uint an=expanded_position_length(v,a),bn=expanded_position_length(v,b),first=NONE;
 for(ulong j=lane;j<min(an,bn);j+=32ul)if(expanded_position(v,a,uint(j))!=expanded_position(v,b,uint(j)))first=min(first,uint(j));
 scratch[lane]=first;threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint width=16u;width;width>>=1u){if(lane<width)scratch[lane]=min(scratch[lane],scratch[lane+width]);threadgroup_barrier(mem_flags::mem_threadgroup);}
 first=scratch[0];bool result=first==NONE?(an==bn?ai<bi:an<bn):expanded_position(v,a,first)<expanded_position(v,b,first);
 threadgroup_barrier(mem_flags::mem_threadgroup);return result;
}
bool expanded_status(device const atomic_uint* status){return atomic_load_explicit(status,memory_order_relaxed)==0u && atomic_load_explicit(status+1u,memory_order_relaxed)==NONE && atomic_load_explicit(status+2u,memory_order_relaxed)==NONE && atomic_load_explicit(status+3u,memory_order_relaxed)<=1u;}
#define EXPANDED_INPUT_ARGS device const uchar* data [[buffer(0)]],device const uint* s [[buffer(1)]],device const uint4* cursors [[buffer(2)]],device const uint4* repeats [[buffer(3)]],device const uint4* captures [[buffer(4)]],device const uint* spans [[buffer(5)]],device const uint* digits [[buffer(6)]],device const uint* steps [[buffer(7)]],device const uint4* offsets [[buffer(8)]],device const uint4* total [[buffer(9)]],device atomic_uint* status [[buffer(10)]]
#define EXPANDED_VIEW expanded_input(data,s,cursors,repeats,captures,spans,digits,steps,offsets,p.input_count,status)
// Six source-value columns and two source-position columns have separate owners.
// Completed prefix validation proves that pending body and offer spans are empty.
kernel void expanded_project_input(device const uchar* data [[buffer(0)]],device const uint* input [[buffer(1)]],device const uint* spans [[buffer(2)]],device uint* values [[buffer(3)]],device uint* positions [[buffer(4)]],device atomic_uint* status [[buffer(5)]],constant ExpandedParams& p [[buffer(6)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.input_count || !expanded_status(status))return;View v=view(data);uint n=p.input_count;
 if(frame_pending(input_scalar(v,input,n,5u,i)) || input_control(spans,n,3u,i) || input_control(spans,n,5u,i)){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 values[i]=input_scalar(v,input,n,0u,i);values[n+i]=input_scalar(v,input,n,4u,i);
 for(uint k=0u;k<3u;k++)values[(2u+k)*n+i]=input_scalar(v,input,n,6u+2u*k,i);
 values[5u*n+i]=input_scalar(v,input,n,11u,i);positions[i]=input_control(spans,n,0u,i);positions[n+i]=input_control(spans,n,1u,i);
}
// Heads store row + 1. Zero is the only empty value, including before row zero wins.
kernel void expanded_keys(EXPANDED_INPUT_ARGS,device atomic_uint* heads [[buffer(11)]],device uint* slots [[buffer(12)]],constant ExpandedParams& p [[buffer(13)]],uint i [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]){
 if(i>=p.p.output || !expanded_status(status))return;ExpandedInput v=EXPANDED_VIEW;ExpandedRow r=expanded_row(v,p,i);if(!r.live || !query_kind(r.kind))return;
 if(!p.p.active_slots){if(!lane)atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 threadgroup uint scratch[32],decision[2];if(!lane)decision[0]=expanded_hash(v,r)%p.p.active_slots;threadgroup_barrier(mem_flags::mem_threadgroup);uint slot=decision[0];bool found=false;
 for(ulong probe=0ul;probe<=ulong(total[0].w);probe++){
  if(!lane){uint previous=atomic_load_explicit(heads+slot,memory_order_relaxed);if(!previous){uint expected;bool exchanged;do{expected=0u;exchanged=atomic_compare_exchange_weak_explicit(heads+slot,&expected,i+1u,memory_order_relaxed,memory_order_relaxed);}while(!exchanged && !expected);previous=exchanged?i+1u:expected;}decision[0]=previous-1u;}
  threadgroup_barrier(mem_flags::mem_threadgroup);uint other=decision[0];if(other>=p.p.output){if(!lane)atomic_store_explicit(status,2u,memory_order_relaxed);return;}
  if(other==i || expanded_equal(v,r,expanded_row(v,p,other),scratch,lane)){found=true;break;}slot=slot+1u==p.p.active_slots?0u:slot+1u;
 }
 if(!found){if(!lane)atomic_store_explicit(status,1u,memory_order_relaxed);return;}
 if(!lane)decision[0]=atomic_load_explicit(heads+slot,memory_order_relaxed)-1u;threadgroup_barrier(mem_flags::mem_threadgroup);
 for(ulong changes=0ul;changes<=ulong(p.p.output);changes++){
  uint previous=decision[0];if(previous>=p.p.output){if(!lane)atomic_store_explicit(status,2u,memory_order_relaxed);return;}
  if(!expanded_less(v,r,expanded_row(v,p,previous),i,previous,scratch,lane)){if(!lane)slots[i]=slot;return;}
  if(!lane){uint expected;bool exchanged;do{expected=previous+1u;exchanged=atomic_compare_exchange_weak_explicit(heads+slot,&expected,i+1u,memory_order_relaxed,memory_order_relaxed);}while(!exchanged && expected==previous+1u);decision[0]=expected-1u;decision[1]=uint(exchanged);}
  threadgroup_barrier(mem_flags::mem_threadgroup);if(decision[1]){if(!lane)slots[i]=slot;return;}
 }
 if(!lane)atomic_store_explicit(status,2u,memory_order_relaxed);
}
// Admission precedes guard work. Readiness proofs bound this linear tail.
void expanded_tail(ExpandedInput v,device const uint* ends,thread ExpandedRow& r,device atomic_uint* status){
 if(!r.live || (r.kind!=0u && r.kind!=2u))return;
 Program pg=v.v.programs[r.program];device const Step& original=step_at(v.steps,r.frame);
 if(r.pc>=pg.structural_count || uint(original.units)>EXPANDED_UNIT_INSTRUCTIONS){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint end=ends[pg.structural_start+r.pc];
 if(end<r.pc || end-r.pc>8u){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 Step x={};x.node=r.node;x.kind=r.kind;x.live=1u;x.path_work=original.path_work;
 for(uint unit=0u;unit<8u && r.pc<end && uint(original.units)+unit<EXPANDED_UNIT_INSTRUCTIONS;unit++){
  if(x.path_work>=PATH_WORK_LIMIT){r.kind=frame_query(r.kind)?FRAME_QUERY_WORK_FAILURE:FRAME_ORDINARY_WORK_FAILURE;return;}
  x.path_work++;uint w=v.v.words[pg.structural_start+r.pc];expanded_guard(v.v,pg,w&65535u,w>>16u,x);
  if(x.boundary){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
  if(!x.live){r.live=0u;return;}r.pc++;
 }
 if(r.pc>=pg.structural_count)r.live=0u;
}
uint expanded_kept(device const uint4* offsets,device const uint4* total,uint n,uint i){return (i+1u<n?offsets[i+1u].x:total[0].x)-offsets[i].x;}
kernel void expanded_counts(EXPANDED_INPUT_ARGS,device const uint* heads [[buffer(11)]],device const uint* slots [[buffer(12)]],device uint4* counts [[buffer(13)]],device uint4* position_counts [[buffer(14)]],device const uint* guard_ends [[buffer(15)]],constant ExpandedParams& p [[buffer(16)]],uint i [[thread_position_in_grid]],uint group [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]){
 uint keep=0u,length=0u;uint4 value=uint4(0u);
 if(i<p.p.output && expanded_status(status)){
  ExpandedInput v=EXPANDED_VIEW;ExpandedRow r=expanded_row(v,p,i);keep=r.live;
  if(keep && query_kind(r.kind)){uint slot=slots[i];if(slot>=p.p.active_slots || !heads[slot] || heads[slot]-1u>=p.p.output){atomic_store_explicit(status,2u,memory_order_relaxed);keep=0u;}else keep=uint(heads[slot]==i+1u);}
  if(keep){expanded_tail(v,guard_ends,r,status);keep=r.live;}
  if(keep){value=uint4(1u,expanded_length(v,r,0u),expanded_length(v,r,1u),expanded_length(v,r,2u));length=expanded_position_length(v,r);}
 }
 if(i<p.p.output)counts[i]=value;
 threadgroup ulong sums[256];sums[lane]=ulong(length);threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint width=128u;width;width>>=1u){if(lane<width)sums[lane]+=sums[lane+width];threadgroup_barrier(mem_flags::mem_threadgroup);}
 if(!lane)position_counts[group]=uint4(uint(min(sums[0],0xfffffffful)),0u,0u,0u);
}
// One group forms both source-value and source-position metadata. No canonical
// row is visible until capture publication consumes every source frame.
kernel void expanded_metadata(EXPANDED_INPUT_ARGS,device const uint4* selected [[buffer(11)]],device const uint4* selected_total [[buffer(12)]],device uint* columns [[buffer(13)]],device const uint* guard_ends [[buffer(14)]],device const uint4* position_offsets [[buffer(15)]],device uint* output_spans [[buffer(16)]],device uint* path_work [[buffer(17)]],constant ExpandedParams& p [[buffer(18)]],uint i [[thread_position_in_grid]],uint group [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]){
 uint keep=i<p.p.output?expanded_kept(selected,selected_total,p.p.output,i):0u,length=0u;
 ExpandedInput v=EXPANDED_VIEW;ExpandedRow r={};
 if(keep>1u){atomic_store_explicit(status,2u,memory_order_relaxed);keep=0u;}
 if(keep && expanded_status(status)){
  r=expanded_row(v,p,i);expanded_tail(v,guard_ends,r,status);
  if(!r.live || selected[i].x>=p.output_count || i>0x0fffffffu || frame_pending(r.kind)){atomic_store_explicit(status,2u,memory_order_relaxed);keep=0u;}
  else length=expanded_position_length(v,r);
 }
 threadgroup ulong values[256];values[lane]=ulong(length);threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint d=1u;d<256u;d*=2u){ulong add=lane>=d?values[lane-d]:0ul;threadgroup_barrier(mem_flags::mem_threadgroup);values[lane]+=add;threadgroup_barrier(mem_flags::mem_threadgroup);}
 if(!keep || !expanded_status(status))return;
 uint at=selected[i].x,n=p.output_count;device const Step& x=step_at(steps,r.frame);
 ulong start=ulong(position_offsets[group].x)+(lane?values[lane-1u]:0ul);
 uint rank=expanded_rank(v.v,x,r.node,r.choice,status);
 if(start>p.p.out_digits || length>p.p.out_digits-start || rank>0x0fffffffu){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 columns[at]=r.task;columns[n+at]=r.pc;columns[2u*n+at]=r.node;columns[3u*n+at]=r.frame;columns[4u*n+at]=r.program;columns[5u*n+at]=(rank<<4u)|r.kind;
 for(uint k=0u;k<3u;k++){columns[(6u+2u*k)*n+at]=s[(2u+k)*v.n+r.parent];columns[(7u+2u*k)*n+at]=expanded_length(v,r,k);}
 output_spans[at]=input_control(spans,p.input_count,0u,r.parent);output_spans[n+at]=length;output_spans[2u*n+at]=uint(start);
 output_spans[3u*n+at]=selected[i].y;output_spans[4u*n+at]=selected[i].z;output_spans[5u*n+at]=selected[i].w;
 uint base=x.expansion==1u && r.choice==1u?x.begin:x.pc;
 if(r.pc<base || r.pc-base>8u){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 path_work[at]=frame_work_failure(r.kind)?PATH_WORK_LIMIT+1u:x.path_work+r.pc-base;
}
// Shared capture transport for production publication and the partition
// conformance fixture. Each row has exactly one 32-lane owner.
void expanded_capture_row(device const uint* columns,device const uint* spans,device const uint* steps,device const uint4* old_captures,device uint4* captures,device atomic_uint* status,constant ExpandedParams& p,uint row,uint lane){
 uint n=p.output_count;if(row>=n)return;
 device const Step& x=step_at(steps,columns[3u*n+row]);
 uint start=columns[10u*n+row],count=columns[11u*n+row],out=spans[5u*n+row];
 if(count<x.capture_add || ulong(out)+count>p.p.out_captures){if(!lane)atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 for(uint k=lane;k<count;k+=32u)captures[out+k]=expanded_capture(old_captures,start,count-x.capture_add,x,k,status);
}
kernel void expanded_copy(device const uchar* data [[buffer(0)]],device const uint* columns [[buffer(1)]],device const uint* spans [[buffer(2)]],device const uint* steps [[buffer(3)]],device const uint4* old_captures [[buffer(4)]],device uint4* captures [[buffer(5)]],device atomic_uint* status [[buffer(6)]],constant ExpandedParams& p [[buffer(7)]],uint row [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]){
 if(expanded_status(status))expanded_capture_row(columns,spans,steps,old_captures,captures,status,p,row,lane);
}
struct FusedExpandedParams {ExpandedParams expanded;ReadyParams ready;};
// One SIMD group owns a row. Three small arenas share one copy dispatch.
// Captures retain a separate allocation boundary and one separate source owner.
kernel void expanded_copy_small(device const uchar* data [[buffer(0)]],device const uint* columns [[buffer(1)]],device const uint* spans [[buffer(2)]],device const uint* steps [[buffer(3)]],device const uint4* old_cursors [[buffer(4)]],device const uint4* old_repeats [[buffer(5)]],device const uint* old_digits [[buffer(6)]],device uint4* cursors [[buffer(7)]],device uint4* repeats [[buffer(8)]],device uint* digits [[buffer(9)]],device atomic_uint* status [[buffer(10)]],device const uint* state_header [[buffer(11)]],device const uint* value_header [[buffer(12)]],device atomic_uint* tasks [[buffer(13)]],device atomic_uint* ready_status [[buffer(14)]],device uint* ready_counts [[buffer(15)]],constant FusedExpandedParams& fp [[buffer(16)]],uint row [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]){
 constant ExpandedParams& p=fp.expanded;
 if(!expanded_status(status))return;
 ready_initialize_item(state_header,value_header,tasks,ready_status,ready_counts,fp.ready,row*32u+lane);
 uint n=p.output_count;if(row>=n)return;
 device const Step& x=step_at(steps,columns[3u*n+row]);
 uint cs=columns[6u*n+row],cn=columns[7u*n+row],rs=columns[8u*n+row],rn=columns[9u*n+row],dn=spans[n+row];
 uint co=spans[3u*n+row],ro=spans[4u*n+row],ds=spans[row],dout=spans[2u*n+row];
 if(ulong(co)+cn>p.p.out_cursors || ulong(ro)+rn>p.p.out_repeats || ulong(dout)+dn>p.p.out_digits || cn!=x.cursor_keep+x.cursor_add || rn!=x.repeat_keep+x.repeat_add || dn<x.branch_count+uint(x.expansion!=0u)){if(!lane)atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint old_length=dn-x.branch_count-uint(x.expansion!=0u);
 for(uint k=lane;k<cn;k+=32u)cursors[co+k]=k<x.cursor_keep?old_cursors[cs+k]:uint4(step_cursors(x)[k-x.cursor_keep],0u,0u);
 for(uint k=lane;k<rn;k+=32u)repeats[ro+k]=uint4(k,k<x.repeat_keep?old_repeats[rs+k].y:step_repeats(x)[k-x.repeat_keep],0u,0u);
 for(uint k=lane;k<dn;k+=32u)digits[dout+k]=k<old_length?old_digits[ds+k]:(k<old_length+x.branch_count?step_branches(x)[k-old_length]:columns[5u*n+row]>>4u);
}
// Capture copying, canonical conversion, and readiness classification share one
// row-owned group. No other group reads this row's mutable bridge metadata.
kernel void expanded_publish(device const uchar* data [[buffer(0)]],device uint* columns [[buffer(1)]],device uint* spans [[buffer(2)]],device const uint* steps [[buffer(3)]],device const uint4* old_captures [[buffer(4)]],device uint4* captures [[buffer(5)]],device atomic_uint* status [[buffer(6)]],device const uint* state_header [[buffer(7)]],device const uint* value_header [[buffer(8)]],device const uint2* proof [[buffer(9)]],device atomic_uint* tasks [[buffer(10)]],device atomic_uint* ready_status [[buffer(11)]],device atomic_uint* ready_counts [[buffer(12)]],constant FusedExpandedParams& fp [[buffer(13)]],uint row [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]){
 constant ExpandedParams& p=fp.expanded;
 uint n=p.output_count;if(row>=n)return;
 threadgroup uint valid_status;if(!lane)valid_status=uint(expanded_status(status));threadgroup_barrier(mem_flags::mem_threadgroup);if(!valid_status)return;
 expanded_capture_row(columns,spans,steps,old_captures,captures,status,p,row,lane);
 threadgroup_barrier(mem_flags::mem_device);
 if(lane || !expanded_status(status))return;
 View v=view(data);columns[3u*n+row]=successor_requirement(v,v.programs[columns[4u*n+row]],columns[n+row]);columns[5u*n+row]&=15u;
 for(uint k=0u;k<3u;k++)columns[(6u+2u*k)*n+row]=spans[(3u+k)*n+row];
 uint ds=spans[2u*n+row];spans[row]=ds;spans[2u*n+row]=spans[4u*n+row]=ds+spans[n+row];spans[3u*n+row]=spans[5u*n+row]=0u;
 ready_classify_item(data,columns,state_header,value_header,proof,tasks,ready_status,ready_counts,fp.ready,row);
}
