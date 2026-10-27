#include <metal_stdlib>
using namespace metal;
struct ReduceParams { uint count,buckets,pair_capacity,capture_capacity,active_exact,max_count,max_captures,max_output,scan_levels,max_cursors,max_repeats; };
constant uint KEY_COLUMNS=8u, CAPTURE_START=8u, CAPTURE_COUNT=9u, ORDINAL=10u, COLUMNS=11u;
uint column(device const uint* states,uint count,uint field,uint index) {
 return states[field*count+index];
}
uint state_hash(device const uint* states,ReduceParams p,uint index) {
 uint h=2166136261u;
 for(uint field=0u;field<KEY_COLUMNS;field++) {
  h=(h^column(states,p.count,field,index))*16777619u;
  h^=h>>16;
 }
 return h&(p.buckets-1u);
}
bool same_key(device const uint* states,uint count,uint a,uint b) {
 bool equal=true;
 for(uint field=0u;field<KEY_COLUMNS;field++) equal &= column(states,count,field,a)==column(states,count,field,b);
 return equal;
}
kernel void frontier_initialize(device const uint* counts [[buffer(0)]],
 device ReduceParams& p [[buffer(1)]],device uint* header [[buffer(2)]],uint tid [[thread_position_in_grid]]) {
 if(tid) return;
 uint n=counts[0],captures=counts[1];
 bool valid=n<=p.max_count && captures<=p.max_captures && counts[2]==uint((ulong(n)+255ul)/256ul) && counts[3]==1u && counts[4]==1u;
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
kernel void frontier_bucket_count(device const uint* states [[buffer(0)]],
 device atomic_uint* counts [[buffer(1)]],device uint2* slots [[buffer(2)]],
 constant ReduceParams& p [[buffer(3)]],uint tid [[thread_position_in_grid]]) {
 if(tid>=p.count) return;
 uint bucket=state_hash(states,p,tid);
 slots[tid]=uint2(bucket,atomic_fetch_add_explicit(counts+bucket*4u,1u,memory_order_relaxed));
}
kernel void frontier_bucket_scatter(device const uint2* slots [[buffer(0)]],
 device const uint4* offsets [[buffer(1)]],device uint* items [[buffer(2)]],
 constant ReduceParams& p [[buffer(3)]],uint tid [[thread_position_in_grid]]) {
 if(tid<p.count) { uint2 slot=slots[tid]; items[offsets[slot.x].x+slot.y]=tid; }
}
kernel void frontier_comparison_count(device const uint* states [[buffer(0)]],
 device const uint2* slots [[buffer(1)]],device const uint4* offsets [[buffer(2)]],
 device const uint4* totals [[buffer(3)]],device uint4* counts [[buffer(4)]],
 device atomic_uint* header [[buffer(5)]],constant ReduceParams& p [[buffer(6)]],
 uint tid [[thread_position_in_grid]]) {
 if(tid>=p.count) return;
 uint bucket=slots[tid].x;
 uint pairs=(bucket+1u<p.buckets?offsets[bucket+1u].x:totals[0].x)-offsets[bucket].x;
 uint captures=column(states,p.count,CAPTURE_COUNT,tid);
 ulong end=ulong(column(states,p.count,CAPTURE_START,tid))+captures;
 if(end>p.capture_capacity) atomic_store_explicit(header+6u,2u,memory_order_relaxed);
 ulong work=ulong(pairs)*max(captures,1u);
 counts[tid]=uint4(pairs,uint(min(work,0xfffffffful)),0u,0u);
}
kernel void frontier_comparison_boundary(device const uint4* totals [[buffer(0)]],
 device uint* header [[buffer(1)]],constant ReduceParams& p [[buffer(2)]],
 uint tid [[thread_position_in_grid]]) {
 if(tid) return;
 uint4 total=totals[0];
 bool valid=header[6]==0u && total.x<=p.pair_capacity && total.y!=0xffffffffu;
 header[0]=valid?uint((ulong(total.y)+255ul)/256ul):0u; header[1]=header[2]=1u;
 header[3]=valid?uint((ulong(total.x)+255ul)/256ul):0u; header[4]=header[5]=1u;
 if(total.x>p.pair_capacity || total.y==0xffffffffu) header[6]|=1u;
}
uint work_owner(device const uint4* offsets,uint count,uint value,uint component) {
 uint low=0u,high=count;
 while(low<high) { uint mid=low+(high-low)/2u; if(offsets[mid][component]<=value) low=mid+1u; else high=mid; }
 return low-1u;
}
kernel void frontier_compare_captures(device const uint* states [[buffer(0)]],
 device const uint4* captures [[buffer(1)]],device const uint2* slots [[buffer(2)]],
 device const uint4* buckets [[buffer(3)]],device const uint* items [[buffer(4)]],
 device const uint4* offsets [[buffer(5)]],device const uint4* totals [[buffer(6)]],
 device atomic_uint* rejected [[buffer(7)]],device atomic_uint* header [[buffer(8)]],
 constant ReduceParams& p [[buffer(9)]],uint tid [[thread_position_in_grid]]) {
 if(tid>=totals[0].y) return;
 uint candidate=work_owner(offsets,p.count,tid,1u);
 uint count=column(states,p.count,CAPTURE_COUNT,candidate);
 uint local=tid-offsets[candidate].y,other_slot=local/max(count,1u),capture=local%max(count,1u);
 uint other=items[buckets[slots[candidate].x].x+other_slot];
 uint pair=offsets[candidate].x+other_slot;
 uint other_count=column(states,p.count,CAPTURE_COUNT,other);
 bool equal=same_key(states,p.count,candidate,other);
 if(p.active_exact) equal &= count==other_count;
 if(equal && count) {
  uint start=column(states,p.count,CAPTURE_START,candidate);
  uint other_start=column(states,p.count,CAPTURE_START,other);
  uint4 value=captures[start+capture];
  if(capture && geometry_less(value,captures[start+capture-1u])) atomic_store_explicit(header+6u,2u,memory_order_relaxed);
  if(p.active_exact) {
   equal=all(captures[other_start+capture]==value);
  } else {
   uint rank=capture-geometry_begin(captures,start,count,value);
   uint target=geometry_begin(captures,other_start,other_count,value)+rank;
   equal=target<other_count && all(captures[other_start+target]==value);
  }
 }
 if(!equal) atomic_store_explicit(rejected+pair,1u,memory_order_relaxed);
}
kernel void frontier_reduce_pairs(device const uint* states [[buffer(0)]],
 device const uint2* slots [[buffer(1)]],device const uint4* buckets [[buffer(2)]],
 device const uint* items [[buffer(3)]],device const uint4* offsets [[buffer(4)]],
 device const uint4* totals [[buffer(5)]],device const uint* rejected [[buffer(6)]],
 device atomic_uint* dominated [[buffer(7)]],constant ReduceParams& p [[buffer(8)]],
 uint tid [[thread_position_in_grid]]) {
 if(tid>=totals[0].x || rejected[tid]) return;
 uint candidate=work_owner(offsets,p.count,tid,0u);
 uint other=items[buckets[slots[candidate].x].x+tid-offsets[candidate].x];
 uint count=column(states,p.count,CAPTURE_COUNT,candidate);
 uint other_count=column(states,p.count,CAPTURE_COUNT,other);
 bool earlier=column(states,p.count,ORDINAL,other)<column(states,p.count,ORDINAL,candidate);
 if((!p.active_exact && other_count>count) || (other_count==count && (earlier || (column(states,p.count,ORDINAL,other)==column(states,p.count,ORDINAL,candidate) && other<candidate))))
  atomic_store_explicit(dominated+candidate,1u,memory_order_relaxed);
}
kernel void frontier_survivor_count(device const uint* dominated [[buffer(0)]],
 device uint4* counts [[buffer(1)]],device const uint* header [[buffer(2)]],
 constant ReduceParams& p [[buffer(3)]],uint tid [[thread_position_in_grid]]) {
 if(tid<p.count) counts[tid]=uint4(uint(!dominated[tid] && header[6]==0u),0u,0u,0u);
}
kernel void frontier_survivor_scatter(device const uint* states [[buffer(0)]],
 device const uint* dominated [[buffer(1)]],device const uint4* offsets [[buffer(2)]],
 device const uint4* totals [[buffer(3)]],device uint* output [[buffer(4)]],
 constant ReduceParams& p [[buffer(5)]],uint tid [[thread_position_in_grid]]) {
 if(tid>=p.count || dominated[tid] || !totals[0].x) return;
 for(uint field=0u;field<COLUMNS;field++) output[field*totals[0].x+offsets[tid].x]=column(states,p.count,field,tid);
}

kernel void frontier_counted_scatter(device const uint* states [[buffer(0)]],
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
 for(uint field=0u;field<COLUMNS;field++) output[field*n+offsets[tid].x]=column(states,p.count,field,tid);
}
