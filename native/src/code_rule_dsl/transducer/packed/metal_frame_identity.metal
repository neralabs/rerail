// Private frame keys: task, kind, length, five zero columns; then value span and
// input index. Shared segmentation uses these keys, never capture geometry.
kernel void frontier_frame_prepare(device const uint4* descriptors [[buffer(0)]],
 device uint* keys [[buffer(1)]],device uint* representatives [[buffer(2)]],
 device atomic_uint* header [[buffer(3)]],constant ReduceParams& p [[buffer(4)]],
 uint tid [[thread_position_in_grid]]) {
 if(tid>=p.count) return;
 uint4 d=descriptors[tid];
 if(d.y>1u || ulong(d.z)+d.w>p.capture_capacity)
  atomic_fetch_or_explicit(header+6u,2u,memory_order_relaxed);
 for(uint field=0u;field<COLUMNS;field++) keys[field*p.count+tid]=0u;
 keys[tid]=d.x; keys[p.count+tid]=d.y; keys[2u*p.count+tid]=d.w;
 keys[CAPTURE_START*p.count+tid]=d.z; keys[CAPTURE_COUNT*p.count+tid]=d.w;
 keys[ORDINAL*p.count+tid]=tid;
 representatives[tid]=d.w?tid+1u:0u;
}

// One thread compares one value in one candidate pair. Hash collisions only
// enlarge comparison work; complete task, kind and ordered values decide equality.
kernel void frontier_compare_frames(device const uint* keys [[buffer(0)]],
 device const uint4* values [[buffer(1)]],device const uint2* slots [[buffer(2)]],
 device const uint4* buckets [[buffer(3)]],device const uint* items [[buffer(4)]],
 device const uint4* offsets [[buffer(5)]],device const uint4* totals [[buffer(6)]],
 device atomic_uint* rejected [[buffer(7)]],device atomic_uint* header [[buffer(8)]],
 constant ReduceParams& p [[buffer(9)]],uint tid [[thread_position_in_grid]]) {
 if(tid>=totals[0].y) return;
 uint candidate=work_owner(offsets,p.count,tid,1u);
 uint count=column(keys,p.count,CAPTURE_COUNT,candidate);
 uint local=tid-offsets[candidate].y,other_slot=local/max(count,1u),entry=local%max(count,1u);
 uint other=items[buckets[slots[candidate].x].x+other_slot];
 uint pair=offsets[candidate].x+other_slot;
 bool equal=same_key(keys,p.count,candidate,other);
 if(count) {
  uint start=column(keys,p.count,CAPTURE_START,candidate);
  uint4 value=values[start+entry];
  uint kind=column(keys,p.count,1u,candidate);
  bool valid=value.z==0u && value.w==0u;
  if(kind==0u) valid &= value.x<=65535u && value.y!=0xffffffffu && (entry==0u || values[start+entry-1u].x<value.x);
  else valid &= value.x==entry && value.y<=65535u;
  if(!valid) atomic_fetch_or_explicit(header+6u,2u,memory_order_relaxed);
  if(equal) equal=all(values[column(keys,p.count,CAPTURE_START,other)+entry]==value);
 }
 if(!equal) atomic_store_explicit(rejected+pair,1u,memory_order_relaxed);
}

kernel void frontier_frame_pairs(device const uint* keys [[buffer(0)]],
 device const uint2* slots [[buffer(1)]],device const uint4* buckets [[buffer(2)]],
 device const uint* items [[buffer(3)]],device const uint4* offsets [[buffer(4)]],
 device const uint4* totals [[buffer(5)]],device const uint* rejected [[buffer(6)]],
 device atomic_uint* representatives [[buffer(7)]],constant ReduceParams& p [[buffer(8)]],
 uint tid [[thread_position_in_grid]]) {
 if(tid>=totals[0].x || rejected[tid]) return;
 uint candidate=work_owner(offsets,p.count,tid,0u);
 uint other=items[buckets[slots[candidate].x].x+tid-offsets[candidate].x];
 if(column(keys,p.count,CAPTURE_COUNT,candidate))
  atomic_fetch_min_explicit(representatives+candidate,other+1u,memory_order_relaxed);
}

kernel void frontier_frame_publish(device const uint* representatives [[buffer(0)]],
 device uint* ids [[buffer(1)]],device uint* counts [[buffer(2)]],
 device atomic_uint* header [[buffer(3)]],constant ReduceParams& p [[buffer(4)]],
 uint tid [[thread_position_in_grid]]) {
 bool valid=p.count<=p.max_output && atomic_load_explicit(header+6u,memory_order_relaxed)==0u;
 if(tid==0u) {
  if(p.count>p.max_output) atomic_fetch_or_explicit(header+6u,1u,memory_order_relaxed);
  counts[0]=valid?p.count:0u; counts[1]=0u;
  counts[2]=valid?uint((ulong(p.count)+255ul)/256ul):0u; counts[3]=counts[4]=1u;
 }
 if(valid && tid<p.count) ids[tid]=representatives[tid];
}
