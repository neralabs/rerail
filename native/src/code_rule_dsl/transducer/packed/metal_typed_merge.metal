// Producers are complete task relations. Their task sets must be disjoint.
struct TypedMergeParams { uint an,ac,ap,ab,bn,bc,bp,bb,limited,ae,aw,ah,be,bw,bh; };
uint4 typed_counts(constant TypedMergeParams& p,bool second) {
 return second?uint4(p.bn,p.bc,p.bp,p.bb):uint4(p.an,p.ac,p.ap,p.ab);
}
ColumnView typed_column(device const uint* words,device const uint2* headers,constant TypedMergeParams& p,bool second,uint arena,device atomic_uint* status) {
 uint4 counts=typed_counts(p,second);uint first=0u;
 for(uint k=0u;k<arena;k++)first+=uint((ulong(counts[k])+255ul)/256ul)*(k==0u?10u:(k==1u?9u:8u));
 return literal_columns(words,headers,arena==3u?counts.x:counts[arena],arena==0u?10u:(arena==1u?9u:(arena==2u?8u:4u)),first,second?p.bh:p.ah,second?p.bw:p.aw,second?p.be:p.ae,status);
}
uint4 typed_layout(ColumnView v,uint row) {
 return uint4(column_value(v,row,0u),column_value(v,row,1u),column_value(v,row,2u),column_value(v,row,3u));
}
uint typed_lower(ColumnView rows,uint n,uint task) {
 uint lo=0u,hi=n;while(lo<hi){uint mid=lo+(hi-lo)/2u;if(column_value(rows,mid,0u)<task)lo=mid+1u;else hi=mid;}return lo;
}
uint typed_span(View v,uint task) {
 uint lo=0u,hi=v.header->span_count;
 while(lo<hi){uint mid=lo+(hi-lo)/2u;if(v.spans[v.semantic_order[mid]].semantic_start<=task)lo=mid+1u;else hi=mid;}
 if(!lo)return NONE;Span span=v.spans[v.semantic_order[lo-1u]];
 return task-span.semantic_start<span.root_count?v.semantic_order[lo-1u]:NONE;
}
bool typed_range(uint start,uint length,uint base,uint count) {
 return start>=base && ulong(start)+length<=ulong(base)+count;
}
kernel void typed_merge_counts(
 device const uchar* data [[buffer(0)]],device const uint* a [[buffer(1)]],device const uint* al [[buffer(2)]],
 device const uint* b [[buffer(3)]],device const uint* bl [[buffer(4)]],device const uint* failures [[buffer(5)]],
 device uint4* sizes [[buffer(6)]],device atomic_uint* status [[buffer(7)]],device const uint2* ah [[buffer(8)]],device const uint2* bh [[buffer(9)]],constant TypedMergeParams& p [[buffer(10)]],uint index [[thread_position_in_grid]]) {
 if(ulong(index)>=ulong(p.an)+p.bn)return;bool second=index>=p.an;uint i=second?index-p.an:index;
 ColumnView av=typed_column(a,ah,p,false,0u,status),bv=typed_column(b,bh,p,true,0u,status);
 ColumnView rows=second?bv:av,other=second?av:bv,layout=typed_column(second?bl:al,second?bh:ah,p,second,3u,status);
 uint4 counts=typed_counts(p,second);uint other_n=second?p.an:p.bn;
 Result row=column_result(rows,i);uint4 l=typed_layout(layout,i);uint before=typed_lower(other,other_n,row.task_ordinal),rank=i+before;
 sizes[rank]=uint4(0u);
 bool valid=before==other_n || column_value(other,before,0u)!=row.task_ordinal;
 valid=valid && (!i || column_value(rows,i-1u,0u)<=row.task_ordinal)
  && ulong(row.capture_start)+row.capture_count<=counts.y && ulong(l.z)+l.x<=counts.z && ulong(l.w)+l.y<=counts.w;
 if(!i)valid=valid && !row.capture_start && !l.z && !l.w;
 if(i+1u<counts.x)valid=valid && row.capture_start+row.capture_count==column_value(rows,i+1u,4u) && l.z+l.x==column_value(layout,i+1u,2u) && l.w+l.y==column_value(layout,i+1u,3u);
 else valid=valid && row.capture_start+row.capture_count==counts.y && l.z+l.x==counts.z && l.w+l.y==counts.w;
 View v=view(data);uint span=typed_span(v,row.task_ordinal);
 if(!valid || span==NONE){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
 Evaluation evaluation=v.evaluations[v.spans[span].evaluation];
 if(failures[v.spans[span].evaluation])return;
 uint first=typed_lower(av,p.an,evaluation.semantic_start)+typed_lower(bv,p.bn,evaluation.semantic_start);
 if(p.limited && rank-first>=v.accepted_limits[evaluation.program])return;
 sizes[rank]=uint4(1u,row.capture_count,l.x,l.y);
}
struct TypedSource {
 ColumnView rows,captures,properties;device const uchar* bytes;ColumnView layout,other;
 uint4 counts;uint other_n;
};
uint typed_owner(TypedSource s,uint component,uint index) {
 if(component==0u)return index;
 uint lo=0u,hi=s.counts.x;
 while(lo<hi){uint mid=lo+(hi-lo)/2u;uint start=component==1u?column_value(s.rows,mid,4u):column_value(s.layout,mid,component==2u?2u:3u);
  if(start<=index)lo=mid+1u;else hi=mid;}
 return lo?lo-1u:NONE;
}
uint4 typed_offset(device const uint4* offsets,device const uint4* total,uint n,uint rank) {
 return rank<n?offsets[rank]:total[0];
}
kernel void typed_merge_scatter(
 device const uint* ar [[buffer(0)]],device const uint* ac [[buffer(1)]],device const uint* ap [[buffer(2)]],device const uchar* ab [[buffer(3)]],device const uint* al [[buffer(4)]],
 device const uint* br [[buffer(5)]],device const uint* bc [[buffer(6)]],device const uint* bp [[buffer(7)]],device const uchar* bb [[buffer(8)]],device const uint* bl [[buffer(9)]],
 device const uint4* offsets [[buffer(10)]],device const uint4* total [[buffer(11)]],device atomic_uint* status [[buffer(12)]],
 device Result* rows [[buffer(13)]],device Capture* captures [[buffer(14)]],device Property* properties [[buffer(15)]],device uchar* bytes [[buffer(16)]],device uint4* layout [[buffer(17)]],
 device const uint2* ah [[buffer(18)]],device const uint2* bh [[buffer(19)]],constant TypedMergeParams& p [[buffer(20)]],uint index [[thread_position_in_grid]]) {
 if(atomic_load_explicit(status,memory_order_relaxed))return;
 for(uint component=0u;component<4u;component++){
  uint a_count=typed_counts(p,false)[component],b_count=typed_counts(p,true)[component];
  if(ulong(index)>=ulong(a_count)+b_count)continue;bool second=index>=a_count;uint i=second?index-a_count:index;
  device const uint2* heads=second?bh:ah;
  TypedSource s={typed_column(second?br:ar,heads,p,second,0u,status),typed_column(second?bc:ac,heads,p,second,1u,status),typed_column(second?bp:ap,heads,p,second,2u,status),second?bb:ab,typed_column(second?bl:al,heads,p,second,3u,status),typed_column(second?ar:br,second?ah:bh,p,!second,0u,status),typed_counts(p,second),second?p.an:p.bn};
  uint owner=typed_owner(s,component,i);
  if(owner>=s.counts.x){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
  Result row=column_result(s.rows,owner);uint4 l=typed_layout(s.layout,owner);uint rank=owner+typed_lower(s.other,s.other_n,row.task_ordinal);
  uint4 at=typed_offset(offsets,total,p.an+p.bn,rank),next=typed_offset(offsets,total,p.an+p.bn,rank+1u);
  if(next.x==at.x)continue;
  if(next.x!=at.x+1u){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
  if(component==0u){
   if((row.property_count && !typed_range(row.property_start,row.property_count,l.z,l.x)) || (row.dynamic_count && !typed_range(row.dynamic_start,row.dynamic_count,l.z,l.x))){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
   row.capture_start=at.y;row.property_start=row.property_count?row.property_start-l.z+at.z:0u;
   row.dynamic_start=row.dynamic_count?row.dynamic_start-l.z+at.z:0u;rows[at.x]=row;layout[at.x]=uint4(l.x,l.y,at.z,at.w);
  }else if(component==1u){
   Capture cap=column_capture(s.captures,i);
   if(!typed_range(i,1u,row.capture_start,row.capture_count) || (cap.property_count && !typed_range(cap.property_start,cap.property_count,l.z,l.x)) || (cap.text_start!=NONE && !typed_range(cap.text_start,cap.text_length,l.w,l.y))){atomic_store_explicit(status,2u,memory_order_relaxed);return;}
   cap.property_start=cap.property_count?cap.property_start-l.z+at.z:0u;if(cap.text_start!=NONE)cap.text_start=cap.text_start-l.w+at.w;
   captures[at.y+i-row.capture_start]=cap;
  }else if(component==2u){
   Property prop=column_property(s.properties,i);bool valid=prop.kind<=2u;
   if(prop.kind==1u){valid=valid && typed_range(prop.owner,1u,row.capture_start,row.capture_count);prop.owner=prop.owner-row.capture_start+at.y;}
   else{valid=valid && prop.owner==owner;prop.owner=at.x;}
   if(prop.kind==2u){valid=valid && prop.value<=1u && typed_range(prop.key_start,prop.key_length,l.w,l.y) && (!prop.value || typed_range(prop.value_start,prop.value_length,l.w,l.y));prop.key_start=prop.key_start-l.w+at.w;if(prop.value)prop.value_start=prop.value_start-l.w+at.w;}
   if(!valid){atomic_store_explicit(status,2u,memory_order_relaxed);return;}properties[at.z+i-l.z]=prop;
  }else bytes[at.w+i-l.w]=s.bytes[i];
 }
}
