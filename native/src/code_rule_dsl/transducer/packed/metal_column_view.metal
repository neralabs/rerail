// A checked physical column view. Both formats preserve every literal u32.
struct ColumnView {
 device const uint* words;device const uint2* headers;
 uint rows,fields,header_start,header_count,word_count,encoded;
 device atomic_uint* status;
 device const uint4* prefixes;device const uint4* segments;device const uint* complex_words;
 uint spans,complex_rows,arena;
};
ColumnView literal_columns(device const uint* words,device const uint2* headers,uint rows,uint fields,
 uint header_start,uint header_count,uint word_count,uint encoded,device atomic_uint* status) {
 ColumnView v={};v.words=words;v.headers=headers;v.rows=rows;v.fields=fields;
 v.header_start=header_start;v.header_count=header_count;v.word_count=word_count;v.encoded=encoded;v.status=status;return v;
}
uint literal_column_value(ColumnView v,uint row,uint field) {
 if(row>=v.rows || field>=v.fields || v.encoded>1u){atomic_store_explicit(v.status,2u,memory_order_relaxed);return 0u;}
 if(!v.encoded)return v.words[row*v.fields+field];
 ulong group=ulong(v.header_start)+ulong(row/256u)*v.fields+field;
 if(group>=v.header_count){atomic_store_explicit(v.status,2u,memory_order_relaxed);return 0u;}
 uint2 h=v.headers[group];uint bits=(h.y&31u)+(h.y>>31u)*32u,at=(h.y&0x7fffffe0u)/4u;
 uint count=min(256u,v.rows-(row/256u)*256u),need=((count*bits+255u)/256u)*8u;
 if(bits>32u || at>v.word_count || need>v.word_count-at){atomic_store_explicit(v.status,2u,memory_order_relaxed);return 0u;}
 uint next=group+1ul<v.header_count?(v.headers[group+1ul].y&0x7fffffe0u)/4u:v.word_count;
 if(next!=at+need || (!group && at)){atomic_store_explicit(v.status,2u,memory_order_relaxed);return 0u;}
 uint value=0u;
 if(bits){uint bit=(row%256u)*bits,word=bit/32u,shift=bit%32u;
  value=v.words[at+word]>>shift;
  if(shift && bits>32u-shift)value|=v.words[at+word+1u]<<(32u-shift);
  if(bits<32u)value&=(1u<<bits)-1u;
 }
 ulong sum=ulong(h.x)+value;
 if(sum>0xfffffffful){atomic_store_explicit(v.status,2u,memory_order_relaxed);return 0u;}
 return uint(sum);
}
uint semantic_raw(ColumnView v,uint span,uint local,uint field) {
 uint4 m=v.segments[span*3u],counts=v.segments[span*3u+1u],bases=v.segments[span*3u+2u];
 uint4 lengths=uint4(m.z,m.w,counts.x,m.z);
 if((m.x!=1u && m.x!=2u) || any(counts.zw!=uint2(0u)) ||
    (m.x==1u && (any(bases!=uint4(0u)) || counts.y)) ||
    (m.x==2u && m.y!=0xffffffffu)) {atomic_store_explicit(v.status,2u,memory_order_relaxed);return 0u;}
 if(m.x==1u){
  ColumnView raw=v;raw.encoded=1u;raw.rows=lengths[v.arena];raw.header_start=m.y;
  for(uint k=0u;k<v.arena;k++){uint fields=k==0u?10u:(k==1u?9u:8u);raw.header_start+=((lengths[k]+255u)/256u)*fields;}
  return literal_column_value(raw,local,field);
 }
 uint base=bases[v.arena==3u?0u:v.arena];
 if(base>v.complex_rows || local>=v.complex_rows-base){atomic_store_explicit(v.status,2u,memory_order_relaxed);return 0u;}
 ColumnView raw=v;raw.encoded=0u;raw.rows=v.complex_rows;raw.words=v.complex_words;
 return literal_column_value(raw,base+local,field);
}
uint column_rebase(ColumnView v,uint value,uint source,uint target,uint count) {
 if(value<source || value-source>count || ulong(target)+value-source>0xfffffffful){atomic_store_explicit(v.status,2u,memory_order_relaxed);return 0u;}
 return target+value-source;
}
uint semantic_column_value(ColumnView v,uint row,uint field) {
 if(row>=v.rows || field>=v.fields || v.arena>=4u || !v.spans){atomic_store_explicit(v.status,2u,memory_order_relaxed);return 0u;}
 uint component=v.arena==3u?0u:v.arena,lo=0u,hi=v.spans;
 while(lo<hi){uint mid=lo+(hi-lo)/2u;if(v.prefixes[mid][component]<=row)lo=mid+1u;else hi=mid;}
 if(!lo){atomic_store_explicit(v.status,2u,memory_order_relaxed);return 0u;}
 uint span=lo-1u;uint4 at=v.prefixes[span],m=v.segments[span*3u],counts=v.segments[span*3u+1u],bases=v.segments[span*3u+2u];
 uint4 lengths=uint4(m.z,m.w,counts.x,counts.y);
 uint length=v.arena==3u?m.z:lengths[component],next=span+1u<v.spans?v.prefixes[span+1u][component]:v.rows;
 if(next<at[component] || next-at[component]!=length || row-at[component]>=length){atomic_store_explicit(v.status,2u,memory_order_relaxed);return 0u;}
 uint local=row-at[component],value=semantic_raw(v,span,local,field);
 if(v.arena==0u){
  if(field==4u)return column_rebase(v,value,bases.y,at.y,m.w);
  if(field==6u && semantic_raw(v,span,local,7u))return column_rebase(v,value,bases.z,at.z,counts.x);
  if(field==8u && semantic_raw(v,span,local,9u))return column_rebase(v,value,bases.z,at.z,counts.x);
 }else if(v.arena==1u){
  if(field==4u && value!=0xffffffffu)return column_rebase(v,value,bases.w,at.w,counts.y);
  if(field==6u && semantic_raw(v,span,local,7u))return column_rebase(v,value,bases.z,at.z,counts.x);
 }else if(v.arena==2u){
  uint kind=semantic_raw(v,span,local,0u);
  if(kind>2u){atomic_store_explicit(v.status,2u,memory_order_relaxed);return 0u;}
  if(field==1u)return kind==1u?column_rebase(v,value,bases.y,at.y,m.w):column_rebase(v,value,bases.x,at.x,m.z);
  if(kind==2u && (field==4u || (field==6u && semantic_raw(v,span,local,3u)==1u)))return column_rebase(v,value,bases.w,at.w,counts.y);
 }else{
  if(field==2u)return column_rebase(v,value,bases.z,at.z,counts.x);
  if(field==3u)return column_rebase(v,value,bases.w,at.w,counts.y);
 }
 return value;
}
uint column_value(ColumnView v,uint row,uint field) {
 return v.encoded==2u?semantic_column_value(v,row,field):literal_column_value(v,row,field);
}
Result column_result(ColumnView v,uint row) {
 return Result{column_value(v,row,0u),column_value(v,row,1u),column_value(v,row,2u),column_value(v,row,3u),column_value(v,row,4u),column_value(v,row,5u),column_value(v,row,6u),column_value(v,row,7u),column_value(v,row,8u),column_value(v,row,9u)};
}
Capture column_capture(ColumnView v,uint row) {
 return Capture{column_value(v,row,0u),column_value(v,row,1u),column_value(v,row,2u),column_value(v,row,3u),column_value(v,row,4u),column_value(v,row,5u),column_value(v,row,6u),column_value(v,row,7u),column_value(v,row,8u)};
}
Property column_property(ColumnView v,uint row) {
 return Property{column_value(v,row,0u),column_value(v,row,1u),column_value(v,row,2u),column_value(v,row,3u),column_value(v,row,4u),column_value(v,row,5u),column_value(v,row,6u),column_value(v,row,7u)};
}
