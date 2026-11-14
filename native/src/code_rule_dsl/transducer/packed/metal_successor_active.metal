// Batch-local exact keys. A published head always names an immutable candidate.
// Replacing a head changes only its earliest logical owner, never its full key.
bool successor_active_equal(device const uint* s,uint n,uint a,uint b,device const uint4* cursors,device const uint4* repeats,device const uint4* captures,threadgroup uint* scratch,uint lane){
 bool equal=true;for(uint k=0u;k<6u;k++)equal &= sc(s,n,k,a)==sc(s,n,k,b);
 for(uint k=0u;k<3u;k++){
  uint as=sc(s,n,6u+2u*k,a),bs=sc(s,n,6u+2u*k,b),al=sc(s,n,7u+2u*k,a),bl=sc(s,n,7u+2u*k,b);
  equal &= al==bl;device const uint4* values=k==0u?cursors:(k==1u?repeats:captures);
  for(ulong entry=lane;entry<min(al,bl);entry+=256ul)equal &= all(values[as+uint(entry)]==values[bs+uint(entry)]);
 }
 scratch[lane]=uint(!equal);threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint width=128u;width;width>>=1u){if(lane<width)scratch[lane]|=scratch[lane+width];threadgroup_barrier(mem_flags::mem_threadgroup);}
 bool result=scratch[0]==0u;threadgroup_barrier(mem_flags::mem_threadgroup);return result;
}
bool successor_active_less(device const uint* spans,device const uint* digits,uint n,uint a,uint b,threadgroup uint* scratch,uint lane){
 uint as=spans[a],bs=spans[b],al=spans[n+a],bl=spans[n+b],first=NONE;
 for(ulong entry=lane;entry<min(al,bl);entry+=256ul)if(digits[as+uint(entry)]!=digits[bs+uint(entry)])first=min(first,uint(entry));
 scratch[lane]=first;threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint width=128u;width;width>>=1u){if(lane<width)scratch[lane]=min(scratch[lane],scratch[lane+width]);threadgroup_barrier(mem_flags::mem_threadgroup);}
 first=scratch[0];bool less=first==NONE?(al==bl?a<b:al<bl):digits[as+first]<digits[bs+first];
 threadgroup_barrier(mem_flags::mem_threadgroup);return less;
}
uint successor_active_hash(device const uint* s,uint n,uint i,device const uint4* cursors,device const uint4* repeats,device const uint4* captures){
 uint hash=2166136261u;
 for(uint k=0u;k<6u;k++){hash=(hash^sc(s,n,k,i))*16777619u;hash^=hash>>16u;}
 for(uint k=0u;k<3u;k++){
  uint start=sc(s,n,6u+2u*k,i),count=sc(s,n,7u+2u*k,i);hash=(hash^count)*16777619u;hash^=hash>>16u;
  if(!count)continue;device const uint4* values=k==0u?cursors:(k==1u?repeats:captures);
  for(uint sample=0u;sample<3u;sample++){uint4 value=values[start+(sample==0u?0u:(sample==1u?count/2u:count-1u))];for(uint cell=0u;cell<4u;cell++){hash=(hash^value[cell])*16777619u;hash^=hash>>16u;}}
 }
 return hash;
}
bool successor_active_offer(device const Candidate* candidates,device const uint* s,uint n,uint i){
 return sc(s,n,5u,i)==0u && i+1u<n && candidates[i+1u].offer && candidates[i].frame_word==candidates[i+1u].frame_word;
}
// A failed exchange observes a strictly earlier owner. Spurious weak failures
// retry that same exchange, as in the shared value-work counter.
bool successor_active_minimum(device atomic_uint* head,uint i,uint n,device const uint* spans,device const uint* digits,threadgroup uint* scratch,threadgroup uint* decision,uint lane){
 if(!lane)decision[0]=atomic_load_explicit(head,memory_order_relaxed);
 threadgroup_barrier(mem_flags::mem_threadgroup);
 for(ulong change=0ul;change<=ulong(n);change++){
  uint previous=decision[0];if(previous!=NONE && previous>=n)return false;
  bool replace=previous==NONE;
  if(!replace)replace=successor_active_less(spans,digits,n,i,previous,scratch,lane);
  if(!replace)return true;
  if(!lane){
   uint expected=previous;bool exchanged;
   do{expected=previous;exchanged=atomic_compare_exchange_weak_explicit(head,&expected,i,memory_order_relaxed,memory_order_relaxed);}while(!exchanged && expected==previous);
   decision[0]=expected;decision[1]=uint(exchanged);
  }
  threadgroup_barrier(mem_flags::mem_threadgroup);
  if(decision[1])return true;
 }
 return false;
}
kernel void successor_active_keys(device const uint* s [[buffer(0)]],device const uint* spans [[buffer(1)]],device const uint* digits [[buffer(2)]],device const uint4* cursors [[buffer(3)]],device const uint4* repeats [[buffer(4)]],device const uint4* captures [[buffer(5)]],device const Candidate* candidates [[buffer(6)]],device const uint4* total [[buffer(7)]],device atomic_uint* heads [[buffer(8)]],device uint* slots [[buffer(9)]],device atomic_uint* status [[buffer(10)]],constant SuccessorParams& p [[buffer(11)]],uint i [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]){
 uint n=total[0].x;if(i>=n || !candidates[i].live || !query_kind(sc(s,n,5u,i)))return;
 threadgroup uint scratch[256],decision[2];
 if(!lane)decision[0]=uint(atomic_load_explicit(status,memory_order_relaxed)==0u && atomic_load_explicit(status+1u,memory_order_relaxed)==NONE && atomic_load_explicit(status+2u,memory_order_relaxed)==NONE);
 threadgroup_barrier(mem_flags::mem_threadgroup);if(!decision[0])return;
 uint keys=total[0].w;
 if(!keys || keys>n || ulong(p.active_slots)<ulong(keys)+(ulong(keys)+1ul)/2ul){if(!lane)atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 if(!lane)decision[0]=successor_active_hash(s,n,i,cursors,repeats,captures)%p.active_slots;
 threadgroup_barrier(mem_flags::mem_threadgroup);uint slot=decision[0];bool found=false;
 for(ulong probe=0ul;probe<=ulong(keys);probe++){
  if(!lane){
   uint previous=atomic_load_explicit(heads+slot*2u,memory_order_relaxed);
   if(previous==NONE){uint expected;bool exchanged;do{expected=NONE;exchanged=atomic_compare_exchange_weak_explicit(heads+slot*2u,&expected,i,memory_order_relaxed,memory_order_relaxed);}while(!exchanged && expected==NONE);previous=exchanged?i:expected;}
   decision[0]=previous;
  }
  threadgroup_barrier(mem_flags::mem_threadgroup);uint other=decision[0];
  if(other>=n){if(!lane)atomic_store_explicit(status,2u,memory_order_relaxed);return;}
  if(other==i || successor_active_equal(s,n,i,other,cursors,repeats,captures,scratch,lane)){found=true;break;}
  slot=slot+1u==p.active_slots?0u:slot+1u;
 }
 if(!found){if(!lane)atomic_store_explicit(status,1u,memory_order_relaxed);return;}
 bool valid=successor_active_minimum(heads+slot*2u,i,n,spans,digits,scratch,decision,lane);
 if(valid && successor_active_offer(candidates,s,n,i))valid=successor_active_minimum(heads+slot*2u+1u,i,n,spans,digits,scratch,decision,lane);
 if(!lane){if(!valid)atomic_store_explicit(status,2u,memory_order_relaxed);else slots[i]=slot;}
}
kernel void successor_active_owners(device const uint* s [[buffer(0)]],device const Candidate* candidates [[buffer(1)]],device const uint4* total [[buffer(2)]],device const uint2* heads [[buffer(3)]],device const uint* slots [[buffer(4)]],device atomic_uint* meta [[buffer(5)]],device atomic_uint* status [[buffer(6)]],constant SuccessorParams& p [[buffer(7)]],uint i [[thread_position_in_grid]]){
 uint n=total[0].x;if(i>=n || !candidates[i].live || !query_kind(sc(s,n,5u,i)) || atomic_load_explicit(status,memory_order_relaxed) || atomic_load_explicit(status+1u,memory_order_relaxed)!=NONE || atomic_load_explicit(status+2u,memory_order_relaxed)!=NONE)return;
 uint slot=slots[i];if(slot>=p.active_slots || heads[slot].x>=n){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 if(heads[slot].x!=i)atomic_store_explicit(meta+i*7u+4u,1u,memory_order_relaxed);
 if(successor_active_offer(candidates,s,n,i)){
  if(heads[slot].y>=n){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
  if(heads[slot].y!=i)atomic_store_explicit(meta+(i+1u)*7u+4u,1u,memory_order_relaxed);
 }
}
