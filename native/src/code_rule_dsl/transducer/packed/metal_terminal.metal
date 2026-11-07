// Immutable provenance replaces the ordered frame after checked projection.
struct TerminalSource {uint task,program,pc;};
struct TerminalParams {uint capacity,results,captures,spans,width;};
uint tc(device const uint* s,uint n,uint field,uint i) {return s[field*n+i];}
uint terminal_evaluation(View v,uint task,uint spans);
kernel void terminal_admission_flags(device const uint* h [[buffer(0)]],device const uint* representatives [[buffer(1)]],device const uint* grouping [[buffer(2)]],device uint4* flags [[buffer(3)]],device atomic_uint* status [[buffer(4)]],device const uchar* data [[buffer(5)]],device const uint* s [[buffer(6)]],device const uint2* errors [[buffer(7)]],constant TerminalParams& p [[buffer(8)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.capacity)return;
 uint error=grouping[6];if(error)atomic_store_explicit(status,(error&2u)?2u:1u,memory_order_relaxed);
 flags[i]=uint4(0u);if(atomic_load_explicit(status,memory_order_relaxed) || i>=h[0] || representatives[i]!=i)return;
 View v=view(data);uint evaluation=terminal_evaluation(v,s[i],p.spans);
 if(evaluation==NONE){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 flags[i].x=uint(frame_terminal(tc(s,h[0],5u,i)) && errors[evaluation].x==0u);
}
kernel void terminal_task_starts(device const uint* s [[buffer(0)]],device const uint* h [[buffer(1)]],device const uint4* offsets [[buffer(2)]],device uint* tasks [[buffer(3)]],device atomic_uint* status [[buffer(4)]],constant TerminalParams& p [[buffer(5)]],uint i [[thread_position_in_grid]]) {
 if(atomic_load_explicit(status,memory_order_relaxed) || i>=h[0])return;
 uint rank=tc(s,h[0],12u,i);if(rank>i){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint first=i-rank;
 if(s[first]!=s[i] || (first && s[first-1u]==s[i])){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 tasks[i]=offsets[first].x;
}
kernel void terminal_project(device const uint* s [[buffer(0)]],device const uint* h [[buffer(1)]],device const uint* representatives [[buffer(2)]],device const uint4* offsets [[buffer(3)]],device const uint4* total [[buffer(4)]],device const uint* tasks [[buffer(5)]],device uint* output [[buffer(6)]],device uint* counts [[buffer(7)]],device uint* admitted [[buffer(8)]],device uint* status [[buffer(9)]],device TerminalSource* sources [[buffer(10)]],constant TerminalParams& p [[buffer(11)]],uint i [[thread_position_in_grid]]) {
 if(!i) {counts[0]=counts[1]=counts[2]=0u;counts[3]=counts[4]=1u;}
 // All lanes use the same immutable preconditions, including overflow.
 if(total[0].x==NONE || total[0].x>p.capacity) {if(!i)status[0]=1u;return;}
 if(status[0])return;
 uint n=h[0],m=total[0].x;
 if(!i) {counts[0]=m;counts[1]=h[1];counts[2]=m/256u+uint(m%256u!=0u);}
 if(i>=n)return;
 sources[i]={s[i],tc(s,n,4u,i),tc(s,n,1u,i)};
 if(representatives[i]!=i || (i+1u<n?offsets[i+1u].x:total[0].x)==offsets[i].x)return;
 uint at=offsets[i].x,task=s[i],ordinal=at-tasks[i];
 // Ordinary occurrences receive distinct scopes. Query containment is task-local.
 output[at]=task;output[m+at]=tc(s,n,5u,i)==1u?0u:ordinal+1u;
 for(uint field=2u;field<8u;field++)output[field*m+at]=0u;
 output[8u*m+at]=tc(s,n,10u,i);output[9u*m+at]=tc(s,n,11u,i);output[10u*m+at]=ordinal;admitted[at]=i;
}
kernel void terminal_candidates(device const uchar* data [[buffer(0)]],device const TerminalSource* s [[buffer(1)]],device const uint* h [[buffer(2)]],device const uint* candidates [[buffer(3)]],device const uint* counts [[buffer(4)]],device const uint* tasks [[buffer(5)]],device const uint* admitted [[buffer(6)]],device const uint4* captures [[buffer(7)]],device const uint* containment [[buffer(8)]],device uint4* metadata [[buffer(9)]],device uint* ids [[buffer(10)]],device uint* output_counts [[buffer(11)]],device atomic_uint* status [[buffer(12)]],device const uint4* admitted_total [[buffer(13)]],constant TerminalParams& p [[buffer(14)]],uint i [[thread_position_in_grid]]) {
 if(!i) {output_counts[0]=output_counts[1]=0u;}
 if(i>=p.capacity)return;ids[i]=i;
 uint error=containment[6];if(error)atomic_store_explicit(status,(error&2u)?2u:1u,memory_order_relaxed);
 if(atomic_load_explicit(status,memory_order_relaxed) || i>=counts[0])return;
 uint n=counts[0],task=candidates[i],ordinal=tc(candidates,n,10u,i),lo=0u,hi=h[0];
 while(lo<hi){uint mid=lo+(hi-lo)/2u;if(s[mid].task<task)lo=mid+1u;else hi=mid;}
 if(lo==h[0] || s[lo].task!=task || ulong(tasks[lo])+ordinal>=admitted_total[0].x){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint source=admitted[tasks[lo]+ordinal];
 if(source>=h[0] || s[source].task!=task){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 View v=view(data);Program pg=v.programs[s[source].program];
 uint first=((pg.flags&1u) && tc(candidates,n,9u,i))?captures[tc(candidates,n,8u,i)].x+1u:0u;
 metadata[i]=uint4(task,first,ordinal,source);
}
bool terminal_less(uint4 a,uint4 b) {return a.x<b.x || (a.x==b.x && (a.y<b.y || (a.y==b.y && a.z<b.z)));}
kernel void terminal_publication_merge(device const uint* h [[buffer(0)]],device const uint4* metadata [[buffer(1)]],device const uint* ids [[buffer(2)]],device uint* output [[buffer(3)]],device const uint* status [[buffer(4)]],constant TerminalParams& p [[buffer(5)]],uint i [[thread_position_in_grid]]) {
 if(status[0] || i>=h[0])return;
 uint n=h[0],base=(i/p.width/2u)*p.width*2u,middle=uint(min(ulong(n),ulong(base)+p.width)),end=uint(min(ulong(n),ulong(base)+2ul*p.width));
 bool left=i<middle;uint begin=left?middle:base,lo=begin,hi=left?end:middle,a=ids[i];
 while(lo<hi) {uint mid=lo+(hi-lo)/2u,b=ids[mid];if(terminal_less(metadata[b],metadata[a]))lo=mid+1u;else hi=mid;}
 output[base+(i-(left?base:middle))+(lo-begin)]=a;
}
kernel void terminal_publication_sizes(device const uint* s [[buffer(0)]],device const uint* h [[buffer(1)]],device const uint* ids [[buffer(2)]],device uint4* flags [[buffer(3)]],device const uint* status [[buffer(4)]],device const uchar* data [[buffer(5)]],device const uint4* metadata [[buffer(6)]],constant TerminalParams& p [[buffer(7)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.capacity)return;flags[i]=uint4(0u);if(status[0] || i>=h[0])return;
 View v=view(data);uint task=metadata[ids[i]].x;
 uint lo=0u,hi=p.spans;while(lo<hi) {uint mid=lo+(hi-lo)/2u;if(v.spans[v.semantic_order[mid]].semantic_start<=task)lo=mid+1u;else hi=mid;}
 Evaluation evaluation=v.evaluations[v.spans[v.semantic_order[lo-1u]].evaluation];
 uint limit=v.limits[evaluation.program];
 lo=0u;hi=i;while(lo<hi) {uint mid=lo+(hi-lo)/2u;if(metadata[ids[mid]].x<evaluation.semantic_start)lo=mid+1u;else hi=mid;}
 if(i-lo<limit)flags[i]=uint4(1u,tc(s,h[0],9u,ids[i]),0u,0u);
}
// The two scatters consume the same immutable scan. Neither reads the other's output.
kernel void terminal_publish(
 device const uchar* data [[buffer(0)]],device const TerminalSource* states [[buffer(1)]],
 device const uint* header [[buffer(2)]],device const uint* candidates [[buffer(3)]],
 device const uint* counts [[buffer(4)]],device const uint4* metadata [[buffer(5)]],
 device const uint* ids [[buffer(6)]],device const uint4* offsets [[buffer(7)]],
 device const uint4* total [[buffer(8)]],device uint* output_counts [[buffer(9)]],
 device Result* results [[buffer(10)]],device const uint4* captures [[buffer(11)]],
 device Capture* output_captures [[buffer(12)]],device atomic_uint* status [[buffer(13)]],
 constant TerminalParams& p [[buffer(14)]],uint i [[thread_position_in_grid]]) {
 if(atomic_load_explicit(status,memory_order_relaxed))return;
 uint4 n=total[0];
 if(n.x>p.results || n.y>p.captures || n.x==NONE || n.y==NONE){if(!i)atomic_store_explicit(status,1u,memory_order_relaxed);return;}
 if(any(n.zw!=uint2(0u))){if(!i)atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 if(!i){output_counts[0]=n.x;output_counts[1]=n.y;}
 if(i<counts[0]){
  uint next=i+1u<counts[0]?offsets[i+1u].x:n.x;
  if(next!=offsets[i].x){
   uint source=ids[i];uint4 meta=metadata[source];View v=view(data);Program pg=v.programs[states[meta.w].program];
   uint low=0u,high=p.spans;while(low<high){uint mid=low+(high-low)/2u;if(v.spans[v.semantic_order[mid]].semantic_start<=meta.x)low=mid+1u;else high=mid;}
   Span span=v.spans[v.semantic_order[low-1u]];Evaluation e=v.evaluations[span.evaluation];Root root=v.roots[select_root(v,v.selectors[e.selector],span.selector_offset+meta.x-span.semantic_start)];
   uint start=v.files[root.file].node_start+root.node,label=v.words[pg.structural_start+states[meta.w].pc]>>16;
   results[offsets[i].x]={meta.x,meta.z,start,label,offsets[i].y,tc(candidates,counts[0],9u,source),0u,0u,0u,0u};
  }
 }
 if(i<n.y){
  uint low=0u,high=counts[0];while(low<high){uint mid=low+(high-low)/2u;if(offsets[mid].y<=i)low=mid+1u;else high=mid;}
  uint at=low-1u;uint4 cell=captures[tc(candidates,counts[0],8u,ids[at])+i-offsets[at].y];
  output_captures[i]={cell.y,cell.x,cell.z,cell.w,NONE,0u,0u,0u,1u};
 }
}
