struct ReconstructParams {uint capacity,ordered,domains,entries,cells,exits,output,cursors,repeats,captures,digits,spans,composed;};
uint rcol(device const uint* s,uint n,uint field,uint i) {return s[field*n+i];}
uint rgroups(uint n) {return n/256u+uint(n%256u!=0u);}
struct ClosureProgress { uint pc,node,kind,work,choice; };
uint reconstruct_failed_guard(uint program,uint pc,Node n) {
 // GENERATED_RECONSTRUCTION_GUARDS
 return NONE;
}
uint reconstruct_sibling_rank(View v,uint parent,uint node) {
 Node p=v.nodes[parent];uint low=0u,high=p.child_count;
 while(low<high){uint middle=low+(high-low)/2u;if(v.children[p.child_start+middle]<node)low=middle+1u;else high=middle;}
 return low<p.child_count && v.children[p.child_start+low]==node?low:NONE;
}
kernel void reconstruct_progress(
 device const uchar* data [[buffer(0)]],device const uint* states [[buffer(1)]],
 device const uint* header [[buffer(2)]],device const uint* replacements [[buffer(3)]],
 device const uint2* exits [[buffer(4)]],device const uint* nodes [[buffer(5)]],
 device const uint* work [[buffer(6)]],device ClosureProgress* progress [[buffer(7)]],
 device atomic_uint* status [[buffer(8)]],constant ReconstructParams& p [[buffer(9)]],
 uint i [[thread_position_in_grid]]) {
 if(i>=p.capacity)return;progress[i]={0u,0u,0u,0u,NONE};
 if(atomic_load_explicit(status,memory_order_relaxed) || i>=header[0] || replacements[i]==NONE)return;
 uint n=header[0],pc=rcol(states,n,1u,i),node=rcol(states,n,2u,i),kind=rcol(states,n,5u,i);
 progress[i]={pc,node,kind,work[i],NONE};if(!replacements[i])return;
 View v=view(data);uint program=rcol(states,n,4u,i);Program pg=v.programs[program];
 uint word=v.words[pg.structural_start+pc],exit=word>>16;
 if(kind!=0u || work[i]>PATH_WORK_LIMIT || (word&255u)!=48u || ((word>>8)&255u)!=1u || exit>=pg.structural_count || exit<pc+4u || exit-pc-4u>64u){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint guards=exit-pc-4u,cycle=guards+4u,endpoint=nodes[exits[replacements[i]-1u].y];
 uint parent=v.nodes[node].parent,first=0u,last=0u;
 bool valid=parent==v.nodes[endpoint].parent;
 if(parent==NONE)valid=valid && endpoint==node;
 else {first=reconstruct_sibling_rank(v,parent,node);last=reconstruct_sibling_rank(v,parent,endpoint);valid=valid && first!=NONE && last!=NONE && last>=first;}
 if(!valid){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint distance=last-first,remaining=PATH_WORK_LIMIT-work[i];ulong completed=ulong(distance)*cycle;
 bool failed=completed>remaining;
 if(!failed){
  uint next=v.nodes[endpoint].next_any,attempt=2u;
  if(next!=NONE){uint guard=reconstruct_failed_guard(program,pc,v.nodes[next]);
   if(guard>guards){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
   // A failed guard ends the attempt. Otherwise it reaches an earlier entry
   // in this batch. Physical exhaustion ends after NEXT.
   attempt=guard<guards?3u+guard:cycle;
  }
  failed=ulong(remaining)<completed+attempt;
 }
 if(!failed){progress[i]={exit,endpoint,0u,uint(ulong(work[i])+completed+1ul),1u};return;}
 uint iterations=min(distance,remaining/cycle),offset=remaining-iterations*cycle;
 if(offset>=cycle){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint current=node;
 if(parent!=NONE){Node chain=v.nodes[parent];uint rank=first+iterations+uint(offset>=2u);
  if(rank>=chain.child_count){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
  current=v.children[chain.child_start+rank];
 }else if(offset>=2u){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 progress[i]={pc+offset,current,FRAME_QUERY_WORK_FAILURE,PATH_WORK_LIMIT+1u,0u};
}
kernel void reconstruct_initialize(device const uint* h [[buffer(0)]],device const uint* ready [[buffer(1)]],device const uint* ranked [[buffer(2)]],device const uint* ranges [[buffer(3)]],device const uint* exits [[buffer(4)]],device const uint* categories [[buffer(5)]],device uint* entries [[buffer(6)]],device uint* replacements [[buffer(7)]],device uint* output [[buffer(8)]],device uint* values [[buffer(9)]],device uint* control [[buffer(10)]],device atomic_uint* status [[buffer(11)]],device const uint* preparation [[buffer(12)]],device const uint* grouping [[buffer(13)]],device const uint* closure [[buffer(14)]],constant ReconstructParams& p [[buffer(15)]],uint i [[thread_position_in_grid]]) {
 if(i<p.capacity) {entries[i]=NONE;replacements[i]=categories[i]==5u?NONE:0u;}
 if(i)return;output[0]=output[1]=output[2]=0u;output[3]=output[4]=1u;values[0]=values[1]=control[0]=0u;
 if(p.composed) {
  uint failure=preparation[0]|grouping[6]|closure[0];
  if(failure)atomic_fetch_max_explicit(status,(failure&2u)?2u:1u,memory_order_relaxed);
 }
 if(atomic_load_explicit(status,memory_order_relaxed))return;
 bool valid=ranked[0]<=p.ordered && ranked[0]==ready[1] && ranked[1]==h[1] && ranked[2]==rgroups(ranked[0]) && ranked[3]==1u && ranked[4]==1u
 && ranges[0]<=p.domains && ranges[1]<=p.entries && ranges[1]==ranked[0] && ranges[2]<=p.cells
 && exits[0]<=p.exits && exits[0]<=ranked[0] && exits[1]==rgroups(exits[0]) && exits[2]==1u && exits[3]==1u;
 if(!valid)atomic_store_explicit(status,2u,memory_order_relaxed);
}
kernel void reconstruct_entries(device const uint* s [[buffer(0)]],device const uint* h [[buffer(1)]],device const uint* ranked [[buffer(2)]],device const uint* rh [[buffer(3)]],device const uint* originals [[buffer(4)]],device const uint* categories [[buffer(5)]],device atomic_uint* entries [[buffer(6)]],device atomic_uint* status [[buffer(7)]],constant ReconstructParams& p [[buffer(8)]],uint i [[thread_position_in_grid]]) {
 if(atomic_load_explicit(status,memory_order_relaxed) || i>=rh[0])return;
 uint original=originals[i],n=h[0],m=rh[0];if(original>=n) {atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 bool valid=categories[original]==5u;
 for(uint k=0u;k<12u;k++)valid &= ranked[k*m+i]==s[k*n+original];
 if(i)valid &= ranked[i-1u]<ranked[i] || (ranked[i-1u]==ranked[i] && ranked[12u*m+i-1u]<ranked[12u*m+i]);
 valid &= atomic_exchange_explicit(entries+original,i,memory_order_relaxed)==NONE;
 if(!valid)atomic_store_explicit(status,2u,memory_order_relaxed);
}
kernel void reconstruct_exits(device const uchar* data [[buffer(0)]],device const uint* s [[buffer(1)]],device const uint* h [[buffer(2)]],device const uint* originals [[buffer(3)]],device const uint* rh [[buffer(4)]],device const uint2* domains [[buffer(5)]],device const uint2* entries [[buffer(6)]],device const uint* ranges [[buffer(7)]],device const uint* nodes [[buffer(8)]],device const uint2* exits [[buffer(9)]],device const uint* eh [[buffer(10)]],device uint* replacements [[buffer(11)]],device atomic_uint* status [[buffer(12)]],constant ReconstructParams& p [[buffer(13)]],uint i [[thread_position_in_grid]]) {
 if(atomic_load_explicit(status,memory_order_relaxed) || i>=eh[0])return;uint2 e=exits[i];
 if(e.x>=rh[0] || e.y>=ranges[2] || (i && exits[i-1u].x>=e.x)) {atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint2 entry=entries[e.x];if(entry.x>=ranges[0]) {atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uint2 domain=domains[entry.x];bool valid=domain.y>0u && domain.x<=ranges[2] && domain.y<=ranges[2]-min(domain.x,ranges[2]) && entry.y<domain.y && e.y>=domain.x && e.y-domain.x<domain.y && e.y-domain.x>=entry.y;
 uint original=originals[e.x],task=s[original],node=nodes[e.y];View v=view(data);uint lo=0u,hi=p.spans;
 while(lo<hi) {uint mid=lo+(hi-lo)/2u;if(v.spans[v.semantic_order[mid]].semantic_start<=task)lo=mid+1u;else hi=mid;}
 File file=v.files[v.evaluations[v.spans[v.semantic_order[lo-1u]].evaluation].file];valid &= node>=file.node_start && node-file.node_start<file.node_count;
 // A cell address has one resident meaning: the entry's sibling offset plus
 // the closure distance. File membership alone cannot establish that meaning.
 uint source=rcol(s,h[0],2u,original),parent=v.nodes[source].parent;
 if(valid && parent==NONE)valid=node==source && domain.y==1u && entry.y==0u;
 else if(valid) {
  Node pnode=v.nodes[parent];uint low=0u,high=pnode.child_count;
  while(low<high) {uint mid=low+(high-low)/2u;if(v.children[pnode.child_start+mid]<source)low=mid+1u;else high=mid;}
  uint distance=e.y-domain.x-entry.y;
  valid=low<pnode.child_count && v.children[pnode.child_start+low]==source && low==entry.y && domain.y==pnode.child_count && distance<pnode.child_count-low;
  if(valid)valid=node==v.children[pnode.child_start+low+distance];
 }
 if(!valid) {atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 replacements[original]=i+1u;
}
kernel void reconstruct_counts(device const uchar* data [[buffer(0)]],device const uint* s [[buffer(1)]],device const uint* h [[buffer(2)]],device const uint* spans [[buffer(3)]],device const uint* replacements [[buffer(4)]],device const uint* categories [[buffer(5)]],device const uint2* proof [[buffer(6)]],device uint4* counts [[buffer(7)]],device atomic_uint* status [[buffer(8)]],constant ReconstructParams& p [[buffer(9)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.capacity)return;counts[i]=uint4(0u);if(atomic_load_explicit(status,memory_order_relaxed) || i>=h[0])return;
 uint n=h[0],replacement=replacements[i];
 if(categories[i]==5u) {View v=view(data);Program pg=v.programs[rcol(s,n,4u,i)];if(rcol(s,n,5u,i)!=0u || proof[pg.structural_start+rcol(s,n,1u,i)].y!=1u) {atomic_store_explicit(status,2u,memory_order_relaxed);return;}}
 if(replacement==NONE)return;
 ulong digits=ulong(rcol(spans,n,1u,i))+rcol(spans,n,3u,i)+rcol(spans,n,5u,i)+uint(replacement!=0u);
 counts[i]=uint4(1u,uint(min(digits,0xfffffffful)),0u,0u);
}
kernel void reconstruct_boundary(device const uint* h [[buffer(0)]],device const uint* values [[buffer(1)]],device const uint4* total [[buffer(2)]],device uint* output [[buffer(3)]],device uint* ov [[buffer(4)]],device uint* control [[buffer(5)]],device uint* status [[buffer(6)]],constant ReconstructParams& p [[buffer(7)]],uint i [[thread_position_in_grid]]) {
 if(i || status[0])return;
 if(total[0].x>p.output || total[0].y>p.digits || total[0].x==NONE || total[0].y==NONE || values[0]>p.cursors || values[1]>p.repeats || h[1]>p.captures) {status[0]=1u;return;}
 output[0]=total[0].x;output[1]=h[1];output[2]=rgroups(total[0].x);output[3]=output[4]=1u;ov[0]=values[0];ov[1]=values[1];control[0]=total[0].y;
}
void reconstruct_records(device const uchar* data,device const uint* s,device const uint* h,device const uint* spans,device const uint* replacements,device const uint2* exits,device const uint* nodes,device const uint4* offsets,device const uint* counts,device const ClosureProgress* progress,device uint* output,device uint* osp,device const uint* status,constant ReconstructParams& p,uint i) {
 if(status[0] || i>=h[0] || replacements[i]==NONE)return;
 uint n=h[0],m=counts[0],at=offsets[i].x,replacement=replacements[i];
 for(uint k=0u;k<12u;k++)output[k*m+at]=s[k*n+i];
 if(replacement) {View v=view(data);Program pg=v.programs[rcol(s,n,4u,i)];ClosureProgress state=progress[i];output[m+at]=state.pc;output[2u*m+at]=state.node;output[5u*m+at]=state.kind;uint op=v.words[pg.structural_start+state.pc]&255u;output[3u*m+at]=op==17u?1u:(op==7u?2u:(op==8u?3u:0u));}
 uint start=offsets[i].y;
 for(uint k=0u;k<3u;k++) {uint length=rcol(spans,n,k*2u+1u,i)+uint(k==0u && replacement!=0u);osp[k*2u*m+at]=start;osp[(k*2u+1u)*m+at]=length;start+=length;}
}
void reconstruct_digits(device const uint* h,device const uint* spans,device const uint* digits,device const uint* replacements,device const uint4* offsets,device const uint* count,device const ClosureProgress* progress,device uint* output,device const uint* status,constant ReconstructParams& p,uint tid) {
 if(status[0] || tid>=count[0])return;uint n=h[0],lo=0u,hi=n;
 while(lo<hi) {uint mid=lo+(hi-lo)/2u;if(offsets[mid].y<=tid)lo=mid+1u;else hi=mid;}
 uint i=lo-1u,entry=tid-offsets[i].y;
 for(uint k=0u;k<3u;k++) {uint old=rcol(spans,n,k*2u+1u,i),length=old+uint(k==0u && replacements[i]!=0u);if(entry<length) {output[tid]=entry<old?digits[rcol(spans,n,k*2u,i)+entry]:progress[i].choice;return;}entry-=length;}
}
void reconstruct_values(device const uint* h,device const uint* values,device const uint4* cursors,device const uint4* repeats,device const uint4* captures,device uint4* oc,device uint4* orepeat,device uint4* ocapture,device const uint* status,constant ReconstructParams& p,uint i) {
 if(status[0])return;if(i<values[0])oc[i]=cursors[i];if(i<values[1])orepeat[i]=repeats[i];if(i<h[1])ocapture[i]=captures[i];
}

// These copies consume the same checked boundary and do not consume each other.
kernel void reconstruct_publish(
 device const uchar* data [[buffer(0)]],device const uint* s [[buffer(1)]],
 device const uint* h [[buffer(2)]],device const uint* spans [[buffer(3)]],
 device const uint* replacements [[buffer(4)]],device const uint2* exits [[buffer(5)]],
 device const uint* nodes [[buffer(6)]],device const uint4* offsets [[buffer(7)]],
 device const uint* counts [[buffer(8)]],device uint* output [[buffer(9)]],
 device uint* osp [[buffer(10)]],device const uint* status [[buffer(11)]],
 device const uint* digits [[buffer(12)]],device const uint* digit_count [[buffer(13)]],
 device uint* odigits [[buffer(14)]],device const uint* values [[buffer(15)]],
 device const uint4* cursors [[buffer(16)]],device const uint4* repeats [[buffer(17)]],
 device const uint4* captures [[buffer(18)]],device uint4* ocursors [[buffer(19)]],
 device uint4* orepeats [[buffer(20)]],device uint4* ocaptures [[buffer(21)]],
 device const ClosureProgress* progress [[buffer(22)]],device uint* output_work [[buffer(23)]],constant ReconstructParams& p [[buffer(24)]],uint i [[thread_position_in_grid]]) {
 reconstruct_records(data,s,h,spans,replacements,exits,nodes,offsets,counts,progress,output,osp,status,p,i);
 if(!status[0] && i<h[0] && replacements[i]!=NONE)output_work[offsets[i].x]=progress[i].work;
 reconstruct_digits(h,spans,digits,replacements,offsets,digit_count,progress,odigits,status,p,i);
 reconstruct_values(h,values,cursors,repeats,captures,ocursors,orepeats,ocaptures,status,p,i);
}
