// Complete-state schema: six scalar keys, three typed spans, logical ordinal.
// This schema never interprets a frame address as a canonical value identity.
constant uint VALUE_KEYS=6u, VALUE_COLUMNS=13u, VALUE_ORDINAL=12u;
// Query kinds use exact admission; ordinary kinds preserve occurrence identity.
bool value_ordinary(uint kind) { return kind==2u || kind==3u || kind==5u || kind==9u || kind==13u; }
uint value_length(device const uint* states,uint n,uint index) {
 return max(1u,max(column(states,n,7u,index),max(column(states,n,9u,index),column(states,n,11u,index))));
}
kernel void state_value_initialize(device const uint* counts [[buffer(0)]],
 device const uint* lengths [[buffer(1)]],device ReduceParams& p [[buffer(2)]],
 device uint* header [[buffer(3)]],uint tid [[thread_position_in_grid]]) {
 if(tid) return;
 uint n=counts[0],captures=counts[1];
 bool valid=n<=p.max_count && captures<=p.max_captures && lengths[0]<=p.max_cursors && lengths[1]<=p.max_repeats
  && counts[2]==uint((ulong(n)+255ul)/256ul) && counts[3]==1u && counts[4]==1u;
 p.count=valid?n:0u; p.capture_capacity=valid?captures:0u;
 header[6]=valid?0u:2u;
 header[8]=uint((ulong(p.count)+255ul)/256ul); header[9]=header[10]=1u;
 uint count=p.count;
 for(uint level=0u;level<p.scan_levels;level++) {
  uint groups=max(1u,uint((ulong(count)+255ul)/256ul));
  header[11u+3u*level]=groups; header[12u+3u*level]=header[13u+3u*level]=1u;
  count=uint((ulong(count)+255ul)/256ul);
 }
}
kernel void state_value_bucket_count(device const uint* states [[buffer(0)]],
 device atomic_uint* counts [[buffer(1)]],device uint2* slots [[buffer(2)]],
 device const uint4* cursors [[buffer(3)]],device const uint4* repeats [[buffer(4)]],
 device const uint4* captures [[buffer(5)]],device const uint* lengths [[buffer(6)]],
 device atomic_uint* header [[buffer(7)]],constant ReduceParams& p [[buffer(8)]],uint tid [[thread_position_in_grid]]) {
 if(tid>=p.count) return;
 uint h=2166136261u;
 for(uint field=0u;field<VALUE_KEYS;field++) { h=(h^column(states,p.count,field,tid))*16777619u; h^=h>>16; }
 for(uint field=7u;field<=11u;field+=2u) { h=(h^column(states,p.count,field,tid))*16777619u; h^=h>>16; }
 // Bounded content sampling improves candidate distribution for equal-length
 // frames. Samples never establish equality: every complete value still enters
 // state_value_compare. No storage address enters this hash.
 for(uint span=0u;span<3u;span++) {
  uint start=column(states,p.count,6u+span*2u,tid),count=column(states,p.count,7u+span*2u,tid);
  uint length=span==2u?p.capture_capacity:lengths[span];
  if(ulong(start)+count>length) {atomic_fetch_or_explicit(header+6u,2u,memory_order_relaxed);continue;}
  if(count==0u)continue;
  device const uint4* arena=span==0u?cursors:(span==1u?repeats:captures);
  for(uint sample=0u;sample<3u;sample++) {
   uint index=sample==0u?0u:(sample==1u?count/2u:count-1u);uint4 value=arena[start+index];
   for(uint component=0u;component<4u;component++) {h=(h^value[component])*16777619u;h^=h>>16;}
  }
 }

 // Distinct ordinary occurrences need only their own validation pair. A hash
 // collision still receives complete comparison and cannot merge occurrences.
 if(value_ordinary(column(states,p.count,5u,tid))) h=(h^tid)*16777619u;
 uint bucket=h&(p.buckets-1u);
 slots[tid]=uint2(bucket,atomic_fetch_add_explicit(counts+bucket*4u,1u,memory_order_relaxed));
}
kernel void state_value_comparison_count(device const uint* states [[buffer(0)]],
 device const uint2* slots [[buffer(1)]],device const uint4* offsets [[buffer(2)]],
 device const uint4* totals [[buffer(3)]],device uint4* counts [[buffer(4)]],
 device atomic_uint* header [[buffer(5)]],device const uint* lengths [[buffer(6)]],
 constant ReduceParams& p [[buffer(7)]],uint tid [[thread_position_in_grid]]) {
 if(tid>=p.count) return;
 uint kind=column(states,p.count,5u,tid);
 if((kind>9u && kind!=12u && kind!=13u) || kind==6u || kind==7u) atomic_fetch_or_explicit(header+6u,2u,memory_order_relaxed);
 uint bucket=slots[tid].x;
 uint pairs=(bucket+1u<p.buckets?offsets[bucket+1u].x:totals[0].x)-offsets[bucket].x;
 for(uint span=0u;span<3u;span++) {
  uint start=column(states,p.count,6u+2u*span,tid), count=column(states,p.count,7u+2u*span,tid);
  uint length=span==2u?p.capture_capacity:lengths[span];
  if(ulong(start)+count>length) atomic_fetch_or_explicit(header+6u,2u,memory_order_relaxed);
 }
 ulong work=ulong(pairs)*value_length(states,p.count,tid);
 counts[tid]=uint4(pairs,uint(min(work,0xfffffffful)),0u,0u);
}
// Each thread compares at most one cursor, repeat and capture entry. Full history
// length changes work size rather than a per-thread traversal or a host command loop.
kernel void state_value_compare(device const uint* states [[buffer(0)]],
 device const uint4* captures [[buffer(1)]],device const uint2* slots [[buffer(2)]],
 device const uint4* buckets [[buffer(3)]],device const uint* items [[buffer(4)]],
 device const uint4* offsets [[buffer(5)]],device const uint4* totals [[buffer(6)]],
 device atomic_uint* rejected [[buffer(7)]],device atomic_uint* header [[buffer(8)]],
 device const uint4* cursors [[buffer(9)]],device const uint4* repeats [[buffer(10)]],
 constant ReduceParams& p [[buffer(11)]],uint tid [[thread_position_in_grid]]) {
 if(tid>=totals[0].y) return;
 uint candidate=work_owner(offsets,p.count,tid,1u),width=value_length(states,p.count,candidate);
 uint local=tid-offsets[candidate].y,other_slot=local/width,entry=local%width;
 uint other=items[buckets[slots[candidate].x].x+other_slot],pair=offsets[candidate].x+other_slot;
 bool equal=true;
 for(uint field=0u;field<VALUE_KEYS;field++) equal &= column(states,p.count,field,candidate)==column(states,p.count,field,other);
 for(uint span=0u;span<3u;span++) {
  uint start=column(states,p.count,6u+2u*span,candidate),count=column(states,p.count,7u+2u*span,candidate);
  uint other_start=column(states,p.count,6u+2u*span,other),other_count=column(states,p.count,7u+2u*span,other);
  equal &= count==other_count;
  if(entry>=count) continue;
  device const uint4* arena=span==0u?cursors:(span==1u?repeats:captures);
  uint4 value=arena[start+entry];
  bool valid;
  if(span==0u) valid=value.x<=65535u && value.y!=0xffffffffu && value.z==0u && value.w==0u && (entry==0u || arena[start+entry-1u].x<value.x);
  else if(span==1u) valid=value.x==entry && value.y<=65535u && value.z==0u && value.w==0u;
  else valid=value.x!=0xffffffffu && value.y<=65535u && value.z<=value.w && (entry==0u || !geometry_less(value,arena[start+entry-1u]));
  if(!valid) atomic_fetch_or_explicit(header+6u,2u,memory_order_relaxed);
  if(entry<other_count) equal &= all(value==arena[other_start+entry]);
 }
 if(!equal) atomic_store_explicit(rejected+pair,1u,memory_order_relaxed);
}
kernel void state_value_reduce_pairs(device const uint* states [[buffer(0)]],
 device const uint2* slots [[buffer(1)]],device const uint4* buckets [[buffer(2)]],
 device const uint* items [[buffer(3)]],device const uint4* offsets [[buffer(4)]],
 device const uint4* totals [[buffer(5)]],device const uint* rejected [[buffer(6)]],
 device atomic_uint* dominated [[buffer(7)]],constant ReduceParams& p [[buffer(8)]],
 uint tid [[thread_position_in_grid]]) {
 if(tid>=totals[0].x || rejected[tid]) return;
 uint candidate=work_owner(offsets,p.count,tid,0u);
 if(value_ordinary(column(states,p.count,5u,candidate))) return;
 uint other=items[buckets[slots[candidate].x].x+tid-offsets[candidate].x];
 uint ordinal=column(states,p.count,VALUE_ORDINAL,candidate),other_ordinal=column(states,p.count,VALUE_ORDINAL,other);
 if(other_ordinal<ordinal || (other_ordinal==ordinal && other<candidate)) atomic_store_explicit(dominated+candidate,1u,memory_order_relaxed);
}
kernel void state_value_scatter(device const uint* states [[buffer(0)]],
 device const uint* dominated [[buffer(1)]],device const uint4* offsets [[buffer(2)]],
 device const uint4* totals [[buffer(3)]],device uint* output [[buffer(4)]],
 device uint* counts [[buffer(5)]],device atomic_uint* header [[buffer(6)]],
 constant ReduceParams& p [[buffer(7)]],uint tid [[thread_position_in_grid]]) {
 uint n=totals[0].x;
 bool valid=n<=p.max_output && atomic_load_explicit(header+6u,memory_order_relaxed)==0u;
 if(tid==0u) {
  if(n>p.max_output) atomic_fetch_or_explicit(header+6u,1u,memory_order_relaxed);
  counts[0]=valid?n:0u; counts[1]=valid?p.capture_capacity:0u;
  counts[2]=valid?uint((ulong(n)+255ul)/256ul):0u; counts[3]=counts[4]=1u;
 }
 if(!valid || tid>=p.count || dominated[tid] || !n) return;
 for(uint field=0u;field<VALUE_COLUMNS;field++) output[field*n+offsets[tid].x]=column(states,p.count,field,tid);
}

// Membership labels are local input positions, never canonical state identities.
// Logical ordinal selection remains the responsibility of admission/output stages.
kernel void state_value_group_seed(device uint* groups [[buffer(0)]],constant ReduceParams& p [[buffer(1)]],uint tid [[thread_position_in_grid]]) {
 if(tid<p.count) groups[tid]=tid;
}
kernel void state_value_group_pairs(device const uint* states [[buffer(0)]],
 device const uint2* slots [[buffer(1)]],device const uint4* buckets [[buffer(2)]],
 device const uint* items [[buffer(3)]],device const uint4* offsets [[buffer(4)]],
 device const uint4* totals [[buffer(5)]],device const uint* rejected [[buffer(6)]],
 device atomic_uint* groups [[buffer(7)]],constant ReduceParams& p [[buffer(8)]],uint tid [[thread_position_in_grid]]) {
 if(tid>=totals[0].x || rejected[tid]) return;
 uint candidate=work_owner(offsets,p.count,tid,0u);
 if(value_ordinary(column(states,p.count,5u,candidate))) return;
 uint other=items[buckets[slots[candidate].x].x+tid-offsets[candidate].x];
 atomic_fetch_min_explicit(groups+candidate,other,memory_order_relaxed);
}
kernel void state_value_group_publish(device const uint* groups [[buffer(0)]],device uint* output [[buffer(1)]],
 device uint* counts [[buffer(2)]],device atomic_uint* header [[buffer(3)]],constant ReduceParams& p [[buffer(4)]],uint tid [[thread_position_in_grid]]) {
 bool valid=p.count<=p.max_output && atomic_load_explicit(header+6u,memory_order_relaxed)==0u;
 if(tid==0u) {
  if(p.count>p.max_output) atomic_fetch_or_explicit(header+6u,1u,memory_order_relaxed);
  counts[0]=valid?p.count:0u;counts[1]=valid?p.capture_capacity:0u;
  counts[2]=valid?uint((ulong(p.count)+255ul)/256ul):0u;counts[3]=counts[4]=1u;
 }
 if(valid && tid<p.count) output[tid]=groups[tid];
}
