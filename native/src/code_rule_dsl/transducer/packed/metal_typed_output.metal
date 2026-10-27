// All offsets remain device-owned until exact final arena counts are available.
struct OutputParams {
 uint n,c,step,max_ops,width,source_n,source_c,result_base,capture_base;
 uint dynamic_slots,transform_bytes;
};
uint mime_value_length(uint4 snapshot);
uchar mime_byte(View v,uint4 snapshot,uint at);
uint output_span(View v,uint task) {
 uint lo=0u,hi=v.header->span_count;
 while(lo<hi){uint mid=lo+(hi-lo)/2u;if(v.spans[v.semantic_order[mid]].semantic_start<=task)lo=mid+1u;else hi=mid;}
 if(!lo)return NONE;uint id=v.semantic_order[lo-1u];Span span=v.spans[id];
 return task-span.semantic_start<span.root_count?id:NONE;
}
uint output_row(device const Result* rows,uint n,uint capture) {
 uint lo=0u,hi=n;while(lo<hi){uint mid=lo+(hi-lo)/2u;if(rows[mid].capture_start<=capture)lo=mid+1u;else hi=mid;}
 return lo?lo-1u:NONE;
}
uint output_line(View v,File f,uint byte) {
 uint lo=0u,hi=f.line_count;while(lo<hi){uint mid=lo+(hi-lo)/2u;if(v.line_starts[f.line_start+mid]<=byte)lo=mid+1u;else hi=mid;}
 return lo-1u;
}
uint output_byte(View v,File f,long row,long column) {
 if(row<0l || ulong(row)>=f.line_count || column<0l)return NONE;
 uint start=v.line_starts[f.line_start+uint(row)];
 uint end=uint(row)+1u<f.line_count?v.line_starts[f.line_start+uint(row)+1u]-1u:f.source_start+f.source_length;
 if(ulong(column)>end-start)return NONE;uint byte=start+uint(column);
 return byte==f.source_start+f.source_length || (v.source[byte]&0xc0u)!=0x80u?byte:NONE;
}
uint4 output_prefix(device const uint4* offsets,device const uint4* total,uint n,uint at) {
 return at<n?offsets[at]:total[0];
}
bool output_kept(device const uint4* offsets,device const uint4* total,uint n,uint at) {
 return output_prefix(offsets,total,n,at+1u).x==offsets[at].x+1u;
}
kernel void output_initialize_results(device const uchar* data [[buffer(0)]],device const Result* rows [[buffer(1)]],device uint4* meta [[buffer(2)]],device uint* ids [[buffer(3)]],device const uint* failures [[buffer(4)]],device atomic_uint* status [[buffer(5)]],device atomic_uint* programs [[buffer(6)]],constant OutputParams& p [[buffer(7)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.n)return;ids[i]=i;meta[i]=uint4(0u);
 if(atomic_load_explicit(status,memory_order_relaxed))return;
 View v=view(data);Result row=rows[i];uint span_id=output_span(v,row.task_ordinal);
 if(span_id==NONE){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 Span span=v.spans[span_id];uint evaluation=span.evaluation;
 if(evaluation>=v.header->evaluation_count){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 Evaluation e=v.evaluations[evaluation];Program pg=v.programs[e.program];File f=v.files[e.file];
 Root root=v.roots[select_root(v,v.selectors[e.selector],span.selector_offset+row.task_ordinal-span.semantic_start)];
 ulong end=ulong(row.capture_start)+row.capture_count;
 bool valid=e.program<v.header->program_count && row.label<pg.label_count && root.file==e.file && row.start==f.node_start+root.node
  && end<=p.c && (i || !row.capture_start)
  && (i+1u<p.n?end==rows[i+1u].capture_start:end==p.c) && !row.property_start && !row.property_count && !row.dynamic_start && !row.dynamic_count && !failures[evaluation];
 if(!valid){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 meta[i]=uint4(evaluation,e.program,e.file,1u);
 atomic_store_explicit(programs+e.program,1u,memory_order_relaxed);
}
kernel void output_initialize_captures(device const uchar* data [[buffer(0)]],device const Result* rows [[buffer(1)]],device const Capture* captures [[buffer(2)]],device const uint4* meta [[buffer(3)]],device uint* owners [[buffer(4)]],device atomic_uint* status [[buffer(5)]],constant OutputParams& p [[buffer(6)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.c)return;owners[i]=NONE;if(atomic_load_explicit(status,memory_order_relaxed))return;
 uint owner=output_row(rows,p.n,i);if(owner==NONE || i-rows[owner].capture_start>=rows[owner].capture_count){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 View v=view(data);uint4 m=meta[owner];Program pg=v.programs[m.y];File f=v.files[m.z];Capture cap=captures[i];
 bool valid=cap.label<pg.label_count && cap.node>=f.node_start && cap.node-f.node_start<f.node_count
  && cap.start>=f.source_start && cap.end>=cap.start && cap.end-f.source_start<=f.source_length
  && cap.text_start==NONE && !cap.text_length && !cap.property_start && !cap.property_count && cap.retained==1u;
 if(valid)valid=cap.start==v.nodes[cap.node].source_start && cap.end==v.nodes[cap.node].source_end;
 if(!valid){atomic_store_explicit(status,2u,memory_order_relaxed);return;}owners[i]=owner;
}
kernel void output_apply_results(device const uchar* data [[buffer(0)]],device Result* rows [[buffer(1)]],device const uint4* meta [[buffer(2)]],device uint* slots [[buffer(3)]],device const uint* status [[buffer(4)]],constant OutputParams& p [[buffer(5)]],uint i [[thread_position_in_grid]]) {
 if(status[0] || i>=p.n || !meta[i].w)return;View v=view(data);Program pg=v.programs[meta[i].y];if(p.step>=pg.output_count)return;
 device const uint* op=v.output_words+pg.output_start+p.step*6u;
 if(op[0]==0u)slots[i*p.max_ops+p.step]=rows[i].property_count++;
}
kernel void output_apply_captures(device const uchar* data [[buffer(0)]],device const Result* rows [[buffer(1)]],device Capture* captures [[buffer(2)]],device uint* meta [[buffer(3)]],device const uint* owners [[buffer(4)]],device uint4* text [[buffer(5)]],device uint* slots [[buffer(6)]],device const uint* status [[buffer(7)]],constant OutputParams& p [[buffer(8)]],uint i [[thread_position_in_grid]]) {
 if(status[0] || i>=p.c)return;uint owner=owners[i];
 if(!atomic_load_explicit((device atomic_uint*)(meta+owner*4u+3u),memory_order_relaxed))return;
 View v=view(data);Program pg=v.programs[meta[owner*4u+1u]];if(p.step>=pg.output_count)return;
 device const uint* op=v.output_words+pg.output_start+p.step*6u;
 if(op[0]==0u || op[1]!=captures[i].label || !captures[i].retained)return;
 if(op[0]==1u)slots[(p.n+i)*p.max_ops+p.step]=captures[i].property_count++;
 else if(op[0]==2u){File f=v.files[meta[owner*4u+2u]];uint start=captures[i].start,end=captures[i].end;
  uint sr=output_line(v,f,start),er=output_line(v,f,end);
  uint next_start=output_byte(v,f,long(sr)+as_type<int>(op[2]),long(start-v.line_starts[f.line_start+sr])+as_type<int>(op[3]));
  uint next_end=output_byte(v,f,long(er)+as_type<int>(op[4]),long(end-v.line_starts[f.line_start+er])+as_type<int>(op[5]));
  if(next_start==NONE || next_end==NONE || next_start>next_end)atomic_store_explicit((device atomic_uint*)(meta+owner*4u+3u),0u,memory_order_relaxed);
  else {captures[i].start=next_start;captures[i].end=next_end;}
 }else if(op[0]==5u){if(!text[i].z)text[i]=uint4(captures[i].start,captures[i].end-captures[i].start,1u,0u);else if(text[i].z==2u)text[i].z=3u;}
}
bool output_less(View v,device const Result* rows,device const Capture* caps,device const uint* firsts,device const uint4* meta,uint a,uint b) {
 if(rows[a].task_ordinal!=rows[b].task_ordinal)return rows[a].task_ordinal<rows[b].task_ordinal;
 if(v.programs[meta[a].y].flags&1u){uint left=firsts[a]==NONE?0u:caps[firsts[a]].node+1u,right=firsts[b]==NONE?0u:caps[firsts[b]].node+1u;if(left!=right)return left<right;}
 if(rows[a].local_ordinal!=rows[b].local_ordinal)return rows[a].local_ordinal<rows[b].local_ordinal;
 return a<b;
}
// Each tile preserves the complete post-transform comparator and raw-ID ties.
kernel void output_sort_tiles(device const Result* rows [[buffer(0)]],device const uint* input [[buffer(1)]],device uint* output [[buffer(2)]],device atomic_uint* status [[buffer(3)]],device const uchar* data [[buffer(4)]],device const Capture* caps [[buffer(5)]],device const uint* firsts [[buffer(6)]],device const uint4* meta [[buffer(7)]],constant OutputParams& p [[buffer(8)]],uint group [[threadgroup_position_in_grid]],uint lane [[thread_index_in_threadgroup]]) {
 uint first=group*256u,at=first+lane;threadgroup uint ids[256];threadgroup atomic_uint invalid;
 if(!lane)atomic_store_explicit(&invalid,uint(atomic_load_explicit(status,memory_order_relaxed)!=0u || first>=p.n),memory_order_relaxed);
 threadgroup_barrier(mem_flags::mem_threadgroup);if(atomic_load_explicit(&invalid,memory_order_relaxed))return;
 ids[lane]=at<p.n?input[at]:NONE;threadgroup_barrier(mem_flags::mem_threadgroup);
 if(at<p.n && ids[lane]>=p.n){atomic_store_explicit(&invalid,1u,memory_order_relaxed);atomic_store_explicit(status,2u,memory_order_relaxed);}
 threadgroup_barrier(mem_flags::mem_threadgroup);if(atomic_load_explicit(&invalid,memory_order_relaxed))return;
 View v=view(data);
 for(uint width=2u;width<=256u;width*=2u)for(uint distance=width/2u;distance;distance/=2u){
  uint other=lane^distance,a=ids[lane],b=ids[other];
  bool less=b!=NONE && (a==NONE || output_less(v,rows,caps,firsts,meta,b,a));
  bool ascending=(lane&width)==0u,take=((lane&distance)==0u)==ascending?less:!less;
  uint selected=take?b:a;threadgroup_barrier(mem_flags::mem_threadgroup);ids[lane]=selected;threadgroup_barrier(mem_flags::mem_threadgroup);
 }
 if(at<p.n)output[at]=ids[lane];
}
kernel void output_merge(device const Result* rows [[buffer(0)]],device const uint* ids [[buffer(1)]],device uint* output [[buffer(2)]],device const uint* status [[buffer(3)]],device const uchar* data [[buffer(4)]],device const Capture* caps [[buffer(5)]],device const uint* firsts [[buffer(6)]],device const uint4* meta [[buffer(7)]],constant OutputParams& p [[buffer(8)]],uint i [[thread_position_in_grid]]) {
 if(status[0] || i>=p.n)return;
 uint base=(i/p.width/2u)*p.width*2u,middle=uint(min(ulong(p.n),ulong(base)+p.width)),end=uint(min(ulong(p.n),ulong(base)+2ul*p.width));
 bool left=i<middle;uint begin=left?middle:base,lo=begin,hi=left?end:middle,a=ids[i];
 View v=view(data);while(lo<hi){uint mid=lo+(hi-lo)/2u;if(output_less(v,rows,caps,firsts,meta,ids[mid],a))lo=mid+1u;else hi=mid;}
 output[base+(i-(left?base:middle))+(lo-begin)]=a;
}
kernel void output_capture_sizes(device const Capture* caps [[buffer(0)]],device const uint4* meta [[buffer(1)]],device const uint* owners [[buffer(2)]],device const uint4* text [[buffer(3)]],device uint4* sizes [[buffer(4)]],device const uint* status [[buffer(5)]],device atomic_uint* firsts [[buffer(6)]],constant OutputParams& p [[buffer(7)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.c)return;sizes[i]=uint4(0u);if(status[0] || !meta[owners[i]].w || !caps[i].retained)return;
 sizes[i]=uint4(caps[i].property_count,text[i].z?text[i].y:0u,0u,0u);
 atomic_fetch_min_explicit(firsts+owners[i],i,memory_order_relaxed);
}
uint4 output_dynamic_prefix(uint owner,device const uint4* offsets,device const uint4* total,constant OutputParams& p,bool end){
 if(!p.dynamic_slots)return uint4(0u);
 return output_prefix(offsets,total,p.dynamic_slots,(owner+uint(end))*p.max_ops);
}
uint4 output_sizes(Result row,uint owner,device const uint4* offsets,device const uint4* total,device const uint4* dynamic_offsets,device const uint4* dynamic_total,constant OutputParams& p){
 uint4 first=output_prefix(offsets,total,p.c,row.capture_start),last=output_prefix(offsets,total,p.c,row.capture_start+row.capture_count);
 uint4 dynamic=output_dynamic_prefix(owner,dynamic_offsets,dynamic_total,p,true)-output_dynamic_prefix(owner,dynamic_offsets,dynamic_total,p,false);
 ulong properties=ulong(row.property_count)+(last.x-first.x)+row.dynamic_count,bytes=ulong(last.y-first.y)+dynamic.y;
 return uint4(1u,row.capture_count,uint(min(properties,ulong(NONE))),uint(min(bytes,ulong(NONE))));
}
kernel void output_candidate_sizes(device const Result* rows [[buffer(0)]],device const uint4* meta [[buffer(1)]],device const uint* ids [[buffer(2)]],device const uint4* cap_offsets [[buffer(3)]],device const uint4* cap_total [[buffer(4)]],device uint4* sizes [[buffer(5)]],device uint* ranks [[buffer(6)]],device atomic_uint* status [[buffer(7)]],device const uint4* dynamic_offsets [[buffer(8)]],device const uint4* dynamic_total [[buffer(9)]],constant OutputParams& p [[buffer(10)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.n)return;sizes[i]=uint4(0u);ranks[ids[i]]=i;
 if(any(cap_total[0]==uint4(NONE)) || any(dynamic_total[0]==uint4(NONE))){atomic_store_explicit(status,1u,memory_order_relaxed);return;}
 if(atomic_load_explicit(status,memory_order_relaxed) || !meta[ids[i]].w)return;
 uint raw=ids[i];Result row=rows[raw];uint4 dynamic=output_dynamic_prefix(raw,dynamic_offsets,dynamic_total,p,true)-output_dynamic_prefix(raw,dynamic_offsets,dynamic_total,p,false);
 if(dynamic.x!=row.dynamic_count){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 sizes[i]=output_sizes(row,raw,cap_offsets,cap_total,dynamic_offsets,dynamic_total,p);
}
kernel void output_limit_sizes(device const uchar* data [[buffer(0)]],device const Result* rows [[buffer(1)]],device const uint4* meta [[buffer(2)]],device const uint* ids [[buffer(3)]],device const uint4* cap_offsets [[buffer(4)]],device const uint4* cap_total [[buffer(5)]],device const uint4* accepted [[buffer(6)]],device uint4* sizes [[buffer(7)]],device const uint* status [[buffer(8)]],device const uint4* dynamic_offsets [[buffer(9)]],device const uint4* dynamic_total [[buffer(10)]],constant OutputParams& p [[buffer(11)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.n)return;sizes[i]=uint4(0u);uint raw=ids[i];if(status[0] || !meta[raw].w)return;
 View v=view(data);Evaluation evaluation=v.evaluations[meta[raw].x];uint lo=0u,hi=i;
 while(lo<hi){uint mid=lo+(hi-lo)/2u;if(rows[ids[mid]].task_ordinal<evaluation.semantic_start)lo=mid+1u;else hi=mid;}
 if(accepted[i].x-accepted[lo].x<v.accepted_limits[evaluation.program])sizes[i]=output_sizes(rows[raw],raw,cap_offsets,cap_total,dynamic_offsets,dynamic_total,p);
}
void output_write_results(device const Result* rows,device const uint* ids,device const uint4* offsets,device const uint4* total,device Result* output,device const atomic_uint* status,device const uint4* cap_offsets,device const uint4* cap_total,device uint4* layout,constant OutputParams& p,uint i) {
 if(atomic_load_explicit(status,memory_order_relaxed) || i>=p.n || !output_kept(offsets,total,p.n,i))return;Result row=rows[ids[i]];uint4 at=offsets[i];
 uint cap_props=output_prefix(cap_offsets,cap_total,p.c,row.capture_start+row.capture_count).x-output_prefix(cap_offsets,cap_total,p.c,row.capture_start).x;
 row.capture_start=at.y;row.property_start=row.property_count?at.z:0u;row.dynamic_start=row.dynamic_count?at.z+row.property_count+cap_props:0u;output[at.x]=row;
 uint4 next=output_prefix(offsets,total,p.n,i+1u);layout[at.x]=uint4(next.z-at.z,next.w-at.w,at.z,at.w);
}
void output_write_captures(device const Result* rows,device const Capture* caps,device const uint* owners,device const uint* ranks,device const uint4* text,device const uint4* cap_offsets,device const uint4* cap_total,device const uint4* offsets,device const uint4* total,device Capture* output,device const atomic_uint* status,constant OutputParams& p,uint i) {
 if(atomic_load_explicit(status,memory_order_relaxed) || i>=p.c)return;uint owner=owners[i],rank=ranks[owner];if(!output_kept(offsets,total,p.n,rank))return;
 Result row=rows[owner];uint4 base=offsets[rank],first=output_prefix(cap_offsets,cap_total,p.c,row.capture_start);Capture cap=caps[i];
 if(!cap.retained){cap.property_count=0u;cap.text_start=NONE;cap.text_length=0u;cap.property_start=0u;output[base.y+i-row.capture_start]=cap;return;}
 cap.property_start=cap.property_count?base.z+row.property_count+(cap_offsets[i].x-first.x):0u;
 cap.text_start=text[i].z?base.w+(cap_offsets[i].y-first.y):NONE;cap.text_length=text[i].z?text[i].y:0u;
 output[base.y+i-row.capture_start]=cap;
}
void output_write_properties(device const uchar* data,device const Result* rows,device const Capture* caps,device const uint4* meta,device const uint* owners,device const uint* ranks,device const uint* slots,device const uint4* cap_offsets,device const uint4* cap_total,device const uint4* offsets,device const uint4* total,device Property* output,device const atomic_uint* status,device const uint4* snapshots,device const uint4* dynamic_offsets,device const uint4* dynamic_total,constant OutputParams& p,uint i) {
 if(atomic_load_explicit(status,memory_order_relaxed) || ulong(i)>=ulong(p.n+p.c)*p.max_ops)return;uint entity=i/p.max_ops,step=i%p.max_ops;
 bool match=entity<p.n;uint cap=match?0u:entity-p.n,owner=match?entity:owners[cap],rank=ranks[owner];
 if(!output_kept(offsets,total,p.n,rank))return;View v=view(data);Program pg=v.programs[meta[owner].y];if(step>=pg.output_count)return;
 device const uint* op=v.output_words+pg.output_start+step*6u;uint4 at=offsets[rank];
 if(match && op[0]==0u)output[at.z+slots[i]]={0u,at.x,op[1],op[2],0u,0u,0u,0u};
 if(match && op[0]==6u){uint slot=owner*p.max_ops+step;uint4 snapshot=snapshots[slot];if(!snapshot.w)return;
  Result row=rows[owner];uint4 cap_first=output_prefix(cap_offsets,cap_total,p.c,row.capture_start),cap_last=output_prefix(cap_offsets,cap_total,p.c,row.capture_start+row.capture_count);
  uint bytes=at.w+(cap_last.y-cap_first.y)+(dynamic_offsets[slot].y-output_dynamic_prefix(owner,dynamic_offsets,dynamic_total,p,false).y);
  output[at.z+row.property_count+(cap_last.x-cap_first.x)+snapshot.w-1u]={2u,at.x,0u,1u,bytes,8u,bytes+8u,mime_value_length(snapshot)};
 }
 if(!match && op[0]==1u && op[1]==caps[cap].label && caps[cap].retained){
  uint first=rows[owner].capture_start;uint local=cap_offsets[cap].x-output_prefix(cap_offsets,cap_total,p.c,first).x;
  output[at.z+rows[owner].property_count+local+slots[i]]={1u,at.y+cap-first,op[2],op[3],0u,0u,0u,0u};
 }
}
void output_write_bytes(device const uchar* data,device const Result* rows,device const uint* ids,device const uint4* text,device const uint4* cap_offsets,device const uint4* cap_total,device const uint4* offsets,device const uint4* total,device uchar* output,device atomic_uint* status,device const uint4* snapshots,device const uint4* dynamic_offsets,device const uint4* dynamic_total,device const uchar* transformed,constant OutputParams& p,uint i) {
 if(atomic_load_explicit(status,memory_order_relaxed) || i>=total[0].w)return;uint lo=0u,hi=p.n;
 while(lo<hi){uint mid=lo+(hi-lo)/2u;if(offsets[mid].w<=i)lo=mid+1u;else hi=mid;}
 uint rank=lo-1u,owner=ids[rank];Result row=rows[owner];uint4 cap_first=output_prefix(cap_offsets,cap_total,p.c,row.capture_start),cap_last=output_prefix(cap_offsets,cap_total,p.c,row.capture_start+row.capture_count);
 uint local=i-offsets[rank].w,cap_bytes=cap_last.y-cap_first.y;View v=view(data);
 if(local>=cap_bytes){uint target=local-cap_bytes+output_dynamic_prefix(owner,dynamic_offsets,dynamic_total,p,false).y;
  lo=owner*p.max_ops;hi=lo+p.max_ops;while(lo<hi){uint mid=lo+(hi-lo)/2u;if(dynamic_offsets[mid].y<=target)lo=mid+1u;else hi=mid;}
  uint slot=lo-1u;output[i]=mime_byte(v,snapshots[slot],target-dynamic_offsets[slot].y);return;
 }
 uint target=local+cap_first.y;
 lo=row.capture_start;hi=lo+row.capture_count;while(lo<hi){uint mid=lo+(hi-lo)/2u;if(cap_offsets[mid].y<=target)lo=mid+1u;else hi=mid;}
 uint cap=lo-1u;uint4 t=text[cap];uint at=target-cap_offsets[cap].y,bound=t.z>=2u?p.transform_bytes:v.header->source_bytes;
 if(!t.z || t.z>3u || t.w || t.x>bound || t.y>bound-t.x || at>=t.y){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 uchar byte=t.z>=2u?transformed[t.x+at]:v.source[t.x+at];
 output[i]=(t.z==1u || t.z==3u) && byte>='A' && byte<='Z'?byte+('a'-'A'):byte;
}

// All four typed columns consume completed offsets and immutable source arenas.
// Their destinations have separate owners, so no publication phase needs a barrier.
kernel void output_publish(device const uchar* data [[buffer(0)]],device const Result* rows [[buffer(1)]],device const Capture* caps [[buffer(2)]],device const uint4* meta [[buffer(3)]],device const uint* ids [[buffer(4)]],device const uint* owners [[buffer(5)]],device const uint* ranks [[buffer(6)]],device const uint4* text [[buffer(7)]],device const uint* slots [[buffer(8)]],device const uint4* cap_offsets [[buffer(9)]],device const uint4* cap_total [[buffer(10)]],device const uint4* offsets [[buffer(11)]],device const uint4* total [[buffer(12)]],device atomic_uint* status [[buffer(13)]],device const uint4* snapshots [[buffer(14)]],device const uint4* dynamic_offsets [[buffer(15)]],device const uint4* dynamic_total [[buffer(16)]],device const uchar* transformed [[buffer(17)]],device Result* out_rows [[buffer(18)]],device Capture* out_caps [[buffer(19)]],device Property* out_props [[buffer(20)]],device uchar* out_bytes [[buffer(21)]],device uint4* out_layout [[buffer(22)]],constant OutputParams& p [[buffer(23)]],uint i [[thread_position_in_grid]]) {
 output_write_results(rows,ids,offsets,total,out_rows,status,cap_offsets,cap_total,out_layout,p,i);
 output_write_captures(rows,caps,owners,ranks,text,cap_offsets,cap_total,offsets,total,out_caps,status,p,i);
 output_write_properties(data,rows,caps,meta,owners,ranks,slots,cap_offsets,cap_total,offsets,total,out_props,status,snapshots,dynamic_offsets,dynamic_total,p,i);
 output_write_bytes(data,rows,ids,text,cap_offsets,cap_total,offsets,total,out_bytes,status,snapshots,dynamic_offsets,dynamic_total,transformed,p,i);
}
