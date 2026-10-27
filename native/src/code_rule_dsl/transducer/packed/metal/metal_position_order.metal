struct PositionParams {uint capacity,output,tasks,width;};
uint poscol(device const uint* s,uint n,uint field,uint i) {return s[field*n+i];}
// One workgroup compares complete paths. Long common prefixes are distributed
// over all lanes; no single lane walks an entire history.
int position_compare(device const uint* s,device const uint* spans,device const uint* digits,uint n,uint a,uint b,threadgroup uint* scratch,uint lane) {
 if(s[a]!=s[b])return s[a]<s[b]?-1:1;
 uint as=spans[a],bs=spans[b],al=spans[n+a],bl=spans[n+b],first=NONE;
 for(ulong k=lane;k<min(al,bl);k+=256ul)if(digits[as+uint(k)]!=digits[bs+uint(k)])first=min(first,uint(k));
 scratch[lane]=first;threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint stride=128u;stride;stride>>=1u) {if(lane<stride)scratch[lane]=min(scratch[lane],scratch[lane+stride]);threadgroup_barrier(mem_flags::mem_threadgroup);}
 first=scratch[0];threadgroup_barrier(mem_flags::mem_threadgroup);
 if(first!=NONE)return digits[as+first]<digits[bs+first]?-1:1;
 return al==bl?0:(al<bl?-1:1);
}
kernel void position_initialize(device uint* ids [[buffer(0)]],device uint* tasks [[buffer(1)]],device uint* h [[buffer(2)]],constant PositionParams& p [[buffer(3)]],uint i [[thread_position_in_grid]]) {
 if(i<p.capacity)ids[i]=i;
 if(i<p.tasks) {for(uint k=0u;k<5u;k++)tasks[6u*i+k]=NONE;tasks[6u*i+5u]=0u;}
 if(!i) {h[0]=h[1]=h[2]=0u;h[3]=h[4]=1u;}
}
kernel void position_merge(device const uint* s [[buffer(0)]],device const uint* h [[buffer(1)]],device const uint* spans [[buffer(2)]],device const uint* digits [[buffer(3)]],device const uint* ids [[buffer(4)]],device uint* output [[buffer(5)]],device const uint* status [[buffer(6)]],constant PositionParams& p [[buffer(7)]],uint i [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]) {
 if(status[0] || i>=h[0])return;threadgroup uint scratch[256];
 uint n=h[0],base=(i/p.width/2u)*p.width*2u,middle=uint(min(ulong(n),ulong(base)+p.width)),end=uint(min(ulong(n),ulong(base)+2ul*p.width));
 bool left=i<middle;uint begin=left?middle:base,lo=begin,hi=left?end:middle,a=ids[i];
 while(lo<hi) {uint mid=lo+(hi-lo)/2u,b=ids[mid];int cmp=position_compare(s,spans,digits,n,b,a,scratch,lane);if(cmp<0 || (cmp==0 && b<a))lo=mid+1u;else hi=mid;}
 if(!lane)output[base+(i-(left?base:middle))+(lo-begin)]=a;
}
// A group performs four merge levels over sixteen complete paths. The same
// cooperative comparator handles every digit, including long common prefixes.
kernel void position_local_sort(device const uint* s [[buffer(0)]],device const uint* h [[buffer(1)]],device const uint* spans [[buffer(2)]],device const uint* digits [[buffer(3)]],device const uint* ids [[buffer(4)]],device uint* output [[buffer(5)]],device const uint* status [[buffer(6)]],constant PositionParams& p [[buffer(7)]],uint group [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]) {
 uint base=group*16u,n=h[0];if(status[0] || base>=n)return;
 uint count=min(16u,n-base);threadgroup uint first[16],second[16],scratch[256];
 if(lane<count)first[lane]=ids[base+lane];threadgroup_barrier(mem_flags::mem_threadgroup);
 bool current=false;
 for(uint width=1u;width<16u;width*=2u){
  for(uint i=0u;i<count;i++){
   uint local=(i/width/2u)*width*2u,middle=min(count,local+width),end=min(count,local+2u*width);
   bool left=i<middle;uint begin=left?middle:local,lo=begin,hi=left?end:middle,a=current?second[i]:first[i];
   while(lo<hi){uint mid=lo+(hi-lo)/2u,b=current?second[mid]:first[mid];int cmp=position_compare(s,spans,digits,n,b,a,scratch,lane);if(cmp<0 || (cmp==0 && b<a))lo=mid+1u;else hi=mid;}
   if(!lane){uint at=local+(i-(left?local:middle))+(lo-begin);if(current)first[at]=a;else second[at]=a;}
  }
  threadgroup_barrier(mem_flags::mem_threadgroup);current=!current;
 }
 if(lane<count)output[base+lane]=current?second[lane]:first[lane];
}
kernel void position_select(device const uchar* data [[buffer(0)]],device const uint* s [[buffer(1)]],device const uint* h [[buffer(2)]],device const uint* spans [[buffer(3)]],device const uint* digits [[buffer(4)]],device const uint* ids [[buffer(5)]],device const uint* categories [[buffer(6)]],device const uint2* proof [[buffer(7)]],device atomic_uint* tasks [[buffer(8)]],device uint4* flags [[buffer(9)]],device atomic_uint* status [[buffer(10)]],constant PositionParams& p [[buffer(11)]],uint at [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]) {
 if(at>=p.capacity)return;if(!lane)flags[at]=uint4(0u);
 // Snapshot the prior status uniformly: another group's duplicate detection
 // must not make only part of this group skip its comparison barriers.
 threadgroup uint prior; if(!lane)prior=atomic_load_explicit(status,memory_order_relaxed);threadgroup_barrier(mem_flags::mem_threadgroup);
 if(prior || at>=h[0])return;
 uint n=h[0],i=ids[at];threadgroup uint scratch[256];
 if(at && position_compare(s,spans,digits,n,ids[at-1u],i,scratch,lane)==0) {if(!lane)atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 if(lane)return;
 uint slot=active_task_slot(tasks,6u,p.tasks,s[i]);if(slot==NONE){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint task=slot*6u+1u,kind=poscol(s,n,5u,i),pc=poscol(s,n,1u,i);View v=view(data);Program pg=v.programs[poscol(s,n,4u,i)];uint2 region=proof[pg.structural_start+pc];
 atomic_fetch_min_explicit(tasks+task,at,memory_order_relaxed);
 if(kind==0u) {if(region.x==NONE)atomic_store_explicit(tasks+task+4u,1u,memory_order_relaxed);else atomic_fetch_min_explicit(tasks+task+2u,region.x,memory_order_relaxed);}
 if(kind==4u)atomic_store_explicit(tasks+task+4u,1u,memory_order_relaxed);
 if(categories[i]!=5u)return;
 if(kind!=0u || region.y!=1u) {atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint exit=v.words[pg.structural_start+pc]>>16;
 atomic_fetch_min_explicit(tasks+task+1u,exit,memory_order_relaxed);atomic_fetch_min_explicit(tasks+task+3u,region.x,memory_order_relaxed);
 flags[at]=uint4(1u,0u,0u,0u);
}
kernel void position_scopes(device const uchar* data [[buffer(0)]],device const uint* s [[buffer(1)]],device const uint* h [[buffer(2)]],device const uint* categories [[buffer(3)]],device const uint2* proof [[buffer(4)]],device const uint* tasks [[buffer(5)]],device atomic_uint* status [[buffer(6)]],constant PositionParams& p [[buffer(7)]],uint i [[thread_position_in_grid]]) {
 if(atomic_load_explicit(status,memory_order_relaxed) || i>=h[0])return;
 uint slot=find_active_task_slot(tasks,6u,p.tasks,s[i]);if(slot==NONE){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint n=h[0],task=slot*6u+1u,pc=poscol(s,n,1u,i),kind=poscol(s,n,5u,i);bool valid=true;
 if(kind==6u && pc==tasks[task+1u])valid=false;
 if(kind==0u && tasks[task+1u]!=NONE) {View v=view(data);Program pg=v.programs[poscol(s,n,4u,i)];if(proof[pg.structural_start+pc].x==tasks[task+2u])valid &= categories[i]==5u;}
 if(categories[i]==5u) {View v=view(data);Program pg=v.programs[poscol(s,n,4u,i)];valid &= tasks[task+4u]==0u && proof[pg.structural_start+pc].x==tasks[task+2u] && (v.words[pg.structural_start+pc]>>16)==tasks[task+1u];}
 if(!valid)atomic_store_explicit(status,2u,memory_order_relaxed);
}
kernel void position_boundary(device const uint* input [[buffer(0)]],device const uint4* total [[buffer(1)]],device uint* status [[buffer(2)]],device uint* output [[buffer(3)]],constant PositionParams& p [[buffer(4)]],uint i [[thread_position_in_grid]]) {
 if(i)return;if(total[0].x>p.output || total[0].x==NONE)status[0]=1u;if(status[0])return;
 uint n=total[0].x;output[0]=n;output[1]=input[1];output[2]=n/256u+uint(n%256u!=0u);output[3]=output[4]=1u;
}
kernel void position_scatter(device const uint* s [[buffer(0)]],device const uint* h [[buffer(1)]],device const uint* ids [[buffer(2)]],device const uint* categories [[buffer(3)]],device const uint* tasks [[buffer(4)]],device const uint4* offsets [[buffer(5)]],device atomic_uint* status [[buffer(6)]],device uint* output [[buffer(7)]],device const uint* counts [[buffer(8)]],device uint* originals [[buffer(9)]],constant PositionParams& p [[buffer(10)]],uint at [[thread_position_in_grid]]) {
 if(atomic_load_explicit(status,memory_order_relaxed) || at>=h[0])return;uint i=ids[at];if(categories[i]!=5u)return;
 uint slot=find_active_task_slot(tasks,6u,p.tasks,s[i]);if(slot==NONE){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint n=h[0],m=counts[0],target=offsets[at].x;for(uint c=0u;c<12u;c++)output[c*m+target]=s[c*n+i];
 output[12u*m+target]=at-tasks[slot*6u+1u];originals[target]=i;
}

// No task can publish while an active or pending continuation remains.
kernel void terminal_order_select(device const uchar* data [[buffer(0)]],device const uint* s [[buffer(1)]],device const uint* h [[buffer(2)]],device const uint* spans [[buffer(3)]],device const uint* digits [[buffer(4)]],device const uint* ids [[buffer(5)]],device const uint* categories [[buffer(6)]],device const uint2* proof [[buffer(7)]],device atomic_uint* tasks [[buffer(8)]],device uint4* flags [[buffer(9)]],device atomic_uint* status [[buffer(10)]],constant PositionParams& p [[buffer(11)]],uint at [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]) {
 if(at>=p.capacity)return;if(!lane)flags[at]=uint4(0u);
 threadgroup uint prior;if(!lane)prior=atomic_load_explicit(status,memory_order_relaxed);threadgroup_barrier(mem_flags::mem_threadgroup);
 if(prior || at>=h[0])return;
 uint n=h[0],i=ids[at];threadgroup uint scratch[256];
 if(at && position_compare(s,spans,digits,n,ids[at-1u],i,scratch,lane)==0) {if(!lane)atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 if(lane)return;uint kind=poscol(s,n,5u,i);
 if(categories[i]!=13u || !frame_finished(kind)) {atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint slot=active_task_slot(tasks,6u,p.tasks,s[i]);if(slot==NONE){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 atomic_fetch_min_explicit(tasks+slot*6u+1u,at,memory_order_relaxed);flags[at]=uint4(1u,0u,0u,0u);
}
kernel void terminal_order_scatter(device const uint* s [[buffer(0)]],device const uint* h [[buffer(1)]],device const uint* ids [[buffer(2)]],device const uint* categories [[buffer(3)]],device const uint* tasks [[buffer(4)]],device const uint4* offsets [[buffer(5)]],device atomic_uint* status [[buffer(6)]],device uint* output [[buffer(7)]],device const uint* counts [[buffer(8)]],device uint* originals [[buffer(9)]],constant PositionParams& p [[buffer(10)]],uint at [[thread_position_in_grid]]) {
 if(atomic_load_explicit(status,memory_order_relaxed) || at>=h[0])return;uint i=ids[at],n=h[0];
 uint slot=find_active_task_slot(tasks,6u,p.tasks,s[i]);if(slot==NONE){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 for(uint c=0u;c<12u;c++)output[c*n+at]=s[c*n+i];
 output[12u*n+at]=at-tasks[slot*6u+1u];originals[at]=i;
}
