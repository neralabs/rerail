// A comparison recipe owns byte windows. Each lane compares at most 64 bytes.
struct PredicatePair {uint owner,member,mode,left_kind,right_kind,left_start,left_length,right_start,right_length,windows,units,answer;};
bool predicate_literal(View v,Program pg,PredicateSpec a) {
 if(a.kind==0u || a.kind==2u || a.kind==4u || a.kind==6u)return true;
 if(a.kind!=1u)return false;Value value=v.values[pg.value_start+a.second];return value.kind<=1u && v.strings[pg.string_start+value.operand].y>64u;
}
uint predicate_prefix_owner(device const uint4* offsets,uint n,uint at,uint field) {
 uint lo=0u,hi=n;while(lo<hi){uint mid=lo+(hi-lo)/2u;if(offsets[mid][field]<=at)lo=mid+1u;else hi=mid;}return lo?lo-1u:NONE;
}
kernel void predicate_literal_counts(device const uchar* data [[buffer(0)]],device const CaptureSpan* rows [[buffer(1)]],device const ArgumentCapture* caps [[buffer(2)]],device const uint4* meta [[buffer(3)]],device const uint* owners [[buffer(4)]],device const uint* ids [[buffer(5)]],device const uint* ranks [[buffer(6)]],device uint4* sizes [[buffer(7)]],device uint* members [[buffer(8)]],device atomic_uint* counts [[buffer(9)]],device const uint* status [[buffer(10)]],device const PredicateSpec* specs [[buffer(11)]],constant CaptureParams& p [[buffer(12)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.c)return;sizes[i]=uint4(0u);members[i]=0u;if(status[0])return;uint owner=owners[i];if(!meta[owner].w)return;
 View v=view(data);Program pg=v.programs[meta[owner].y];PredicateSpec a=argument_spec(v,meta,specs,owner,p);
 if(argument_capture(caps,i,p).label!=a.first || !predicate_literal(v,pg,a))return;
 if(a.kind==2u || a.kind==4u){atomic_fetch_add_explicit(counts+owner*4u+2u,1u,memory_order_relaxed);sizes[i].x=a.data_count;}
 else if(a.kind==0u || a.kind==6u)sizes[i].x=predicate_partner(caps,ids,ranks,argument_span(rows,owner,p),i,a.second,p)!=NONE?1u:0u;
 else sizes[i].x=1u;
}
kernel void predicate_literal_scatter(device const uchar* data [[buffer(0)]],device const CaptureSpan* rows [[buffer(1)]],device const ArgumentCapture* caps [[buffer(2)]],device const uint4* meta [[buffer(3)]],device const uint* owners [[buffer(4)]],device const uint* ids [[buffer(5)]],device const uint* ranks [[buffer(6)]],device const uint4* offsets [[buffer(7)]],device PredicatePair* pairs [[buffer(8)]],device uint4* work [[buffer(9)]],device const uint* status [[buffer(10)]],device const PredicateSpec* specs [[buffer(11)]],constant CaptureParams& p [[buffer(12)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.pairs || status[0])return;uint member=predicate_prefix_owner(offsets,p.c,i,0u),owner=owners[member];
 View v=view(data);Program pg=v.programs[meta[owner].y];PredicateSpec a=argument_spec(v,meta,specs,owner,p);ArgumentCapture cap=argument_capture(caps,member,p);
 PredicatePair pair={owner,member,0u,0u,1u,cap.start,cap.end-cap.start,0u,0u,0u,0u,0u};
 if(a.kind==0u || a.kind==6u){uint right=predicate_partner(caps,ids,ranks,argument_span(rows,owner,p),member,a.second,p);ArgumentCapture other=argument_capture(caps,right,p);pair.right_kind=0u;pair.right_start=other.start;pair.right_length=other.end-other.start;
  if(a.kind==6u){File f=v.files[meta[owner].z];pair.mode=3u;pair.answer=uint(output_line(v,f,cap.start)==output_line(v,f,other.start));}
 }else if(a.kind==1u){Value value=v.values[pg.value_start+a.second];uint2 string=v.strings[pg.string_start+value.operand];pair.mode=value.kind;pair.right_start=string.x;pair.right_length=string.y;}
 else {uint string_index=v.output_words[a.data_start+i-offsets[member].x];uint2 string=v.strings[pg.string_start+string_index];pair.mode=a.kind==4u?2u:0u;pair.right_start=string.x;pair.right_length=string.y;}
 if(pair.mode!=3u){bool fits=pair.left_length>=pair.right_length && (pair.mode!=0u || pair.left_length==pair.right_length);
  if(fits){if(!pair.right_length)pair.answer=1u;else {pair.windows=pair.mode==2u?pair.left_length-pair.right_length+1u:1u;ulong units=ulong(pair.windows)*((ulong(pair.right_length)+63ul)/64ul);pair.units=uint(min(units,ulong(NONE)));}}
 }
 pairs[i]=pair;work[i]=uint4(pair.windows,pair.units,0u,0u);
}
kernel void predicate_literal_window_init(device uint* windows [[buffer(0)]],constant CaptureParams& p [[buffer(1)]],uint i [[thread_position_in_grid]]) {if(i<p.windows)windows[i]=1u;}
kernel void predicate_literal_compare(device const uchar* data [[buffer(0)]],device const PredicatePair* pairs [[buffer(1)]],device const uint4* offsets [[buffer(2)]],device atomic_uint* windows [[buffer(3)]],device const uint* status [[buffer(4)]],constant CaptureParams& p [[buffer(5)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.units || status[0])return;uint index=predicate_prefix_owner(offsets,p.pairs,i,1u);PredicatePair pair=pairs[index];
 uint chunks=uint((ulong(pair.right_length)+63ul)/64ul),local=i-offsets[index].y,window=local/chunks,at=(local%chunks)*64u;
 View v=view(data);device const uchar* left=pair.left_kind?v.string_bytes:v.source;device const uchar* right=pair.right_kind?v.string_bytes:v.source;
 bool equal=true;for(uint byte=0u;byte<64u;byte++)if(byte<pair.right_length-at)equal=equal && left[pair.left_start+window+at+byte]==right[pair.right_start+at+byte];
 if(!equal)atomic_store_explicit(windows+offsets[index].x+window,0u,memory_order_relaxed);
}
kernel void predicate_literal_windows(device uint* pairs [[buffer(0)]],device const uint4* offsets [[buffer(1)]],device const uint* windows [[buffer(2)]],device const uint* status [[buffer(3)]],constant CaptureParams& p [[buffer(4)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.windows || status[0] || !windows[i])return;uint index=predicate_prefix_owner(offsets,p.pairs,i,0u);atomic_store_explicit((device atomic_uint*)(pairs+index*12u+11u),1u,memory_order_relaxed);
}
kernel void predicate_literal_results(device const uchar* data [[buffer(0)]],device const PredicatePair* pairs [[buffer(1)]],device const uint4* meta [[buffer(2)]],device atomic_uint* counts [[buffer(3)]],device atomic_uint* members [[buffer(4)]],device const uint* status [[buffer(5)]],device const PredicateSpec* specs [[buffer(6)]],constant CaptureParams& p [[buffer(7)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.pairs || status[0])return;PredicatePair pair=pairs[i];View v=view(data);Program pg=v.programs[meta[pair.owner].y];PredicateSpec a=argument_spec(v,meta,specs,pair.owner,p);
 if(a.kind==2u){if(pair.answer)atomic_store_explicit(members+pair.member,1u,memory_order_relaxed);return;}
 atomic_fetch_add_explicit(counts+pair.owner*4u,1u,memory_order_relaxed);if(pair.answer^(a.flags&1u))atomic_fetch_add_explicit(counts+pair.owner*4u+1u,1u,memory_order_relaxed);
}
kernel void predicate_literal_members(device const uchar* data [[buffer(0)]],device const ArgumentCapture* caps [[buffer(1)]],device const uint4* meta [[buffer(2)]],device const uint* owners [[buffer(3)]],device atomic_uint* counts [[buffer(4)]],device const uint* members [[buffer(5)]],device const uint* status [[buffer(6)]],device const PredicateSpec* specs [[buffer(7)]],constant CaptureParams& p [[buffer(8)]],uint i [[thread_position_in_grid]]) {
 if(i>=p.c || status[0])return;uint owner=owners[i];if(!meta[owner].w)return;View v=view(data);Program pg=v.programs[meta[owner].y];PredicateSpec a=argument_spec(v,meta,specs,owner,p);if(a.kind!=2u || argument_capture(caps,i,p).label!=a.first)return;
 atomic_fetch_add_explicit(counts+owner*4u,1u,memory_order_relaxed);if(members[i]^(a.flags&1u))atomic_fetch_add_explicit(counts+owner*4u+1u,1u,memory_order_relaxed);
}
