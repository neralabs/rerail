// Saved output ranks belong to the immutable ordered row arena, not raw ordinals.
struct PageParams {
 uint n,c,properties,bytes,typed,cursor,file_end,row_limit;
 uint capture_limit,property_limit,byte_limit,span,lane,rank,encoded,words,headers;
 uint semantic_spans,complex_rows,complex_captures,complex_properties;
};
ColumnView page_columns(device const uint* words,device const uint2* heads,
 constant PageParams& p,uint arena,device uint* header,device const uint4* prefixes,
 device const uint4* segments,device const uint* complex_words) {
 uint4 counts=uint4(p.n,p.c,p.properties,p.typed?p.n:0u);uint first=0u;
 for(uint k=0u;k<arena;k++){uint fields=k==0u?10u:(k==1u?9u:8u);first+=((counts[k]+255u)/256u)*fields;}
 uint fields=arena==0u?10u:(arena==1u?9u:(arena==2u?8u:4u));
 ColumnView v=literal_columns(words,heads,counts[arena],fields,first,p.headers,p.words,p.encoded,(device atomic_uint*)(header+9));
 v.prefixes=prefixes;v.segments=segments;v.complex_words=complex_words;v.spans=p.semantic_spans;v.arena=arena;
 v.complex_rows=arena==0u || arena==3u?p.complex_rows:(arena==1u?p.complex_captures:p.complex_properties);return v;
}
uint page_lower(ColumnView rows,uint n,uint task) {
 uint lo=0u,hi=n;while(lo<hi){uint mid=lo+(hi-lo)/2u;if(column_value(rows,mid,0u)<task)lo=mid+1u;else hi=mid;}return lo;
}
uint page_span(View v,uint task) {
 uint lo=0u,hi=v.header->span_count;
 while(lo<hi){uint mid=lo+(hi-lo)/2u;if(v.spans[v.semantic_order[mid]].semantic_start<=task)lo=mid+1u;else hi=mid;}
 if(!lo)return NONE;uint position=lo-1u;Span s=v.spans[v.semantic_order[position]];
 return task-s.semantic_start<s.root_count?position:NONE;
}
uint4 page_layout(ColumnView layout,constant PageParams& p,uint i) {
 return p.typed?uint4(column_value(layout,i,0u),column_value(layout,i,1u),column_value(layout,i,2u),column_value(layout,i,3u)):uint4(0u);
}
uint page_file(View v,ColumnView rows,uint i) {
 uint position=page_span(v,column_value(rows,i,0u));return position==NONE?NONE:v.evaluations[v.spans[v.semantic_order[position]].evaluation].file;
}
void page_finish(View v,ColumnView rows,constant PageParams& p,
 device uint* header,uint count,uint4 totals,uint status) {
 uint cursor=p.cursor+count;
  uint span=v.header->span_count,block_lane=0u,rank=0u,file=NONE;
  if(cursor<p.n){span=page_span(v,column_value(rows,cursor,0u));
   if(span==NONE)status=2u;else{Span s=v.spans[v.semantic_order[span]];block_lane=s.lane_start+column_value(rows,cursor,0u)-s.semantic_start;
    rank=cursor-page_lower(rows,p.n,column_value(rows,cursor,0u));file=v.evaluations[s.evaluation].file;}
  }
  if(!count && cursor<p.n && file<p.file_end && !status)status=1u;
  header[0]=totals.x;header[1]=totals.y;header[2]=totals.z;header[3]=totals.w;
  header[4]=cursor;header[5]=span;header[6]=block_lane;header[7]=rank;header[8]=file;atomic_store_explicit((device atomic_uint*)(header+9),max(status,atomic_load_explicit((device atomic_uint*)(header+9),memory_order_relaxed)),memory_order_relaxed);
}
kernel void output_page_count(
 device const uchar* data [[buffer(0)]],device const uint* row_words [[buffer(1)]],
 device const uint* layout_words [[buffer(2)]],device uint4* prefixes [[buffer(3)]],
 device uint* header [[buffer(4)]],device uint4* group_counts [[buffer(5)]],
 device uint* group_selected [[buffer(6)]],device const uint2* heads [[buffer(7)]],
 device const uint4* semantic_prefixes [[buffer(8)]],device const uint4* semantic_segments [[buffer(9)]],
 device const uint* complex_rows [[buffer(10)]],device const uint* complex_layout [[buffer(11)]],constant PageParams& p [[buffer(12)]],
 uint lane [[thread_index_in_threadgroup]],uint group [[threadgroup_position_in_grid]]) {
 threadgroup ulong4 sums[256];threadgroup atomic_uint selected;threadgroup atomic_uint failure;
 View v=view(data);ColumnView rows=page_columns(row_words,heads,p,0u,header,semantic_prefixes,semantic_segments,complex_rows),layout=page_columns(layout_words,heads,p,3u,header,semantic_prefixes,semantic_segments,complex_layout);uint candidates=min(p.row_limit,p.n-p.cursor),base=group*16384u;
 uint local_count=min(16384u,candidates-base),width=(local_count+255u)/256u;
 if(!lane){
  atomic_store_explicit(&selected,local_count,memory_order_relaxed);atomic_store_explicit(&failure,0u,memory_order_relaxed);
  bool valid=p.row_limit && p.row_limit<=16777216u && p.cursor<=p.n && p.span<=v.header->span_count;
  if(valid && p.span==v.header->span_count)valid=p.cursor==p.n && !p.lane && !p.rank;
  else if(valid){Span span=v.spans[v.semantic_order[p.span]];
   valid=p.lane>=span.lane_start && p.lane-span.lane_start<span.root_count;
   if(valid){uint task=span.semantic_start+p.lane-span.lane_start,first=page_lower(rows,p.n,task);
    valid=ulong(first)+p.rank==p.cursor && (!p.rank || (p.cursor && column_value(rows,p.cursor-1u,0u)==task));
    if(valid && p.cursor<p.n)valid=column_value(rows,p.cursor,0u)>=task;
   }
  }
  if(!valid && !group)atomic_store_explicit(&failure,2u,memory_order_relaxed);
 }
 threadgroup_barrier(mem_flags::mem_threadgroup);
 ulong4 local=ulong4(0ul);uint first=lane*width,last=min(first+width,local_count);
 for(uint i=first;i<last;i++){
  uint source=p.cursor+base+i;Result row=column_result(rows,source);uint4 l=page_layout(layout,p,source);
  ulong cap_end=ulong(row.capture_start)+row.capture_count,prop_end=ulong(l.z)+l.x,byte_end=ulong(l.w)+l.y;
  bool valid=page_span(v,row.task_ordinal)!=NONE && cap_end<=p.c && prop_end<=p.properties && byte_end<=p.bytes;
  if(!source)valid=valid && !row.capture_start && !l.z && !l.w;
  if(source+1u<p.n){uint4 next=page_layout(layout,p,source+1u);valid=valid && cap_end==column_value(rows,source+1u,4u) && prop_end==next.z && byte_end==next.w;}
  else valid=valid && cap_end==p.c && prop_end==p.properties && byte_end==p.bytes;
  if(source)valid=valid && column_value(rows,source-1u,0u)<=row.task_ordinal;
  if(!valid)atomic_store_explicit(&failure,2u,memory_order_relaxed);
  if(page_file(v,rows,source)>=p.file_end)atomic_fetch_min_explicit(&selected,i,memory_order_relaxed);
  local+=ulong4(1ul,row.capture_count,l.x,l.y);
 }
 sums[lane]=local;threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint distance=1u;distance<256u;distance*=2u){
  ulong4 value=lane>=distance?sums[lane-distance]:ulong4(0ul);
  threadgroup_barrier(mem_flags::mem_threadgroup);sums[lane]+=value;threadgroup_barrier(mem_flags::mem_threadgroup);
 }
 ulong4 at=lane?sums[lane-1u]:ulong4(0ul),limits=ulong4(p.row_limit,p.capture_limit,p.property_limit,p.byte_limit);
 for(uint i=first;i<last;i++){
  uint source=p.cursor+base+i;Result row=column_result(rows,source);uint4 l=page_layout(layout,p,source);
  prefixes[base+i]=uint4(min(at,ulong4(0xfffffffful)));
  at+=ulong4(1ul,row.capture_count,l.x,l.y);
  if(any(at>limits))atomic_fetch_min_explicit(&selected,i,memory_order_relaxed);
 }
 if(lane==255u && base+local_count==candidates)prefixes[candidates]=uint4(min(sums[255],ulong4(0xfffffffful)));
 threadgroup_barrier(mem_flags::mem_threadgroup|mem_flags::mem_device);
 if(!lane){
  group_counts[group]=uint4(min(sums[255],ulong4(0xfffffffful)));
  group_selected[group]=atomic_load_explicit(&selected,memory_order_relaxed);
  uint failure_code=atomic_load_explicit(&failure,memory_order_relaxed);
  if(any(sums[255]>ulong4(0xfffffffful)))failure_code=2u;
  if(failure_code)atomic_store_explicit((device atomic_uint*)(header+9),failure_code,memory_order_relaxed);
  if(candidates<=16384u){
   uint count=atomic_load_explicit(&selected,memory_order_relaxed);
   group_counts[0]=uint4(0u);
   page_finish(v,rows,p,header,count,prefixes[count],failure_code);
  }
 }
}
uint4 page_offset(device const uint4* offsets,device const uint4* groups,uint i) {
 return offsets[i]+groups[i/16384u];
}
uint4 page_prefix(device const uint4* offsets,device const uint4* groups,device const uint4* total,uint n,uint i) {
 return i==n?total[0]:page_offset(offsets,groups,i);
}
kernel void output_page_boundary(
 device const uchar* data [[buffer(0)]],device const uint* row_words [[buffer(1)]],
 device const uint4* offsets [[buffer(2)]],device uint4* groups [[buffer(3)]],
 device uint4* total [[buffer(4)]],device const uint* group_selected [[buffer(5)]],
 device uint* header [[buffer(6)]],device const uint2* heads [[buffer(7)]],
 device const uint4* semantic_prefixes [[buffer(8)]],device const uint4* semantic_segments [[buffer(9)]],
 device const uint* complex_rows [[buffer(10)]],constant PageParams& p [[buffer(11)]],uint lane [[thread_index_in_threadgroup]]) {
 // PageBounds permits at most 1024 count groups. Each scan lane owns at
 // most four group totals; the broad count stage still owns all row work.
 if(!p.row_limit || p.row_limit>16777216u || p.cursor>p.n){
  if(!lane)atomic_store_explicit((device atomic_uint*)(header+9),2u,memory_order_relaxed);return;
 }
 threadgroup ulong4 sums[256];threadgroup atomic_uint selected;
 uint candidates=min(p.row_limit,p.n-p.cursor),group_count=(candidates+16383u)/16384u;
 compact_cooperative(groups,total,group_count,lane,sums);
 // The boundary searches consume device offsets written by every scan lane.
 threadgroup_barrier(mem_flags::mem_threadgroup|mem_flags::mem_device);
 if(!lane)atomic_store_explicit(&selected,candidates,memory_order_relaxed);
 threadgroup_barrier(mem_flags::mem_threadgroup);
 uint4 limits=uint4(p.row_limit,p.capture_limit,p.property_limit,p.byte_limit);
 for(uint group=lane;group<group_count;group+=256u){
  uint first=group*16384u,lo=0u,hi=min(16384u,candidates-first);
  while(lo<hi){uint mid=lo+(hi-lo+1u)/2u;
   if(all(page_prefix(offsets,groups,total,candidates,first+mid)<=limits))lo=mid;else hi=mid-1u;
  }
  uint accepted=min(lo,group_selected[group]),count=min(16384u,candidates-first);
  if(accepted<count)atomic_fetch_min_explicit(&selected,first+accepted,memory_order_relaxed);
 }
 threadgroup_barrier(mem_flags::mem_threadgroup);
 if(!lane){
  View v=view(data);ColumnView rows=page_columns(row_words,heads,p,0u,header,semantic_prefixes,semantic_segments,complex_rows);uint count=atomic_load_explicit(&selected,memory_order_relaxed);
  uint4 totals=page_prefix(offsets,groups,total,candidates,count);uint status=header[9];
  page_finish(v,rows,p,header,count,totals,status);
 }
}
uint page_owner(device const uint4* offsets,device const uint4* groups,uint n,uint component,uint index) {
 uint lo=0u,hi=n;while(lo<hi){uint mid=lo+(hi-lo)/2u;if(page_offset(offsets,groups,mid)[component]<=index)lo=mid+1u;else hi=mid;}return lo?lo-1u:NONE;
}
bool page_range(uint start,uint length,uint base,uint count) {
 return start>=base && ulong(start)+length<=ulong(base)+count;
}
kernel void output_page_scatter(
 device const uint* row_words [[buffer(0)]],device const uint* cap_words [[buffer(1)]],
 device const uint* property_words [[buffer(2)]],device const uchar* bytes [[buffer(3)]],
 device const uint* layout_words [[buffer(4)]],device const uint4* offsets [[buffer(5)]],
 device uint* header [[buffer(6)]],device Result* out_rows [[buffer(7)]],
 device Capture* out_caps [[buffer(8)]],device Property* out_properties [[buffer(9)]],
 device uchar* out_bytes [[buffer(10)]],device const uint4* groups [[buffer(11)]],device const uint2* heads [[buffer(12)]],
 device const uint4* semantic_prefixes [[buffer(13)]],device const uint4* semantic_segments [[buffer(14)]],
 device const uint* complex_rows [[buffer(15)]],device const uint* complex_captures [[buffer(16)]],
 device const uint* complex_properties [[buffer(17)]],device const uint* complex_layout [[buffer(18)]],
 constant PageParams& p [[buffer(19)]],uint i [[thread_position_in_grid]]) {
 // Count completed and the host validated its scalar header before this dispatch.
 ColumnView rows=page_columns(row_words,heads,p,0u,header,semantic_prefixes,semantic_segments,complex_rows),caps=page_columns(cap_words,heads,p,1u,header,semantic_prefixes,semantic_segments,complex_captures),properties=page_columns(property_words,heads,p,2u,header,semantic_prefixes,semantic_segments,complex_properties),layout=page_columns(layout_words,heads,p,3u,header,semantic_prefixes,semantic_segments,complex_layout);
 uint n=header[0];
 if(i<n){Result row=column_result(rows,p.cursor+i);uint4 at=page_offset(offsets,groups,i),l=page_layout(layout,p,p.cursor+i);
  if((row.property_count && !page_range(row.property_start,row.property_count,l.z,l.x)) ||
     (row.dynamic_count && !page_range(row.dynamic_start,row.dynamic_count,l.z,l.x))){atomic_store_explicit((device atomic_uint*)(header+9),2u,memory_order_relaxed);return;}
  row.capture_start=at.y;row.property_start=row.property_count?row.property_start-l.z+at.z:0u;
  row.dynamic_start=row.dynamic_count?row.dynamic_start-l.z+at.z:0u;out_rows[i]=row;
 }
 if(i<header[1]){uint owner=page_owner(offsets,groups,n,1u,i);uint4 at=page_offset(offsets,groups,owner),l=page_layout(layout,p,p.cursor+owner);Result row=column_result(rows,p.cursor+owner);
  uint source=row.capture_start+i-at.y;Capture cap=column_capture(caps,source);
  if(!page_range(source,1u,row.capture_start,row.capture_count) ||
     (cap.property_count && !page_range(cap.property_start,cap.property_count,l.z,l.x)) ||
     (cap.text_start!=NONE && !page_range(cap.text_start,cap.text_length,l.w,l.y))){atomic_store_explicit((device atomic_uint*)(header+9),2u,memory_order_relaxed);return;}
  cap.property_start=cap.property_count?cap.property_start-l.z+at.z:0u;
  if(cap.text_start!=NONE)cap.text_start=cap.text_start-l.w+at.w;out_caps[i]=cap;
 }
 if(i<header[2]){uint owner=page_owner(offsets,groups,n,2u,i);uint4 at=page_offset(offsets,groups,owner),l=page_layout(layout,p,p.cursor+owner);Result row=column_result(rows,p.cursor+owner);
  Property prop=column_property(properties,l.z+i-at.z);bool valid=prop.kind<=2u;
  if(prop.kind==1u){valid=valid && page_range(prop.owner,1u,row.capture_start,row.capture_count);prop.owner=prop.owner-row.capture_start+at.y;}
  else {valid=valid && prop.owner==p.cursor+owner;prop.owner=owner;}
  if(prop.kind==2u){valid=valid && prop.value<=1u && page_range(prop.key_start,prop.key_length,l.w,l.y) && (!prop.value || page_range(prop.value_start,prop.value_length,l.w,l.y));
   prop.key_start=prop.key_start-l.w+at.w;if(prop.value)prop.value_start=prop.value_start-l.w+at.w;}
  if(!valid){atomic_store_explicit((device atomic_uint*)(header+9),2u,memory_order_relaxed);return;}out_properties[i]=prop;
 }
 if(i<header[3]){uint owner=page_owner(offsets,groups,n,3u,i);uint4 at=page_offset(offsets,groups,owner),l=page_layout(layout,p,p.cursor+owner);out_bytes[i]=bytes[l.w+i-at.w];}
}
