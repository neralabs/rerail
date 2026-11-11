#include <metal_stdlib>
using namespace metal;
uint4 compact_sum(uint4 a, uint4 b) {
 return uint4(min(ulong4(a)+ulong4(b),ulong4(0xfffffffful)));
}
kernel void compact_scan(device const uint4* input [[buffer(0)]],
 device uint4* offsets [[buffer(1)]], device uint4* sums [[buffer(2)]],
 constant uint& count [[buffer(3)]], uint tid [[thread_position_in_grid]],
 uint lane [[thread_index_in_threadgroup]], uint group [[threadgroup_position_in_grid]]) {
 threadgroup uint4 values[256];
 values[lane]=tid<count?input[tid]:uint4(0);
 threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint step=1u;step<256u;step<<=1u) {
  uint4 add=lane>=step?values[lane-step]:uint4(0);
  threadgroup_barrier(mem_flags::mem_threadgroup);
  values[lane]=compact_sum(values[lane],add);
  threadgroup_barrier(mem_flags::mem_threadgroup);
 }
 if(tid<count) offsets[tid]=lane?values[lane-1u]:uint4(0);
 if(lane==255u) sums[group]=values[lane];
}
kernel void compact_add_parent(device uint4* offsets [[buffer(0)]],
 device const uint4* parent [[buffer(1)]],constant uint& count [[buffer(2)]],
 uint tid [[thread_position_in_grid]]) {
 if(tid<count) offsets[tid]=compact_sum(offsets[tid],parent[tid/256u]);
}
uint compact_level_count(uint count,uint level) {
 for(uint at=0u;at<level;at++) count=uint((ulong(count)+255ul)/256ul);
 return count;
}
kernel void compact_scan_counted(device const uint4* input [[buffer(0)]],
 device uint4* offsets [[buffer(1)]],device uint4* sums [[buffer(2)]],
 device const uint* length [[buffer(3)]],constant uint& level [[buffer(4)]],
 uint tid [[thread_position_in_grid]],uint lane [[thread_index_in_threadgroup]],uint group [[threadgroup_position_in_grid]]) {
 uint count=compact_level_count(length[0],level);
 threadgroup uint4 values[256];
 values[lane]=tid<count?input[tid]:uint4(0);
 threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint step=1u;step<256u;step<<=1u) {
  uint4 add=lane>=step?values[lane-step]:uint4(0);
  threadgroup_barrier(mem_flags::mem_threadgroup);
  values[lane]=compact_sum(values[lane],add);
  threadgroup_barrier(mem_flags::mem_threadgroup);
 }
 if(tid<count) offsets[tid]=lane?values[lane-1u]:uint4(0);
 if(lane==255u) sums[group]=values[lane];
}
kernel void compact_add_parent_counted(device uint4* offsets [[buffer(0)]],
 device const uint4* parent [[buffer(1)]],device const uint* length [[buffer(2)]],
 constant uint& level [[buffer(3)]],uint tid [[thread_position_in_grid]]) {
 uint count=compact_level_count(length[0],level);
 if(tid<count) offsets[tid]=compact_sum(offsets[tid],parent[tid/256u]);
}
// Eight independent columns share each hierarchy dispatch and its barriers.
kernel void compact_scan_pair(device const uint4* a [[buffer(0)]],device uint4* ao [[buffer(1)]],device uint4* asums [[buffer(2)]],
 device const uint4* b [[buffer(3)]],device uint4* bo [[buffer(4)]],device uint4* bsums [[buffer(5)]],constant uint& count [[buffer(6)]],
 uint tid [[thread_position_in_grid]],uint lane [[thread_index_in_threadgroup]],uint group [[threadgroup_position_in_grid]]) {
 threadgroup uint4 av[256],bv[256];av[lane]=tid<count?a[tid]:uint4(0u);bv[lane]=tid<count?b[tid]:uint4(0u);
 threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint step=1u;step<256u;step*=2u){uint4 aa=lane>=step?av[lane-step]:uint4(0u),bb=lane>=step?bv[lane-step]:uint4(0u);
  threadgroup_barrier(mem_flags::mem_threadgroup);av[lane]=compact_sum(av[lane],aa);bv[lane]=compact_sum(bv[lane],bb);
  threadgroup_barrier(mem_flags::mem_threadgroup);}
 if(tid<count){ao[tid]=lane?av[lane-1u]:uint4(0u);bo[tid]=lane?bv[lane-1u]:uint4(0u);}
 if(lane==255u){asums[group]=av[lane];bsums[group]=bv[lane];}
}
kernel void compact_add_parent_pair(device uint4* a [[buffer(0)]],device const uint4* ap [[buffer(1)]],
 device uint4* b [[buffer(2)]],device const uint4* bp [[buffer(3)]],constant uint& count [[buffer(4)]],uint tid [[thread_position_in_grid]]) {
 if(tid<count){a[tid]=compact_sum(a[tid],ap[tid/256u]);b[tid]=compact_sum(b[tid],bp[tid/256u]);}
}

// One bounded threadgroup owns disjoint contiguous intervals. Every input is
// reduced before any interval is overwritten. Unsaturated ulong sums preserve
// the exact exclusive prefix even after a preceding interval saturates uint.
void compact_cooperative(device uint4* values, device uint4* total,
 uint count, uint lane, threadgroup ulong4* sums) {
 uint width=(count+255u)/256u;
 uint begin=min(count,lane*width),end=min(count,begin+width);
 ulong4 sum=ulong4(0);
 for(uint i=begin;i<end;i++)sum+=ulong4(values[i]);
 sums[lane]=sum;
 threadgroup_barrier(mem_flags::mem_threadgroup);
 for(uint step=1u;step<256u;step*=2u) {
  ulong4 add=lane>=step?sums[lane-step]:ulong4(0);
  threadgroup_barrier(mem_flags::mem_threadgroup);
  sums[lane]+=add;
  threadgroup_barrier(mem_flags::mem_threadgroup);
 }
 ulong4 prefix=lane?sums[lane-1u]:ulong4(0);
 if(lane==255u)total[0]=uint4(min(sums[lane],ulong4(0xfffffffful)));
 for(uint i=begin;i<end;i++) {
  uint4 value=values[i];
  values[i]=uint4(min(prefix,ulong4(0xfffffffful)));
  prefix+=ulong4(value);
 }
 // A paired arena can reuse the same scratch only after every lane finishes.
 threadgroup_barrier(mem_flags::mem_threadgroup);
}
kernel void compact_scan_cooperative(device uint4* values [[buffer(0)]],
 device uint4* total [[buffer(1)]],constant uint& count [[buffer(2)]],
 uint lane [[thread_index_in_threadgroup]]) {
 threadgroup ulong4 sums[256];
 compact_cooperative(values,total,count,lane,sums);
}
kernel void compact_scan_cooperative_pair(device uint4* a [[buffer(0)]],
 device uint4* atotal [[buffer(1)]],device uint4* b [[buffer(2)]],
 device uint4* btotal [[buffer(3)]],constant uint& count [[buffer(4)]],
 uint lane [[thread_index_in_threadgroup]]) {
 threadgroup ulong4 sums[256];
 compact_cooperative(a,atotal,count,lane,sums);
 compact_cooperative(b,btotal,count,lane,sums);
}
kernel void compact_scan_cooperative_domains(device uint4* a [[buffer(0)]],
 device uint4* atotal [[buffer(1)]],device uint4* b [[buffer(2)]],
 device uint4* btotal [[buffer(3)]],constant uint2& counts [[buffer(4)]],
 uint lane [[thread_index_in_threadgroup]]) {
 threadgroup ulong4 sums[256];
 compact_cooperative(a,atotal,counts.x,lane,sums);
 compact_cooperative(b,btotal,counts.y,lane,sums);
}
kernel void compact_scan_cooperative_counted(device uint4* values [[buffer(0)]],
 device uint4* total [[buffer(1)]],device const uint* length [[buffer(2)]],
 constant uint& capacity [[buffer(3)]],uint lane [[thread_index_in_threadgroup]]) {
 threadgroup ulong4 sums[256];
 uint count=length[0];
 if(count>capacity) {if(lane==0u)total[0]=uint4(0xffffffffu);return;}
 compact_cooperative(values,total,count,lane,sums);
}
