#include <metal_stdlib>
using namespace metal;
// Allocation counts depend on complete domain identity, not entry priority.
// Preserve all values and provide distinct occurrence ranks for grouping.
kernel void closure_count_headers(device const uint* states [[buffer(0)]],
 device const uint* header [[buffer(1)]],device const uint* ready [[buffer(2)]],
 device const uint* counts [[buffer(3)]],device const uint* status [[buffer(4)]],
 device uint* output [[buffer(5)]],device uint* length [[buffer(6)]],
 constant uint& capacity [[buffer(7)]],uint i [[thread_position_in_grid]]) {
 uint n=status[0]?0u:counts[1];
 if(!i) {length[0]=n;length[1]=status[0]?0u:header[1];length[2]=n/256u+uint(n%256u!=0u);length[3]=length[4]=1u;}
 if(i>=n)return;
 uint source=ready[capacity+i];
 for(uint column=0;column<12u;column++)output[column*n+i]=states[column*header[0]+source];
 output[12u*n+i]=i;
}
